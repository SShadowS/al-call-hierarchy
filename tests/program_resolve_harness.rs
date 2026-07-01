//! Phase 0: span-based site matcher fixture matrix (no env needed).
//!
//! Exercises [`match_sites`] — the cascade-resistance spine of the
//! L3-INDEPENDENT site matcher.  All tests construct synthetic edges via
//! [`canonical_call_edge_for_test`] so no real workspace is required.
//!
//! 1B.3b Task 3: the four live dual-run "fresh vs L3" comparison gates that
//! used to live here (`run_harness`/`run_site_harness`/
//! `run_resolution_harness`/`run_member_resolution_harness`/
//! `run_implicit_trigger_harness`/`run_event_flow_gate`, and their
//! `DiffReport`/`ResolutionReport`/`MemberResolutionReport`/
//! `ImplicitTriggerResolutionReport`/`EventFlowGateReport` report types) were
//! DELETED — replaced by the frozen semantic/trigger/event goldens + the
//! ported fan-out applicability teeth (Tests 14-20 below) and the
//! L3-INDEPENDENT fixture tests (`event_fixture_two_stage_join`,
//! `implicit_trigger_fixture_resolves_exact_target_set`).

use al_call_hierarchy::program::node::{
    AppRegistry, ObjKey, ObjectKind, ObjectNodeId, RoutineNodeId,
};
use al_call_hierarchy::program::resolve::differential::{
    SiteMatch, canonical_call_edge_for_test, match_sites, project_fresh_event_rows,
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
// Test 10 (Phase 4b Task 4; converted 1B.3b Task 1 Step 4): Fixture —
// L3-INDEPENDENT EventFlow target-set baseline
// ---------------------------------------------------------------------------

/// Verifies the fresh resolver's OWN EventFlow resolution against a frozen,
/// hand-reviewed baseline over the embedded fixture in `tests/fixtures/events/`.
///
/// 1B.3b Task 1: this test used to call `run_event_flow_gate` (a LIVE L3
/// comparison, even on this small synthetic fixture). It now calls
/// [`project_fresh_event_rows`] — L3-INDEPENDENT, no `engine::l3` build at
/// all — and asserts the EXACT resolved publisher→subscriber pair set
/// against a baseline frozen below. 1B.3b Task 3 deleted the old live,
/// CDO-gated `run_event_flow_gate` dual-run gate entirely; the zero-tolerance
/// EventFlow gate is now this fixture plus `cdo_event_audit_frozen_load`
/// (the frozen event golden) and the ported event-route teeth.
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
    ClassifiedEdge, Coverage, ObligationId, ProgramReport, coverage_holds, resolve_full_program,
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
    ANON_GOLDEN_SCHEMA_VERSION, AdjudicatedOverride, GoldenSiteKey, SemanticGolden,
    adjudicated_overrides_path, cdo_anon_golden_path, cdo_event_anon_golden_path,
    cdo_trigger_anon_golden_path, load_adjudicated_overrides, load_anon_event_golden,
    load_anon_golden, mint_fresh_golden_for_kind, mint_l3_validated_golden, run_cdo_event_audit,
    run_cdo_semantic_audit, run_cdo_trigger_audit, run_route_applicability, run_semantic_diff,
};

