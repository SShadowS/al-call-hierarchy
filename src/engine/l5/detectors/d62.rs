//! D62 — `FeatureTelemetry.LogUsage` before the success path (OPT-IN).
//! BCQuality `feature-usage-only-after-success`: usage logged before a fallible
//! step (record write or explicit Error call later in the routine) counts
//! failed runs as feature usage.
//!
//! Join: member call `<v>.LogUsage(..)` where `<v>`'s DECLARED type contains
//! `codeunit "feature telemetry"` (text match — the System Application codeunit
//! is not in workspace source), with any record write op or error-call
//! operation site strictly AFTER it in the same routine (straight-line source
//! order). Severity: low. Confidence: possible.

use crate::engine::l2::features::PCallee;
use crate::engine::l3::l3_workspace::L3Resolved;
use crate::engine::l5::confidence::to_confidence;
use crate::engine::l5::detector_context::DetectorContext;
use crate::engine::l5::detectors::{anchor_of, before_anchor};
use crate::engine::l5::finding::{Evidence, EvidenceStep, Finding, FindingConfidence, FixOption};
use crate::engine::l5::fingerprint::FingerprintIndex;
use crate::engine::l5::registry::{DetectorError, DetectorOutput, DetectorStats};

const DETECTOR: &str = "d62-telemetry-before-success";

const WRITE_OPS: &[&str] = &[
    "Insert",
    "Modify",
    "Delete",
    "DeleteAll",
    "ModifyAll",
    "Rename",
];

pub fn detect_d62(
    resolved: &L3Resolved,
    _ctx: &DetectorContext,
) -> Result<DetectorOutput, DetectorError> {
    let ws = &resolved.workspace;
    let fp_index = FingerprintIndex::build(&ws.routines, &ws.objects);
    let mut findings: Vec<Finding> = Vec::new();
    let mut candidates_considered = 0usize;
    let mut skipped_terminal_log = 0u64;

    for routine in &ws.routines {
        if !routine.body_available || routine.parse_incomplete {
            continue;
        }
        let ft_vars: Vec<String> = routine
            .variables
            .iter()
            .filter(|v| {
                let t = v.declared_type.to_lowercase();
                t.starts_with("codeunit") && t.contains("feature telemetry")
            })
            .map(|v| v.name.to_lowercase())
            .collect();
        if ft_vars.is_empty() {
            continue;
        }

        for cs in &routine.call_sites {
            let PCallee::Member { receiver, method } = &cs.callee else {
                continue;
            };
            if !method.eq_ignore_ascii_case("LogUsage") {
                continue;
            }
            if !ft_vars.contains(&receiver.to_lowercase()) {
                continue;
            }
            candidates_considered += 1;

            let fallible_after = routine.record_operations.iter().any(|op| {
                WRITE_OPS.contains(&op.op.as_str())
                    && before_anchor(&cs.source_anchor, &op.source_anchor)
            }) || routine.operation_sites.iter().any(|s| {
                s.kind == "error-call" && before_anchor(&cs.source_anchor, &s.source_anchor)
            });
            if !fallible_after {
                skipped_terminal_log += 1;
                continue;
            }

            let confidence: FindingConfidence = to_confidence(&[], "possible");
            let id = format!("d62/{}/{}", routine.id, cs.id);
            let mut finding = Finding {
                id: id.clone(),
                root_cause_key: id,
                detector: DETECTOR.to_string(),
                title: "Feature usage logged before success".to_string(),
                root_cause: format!(
                    "{} calls FeatureTelemetry.LogUsage before fallible work later in the \
                     routine — runs that fail after the log still count as feature usage.",
                    routine.name
                ),
                severity: "low".to_string(),
                confidence,
                primary_location: anchor_of(&cs.source_anchor, routine),
                evidence_path: vec![EvidenceStep {
                    routine_id: routine.id.clone(),
                    operation_id: None,
                    callsite_id: Some(cs.id.clone()),
                    loop_id: None,
                    source_anchor: anchor_of(&cs.source_anchor, routine),
                    note: "LogUsage before fallible operations".to_string(),
                }],
                additional_paths: None,
                affected_objects: vec![routine.object_id.clone()],
                affected_tables: Vec::new(),
                fix_options: vec![FixOption {
                    description: "Move LogUsage after the operation's success point (end of \
                                  the routine / after the final write)."
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
    let mut stats = DetectorStats::new(DETECTOR, candidates_considered, emitted);
    stats.add_skip("terminalLog", skipped_terminal_log);
    Ok(DetectorOutput::no_diag(findings, stats))
}
