//! The detector registry + `run_detectors` — port of al-sem
//! `src/detectors/registry.ts`.
//!
//! `run_detectors` builds the shared `DetectorContext`, runs each detector
//! (an `Err` becomes a diagnostic so one detector cannot kill the run — the real,
//! abort-safe contract; see the `run_detectors` doc comment), applies the
//! role-scoping filter, then sorts the combined Finding[] by
//! `(detector compareNatural, primaryLocationKey compareStrings, rootCauseKey
//! compareStrings)` over the INTERNAL ids. Per-detector dedup-by-id happens INSIDE
//! each detector, not here.
//!
//! `compare_natural` is ported exactly from al-sem `uncertainty-util.ts`
//! (digit-runs numeric, letter-runs lexicographic; "d2" < "d10"). `compareStrings`
//! is plain `str::cmp` (byte order), used inline.

use crate::engine::l3::l3_workspace::L3Resolved;
use crate::engine::l5::detector_context::{
    DetectorContext, build_detector_context, build_detector_context_cross_app,
};
use crate::engine::l5::finding::Finding;

/// Substrate demand bits (W1.0 demand-driven detector substrate).
///
/// `build_detector_context` ALWAYS builds the cheap, many-consumer CORE surface —
/// symbol table, `resolve_calls`, event graph, combined graph, reverse graph, entry
/// points, reachable roots, all borrowed indexes (routine/object/table/call-site
/// maps), `resolved_call_edge_by_callsite`, `uncertainty_edges_by_from`,
/// `upgraded_bindings_by_callsite`, `event_flow_indexes`, `cross_extension_subscribers`
/// (T3), `fingerprint_index` (T1), and `root_classifications_by_routine`. The four
/// EXPENSIVE substrates below are built only when some selected detector demands them;
/// `run_detectors` folds every detector's `requires` into the union it passes here.
///
/// Skipped substrates leave their ctx fields EMPTY (`HashMap::new()`/`Vec::new()`/
/// `Default::default()`) — the field TYPES are unchanged, so no detector needs to
/// change. The per-detector full-vs-minimal parity test is the enforcement: an
/// under-declared `requires` produces a finding divergence and fails the test.
///
/// A full/preset/all-detector run demands `ALL`, so the whole context — and thus the
/// entire report — is byte-identical to the pre-W1.0 eager build. The ONLY permitted
/// output change (decision (a), user-approved) is that a selection NOT demanding
/// `CORE_SUMMARIES` emits no summarize cap-hit diagnostics (they are harvested from
/// the same `compute_summaries` call this substrate gates).
pub mod substrate {
    /// Capability cones + the per-routine `FullRoutineSummary` map (`ctx.summaries`).
    pub const SUMMARIES: u32 = 1 << 0;
    /// The second Tarjan SCC + Jacobi CORE summaries → `ctx.uncertainties_by_node`,
    /// `ctx.parameter_roles_by_routine`, and the `summarize_diagnostics` cap-hit set.
    pub const CORE_SUMMARIES: u32 = 1 << 1;
    /// Transaction spans (`ctx.transaction_spans`). Requires SUMMARIES internally —
    /// `compute_transaction_spans` folds over the summaries map — so the summaries
    /// block is built whenever this bit is set (see `build_detector_context`).
    pub const TRANSACTION_SPANS: u32 = 1 << 2;
    /// Closed-world proven-temp params (`ctx.closed_world_temp_params`).
    pub const CLOSED_WORLD_TEMP: u32 = 1 << 3;
    /// Every substrate — the eager, pre-W1.0 behavior. Full/preset/all-detector runs
    /// and every non-registry `build_detector_context` caller pass this.
    pub const ALL: u32 = SUMMARIES | CORE_SUMMARIES | TRANSACTION_SPANS | CLOSED_WORLD_TEMP;
}

