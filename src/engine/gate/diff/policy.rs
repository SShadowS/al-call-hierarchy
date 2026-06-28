//! Coverage policy post-pass. Port of al-sem `src/diff/diff-policy.ts`.
//!
//! Annotates coverage-sensitive findings with `{old,new}` cone coverage. Under
//! `strict`, drops findings whose cone is incomplete (emitting a
//! `coverage-incomplete` diagnostic); under `loose`, downgrades `CapabilityLost`
//! → `CapabilityLostUnderCoverage` when the NEW cone is incomplete.

use crate::engine::gate::cbor::CborValue;

use super::fingerprint::DiffKind;
use super::indexes::DiffIndexes;
use super::{CoveragePolicy, DiffDiagnostic, DiffFinding, get_str};

/// The cone coverage status: `complete` unless any cone member's inheritedStatus
/// is `partial` (→ partial) or `unknown`/absent (→ unknown, short-circuits).
fn cone_status(cone: &[String], source: &super::indexes::DiffIndexes, new_side: bool) -> String {
    let mut worst = "complete";
    for id in cone {
        let rec = if new_side {
            source.new_coverage_by_subject.get(id)
        } else {
            source.old_coverage_by_subject.get(id)
        };
        let s = rec
            .and_then(|r| get_str(r, "inheritedStatus"))
            .unwrap_or("unknown");
        if s == "unknown" {
            return "unknown".to_string();
        }
        if s == "partial" {
            worst = "partial";
        }
    }
    worst.to_string()
}

fn is_coverage_sensitive(kind: DiffKind) -> bool {
    matches!(
        kind,
        DiffKind::CapabilityLost
            | DiffKind::CapabilityGainedWrite
            | DiffKind::CapabilityGainedRead
            | DiffKind::CapabilityGainedCommit
            | DiffKind::CapabilityGainedHttp
            | DiffKind::CapabilityGainedTelemetry
            | DiffKind::CapabilityGainedIsolatedStorage
            | DiffKind::CapabilityGainedFile
            | DiffKind::CapabilityGainedDynamicDispatch
            | DiffKind::CapabilityGainedEventPublish
    )
}

pub fn apply_coverage_policy(
    findings: Vec<DiffFinding>,
    indexes: &DiffIndexes,
    coverage_policy: CoveragePolicy,
) -> (Vec<DiffFinding>, Vec<DiffDiagnostic>) {
    let mut out_findings: Vec<DiffFinding> = Vec::new();
    let mut diagnostics: Vec<DiffDiagnostic> = Vec::new();

    for finding in findings {
        if !is_coverage_sensitive(finding.kind) {
            out_findings.push(finding);
            continue;
        }
        let old_status = cone_status(&finding.comparison_cone, indexes, false);
        let new_status = cone_status(&finding.comparison_cone, indexes, true);
        let old_partial = old_status != "complete";
        let new_partial = new_status != "complete";

        let mut annotated = finding;
        annotated.coverage_state = Some((old_status.clone(), new_status.clone()));

        if coverage_policy == CoveragePolicy::Strict {
            if old_partial || new_partial {
                let cone = if old_partial && new_partial {
                    "both"
                } else if old_partial {
                    "old"
                } else {
                    "new"
                };
                let subject = annotated
                    .comparison_cone
                    .first()
                    .cloned()
                    .unwrap_or_default();
                diagnostics.push(DiffDiagnostic {
                    kind: "coverage-incomplete".into(),
                    fields: vec![
                        ("kind".into(), CborValue::Text("coverage-incomplete".into())),
                        ("subject".into(), CborValue::Text(subject)),
                        ("cone".into(), CborValue::Text(cone.into())),
                    ],
                });
                continue; // drop under strict
            }
            out_findings.push(annotated);
            continue;
        }

        // Loose: downgrade CapabilityLost → CapabilityLostUnderCoverage when new
        // partial. al-sem spreads `{ ...annotated, kind: … }` — it changes ONLY the
        // `kind`, NOT the severity (so the downgraded finding keeps its medium
        // severity; verified against the abi-signature-changed golden). The `details`
        // map is NOT updated either (it still carries `kind:"capability-lost"`).
        if annotated.kind == DiffKind::CapabilityLost && new_partial {
            annotated.kind = DiffKind::CapabilityLostUnderCoverage;
        }
        out_findings.push(annotated);
    }

    (out_findings, diagnostics)
}