// beyond-1B.3b Task 3: the INDEPENDENT adjudication test's inputs — the
// structural builtin catalog (the SAME data the fresh resolver's `builtin`
// classification itself is built on; using it directly here is sanctioned by
// the brief's "structural catalog" independence criterion) and a hasher for
// the `source_sha256` drift check. Deliberately NOT importing
// `resolve_full_program`/`Edge`/`CanonicalEdge` anywhere near the adjudicator
// — see `cdo_genuine_wrong_is_precedence_adjudicated`'s doc comment.
use al_call_hierarchy::program::resolve::builtins::is_global_builtin;
use al_call_hierarchy::program::resolve::member_catalog::{MemberCatalogKind, member_builtin};
use al_call_hierarchy::program::resolve::receiver::FrameworkKind;
use sha2::{Digest, Sha256};

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
/// check).
///
/// `golden_loaded` stays ENFORCE-gated here (a missing/invalid committed
/// golden is hard-failed only on the gated/internal runner) — but every
/// caller of this function ALSO asserts `audit.golden_loaded` unconditionally
/// in the test body, so a missing golden fails everywhere regardless.
///
/// **1B.3b Task 1 fix (Fix 3): `checked_sites > 0` is now UNCONDITIONAL** —
/// it no longer requires `ENFORCE_CDO_WS=1`. This function is only ever
/// reached after `cdo_ws_or_enforce()` returned `Some` (i.e. `CDO_WS` is
/// present), so "unconditional" here means "whenever CDO_WS is set", which is
/// exactly the scope Fix 3 closes: before this fix, an orphaned anonymization
/// key (mint and audit hashing under different keys, so every `AnonSiteKey`
/// lookup silently misses) was caught ONLY on the gated/internal runner
/// (`ENFORCE_CDO_WS=1`) — the default local dev path (`CDO_WS` set, `ENFORCE`
/// unset) compared nothing and reported success. A golden that loaded but
/// paired zero sites is exactly as broken as one that failed to load; both
/// must fail loudly on every CDO-capable run, not just the gated one.
fn enforce_audit_ran(golden_loaded: bool, checked_sites: usize) {
    if std::env::var("ENFORCE_CDO_WS").as_deref() == Ok("1") {
        assert!(
            golden_loaded,
            "ENFORCE_CDO_WS=1: committed golden missing/invalid"
        );
    }
    assert!(
        checked_sites > 0,
        "checked_sites==0 (audit ran but paired nothing — floor evaporated, e.g. an \
         anon-key mismatch between mint and audit, a renamed golden file, or CDO_WS \
         pointed at the wrong tree). UNCONDITIONAL: this check does not require \
         ENFORCE_CDO_WS=1 (1B.3b Task 1 fix, Fix 3)."
    );
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
    // beyond-1B.3b Task 3: ALL 42 manifest entries are now adjudicated
    // `l3_error_intrinsic` and overlaid (`run_cdo_semantic_audit` applies
    // `adjudicated-overrides.json` in-memory before diffing) — fresh is
    // compared against the ADJUDICATED target for these sites, which fresh
    // matches by construction (that agreement is what the independent
    // adjudication in `cdo_genuine_wrong_is_precedence_adjudicated`
    // confirms). So `genuine_wrong_count` must now be EXACTLY 0: a nonzero
    // count means either the overlay failed to apply (a wiring bug) or a
    // genuinely NEW disjoint divergence appeared that is not one of the 42
    // known/adjudicated sites — both are real bugs, not "still-acceptable
    // known wrongness". The manifest/set-membership checks above stay as
    // defense-in-depth for that second case.
    assert_eq!(
        audit.genuine_wrong_count, 0,
        "genuine_wrong_count={} (expected 0): all 42 known-genuine-divergences.json sites \
         are adjudicated l3_error_intrinsic and should have been overlaid to match fresh \
         exactly (see adjudicated-overrides.json / apply_adjudicated_overrides). A nonzero \
         count means either the overlay didn't apply (check for an \
         'Adjudication overlay: N/42' log line above — N should be 42) or a genuinely NEW \
         divergence appeared beyond the 42 adjudicated ones.",
        audit.genuine_wrong_count,
    );

    // fresh_ahead_dispatch is always ALLOWED (printed above for visibility).

    // ── COMPLETENESS FLOOR (1B.3b whole-branch fix): re-instate the deleted
    // `regression_unexplained == 0` leg as a pinned CEILING on `fresh_missing`.
    //
    // `fresh_missing` (L3 resolved a target, fresh emitted nothing) was
    // previously informational-only: a dropped trigger/event/member target at
    // CDO scale could increment this counter silently and the test would
    // still pass. 191 is the CURRENT, EXACT `fresh_missing_count` reproduced
    // against the live CDO_WS fresh resolver on 2026-07-01 — it matches the
    // documented characterization in CHANGELOG.md (1B.3a Task 4 entry):
    // `page_rec=115 + codeunit_implicit_rec=24 + trigger=38 + other=14 = 191`,
    // all KNOWN, ALREADY-DEFERRED buckets (Page/PageExt implicit-Rec,
    // Codeunit TableNo/TestRunner implicit-Rec, ImplicitTrigger-shaped member
    // calls, and a long tail), not a fresh regression. Pinning the exact
    // current value (rather than a round-number ceiling) means a NEW drop —
    // even a single one beyond these known buckets — pushes the count to 192
    // and FAILS, restoring the floor the old dual-run gate's
    // `regression_unexplained == 0` provided. A manifest mirroring
    // `known-genuine-divergences.json` would be the ideal (set-membership,
    // immune to swaps) but is out of scope for this fix — see this fix's
    // CHANGELOG/report entry for why a ceiling was chosen over a manifest
    // here. Raising this ceiling requires re-justifying the new value against
    // a real characterization, not just bumping the number.
    const FRESH_MISSING_CEILING: usize = 191;
    assert!(
        audit.fresh_missing_count <= FRESH_MISSING_CEILING,
        "COMPLETENESS REGRESSION: fresh_missing_count={} exceeds the recorded \
         ceiling {} (known-deferred-bucket baseline pinned 2026-07-01: \
         page_rec=115 codeunit_implicit_rec=24 trigger=38 other=14 = 191; see \
         CHANGELOG.md 1B.3a Task 4). The fresh resolver lost an L3-resolved \
         target it used to find — investigate before raising the ceiling.",
        audit.fresh_missing_count,
        FRESH_MISSING_CEILING,
    );

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
/// ImplicitTrigger gate is this frozen audit plus the
/// `implicit_trigger_fixture_resolves_exact_target_set` fixture test and the
/// ported applicability teeth (`fan_out_applicability_zero_violations`) — the
/// old live, CDO-gated `run_implicit_trigger_harness` dual-run gate was
/// deleted in 1B.3b Task 3.
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

    // ── COMPLETENESS FLOOR (1B.3b whole-branch fix): re-instate the deleted
    // `regression_unexplained == 0` leg for ImplicitTrigger.
    //
    // Zero tolerance for fresh confidently resolving a paired trigger site to
    // the WRONG target set — this mirrors the old live gate's hard
    // zero-tolerance and currently holds (`fresh_wrong_count == 0`).
    assert_eq!(
        audit.fresh_wrong_count, 0,
        "COMPLETENESS REGRESSION: ImplicitTrigger fresh_wrong_count={} (must be 0) \
         — fresh disagreeing with a frozen, L3-verified trigger target is a real \
         resolver bug, investigate.",
        audit.fresh_wrong_count,
    );
    // `fresh_missing` (L3 resolved a trigger target, fresh emitted nothing) is
    // NOT presently zero: this golden carries a SMALL, STABLE, pre-existing
    // gap of 3 sites that was already present at golden MINT time (1B.3b
    // Task 1, `.superpowers/sdd/task-1-report.md`: "total_paired=188
    // matches=185 fresh_wrong=0 fresh_missing=3") and has been UNCHANGED
    // through every capstone verification since (1B.3b Task 4 capstone:
    // identical `matches=185`; reproduced again on 2026-07-01 for this fix:
    // identical `total_paired=188 matches=185 fresh_missing=3`). It predates
    // the gate-completeness deletion this fix restores, so asserting literal
    // `matches == total_paired` would fail on a KNOWN, already-accepted gap,
    // not a new one. Pin it as a CEILING instead (same pattern as Test 16):
    // any NEW drop (4+) is a real completeness regression and FAILS.
    const FRESH_MISSING_CEILING: usize = 3;
    assert!(
        audit.fresh_missing <= FRESH_MISSING_CEILING,
        "COMPLETENESS REGRESSION: ImplicitTrigger fresh_missing={} exceeds the \
         recorded ceiling {} (stable since the golden's 1B.3b Task 1 mint-time \
         verification — see task-1-report.md). A NEW dropped trigger target. \
         Investigate before raising the ceiling.",
        audit.fresh_missing,
        FRESH_MISSING_CEILING,
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
/// gate is this frozen audit plus the `event_fixture_two_stage_join` fixture
/// test and the ported event-route teeth — the old live, CDO-gated
/// `run_event_flow_gate` dual-run gate was deleted in 1B.3b Task 3.
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

    // ── COMPLETENESS FLOOR (1B.3b whole-branch fix): re-instate the deleted
    // `regression_unexplained == 0` leg for EventFlow. Zero tolerance: every
    // frozen L3 publisher→subscriber pair must still be found by fresh.
    assert_eq!(
        audit.pair_l3_only, 0,
        "COMPLETENESS REGRESSION: {} frozen L3 EventFlow pair(s) are missing from \
         fresh (pair_l3_only must be 0) — fresh lost a publisher\u{2192}subscriber \
         pair it used to resolve, investigate.",
        audit.pair_l3_only,
    );
    assert_eq!(
        audit.matched_pairs, audit.l3_total,
        "COMPLETENESS REGRESSION: matched_pairs={} != l3_total={} — every frozen \
         L3 EventFlow pair must be matched by fresh.",
        audit.matched_pairs, audit.l3_total,
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

    // ── beyond-1B.3b Task 3: manifest + overlay invariants (replaces the bare
    // `assert_eq!(len, 42)`) ────────────────────────────────────────────────
    //
    // Every `known-genuine-divergences.json` entry now carries an adjudicated
    // `verdict` (Task 3). Split: 42 `l3_error_intrinsic` / 0
    // `fresh_false_builtin` (would mean Tasks 1-2 left a real fresh bug
    // unabsorbed) / 0 `needs_manual_review` (fail-closed — an unresolved
    // dimension is never silently treated as passing).
    let mut manifest_site_keys: std::collections::HashSet<(String, u64, u64)> =
        std::collections::HashSet::new();
    let mut manifest_intrinsic_keys: std::collections::HashSet<(String, u64, u64)> =
        std::collections::HashSet::new();
    for entry in manifest_entries {
        let unit = entry["unit"]
            .as_str()
            .expect("manifest entry missing 'unit'")
            .to_string();
        let line = entry["line"]
            .as_u64()
            .expect("manifest entry missing 'line'");
        let callee_fp = entry["callee_fp"]
            .as_u64()
            .expect("manifest entry missing 'callee_fp'");
        let verdict = entry["verdict"]
            .as_str()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| {
                panic!("manifest entry {unit}:{line} missing non-empty 'verdict' (Task 3)")
            });
        assert!(
            matches!(
                verdict,
                "l3_error_intrinsic" | "fresh_false_builtin" | "needs_manual_review"
            ),
            "manifest entry {unit}:{line} has unrecognized verdict {verdict:?}"
        );
        let key = (unit.clone(), line, callee_fp);
        assert!(
            manifest_site_keys.insert(key.clone()),
            "duplicate site key in known-genuine-divergences.json: {unit}:{line} fp={callee_fp}"
        );
        if verdict == "l3_error_intrinsic" {
            manifest_intrinsic_keys.insert(key);
        }
    }
    assert_eq!(
        manifest_entries.len(),
        42,
        "known-genuine-divergences.json must carry exactly 42 adjudicated entries \
         (beyond-1B.3b Task 3: 42 l3_error_intrinsic / 0 fresh_false_builtin / \
         0 needs_manual_review) — this assertion is UNCONDITIONAL (no CDO_WS needed)"
    );
    assert_eq!(
        manifest_intrinsic_keys.len(),
        42,
        "expected all 42 known-genuine-divergences.json entries to be adjudicated \
         l3_error_intrinsic; a non-42 count means a fresh_false_builtin or \
         needs_manual_review survivor slipped through — investigate before relying \
         on the overlay"
    );

    // The adjudication overlay itself (`adjudicated-overrides.json`) — also
    // unconditionally checkable (pure JSON, no CDO_WS needed to validate its
    // SHAPE; the CDO-gated `cdo_genuine_wrong_is_precedence_adjudicated` test
    // re-verifies its CONTENT against live source).
    let overrides =
        load_adjudicated_overrides(&adjudicated_overrides_path()).unwrap_or_else(|| {
            panic!(
                "adjudicated-overrides.json missing/invalid at {}",
                adjudicated_overrides_path().display(),
            )
        });
    let mut override_site_keys: std::collections::HashSet<(String, u64, u64)> =
        std::collections::HashSet::new();
    for ov in &overrides.entries {
        assert!(!ov.callee_text.is_empty(), "override missing callee_text");
        assert!(!ov.catalog_key.is_empty(), "override missing catalog_key");
        assert!(
            !ov.receiver_kind.is_empty(),
            "override missing receiver_kind"
        );
        assert_eq!(
            ov.source_sha256.len(),
            64,
            "override source_sha256 must be a 64-hex-char SHA-256 digest (unit={})",
            ov.unit
        );
        assert!(
            ov.source_sha256
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
            "override source_sha256 must be lowercase hex (unit={})",
            ov.unit
        );
        assert!(!ov.verdict.is_empty(), "override missing verdict");
        let key = (ov.unit.clone(), ov.line as u64, ov.callee_fp);
        assert!(
            override_site_keys.insert(key),
            "duplicate site key in adjudicated-overrides.json: {}:{} fp={}",
            ov.unit,
            ov.line,
            ov.callee_fp
        );
    }
    // Every `l3_error_intrinsic` manifest entry must have a matching overlay
    // entry (also verdict `l3_error_intrinsic`) — the overlay is what
    // actually makes `run_cdo_semantic_audit` stop flagging these sites, so a
    // manifest entry without a matching overlay entry would silently keep
    // failing the CDO gate despite claiming to be adjudicated.
    let override_intrinsic_keys: std::collections::HashSet<(String, u64, u64)> = overrides
        .entries
        .iter()
        .filter(|ov| ov.verdict == "l3_error_intrinsic")
        .map(|ov| (ov.unit.clone(), ov.line as u64, ov.callee_fp))
        .collect();
    assert_eq!(
        manifest_intrinsic_keys, override_intrinsic_keys,
        "every known-genuine-divergences.json entry adjudicated l3_error_intrinsic must \
         have a matching adjudicated-overrides.json entry (also l3_error_intrinsic), and \
         vice versa — the two sets diverged"
    );
    assert_eq!(
        overrides.entries.len(),
        42,
        "adjudicated-overrides.json must carry exactly 42 entries (one per adjudicated \
         known-genuine-divergences.json site; beyond-1B.3b Task 3)"
    );

    // ── Non-circularity invariant (testable): overlay entries hold CANONICAL
    // CATALOG KEYS / expected-route FACTS, never a serialized fresh edge id.
    // Parse the raw JSON (not the typed struct, which would silently drop an
    // unexpected field) and assert no entry's key set contains anything
    // shaped like a fresh-computed graph/edge/routine identifier.
    let overrides_json: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(adjudicated_overrides_path())
            .expect("adjudicated-overrides.json must be readable"),
    )
    .expect("adjudicated-overrides.json must be valid JSON");
    const FORBIDDEN_FRESH_EDGE_ID_FIELDS: &[&str] = &[
        "resolved_target",
        "resolved_target_id",
        "fresh_edge_id",
        "fresh_target",
        "edge_id",
        "routine_node_id",
        "object_node_id",
        "target_id",
        "route_target",
    ];
    for ov in overrides_json["entries"]
        .as_array()
        .expect("overrides 'entries' must be an array")
    {
        let obj = ov
            .as_object()
            .expect("override entry must be a JSON object");
        for forbidden in FORBIDDEN_FRESH_EDGE_ID_FIELDS {
            assert!(
                !obj.contains_key(*forbidden),
                "adjudicated-overrides.json entry carries a fresh-edge-id-shaped field \
                 {forbidden:?} — overlay entries must hold only canonical catalog keys \
                 (name+arity+receiver-kind) derived independently of fresh's output, \
                 NEVER a serialized fresh edge/route/graph-node id (non-circularity \
                 invariant)"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Test 19b (CDO env-gated, beyond-1B.3b Task 3): genuine_wrong sites are
// precedence-adjudicated from INDEPENDENT source criteria
// ---------------------------------------------------------------------------

/// Case-insensitive, whole-token scan for a LOCAL `procedure <name>(`
/// declaration anywhere in `unit_content` — the lookup-precedence "does a
/// source competitor shadow the catalog hit" check (Task 1: Source shadows
/// Catalog).
///
/// Pure text search over the SAME live CDO source the test reads — no
/// engine/resolver/graph involvement whatsoever. Deliberately permissive
/// (matches any object member named `name`, not just ones reachable from a
/// specific call site) so it stays conservative: a false POSITIVE here would
/// only push a site toward `fresh_false_builtin`/re-investigation, never
/// toward a false PASS.
fn unit_declares_procedure_named(unit_content: &str, name_lc: &str) -> bool {
    let lc = unit_content.to_ascii_lowercase();
    let bytes = lc.as_bytes();
    let needle = "procedure";
    let mut start = 0usize;
    while let Some(pos) = lc[start..].find(needle) {
        let abs = start + pos;
        let before_ok = abs == 0 || {
            let c = bytes[abs - 1];
            !(c.is_ascii_alphanumeric() || c == b'_')
        };
        let after_idx = abs + needle.len();
        let after_ok = after_idx < bytes.len() && bytes[after_idx].is_ascii_whitespace();
        if before_ok && after_ok {
            let mut i = after_idx;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            let tok_start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let tok = &lc[tok_start..i];
            if tok == name_lc {
                let mut j = i;
                while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                    j += 1;
                }
                if j < bytes.len() && bytes[j] == b'(' {
                    return true;
                }
            }
        }
        start = abs + needle.len();
    }
    false
}

/// Independently re-derive an [`AdjudicatedOverride`]'s verdict from LIVE
/// `unit_content` plus the structural builtin catalog — see
/// `semantic_golden.rs`'s `AdjudicatedOverride` doc comment for the full
/// independence contract this function embodies: it calls ONLY
/// [`is_global_builtin`]/[`member_builtin`] (the structural catalog) and
/// [`unit_declares_procedure_named`] (a plain-text scan of the SAME unit) —
/// never `resolve_full_program`, never a fresh-computed `Edge`.
fn derive_verdict(ov: &AdjudicatedOverride, unit_content: &str) -> &'static str {
    let method_lc = ov
        .catalog_key
        .rsplit("::")
        .next()
        .unwrap_or(&ov.catalog_key)
        .to_ascii_lowercase();

    let catalog_match = match ov.receiver_kind.as_str() {
        "Global" => is_global_builtin(&method_lc),
        "PageInstance" => member_builtin(
            MemberCatalogKind::Framework(&FrameworkKind::PageInstance),
            &method_lc,
        ),
        "Record" => member_builtin(MemberCatalogKind::Record, &method_lc),
        "RecordRef" => member_builtin(MemberCatalogKind::RecordRef, &method_lc),
        _ => return "needs_manual_review", // unrecognized receiver kind — fail closed
    };
    if !catalog_match {
        // The claimed catalog member doesn't actually exist for this
        // receiver kind — fresh's builtin claim would be unsupported.
        return "fresh_false_builtin";
    }
    if unit_declares_procedure_named(unit_content, &method_lc) {
        // A source competitor shadows the catalog hit (Task 1 lookup
        // precedence: Source shadows Catalog) — fresh should have picked the
        // source routine, so a `builtin` claim here would be a fresh bug.
        return "fresh_false_builtin";
    }
    "l3_error_intrinsic"
}

/// The call SHAPE parsed straight from `callee_text`, independent of the
/// overlay's own `receiver_kind`: a bare GLOBAL call (no `.`) or a MEMBER
/// call `<receiver>.<method>`, split on the FINAL `.`. Every `callee_text`
/// in the overlay is a simple `Receiver.Method` token pair — no
/// chained/qualified receivers appear among the 42 adjudicated sites — so a
/// single `rfind('.')` split is sufficient. Deliberately lightweight: this
/// is a syntax check, not a type-inferring parser (see
/// `assert_shape_matches_receiver_kind`'s doc comment for what it does and
/// does not prove).
enum CallShape<'a> {
    Global(&'a str),
    Member { receiver: &'a str, method: &'a str },
}

fn parse_callee_shape(callee_text: &str) -> CallShape<'_> {
    match callee_text.rfind('.') {
        Some(idx) => CallShape::Member {
            receiver: &callee_text[..idx],
            method: &callee_text[idx + 1..],
        },
        None => CallShape::Global(callee_text),
    }
}

/// Review-fix (beyond-1B.3b Task 3 fix pass): independently cross-check
/// `ov.receiver_kind` and `ov.catalog_key`'s method component against the
/// call SHAPE parsed straight from `ov.callee_text`, BEFORE `derive_verdict`
/// is allowed to trust `receiver_kind` as given. Closes the review gap where
/// a mislabeled `receiver_kind` (e.g. `"Global"` recorded for what is
/// actually a member call `X.Method(...)` whose method name also happens to
/// be a valid global builtin) would otherwise sail through `derive_verdict`
/// unchallenged.
///
/// Checks performed (a lightweight SYNTAX check, not full type inference of
/// the receiver variable's declared type — the shadow-absence and
/// catalog-membership checks in `derive_verdict` already bound that; this
/// only needs to catch a Global-vs-member/page-instance MISLABEL):
/// - `Global` receiver_kind ⟺ `callee_text` has no `.`.
/// - `PageInstance`/`Record`/`RecordRef` receiver_kind ⟺ `callee_text` has a
///   `.` (a member call).
/// - For a member call with `receiver_kind == "PageInstance"`, the receiver
///   token (text before the final `.`) must be `CurrPage` or `Page` — the
///   only page-instance forms this overlay uses.
/// - In both shapes, the parsed method token must match `catalog_key`'s
///   method component (the part after `::`, or the whole key for a bare
///   global).
///
/// Panics via `assert!`/`assert_eq!` on any mismatch — a hard, load-bearing
/// check, not advisory.
fn assert_shape_matches_receiver_kind(ov: &AdjudicatedOverride) {
    let expected_method_lc = ov
        .catalog_key
        .rsplit("::")
        .next()
        .unwrap_or(&ov.catalog_key)
        .to_ascii_lowercase();
    match parse_callee_shape(&ov.callee_text) {
        CallShape::Global(method) => {
            assert_eq!(
                ov.receiver_kind, "Global",
                "{}:{}: callee_text {:?} is a bare (dot-free) call, but receiver_kind is \
                 {:?} not \"Global\" — shape/receiver_kind mismatch",
                ov.unit, ov.line, ov.callee_text, ov.receiver_kind,
            );
            assert_eq!(
                method.to_ascii_lowercase(),
                expected_method_lc,
                "{}:{}: callee_text {:?} does not match catalog_key {:?}'s method component",
                ov.unit,
                ov.line,
                ov.callee_text,
                ov.catalog_key,
            );
        }
        CallShape::Member { receiver, method } => {
            assert!(
                matches!(
                    ov.receiver_kind.as_str(),
                    "PageInstance" | "Record" | "RecordRef"
                ),
                "{}:{}: callee_text {:?} is a member call (`<receiver>.<method>`), but \
                 receiver_kind is {:?} — expected PageInstance/Record/RecordRef",
                ov.unit,
                ov.line,
                ov.callee_text,
                ov.receiver_kind,
            );
            if ov.receiver_kind == "PageInstance" {
                assert!(
                    receiver.eq_ignore_ascii_case("CurrPage")
                        || receiver.eq_ignore_ascii_case("Page"),
                    "{}:{}: PageInstance member call {:?} has receiver token {:?}, expected \
                     CurrPage or Page (the page-instance forms this overlay uses)",
                    ov.unit,
                    ov.line,
                    ov.callee_text,
                    receiver,
                );
            }
            assert_eq!(
                method.to_ascii_lowercase(),
                expected_method_lc,
                "{}:{}: callee_text {:?}'s method {:?} does not match catalog_key {:?}'s \
                 method component",
                ov.unit,
                ov.line,
                ov.callee_text,
                method,
                ov.catalog_key,
            );
        }
    }
}

/// Count the top-level (paren/quote-depth-aware) comma-separated arguments
/// of the call to `callee_text` found on `line_text` — an independent arity
/// cross-check against `ov.arity`, so that field is load-bearing rather than
/// vestigial (review-fix, beyond-1B.3b Task 3 fix pass).
///
/// Returns `None` — a deliberate, conservative bail-out, NOT arity 0 — when
/// `callee_text` isn't immediately followed by `(` on this line, or when the
/// argument list doesn't close before line end (e.g. a call whose arguments
/// wrap onto a following line). Robustly counting arguments from source text
/// is not reliable for every call form; callers must treat `None` as
/// "cannot cross-check this site", never as a synthesized answer.
///
/// Quote-aware (both `'...'` string literals and `"..."` quoted
/// identifiers, including the AL doubled-quote escape `''`/`""`) so commas
/// inside string/identifier literals are never miscounted, and
/// paren-depth-aware so a nested call's arguments (e.g. `CopyStr(X, 1,
/// MaxStrLen(X))`) are not double-counted at the outer level.
fn count_call_arity_on_line(line_text: &str, callee_text: &str) -> Option<usize> {
    let lc_line = line_text.to_ascii_lowercase();
    let lc_callee = callee_text.to_ascii_lowercase();
    let start = lc_line.find(&lc_callee)?;
    let after_callee = start + callee_text.len();
    let bytes = line_text.as_bytes();

    let mut i = after_callee;
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    if bytes.get(i) != Some(&b'(') {
        return None; // not a call at this occurrence — cannot cross-check
    }
    i += 1; // past the opening '('
    let arg_start = i;

    let mut depth = 1i32;
    let mut quote: Option<u8> = None;
    let mut commas_at_top = 0usize;
    let mut close_idx = None;
    while i < bytes.len() {
        let c = bytes[i];
        if let Some(q) = quote {
            if c == q {
                if bytes.get(i + 1) == Some(&q) {
                    i += 2; // doubled-quote escape — stays inside the quote
                    continue;
                }
                quote = None;
            }
            i += 1;
            continue;
        }
        match c {
            b'\'' | b'"' => quote = Some(c),
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    close_idx = Some(i);
                    break;
                }
            }
            b',' if depth == 1 => commas_at_top += 1,
            _ => {}
        }
        i += 1;
    }

    let close_idx = close_idx?; // unbalanced by line end — bail out, don't guess
    let inner = line_text[arg_start..close_idx].trim();
    if inner.is_empty() {
        Some(0)
    } else {
        Some(commas_at_top + 1)
    }
}

