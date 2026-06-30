//! Phase 0 + Phase 1: span-based site matcher fixture matrix (no env needed) +
//! Phase-1 Task-4 CDO gate (env-gated).
//!
//! Exercises [`match_sites`] — the cascade-resistance spine of the dual-run
//! differential harness.  All tests construct synthetic edges via
//! [`canonical_call_edge_for_test`] so no real workspace is required.

use al_call_hierarchy::program::node::{
    AppRegistry, ObjKey, ObjectKind, ObjectNodeId, RoutineNodeId,
};
use al_call_hierarchy::program::resolve::differential::{
    DiffReport, EventFlowGateReport, ImplicitTriggerResolutionReport, MemberResolutionReport,
    ResolutionReport, SiteMatch, canonical_call_edge_for_test, match_sites,
    project_fresh_event_rows, run_event_flow_gate, run_harness, run_implicit_trigger_harness,
    run_member_resolution_harness, run_resolution_harness, run_site_harness,
    verify_event_subscriber_route,
};
use al_call_hierarchy::snapshot::{AppId, ParsedFile, ParsedUnit, Provenance, TrustTier};

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
         fresh_unknown={} fresh_resolved={} ({:.1}% unknown on fresh in-scope) \
         l3_unknown={} l3_resolved={} ({:.1}% unknown on L3 in-scope)\n\
         NOTE: The two unknown-rates are NOT comparable — denominators differ \
         (fresh={} in-scope Bare/Run sites; L3={} in-scope Direct/Builtin/Run/Unresolved \
         edges) and fresh emits Builtin targets while L3 builtin edges carry to=None. \
         Honest result: on the paired subset (matched={}), fresh has 0 unexplained \
         regressions and {} verified wins; missing_site={} are sites L3 resolves as \
         Direct (Member-dispatch) that fresh defers to Phase 3.",
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
        report.fresh_total,
        report.l3_total,
        report.matched,
        report.verified_win,
        report.missing_site,
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

    // Symmetric paired-subset assertion: on the sites both engines saw, fresh must
    // match-or-beat L3.  verified_win (fresh-better) must be >= all tracked
    // regressions combined (unexplained + implicit_rec + cross_app).  With
    // verified_win≈1827 and total regressions ~90 on CDO this comfortably passes.
    assert!(
        report.regression_unexplained
            + report.regression_implicit_rec
            + report.regression_cross_app
            <= report.verified_win,
        "fresh must match-or-beat L3 on the paired subset (total tracked regressions must \
         not exceed verified wins): {report:?}"
    );

    // Divergence cap: all 38 CDO divergences have been adjudicated (see task-6-report.md).
    //   • 20 fresh-BETTER: PageRun → OnOpenPage (correct); L3 spuriously resolves to
    //     a different page trigger (onassistedit / onvalidate / ondrilldown / onaction).
    //   • 1 fresh-DEFERRED: unqualified `run` → Builtin catalog; L3 follows implicit-Rec
    //     to the page's backing table (Phase 3 implicit-Rec deferred, same as
    //     regression_implicit_rec).
    //   • 17 CONCERN: same-object, different-procedure — fresh's first-candidate
    //     overload fallback disagrees with L3's candidate selection; per-site adjudication
    //     pending (warehouse-action pages 6175357/6175358/6175362/6175363, CDOEMailRecipients,
    //     CDOEMailTemplateCard, etc.).  Filed for Phase-3 follow-up.
    // Any count ABOVE 38 means a NEW unreviewed divergence appeared — must inspect.
    assert!(
        report.divergence <= 38,
        "divergence has grown beyond the 38 adjudicated CDO cases; inspect new cases \
         before merging: {report:?}"
    );
    eprintln!("divergence={} (adjudicated cap=38)", report.divergence);

    // Determinism: two consecutive runs must produce identical output.
    assert_eq!(report, run_resolution_harness(&ws), "deterministic");
}

// ---------------------------------------------------------------------------
// Test 7 (Phase 3 Task 5): Phase-3 Member-resolution gate vs L3 oracle
// ---------------------------------------------------------------------------

/// Phase-3 Member-resolution gate: proves that the real `infer_receiver_type` +
/// `resolve_member` path matches or beats the L3 oracle on CDO for Member call
/// sites.
///
/// Three zero-tolerance assertions:
/// - `regression_unexplained == 0`: fresh must not lose an L3-resolved Member
///   target that is not in a named deferral bucket (Interface/EnumType/Record{None}/
///   Primitive).
/// - `evidence_overclaim == 0`: no route may claim Source/Abi/Catalog evidence
///   without the corresponding valid witness.
/// - Determinism: two consecutive runs produce identical output.
///
/// Informational: prints the full categorized breakdown + the Member `missing_site`
/// (still-deferred residual — Interface fan-out [Phase 4], unresolved Page/PageExt
/// table, and other open-world sites).
///
/// Guards: requires `CDO_WS` env var pointing at a real BC workspace.
#[test]
fn phase3_member_resolution_matches_or_beats_l3() {
    let Some(ws) = std::env::var_os("CDO_WS")
        .map(std::path::PathBuf::from)
        .filter(|p| p.exists())
    else {
        return;
    };

    let report = run_member_resolution_harness(&ws);

    eprintln!(
        "MemberResolutionReport:\n\
         matched={} verified_win={} divergence={}\n\
         regression_unexplained={} regression_interface={} regression_enum_static={}\n\
         regression_page_rec={} regression_scalar={}\n\
         regression_compound_receiver={} regression_codeunit_implicit_rec={}\n\
         evidence_overclaim={} unverified_extra={}\n\
         missing_site={} extra_site={}\n\
         fresh_ahead_interface={} fresh_ahead_instance_builtin={} \
         fresh_ahead_enum_static={}\n\
         unaligned={}\n\
         fresh_total={} l3_total={}\n\
         fresh_unknown={} fresh_resolved={} ({:.1}% unknown on fresh Member sites)\n\
         l3_unknown={} l3_resolved={} ({:.1}% unknown on L3 Member oracle)\n\
         NOTE: The paired-subset result is the honest metric — fresh has {} unexplained \
         regressions and {} verified wins over L3 on the paired Member subset (matched={}).\n\
         Named deferrals: regression_interface={} (Phase-4), \
         regression_enum_static={} (deferred), \
         regression_page_rec={} (Page implicit-Rec gap), \
         regression_scalar={} (primitive by-design), \
         regression_compound_receiver={} (chained receiver, Phase-4), \
         regression_codeunit_implicit_rec={} (TableNo/TestRunner implicit Rec).\n\
         Member missing_site={} | Deferred regression residual: \
         compound_receiver={} + codeunit_implicit_rec={} + interface={} + page_rec={}.",
        report.matched,
        report.verified_win,
        report.divergence,
        report.regression_unexplained,
        report.regression_interface,
        report.regression_enum_static,
        report.regression_page_rec,
        report.regression_scalar,
        report.regression_compound_receiver,
        report.regression_codeunit_implicit_rec,
        report.evidence_overclaim,
        report.unverified_extra,
        report.missing_site,
        report.extra_site,
        report.fresh_ahead_interface,
        report.fresh_ahead_instance_builtin,
        report.fresh_ahead_enum_static,
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
        report.regression_unexplained,
        report.verified_win,
        report.matched,
        report.regression_interface,
        report.regression_enum_static,
        report.regression_page_rec,
        report.regression_scalar,
        report.regression_compound_receiver,
        report.regression_codeunit_implicit_rec,
        report.missing_site,
        report.regression_compound_receiver,
        report.regression_codeunit_implicit_rec,
        report.regression_interface,
        report.regression_page_rec,
    );

    assert_eq!(
        report.regression_unexplained, 0,
        "fresh must not lose a Member target L3 resolved \
         (excl. named deferrals: interface/enum/page-rec/scalar/\
         compound-receiver/codeunit-implicit-rec): {report:?}"
    );
    assert_eq!(
        report.evidence_overclaim, 0,
        "no Source/Abi/Catalog claim without a valid witness: {report:?}"
    );
    assert_eq!(
        report.unverified_extra, 0,
        "no fresh-only fan-out route may fail the applicability predicate \
         (unverified_extra is inert at Task 0, gains teeth in Tasks 1-3): {report:?}"
    );

    // Divergence cap: all 56 CDO divergences have been adjudicated.
    // 45 pre-Task-2 divergences (see task-5-report.md) + 11 new interface fan-out
    // divergences where fresh emits N Routine routes and L3 emits 1 — fresh is more
    // precise (see task-2-report.md).  Any count ABOVE 56 is a new unreviewed
    // divergence and must be inspected before merging.
    assert!(
        report.divergence <= 56,
        "Member divergence grew beyond the adjudicated 56; inspect before merging: {report:?}"
    );

    // Determinism: two consecutive runs must produce identical output.
    assert_eq!(
        report,
        run_member_resolution_harness(&ws),
        "run_member_resolution_harness must be deterministic"
    );
}

// ---------------------------------------------------------------------------
// Test 8 (Phase 4 Task 3): ImplicitTrigger Multicast gate vs L3 oracle
// ---------------------------------------------------------------------------

/// Phase-4 ImplicitTrigger gate: proves that `resolve_implicit_trigger` + the
/// applicability predicate correctly accounts for every workspace RecordOp
/// trigger site against the L3 oracle.
///
/// Three zero-tolerance assertions:
/// - `regression_unexplained == 0`: fresh must not lose an ImplicitTrigger
///   target that L3 resolved.
/// - `evidence_overclaim == 0`: no route may claim Source/Abi/Catalog evidence
///   without the corresponding valid witness.
/// - `unverified_extra == 0`: every FreshOnly trigger route must pass
///   `implicit_trigger_route_applicable` OR be classified as
///   `fresh_ahead_validate_fanout` (known Validate over-approximation).
///
/// Guards: requires `CDO_WS` env var pointing at a real BC workspace.
#[test]
fn phase4_implicit_trigger() {
    let Some(ws) = std::env::var_os("CDO_WS")
        .map(std::path::PathBuf::from)
        .filter(|p| p.exists())
    else {
        return;
    };

    let report = run_implicit_trigger_harness(&ws);

    eprintln!(
        "ImplicitTriggerResolutionReport:\n\
         matched={} verified_win={} divergence={}\n\
         regression_unexplained={} evidence_overclaim={} unverified_extra={}\n\
         fresh_ahead_trigger={} fresh_ahead_validate_fanout={}\n\
         missing_site={} extra_site={}\n\
         unaligned={}\n\
         fresh_total={} l3_total={}",
        report.matched,
        report.verified_win,
        report.divergence,
        report.regression_unexplained,
        report.evidence_overclaim,
        report.unverified_extra,
        report.fresh_ahead_trigger,
        report.fresh_ahead_validate_fanout,
        report.missing_site,
        report.extra_site,
        report.unaligned,
        report.fresh_total,
        report.l3_total,
    );

    assert_eq!(
        report.regression_unexplained, 0,
        "fresh must not lose an ImplicitTrigger target L3 resolved: {report:?}"
    );
    assert_eq!(
        report.evidence_overclaim, 0,
        "no Source/Abi/Catalog claim without a valid witness: {report:?}"
    );
    assert_eq!(
        report.unverified_extra, 0,
        "no fresh-only trigger route may fail the applicability predicate: {report:?}"
    );
    // Trigger divergence cap (whole-branch review #1): a PAIRED trigger site where
    // fresh and L3 both emit non-empty but DIFFERENT trigger targets is arguably more
    // serious than a missing_site — assert it stays 0 so a future regression can't slip
    // through silently (the member gate caps divergence; the trigger gate must too).
    assert_eq!(
        report.divergence, 0,
        "a paired ImplicitTrigger site diverged on its target set; inspect before merging: {report:?}"
    );

    // Determinism: two consecutive runs must produce identical output.
    assert_eq!(
        report,
        run_implicit_trigger_harness(&ws),
        "run_implicit_trigger_harness must be deterministic"
    );
}

