//! R2.5b-c EXIT-GATE — native CROSS-APP L3 event-graph resolution oracle.
//!
//! Ground-truth-free, STRUCTURAL oracles run NATIVELY against the Rust cross-app L3
//! event graph (`build_cross_app_l3_from_workspace` → the unchanged
//! `build_event_graph` over the merged workspace+`.app`-dep index →
//! `project_event_graph`). Each invariant asserts a SPECIFIC expected id DIRECTLY on
//! the resolved cross-app model — NOT "links to a dep event" (Rev 2 #1: an aggregate
//! `≥1` count passes even if BOTH sides linked to the WRONG dep publisher).
//!
//! ## Why a native oracle (not just the byte-parity differential)
//!
//! `r2_5b_eg_differential.rs` is byte-parity with al-sem: if BOTH engines made the
//! same cross-app linkage mistake (e.g. linked a subscriber to the WRONG dep
//! publisher, or falsely `resolved` an unpublished event), a pure equality diff would
//! still pass. These oracles assert the cross-app event-graph CONTRACT in ABSOLUTE
//! terms against the EXACT expected dep publisher EventId — derived BY NAME from the
//! resolved merged model's dep publisher routine, so the assertion is both SPECIFIC
//! and resilient to a signature-hash refresh.
//!
//! ## Covered (cross-app event-graph resolution — R2.5b-c's guard)
//!   - a workspace [EventSubscriber] links to the EXACT dep publisher EventId (Dep Mgt
//!     50100 OnBeforeCompute), NOT another dep event, resolution `resolved`;
//!   - a dep [EventSubscriber] links to a WORKSPACE event (Host Mgt 70000
//!     OnHostStarted) — the reverse cross-app direction (Rev 2 #7, via the
//!     parity-projected dep attributesParsed); proves the string-encoded subscriber
//!     attrs resolve cross-app;
//!   - the UNLINKED subscriber (a PRESENT dep object's UNPUBLISHED event) forms NO
//!     `resolved` edge — it stays `maybe`, the synthesized symbol carries NO
//!     publisherRoutineId (open-world soundness: target-runs binding, NOT a refutation
//!     — per [[event-crossed-refutation-unsound-openworld]]; we add no false link);
//!   - the cross-app `resolved` publisher EventId is the EXACT one the dep publisher
//!     routine produces (≥2 dep publisher candidates — OnBeforeCompute vs OnNever
//!     Published synthesized — so a wrong-but-same link is DETECTABLE).

use std::path::PathBuf;

use al_call_hierarchy::engine::deps::cross_app_l3::{
    build_cross_app_l3_from_workspace, CrossAppL3,
};
use al_call_hierarchy::engine::l3::event_graph::{L3EventGraphProjection, PEventEdge};

const MODEL_INSTANCE_ID: &str = "r2.5b";
const DEP_CORE: &str = "dddddddd-0000-0000-0000-000000000001";
const DEP_OTHER: &str = "eeeeeeee-0000-0000-0000-000000000002";
const WS_APP_GUID: &str = "11111111-0000-0000-0000-0000000000aa";

// SPECIFIC publisher object ids (Rev 2 #1 — not "is-a-dep-event").
const DEP_MGT: &str = "dddddddd-0000-0000-0000-000000000001:Codeunit:50100";
const HOST_MGT: &str = "11111111-0000-0000-0000-0000000000aa:Codeunit:70000";

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/r2-5b-fixtures/cross-app-resolution")
}

/// Build the cross-app L3 over the committed `.app`-bearing fixture.
fn build() -> CrossAppL3 {
    build_cross_app_l3_from_workspace(&fixture(), MODEL_INSTANCE_ID)
        .expect("cross-app L3 builds over the `.app`-bearing workspace")
}