/// Whether a source-level top-level comma count is a SOUND oracle for the
/// overlay's recorded `arity` at this call (review-fix, beyond-1B.3b Task 3
/// fix pass).
///
/// It is NOT sound for the "object-run static" dispatch forms —
/// `Page.RunModal` / `Page.Run` / `Report.Run` / `Report.RunModal` /
/// `Codeunit.Run` / `Query.Open` / `XmlPort.*` — whose FIRST syntactic
/// argument is an object DESIGNATOR (`Page::"…"`) rather than a value
/// argument. Whether that designator counts toward "arity" is a convention
/// the committed overlay does NOT fix consistently: the two `Page.RunModal`
/// entries disagree — `Page.RunModal(Page::"User Setup")` records arity 1
/// (counting the designator), while `Page.RunModal(Page::"CDO Field List",
/// Field)` records arity 1 (NOT counting it, i.e. only the record). Because
/// `arity` is descriptive metadata only (it is NOT part of the site key and
/// is never consumed by `apply_adjudicated_overrides`/the audit), this
/// inconsistency is cosmetic; rather than false-fail a valid entry on an
/// ambiguous convention we skip the numeric arity oracle for exactly these
/// forms and document it (the shape/receiver-kind cross-check STILL runs for
/// them). For every OTHER call form — bare globals and `CurrPage`/Record/
/// RecordRef member calls — the parenthesized arguments are all value
/// arguments and the count is a sound oracle.
fn arity_source_count_is_sound(callee_text: &str) -> bool {
    match parse_callee_shape(callee_text) {
        CallShape::Member { receiver, .. } => !matches!(
            receiver.to_ascii_lowercase().as_str(),
            "page" | "report" | "codeunit" | "query" | "xmlport"
        ),
        CallShape::Global(_) => true,
    }
}