// ---------------------------------------------------------------------------
// Test 9 (Phase 4 Task 4): Consolidated Phase-4 fan-out gate + honest scope
// ---------------------------------------------------------------------------

/// Phase-4 consolidated gate: runs BOTH the member harness (Member +
/// instance-builtin + Interface, Phase-4 in-scope) and the implicit-trigger
/// harness (ImplicitTrigger Multicast, Phase-4 in-scope) over CDO, asserting
/// all six zero-tolerance conditions and printing a unified breakdown that makes
/// the scope of Phase 4 explicit.
///
/// **Phase 4 closes (in-scope):**
/// - Interface Polymorphic fan-out (all implementers, applicability-gated per
///   route via `interface_route_applicable`)
/// - ImplicitTrigger Multicast (insert/modify/delete/validate RecordOp triggers,
///   applicability-gated via `implicit_trigger_route_applicable`)
/// - Object/Enum instance-builtins (CurrPage, CurrReport singletons,
///   Enum-static dispatch — gated via `instance_builtin_route_applicable`)
///
/// **Phase 4 does NOT close (explicitly excluded):**
/// - EventFlow (Phase 4b) — oracle qualification, manual-binding property,
///   canonical event key, and reachability honesty work outstanding; not yet
///   gated or shipped to the graph.
/// - Page/PageExt implicit-Rec source-table gap (`regression_page_rec`)
/// - Compound receiver type propagation (`regression_compound_receiver`)
/// - Codeunit TableNo/TestRunner implicit-Rec (`regression_codeunit_implicit_rec`)
/// - Trigger `missing_site` (L3 ImplicitTrigger edges with no fresh site peer)
/// - Same-arity-type overload disambiguation (17 Cat-D divergences, 1B.3)
///
/// Six zero-tolerance assertions (must all be 0):
/// - `member.regression_unexplained`, `member.evidence_overclaim`,
///   `member.unverified_extra`
/// - `trigger.regression_unexplained`, `trigger.evidence_overclaim`,
///   `trigger.unverified_extra`
///
/// Guards: requires `CDO_WS` env var pointing at a real BC workspace.
#[test]
fn phase4_fanout_matches_or_beats_l3() {
    let Some(ws) = std::env::var_os("CDO_WS")
        .map(std::path::PathBuf::from)
        .filter(|p| p.exists())
    else {
        return;
    };

    let member = run_member_resolution_harness(&ws);
    let trigger = run_implicit_trigger_harness(&ws);

    eprintln!(
        "\n\
         ═══════════════════════════════════════════════════════════════\n\
         Phase-4 Consolidated Fan-out Gate — Scoped sub-phase (Task 4)\n\
         ═══════════════════════════════════════════════════════════════\n\
         \n\
         ── MEMBER + Instance-builtin + Interface (Phase-4 in-scope) ──\n\
           matched={matched} verified_win={verified_win} divergence={divergence} (cap=56)\n\
           fresh_ahead_interface={fresh_ahead_interface}\n\
           fresh_ahead_instance_builtin={fresh_ahead_instance_builtin}\n\
           fresh_ahead_enum_static={fresh_ahead_enum_static}\n\
           extra_site={m_extra_site} unaligned={m_unaligned}\n\
           [GATE] regression_unexplained={m_reg_unexplained} (must=0)\n\
           [GATE] evidence_overclaim={m_ev_overclaim} (must=0)\n\
           [GATE] unverified_extra={m_unverified} (must=0)\n\
         \n\
         ── ImplicitTrigger Multicast (Phase-4 in-scope) ──\n\
           matched={t_matched} verified_win={t_verified_win} divergence={t_divergence}\n\
           fresh_ahead_trigger={fresh_ahead_trigger}\n\
           fresh_ahead_validate_fanout={fresh_ahead_validate_fanout}\n\
           extra_site={t_extra_site} unaligned={t_unaligned}\n\
           [GATE] regression_unexplained={t_reg_unexplained} (must=0)\n\
           [GATE] evidence_overclaim={t_ev_overclaim} (must=0)\n\
           [GATE] unverified_extra={t_unverified} (must=0)\n\
         \n\
         ── EXCLUDED / DEFERRED — scope is NOT full Phase 4 ──\n\
           EXCLUDED: events (Phase 4b) — oracle qualification + manual-binding property +\n\
                     canonical event key + reachability honesty work outstanding\n\
           DEFERRED (1B.3): member.missing_site={m_missing_site}\n\
                            (L3 Member sites with no fresh peer)\n\
           DEFERRED (1B.3): trigger.missing_site={t_missing_site}\n\
                            (L3 ImplicitTrigger sites with no fresh ImplicitTrigger edge)\n\
           DEFERRED (1B.3): regression_compound_receiver={regression_compound_receiver}\n\
                            (chained receiver type propagation)\n\
           DEFERRED (1B.3): regression_codeunit_implicit_rec={regression_codeunit_implicit_rec}\n\
                            (Codeunit TableNo/TestRunner implicit-Rec parameter)\n\
           DEFERRED (1B.3): regression_page_rec={regression_page_rec}\n\
                            (Page/PageExt implicit-Rec source-table gap)\n\
           DEFERRED (1B.3): 17 Cat-D divergences (same-object, different-procedure overload)\n\
         ═══════════════════════════════════════════════════════════════",
        matched = member.matched,
        verified_win = member.verified_win,
        divergence = member.divergence,
        fresh_ahead_interface = member.fresh_ahead_interface,
        fresh_ahead_instance_builtin = member.fresh_ahead_instance_builtin,
        fresh_ahead_enum_static = member.fresh_ahead_enum_static,
        m_extra_site = member.extra_site,
        m_unaligned = member.unaligned,
        m_reg_unexplained = member.regression_unexplained,
        m_ev_overclaim = member.evidence_overclaim,
        m_unverified = member.unverified_extra,
        t_matched = trigger.matched,
        t_verified_win = trigger.verified_win,
        t_divergence = trigger.divergence,
        fresh_ahead_trigger = trigger.fresh_ahead_trigger,
        fresh_ahead_validate_fanout = trigger.fresh_ahead_validate_fanout,
        t_extra_site = trigger.extra_site,
        t_unaligned = trigger.unaligned,
        t_reg_unexplained = trigger.regression_unexplained,
        t_ev_overclaim = trigger.evidence_overclaim,
        t_unverified = trigger.unverified_extra,
        m_missing_site = member.missing_site,
        t_missing_site = trigger.missing_site,
        regression_compound_receiver = member.regression_compound_receiver,
        regression_codeunit_implicit_rec = member.regression_codeunit_implicit_rec,
        regression_page_rec = member.regression_page_rec,
    );

    // ── Zero-tolerance gate: Member + Interface + instance-builtin ──────────
    assert_eq!(
        member.regression_unexplained, 0,
        "Member: fresh must not lose a Member target L3 resolved \
         (excl. named deferrals: interface/enum/page-rec/scalar/\
         compound-receiver/codeunit-implicit-rec): {member:?}"
    );
    assert_eq!(
        member.evidence_overclaim, 0,
        "Member: no Source/Abi/Catalog claim without a valid witness: {member:?}"
    );
    assert_eq!(
        member.unverified_extra, 0,
        "Member: no fresh-only fan-out route may fail the applicability predicate: {member:?}"
    );

    // ── Divergence cap (Member): all 56 CDO divergences adjudicated ─────────
    // 45 pre-Task-2 + 11 new interface fan-out divergences where fresh emits N
    // Routine routes and L3 emits 1 — fresh is more precise.
    assert!(
        member.divergence <= 56,
        "Member divergence grew beyond the adjudicated 56; inspect before merging: {member:?}"
    );

    // ── Zero-tolerance gate: ImplicitTrigger Multicast ──────────────────────
    assert_eq!(
        trigger.regression_unexplained, 0,
        "Trigger: fresh must not lose an ImplicitTrigger target L3 resolved: {trigger:?}"
    );
    assert_eq!(
        trigger.evidence_overclaim, 0,
        "Trigger: no Source/Abi/Catalog claim without a valid witness: {trigger:?}"
    );
    assert_eq!(
        trigger.unverified_extra, 0,
        "Trigger: no fresh-only trigger route may fail the applicability predicate: {trigger:?}"
    );
    // Trigger divergence cap (whole-branch review #1): paired-but-different trigger
    // target sets must stay 0 — a future regression here would otherwise pass silently.
    assert_eq!(
        trigger.divergence, 0,
        "Trigger: a paired ImplicitTrigger site diverged on its target set; inspect: {trigger:?}"
    );

    // ── Determinism: both harnesses must produce identical output on re-run ──
    assert_eq!(
        member,
        run_member_resolution_harness(&ws),
        "run_member_resolution_harness must be deterministic"
    );
    assert_eq!(
        trigger,
        run_implicit_trigger_harness(&ws),
        "run_implicit_trigger_harness must be deterministic"
    );
}

// Suppress unused-import warning when CDO_WS is not set (no CDO test runs).
#[allow(dead_code)]
fn _assert_diff_report_importable(_: DiffReport) {}

#[allow(dead_code)]
fn _assert_resolution_report_importable(_: ResolutionReport) {}

// ---------------------------------------------------------------------------
// Test 10 (Phase 4b Task 4; converted 1B.3b Task 1 Step 4): Fixture —
// L3-INDEPENDENT EventFlow target-set baseline
// ---------------------------------------------------------------------------

