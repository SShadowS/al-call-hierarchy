//! Phase 0 + Phase 1: span-based site matcher fixture matrix (no env needed) +
//! Phase-1 Task-4 CDO gate (env-gated).
//!
//! Exercises [`match_sites`] — the cascade-resistance spine of the dual-run
//! differential harness.  All tests construct synthetic edges via
//! [`canonical_call_edge_for_test`] so no real workspace is required.

use al_call_hierarchy::program::resolve::differential::{
    DiffReport, ResolutionReport, SiteMatch, canonical_call_edge_for_test, match_sites,
    run_harness, run_resolution_harness, run_site_harness,
};

// ---------------------------------------------------------------------------
// Test 1 (from brief): one missing L3 site must NOT cascade
// ---------------------------------------------------------------------------

/// Verifies the core cascade-resistance guarantee: when the L3 oracle is
/// missing exactly one site that the fresh side emits, that site becomes a
/// single `FreshOnly` and all other pairings are undisturbed.
#[test]
fn one_missing_site_does_not_cascade() {
    // Build 5 fresh sites at increasing spans; L3 has the same 5 minus the 2nd.
    let mk = |start: u32, fp: u64| canonical_call_edge_for_test("cu:c:run", start, fp);
    let fresh = vec![mk(10, 1), mk(20, 2), mk(30, 3), mk(40, 4), mk(50, 5)];
    let l3 = vec![mk(10, 1), mk(30, 3), mk(40, 4), mk(50, 5)];
    let matches = match_sites(&fresh, &l3);
    let paired = matches
        .iter()
        .filter(|m| matches!(m, SiteMatch::Paired(_, _)))
        .count();
    let fresh_only = matches
        .iter()
        .filter(|m| matches!(m, SiteMatch::FreshOnly(_)))
        .count();
    let l3_only = matches
        .iter()
        .filter(|m| matches!(m, SiteMatch::L3Only(_)))
        .count();
    let unaligned = matches
        .iter()
        .filter(|m| matches!(m, SiteMatch::Unaligned(_, _)))
        .count();
    // 4 clean pairs; the 2nd fresh site is the single FreshOnly; NO cascade on 3/4/5.
    assert_eq!(paired, 4, "matches: {matches:?}");
    assert_eq!(fresh_only, 1);
    assert_eq!(
        matches.len(),
        5,
        "every site must be in exactly one bucket: {matches:?}"
    );
    assert_eq!(l3_only, 0, "no L3-only sites in this test");
    assert_eq!(unaligned, 0, "no unaligned duplicates in this test");
}

// ---------------------------------------------------------------------------
// Test 2: duplicate calls on the same line pair cleanly
// ---------------------------------------------------------------------------

/// When two fresh sites and two L3 sites share the same strong key
/// `(unit, start_line, callee_fp)` (e.g. identical back-to-back calls on one
/// line), the matcher pairs them positionally — 2 `Paired`, no `Unaligned`.
#[test]
fn duplicate_calls_on_same_line_pair_cleanly() {
    let mk = |start: u32, fp: u64| canonical_call_edge_for_test("cu:c:run", start, fp);
    // Two identical sites in both fresh and L3.
    let fresh = vec![mk(10, 1), mk(10, 1)];
    let l3 = vec![mk(10, 1), mk(10, 1)];
    let matches = match_sites(&fresh, &l3);
    let paired = matches
        .iter()
        .filter(|m| matches!(m, SiteMatch::Paired(_, _)))
        .count();
    let unaligned = matches
        .iter()
        .filter(|m| matches!(m, SiteMatch::Unaligned(_, _)))
        .count();
    assert_eq!(paired, 2, "matches: {matches:?}");
    assert_eq!(
        unaligned, 0,
        "equal-count duplicates must not produce Unaligned"
    );
}

// ---------------------------------------------------------------------------
// Test 3: FreshOnly in a different (from,kind) group does not cascade
// ---------------------------------------------------------------------------

