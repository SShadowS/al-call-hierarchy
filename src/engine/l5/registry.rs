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
use crate::engine::l5::detector_context::{
    build_detector_context, build_detector_context_cross_app, DetectorContext,
};
use crate::engine::l5::finding::Finding;

/// A diagnostic emitted when a detector panics (stage = "detect").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub severity: String,
    pub stage: String,
    pub message: String,
}

/// Per-detector stats (al-sem `DetectorStats`).
///
/// `skipped` is a `BTreeMap<String, u64>` that serializes as a JSON object with keys
/// in alphabetical order (BTreeMap gives this for free). A key is inserted ONLY when
/// its count is > 0 — except for d43 which always emits `other` (even when 0).
/// An empty map serializes as `{}`.
///
/// Serialization contract (canonical sorted-key JSON):
///   - Keys are sorted alphabetically (`BTreeMap` iteration order).
///   - Field order in the JSON object is: `candidatesConsidered`, `detector`,
///     `findingsEmitted`, `skipped` — exactly the alphabetical order of those names.
///   - 2-space indent, trailing newline on the array.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectorStats {
    pub detector: String,
    pub candidates_considered: usize,
    pub findings_emitted: usize,
    /// Skip counters. Insert with `skipped.entry(key).and_modify(|v| *v += 1).or_insert(1)`
    /// or the convenience `insert_skip` method. Keys must match the taxonomy exactly.
    pub skipped: std::collections::BTreeMap<String, u64>,
}

impl DetectorStats {
    /// Create a new `DetectorStats` with an empty skipped map.
    pub fn new(detector: impl Into<String>, candidates_considered: usize, findings_emitted: usize) -> Self {
        Self {
            detector: detector.into(),
            candidates_considered,
            findings_emitted,
            skipped: std::collections::BTreeMap::new(),
        }
    }

    /// Increment a skip counter by 1, inserting it if absent.
    pub fn inc_skip(&mut self, key: &str) {
        *self.skipped.entry(key.to_string()).or_insert(0) += 1;
    }

    /// Add `n` to a skip counter, inserting it if absent. Only inserts when `n > 0`.
    pub fn add_skip(&mut self, key: &str, n: u64) {
        if n > 0 {
            *self.skipped.entry(key.to_string()).or_insert(0) += n;
        }
    }

    /// Serialize this stats object to a `serde_json::Value` with alphabetically-sorted
    /// keys, matching the al-sem `sortedReplacer` output. The field order matches the
    /// alphabetical key sort: `candidatesConsidered`, `detector`, `findingsEmitted`, `skipped`.
    pub fn to_json_value(&self) -> serde_json::Value {
        let skipped_obj: serde_json::Map<String, serde_json::Value> = self
            .skipped
            .iter()
            .map(|(k, v)| (k.clone(), serde_json::Value::Number((*v).into())))
            .collect();
        // The field ORDER in the serde_json::Map must be alphabetical so that
        // serde_json::to_string_pretty emits them in alphabetical order.
        // Use a BTreeMap-backed Map by constructing via sorted insertion.
        let mut obj = serde_json::Map::new();
        obj.insert(
            "candidatesConsidered".to_string(),
            serde_json::Value::Number(self.candidates_considered.into()),
        );
        obj.insert(
            "detector".to_string(),
            serde_json::Value::String(self.detector.clone()),
        );
        obj.insert(
            "findingsEmitted".to_string(),
            serde_json::Value::Number(self.findings_emitted.into()),
        );
        obj.insert(
            "skipped".to_string(),
            serde_json::Value::Object(skipped_obj),
        );
        serde_json::Value::Object(obj)
    }
}

/// Serialize a `Vec<DetectorStats>` to a canonical sorted-key JSON string: 2-space
/// indent, trailing newline, all object keys in alphabetical order. This is the format
/// the al-sem golden files use (JSON.stringify with sortedReplacer + 2-space indent +
/// trailing newline).
pub fn serialize_detector_stats(stats: &[DetectorStats]) -> String {
    let arr: Vec<serde_json::Value> = stats.iter().map(|s| s.to_json_value()).collect();
    let val = serde_json::Value::Array(arr);
    let mut out = serde_json::to_string_pretty(&val).expect("DetectorStats serialization failed");
    out.push('\n');
    out
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
    let (findings, diagnostics, detector_stats) = run_each(resolved, &ctx, detectors);

    // Role-scoping filter (registry.ts:161-172). Source-only: every routine's role
    // is "primary" (no analysisRole), so the predicate keeps everything.
    let role_by_routine: std::collections::HashMap<&str, &str> = resolved
        .workspace
        .routines
        .iter()
        // analysisRole is not modeled on L3Routine (source-only ⇒ always primary).
        .map(|r| (r.id.as_str(), "primary"))
        .collect();
    let scoped = role_scope_and_sort(findings, &role_by_routine);

    RunOutput {
        findings: scoped,
        diagnostics,
        detector_stats,
    }
}