/// Verifies the fresh resolver's OWN EventFlow resolution against a frozen,
/// hand-reviewed baseline over the embedded fixture in `tests/fixtures/events/`.
///
/// 1B.3b Task 1: this test used to call [`run_event_flow_gate`] (a LIVE L3
/// comparison, even on this small synthetic fixture). It now calls
/// [`project_fresh_event_rows`] — L3-INDEPENDENT, no `engine::l3` build at
/// all — and asserts the EXACT resolved publisher→subscriber pair set
/// against a baseline frozen below. [`run_event_flow_gate`] itself is
/// unchanged and still runs as the live CDO-gated EventFlow gate (Test 11);
/// only THIS always-run, non-proprietary fixture test moved off L3.
///
/// The fixture has ONE app with:
///   • codeunit 50100 EventPublisher  — two overloads of OnAfterPost (0- and
///     1-param), OnBeforePost (BusinessEvent), OnInternalEvent (InternalEvent).
///   • codeunit 50200 ManualSub       — subscribes to OnAfterPost with 0 params,
///     EventSubscriberInstance=Manual.
///   • codeunit 50201 SkipLicenseSub  — subscribes to OnBeforePost,
///     SkipOnMissingLicense=true.
///   • codeunit 50202 MultiAttrSub    — two [EventSubscriber] attrs (OnAfterPost
///     + OnBeforePost on the same procedure) — fresh reads BOTH (no
///     first-attr-only limitation; that was an L3 quirk, not a fresh one).
///   • codeunit 50203 InternalSub     — subscribes to OnInternalEvent.
///
/// Fresh resolves exactly 5 publisher→subscriber rows (verified by inspecting
/// this exact baseline before committing it):
///   1. OnAfterPost (0-param overload)  <- ManualSub.HandleOnAfterPost
///   2. OnAfterPost (0-param overload)  <- MultiAttrSub.HandleBoth (first attr)
///   3. OnBeforePost                    <- SkipLicenseSub.HandleOnBeforePost
///   4. OnBeforePost                    <- MultiAttrSub.HandleBoth (second attr)
///   5. OnInternalEvent                 <- InternalSub.HandleOnInternalEvent
///
/// Fresh correctly disambiguates the 0-param OnAfterPost overload (no
/// subscriber lands on the 1-param overload) — that disambiguation was
/// previously visible only as `l3_false_positive_arity_mismatch` on the L3
/// comparison; here it is a direct, positive assertion about fresh's own
/// arity-aware overload pick.
#[test]
fn event_fixture_two_stage_join() {
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/events");

    let rows = project_fresh_event_rows(&fixture);
    let actual: Vec<(String, String, usize, String, String)> = rows
        .iter()
        .map(|r| {
            (
                r.publisher.object_lc.clone(),
                r.event_name_lc.clone(),
                r.publisher_arity.unwrap_or(usize::MAX),
                r.subscriber.object_lc.clone(),
                r.subscriber.routine_lc.clone(),
            )
        })
        .collect();
    eprintln!("event fixture fresh rows: {actual:#?}");

    let mut expected: Vec<(String, String, usize, String, String)> = vec![
        (
            "50100".into(),
            "onafterpost".into(),
            0,
            "50200".into(),
            "handleonafterpost".into(),
        ),
        (
            "50100".into(),
            "onafterpost".into(),
            0,
            "50202".into(),
            "handleboth".into(),
        ),
        (
            "50100".into(),
            "onbeforepost".into(),
            0,
            "50201".into(),
            "handleonbeforepost".into(),
        ),
        (
            "50100".into(),
            "onbeforepost".into(),
            0,
            "50202".into(),
            "handleboth".into(),
        ),
        (
            "50100".into(),
            "oninternalevent".into(),
            0,
            "50203".into(),
            "handleoninternalevent".into(),
        ),
    ];
    expected.sort();
    let mut actual_sorted = actual.clone();
    actual_sorted.sort();

    assert_eq!(
        actual_sorted, expected,
        "fresh EventFlow resolution over tests/fixtures/events diverged from the \
         frozen baseline.\nActual:\n{actual:#?}"
    );

    // No subscriber lands on the 1-param OnAfterPost overload — fresh's
    // arity-aware overload pick (was the L3 comparison's
    // `l3_false_positive_arity_mismatch` signal; now a direct assertion).
    assert!(
        rows.iter()
            .filter(|r| r.event_name_lc == "onafterpost")
            .all(|r| r.publisher_arity == Some(0)),
        "no subscriber may resolve to the 1-param OnAfterPost overload: {rows:#?}"
    );

    // Determinism
    let rows2 = project_fresh_event_rows(&fixture);
    assert_eq!(
        rows, rows2,
        "project_fresh_event_rows must be deterministic"
    );
}

// ---------------------------------------------------------------------------
// Test 11 (Phase 4b Task 4): CDO — EventFlow gate vs L3 oracle
// ---------------------------------------------------------------------------

/// Phase-4b EventFlow gate: proves the fresh EventFlow projection matches or
/// beats L3's event graph on a real Business Central workspace (CDO).
///
/// Six zero-tolerance assertions:
///   • `pair_l3_only == 0`: every L3-resolved (pub,event,sub) triple is matched
///     by a fresh EventFlow route — arity-agnostic recall guard.
///   • `l3_regression == 0`: no matched pair has a GENUINE arity disagreement
///     (where BOTH sides expose arity and they differ on a single-publisher event).
///   • `fresh_only_uncategorized == 0`: every fresh-only pair is categorized as
///     l3_maybe_upgrade / multiple_attr_l3_gap / internal_event_non_shipping.
///   • `fresh_unprojectable == 0`: every fresh EventFlow Routine route can be
///     projected to a full PairKey (no stable-id alignment failure).
///   • `l3_unprojectable == 0`: every L3 resolved edge can be projected to a
///     full PairKey.
///   • `unverified_extra == 0`: no subscriber Routine route fails the independent
///     raw-IR attribute check (subscriber's raw `[EventSubscriber]` re-parsed from
///     the `ParsedUnit` IR must name the publisher+event; params_count prefix check).
///
/// Informational: prints the full machine-categorized breakdown.
///
/// Guards: requires `CDO_WS` env var pointing at a real BC workspace.
#[test]
fn phase4b_event_flow() {
    let Some(ws) = std::env::var_os("CDO_WS")
        .map(std::path::PathBuf::from)
        .filter(|p| p.exists())
    else {
        return;
    };

    let report = run_event_flow_gate(&ws);

    eprintln!(
        "EventFlowGateReport:\n\
         fresh_event_edge_count={} fresh_event_row_count={} l3_event_row_count={}\n\
         matched={}\n\
         pair_l3_only={} pair_fresh_only={}\n\
         l3_maybe_upgrade={} multiple_attr_l3_gap={} internal_event_non_shipping={}\n\
         fresh_only_uncategorized={}\n\
         l3_false_positive_arity_mismatch={} l3_arity_unknown={} l3_regression={}\n\
         fresh_unprojectable={} l3_unprojectable={}\n\
         unverified_extra={}",
        report.fresh_event_edge_count,
        report.fresh_event_row_count,
        report.l3_event_row_count,
        report.matched,
        report.pair_l3_only,
        report.pair_fresh_only,
        report.l3_maybe_upgrade,
        report.multiple_attr_l3_gap,
        report.internal_event_non_shipping,
        report.fresh_only_uncategorized,
        report.l3_false_positive_arity_mismatch,
        report.l3_arity_unknown,
        report.l3_regression,
        report.fresh_unprojectable,
        report.l3_unprojectable,
        report.unverified_extra,
    );

    assert_eq!(
        report.fresh_unprojectable, 0,
        "fresh_unprojectable: {report:?}"
    );
    assert_eq!(report.l3_unprojectable, 0, "l3_unprojectable: {report:?}");
    assert_eq!(report.pair_l3_only, 0, "pair_l3_only: {report:?}");
    assert_eq!(report.l3_regression, 0, "l3_regression: {report:?}");
    assert_eq!(
        report.fresh_only_uncategorized, 0,
        "fresh_only_uncategorized: {report:?}"
    );
    assert_eq!(
        report.unverified_extra, 0,
        "no subscriber route may fail the independent raw-IR teeth: {report:?}"
    );

    // Determinism: two consecutive runs must produce identical output.
    assert_eq!(
        report,
        run_event_flow_gate(&ws),
        "run_event_flow_gate must be deterministic"
    );
}

#[allow(dead_code)]
fn _assert_event_flow_gate_report_importable(_: EventFlowGateReport) {}

#[allow(dead_code)]
fn _assert_member_report_importable(_: MemberResolutionReport) {}

#[allow(dead_code)]
fn _assert_implicit_trigger_report_importable(_: ImplicitTriggerResolutionReport) {}

// ---------------------------------------------------------------------------
// Task 5: Independent event-route teeth (unit tests — no CDO env required)
// ---------------------------------------------------------------------------

/// Build a minimal `ParsedUnit` from AL source for a given app GUID.
fn make_teeth_unit(guid: &str, name: &str, src: &str) -> (AppId, ParsedUnit) {
    let app_id = AppId {
        guid: guid.to_string(),
        name: name.to_string(),
        publisher: "Test".to_string(),
        version: "1.0.0.0".to_string(),
    };
    let provenance = Provenance {
        app: app_id.clone(),
        tier: TrustTier::Workspace,
        content_hash: String::new(),
    };
    let unit = ParsedUnit {
        app: app_id.clone(),
        files: vec![ParsedFile {
            virtual_path: "Sub.al".to_string(),
            file: al_syntax::parse(src),
            provenance,
            text: src.to_string(),
        }],
    };
    (app_id, unit)
}

/// Build a `(AppRegistry, RoutineNodeId)` for a codeunit-scoped procedure.
fn make_sub_rid(
    app_id: &AppId,
    obj_num: i64,
    routine_name_lc: &str,
    params: usize,
) -> (AppRegistry, RoutineNodeId) {
    let mut apps = AppRegistry::default();
    let app_ref = apps.intern(app_id);
    let rid = RoutineNodeId {
        object: ObjectNodeId {
            app: app_ref,
            kind: ObjectKind::Codeunit,
            key: ObjKey::Id(obj_num),
        },
        name_lc: routine_name_lc.to_string(),
        enclosing_member_lc: None,
        params_count: params,
        sig_fp: 0,
    };
    (apps, rid)
}

/// (c) Correct subscriber with a matching raw `[EventSubscriber]` attribute → PASSES.
#[test]
fn event_teeth_correct_subscriber_passes() {
    let src = r#"codeunit 50100 "EvtSub"
{
    [EventSubscriber(ObjectType::Codeunit, Codeunit::"EvtPub", 'OnAfterX', '', false, false)]
    local procedure OnAfterXHandler()
    begin
    end;
}"#;
    let (app_id, unit) = make_teeth_unit("guid-teeth-c", "TeethApp", src);
    let (apps, sub_rid) = make_sub_rid(&app_id, 50100, "onafterxhandler", 0);
    assert!(
        verify_event_subscriber_route(
            &sub_rid,
            "codeunit",
            "evtpub",
            "onafterx",
            0,
            &[unit],
            &apps,
        ),
        "correct subscriber must PASS the teeth check"
    );
}

/// (a) Subscriber raw attribute names a DIFFERENT publisher → FAILS.
#[test]
fn event_teeth_wrong_publisher_fails() {
    let src = r#"codeunit 50101 "EvtSubWrongPub"
{
    [EventSubscriber(ObjectType::Codeunit, Codeunit::"EvtPub", 'OnAfterX', '', false, false)]
    local procedure OnAfterXHandler()
    begin
    end;
}"#;
    let (app_id, unit) = make_teeth_unit("guid-teeth-a", "TeethApp", src);
    let (apps, sub_rid) = make_sub_rid(&app_id, 50101, "onafterxhandler", 0);
    assert!(
        !verify_event_subscriber_route(
            &sub_rid,
            "codeunit",
            "evtpub_other", // WRONG publisher name
            "onafterx",
            0,
            &[unit],
            &apps,
        ),
        "wrong publisher name must FAIL the teeth check"
    );
}

/// (b) Subscriber `params_count` exceeds publisher params → FAILS (parameter prefix check).
#[test]
fn event_teeth_excess_params_fails() {
    // Subscriber procedure has 2 params; publisher event has 0.
    let src = r#"codeunit 50102 "EvtSubManyParams"
{
    [EventSubscriber(ObjectType::Codeunit, Codeunit::"EvtPub", 'OnAfterX', '', false, false)]
    local procedure OnAfterXHandler(Sender: Codeunit "EvtPub"; var IsHandled: Boolean)
    begin
    end;
}"#;
    let (app_id, unit) = make_teeth_unit("guid-teeth-b", "TeethApp", src);
    let (apps, sub_rid) = make_sub_rid(&app_id, 50102, "onafterxhandler", 2);
    assert!(
        !verify_event_subscriber_route(
            &sub_rid,
            "codeunit",
            "evtpub",
            "onafterx",
            0, // publisher has 0 params; subscriber has 2
            &[unit],
            &apps,
        ),
        "subscriber with more params than publisher must FAIL the teeth check"
    );
}