/// The EXACT expected StableEventId for a publisher routine, derived BY NAME from the
/// resolved merged model's event publishers (Rev 2 #1: SPECIFIC; derived by name so it
/// stays correct across a signature-hash refresh). The projected EventSymbol's `id`
/// is `${stableObjectId}::${eventName}::${signatureHash}`.
fn publisher_event_id_by_name(
    eg: &L3EventGraphProjection,
    publisher_object_prefix: &str,
    event_name: &str,
) -> String {
    eg.events
        .iter()
        .find(|s| {
            s.publisher_object_id == publisher_object_prefix
                && s.event_name == event_name
                && s.publisher_routine_id.is_some()
        })
        .unwrap_or_else(|| {
            panic!("a REAL publisher EventSymbol for {publisher_object_prefix}::{event_name}")
        })
        .id
        .clone()
}

// ============================================================================
// 1. A workspace subscriber links to the EXACT dep publisher EventId (Dep Mgt
//    50100 OnBeforeCompute), NOT another event (Rev 2 #1).
// ============================================================================

#[test]
fn workspace_subscriber_links_to_exact_dep_publisher_event_id() {
    let cross = build();
    let eg = cross.project_event_graph();
    let expected = publisher_event_id_by_name(&eg, DEP_MGT, "OnBeforeCompute");

    // The workspace subscriber (Host Sub 70001) → the EXACT dep publisher EventId,
    // resolution `resolved` (cross-app link: a present dep publisher produces the id).
    let ws_to_dep: Vec<&PEventEdge> = eg
        .edges
        .iter()
        .filter(|e| {
            e.resolution == "resolved"
                && e.subscriber_app_id == WS_APP_GUID
                && e.event_id == expected
        })
        .collect();
    assert_eq!(
        ws_to_dep.len(),
        1,
        "exactly one workspace→dep `resolved` edge to the EXACT Dep Mgt.OnBeforeCompute EventId ({expected})",
    );
    // The subscriber is the workspace Host Sub 70001.
    assert!(
        ws_to_dep[0]
            .subscriber_routine_id
            .starts_with(&format!("{WS_APP_GUID}:Codeunit:70001#")),
        "the subscriber is the workspace Host Sub 70001",
    );
    // SPECIFIC: it is NOT the OnNeverPublished (unlinked) event id — a wrong-but-same
    // link would fail here (≥2 dep events on the same object make it detectable).
    let unpublished = eg
        .events
        .iter()
        .find(|s| s.publisher_object_id == DEP_MGT && s.event_name == "OnNeverPublished")
        .expect("the synthesized OnNeverPublished symbol exists")
        .id
        .clone();
    assert_ne!(
        expected, unpublished,
        "OnBeforeCompute and OnNeverPublished are DISTINCT event ids",
    );
}

// ============================================================================
// 2. A dep subscriber links to a WORKSPACE event (Host Mgt 70000 OnHostStarted)
//    — the reverse cross-app direction (Rev 2 #7). Also proves the string-encoded
//    subscriber attrs resolve cross-app (else NO edge forms).
// ============================================================================

#[test]
fn dep_subscriber_links_to_exact_workspace_event_id() {
    let cross = build();
    let eg = cross.project_event_graph();
    let expected = publisher_event_id_by_name(&eg, HOST_MGT, "OnHostStarted");

    // The dep subscriber (Other Mgt 60100, app eeee) → the EXACT workspace event id.
    let dep_to_ws: Vec<&PEventEdge> = eg
        .edges
        .iter()
        .filter(|e| {
            e.resolution == "resolved" && e.subscriber_app_id == DEP_OTHER && e.event_id == expected
        })
        .collect();
    assert_eq!(
        dep_to_ws.len(),
        1,
        "exactly one dep→workspace `resolved` edge to the EXACT Host Mgt.OnHostStarted EventId ({expected})",
    );
    // The subscriber is the DEP routine Other Mgt 60100 — its string-encoded
    // [EventSubscriber] attrs ("ObjectType::Codeunit", Codeunit::"Host Mgt",
    // "'OnHostStarted'") MUST have parsed cross-app or NO edge would form.
    assert!(
        dep_to_ws[0]
            .subscriber_routine_id
            .starts_with(&format!("{DEP_OTHER}:Codeunit:60100#")),
        "the subscriber is the dep Other Mgt 60100 (string-encoded attrs resolved cross-app)",
    );
}

// ============================================================================
// 3. The UNLINKED subscriber forms NO `resolved` edge: a PRESENT dep object's
//    UNPUBLISHED event stays `maybe`; the synthesized symbol has NO publisherRoutineId
//    (open-world soundness — target-runs binding, NOT a refutation; NO false link).
// ============================================================================