/// CROSS-APP variant of `run_detectors`: build the cross-app context from the
/// pre-assembled `R3a5CrossAppBase`, run every detector, then role-scope with
/// `dep_routine_ids`-derived roles (`"dependency"` for dep routines, `"primary"`
/// else) so dep-anchored findings are dropped by the existing scope filter. d13/d16
/// already gate `roleOf(caller)` internally via `ctx.dep_routine_ids`; the scope
/// filter is the second, anchor-based safety net (registry.ts:161-172 parity).
pub(crate) fn run_detectors_cross_app(
    base: &crate::engine::l4::capability_cone::R3a5CrossAppBase,
    detectors: &[Detector],
) -> RunOutput {
    let ctx = build_detector_context_cross_app(base);
    // The detectors close over `(resolved, ctx)`. Build a throwaway L3Resolved view
    // over the merged routines so the `resolved.workspace` arg is consistent with the
    // ctx (detectors read `resolved.workspace.routines`/`.objects` for the fingerprint
    // index + role map; for d13/d16/d17 those are the merged sets in `base`).
    let resolved = L3Resolved {
        workspace: merged_workspace_view(base),
        root_classifications: Vec::new(),
        primary_app: None,
    };
    let (findings, diagnostics, detector_stats) = run_each(&resolved, &ctx, detectors);

    // role_by_routine: dep routines → "dependency", else "primary".
    let role_by_routine: std::collections::HashMap<&str, &str> = base
        .ws_routines
        .iter()
        .map(|r| {
            let role = if base.dep_routine_ids.contains(&r.id) {
                "dependency"
            } else {
                "primary"
            };
            (r.id.as_str(), role)
        })
        .collect();
    let scoped = role_scope_and_sort(findings, &role_by_routine);

    RunOutput {
        findings: scoped,
        diagnostics,
        detector_stats,
    }
}

/// Build an `L3Workspace` view over the merged base routines/objects/tables — the
/// `resolved.workspace` arg every detector receives. The cross-app detectors read
/// `routines` (role map + fingerprint index) and `objects` (fingerprint index);
/// the merged sets come straight from `base`.
fn merged_workspace_view(
    base: &crate::engine::l4::capability_cone::R3a5CrossAppBase,
) -> crate::engine::l3::l3_workspace::L3Workspace {
    crate::engine::l3::l3_workspace::L3Workspace {
        objects: base.objects.clone(),
        tables: base.tables.clone(),
        routines: base.ws_routines.clone(),
    }
}

/// Run each detector in isolation (panic → diagnostic), collecting findings + stats.
fn run_each(
    resolved: &L3Resolved,
    ctx: &DetectorContext,
    detectors: &[Detector],
) -> (Vec<Finding>, Vec<Diagnostic>, Vec<DetectorStats>) {
    let mut findings: Vec<Finding> = Vec::new();
    let mut diagnostics: Vec<Diagnostic> = Vec::new();
    let mut detector_stats: Vec<DetectorStats> = Vec::new();

    for detector in detectors {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            (detector.run)(resolved, ctx)
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
    (findings, diagnostics, detector_stats)
}

/// Apply the role-scope filter (drop dep-anchored findings) then the stable sort.
fn role_scope_and_sort(
    findings: Vec<Finding>,
    role_by_routine: &std::collections::HashMap<&str, &str>,
) -> Vec<Finding> {
    let mut scoped: Vec<Finding> = findings
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

    scoped.sort_by(|a, b| {
        compare_natural(&a.detector, &b.detector)
            .then_with(|| primary_location_key(a).cmp(&primary_location_key(b)))
            .then_with(|| a.root_cause_key.cmp(&b.root_cause_key))
    });
    scoped
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