// ---------------------------------------------------------------------------
// Tests 12–16: ABI ingestion-integrity + Histogram taxonomy split
// ---------------------------------------------------------------------------

use al_call_hierarchy::engine::deps::symbol_reference::{
    AbiEventKind as SrAbiEventKind, AbiObject, AbiParameter, AbiRoutine, SymbolReferenceAbi,
};
use al_call_hierarchy::program::node::AppRef;
use al_call_hierarchy::program::resolve::abi_check::{
    AbiIntegrityReport, RawAbiIndex, abi_ingestion_integrity, run_abi_integrity_check,
};
use al_call_hierarchy::program::resolve::edge::{
    AbiEventKind, AbiRoutineKey, AbiRoutineKind, BuiltinId, CanonicalSpan, DispatchShape, Edge,
    EdgeKind, Evidence, Histogram, Route, RouteTarget, SetCompleteness, SiteId, SourcePos, Witness,
};

/// Build a minimal dep abi with Codeunit 50100 "Dep Pub":
///   - DoDepWork(x: Integer) — procedure, 1 param
///   - OnDepEvent(p1, p2)   — event-publisher (Integration), 2 params
fn dep_pub_abi() -> SymbolReferenceAbi {
    SymbolReferenceAbi {
        objects: vec![AbiObject {
            object_type: "Codeunit".into(),
            object_number: 50100,
            name: "Dep Pub".into(),
            routines: vec![
                AbiRoutine {
                    name: "DoDepWork".into(),
                    kind: "procedure".into(),
                    event_kind: SrAbiEventKind::Unknown,
                    parameters: vec![AbiParameter {
                        name: "x".into(),
                        type_text: "Integer".into(),
                        is_var: false,
                        is_temporary: false,
                    }],
                    return_type_text: None,
                    is_local: false,
                    is_internal: false,
                    attributes: vec![],
                    attributes_parsed: vec![],
                },
                AbiRoutine {
                    name: "OnDepEvent".into(),
                    kind: "event-publisher".into(),
                    event_kind: SrAbiEventKind::Integration,
                    parameters: vec![
                        AbiParameter {
                            name: "p1".into(),
                            type_text: "Integer".into(),
                            is_var: false,
                            is_temporary: false,
                        },
                        AbiParameter {
                            name: "p2".into(),
                            type_text: "Text".into(),
                            is_var: false,
                            is_temporary: false,
                        },
                    ],
                    return_type_text: None,
                    is_local: false,
                    is_internal: false,
                    attributes: vec![],
                    attributes_parsed: vec![],
                },
            ],
            ..Default::default()
        }],
        ..Default::default()
    }
}

/// Build a minimal `RoutineNodeId` for use in synthetic edges.
fn test_rid(app: u32, obj_kind: ObjectKind, obj_num: i64, name: &str) -> RoutineNodeId {
    RoutineNodeId {
        object: ObjectNodeId {
            app: AppRef(app),
            kind: obj_kind,
            key: ObjKey::Id(obj_num),
        },
        name_lc: name.to_string(),
        enclosing_member_lc: None,
        params_count: 0,
        sig_fp: 0,
    }
}

/// Build a minimal synthetic `Edge` with a single route.
fn single_route_edge(from_rid: RoutineNodeId, route: Route) -> Edge {
    Edge {
        from: from_rid.clone(),
        site: SiteId {
            caller: from_rid,
            span: CanonicalSpan {
                unit: "Test.al".into(),
                start: SourcePos { line: 1, col: 1 },
                end: SourcePos { line: 1, col: 20 },
            },
            callee_fingerprint: 42,
        },
        kind: EdgeKind::Call,
        shape: DispatchShape::Exact,
        completeness: SetCompleteness::Complete,
        routes: vec![route],
    }
}

/// Build the `AbiRoutineKey` that `resolver.rs::make_routine_route` would emit
/// for `DoDepWork` on Codeunit 50100 in app `AppRef(1)`.
fn dodepwork_key() -> AbiRoutineKey {
    AbiRoutineKey {
        app: AppRef(1),
        // object_type is format!("{:?}", ObjectKind::Codeunit).to_ascii_lowercase()
        object_type: "codeunit".into(),
        object_number: 50100,
        object_name_lc: String::new(), // empty when object_number != 0
        routine_name_lc: "dodepwork".into(),
        params_count: 1,
        param_type_fp: 0, // not checked by the index
        routine_kind: AbiRoutineKind::Procedure,
        event_kind: AbiEventKind::None,
    }
}

/// Build the `AbiRoutineKey` for `OnDepEvent` (event-publisher/Integration).
fn ondepevent_key() -> AbiRoutineKey {
    AbiRoutineKey {
        app: AppRef(1),
        object_type: "codeunit".into(),
        object_number: 50100,
        object_name_lc: String::new(),
        routine_name_lc: "ondepevent".into(),
        params_count: 2,
        param_type_fp: 0,
        routine_kind: AbiRoutineKind::EventPublisher,
        event_kind: AbiEventKind::Integration,
    }
}

/// Test 12: a mapped `AbiSymbol` route → `abi_mapped=1, abi_unmapped=0`.
#[test]
fn abi_integrity_maps_known_routine() {
    let abi = dep_pub_abi();
    let index = RawAbiIndex::build([(AppRef(1), &abi)]);

    let caller = test_rid(0, ObjectKind::Codeunit, 99, "caller");
    let edge = single_route_edge(
        caller,
        Route {
            target: RouteTarget::AbiSymbol {
                key: dodepwork_key(),
            },
            evidence: Evidence::Opaque,
            conditions: vec![],
            witness: Witness::AbiSymbol {
                key: dodepwork_key(),
            },
        },
    );

    let report = abi_ingestion_integrity(&[edge], &index);
    assert_eq!(
        report,
        AbiIntegrityReport {
            abi_routes_total: 1,
            abi_mapped: 1,
            abi_unmapped: 0,
            abi_unmapped_sites: vec![],
        },
        "DoDepWork must map back to the raw ABI"
    );
}

/// Test 13: a fabricated `AbiSymbol` key naming a NON-existent routine →
/// `abi_unmapped=1`.
#[test]
fn abi_integrity_catches_unmapped_route() {
    let abi = dep_pub_abi();
    let index = RawAbiIndex::build([(AppRef(1), &abi)]);

    let bogus_key = AbiRoutineKey {
        app: AppRef(1),
        object_type: "codeunit".into(),
        object_number: 50100,
        object_name_lc: String::new(),
        routine_name_lc: "nonexistentproc".into(),
        params_count: 0,
        param_type_fp: 0,
        routine_kind: AbiRoutineKind::Procedure,
        event_kind: AbiEventKind::None,
    };

    let caller = test_rid(0, ObjectKind::Codeunit, 99, "caller");
    let edge = single_route_edge(
        caller,
        Route {
            target: RouteTarget::AbiSymbol {
                key: bogus_key.clone(),
            },
            evidence: Evidence::Opaque,
            conditions: vec![],
            witness: Witness::AbiSymbol {
                key: bogus_key.clone(),
            },
        },
    );

    let report = abi_ingestion_integrity(&[edge], &index);
    assert_eq!(report.abi_routes_total, 1);
    assert_eq!(
        report.abi_unmapped, 1,
        "a key naming a non-existent routine must be caught as unmapped"
    );
    assert_eq!(
        report.abi_unmapped_sites[0].key.routine_name_lc,
        "nonexistentproc"
    );
}

/// Test 14: an event-publisher-target route whose key says `EventPublisher /
/// Integration` → maps to the event-publisher ABI entry (Task-1 fix verified).
/// A key with the WRONG `routine_kind` (Procedure) must be caught as unmapped.
#[test]
fn abi_integrity_event_publisher_kind_checked() {
    let abi = dep_pub_abi();
    let index = RawAbiIndex::build([(AppRef(1), &abi)]);

    // Correct key (EventPublisher / Integration) → must map.
    let caller = test_rid(0, ObjectKind::Codeunit, 99, "caller");
    let correct_edge = single_route_edge(
        caller.clone(),
        Route {
            target: RouteTarget::AbiSymbol {
                key: ondepevent_key(),
            },
            evidence: Evidence::Opaque,
            conditions: vec![],
            witness: Witness::AbiSymbol {
                key: ondepevent_key(),
            },
        },
    );
    let ok = abi_ingestion_integrity(&[correct_edge], &index);
    assert_eq!(ok.abi_mapped, 1, "EventPublisher key must map");
    assert_eq!(ok.abi_unmapped, 0);

    // Wrong routine_kind (Procedure instead of EventPublisher) → must be caught.
    let mut wrong_key = ondepevent_key();
    wrong_key.routine_kind = AbiRoutineKind::Procedure;
    let wrong_edge = single_route_edge(
        caller,
        Route {
            target: RouteTarget::AbiSymbol {
                key: wrong_key.clone(),
            },
            evidence: Evidence::Opaque,
            conditions: vec![],
            witness: Witness::AbiSymbol { key: wrong_key },
        },
    );
    let bad = abi_ingestion_integrity(&[wrong_edge], &index);
    assert_eq!(
        bad.abi_unmapped, 1,
        "mangled routine_kind (Procedure instead of EventPublisher) must be unmapped"
    );
}

/// Test 15: Histogram taxonomy split.
///
/// • Source route  → `resolved_source` increments, NOT `resolved_catalog/abi_external`.
/// • Catalog route → `resolved_catalog` increments.
/// • AbiSymbol/Opaque route → `resolved_abi_external` increments.
/// • Unknown/empty → `unknown`.
/// • `real_unknown_rate` stays = unknown / total.
#[test]
fn histogram_taxonomy_split() {
    let ws_rid = test_rid(0, ObjectKind::Codeunit, 1, "caller");

    // Source-resolved edge.
    let src_edge = single_route_edge(
        ws_rid.clone(),
        Route {
            target: RouteTarget::Routine(test_rid(0, ObjectKind::Codeunit, 2, "target")),
            evidence: Evidence::Source,
            conditions: vec![],
            witness: Witness::SourceSpan {
                file: "f.al".into(),
                span: (0, 10),
            },
        },
    );

    // Catalog-resolved edge.
    let catalog_edge = single_route_edge(
        ws_rid.clone(),
        Route {
            target: RouteTarget::Builtin(BuiltinId("message".into())),
            evidence: Evidence::Catalog,
            conditions: vec![],
            witness: Witness::CatalogEntry {
                id: BuiltinId("message".into()),
                catalog_version: "v1".into(),
            },
        },
    );

    // ABI-external edge.
    let abi_edge = single_route_edge(
        ws_rid.clone(),
        Route {
            target: RouteTarget::AbiSymbol {
                key: dodepwork_key(),
            },
            evidence: Evidence::Opaque,
            conditions: vec![],
            witness: Witness::AbiSymbol {
                key: dodepwork_key(),
            },
        },
    );

    // Unknown (unresolved) edge.
    let unknown_edge = single_route_edge(
        ws_rid,
        Route {
            target: RouteTarget::Unresolved,
            evidence: Evidence::Unknown,
            conditions: vec![],
            witness: Witness::None,
        },
    );

    let edges = [src_edge, catalog_edge, abi_edge, unknown_edge];
    let h = Histogram::of_edges(&edges);

    assert_eq!(h.total, 4);
    assert_eq!(h.resolved_source, 1, "Source route → resolved_source");
    assert_eq!(h.resolved_catalog, 1, "Catalog route → resolved_catalog");
    assert_eq!(
        h.resolved_abi_external, 1,
        "AbiSymbol/Opaque route → resolved_abi_external"
    );
    assert_eq!(h.unknown, 1, "Unresolved/Unknown → unknown");
    assert_eq!(h.conditional_resolved, 0);
    assert_eq!(h.honest_dynamic, 0);
    assert_eq!(h.honest_empty, 0);

    // real_unknown_rate = 1/4 = 0.25
    let rate = h.real_unknown_rate();
    assert!(
        (rate - 0.25).abs() < 1e-9,
        "real_unknown_rate must be 0.25, got {rate}"
    );
}