#[test]
fn unlinked_subscriber_forms_no_false_resolved_edge() {
    let cross = build();
    let eg = cross.project_event_graph();

    // Host Unlinked Sub 70003 → Dep Mgt 50100 OnNeverPublished (PRESENT dep object,
    // UNPUBLISHED event). The synthesized `maybe` symbol's StableEventId.
    let unlinked_event_id = eg
        .events
        .iter()
        .find(|s| s.publisher_object_id == DEP_MGT && s.event_name == "OnNeverPublished")
        .expect("the synthesized OnNeverPublished symbol exists")
        .id
        .clone();

    let unlinked: Vec<&PEventEdge> = eg
        .edges
        .iter()
        .filter(|e| e.event_id == unlinked_event_id && e.subscriber_app_id == WS_APP_GUID)
        .collect();
    assert_eq!(unlinked.len(), 1, "exactly one unlinked-subscriber edge");
    // The KEY soundness assertion: it is `maybe`, NEVER a false `resolved` (no false
    // link to a present-but-unpublished dep event — the cross-app analog of the R2c
    // realPublisherEventIds fix).
    assert_eq!(
        unlinked[0].resolution, "maybe",
        "the unlinked subscriber stays `maybe` (a present dep object's UNPUBLISHED event is NOT a false `resolved`)",
    );
    assert_ne!(
        unlinked[0].resolution, "resolved",
        "the unlinked subscriber is NOT falsely resolved (open-world: target-runs binding, NOT a refutation)",
    );
    // The synthesized symbol carries NO publisherRoutineId (no real publisher) and is
    // `unknown` kind.
    let sym = eg
        .events
        .iter()
        .find(|s| s.id == unlinked_event_id)
        .expect("the unlinked event symbol exists");
    assert!(
        sym.publisher_routine_id.is_none(),
        "the synthesized `maybe` symbol has NO publisherRoutineId (no real publisher)",
    );
    assert_eq!(sym.event_kind, "unknown");
}

// ============================================================================
// 4. The cross-app matrix is non-degenerate AND open-world-sound: exactly two
//    `resolved` cross-app edges (one each direction) + exactly one `maybe`
//    (unlinked) + NO `unknown` (every target object is present).
// ============================================================================

#[test]
fn cross_app_event_matrix_is_non_degenerate_and_sound() {
    let cross = build();
    let eg = cross.project_event_graph();

    let resolved = eg
        .edges
        .iter()
        .filter(|e| e.resolution == "resolved")
        .count();
    let maybe = eg.edges.iter().filter(|e| e.resolution == "maybe").count();
    let unknown = eg
        .edges
        .iter()
        .filter(|e| e.resolution == "unknown")
        .count();

    assert_eq!(
        resolved, 2,
        "two cross-app `resolved` edges (ws→dep + dep→ws)"
    );
    assert_eq!(maybe, 1, "one `maybe` edge (the unlinked subscriber)");
    assert_eq!(
        unknown, 0,
        "no `unknown` edges — every subscriber's target object IS present (cross-app resolved or workspace)",
    );

    // ≥1 dep-publisher-linked edge (the eventId is dep-owned).
    let dep_pub = eg
        .edges
        .iter()
        .filter(|e| {
            e.resolution == "resolved"
                && (e.event_id.starts_with(&format!("{DEP_CORE}:"))
                    || e.event_id.starts_with(&format!("{DEP_OTHER}:")))
        })
        .count();
    assert!(
        dep_pub >= 1,
        "≥1 subscriber edge whose publisher is a DEP event"
    );

    // ≥1 dep-subscriber edge (the subscriberAppId is a dep app).
    let dep_sub = eg
        .edges
        .iter()
        .filter(|e| {
            e.resolution == "resolved"
                && (e.subscriber_app_id == DEP_CORE || e.subscriber_app_id == DEP_OTHER)
        })
        .count();
    assert!(
        dep_sub >= 1,
        "≥1 dep-subscriber→workspace-event edge (Rev 2 #7)"
    );
}