/// beyond-1B.3b Task 3: for every entry in the committed adjudication overlay
/// (`adjudicated-overrides.json`), INDEPENDENTLY re-derive/cross-check it
/// from LIVE CDO source + the structural builtin catalog (never from
/// fresh's output, never from this override's own committed fields) and
/// assert agreement. Concretely, for each entry this test:
///
/// 1. Re-hashes the unit at test time and FAILS LOUDLY on any
///    `source_sha256` mismatch (source drift — CDO_WS is a dirty live
///    workspace with uncommitted edits) rather than silently trusting a
///    possibly-stale adjudication.
/// 2. Confirms `callee_text` still appears on the claimed line (line-drift
///    catch).
/// 3. Cross-checks the call SHAPE parsed straight from `callee_text` against
///    `receiver_kind` and `catalog_key`'s method component
///    ([`assert_shape_matches_receiver_kind`]) — BEFORE anything downstream
///    is allowed to trust `receiver_kind` as given. Catches a
///    Global-vs-member (and page-instance) mislabel.
/// 4. Cross-checks `arity` against an independently-counted top-level
///    argument count parsed from the call site
///    ([`count_call_arity_on_line`]), when that count can be determined
///    soundly from the single source line (a conservative bail-out
///    otherwise — see that function's doc comment).
/// 5. Re-derives the `verdict` itself ([`derive_verdict`]) from the
///    structural catalog + a same-unit source-shadow scan, and asserts it
///    matches the committed value.
///
/// This does NOT re-derive every field of [`AdjudicatedOverride`] — the site
/// KEY fields (`from_app_guid`/`from_object_kind`/`from_object_lc`/
/// `from_routine_lc`/`edge_kind`/`unit`/`line`/`callee_fp`) are identity
/// fields used only to locate the site, not independently re-computed facts.
///
/// Fail-closed: ANY `needs_manual_review` or `fresh_false_builtin` survivor
/// is a real bug (a mis-adjudicated site, or a genuine fresh-catalog gap
/// Tasks 1-2 should have absorbed) and fails the test — never auto-passed.
#[test]
fn cdo_genuine_wrong_is_precedence_adjudicated() {
    let Some(ws) = cdo_ws_or_enforce() else {
        return;
    };

    let overrides =
        load_adjudicated_overrides(&adjudicated_overrides_path()).unwrap_or_else(|| {
            panic!(
                "adjudicated-overrides.json missing/invalid at {}",
                adjudicated_overrides_path().display(),
            )
        });
    assert!(
        !overrides.entries.is_empty(),
        "adjudicated-overrides.json must be non-empty"
    );

    let mut l3_error_intrinsic = 0usize;
    let mut fresh_false_builtin = 0usize;
    let mut needs_manual_review = 0usize;

    for ov in &overrides.entries {
        let path = ws.join(&ov.unit);
        let content = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read adjudicated unit {}: {e}", path.display()));

        // ── source_sha256 drift check — FAIL, never silently skip ──────────
        let mut hasher = Sha256::new();
        hasher.update(content.as_bytes());
        let actual_sha: String = hasher
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(
            actual_sha,
            ov.source_sha256,
            "SOURCE DRIFT at {} ({}:{}): the CDO unit has changed since this adjudication \
             was recorded. Re-verify the site against the CURRENT source, then update \
             adjudicated-overrides.json's source_sha256 (and re-derive the verdict if the \
             call site itself changed).",
            path.display(),
            ov.unit,
            ov.line,
        );

        // ── callee_text sanity: still on the claimed (1-based) line ────────
        let line_1based = ov.line as usize + 1;
        let lines: Vec<&str> = content.lines().collect();
        let line_text = lines.get(line_1based - 1).copied().unwrap_or("");
        assert!(
            line_text
                .to_ascii_lowercase()
                .contains(&ov.callee_text.to_ascii_lowercase()),
            "callee_text {:?} not found on {}:{} (line drifted?) — line reads: {:?}",
            ov.callee_text,
            ov.unit,
            line_1based,
            line_text,
        );

        // ── shape / receiver_kind cross-check — BEFORE trusting either ──────
        assert_shape_matches_receiver_kind(ov);

        // ── arity cross-check — BEFORE trusting `arity` ─────────────────────
        // Only where source-level comma counting is a sound oracle for the
        // recorded arity (see `arity_source_count_is_sound`: the object-run
        // static forms carry an object-designator first argument whose
        // arity convention the overlay does not fix, so they are skipped).
        if arity_source_count_is_sound(&ov.callee_text) {
            match count_call_arity_on_line(line_text, &ov.callee_text) {
                Some(counted_arity) => {
                    assert_eq!(
                        counted_arity, ov.arity,
                        "{}:{}: counted {counted_arity} top-level argument(s) for {:?} at the \
                         call site, but the committed arity is {} — arity cross-check mismatch \
                         (re-verify the site)",
                        ov.unit, ov.line, ov.callee_text, ov.arity,
                    );
                }
                None => {
                    eprintln!(
                        "NOTE: arity cross-check skipped for {}:{} ({:?}) — could not robustly \
                         parse a single-line balanced argument list (conservative bail-out, not \
                         a failure)",
                        ov.unit, ov.line, ov.callee_text,
                    );
                }
            }
        } else {
            eprintln!(
                "NOTE: arity cross-check skipped for {}:{} ({:?}) — object-run static dispatch \
                 form (object-designator first argument makes source comma count an unsound \
                 arity oracle; shape/receiver-kind still checked)",
                ov.unit, ov.line, ov.callee_text,
            );
        }

        // ── independent verdict re-derivation ───────────────────────────────
        let verdict = derive_verdict(ov, &content);
        assert_eq!(
            verdict, ov.verdict,
            "independently-derived verdict for {}:{} (catalog_key={:?}, receiver_kind={:?}) \
             is {:?}, but the committed adjudication says {:?} — re-investigate before \
             trusting the overlay.",
            ov.unit, ov.line, ov.catalog_key, ov.receiver_kind, verdict, ov.verdict,
        );

        match verdict {
            "l3_error_intrinsic" => l3_error_intrinsic += 1,
            "fresh_false_builtin" => fresh_false_builtin += 1,
            _ => needs_manual_review += 1,
        }
    }

    eprintln!(
        "Test 19b — independent source adjudication: l3_error_intrinsic={l3_error_intrinsic} \
         fresh_false_builtin={fresh_false_builtin} needs_manual_review={needs_manual_review} \
         (total={})",
        overrides.entries.len(),
    );

    assert_eq!(
        needs_manual_review, 0,
        "needs_manual_review must be 0 — any survivor is an unresolved adjudication \
         dimension (fail-closed, never auto-passed)"
    );
    assert_eq!(
        fresh_false_builtin, 0,
        "fresh_false_builtin must be 0 — any survivor is a real fresh-catalog bug that \
         Tasks 1-2 should have absorbed (source shadows catalog, or the claimed catalog \
         member doesn't actually exist)"
    );
}