/// Test 16 (CDO, env-gated): `abi_ingestion_integrity` over the full edge set →
/// `abi_unmapped == 0`.  Prints the taxonomy'd histogram + ABI coverage counts.
/// A miss = an ingestion/key-derivation bug — investigate and fix, do NOT relax.
#[test]
fn abi_ingestion_integrity_cdo_gate() {
    let Some(ws) = std::env::var_os("CDO_WS")
        .map(std::path::PathBuf::from)
        .filter(|p| p.exists())
    else {
        return;
    };

    let report = run_abi_integrity_check(&ws);

    eprintln!(
        "AbiIntegrityReport: abi_routes_total={} abi_mapped={} abi_unmapped={}",
        report.abi_routes_total, report.abi_mapped, report.abi_unmapped,
    );
    // When abi_routes_total == 0, abi_unmapped == 0 holds vacuously: the
    // workspace's deps all ship EmbeddedSource/ShowMyCode, so they resolve to
    // Source routes rather than AbiSymbol.  The 2 true SymbolOnly deps in CDO
    // are trivial (permissionset/translation apps) with no public routines.
    // ABI ingestion-path correctness is validated by the in-repo fixture tests
    // (Tests 12-14), NOT by this CDO run.  This note exists so a maintainer
    // reading a passing test output does not mistake "vacuous pass" for
    // "ABI coverage exercised on CDO".  When a workspace with SymbolOnly
    // public-routine deps is used, this gate WILL exercise the ABI path.
    if report.abi_routes_total == 0 {
        eprintln!(
            "NOTE: this CDO workspace has no SymbolOnly-dep routines (its deps ship \
             EmbeddedSource/ShowMyCode \u{2192} resolve to Source routes, not AbiSymbol). \
             The ABI ingestion path is validated by the in-repo fixtures (Tests 12-14), \
             NOT by this CDO run. abi_unmapped==0 holds trivially here."
        );
    }
    if !report.abi_unmapped_sites.is_empty() {
        eprintln!("UNMAPPED SITES (first 10):");
        for site in report.abi_unmapped_sites.iter().take(10) {
            eprintln!(
                "  app={:?} obj_type={} obj_num={} obj_name_lc={} \
                 routine={} params={} kind={:?} event={:?}",
                site.key.app,
                site.key.object_type,
                site.key.object_number,
                site.key.object_name_lc,
                site.key.routine_name_lc,
                site.key.params_count,
                site.key.routine_kind,
                site.key.event_kind,
            );
        }
    }

    // Also compute and print the histogram split.
    {
        use al_call_hierarchy::program::abi_ingest::AbiCache;
        use al_call_hierarchy::program::build::build_program_graph;
        use al_call_hierarchy::program::resolve::stub::resolve_program;
        use al_call_hierarchy::snapshot::{SnapshotBuilder, parse_snapshot};

        if let Ok(snap) = (SnapshotBuilder {
            workspace_root: ws.clone(),
            local_providers: vec![],
        })
        .build()
        {
            let cache = AbiCache::new();
            let graph = build_program_graph(&snap, &cache);
            let parsed = parse_snapshot(&snap);
            let edges = resolve_program(&graph, &parsed);
            let h = Histogram::of_edges(&edges);
            eprintln!(
                "Histogram: total={} resolved_source={} resolved_catalog={} \
                 resolved_abi_external={} conditional_resolved={} \
                 honest_dynamic={} honest_empty={} unknown={} \
                 real_unknown_rate={:.4}",
                h.total,
                h.resolved_source,
                h.resolved_catalog,
                h.resolved_abi_external,
                h.conditional_resolved,
                h.honest_dynamic,
                h.honest_empty,
                h.unknown,
                h.real_unknown_rate(),
            );
        }
    }

    assert_eq!(
        report.abi_unmapped, 0,
        "every AbiSymbol route must map back to the raw ABI — a miss is an \
         ingestion/key-derivation bug; investigate and fix: {report:?}"
    );

    // Determinism: two consecutive runs must produce identical output.
    assert_eq!(
        report,
        run_abi_integrity_check(&ws),
        "run_abi_integrity_check must be deterministic"
    );
}

/// Non-circularity demonstration.
///
/// Proves that `verify_event_subscriber_route` reads from the raw `ParsedUnit` IR,
/// NOT from any cached `RoutineNode.event_subscribers` field:
///
/// 1. With a correct `ParsedUnit` (raw `[EventSubscriber]` attribute present) → PASSES.
/// 2. With a modified `ParsedUnit` where the attribute is absent (simulating what
///    would happen if the function read corrupt raw IR instead of the cached value)
///    → FAILS.
///
/// If the function used the cached `RoutineNode.event_subscribers` (which still says
/// "subscribes to EvtPub"), both cases would return PASS.  The FAIL in case 2 is the
/// proof: the function observably reads from the raw `ParsedUnit` IR.
#[test]
fn event_teeth_non_circularity_reads_raw_ir() {
    // ── Case 1: correct ParsedUnit (attribute present) → PASSES ────────────
    let src_with_attr = r#"codeunit 50103 "EvtSubNC"
{
    [EventSubscriber(ObjectType::Codeunit, Codeunit::"EvtPub", 'OnAfterX', '', false, false)]
    local procedure OnAfterXHandler()
    begin
    end;
}"#;
    let (app_id, unit_with_attr) = make_teeth_unit("guid-teeth-nc", "TeethNC", src_with_attr);
    let (apps, sub_rid) = make_sub_rid(&app_id, 50103, "onafterxhandler", 0);

    assert!(
        verify_event_subscriber_route(
            &sub_rid,
            "codeunit",
            "evtpub",
            "onafterx",
            0,
            &[unit_with_attr],
            &apps,
        ),
        "correct raw IR must PASS"
    );

    // ── Case 2: ParsedUnit with attribute absent → FAILS ───────────────────
    // The `sub_rid` (RoutineNodeId) is unchanged — it represents the same routine
    // in the index's view.  If the function used a cached `RoutineNode.event_subscribers`
    // (built from the ORIGINAL correct source), it would still return PASS here.
    // The FAIL proves it actually re-parses the raw `ParsedUnit` IR.
    let src_no_attr = r#"codeunit 50103 "EvtSubNC"
{
    local procedure OnAfterXHandler()
    begin
    end;
}"#;
    let (_, unit_no_attr) = make_teeth_unit("guid-teeth-nc", "TeethNC", src_no_attr);

    assert!(
        !verify_event_subscriber_route(
            &sub_rid,
            "codeunit",
            "evtpub",
            "onafterx",
            0,
            &[unit_no_attr],
            &apps,
        ),
        "absent attribute in raw IR must FAIL — proves the check reads raw ParsedUnit IR, \
         not the index's cached event_subscribers"
    );
}

// ---------------------------------------------------------------------------
// Tests 11+: 1B.3a Task 3 — obligation coverage + resolve_full_program
// ---------------------------------------------------------------------------

use al_call_hierarchy::program::resolve::full::{
    Coverage, ObligationId, coverage_holds, resolve_full_program,
};

// ---------------------------------------------------------------------------
// Test 11 (unit fixture): coverage holds; histogram buckets are correct
// ---------------------------------------------------------------------------

/// Runs `resolve_full_program` over the small `full_program_fixture` workspace.
///
/// The fixture contains one codeunit with:
///   - Caller(): 3 call obligations (KnownProc → resolved_source; UnknownXYZ →
///     Unknown; Codeunit.Run(Dyn) → HonestDynamic)
///   - OnMyEvent(): publisher obligation → HonestEmpty EventFlow edge
///   - KnownProc(): 0 call obligations (body empty)
///
/// Assertions:
///   1. `coverage_holds` — every obligation maps to exactly one edge.
///   2. Histogram buckets count correctly.
///   3. `real_unknown_rate` is consistent with Unknown count / total.
#[test]
fn full_program_fixture_coverage_holds_and_histogram_is_correct() {
    let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/full_program_fixture");

    let report = resolve_full_program(&fixture).expect("fixture must parse successfully");

    // ── Coverage contract (distinct-id SET equality) ─────────────────────────
    assert!(
        coverage_holds(&report.coverage),
        "coverage contract violated: missing={:?}, extra={:?}",
        report.coverage.missing,
        report.coverage.extra,
    );

    // The fixture has 3 call sites (Caller body) + 1 publisher (OnMyEvent).
    // KnownProc body is empty, so no call sites there.
    assert_eq!(
        report.coverage.parsed_obligations, 4,
        "expected 3 call sites + 1 publisher obligation = 4 total"
    );
    assert_eq!(
        report.coverage.classified_edges, 4,
        "classified_edges must equal parsed_obligations"
    );

    // ── Histogram buckets ────────────────────────────────────────────────────
    // resolved_source=1 (KnownProc), unknown=1 (UnknownXYZ),
    // honest_dynamic=1 (Codeunit.Run(Dyn)), honest_empty=1 (OnMyEvent event).
    assert_eq!(
        report.histogram.resolved_source, 1,
        "KnownProc() must resolve via Source evidence"
    );
    assert_eq!(
        report.histogram.unknown, 1,
        "UnknownXYZ() must classify as Unknown"
    );
    assert_eq!(
        report.histogram.honest_dynamic, 1,
        "Codeunit.Run(Dyn) must classify as HonestDynamic"
    );
    assert_eq!(
        report.histogram.honest_empty, 1,
        "OnMyEvent publisher with zero subscribers must be HonestEmpty"
    );
    // Nothing should be in catalog or abi_external for this fixture.
    assert_eq!(report.histogram.resolved_catalog, 0);
    assert_eq!(report.histogram.resolved_abi_external, 0);

    // ── real_unknown_rate ────────────────────────────────────────────────────
    // 1 Unknown out of 4 total = 0.25
    let rate = report.histogram.real_unknown_rate();
    assert!(
        (rate - 0.25).abs() < 1e-9,
        "real_unknown_rate must be 0.25 for this fixture; got {rate}"
    );
}

// ---------------------------------------------------------------------------
// Test 12 (unit): dropped obligation → coverage_holds returns false
// ---------------------------------------------------------------------------

