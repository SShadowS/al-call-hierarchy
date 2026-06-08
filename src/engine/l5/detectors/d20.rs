//! D20 — a statement that immediately follows an unconditional exit
//! (`Exit;` / `Error(...)` / `CurrReport.Quit`) inside the same code block.
//! Port of al-sem `src/detectors/d20-unreachable-after-exit.ts`.
//!
//! The L2 indexer captures the exit anchor + first unreachable sibling during the
//! body DFS (`features.unreachableStatements`); this detector emits one finding per
//! recorded pair. primary + bodyAvailable + !parseIncomplete.
//!
//! Within-detector sort by `compareStrings(a.id, b.id)` (byte order).

use crate::engine::l3::l3_workspace::L3Resolved;
use crate::engine::l5::confidence::to_confidence;
use crate::engine::l5::detector_context::DetectorContext;
use crate::engine::l5::detectors::anchor_of;
use crate::engine::l5::finding::{Evidence, EvidenceStep, Finding, FindingConfidence, FixOption};
use crate::engine::l5::fingerprint::FingerprintIndex;
use crate::engine::l5::registry::{DetectorOutput, DetectorStats};

const DETECTOR: &str = "d20-unreachable-after-exit";

/// `EXIT_DESCRIPTION` — the human label for each exit kind.
fn kind_label(exit_kind: &str) -> &'static str {
    match exit_kind {
        "exit" => "Exit",
        "error" => "Error",
        "currreport-quit" => "CurrReport.Quit",
        // al-sem indexes only these three kinds; an unrecognised kind cannot occur,
        // but never panic in the engine — fall back to the raw kind via a stable
        // empty label would diverge, so this arm is unreachable in practice.
        _ => "Exit",
    }
}

pub fn detect_d20(resolved: &L3Resolved, _ctx: &DetectorContext) -> DetectorOutput {
    let ws = &resolved.workspace;
    let fp_index = FingerprintIndex::build(&ws.routines, &ws.objects);
    let mut findings: Vec<Finding> = Vec::new();
    let mut candidates_considered = 0usize;

    for routine in &ws.routines {
        // roleOf(routine) !== "primary" → skip. Source-only: every routine is
        // primary, so this never skips (mirrors al-sem semantics).
        if !routine.body_available {
            continue;
        }
        if routine.parse_incomplete {
            continue;
        }
        candidates_considered += 1;

        for u in &routine.unreachable_statements {
            let label = kind_label(&u.exit_kind);

            let path = vec![
                EvidenceStep {
                    routine_id: routine.id.clone(),
                    operation_id: None,
                    callsite_id: None,
                    loop_id: None,
                    source_anchor: anchor_of(&u.exit_anchor, routine),
                    note: format!("{label} statement — control leaves the routine here"),
                },
                EvidenceStep {
                    routine_id: routine.id.clone(),
                    operation_id: None,
                    callsite_id: None,
                    loop_id: None,
                    source_anchor: anchor_of(&u.unreachable_anchor, routine),
                    note: "this statement (and any siblings after it) is never executed"
                        .to_string(),
                },
            ];

            let id = format!("d20/{}", u.id);
            let root_cause_key = id.clone();

            let confidence: FindingConfidence = to_confidence(&[], "likely");

            let root_cause = format!(
                "{}: the statement after `{}` is unreachable — control leaves the routine \
                 before it can run.",
                routine.name, label
            );

            let mut finding = Finding {
                id,
                root_cause_key,
                detector: DETECTOR.to_string(),
                title: "Unreachable statement after unconditional exit".to_string(),
                root_cause,
                severity: "low".to_string(),
                confidence,
                primary_location: anchor_of(&u.unreachable_anchor, routine),
                evidence_path: path,
                additional_paths: None,
                affected_objects: vec![routine.object_id.clone()],
                affected_tables: Vec::new(),
                fix_options: vec![FixOption {
                    description:
                        "Remove the unreachable statement, or move the preceding exit / Error / \
                         Quit inside a conditional so the later code can run."
                            .to_string(),
                    safety: "high".to_string(),
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
    DetectorOutput {
        findings,
        stats: DetectorStats {
            detector: DETECTOR.to_string(),
            candidates_considered,
            findings_emitted: emitted,
        },
    }
}