// ---------------------------------------------------------------------------
// Test 20 (fixture + CDO env-gated): 1B.3b Task 2 — fan-out applicability
// SOUNDNESS teeth (ported into route_applicability)
// ---------------------------------------------------------------------------

/// Asserts the four 1B.3b Task 2 fan-out SOUNDNESS counters
/// (`interface_applicability_violations` / `instance_builtin_violations` /
/// `implicit_trigger_violations` / `event_violations`) are all 0 over the
/// `fanout-applicability` fixture, which genuinely exercises all four
/// dispatch kinds end-to-end through `resolve_full_program` (not
/// hand-constructed edges — see `semantic_golden.rs`'s own unit tests for the
/// hand-built positive/negative predicate-level proof that the teeth bite).
/// Also asserts `resolve_full_program`'s fixture output actually contains a
/// Polymorphic Call edge, a Multicast ImplicitTrigger edge, an EventFlow edge
/// with >=1 Routine route, and a `PageInstance::` Builtin route — so this
/// assertion is not vacuous.
///
/// SOUNDNESS, not completeness: these check that every fan-out route the
/// resolver DID emit is individually well-formed/applicable — NOT that the
/// resolver emitted every route it should have (that's the frozen,
/// L3-validated goldens' job — 1B.3a/1B.3b Task 1). Distinct from
/// `route_applicability_zero_violations` (Test 15)'s structural
/// witness-contract/ABI checks; `ApplicabilityReport::is_clean()` now folds
/// both families together, so Test 15 already covers this on the same
/// fixture/CDO inputs — this test adds the targeted per-kind assertions and
/// the fixture-exercises-every-kind sanity check.
#[test]
fn fan_out_applicability_zero_violations() {
    // ── Fixture (no env needed) ───────────────────────────────────────────────
    let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/fanout-applicability");

    let program_report =
        resolve_full_program(&fixture).expect("resolve_full_program must succeed on fixture");
    let has_polymorphic_call = program_report
        .edges
        .iter()
        .any(|ce| ce.edge.kind == EdgeKind::Call && ce.edge.shape == DispatchShape::Polymorphic);
    let has_multicast_trigger = program_report.edges.iter().any(|ce| {
        ce.edge.kind == EdgeKind::ImplicitTrigger && ce.edge.shape == DispatchShape::Multicast
    });
    let has_event_flow_route = program_report.edges.iter().any(|ce| {
        ce.edge.kind == EdgeKind::EventFlow
            && ce
                .edge
                .routes
                .iter()
                .any(|r| matches!(r.target, RouteTarget::Routine(_)))
    });
    let has_page_instance_builtin = program_report.edges.iter().any(|ce| {
        ce.edge.routes.iter().any(
            |r| matches!(&r.target, RouteTarget::Builtin(b) if b.0.starts_with("PageInstance::")),
        )
    });
    assert!(
        has_polymorphic_call,
        "fixture must exercise an Interface (Polymorphic) Call edge"
    );
    assert!(
        has_multicast_trigger,
        "fixture must exercise a Multicast ImplicitTrigger edge"
    );
    assert!(
        has_event_flow_route,
        "fixture must exercise an EventFlow edge with a Routine route"
    );
    assert!(
        has_page_instance_builtin,
        "fixture must exercise a PageInstance:: instance-builtin route"
    );

    let appl = run_route_applicability(&fixture);
    assert_eq!(
        appl.interface_applicability_violations, 0,
        "Interface fan-out soundness violated on fixture"
    );
    assert_eq!(
        appl.instance_builtin_violations, 0,
        "instance-builtin/enum-static soundness violated on fixture"
    );
    assert_eq!(
        appl.implicit_trigger_violations, 0,
        "ImplicitTrigger fan-out soundness violated on fixture"
    );
    assert_eq!(
        appl.event_violations, 0,
        "EventFlow soundness violated on fixture"
    );
    assert!(
        appl.is_clean(),
        "route-applicability contract violated on fixture: {appl:?}"
    );
    eprintln!(
        "Test 20 (fixture) — fan-out applicability: interface=0 instance_builtin=0 \
         implicit_trigger=0 event=0 (total_routes={}) \
         routes_checked[interface={} instance_builtin={} implicit_trigger={} event={}]",
        appl.total_routes,
        appl.interface_routes_checked,
        appl.instance_builtin_routes_checked,
        appl.implicit_trigger_routes_checked,
        appl.event_routes_checked,
    );

    // ── CDO (env-gated) ───────────────────────────────────────────────────────
    let Some(ws) = std::env::var_os("CDO_WS")
        .map(std::path::PathBuf::from)
        .filter(|p| p.exists())
    else {
        return;
    };

    let appl_cdo = run_route_applicability(&ws);
    assert_eq!(
        appl_cdo.interface_applicability_violations, 0,
        "Interface fan-out soundness violated on CDO_WS — a real bug, investigate \
         and fix (do not relax)."
    );
    assert_eq!(
        appl_cdo.instance_builtin_violations, 0,
        "instance-builtin/enum-static soundness violated on CDO_WS — investigate."
    );
    assert_eq!(
        appl_cdo.implicit_trigger_violations, 0,
        "ImplicitTrigger fan-out soundness violated on CDO_WS — investigate."
    );
    assert_eq!(
        appl_cdo.event_violations, 0,
        "EventFlow soundness violated on CDO_WS — investigate."
    );
    // ── Fix 2 (1B.3b whole-branch fix): non-vacuity must be ASSERTED, not just
    // printed. `violations == 0` is meaningless if `routes_checked == 0` — a
    // `build_fan_out_site_context` regression silently dropping context would
    // collapse every denominator to 0 and pass vacuously. Fail closed instead.
    assert!(
        appl_cdo.interface_routes_checked > 0
            && appl_cdo.instance_builtin_routes_checked > 0
            && appl_cdo.implicit_trigger_routes_checked > 0
            && appl_cdo.event_routes_checked > 0,
        "VACUOUS PASS: routes_checked[interface={} instance_builtin={} \
         implicit_trigger={} event={}] must all be NON-TRIVIAL (>0) — a \
         collapse toward 0 with violations==0 signals a build_fan_out_site_context \
         regression silently dropping context, not a genuine clean run.",
        appl_cdo.interface_routes_checked,
        appl_cdo.instance_builtin_routes_checked,
        appl_cdo.implicit_trigger_routes_checked,
        appl_cdo.event_routes_checked,
    );
    eprintln!(
        "Test 20 (CDO) — fan-out applicability: total_routes={} \
         interface_violations={} instance_builtin_violations={} \
         implicit_trigger_violations={} event_violations={} (all must be 0)\n\
         Test 20 (CDO) — non-vacuity routes_checked: interface={} instance_builtin={} \
         implicit_trigger={} event={} (each must be NON-TRIVIAL — a collapse toward 0 \
         with violations==0 would signal a vacuous pass, e.g. a \
         build_fan_out_site_context regression silently dropping context)",
        appl_cdo.total_routes,
        appl_cdo.interface_applicability_violations,
        appl_cdo.instance_builtin_violations,
        appl_cdo.implicit_trigger_violations,
        appl_cdo.event_violations,
        appl_cdo.interface_routes_checked,
        appl_cdo.instance_builtin_routes_checked,
        appl_cdo.implicit_trigger_routes_checked,
        appl_cdo.event_routes_checked,
    );
}

// ---------------------------------------------------------------------------
// Tests 21+: beyond-1B.3b Task 1 — source shadows builtin (lookup precedence)
// + structural builtin-catalog match, end-to-end over `ws-builtin-shadow`.
//
// Root-cause fix: `resolve_member`'s Record arm was catalog-FIRST (a user
// table method whose name+arity matched a genuine Record builtin was
// mis-classified `Catalog` instead of the local `Source`). AL semantics: a
// visible source/ABI routine of matching name+arity SHADOWS a same-named
// intrinsic. This is the exact shape behind the 42 real CDO
// `builtin-catalog-fp-collision` divergences.
// ---------------------------------------------------------------------------