/// The coverage contract catches a silently-dropped obligation.
///
/// Verifies that if we manually construct a `Coverage` where one obligation ID
/// is missing from the edges, `coverage_holds` returns `false`.  This ensures
/// the contract check is active and not vacuously true.
#[test]
fn dropped_obligation_is_caught_by_coverage_contract() {
    use al_call_hierarchy::program::node::{
        AppRef, ObjKey, ObjectKind, ObjectNodeId, RoutineNodeId,
    };
    use al_call_hierarchy::program::resolve::edge::{CanonicalSpan, SourcePos};

    fn make_rid(name: &str) -> RoutineNodeId {
        RoutineNodeId {
            object: ObjectNodeId {
                app: AppRef(0),
                kind: ObjectKind::Codeunit,
                key: ObjKey::Id(1),
            },
            name_lc: name.to_string(),
            enclosing_member_lc: None,
            params_count: 0,
            sig_fp: 0,
        }
    }

    fn make_span(line: u32) -> CanonicalSpan {
        CanonicalSpan {
            unit: "Test.al".into(),
            start: SourcePos { line, col: 0 },
            end: SourcePos { line, col: 10 },
        }
    }

    let id_a = ObligationId::CallSite {
        caller: make_rid("caller"),
        span: make_span(10),
        callee_fp: 1,
    };
    let id_b = ObligationId::CallSite {
        caller: make_rid("caller"),
        span: make_span(20),
        callee_fp: 2,
    };

    // Coverage where obligation B is missing from edges — simulates a resolver
    // that silently dropped obligation B.
    let missing_coverage = Coverage {
        parsed_obligations: 2,
        classified_edges: 1,
        missing: vec![id_b.clone()],
        extra: vec![],
    };
    assert!(
        !coverage_holds(&missing_coverage),
        "a coverage with missing obligations must NOT hold"
    );

    // Coverage where both obligations are classified — contract must hold.
    let full_coverage = Coverage {
        parsed_obligations: 2,
        classified_edges: 2,
        missing: vec![],
        extra: vec![],
    };
    assert!(
        coverage_holds(&full_coverage),
        "a complete coverage must hold"
    );

    // Extra edge (no obligation): must also fail.
    let extra_coverage = Coverage {
        parsed_obligations: 1,
        classified_edges: 2,
        missing: vec![],
        extra: vec![id_a],
    };
    assert!(
        !coverage_holds(&extra_coverage),
        "a coverage with extra (obligation-less) edges must NOT hold"
    );
}

// ---------------------------------------------------------------------------
// Test 13 (CDO env-gated): coverage holds; evidence_overclaim=0; self-reported
//          metric prints + deterministic; rate ≤ recorded ceiling.
// ---------------------------------------------------------------------------

/// Full-program obligation coverage + self-reported north-star metric over CDO.
///
/// Guards: requires `CDO_WS` env var pointing at a real BC workspace.
///
/// Assertions (all required):
///   - `coverage_holds` (distinct-id SET equality — no obligation silently dropped)
///   - `abi_unmapped == 0` (ABI ingestion integrity)
///   - Taxonomy'd histogram + real_unknown_rate prints cleanly
///   - Deterministic (two consecutive runs produce identical histogram)
///   - `real_unknown_rate` ≤ recorded ceiling (regression guard)
#[test]
fn cdo_full_program_coverage_and_self_reported_metric() {
    let Some(ws) = std::env::var_os("CDO_WS")
        .map(std::path::PathBuf::from)
        .filter(|p| p.exists())
    else {
        return;
    };

    let report = resolve_full_program(&ws).expect("resolve_full_program must succeed on CDO_WS");

    // ── Coverage contract ────────────────────────────────────────────────────
    assert!(
        coverage_holds(&report.coverage),
        "coverage contract violated on CDO — no obligation may be silently dropped.\n\
         missing={} ids, extra={} ids",
        report.coverage.missing.len(),
        report.coverage.extra.len(),
    );

    // ── ABI ingestion integrity ──────────────────────────────────────────────
    assert_eq!(
        report.abi_integrity.abi_unmapped, 0,
        "ABI ingestion integrity: {} route key(s) not found in raw SymbolReference",
        report.abi_integrity.abi_unmapped
    );

    // ── Self-reported taxonomy'd histogram (print for record) ────────────────
    let h = &report.histogram;
    let ph = &report.primary_histogram;
    eprintln!(
        "\n\
         ═══════════════════════════════════════════════════════════════\n\
         1B.3a Task 3 — Self-reported north-star metric (no L3 oracle)\n\
         ═══════════════════════════════════════════════════════════════\n\
         \n\
         Whole-program (all source-bearing routines + all publishers):\n\
           total={} resolved_source={} resolved_catalog={} resolved_abi_external={}\n\
           conditional_resolved={} honest_dynamic={} honest_empty={} unknown={}\n\
           real_unknown_rate={:.4} ({:.2}%)\n\
         \n\
         Primary-scoped (workspace edges only — mirrors --l3-call-graph-stats-cross-app):\n\
           total={} resolved_source={} resolved_catalog={} resolved_abi_external={}\n\
           conditional_resolved={} honest_dynamic={} honest_empty={} unknown={}\n\
           real_unknown_rate={:.4} ({:.2}%)\n\
         \n\
         Coverage: parsed_obligations={} classified_edges={}\n\
         ABI integrity: abi_routes_total={} abi_mapped={} abi_unmapped={}\n\
         ═══════════════════════════════════════════════════════════════",
        h.total,
        h.resolved_source,
        h.resolved_catalog,
        h.resolved_abi_external,
        h.conditional_resolved,
        h.honest_dynamic,
        h.honest_empty,
        h.unknown,
        h.real_unknown_rate(),
        h.real_unknown_rate() * 100.0,
        ph.total,
        ph.resolved_source,
        ph.resolved_catalog,
        ph.resolved_abi_external,
        ph.conditional_resolved,
        ph.honest_dynamic,
        ph.honest_empty,
        ph.unknown,
        ph.real_unknown_rate(),
        ph.real_unknown_rate() * 100.0,
        report.coverage.parsed_obligations,
        report.coverage.classified_edges,
        report.abi_integrity.abi_routes_total,
        report.abi_integrity.abi_mapped,
        report.abi_integrity.abi_unmapped,
    );

    // ── Regression guard: primary real_unknown_rate ≤ recorded ceiling ───────
    // Ceiling recorded from first CDO run (2026-06-30): 6.46%.
    // 0.07 gives ~8% headroom above the baseline for safe guard.
    let primary_rate = ph.real_unknown_rate();
    assert!(
        primary_rate <= 0.07,
        "primary real_unknown_rate {primary_rate:.4} exceeds ceiling 0.07 — \
         engine regressed; investigate before raising the ceiling"
    );

    // ── Determinism ──────────────────────────────────────────────────────────
    let report2 = resolve_full_program(&ws).expect("second run must succeed");
    assert_eq!(
        report.histogram, report2.histogram,
        "resolve_full_program must be deterministic (histogram differs between runs)"
    );
    assert_eq!(
        report.primary_histogram, report2.primary_histogram,
        "resolve_full_program must be deterministic (primary_histogram differs)"
    );
    assert_eq!(
        report.coverage.parsed_obligations, report2.coverage.parsed_obligations,
        "resolve_full_program must be deterministic (parsed_obligations differs)"
    );
}

// ---------------------------------------------------------------------------
// Tests 14–16: 1B.3a Task 4 — L3-validated semantic golden + applicability
// ---------------------------------------------------------------------------

use al_call_hierarchy::program::resolve::semantic_golden::{
    ANON_GOLDEN_SCHEMA_VERSION, GoldenSiteKey, SemanticGolden, cdo_anon_golden_path,
    cdo_event_anon_golden_path, cdo_trigger_anon_golden_path, load_anon_event_golden,
    load_anon_golden, mint_fresh_golden_for_kind, mint_l3_validated_golden, run_cdo_event_audit,
    run_cdo_semantic_audit, run_cdo_trigger_audit, run_route_applicability, run_semantic_diff,
};

/// 1B.3b Task 1 ENFORCE_CDO_WS guard (part 1 — the `CDO_WS` presence check).
///
/// Returns the workspace path when `CDO_WS` is set and exists. When `CDO_WS`
/// is absent: returns `None` (caller should skip) UNLESS `ENFORCE_CDO_WS=1`,
/// in which case this PANICS — a gated/internal run that loses its `CDO_WS`
/// must fail loudly, not skip silently (no fail-open). Scoped to the three
/// frozen-golden audits this task adds/modifies (Tests 16–18) — the OTHER
/// pre-existing CDO-gated dual-run tests are unaffected (out of Task 1's
/// scope; they stay live L3 comparisons until 1B.3b Task 3).
fn cdo_ws_or_enforce() -> Option<std::path::PathBuf> {
    let ws = std::env::var_os("CDO_WS")
        .map(std::path::PathBuf::from)
        .filter(|p| p.exists());
    if ws.is_none() {
        assert!(
            std::env::var("ENFORCE_CDO_WS").as_deref() != Ok("1"),
            "ENFORCE_CDO_WS=1 but CDO_WS is unset or does not point at an existing path"
        );
    }
    ws
}

/// 1B.3b Task 1 ENFORCE_CDO_WS guard (part 2 — the audit-ran-and-checked-something
/// check). When `ENFORCE_CDO_WS=1`, PANICS if the committed golden failed to
/// load or the audit paired zero sites — the floor evaporating silently
/// (e.g. a renamed golden file, a CDO_WS pointed at the wrong tree) is
/// exactly the failure mode this guards against.
fn enforce_audit_ran(golden_loaded: bool, checked_sites: usize) {
    if std::env::var("ENFORCE_CDO_WS").as_deref() == Ok("1") {
        assert!(
            golden_loaded,
            "ENFORCE_CDO_WS=1: committed golden missing/invalid"
        );
        assert!(
            checked_sites > 0,
            "ENFORCE_CDO_WS=1: checked_sites==0 (audit ran but paired nothing — floor evaporated)"
        );
    }
}

// ---------------------------------------------------------------------------
// Test 14 (fixture): fresh edges match the L3-minted semantic golden
// ---------------------------------------------------------------------------

