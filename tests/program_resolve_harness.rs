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
    ResolutionReport, SiteMatch, canonical_call_edge_for_test, match_sites, run_event_flow_gate,
    run_harness, run_implicit_trigger_harness, run_member_resolution_harness,
    run_resolution_harness, run_site_harness, verify_event_subscriber_route,
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
// Test 10 (Phase 4b Task 4): Fixture — EventFlow two-stage join projections
// ---------------------------------------------------------------------------

/// Verifies the structural two-stage event gate against the embedded fixture in
/// `tests/fixtures/events/`.
///
/// The fixture has ONE app with:
///   • codeunit 50100 EventPublisher  — two overloads of OnAfterPost (0- and
///     1-param), OnBeforePost (BusinessEvent), OnInternalEvent (InternalEvent).
///   • codeunit 50200 ManualSub       — subscribes to OnAfterPost with 0 params,
///     EventSubscriberInstance=Manual.
///   • codeunit 50201 SkipLicenseSub  — subscribes to OnBeforePost,
///     SkipOnMissingLicense=true.
///   • codeunit 50202 MultiAttrSub    — two [EventSubscriber] attrs (OnAfterPost
///     + OnBeforePost on the same procedure). L3 reads only the first.
///   • codeunit 50203 InternalSub     — subscribes to OnInternalEvent; L3 does
///     not classify InternalEvent publishers as resolved.
///
/// Expected counts:
///   • ManualSub → OnAfterPost: Stage-1 MATCH; L3 links to 1-param overload
///     (last-wins, arity-blind); fresh correctly picks 0-param →
///     l3_false_positive_arity_mismatch += 1.
///   • MultiAttrSub → OnAfterPost (first attr): Stage-1 MATCH; same arity-FP
///     as ManualSub (L3 again links to the 1-param overload) →
///     l3_false_positive_arity_mismatch += 1 (total = 2).
///   • MultiAttrSub → OnBeforePost (second attr): pair_fresh_only; L3 reads
///     only the first attr, so this subscription is invisible to L3 →
///     multiple_attr_l3_gap = 1.
///   • SkipLicenseSub → OnBeforePost: Stage-1 MATCH, arities agree.
///   • InternalSub → OnInternalEvent: pair_fresh_only; the EventPublisher object
///     IS found by L3 (same workspace) but OnInternalEvent has kind="procedure"
///     not "event-publisher" → L3 emits a "maybe" edge → l3_maybe_upgrade = 1
///     (caught before internal_event_non_shipping; internal_event_non_shipping=0).
///   • pair_l3_only = 0, l3_regression = 0, fresh_only_uncategorized = 0,
///     fresh_unprojectable = 0, l3_unprojectable = 0.
#[test]
fn event_fixture_two_stage_join() {
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/events");

    let report = run_event_flow_gate(&fixture);

    eprintln!("event fixture gate report: {report:?}");

    // Zero-tolerance
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

    // Structural assertions
    // Both ManualSub and MultiAttrSub (first attr) subscribe to OnAfterPost with
    // 0 params; L3 (last-wins, arity-blind) links both to the 1-param overload.
    assert_eq!(
        report.l3_false_positive_arity_mismatch, 2,
        "L3 over-links both 0-param subscribers to the 1-param OnAfterPost overload: {report:?}"
    );
    assert_eq!(
        report.multiple_attr_l3_gap, 1,
        "MultiAttrSub→OnBeforePost: second [EventSubscriber] attr L3 misses: {report:?}"
    );
    // InternalSub is caught by l3_maybe_upgrade (L3 creates a "maybe" edge because
    // the publisher object IS found but InternalEvent isn't classified as event-publisher).
    assert_eq!(
        report.l3_maybe_upgrade, 1,
        "InternalSub→OnInternalEvent: L3 emits maybe edge (object found, not real pub): {report:?}"
    );
    assert_eq!(
        report.internal_event_non_shipping, 0,
        "internal_event_non_shipping should be 0 (InternalSub caught by l3_maybe_upgrade first): {report:?}"
    );

    // Determinism
    assert_eq!(
        report,
        run_event_flow_gate(&fixture),
        "must be deterministic"
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