/// Loads `tests/r0-corpus/ws-builtin-shadow` and returns the full
/// `resolve_full_program` report — shared by Tests 21a-21e below.
fn ws_builtin_shadow_report() -> ProgramReport {
    let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/r0-corpus/ws-builtin-shadow");
    resolve_full_program(&fixture).expect("resolve_full_program must succeed on ws-builtin-shadow")
}

/// All classified edges whose call-site obligation's caller routine has
/// `name_lc == caller_name_lc` (case-insensitive by construction — callers
/// pass already-lowercased names).
fn edges_for_caller<'a>(
    report: &'a ProgramReport,
    caller_name_lc: &str,
) -> Vec<&'a ClassifiedEdge> {
    report
        .edges
        .iter()
        .filter(|ce| match &ce.obligation_id {
            ObligationId::CallSite { caller, .. } => caller.name_lc == caller_name_lc,
            ObligationId::Publisher(_) => false,
        })
        .collect()
}

/// Test 21a (fixture a): `R.FieldNo('No.')` on a `Record Acme` whose table
/// declares its OWN `FieldNo(FieldName: Text): Integer` (arity 1, matching the
/// call) must resolve to `Acme.FieldNo` with `Evidence::Source` — NOT the
/// `Record::fieldno` catalog builtin (today, pre-fix: catalog-first → false
/// `builtin`).
#[test]
fn ws_builtin_shadow_record_member_source_shadows_catalog() {
    let report = ws_builtin_shadow_report();
    let edges = edges_for_caller(&report, "calla");
    assert_eq!(
        edges.len(),
        1,
        "CallA must have exactly one call obligation"
    );
    let routes = &edges[0].edge.routes;
    assert_eq!(
        routes.len(),
        1,
        "Record member call is single-dispatch (Exact)"
    );
    let route = &routes[0];

    assert_eq!(
        route.evidence,
        Evidence::Source,
        "local Acme.FieldNo must SHADOW the Record::fieldno catalog builtin; got {route:?}"
    );
    let RouteTarget::Routine(ref rid) = route.target else {
        panic!("expected RouteTarget::Routine, got {:?}", route.target);
    };
    assert_eq!(rid.name_lc, "fieldno");
    assert_eq!(rid.object.kind, ObjectKind::Table);
    assert_eq!(rid.params_count, 1);
    assert!(
        matches!(route.witness, Witness::SourceSpan { .. }),
        "witness must be SourceSpan; got {:?}",
        route.witness
    );
}

/// Test 21b (fixture b): bare `Error('boom')` inside a Codeunit that ALSO
/// declares a local `procedure Error(Msg: Text)` must resolve to the LOCAL
/// `Error` (`Evidence::Source`), not the `error` global intrinsic.
///
/// NOTE: `resolve_bare`'s own-object-first precedence (module doc,
/// `resolver.rs:1-12`) already implemented this correctly BEFORE the Task 1
/// fix — the only genuinely catalog-FIRST arm was `resolve_member`'s Record
/// arm (Test 21a). This is kept as the brief-mandated regression-locking
/// fixture, not as a second bug reproduction.
#[test]
fn ws_builtin_shadow_bare_source_shadows_catalog() {
    let report = ws_builtin_shadow_report();
    let edges = edges_for_caller(&report, "callb");
    assert_eq!(
        edges.len(),
        1,
        "CallB must have exactly one call obligation"
    );
    let routes = &edges[0].edge.routes;
    assert_eq!(routes.len(), 1);
    let route = &routes[0];

    assert_eq!(
        route.evidence,
        Evidence::Source,
        "local ShadowCallerB.Error must SHADOW the global `error` intrinsic; got {route:?}"
    );
    let RouteTarget::Routine(ref rid) = route.target else {
        panic!("expected RouteTarget::Routine, got {:?}", route.target);
    };
    assert_eq!(rid.name_lc, "error");
    assert_eq!(rid.object.kind, ObjectKind::Codeunit);
}

/// Test 21c (fixture c): genuine builtins with ZERO source competitors
/// anywhere in the workspace must STAY `Catalog` after the precedence fix —
/// `R.FieldCaption(1)` (Record builtin), bare `Message('hi')` (global
/// builtin), and `J.Add(...)` on a `JsonObject` (framework-member builtin).
///
/// NOTE: deliberately NOT a `record_op_names()` method (`SetRange`/`Insert`/
/// `Modify`/...) — those 28 names are classified `CalleeShape::RecordOp`
/// (`extract.rs`) and resolved via the SEPARATE implicit-trigger path, not
/// `resolve_member`'s Record arm; `FieldCaption` exercises the Record-arm
/// catalog fallback this test targets.
#[test]
fn ws_builtin_shadow_genuine_builtins_stay_catalog() {
    let report = ws_builtin_shadow_report();
    let edges = edges_for_caller(&report, "callc");
    assert_eq!(edges.len(), 3, "CallC has 3 call obligations");

    let mut catalog_ids: Vec<String> = Vec::new();
    for ce in &edges {
        assert_eq!(ce.edge.routes.len(), 1);
        let route = &ce.edge.routes[0];
        assert_eq!(
            route.evidence,
            Evidence::Catalog,
            "genuine builtin with no source competitor must stay Catalog; got {route:?}"
        );
        let RouteTarget::Builtin(BuiltinId(ref id)) = route.target else {
            panic!("expected RouteTarget::Builtin, got {:?}", route.target);
        };
        assert!(
            matches!(route.witness, Witness::CatalogEntry { .. }),
            "witness must be CatalogEntry; got {:?}",
            route.witness
        );
        catalog_ids.push(id.clone());
    }
    catalog_ids.sort();
    assert_eq!(
        catalog_ids,
        vec![
            "JsonObject::add".to_string(),
            "Record::fieldcaption".to_string(),
            "message".to_string(),
        ],
        "all three genuine-builtin call sites must resolve to their expected catalog ids"
    );
}

/// Test 21d (fixture d): a near-miss name (`ZzNotARealBuiltinFp`, not a real
/// catalog member despite being textually adjacent to real builtins) must NOT
/// be classified `builtin` — falls through to honest `Unknown`. Locks in that
/// the catalog match is exact-string (no fingerprint/hash digest — see
/// `builtins.rs`/`member_catalog.rs`), so a fabricated "fingerprint collision"
/// cannot surface as a false `builtin`.
#[test]
fn ws_builtin_shadow_near_miss_name_is_not_classified_builtin() {
    let report = ws_builtin_shadow_report();
    let edges = edges_for_caller(&report, "calld");
    assert_eq!(
        edges.len(),
        1,
        "CallD must have exactly one call obligation"
    );
    let routes = &edges[0].edge.routes;
    assert_eq!(routes.len(), 1);
    let route = &routes[0];

    assert_eq!(
        route.target,
        RouteTarget::Unresolved,
        "near-miss name must NOT resolve to any target; got {:?}",
        route.target
    );
    assert_eq!(
        route.evidence,
        Evidence::Unknown,
        "near-miss name must NOT be classified Catalog; got {route:?}"
    );
    assert_eq!(route.witness, Witness::None);
}

/// Test 21e (fixture e): qualified-intrinsic bypass. `CreateGuid()` (bare,
/// inside a Codeunit that ALSO declares a local `procedure CreateGuid(): Guid`)
/// must resolve to the LOCAL Source (shadowing the global `createguid`
/// intrinsic); `System.CreateGuid()` (fully qualified) must STILL bind to the
/// `System::createguid` Catalog entry — the local declaration does NOT shadow
/// a qualified platform call, because `System.*` is dispatched via the
/// `Framework(System)` singleton receiver, which never consults source
/// candidates (a structurally distinct path from the bare-call shadow check;
/// see `ws-builtin-shadow` Step-3 investigation note in the Task 1 report).
#[test]
fn ws_builtin_shadow_qualified_intrinsic_bypasses_local_shadow() {
    let report = ws_builtin_shadow_report();
    let edges = edges_for_caller(&report, "calle");
    assert_eq!(
        edges.len(),
        2,
        "CallE has 2 call obligations (bare + qualified)"
    );

    let mut source_hit = false;
    let mut catalog_hit = false;
    for ce in &edges {
        assert_eq!(ce.edge.routes.len(), 1);
        let route = &ce.edge.routes[0];
        match route.evidence {
            Evidence::Source => {
                source_hit = true;
                let RouteTarget::Routine(ref rid) = route.target else {
                    panic!("expected RouteTarget::Routine, got {:?}", route.target);
                };
                assert_eq!(rid.name_lc, "createguid");
                assert_eq!(rid.object.kind, ObjectKind::Codeunit);
            }
            Evidence::Catalog => {
                catalog_hit = true;
                let RouteTarget::Builtin(BuiltinId(ref id)) = route.target else {
                    panic!("expected RouteTarget::Builtin, got {:?}", route.target);
                };
                assert_eq!(
                    id, "System::createguid",
                    "qualified call must bind to the System Framework catalog entry"
                );
            }
            other => panic!("unexpected evidence {other:?} on CallE route: {route:?}"),
        }
    }
    assert!(source_hit, "bare CreateGuid() must resolve to local Source");
    assert!(
        catalog_hit,
        "System.CreateGuid() must resolve to Catalog despite the local shadow"
    );
}