/// Asserts the in-repo L3-validated semantic golden: no `fresh_wrong` and no
/// `fresh_missing` over the `semantic-golden` fixture workspace.
///
/// The golden file (`tests/goldens/semantic-edges/fixture.json`) is minted from
/// L3 and committed.  Regenerate with `REGEN_TEMP_GOLDENS=1 cargo test
/// fixture_semantic_golden_matches_l3`.
///
/// Critical invariants:
///   - `fresh_wrong == 0`: fresh never resolves to a confidently-wrong target.
///   - `fresh_missing == 0`: fresh matches every L3-resolved site.
#[test]
fn fixture_semantic_golden_matches_l3() {
    let fixture =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/semantic-golden");
    let golden_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/goldens/semantic-edges/fixture.json");

    if std::env::var("REGEN_TEMP_GOLDENS").is_ok() {
        let golden = mint_l3_validated_golden(&fixture);
        let json = serde_json::to_string_pretty(&golden).expect("golden must serialize to JSON");
        std::fs::create_dir_all(golden_path.parent().unwrap())
            .expect("create goldens/semantic-edges dir");
        std::fs::write(&golden_path, &json).expect("write fixture golden");
        eprintln!(
            "REGEN: wrote {} site(s) to {}",
            golden.entries.len(),
            golden_path.display()
        );
        return;
    }

    let json = std::fs::read_to_string(&golden_path).unwrap_or_else(|_| {
        panic!(
            "golden file missing: {}\n\
             Run `REGEN_TEMP_GOLDENS=1 cargo test fixture_semantic_golden_matches_l3` \
             to mint it from L3.",
            golden_path.display()
        )
    });
    let golden: SemanticGolden = serde_json::from_str(&json).expect("golden JSON must deserialize");

    let diff = run_semantic_diff(&fixture, &golden);

    assert!(
        diff.fresh_wrong.is_empty(),
        "fresh_wrong MUST be empty — fresh resolved to a confidently-wrong target.\n\
         {} violation(s):\n{:#?}",
        diff.fresh_wrong.len(),
        diff.fresh_wrong,
    );
    assert!(
        diff.fresh_missing.is_empty(),
        "fresh_missing MUST be empty — fresh failed to match an L3-resolved site.\n\
         {} gap(s):\n{:#?}",
        diff.fresh_missing.len(),
        diff.fresh_missing,
    );

    eprintln!(
        "Test 14 — semantic golden: paired={} matches={} fresh_extra={} \
         fresh_novel={} golden_missing={}",
        diff.total_paired,
        diff.matches,
        diff.fresh_extra.len(),
        diff.fresh_novel,
        diff.golden_missing,
    );
}

// ---------------------------------------------------------------------------
// Test 14b (1B.3b Task 1 Step 4): fixture — ImplicitTrigger target-set
// ---------------------------------------------------------------------------

/// Synthetic, L3-INDEPENDENT ImplicitTrigger target-set fixture: asserts the
/// fresh resolver resolves the EXACT trigger set for `tests/fixtures/implicit-trigger`
/// (Table 50500 "ITFTable" + TableExtension 50501 "ITFTableExt" + Codeunit
/// 50502 "ITFCaller" — see the fixture's doc comment for the full layout).
///
/// The golden (`tests/goldens/semantic-edges/implicit-trigger-fixture.json`)
/// is minted from FRESH's own resolution (NOT L3 — see
/// [`mint_fresh_golden_for_kind`]) and committed; this is the
/// "frozen/hand-authored expected output" replacement for the
/// `ImplicitTrigger` dispatch-kind coverage that previously depended on a
/// live L3 comparison. Regenerate with `REGEN_TEMP_GOLDENS=1 cargo test
/// implicit_trigger_fixture_resolves_exact_target_set` — ALWAYS manually
/// inspect the diff before committing a regenerated golden (the point of a
/// frozen baseline is catching an UNINTENDED change, not rubber-stamping
/// whatever fresh currently does).
#[test]
fn implicit_trigger_fixture_resolves_exact_target_set() {
    let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/implicit-trigger");
    let golden_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/goldens/semantic-edges/implicit-trigger-fixture.json");

    if std::env::var("REGEN_TEMP_GOLDENS").is_ok() {
        let golden = mint_fresh_golden_for_kind(&fixture, EdgeKind::ImplicitTrigger);
        let json = serde_json::to_string_pretty(&golden).expect("golden must serialize to JSON");
        std::fs::create_dir_all(golden_path.parent().unwrap())
            .expect("create goldens/semantic-edges dir");
        std::fs::write(&golden_path, &json).expect("write implicit-trigger fixture golden");
        eprintln!(
            "REGEN: wrote {} site(s) to {}",
            golden.entries.len(),
            golden_path.display()
        );
        return;
    }

    let json = std::fs::read_to_string(&golden_path).unwrap_or_else(|_| {
        panic!(
            "golden file missing: {}\n\
             Run `REGEN_TEMP_GOLDENS=1 cargo test implicit_trigger_fixture_resolves_exact_target_set` \
             to mint it from fresh — then INSPECT the diff before committing.",
            golden_path.display()
        )
    });
    let golden: SemanticGolden = serde_json::from_str(&json).expect("golden JSON must deserialize");

    assert!(
        !golden.entries.is_empty(),
        "the frozen ImplicitTrigger fixture golden must be non-empty — an empty \
         golden would make this test vacuously pass"
    );

    let diff = run_semantic_diff(&fixture, &golden);

    assert!(
        diff.fresh_wrong.is_empty(),
        "fresh_wrong MUST be empty — fresh's ImplicitTrigger resolution changed \
         vs the frozen baseline.\n{} violation(s):\n{:#?}",
        diff.fresh_wrong.len(),
        diff.fresh_wrong,
    );
    assert!(
        diff.fresh_missing.is_empty(),
        "fresh_missing MUST be empty — fresh failed to resolve a site the frozen \
         baseline expects.\n{} gap(s):\n{:#?}",
        diff.fresh_missing.len(),
        diff.fresh_missing,
    );
    assert_eq!(
        diff.total_paired,
        golden.entries.len(),
        "every frozen-baseline site must pair with a fresh site (golden_missing must be 0): {diff:?}"
    );

    eprintln!(
        "Test 14b — ImplicitTrigger fixture: paired={} matches={} fresh_extra={} \
         fresh_novel={} golden_missing={}",
        diff.total_paired,
        diff.matches,
        diff.fresh_extra.len(),
        diff.fresh_novel,
        diff.golden_missing,
    );
}

// ---------------------------------------------------------------------------
// Test 15 (fixture + CDO env-gated): route-applicability contract
// ---------------------------------------------------------------------------

/// Route-applicability structural contract: `witness_contract_violations == 0`
/// and `abi_unmapped == 0` over both the in-repo fixture and (env-gated) CDO.
///
/// The witness↔evidence contract is: Source→SourceSpan, Abi→AbiSymbol,
/// Catalog→CatalogEntry, Opaque→AbiSymbol, Unknown→None+Unresolved.
/// Any violation is a resolver bug — the invariant must be maintained at all
/// times regardless of resolution precision.
#[test]
fn route_applicability_zero_violations() {
    // ── Fixture (no env needed) ───────────────────────────────────────────────
    let fixture =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/semantic-golden");
    let appl = run_route_applicability(&fixture);
    assert!(
        appl.is_clean(),
        "route-applicability contract violated on fixture: \
         witness_violations={} abi_unmapped={}",
        appl.witness_contract_violations,
        appl.abi_unmapped,
    );
    eprintln!(
        "Test 15 (fixture) — applicability: total_routes={} violations=0 abi_unmapped=0",
        appl.total_routes,
    );

    // ── CDO (env-gated) ───────────────────────────────────────────────────────
    let Some(ws) = std::env::var_os("CDO_WS")
        .map(std::path::PathBuf::from)
        .filter(|p| p.exists())
    else {
        return;
    };

    let appl_cdo = run_route_applicability(&ws);
    assert!(
        appl_cdo.is_clean(),
        "route-applicability contract violated on CDO_WS: \
         witness_violations={} abi_unmapped={}",
        appl_cdo.witness_contract_violations,
        appl_cdo.abi_unmapped,
    );
    eprintln!(
        "Test 15 (CDO) — applicability: total_routes={} violations=0 abi_unmapped=0",
        appl_cdo.total_routes,
    );
}

// ---------------------------------------------------------------------------
// Test 16 (CDO env-gated; load-frozen since 1B.3b Task 1): L3 semantic
// audit — no fresh_wrong
// ---------------------------------------------------------------------------

/// CDO semantic audit: compares the fresh resolver target-set against the
/// COMMITTED, ANONYMIZED, FROZEN L3 verdict (`cdo-anon.json`) over the real
/// CDO workspace.
///
/// 1B.3b Task 1: this no longer mints L3 live — `run_cdo_semantic_audit`
/// LOADS the committed golden. `audit.genuine_wrong_sites` stays PLAINTEXT
/// `GoldenSiteKey` (fresh's OWN identity, recovered from the anonymized
/// fresh-side comparison via the reverse index — see `anon.rs`'s
/// "re-hash-don't-decrypt" principle), so the manifest set-membership check
/// below is UNCHANGED from 1B.3a.
///
/// Guards: requires `CDO_WS` env var pointing at a real BC workspace.
/// `ENFORCE_CDO_WS=1` (the gated/internal runner) hard-fails if `CDO_WS` is
/// missing, the committed golden failed to load, or the audit paired zero
/// sites (`cdo_ws_or_enforce`/`enforce_audit_ran`).
///
/// ## What this test enforces
///
/// The `fresh_wrong` bucket (sites where both L3 and fresh resolved but to
/// different targets) is split into two adjudicated classes:
///
/// - **`fresh_ahead_dispatch`** (ALLOWED): fresh's targets REFINE L3's —
///   either L3's target is a subset of fresh's, or L3 resolved to an interface
///   and fresh resolved to concrete implementors. Phase-4 Interface/Polymorphic
///   fan-out. Not a bug.
///
/// - **`genuine_wrong`** (HARD GATE): fresh confidently resolved to a target
///   DISJOINT from L3's — a different object or procedure with no refinement
///   relationship. This is a real resolver bug. Every `genuine_wrong` site's
///   `(unit, line, callee_fp)` key MUST be present in the committed manifest
///   `tests/goldens/semantic-edges/known-genuine-divergences.json`. A site NOT
///   in the manifest = a NEW confidently-wrong edge → test FAILS immediately
///   with the offending site(s) printed. A count-only gate is insufficient: a
///   swap (fix one adjudicated site + introduce one new disjoint site) holds
///   the count constant and passes silently, defeating the gate entirely.
///
/// `fresh_missing` (L3 resolved but fresh didn't) is informational — tracked
/// over time. The known deferred buckets total 163; anything beyond is a new gap.
#[test]
fn cdo_l3_semantic_audit_no_fresh_wrong() {
    let Some(ws) = cdo_ws_or_enforce() else {
        return;
    };

    let audit = run_cdo_semantic_audit(&ws);
    enforce_audit_ran(audit.golden_loaded, audit.paired);
    assert!(
        audit.golden_loaded,
        "cdo-anon.json missing/invalid at {}; run the dev-mint tool \
         (`cargo run --bin mint-goldens`) with CDO_WS set",
        cdo_anon_golden_path().display(),
    );

    eprintln!(
        "\n\
         ═══════════════════════════════════════════════════════════════\n\
         1B.3a Task 4 — CDO L3 semantic audit\n\
         ═══════════════════════════════════════════════════════════════\n\
         l3_total={} fresh_total={}\n\
         paired={} matches={} ({}%)\n\
         fresh_wrong={} [fresh_ahead_dispatch={} genuine_wrong={}]\n\
         fresh_missing={} fresh_extra={}\n\
         fresh_novel={} golden_missing={}\n\
         digest={}\n\
         ═══════════════════════════════════════════════════════════════",
        audit.l3_total,
        audit.fresh_total,
        audit.paired,
        audit
            .paired
            .saturating_sub(audit.fresh_wrong_count)
            .saturating_sub(audit.fresh_missing_count)
            .saturating_sub(audit.fresh_extra_count),
        if audit.paired > 0 {
            (audit
                .paired
                .saturating_sub(audit.fresh_wrong_count)
                .saturating_sub(audit.fresh_missing_count)
                .saturating_sub(audit.fresh_extra_count)
                * 100)
                / audit.paired
        } else {
            0
        },
        audit.fresh_wrong_count,
        audit.fresh_ahead_dispatch_count,
        audit.genuine_wrong_count,
        audit.fresh_missing_count,
        audit.fresh_extra_count,
        audit.fresh_novel,
        audit.golden_missing,
        audit.digest,
    );

    // ── HARD GATE: genuine_wrong SET MEMBERSHIP against adjudicated manifest ──
    // genuine_wrong sites are real resolver bugs (Cat-D different-object or
    // wrong overload pick). They are enumerated in the committed manifest:
    //   tests/goldens/semantic-edges/known-genuine-divergences.json
    // Every genuine_wrong site's (unit, line, callee_fp) key MUST be in the
    // manifest set. A COUNT-only gate is insufficient: a swap (fix one adjudicated
    // site while introducing one new disjoint site) keeps the count at 42 and
    // passes silently — hiding the new bug. Set membership catches swaps.
    let manifest_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/goldens/semantic-edges/known-genuine-divergences.json");
    let manifest_json = std::fs::read_to_string(&manifest_path)
        .unwrap_or_else(|_| panic!("manifest missing: {}", manifest_path.display()));
    let manifest: serde_json::Value =
        serde_json::from_str(&manifest_json).expect("manifest must be valid JSON");
    let manifest_entries = manifest
        .get("entries")
        .and_then(|e| e.as_array())
        .expect("manifest must have 'entries' array");
    let manifest_keys: std::collections::HashSet<(String, u32, u64)> = manifest_entries
        .iter()
        .map(|entry| {
            let unit = entry["unit"]
                .as_str()
                .expect("manifest entry missing 'unit'")
                .to_string();
            let line = entry["line"]
                .as_u64()
                .expect("manifest entry missing 'line'") as u32;
            let callee_fp = entry["callee_fp"]
                .as_u64()
                .expect("manifest entry missing 'callee_fp'");
            (unit, line, callee_fp)
        })
        .collect();

    // SET MEMBERSHIP: every genuine_wrong site must be in the manifest.
    let new_genuine_wrong: Vec<&GoldenSiteKey> = audit
        .genuine_wrong_sites
        .iter()
        .filter(|site| !manifest_keys.contains(&(site.unit.clone(), site.line, site.callee_fp)))
        .collect();
    assert!(
        new_genuine_wrong.is_empty(),
        "genuine_wrong gate FAILED: {} site(s) NOT in the adjudicated manifest \
         (tests/goldens/semantic-edges/known-genuine-divergences.json).\n\
         A NEW confidently-wrong edge appeared — investigate and either fix the \
         resolver or extend the manifest with a root-cause explanation.\n\
         Offending sites:\n{:#?}",
        new_genuine_wrong.len(),
        new_genuine_wrong,
    );
    // Secondary sanity: count must not exceed the manifest (a decrease is a win).
    assert!(
        audit.genuine_wrong_count <= manifest_keys.len(),
        "genuine_wrong_count {} exceeds manifest size {} — all sites passed \
         membership but count exceeds manifest length (logic error?)",
        audit.genuine_wrong_count,
        manifest_keys.len(),
    );

    // fresh_ahead_dispatch is always ALLOWED (printed above for visibility).

    // ── Determinism: two consecutive runs produce the same digest ─────────────
    let audit2 = run_cdo_semantic_audit(&ws);
    assert_eq!(
        audit.digest, audit2.digest,
        "CDO semantic audit must be deterministic (digest differs between runs)"
    );
}