/// A fresh site whose caller has NO L3 peer at all (different `from` key →
/// different partition) is emitted as `FreshOnly`.  The two other sites from
/// the first caller still pair cleanly — proving that one partition's
/// mismatch is invisible to another partition.
#[test]
fn fresh_only_different_caller_does_not_cascade() {
    let mk = |caller: &str, start: u32, fp: u64| canonical_call_edge_for_test(caller, start, fp);
    let fresh = vec![
        mk("cu:c:run", 10, 1),
        mk("cu:c:run", 20, 2),
        mk("cu:c:post", 10, 1), // different caller — no L3 peer
    ];
    let l3 = vec![mk("cu:c:run", 10, 1), mk("cu:c:run", 20, 2)];
    let matches = match_sites(&fresh, &l3);
    let paired = matches
        .iter()
        .filter(|m| matches!(m, SiteMatch::Paired(_, _)))
        .count();
    let fresh_only = matches
        .iter()
        .filter(|m| matches!(m, SiteMatch::FreshOnly(_)))
        .count();
    let l3_only = matches
        .iter()
        .filter(|m| matches!(m, SiteMatch::L3Only(_)))
        .count();
    // 2 clean pairs in "cu:c:run"; 1 FreshOnly in "cu:c:post"; no L3Only.
    assert_eq!(paired, 2, "matches: {matches:?}");
    assert_eq!(fresh_only, 1, "the cu:c:post site has no L3 peer");
    assert_eq!(l3_only, 0);
}

// ---------------------------------------------------------------------------
// Test 4 (Phase 0 Task 7 CDO gate): end-to-end dual-run differential harness
// ---------------------------------------------------------------------------

/// End-to-end CDO gate: verifies that the dual-run differential harness wires
/// the full pipeline (snapshot → graph → fresh resolve → canonical projection
/// → L3 oracle projection → span matcher → diff buckets) and that:
///
/// 1. Both projections produce >1000 edges (real data, not empty).
/// 2. The site matcher aligns >1000 sites between the two projections,
///    proving that Tasks 4–6 key encodings agree on real data.
/// 3. Phase-0 stub resolves nothing → every matched site is a regression.
/// 4. UNALIGNED is <5% of the site population (matcher is stable).
/// 5. Two consecutive runs produce identical output (determinism).
///
/// Guards: requires `CDO_WS` env var pointing at a real BC workspace with
/// `.alpackages` deps.  Plain `cargo test` skips this automatically.
#[test]
fn harness_runs_end_to_end_on_cdo_and_measures_the_gap() {
    let Some(ws) = std::env::var_os("CDO_WS")
        .map(std::path::PathBuf::from)
        .filter(|p| p.exists())
    else {
        return;
    };

    let report = run_harness(&ws);

    // Print the full breakdown so `-- --nocapture` shows the real numbers.
    eprintln!(
        "DiffReport: fresh_total_all_apps={} fresh_total_workspace={} l3_edges={} \
         matched={} regression={} missing_site={} extra_site={} unaligned={}",
        report.fresh_total_all_apps,
        report.fresh_total_workspace,
        report.l3_edges,
        report.matched,
        report.regression,
        report.missing_site,
        report.extra_site,
        report.unaligned,
    );

    // L3 oracle has many edges; fresh stub extracted many workspace sites.
    assert!(report.l3_edges > 1000, "{report:?}");
    assert!(report.fresh_total_workspace > 1000, "{report:?}");

    // Pipeline health: the strong site-key must match thousands of sites,
    // proving that Tasks 4–6 (fresh keys / L3 keys / span matcher) all agree
    // on real data.  A value near 0 means a key encoding mismatch to fix.
    assert!(
        report.matched > 1000,
        "keys must align across the two projections: {report:?}"
    );

    // Phase-0 baseline: stub fresh resolves nothing (all empty targets), so
    // regression sites (fresh-empty AND l3-non-empty) are a subset of matched.
    assert!(
        report.regression <= report.matched,
        "regression is a subset of matched (fresh-empty AND l3-non-empty paired sites): {report:?}"
    );

    // Alignment quality: UNALIGNED must be <5% of the site population.
    let denom = (report.matched + report.missing_site + report.extra_site).max(1);
    assert!(
        report.unaligned * 20 < denom,
        "UNALIGNED must be <5%: {} of {}",
        report.unaligned,
        denom
    );

    // Determinism: two consecutive runs must produce identical output.
    assert_eq!(
        report,
        run_harness(&ws),
        "run_harness must be deterministic"
    );
}

// ---------------------------------------------------------------------------
// Test 5 (Phase 1 Task 4): L3 PCallSite projection + site-parity gate
// ---------------------------------------------------------------------------