/// A diagnostic emitted when a detector fails — returns `Err`, or (debug builds
/// only) panics — (stage = "detect").
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
/// its count is > 0 — this present-iff-nonzero rule is UNIVERSAL across all detectors
/// (d43 was normalized to it too; its golden shows `{}`). An empty map serializes as `{}`.
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
    /// Skip counters. Insert via the `add_skip` method (present-iff-nonzero). Keys must
    /// match the taxonomy exactly.
    pub skipped: std::collections::BTreeMap<String, u64>,
}

impl DetectorStats {
    /// Create a new `DetectorStats` with an empty skipped map.
    pub fn new(
        detector: impl Into<String>,
        candidates_considered: usize,
        findings_emitted: usize,
    ) -> Self {
        Self {
            detector: detector.into(),
            candidates_considered,
            findings_emitted,
            skipped: std::collections::BTreeMap::new(),
        }
    }

    /// Add `n` to a skip counter, inserting it if absent. Only inserts when `n > 0`.
    pub fn add_skip(&mut self, key: &str, n: u64) {
        if n > 0 {
            *self.skipped.entry(key.to_string()).or_insert(0) += n;
        }
    }

    /// Serialize this stats object to a `serde_json::Value` with alphabetically-sorted
    /// keys, matching the al-sem `sortedReplacer` output. The field order is the
    /// alphabetical key sort: `candidatesConsidered`, `detector`, `findingsEmitted`, `skipped`.
    ///
    /// Correctness does NOT depend on the insertion order below: `serde_json::Map` is
    /// `BTreeMap`-backed (this crate does not enable the `preserve_order` feature), so it
    /// sorts keys automatically on serialization. If a future maintainer enables
    /// `preserve_order` (making `Map` an `IndexMap`), THIS code would then emit keys in
    /// insertion order — and would need explicit alphabetical insertion (which it already
    /// happens to do) plus the `skipped_obj` map to be sorted too.
    pub fn to_json_value(&self) -> serde_json::Value {
        let skipped_obj: serde_json::Map<String, serde_json::Value> = self
            .skipped
            .iter()
            .map(|(k, v)| (k.clone(), serde_json::Value::Number((*v).into())))
            .collect();
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
///
/// Delegates to `format_json::serialize_document_value` (the single canonical
/// serializer) so stats and envelopes are always byte-consistent.
pub fn serialize_detector_stats(stats: &[DetectorStats]) -> String {
    let arr: Vec<serde_json::Value> = stats.iter().map(|s| s.to_json_value()).collect();
    let val = serde_json::Value::Array(arr);
    crate::engine::gate::format_json::serialize_document_value(val)
}

/// A detector's output.
///
/// Most detectors construct this as `DetectorOutput { findings, stats }` (no diagnostics).
/// The `diagnostics` field defaults to `vec![]` via the `Default` partial support:
/// use `DetectorOutput { findings, stats, ..DetectorOutput::empty() }` or the
/// two-field shorthand `{ findings, stats }` — BUT note the struct is not `Default`
/// (requires `Finding`/`DetectorStats` Default impls). For detectors that emit
/// diagnostics (e.g. d43 substrate guard), populate the field explicitly.
pub struct DetectorOutput {
    pub findings: Vec<Finding>,
    pub stats: DetectorStats,
    /// Non-panic diagnostics emitted by the detector (e.g. d43 substrate guard warning).
    /// Propagates to `RunOutput.diagnostics` and thence to the JSON envelope.
    /// Omit in struct literals when empty — existing `{ findings, stats }` constructions
    /// must be updated to `{ findings, stats, diagnostics: vec![] }`. The helper
    /// `DetectorOutput::no_diag(findings, stats)` reduces boilerplate for detectors
    /// that never emit diagnostics.
    pub diagnostics: Vec<Diagnostic>,
}

impl DetectorOutput {
    /// Convenience constructor for the common case: no diagnostics.
    pub fn no_diag(findings: Vec<Finding>, stats: DetectorStats) -> Self {
        DetectorOutput {
            findings,
            stats,
            diagnostics: vec![],
        }
    }
}

/// A recoverable detector failure. THE isolation contract (see `run_detectors`):
/// every detector returns `Result<DetectorOutput, DetectorError>`, and `run_each`
/// turns an `Err` into the `Detector "<name>" threw: <msg>` warning diagnostic —
/// the SAME message format a caught panic produces, so callers cannot tell the two
/// apart. `Display` supplies `<msg>`.
#[derive(Debug, Clone)]
pub struct DetectorError(String);

impl DetectorError {
    pub fn new(msg: impl Into<String>) -> Self {
        DetectorError(msg.into())
    }
}

impl std::fmt::Display for DetectorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for DetectorError {}

/// A detector: a pure query over the resolved model + shared context. The closure
/// receives `(resolved, ctx)`; al-sem also passes the combined graph, which the
/// ctx already carries (`ctx.graph`), so detectors read it from there.
///
/// Returns `Result` — this IS the isolation contract (see `run_detectors`): an
/// `Err` degrades to a warning diagnostic while every other detector still runs,
/// and unlike `catch_unwind` this works identically under `panic = "abort"`
/// (`[profile.release]`, Cargo.toml), the profile every shipped binary uses.
pub struct Detector {
    pub name: String,
    pub run: fn(&L3Resolved, &DetectorContext) -> Result<DetectorOutput, DetectorError>,
    /// The substrate bits (see `substrate`) this detector reads from the context.
    /// `run_detectors` folds every selected detector's `requires` into the union it
    /// hands to `build_detector_context`, so a substrate is built iff some selected
    /// detector demands it. Over-inclusive is SAFE (just less skipping);
    /// under-inclusive is caught by the full-vs-minimal parity test.
    pub requires: u32,
}

/// The combined output of `run_detectors`.
pub struct RunOutput {
    pub findings: Vec<Finding>,
    pub diagnostics: Vec<Diagnostic>,
    pub detector_stats: Vec<DetectorStats>,
    /// The L4 "summarizeDiagnostics" source (TS-order slot 3 — see
    /// `gate/run.rs`'s `compute_analyzer_diagnostics` doc) — presently just the
    /// JACOBI fixed-point cap-hit. Kept SEPARATE from `diagnostics` (the
    /// detect-stage source, slot 6) so the gate boundary can place each in its
    /// documented TS-concat position rather than collapsing both into "detect".
    /// Empty whenever every SCC converges (additive).
    pub summarize_diagnostics: Vec<Diagnostic>,
}

/// Convert an L4 `SummarizeDiagnostic` into the shared `l5::registry::Diagnostic`
/// shape — the same seam-conversion `gate/run.rs` already does for
/// `root_classification::InfraDiagnostic`.
fn from_summarize_diagnostic(
    d: &crate::engine::l4::summary_runner::SummarizeDiagnostic,
) -> Diagnostic {
    Diagnostic {
        severity: d.severity.clone(),
        stage: d.stage.clone(),
        message: d.message.clone(),
    }
}

/// `primaryLocationKey(f) = ${sourceUnitId}:${startLine}:${startColumn}` over the
/// INTERNAL anchor.
fn primary_location_key(f: &Finding) -> String {
    let a = &f.primary_location;
    format!("{}:{}:{}", a.source_unit_id, a.start_line, a.start_column)
}

/// Run every registered detector in isolation, then role-scope + sort.
///
/// Isolation is a `Result` CONTRACT (see `Detector::run` / `run_each`): every
/// detector returns `Result<DetectorOutput, DetectorError>`, and an `Err` becomes a
/// `Diagnostic(stage: "detect")` while the rest still run. This holds under BOTH
/// panic=unwind (`cargo test`) and the shipped `[profile.release] panic = "abort"`
/// (Cargo.toml) — it never depends on unwinding. `run_each` ALSO wraps each call in
/// `catch_unwind` as debug-build-only defense-in-depth (a detector that panics
/// despite the contract still degrades to the identical diagnostic under
/// panic=unwind); that wrapper is INERT in an abort release binary — `catch_unwind`
/// never catches anything there — so it must never be relied on as the real
/// guarantee.
pub fn run_detectors(resolved: &L3Resolved, detectors: &[Detector]) -> RunOutput {
    // W1.0 demand-driven substrate: build only the expensive substrates some selected
    // detector actually reads. A full/preset/all-detector selection unions to
    // `substrate::ALL`, so the context — and the whole report — stays byte-identical.
    let demanded = detectors.iter().fold(0u32, |acc, d| acc | d.requires);
    let ctx = build_detector_context(resolved, demanded);
    crate::stage_probe::stage("l4_detector_context:end");
    let summarize_diagnostics: Vec<Diagnostic> = ctx
        .summarize_diagnostics
        .iter()
        .map(from_summarize_diagnostic)
        .collect();
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
        summarize_diagnostics,
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
    let summarize_diagnostics: Vec<Diagnostic> = ctx
        .summarize_diagnostics
        .iter()
        .map(from_summarize_diagnostic)
        .collect();
    // The detectors close over `(resolved, ctx)`. Build a throwaway L3Resolved view
    // over the merged routines so the `resolved.workspace` arg is consistent with the
    // ctx (detectors read `resolved.workspace.routines`/`.objects` for the fingerprint
    // index + role map; for d13/d16/d17 those are the merged sets in `base`).
    let resolved = L3Resolved {
        workspace: merged_workspace_view(base),
        root_classifications: Vec::new(),
        primary_app: None,
        infra_diagnostics: Vec::new(),
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
        summarize_diagnostics,
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

/// Run each detector in isolation via the `Result` contract (see `run_detectors`'s
/// doc comment for the full guarantee), collecting findings + stats.
fn run_each(
    resolved: &L3Resolved,
    ctx: &DetectorContext,
    detectors: &[Detector],
) -> (Vec<Finding>, Vec<Diagnostic>, Vec<DetectorStats>) {
    let mut findings: Vec<Finding> = Vec::new();
    let mut diagnostics: Vec<Diagnostic> = Vec::new();
    let mut detector_stats: Vec<DetectorStats> = Vec::new();

    for detector in detectors {
        // `catch_unwind` here is debug-build-only defense-in-depth (see the
        // `run_detectors` doc comment) — it is INERT under `panic = "abort"`. The
        // real, abort-safe isolation is the `Result` returned by `detector.run`
        // itself, handled in the `Ok(Err(e))` arm below with the identical
        // diagnostic shape a caught panic produces.
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            (detector.run)(resolved, ctx)
        }));
        crate::stage_probe::stage(&format!("detector:{}:end", detector.name));
        match outcome {
            Ok(Ok(output)) => {
                findings.extend(output.findings);
                // Collect detector-emitted diagnostics (non-error; d43 substrate guard etc.)
                diagnostics.extend(output.diagnostics);
                detector_stats.push(output.stats);
            }
            Ok(Err(e)) => {
                diagnostics.push(Diagnostic {
                    severity: "warning".to_string(),
                    stage: "detect".to_string(),
                    message: format!("Detector \"{}\" threw: {e}", detector.name),
                });
            }
            Err(panic_payload) => {
                let msg = panic_payload
                    .downcast_ref::<&str>()
                    .map(|s| s.to_string())
                    .or_else(|| panic_payload.downcast_ref::<String>().cloned())
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
    use crate::engine::l3::l3_workspace::L3Workspace;
    use crate::engine::l5::finding::{Finding, FindingConfidence, SourceAnchor};
    use std::cmp::Ordering;

    fn empty_resolved() -> L3Resolved {
        L3Resolved {
            workspace: L3Workspace {
                objects: vec![],
                tables: vec![],
                routines: vec![],
            },
            root_classifications: vec![],
            primary_app: None,
            infra_diagnostics: vec![],
        }
    }

    fn test_finding(id: &str) -> Finding {
        Finding {
            id: id.to_string(),
            root_cause_key: id.to_string(),
            detector: "test-ok-detector".to_string(),
            title: "test finding".to_string(),
            root_cause: "test root cause".to_string(),
            severity: "info".to_string(),
            confidence: FindingConfidence {
                level: "likely".to_string(),
                capped_by: None,
                evidence: vec![],
            },
            primary_location: SourceAnchor {
                source_unit_id: "u0".to_string(),
                start_line: 1,
                start_column: 1,
                end_line: 1,
                end_column: 1,
                enclosing_routine_id: "r0".to_string(),
                syntax_kind: "call".to_string(),
                normalized_text_hash: None,
                leading_context_hash: None,
                trailing_context_hash: None,
            },
            evidence_path: vec![],
            additional_paths: None,
            affected_objects: vec![],
            affected_tables: vec![],
            fix_options: vec![],
            provenance: vec![],
            actionable_anchor: None,
            fingerprint: None,
            event_kind: None,
            cross_extension_subscribers: None,
        }
    }

    fn ok_detector(
        _resolved: &L3Resolved,
        _ctx: &DetectorContext,
    ) -> Result<DetectorOutput, DetectorError> {
        Ok(DetectorOutput::no_diag(
            vec![test_finding("ok-detector-finding")],
            DetectorStats::new("test-ok-detector", 1, 1),
        ))
    }

    /// THE MISSING TEST (T2.3): a detector that returns `Err` — the abort-safe
    /// isolation path — degrades to the exact warning-diagnostic format while every
    /// other registered detector still runs to completion.
    fn err_detector(
        _resolved: &L3Resolved,
        _ctx: &DetectorContext,
    ) -> Result<DetectorOutput, DetectorError> {
        Err(DetectorError::new("boom"))
    }

    /// A detector that panics despite the `Result` contract. Only reachable via the
    /// debug-build-only `catch_unwind` backstop (see `run_each`) — this test runs
    /// under `cargo test`, which unwinds (unlike the shipped `panic = "abort"`
    /// release profile), so it exercises that backstop specifically.
    fn panic_detector(
        _resolved: &L3Resolved,
        _ctx: &DetectorContext,
    ) -> Result<DetectorOutput, DetectorError> {
        panic!("boom-panic");
    }

    #[test]
    fn err_returning_detector_degrades_to_warning_others_still_run() {
        let resolved = empty_resolved();
        let detectors = vec![
            Detector {
                name: "d-ok".to_string(),
                run: ok_detector,
                requires: substrate::ALL,
            },
            Detector {
                name: "d-err".to_string(),
                run: err_detector,
                requires: substrate::ALL,
            },
        ];
        let out = run_detectors(&resolved, &detectors);

        assert_eq!(
            out.findings.len(),
            1,
            "the ok detector's finding must still appear"
        );
        assert_eq!(out.findings[0].id, "ok-detector-finding");

        assert_eq!(out.diagnostics.len(), 1);
        assert_eq!(out.diagnostics[0].severity, "warning");
        assert_eq!(out.diagnostics[0].stage, "detect");
        assert_eq!(
            out.diagnostics[0].message, "Detector \"d-err\" threw: boom",
            "exact message format is relied on by consumers (may be golden-pinned)"
        );

        assert_eq!(
            out.detector_stats.len(),
            1,
            "the failing detector never reaches the stats-push line"
        );
    }

    #[test]
    fn panicking_detector_degrades_to_warning_others_still_run() {
        let resolved = empty_resolved();
        let detectors = vec![
            Detector {
                name: "d-ok".to_string(),
                run: ok_detector,
                requires: substrate::ALL,
            },
            Detector {
                name: "d-panic".to_string(),
                run: panic_detector,
                requires: substrate::ALL,
            },
        ];
        let out = run_detectors(&resolved, &detectors);

        assert_eq!(
            out.findings.len(),
            1,
            "the ok detector's finding must still appear"
        );
        assert_eq!(out.findings[0].id, "ok-detector-finding");

        assert_eq!(out.diagnostics.len(), 1);
        assert_eq!(out.diagnostics[0].severity, "warning");
        assert_eq!(out.diagnostics[0].stage, "detect");
        assert_eq!(
            out.diagnostics[0].message, "Detector \"d-panic\" threw: boom-panic",
            "a caught panic and a returned Err must produce the IDENTICAL message shape"
        );
    }

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