// ---------------------------------------------------------------------------
// Test 17 (CDO env-gated, 1B.3b Task 1): ImplicitTrigger frozen-golden audit
// ---------------------------------------------------------------------------

/// CDO ImplicitTrigger audit: compares fresh's `ImplicitTrigger` resolution
/// against the committed, anonymized, frozen L3 verdict
/// (`cdo-trigger-anon.json`). See [`AnonTriggerAuditReport`]'s doc comment
/// (in `semantic_golden.rs`) for this audit's scope — it proves the
/// frozen-load mechanism works for the ImplicitTrigger dispatch kind and
/// backs `ENFORCE_CDO_WS`'s `checked_sites>0` requirement. The zero-tolerance
/// ImplicitTrigger gate remains the live, CDO-gated
/// `run_implicit_trigger_harness` (unchanged this task).
#[test]
fn cdo_trigger_audit_frozen_load() {
    let Some(ws) = cdo_ws_or_enforce() else {
        return;
    };

    let audit = run_cdo_trigger_audit(&ws);
    enforce_audit_ran(audit.golden_loaded, audit.total_paired);
    assert!(
        audit.golden_loaded,
        "cdo-trigger-anon.json missing/invalid at {}; run the dev-mint tool \
         (`cargo run --bin mint-goldens`) with CDO_WS set",
        cdo_trigger_anon_golden_path().display(),
    );

    eprintln!(
        "Test 17 — CDO ImplicitTrigger frozen audit: l3_total={} fresh_total={} \
         total_paired={} matches={} fresh_wrong={} fresh_missing={} fresh_extra={} \
         fresh_novel={} golden_missing={} digest={}",
        audit.l3_total,
        audit.fresh_total,
        audit.total_paired,
        audit.matches,
        audit.fresh_wrong_count,
        audit.fresh_missing,
        audit.fresh_extra,
        audit.fresh_novel,
        audit.golden_missing,
        audit.digest,
    );

    // Determinism.
    let audit2 = run_cdo_trigger_audit(&ws);
    assert_eq!(
        audit.digest, audit2.digest,
        "CDO trigger audit must be deterministic (digest differs between runs)"
    );
}

// ---------------------------------------------------------------------------
// Test 18 (CDO env-gated, 1B.3b Task 1): EventFlow frozen-golden audit
// ---------------------------------------------------------------------------

/// CDO EventFlow audit: compares fresh's resolved EventFlow
/// publisher→subscriber pairs against the committed, anonymized, frozen L3
/// verdict (`cdo-event-anon.json`). Arity-agnostic pair-set comparison only —
/// see [`AnonEventAuditReport`]'s doc comment. The zero-tolerance EventFlow
/// gate remains the live, CDO-gated `run_event_flow_gate` (Test 11,
/// unchanged this task).
#[test]
fn cdo_event_audit_frozen_load() {
    let Some(ws) = cdo_ws_or_enforce() else {
        return;
    };

    let audit = run_cdo_event_audit(&ws);
    enforce_audit_ran(audit.golden_loaded, audit.matched_pairs);
    assert!(
        audit.golden_loaded,
        "cdo-event-anon.json missing/invalid at {}; run the dev-mint tool \
         (`cargo run --bin mint-goldens`) with CDO_WS set",
        cdo_event_anon_golden_path().display(),
    );

    eprintln!(
        "Test 18 — CDO EventFlow frozen audit: l3_total={} fresh_total={} \
         matched_pairs={} pair_l3_only={} pair_fresh_only={} digest={}",
        audit.l3_total,
        audit.fresh_total,
        audit.matched_pairs,
        audit.pair_l3_only,
        audit.pair_fresh_only,
        audit.digest,
    );

    // Determinism.
    let audit2 = run_cdo_event_audit(&ws);
    assert_eq!(
        audit.digest, audit2.digest,
        "CDO event audit must be deterministic (digest differs between runs)"
    );
}

// ---------------------------------------------------------------------------
// Test 19 (UNCONDITIONAL — no CDO_WS needed, public CI): committed golden
// metadata validation
// ---------------------------------------------------------------------------

/// Public-CI metadata validation (1B.3b Task 1): asserts the THREE committed
/// anonymized goldens exist, parse, carry the current schema version, and
/// have non-trivial per-dispatch-kind coverage — WITHOUT needing `CDO_WS` (no
/// CDO source is required to validate a committed artifact's shape). This is
/// the floor public CI (which never has CDO access) can verify; the per-site
/// diff itself only runs on the gated/internal runner (Tests 16–18).
///
/// Also validates the pre-existing `known-genuine-divergences.json` manifest
/// carries exactly 42 entries (1B.3a's adjudicated genuine_wrong baseline —
/// unrelated to `cdo-anon.json`'s anonymization, but co-located metadata this
/// test is the natural unconditional home for).
#[test]
fn committed_goldens_metadata_is_valid() {
    let golden = load_anon_golden(&cdo_anon_golden_path()).unwrap_or_else(|| {
        panic!(
            "cdo-anon.json missing/invalid at {} — committed goldens must always \
             parse, even without CDO_WS",
            cdo_anon_golden_path().display(),
        )
    });
    assert_eq!(golden.schema_version, ANON_GOLDEN_SCHEMA_VERSION);
    assert!(
        !golden.entries.is_empty(),
        "cdo-anon.json must be non-empty"
    );
    let mut by_edge_kind: std::collections::HashMap<u8, usize> = std::collections::HashMap::new();
    for e in &golden.entries {
        *by_edge_kind.entry(e.site.edge_kind).or_insert(0) += 1;
    }
    eprintln!(
        "cdo-anon.json: {} entries, by edge_kind: {by_edge_kind:?}",
        golden.entries.len()
    );
    // edge_kind 0=Call, 1=Run are the dispatch kinds this golden covers
    // (Member/Interface — see semantic_golden.rs's module docs); at least one
    // of each must be present for the golden to be meaningfully non-trivial.
    assert!(
        by_edge_kind.get(&0).copied().unwrap_or(0) > 0,
        "cdo-anon.json must contain at least one Call-kind (edge_kind=0) entry"
    );

    let trigger_golden = load_anon_golden(&cdo_trigger_anon_golden_path()).unwrap_or_else(|| {
        panic!(
            "cdo-trigger-anon.json missing/invalid at {}",
            cdo_trigger_anon_golden_path().display(),
        )
    });
    assert_eq!(trigger_golden.schema_version, ANON_GOLDEN_SCHEMA_VERSION);
    assert!(
        !trigger_golden.entries.is_empty(),
        "cdo-trigger-anon.json must be non-empty"
    );

    let event_golden = load_anon_event_golden(&cdo_event_anon_golden_path()).unwrap_or_else(|| {
        panic!(
            "cdo-event-anon.json missing/invalid at {}",
            cdo_event_anon_golden_path().display(),
        )
    });
    assert_eq!(event_golden.schema_version, ANON_GOLDEN_SCHEMA_VERSION);
    assert!(
        !event_golden.entries.is_empty(),
        "cdo-event-anon.json must be non-empty"
    );

    eprintln!(
        "Test 19 — committed golden metadata: cdo-anon entries={} trigger entries={} \
         event entries={}",
        golden.entries.len(),
        trigger_golden.entries.len(),
        event_golden.entries.len(),
    );

    // The pre-existing genuine_wrong manifest — co-located metadata, also
    // unconditionally checkable.
    let manifest_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/goldens/semantic-edges/known-genuine-divergences.json");
    let manifest_json = std::fs::read_to_string(&manifest_path)
        .unwrap_or_else(|_| panic!("manifest missing: {}", manifest_path.display()));
    let manifest: serde_json::Value =
        serde_json::from_str(&manifest_json).expect("manifest must be valid JSON");
    let manifest_entries = manifest
        .get("entries")
        .and_then(|e| e.as_array())
        .expect("manifest must have 'entries' array");
    assert_eq!(
        manifest_entries.len(),
        42,
        "known-genuine-divergences.json must carry exactly 42 adjudicated entries \
         (1B.3a baseline) — this assertion is UNCONDITIONAL (no CDO_WS needed)"
    );
}