// ---------------------------------------------------------------------------
// Test 22: beyond-1B.3b Task 1 REVIEW FIX (Finding 1) — base-table wrong-arity
// falls through to a TableExtension's matching-arity overload.
//
// The precedence rewrite (Test 21a) made a real, correct, but previously
// undisclosed secondary behavior change: pre-fix, ANY name match on the base
// table (regardless of arity) short-circuited the Record arm straight to that
// base-table routine (or, in the catalog-first world, to the catalog) without
// ever considering a TableExtension. Post-fix, `object_has_member_candidate`
// requires an EXACT arity match for Source/ABI-tier objects, so a base-table
// name-only match with the wrong arity is no longer a candidate at all — the
// scope walk correctly falls through to a TableExtension that DOES declare
// the matching arity. `tests/r0-corpus/ws-builtin-shadow-arity/`: `BaseTable`
// declares `Foo()` (arity 0); `BaseTableExt` (a TableExtension of it)
// declares `Foo(X: Integer)` (arity 1); the caller does `R.Foo(5)` (arity 1).
// ---------------------------------------------------------------------------

/// Loads `tests/r0-corpus/ws-builtin-shadow-arity` and returns the full
/// `resolve_full_program` report.
fn ws_builtin_shadow_arity_report() -> ProgramReport {
    let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/r0-corpus/ws-builtin-shadow-arity");
    resolve_full_program(&fixture)
        .expect("resolve_full_program must succeed on ws-builtin-shadow-arity")
}

/// `R.Foo(5)` (arity 1) must resolve to `BaseTableExt.Foo` (`Evidence::Source`)
/// — the base table's `Foo()` (arity 0) name-matches but arity-mismatches, so
/// it must NOT short-circuit the scope walk; it must NOT resolve to
/// `Unresolved`/`Unknown` either (that would mean the wrong-arity base-table
/// match incorrectly suppressed the extension candidate, or the ambiguity
/// branch incorrectly fired for what is actually a single valid candidate).
///
/// Sanity-checked by reasoning against `object_has_member_candidate`
/// (`resolver.rs`): for a Source-tier object, `candidates.iter().any(|rid|
/// rid.params_count == arity)` — the base table's ONLY `foo` candidate has
/// `params_count == 0 != 1`, so `object_has_member_candidate` returns `false`
/// for the base table and the arity-1 scan advances to the TableExtension,
/// which has exactly one `params_count == 1` match. If the pre-fix
/// any-name-match short-circuit were reintroduced (matching on name alone,
/// ignoring arity), the base table would wrongly become the sole "candidate"
/// found and `resolve_in_object` would either mis-resolve to the arity-0
/// `Foo` or fail to find an arity-1 routine there and fall through to the
/// Record builtin catalog (there is no arity-1 Record builtin named `Foo`),
/// landing on `Unresolved`/`Unknown` — either way NOT this test's asserted
/// `Evidence::Source` + `TableExtension` target, so this test fails under
/// that regression.
#[test]
fn ws_builtin_shadow_arity_base_wrong_arity_falls_through_to_extension() {
    let report = ws_builtin_shadow_arity_report();
    let edges = edges_for_caller(&report, "callfoo");
    assert_eq!(
        edges.len(),
        1,
        "CallFoo must have exactly one call obligation"
    );
    let routes = &edges[0].edge.routes;
    assert_eq!(
        routes.len(),
        1,
        "Record member call is single-dispatch (Exact)"
    );
    let route = &routes[0];

    assert_eq!(
        route.evidence,
        Evidence::Source,
        "arity-1 call must resolve to the TableExtension's Foo(X: Integer), \
         not fall through to Unknown or mis-hit the base table's arity-0 Foo(); \
         got {route:?}"
    );
    let RouteTarget::Routine(ref rid) = route.target else {
        panic!(
            "expected RouteTarget::Routine (the TableExtension's Foo), got {:?}",
            route.target
        );
    };
    assert_eq!(rid.name_lc, "foo");
    assert_eq!(rid.params_count, 1);
    assert_eq!(
        rid.object.kind,
        ObjectKind::TableExtension,
        "must resolve to the TableExtension's overload, not the base table's; got {:?}",
        rid.object.kind
    );
    assert!(
        matches!(route.witness, Witness::SourceSpan { .. }),
        "witness must be SourceSpan; got {:?}",
        route.witness
    );
}

// ---------------------------------------------------------------------------
// Tests 23+: beyond-1B.3b Task 2 — fail-closed same-arity SOURCE-overload
// guard. `resolve_in_object` used to pick the FIRST arity-matched candidate
// with no ambiguity check; worse, two same-name/same-arity SOURCE overloads
// collapse to one `RoutineNodeId` (source `sig_fp` is always `0`), so
// `build_program_graph`'s post-sort dedup could silently drop one of them.
// `tests/r0-corpus/ws-overload-collision/`: `Ambiguous.Codeunit.al` declares
// two `Resolve` overloads (arity 1, differing only by param TYPE); `Caller`
// invokes `Target.Resolve(5)` (member-Object dispatch). A single-overload
// `Control.Codeunit.al` proves the guard does not over-fire.
// ---------------------------------------------------------------------------

/// Loads `tests/r0-corpus/ws-overload-collision` and returns the full
/// `resolve_full_program` report — shared by Tests 23a-23c below.
fn ws_overload_collision_report() -> ProgramReport {
    let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/r0-corpus/ws-overload-collision");
    resolve_full_program(&fixture)
        .expect("resolve_full_program must succeed on ws-overload-collision")
}

/// Test 23a: `Target.Resolve(5)` against a same-name/same-arity SOURCE
/// overload pair must NOT resolve to a confident `Source` route — no
/// arg-type evidence exists to pick between the two overloads (full arg-type
/// dispatch is out of scope for this task). Must be honest ambiguous/Unknown:
/// `RouteTarget::Unresolved` + `Evidence::Unknown`, never a guessed pick-first
/// route to either overload.
#[test]
fn ws_overload_collision_ambiguous_call_is_honest_unknown() {
    let report = ws_overload_collision_report();
    let edges = edges_for_caller(&report, "callambiguous");
    assert_eq!(
        edges.len(),
        1,
        "CallAmbiguous must have exactly one call obligation"
    );
    let routes = &edges[0].edge.routes;
    assert_eq!(
        routes.len(),
        1,
        "member-Object call is single-dispatch (Exact)"
    );
    let route = &routes[0];

    assert_eq!(
        route.target,
        RouteTarget::Unresolved,
        "an unresolvable same-arity overload set must NEVER pick a route by \
         guessing; got {route:?}"
    );
    assert_eq!(
        route.evidence,
        Evidence::Unknown,
        "no arg-type evidence exists to disambiguate the two `Resolve` \
         overloads — must be honest Unknown, not a confident Source \
         pick-first; got {route:?}"
    );
    assert_eq!(route.witness, Witness::None);
}

/// Test 23b: the graph must not silently DROP one of the two colliding
/// overloads. Builds the `ProgramGraph` directly (bypassing
/// `resolve_full_program`'s obligation layer) and asserts BOTH raw `Resolve`
/// entries survive `build_program_graph`'s post-sort dedup for the
/// `Ambiguous.Codeunit.al` object — the collision is marked/preserved, never
/// silently collapsed to one entry with no record.
#[test]
fn ws_overload_collision_graph_preserves_both_overloads() {
    use al_call_hierarchy::program::abi_ingest::AbiCache;
    use al_call_hierarchy::program::build::build_program_graph;
    use al_call_hierarchy::snapshot::SnapshotBuilder;

    let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/r0-corpus/ws-overload-collision");
    let snap = (SnapshotBuilder {
        workspace_root: fixture,
        local_providers: vec![],
    })
    .build()
    .expect("snapshot must build for ws-overload-collision");
    let cache = AbiCache::new();
    let graph = build_program_graph(&snap, &cache);

    let resolve_entries: Vec<_> = graph
        .routines
        .iter()
        .filter(|r| r.id.name_lc == "resolve")
        .collect();
    assert_eq!(
        resolve_entries.len(),
        2,
        "both `Resolve` overloads must survive the graph build — one must \
         NEVER be silently dropped by the post-sort dedup; got {} entries: {:?}",
        resolve_entries.len(),
        resolve_entries.iter().map(|r| &r.name).collect::<Vec<_>>()
    );
    assert!(
        resolve_entries.iter().all(|r| r.id.params_count == 1),
        "both overloads share arity 1 (the genuine collision shape); got {:?}",
        resolve_entries
            .iter()
            .map(|r| r.id.params_count)
            .collect::<Vec<_>>()
    );

    // Sanity: the single-overload control target must NOT be duplicated —
    // proves the preservation logic is collision-specific, not a blanket
    // "never dedup" change.
    let solo_entries: Vec<_> = graph
        .routines
        .iter()
        .filter(|r| r.id.name_lc == "solo")
        .collect();
    assert_eq!(
        solo_entries.len(),
        1,
        "the single-overload control routine must be exactly one entry \
         (no spurious ambiguity); got {} entries",
        solo_entries.len()
    );
}

/// Test 23c (control): a single-overload target (`Control.Solo`) must still
/// resolve cleanly to `Evidence::Source` — the ambiguity guard must not
/// over-fire on an ordinary, unambiguous procedure.
#[test]
fn ws_overload_collision_control_single_overload_resolves_cleanly() {
    let report = ws_overload_collision_report();
    let edges = edges_for_caller(&report, "callcontrol");
    assert_eq!(
        edges.len(),
        1,
        "CallControl must have exactly one call obligation"
    );
    let routes = &edges[0].edge.routes;
    assert_eq!(routes.len(), 1);
    let route = &routes[0];

    assert_eq!(
        route.evidence,
        Evidence::Source,
        "the single-overload control target must resolve cleanly; got {route:?}"
    );
    let RouteTarget::Routine(ref rid) = route.target else {
        panic!("expected RouteTarget::Routine, got {:?}", route.target);
    };
    assert_eq!(rid.name_lc, "solo");
    assert_eq!(rid.params_count, 1);
    assert_eq!(rid.object.kind, ObjectKind::Codeunit);
    assert!(
        matches!(route.witness, Witness::SourceSpan { .. }),
        "witness must be SourceSpan; got {:?}",
        route.witness
    );
}

