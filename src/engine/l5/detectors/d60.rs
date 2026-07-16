//! D60 — repeat…Modify…until loop in an Upgrade/Install codeunit. BCQuality
//! `datatransfer-for-bulk-init`: upgrade code rewriting rows one-by-one should
//! use DataTransfer (set-based SQL, no per-row trigger cost) — on large tables
//! the difference is hours vs seconds.
//!
//! Join: object_subtype ∈ {Upgrade, Install} (Codeunit), a Modify record op with
//! non-empty loop_stack whose receiver is a live cursor (FindSet/Find/FindFirst/
//! Next on the same var in the routine). One finding per (routine, loop, var) —
//! first op wins. Severity: medium. Confidence: likely.

use std::collections::{HashMap, HashSet};

use crate::engine::l2::features::PLoop;
use crate::engine::l3::l3_workspace::L3Resolved;
use crate::engine::l5::confidence::to_confidence;
use crate::engine::l5::detector_context::DetectorContext;
use crate::engine::l5::detectors::anchor_of;
use crate::engine::l5::finding::{Evidence, EvidenceStep, Finding, FindingConfidence, FixOption};
use crate::engine::l5::fingerprint::FingerprintIndex;
use crate::engine::l5::registry::{DetectorError, DetectorOutput, DetectorStats};

const DETECTOR: &str = "d60-upgrade-loop-should-be-datatransfer";

const CURSOR_OPS: &[&str] = &["FindSet", "Find", "FindFirst", "Next"];

pub fn detect_d60(
    resolved: &L3Resolved,
    ctx: &DetectorContext,
) -> Result<DetectorOutput, DetectorError> {
    let ws = &resolved.workspace;
    let fp_index = FingerprintIndex::build(&ws.routines, &ws.objects);
    let mut findings: Vec<Finding> = Vec::new();
    let mut candidates_considered = 0usize;
    let mut skipped_not_cursor = 0u64;

    let lifecycle_objects: HashSet<&str> = ws
        .objects
        .iter()
        .filter(|o| {
            o.object_type == "Codeunit"
                && o.object_subtype.as_deref().is_some_and(|s| {
                    s.eq_ignore_ascii_case("Upgrade") || s.eq_ignore_ascii_case("Install")
                })
        })
        .map(|o| o.id.as_str())
        .collect();
    if lifecycle_objects.is_empty() {
        let stats = DetectorStats::new(DETECTOR, 0, 0);
        return Ok(DetectorOutput::no_diag(findings, stats));
    }

    for routine in &ws.routines {
        if !lifecycle_objects.contains(routine.object_id.as_str()) {
            continue;
        }
        if !routine.body_available || routine.parse_incomplete {
            continue;
        }
        let loop_by_id: HashMap<&str, &PLoop> =
            routine.loops.iter().map(|l| (l.id.as_str(), l)).collect();
        let cursor_vars: HashSet<String> = routine
            .record_operations
            .iter()
            .filter(|op| CURSOR_OPS.contains(&op.op.as_str()))
            .map(|op| op.record_variable_name.to_lowercase())
            .collect();

        // One finding per (loop, var): first Modify wins.
        let mut reported: HashSet<(String, String)> = HashSet::new();
        for op in &routine.record_operations {
            if op.op != "Modify" {
                continue;
            }
            let Some(rep_loop_id) = op.loop_stack.last() else {
                continue;
            };
            candidates_considered += 1;
            let var_lc = op.record_variable_name.to_lowercase();
            if !cursor_vars.contains(&var_lc) {
                skipped_not_cursor += 1;
                continue;
            }
            if !reported.insert((rep_loop_id.clone(), var_lc.clone())) {
                continue;
            }
            let Some(loop_info) = loop_by_id.get(rep_loop_id.as_str()) else {
                continue;
            };

            let table_name = op
                .table_id
                .as_deref()
                .and_then(|tid| ctx.table_by_id.get(tid).map(|t| t.name.clone()))
                .unwrap_or_else(|| op.record_variable_name.clone());

            let confidence: FindingConfidence = to_confidence(&[], "likely");
            let mut finding = Finding {
                id: format!("d60/{}/{}/{}", routine.id, loop_info.id, op.id),
                root_cause_key: format!("d60/{}/{}/{}", routine.id, loop_info.id, var_lc),
                detector: DETECTOR.to_string(),
                title: "Row-by-row upgrade loop (use DataTransfer)".to_string(),
                root_cause: format!(
                    "{} (upgrade/install codeunit) rewrites {} row-by-row in a {} loop — \
                     DataTransfer performs the same bulk init/copy set-based, without \
                     per-row trigger cost.",
                    routine.name, table_name, loop_info.loop_type
                ),
                severity: "medium".to_string(),
                confidence,
                primary_location: anchor_of(&op.source_anchor, routine),
                evidence_path: vec![
                    EvidenceStep {
                        routine_id: routine.id.clone(),
                        operation_id: None,
                        callsite_id: None,
                        loop_id: Some(loop_info.id.clone()),
                        source_anchor: anchor_of(&loop_info.source_anchor, routine),
                        note: format!("{} loop over {}", loop_info.loop_type, table_name),
                    },
                    EvidenceStep {
                        routine_id: routine.id.clone(),
                        operation_id: Some(op.id.clone()),
                        callsite_id: None,
                        loop_id: Some(loop_info.id.clone()),
                        source_anchor: anchor_of(&op.source_anchor, routine),
                        note: "per-row Modify".to_string(),
                    },
                ],
                additional_paths: None,
                affected_objects: vec![routine.object_id.clone()],
                affected_tables: op.table_id.iter().cloned().collect(),
                fix_options: vec![FixOption {
                    description: "Replace the loop with a DataTransfer (SourceTable/\
                                  DestinationTable + CopyFields/ConstantValue), or ModifyAll \
                                  when a single field gets a constant."
                        .to_string(),
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
    stats.add_skip("notCursorVar", skipped_not_cursor);
    Ok(DetectorOutput::no_diag(findings, stats))
}
