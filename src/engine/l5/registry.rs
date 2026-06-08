//! The detector registry + `run_detectors` — port of al-sem
//! `src/detectors/registry.ts`.
//!
//! `run_detectors` builds the shared `DetectorContext`, runs each detector
//! (catching a panic into a diagnostic so one detector cannot kill the run),
//! applies the role-scoping filter, then sorts the combined Finding[] by
//! `(detector compareNatural, primaryLocationKey compareStrings, rootCauseKey
//! compareStrings)` over the INTERNAL ids. Per-detector dedup-by-id happens INSIDE
//! each detector, not here.
//!
//! `compare_natural` is ported exactly from al-sem `uncertainty-util.ts`
//! (digit-runs numeric, letter-runs lexicographic; "d2" < "d10"). `compareStrings`
//! is plain `str::cmp` (byte order), used inline.

use crate::engine::l3::l3_workspace::L3Resolved;
use crate::engine::l5::detector_context::{build_detector_context, DetectorContext};
use crate::engine::l5::finding::Finding;

/// A diagnostic emitted when a detector panics (stage = "detect").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub severity: String,
    pub stage: String,
    pub message: String,
}

/// Per-detector stats (al-sem `DetectorStats`). Only the always-present fields are
/// modeled; the skip-counter map is kept opaque (the projection never reads it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectorStats {
    pub detector: String,
    pub candidates_considered: usize,
    pub findings_emitted: usize,
}

/// A detector's output.
pub struct DetectorOutput {
    pub findings: Vec<Finding>,
    pub stats: DetectorStats,
}

/// A detector: a pure query over the resolved model + shared context. The closure
/// receives `(resolved, ctx)`; al-sem also passes the combined graph, which the
/// ctx already carries (`ctx.graph`), so detectors read it from there.
pub struct Detector {
    pub name: String,
    pub run: fn(&L3Resolved, &DetectorContext) -> DetectorOutput,
}

/// The combined output of `run_detectors`.
pub struct RunOutput {
    pub findings: Vec<Finding>,
    pub diagnostics: Vec<Diagnostic>,
    pub detector_stats: Vec<DetectorStats>,
}

/// `primaryLocationKey(f) = ${sourceUnitId}:${startLine}:${startColumn}` over the
/// INTERNAL anchor.
fn primary_location_key(f: &Finding) -> String {
    let a = &f.primary_location;
    format!("{}:{}:{}", a.source_unit_id, a.start_line, a.start_column)
}

/// Run every registered detector in isolation, then role-scope + sort. A detector
/// that panics becomes a `Diagnostic(stage: "detect")` and the rest still run.
pub fn run_detectors(resolved: &L3Resolved, detectors: &[Detector]) -> RunOutput {
    let ctx = build_detector_context(resolved);
    let mut findings: Vec<Finding> = Vec::new();
    let mut diagnostics: Vec<Diagnostic> = Vec::new();
    let mut detector_stats: Vec<DetectorStats> = Vec::new();

    for detector in detectors {
        // Catch a panic so one detector cannot kill the run (al-sem try/catch).
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            (detector.run)(resolved, &ctx)
        }));
        match result {
            Ok(output) => {
                findings.extend(output.findings);
                detector_stats.push(output.stats);
            }
            Err(err) => {
                let msg = err
                    .downcast_ref::<&str>()
                    .map(|s| s.to_string())
                    .or_else(|| err.downcast_ref::<String>().cloned())
                    .unwrap_or_else(|| "panic".to_string());
                diagnostics.push(Diagnostic {
                    severity: "warning".to_string(),
                    stage: "detect".to_string(),
                    message: format!("Detector \"{}\" threw: {msg}", detector.name),
                });
            }
        }
    }

    // Role-scoping filter (registry.ts:161-172). Source-only: every routine's role
    // is "primary" (no analysisRole), so the predicate keeps everything. The
    // role-by-id map is built over the internal routine ids; the
    // primaryLocation.enclosingRoutineId is an internal id, so the lookup matches.
    // For the source-only R4-A wave there are no dependency routines, so the
    // `actionableAnchor` branch never engages; we still honour the "default
    // primary when unknown" semantics.
    let role_by_routine: std::collections::HashMap<&str, &str> = resolved
        .workspace
        .routines
        .iter()
        // analysisRole is not modeled on L3Routine (source-only ⇒ always primary).
        .map(|r| (r.id.as_str(), "primary"))
        .collect();
    let scoped: Vec<Finding> = findings
        .into_iter()
        .filter(|f| {
            let primary_role = role_by_routine
                .get(f.primary_location.enclosing_routine_id.as_str())
                .copied()
                .unwrap_or("primary");
            if primary_role == "primary" {
                return true;
            }
            if let Some(anchor) = &f.actionable_anchor {
                let anchor_role = role_by_routine
                    .get(anchor.enclosing_routine_id.as_str())
                    .copied()
                    .unwrap_or("primary");
                if anchor_role == "primary" {
                    return true;
                }
            }
            false
        })
        .collect();

    let mut scoped = scoped;
    scoped.sort_by(|a, b| {
        compare_natural(&a.detector, &b.detector)
            .then_with(|| primary_location_key(a).cmp(&primary_location_key(b)))
            .then_with(|| a.root_cause_key.cmp(&b.root_cause_key))
    });

    RunOutput {
        findings: scoped,
        diagnostics,
        detector_stats,
    }
}