/// Phase-1 site-parity gate: proves that fresh's STRUCTURED call-site
/// classification (via `extract_sites`/`CalleeShape`) reconciles with the L3
/// PCallSite oracle.
///
/// Every fresh call-category site (Bare/Member/ObjectRun/Unknown) must either
/// match an L3 PCallSite or be counted in a named justified-extra bucket
/// (RecordOp / Commit / implicit-Rec bare).  `extra_unexplained` MUST be 0.
///
/// Guards: requires `CDO_WS` env var pointing at a real BC workspace.
#[test]
fn phase1_site_extraction_reconciles_with_l3() {
    let Some(ws) = std::env::var_os("CDO_WS")
        .map(std::path::PathBuf::from)
        .filter(|p| p.exists())
    else {
        return;
    };

    let report = run_site_harness(&ws);

    eprintln!(
        "SiteReport: matched={} missing_site={} extra_recordop={} extra_commit={} \
         extra_implicit_rec={} extra_error={} extra_unexplained={} unaligned={}",
        report.matched,
        report.missing_site,
        report.extra_recordop,
        report.extra_commit,
        report.extra_implicit_rec,
        report.extra_error,
        report.extra_unexplained,
        report.unaligned,
    );

    assert_eq!(report.missing_site, 0, "{report:?}");
    assert_eq!(report.unaligned, 0, "{report:?}");
    assert_eq!(
        report.extra_unexplained, 0,
        "every fresh call-category site must reconcile with an L3 PCallSite or be \
         categorized: {report:?}"
    );
    assert!(
        report.extra_recordop > 0,
        "record-op sites should be a large justified-extra bucket: {report:?}"
    );
    // Determinism: two consecutive runs must produce identical output.
    assert_eq!(report, run_site_harness(&ws), "deterministic");
}

// ---------------------------------------------------------------------------
// Test 6 (Phase 2 Task 6): Phase-2 Bare/Run resolution gate vs L3 oracle
// ---------------------------------------------------------------------------

/// Phase-2 resolution gate: proves that the real `resolve_bare` / `resolve_object_run`
/// path matches or beats the L3 oracle on CDO for in-scope (Bare + ObjectRun)
/// call sites.
///
/// Three zero-tolerance assertions (gates that must pass before commit):
/// - `regression_unexplained == 0`: fresh must not lose a Bare/Run target that
///   L3 resolved (implicit-Rec deferrals are tracked separately as
///   `regression_implicit_rec`, which is informational).
/// - `evidence_overclaim == 0`: no route may claim Source/Abi/Catalog evidence
///   without the corresponding valid witness.
/// - `unverified_extra == 0`: fresh must not produce non-empty targets on a
///   FreshOnly site (a site with no matching L3 peer).
///
/// Also verifies determinism by running twice and comparing.
///
/// Guards: requires `CDO_WS` env var pointing at a real BC workspace.
#[test]
fn phase2_bare_run_resolution_matches_or_beats_l3() {
    let Some(ws) = std::env::var_os("CDO_WS")
        .map(std::path::PathBuf::from)
        .filter(|p| p.exists())
    else {
        return;
    };

    let report = run_resolution_harness(&ws);

    eprintln!(
        "ResolutionReport: matched={} regression_unexplained={} regression_implicit_rec={} \
         regression_cross_app={} evidence_overclaim={} unverified_extra={} \
         verified_win={} divergence={} \
         missing_site={} extra_site={} unaligned={} \
         fresh_total={} l3_total={} \
         fresh_unknown={} fresh_resolved={} ({:.1}% unknown) \
         l3_unknown={} l3_resolved={} ({:.1}% unknown)",
        report.matched,
        report.regression_unexplained,
        report.regression_implicit_rec,
        report.regression_cross_app,
        report.evidence_overclaim,
        report.unverified_extra,
        report.verified_win,
        report.divergence,
        report.missing_site,
        report.extra_site,
        report.unaligned,
        report.fresh_total,
        report.l3_total,
        report.fresh_unknown_count,
        report.fresh_resolved_count,
        if report.fresh_total > 0 {
            report.fresh_unknown_count as f64 / report.fresh_total as f64 * 100.0
        } else {
            0.0
        },
        report.l3_unknown_count,
        report.l3_resolved_count,
        if report.l3_total > 0 {
            report.l3_unknown_count as f64 / report.l3_total as f64 * 100.0
        } else {
            0.0
        },
    );

    assert_eq!(
        report.regression_unexplained, 0,
        "fresh must not lose a Bare/Run target L3 resolved \
         (excl. known implicit-Rec deferral): {report:?}"
    );
    assert_eq!(
        report.evidence_overclaim, 0,
        "no Source/Abi/Catalog claim without a valid witness: {report:?}"
    );
    assert_eq!(
        report.unverified_extra, 0,
        "no unwitnessed new non-dynamic edge: {report:?}"
    );

    // Determinism: two consecutive runs must produce identical output.
    assert_eq!(report, run_resolution_harness(&ws), "deterministic");
}

// Suppress unused-import warning when CDO_WS is not set (no CDO test runs).
#[allow(dead_code)]
fn _assert_diff_report_importable(_: DiffReport) {}

#[allow(dead_code)]
fn _assert_resolution_report_importable(_: ResolutionReport) {}