// ---------------------------------------------------------------------------
// Tests 23d-23e: beyond-1B.3b Task 2 REVIEW FIX — compound object-duplication
// × genuine-overload dedup. `dedup_routines_preserving_genuine_overloads`
// used to be binary per run of equal-`RoutineNodeId` routines: collapse the
// WHOLE run to 1 when `run_len <= obj_dup`, else keep EVERY entry. When an
// object is embedded BOTH as workspace source AND as an embedded dep
// (`obj_dup=2`) AND that object declares a genuine same-name/same-arity
// overload pair (2 distinct source procedures colliding onto ONE
// `RoutineNodeId`, since source `sig_fp` is always `0`), the run holds 4 raw
// entries — `run_len(4) > obj_dup(2)` kept all 4 instead of the canonical 2.
// This inflated `graph.routines` and could push a legitimate single-target
// event subscription into `ambiguous_subscriptions` (candidate count 2
// instead of 1). The fix groups a run by the routine's PARAMETER-TYPE
// SIGNATURE before collapsing, so genuine re-parse duplicates collapse
// per-signature while genuine overloads (distinct signatures) are preserved
// — 2 canonical entries in every case, never 4.
// ---------------------------------------------------------------------------

/// Hand-builds an `AppSetSnapshot` with the SAME app identity appearing
/// TWICE — once as the workspace unit, once as a synthetic embedded-dep
/// unit — mirroring the real "sibling apps in a multi-app workspace whose
/// compiled .app lands in .alpackages" scenario `build_program_graph`'s Step
/// 4 comment documents (both units interning to the SAME `AppRef`). Both
/// units embed the identical `CompoundTarget.al` source, which declares a
/// genuine same-name/same-arity `Resolve` overload pair — one plain
/// `Resolve(Value: Integer)`, one `[IntegrationEvent]`-tagged
/// `Resolve(Value: Text)`. Only the workspace unit also carries the
/// subscriber file, so the compound duplication is isolated to the
/// `Compound Overload Target` object.
fn compound_overload_dup_snapshot() -> al_call_hierarchy::snapshot::AppSetSnapshot {
    use al_call_hierarchy::snapshot::compilation::CompilationContext;
    use al_call_hierarchy::snapshot::embedded::SourceFile;
    use al_call_hierarchy::snapshot::provider::SourceRoot;
    use al_call_hierarchy::snapshot::{AppSetSnapshot, AppUnit, World};

    let target_src = r#"
codeunit 50970 "Compound Overload Target"
{
    // Non-publisher overload — arity 1, param type Integer.
    procedure Resolve(Value: Integer)
    begin
    end;

    // Publisher overload — SAME name + SAME arity as the sibling above,
    // differing only by param TYPE (Text). Together they collide onto ONE
    // `RoutineNodeId` (source `sig_fp` is always 0).
    [IntegrationEvent(false, false)]
    procedure Resolve(Value: Text)
    begin
    end;
}
"#
    .to_string();

    let subscriber_src = r#"
codeunit 50971 "Compound Overload Subscriber"
{
    [EventSubscriber(ObjectType::Codeunit, Codeunit::"Compound Overload Target", 'Resolve', '', false, false)]
    procedure OnResolve(Value: Text)
    begin
    end;
}
"#
    .to_string();

    let app_id = AppId {
        guid: String::new(),
        name: "Compound App".into(),
        publisher: "Test".into(),
        version: "1.0.0.0".into(),
    };

    let ws_source = SourceRoot {
        files: vec![
            SourceFile {
                virtual_path: "CompoundTarget.al".into(),
                text: target_src.clone(),
            },
            SourceFile {
                virtual_path: "CompoundSubscriber.al".into(),
                text: subscriber_src,
            },
        ],
        tier: TrustTier::Workspace,
        content_hash: "ws-hash".into(),
    };
    // Synthetic "embedded dep" copy of the SAME source file — the exact
    // compound scenario `build_program_graph`'s Step 4 comment documents
    // ("Same app can appear as both a workspace source and an embedded dep").
    let dep_source = SourceRoot {
        files: vec![SourceFile {
            virtual_path: "CompoundTarget.al".into(),
            text: target_src,
        }],
        tier: TrustTier::EmbeddedSource,
        content_hash: "dep-hash".into(),
    };

    let ws_unit = AppUnit {
        id: app_id.clone(),
        provenance: Provenance {
            app: app_id.clone(),
            tier: TrustTier::Workspace,
            content_hash: "ws-hash".into(),
        },
        source: Some(ws_source),
        compilation: CompilationContext::default(),
        declared_deps: vec![],
        abi: None,
        app_path: None,
    };
    let dep_unit = AppUnit {
        id: app_id.clone(),
        provenance: Provenance {
            app: app_id.clone(),
            tier: TrustTier::EmbeddedSource,
            content_hash: "dep-hash".into(),
        },
        source: Some(dep_source),
        compilation: CompilationContext::default(),
        declared_deps: vec![],
        abi: None,
        app_path: None,
    };

    AppSetSnapshot {
        workspace_app: app_id,
        apps: vec![ws_unit, dep_unit],
        world: World::Closed,
    }
}

/// Test 23d: the compound duplication must collapse `graph.routines` to the
/// CANONICAL count (2 — one per genuine overload), never inflate to 4 (2
/// overloads × obj_dup 2). Proves the content-aware (param-signature) dedup
/// fix at the `build_program_graph` layer.
#[test]
fn compound_obj_dup_and_overload_dedups_to_canonical_count() {
    use al_call_hierarchy::program::abi_ingest::AbiCache;
    use al_call_hierarchy::program::build::build_program_graph;

    let snap = compound_overload_dup_snapshot();
    let cache = AbiCache::new();
    let graph = build_program_graph(&snap, &cache);

    let resolve_entries: Vec<_> = graph
        .routines
        .iter()
        .filter(|r| r.id.name_lc == "resolve")
        .collect();
    assert_eq!(
        resolve_entries.len(),
        2,
        "compound case (obj_dup=2 x 2 genuine overloads = 4 raw entries) must \
         collapse to the CANONICAL count of 2 -- one per genuine overload, \
         never inflate to 4; got {} entries: {:?}",
        resolve_entries.len(),
        resolve_entries.iter().map(|r| &r.name).collect::<Vec<_>>()
    );

    // Exactly one of the two canonical entries carries the publisher
    // attribute (the `[IntegrationEvent]`-tagged overload); the other does
    // not -- proves BOTH signature groups survived distinctly, not two
    // copies of the same one.
    let publisher_count = resolve_entries
        .iter()
        .filter(|r| r.publisher_kind.is_some())
        .count();
    assert_eq!(
        publisher_count,
        1,
        "exactly one canonical `Resolve` entry must carry the publisher \
         attribute; got {publisher_count} of {}",
        resolve_entries.len()
    );

    // The object itself must still be deduped to exactly one entry (Step 4's
    // existing unconditional `objects.dedup_by` -- unaffected by this fix).
    let target_objects: Vec<_> = graph
        .objects
        .iter()
        .filter(|o| o.name == "Compound Overload Target")
        .collect();
    assert_eq!(target_objects.len(), 1, "object dedup must be unaffected");
}

/// Test 23e: the compound duplication must NOT push the legitimate
/// single-target `OnResolve` subscription into `ambiguous_subscriptions`.
/// Before the fix, the inflated 4-entry run left 2 publisher-tagged raw
/// candidates (both from the SAME genuine overload, duplicated by `obj_dup`)
/// with equal arity, so `ResolveIndex::build`'s `>1` arm found no unique
/// strict-arity match and dropped the subscription as ambiguous.
#[test]
fn compound_obj_dup_and_overload_subscription_resolves_not_ambiguous() {
    use al_call_hierarchy::program::abi_ingest::AbiCache;
    use al_call_hierarchy::program::build::build_program_graph;
    use al_call_hierarchy::program::resolve::index::ResolveIndex;

    let snap = compound_overload_dup_snapshot();
    let cache = AbiCache::new();
    let graph = build_program_graph(&snap, &cache);
    let idx = ResolveIndex::build(&graph);

    assert!(
        idx.ambiguous_subscriptions().is_empty(),
        "a legitimate single-target subscription must NOT be pushed into \
         ambiguous_subscriptions by the compound obj_dup x overload \
         inflation; got {:?}",
        idx.ambiguous_subscriptions()
            .iter()
            .map(|a| (a.event_name_lc.clone(), a.candidate_count))
            .collect::<Vec<_>>()
    );

    let app = graph
        .apps
        .find_by_name("Compound App")
        .expect("app interned");
    let target_id = ObjectNodeId {
        app,
        kind: ObjectKind::Codeunit,
        key: ObjKey::Id(50970),
    };
    let publisher_id = RoutineNodeId {
        object: target_id,
        name_lc: "resolve".into(),
        enclosing_member_lc: None,
        params_count: 1,
        sig_fp: 0,
    };
    assert_eq!(
        idx.subscribers_of(&publisher_id).len(),
        1,
        "the legitimate subscriber must resolve to exactly one entry"
    );
}