/// Port of al-sem `compareNatural`: split each string into runs of digits and
/// non-digits; compare digit runs numerically and non-digit runs by byte order.
/// On a prefix tie, the shorter token list sorts first. ("d2" < "d10".)
pub fn compare_natural(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let pa = tokenize(a);
    let pb = tokenize(b);
    let len = pa.len().min(pb.len());
    for i in 0..len {
        let ta = &pa[i];
        let tb = &pb[i];
        let a_is_num = ta.chars().next().is_some_and(|c| c.is_ascii_digit());
        let b_is_num = tb.chars().next().is_some_and(|c| c.is_ascii_digit());
        if a_is_num && b_is_num {
            // Compare numerically. al-sem uses parseInt (base 10) into a JS
            // number; corpus ids stay well within u128, so parse into u128.
            let na: u128 = ta.parse().unwrap_or(0);
            let nb: u128 = tb.parse().unwrap_or(0);
            if na != nb {
                return na.cmp(&nb);
            }
        } else if a_is_num != b_is_num {
            // A numeric token sorts before a non-numeric token.
            return if a_is_num {
                Ordering::Less
            } else {
                Ordering::Greater
            };
        } else if ta != tb {
            return ta.as_str().cmp(tb.as_str());
        }
    }
    pa.len().cmp(&pb.len())
}

/// Split a string into maximal runs of digits / non-digits, matching the JS regex
/// `/(\d+)|(\D+)/g`.
fn tokenize(s: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut cur_is_digit: Option<bool> = None;
    for ch in s.chars() {
        let is_digit = ch.is_ascii_digit();
        match cur_is_digit {
            Some(d) if d == is_digit => cur.push(ch),
            Some(_) => {
                out.push(std::mem::take(&mut cur));
                cur.push(ch);
                cur_is_digit = Some(is_digit);
            }
            None => {
                cur.push(ch);
                cur_is_digit = Some(is_digit);
            }
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp::Ordering;

    #[test]
    fn natural_orders_detectors_numerically() {
        assert_eq!(compare_natural("d2", "d10"), Ordering::Less);
        assert_eq!(compare_natural("d10", "d2"), Ordering::Greater);
        assert_eq!(compare_natural("d4", "d4"), Ordering::Equal);
        assert_eq!(
            compare_natural("d4-repeated-lookup-in-loop", "d4-repeated-lookup-in-loop"),
            Ordering::Equal
        );
    }

    #[test]
    fn natural_numeric_token_before_alpha_token() {
        // "1a" vs "a1": first tokens are "1" (num) and "a" (non-num) → num first.
        assert_eq!(compare_natural("1a", "a1"), Ordering::Less);
    }

    #[test]
    fn natural_shorter_prefix_first() {
        assert_eq!(compare_natural("d4", "d4x"), Ordering::Less);
    }
}
