//! D56 — Record cloned before Modify/Delete inside a loop. BCQuality
//! `avoid-cloning-records-before-modify-delete-in-loops`: `Copy := Cursor;
//! Copy.Modify();` per iteration re-reads/re-writes the row the cursor already
//! holds — an extra SQL round-trip per row. Modify the cursor directly (or use
//! ModifyAll / a temp buffer).
//!
//! Join (all intraprocedural, exact):
//!  - a whole-record copy `lhs := rhs` (PVarAssignment.rhs_identifier — set ONLY
//!    for bare-identifier-to-bare-identifier copies) between two RECORD vars,
//!  - the assignment sits inside a loop (innermost containing PLoop by anchor),
//!  - a Modify/Delete on the CLONE, in the SAME loop (op.loop_stack), AFTER the copy,
//!  - the SOURCE is a live cursor (has FindSet/Find/FindFirst/Next ops in the routine).
//!
//! Severity: medium. Confidence: likely.

use std::collections::HashSet;

use crate::engine::l2::features::PAnchor;
use crate::engine::l3::l3_workspace::L3Resolved;
use crate::engine::l5::confidence::to_confidence;
use crate::engine::l5::detector_context::DetectorContext;
use crate::engine::l5::detectors::{anchor_of, before_anchor};
use crate::engine::l5::finding::{Evidence, EvidenceStep, Finding, FindingConfidence, FixOption};
use crate::engine::l5::fingerprint::FingerprintIndex;
use crate::engine::l5::registry::{DetectorError, DetectorOutput, DetectorStats};

const DETECTOR: &str = "d56-clone-before-write-in-loop";

const CURSOR_OPS: &[&str] = &["FindSet", "Find", "FindFirst", "Next"];
const WRITE_BACK_OPS: &[&str] = &["Modify", "Delete"];

/// Anchor containment: `inner` fully inside `outer`.
fn anchor_within(inner: &PAnchor, outer: &PAnchor) -> bool {
    let starts_ok = outer.start_line < inner.start_line
        || (outer.start_line == inner.start_line && outer.start_column <= inner.start_column);
    let ends_ok = inner.end_line < outer.end_line
        || (inner.end_line == outer.end_line && inner.end_column <= outer.end_column);
    starts_ok && ends_ok
}

pub fn detect_d56(
    resolved: &L3Resolved,
    _ctx: &DetectorContext,
) -> Result<DetectorOutput, DetectorError> {
    let ws = &resolved.workspace;
    let fp_index = FingerprintIndex::build(&ws.routines, &ws.objects);
    let mut findings: Vec<Finding> = Vec::new();
    let mut candidates_considered = 0usize;
    let mut skipped_no_write_back = 0u64;
    let mut skipped_source_not_cursor = 0u64;

    for routine in &ws.routines {
        if !routine.body_available || routine.parse_incomplete {
            continue;
        }
        if routine.loops.is_empty() || routine.var_assignments.is_empty() {
            continue;
        }
        let record_names: HashSet<String> = routine
            .record_variables
            .iter()
            .map(|rv| rv.name.to_lowercase())
            .collect();

        for asg in &routine.var_assignments {
            let Some(rhs_lc) = asg.rhs_identifier.as_deref() else {
                continue;
            };
            // Whole-record copy between two record vars only.
            if !record_names.contains(rhs_lc) || !record_names.contains(&asg.lhs_name) {
                continue;
            }
            // Innermost loop containing the assignment (assignments carry no
            // loop_stack; containment is by anchor).
            let Some(lp) = routine
                .loops
                .iter()
                .filter(|l| anchor_within(&asg.source_anchor, &l.source_anchor))
                .max_by_key(|l| (l.source_anchor.start_line, l.source_anchor.start_column))
            else {
                continue;
            };
            candidates_considered += 1;

            // Written back inside the SAME loop, after the copy.
            let write = routine.record_operations.iter().find(|op| {
                WRITE_BACK_OPS.contains(&op.op.as_str())
                    && op.record_variable_name.to_lowercase() == asg.lhs_name
                    && op.loop_stack.iter().any(|id| id == &lp.id)
                    && before_anchor(&asg.source_anchor, &op.source_anchor)
            });
            let Some(write) = write else {
                skipped_no_write_back += 1;
                continue;
            };
            // The copy SOURCE must be a live cursor.
            let src_is_cursor = routine.record_operations.iter().any(|op| {
                CURSOR_OPS.contains(&op.op.as_str())
                    && op.record_variable_name.to_lowercase() == rhs_lc
            });
            if !src_is_cursor {
                skipped_source_not_cursor += 1;
                continue;
            }

            let confidence: FindingConfidence = to_confidence(&[], "likely");
            let id = format!("d56/{}/{}/{}", routine.id, lp.id, write.id);
            let mut finding = Finding {
                id: id.clone(),
                root_cause_key: format!("d56/{}/{}", routine.id, lp.id),
                detector: DETECTOR.to_string(),
                title: format!("Record cloned before {} in loop", write.op),
                root_cause: format!(
                    "{} copies the loop cursor {} into {} and calls {} on the copy inside \
                     the loop — an extra SQL round-trip per row; the cursor already holds \
                     the row.",
                    routine.name, rhs_lc, asg.lhs_name, write.op
                ),
                severity: "medium".to_string(),
                confidence,
                primary_location: anchor_of(&write.source_anchor, routine),
                evidence_path: vec![
                    EvidenceStep {
                        routine_id: routine.id.clone(),
                        operation_id: None,
                        callsite_id: None,
                        loop_id: Some(lp.id.clone()),
                        source_anchor: anchor_of(&asg.source_anchor, routine),
                        note: format!(
                            "clone {} := {} inside {} loop",
                            asg.lhs_name, rhs_lc, lp.loop_type
                        ),
                    },
                    EvidenceStep {
                        routine_id: routine.id.clone(),
                        operation_id: Some(write.id.clone()),
                        callsite_id: None,
                        loop_id: Some(lp.id.clone()),
                        source_anchor: anchor_of(&write.source_anchor, routine),
                        note: format!("{} on the clone", write.op),
                    },
                ],
                additional_paths: None,
                affected_objects: vec![routine.object_id.clone()],
                affected_tables: write.table_id.iter().cloned().collect(),
                fix_options: vec![FixOption {
                    description: format!(
                        "Call {} on the cursor ({}) directly, or restructure to a set-based \
                         write (ModifyAll/DeleteAll) outside the loop.",
                        write.op, rhs_lc
                    ),
                    safety: "medium".to_string(),
                }],
                provenance: vec![Evidence {
                    source: "tree-sitter".to_string(),
                    note: None,
                }],
                actionable_anchor: None,
                fingerprint: None,
                event_kind: None,
                cross_extension_subscribers: None,
            };
            finding.fingerprint = Some(fp_index.fingerprint_of(&finding));
            findings.push(finding);
        }
    }

    findings.sort_by(|a, b| a.id.cmp(&b.id));
    let emitted = findings.len();
    let mut stats = DetectorStats::new(DETECTOR, candidates_considered, emitted);
    stats.add_skip("noWriteBack", skipped_no_write_back);
    stats.add_skip("sourceNotCursor", skipped_source_not_cursor);
    Ok(DetectorOutput::no_diag(findings, stats))
}
