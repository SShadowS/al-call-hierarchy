//! R2c EXIT GATE — native L3-DIRECT event-graph resolution oracle.
//!
//! Ground-truth-free, STRUCTURAL oracles run NATIVELY against the Rust L3 event
//! graph (`src/engine/l3/event_graph.rs::build_event_graph` — the FIXED open-world
//! semantics). Each invariant drives an inline single-app workspace through the real
//! `assemble_and_resolve_default → SymbolTable::build → build_event_graph` path and
//! asserts an event-resolution PROPERTY DIRECTLY on the internal `EventGraph`
//! (`EventSymbol[]` + `EventEdge[]`) — NOT a golden diff against al-sem expected
//! strings (that is `l3eg_vectors.rs` + the differential's `*.l3eg.golden.json`).
//!
//! ## Why an L3-DIRECT oracle (not just the byte-parity differential)
//!
//! The corpus differential (`tests/differential.rs`,
//! `differential_l3_event_graph_match_goldens`) is BYTE-PARITY with al-sem: if BOTH
//! engines made the same resolution mistake, a pure equality diff would still pass.
//! That is EXACTLY how the 6th al-sem oracle bug (false-`resolved`: the 2nd+
//! subscriber to an unindexed event upgraded to `resolved` by consulting the
//! synthesized `event_by_id` instead of the `real_publisher_event_ids` set) would
//! have survived a pure diff. These oracles assert the event-graph CONTRACT in
//! absolute terms — open-world means a parseable subscriber ALWAYS produces exactly
//! one edge (no silent gap) and a non-parseable one produces NONE; `resolved` IFF a
//! REAL `[IntegrationEvent]`/`[BusinessEvent]` publisher routine produced the
//! eventId; two subscribers to an unindexed event are BOTH `maybe` (never one
//! `resolved`); a missing target is `unknown` + a sentinel-id symbol; a synthesized
//! symbol's `signatureHash == sha256Hex(raw eventId)`; eventKind tracks the
//! publisher attribute; the eventName is lowercased in the raw id. A FAILURE here
//! that the differential misses means BOTH engines are wrong — flag it loudly (it is
//! NOT "fix the golden").
//!
//! ## Covered (source-only intra-workspace event resolution — R2c's guard)
//!   - OPEN-WORLD: every subscriber with a parseable `[EventSubscriber(...)]`
//!     produces EXACTLY ONE EventEdge; a NON-parseable `[EventSubscriber()]`
//!     produces NO edge (no silent gap, no spurious edge);
//!   - `resolved` IFF a REAL `[IntegrationEvent]`/`[BusinessEvent]` publisher routine
//!     produced the eventId (the FIXED `real_publisher_event_ids` semantics) — two
//!     subscribers to an unindexed event on an EXISTING object → BOTH `maybe` (the
//!     6th-oracle-bug regression guard), a subscriber to a real publisher →
//!     `resolved`;
//!   - target object NOT found → `unknown` + a synthesized sentinel-id symbol;
//!     target found, NO real publisher → `maybe` + a synthesized (conforming
//!     objectId) symbol; the synthesized `signatureHash == sha256Hex(raw eventId)`;
//!   - publisher eventKind: `[IntegrationEvent]` → integration, `[BusinessEvent]` →
//!     business; isolated parsing (true / explicit-false-omitted /
//!     present-unparseable-conservative-true);
//!   - eventName is LOWERCASED in the raw id (a mixed-case event resolves).
//!
//! ## Deferred (NOT source-only intra-workspace; later gates — where they land)
//!   - SOURCE-ONLY intra-workspace event resolution is all this gate covers; a
//!     publisher / subscriber whose target object lives in a `.app` symbol package
//!     (cross-app event resolution) is structurally unreachable here (the R2c corpus
//!     and this oracle are SOURCE-ONLY, empty `.app` ingestion) → R2.5 (`.app`
//!     projection) then cross-app.
//!   - PUBLISH-capability facts (does a publisher's transitive callee actually
//!     `Commit`/raise before the subscriber observes) are an effect-summary property
//!     the event graph does NOT compute → L4.

use al_call_hierarchy::engine::ids::{sha256_hex, to_stable_object_id};
use al_call_hierarchy::engine::l3::event_graph::{EventEdge, EventGraph, build_event_graph};
use al_call_hierarchy::engine::l3::l3_workspace::assemble_and_resolve_default;
use al_call_hierarchy::engine::l3::symbol_table::SymbolTable;

const APP_GUID: &str = "2c000000-0000-0000-0000-0000000002cc";

/// Assemble + resolve an inline workspace, then build the INTERNAL event graph over
/// it (symbol table built ONCE over the resolved workspace — the same path
/// `L3Resolved::project_event_graph` drives).
fn event_graph(files: &[(&str, &str)]) -> EventGraph {
    let owned: Vec<(String, String)> = files
        .iter()
        .map(|(n, s)| (n.to_string(), s.to_string()))
        .collect();
    let resolved = assemble_and_resolve_default(&owned, APP_GUID);
    let ws = &resolved.workspace;
    let symbols = SymbolTable::build(&ws.objects, &ws.tables, &ws.routines);
    build_event_graph(&ws.routines, &symbols)
}

/// All edges whose `event_id` resolves the named (raw, post-lowercase) event — used
/// to group the subscribers to a shared event.
fn edges_for_event_name<'a>(g: &'a EventGraph, event_name_lc: &str) -> Vec<&'a EventEdge> {
    g.edges
        .iter()
        .filter(|e| e.event_id.ends_with(&format!("/event/{event_name_lc}")))
        .collect()
}

// ---------------------------------------------------------------------------
// Invariant 1: OPEN-WORLD — a parseable subscriber produces EXACTLY ONE edge; a
// non-parseable `[EventSubscriber()]` produces NONE (no silent gap, no spurious).
// ---------------------------------------------------------------------------

#[test]
fn parseable_subscriber_makes_exactly_one_edge_unparseable_makes_none() {
    // (a) A parseable subscriber to a (missing) target → exactly one edge.
    let parseable = &[(
        "a.al",
        "codeunit 50101 Sub { [EventSubscriber(ObjectType::Codeunit, Codeunit::\"Out Of World\", 'OnGone', '', false, false)] local procedure Handle() begin end; }",
    )];
    let g = event_graph(parseable);
    assert_eq!(
        g.edges.len(),
        1,
        "a parseable [EventSubscriber(...)] produces EXACTLY ONE edge (open-world, no silent gap); got {:#?}",
        g.edges
    );

    // (b) A NON-parseable `[EventSubscriber()]` (no target args) → NO edge, and no
    //     synthesized symbol from it either.
    let unparseable = &[(
        "a.al",
        "codeunit 50101 Sub { [EventSubscriber()] local procedure Handle() begin end; }",
    )];
    let g = event_graph(unparseable);
    assert!(
        g.edges.is_empty(),
        "a non-parseable [EventSubscriber()] produces NO edge; got {:#?}",
        g.edges
    );
    assert!(
        g.events.is_empty(),
        "a non-parseable subscriber synthesizes NO event symbol; got {:#?}",
        g.events
    );
}

// ---------------------------------------------------------------------------
// Invariant 2: `resolved` IFF a REAL publisher routine produced the eventId. A
// subscriber to a real [IntegrationEvent] publisher → resolved (no synthesis).
// ---------------------------------------------------------------------------

#[test]
fn subscriber_to_real_publisher_is_resolved() {
    let files = &[(
        "a.al",
        "codeunit 50100 Engine { [IntegrationEvent(false, false)] procedure OnDone() begin end; } \
         codeunit 50101 Listener { [EventSubscriber(ObjectType::Codeunit, Codeunit::Engine, 'OnDone', '', false, false)] local procedure Handle() begin end; }",
    )];
    let g = event_graph(files);

    assert_eq!(g.edges.len(), 1, "one subscriber → one edge");
    assert_eq!(
        g.edges[0].resolution, "resolved",
        "a subscriber to a REAL [IntegrationEvent] publisher is resolved"
    );
    // Exactly one event symbol — the real publisher's. NO synthesized symbol was
    // emitted (resolved subscribers never synthesize).
    assert_eq!(
        g.events.len(),
        1,
        "a resolved subscriber adds NO synthesized symbol; got {:#?}",
        g.events
    );
    let pub_sym = &g.events[0];
    assert!(
        pub_sym.publisher_routine_id.is_some(),
        "the sole event symbol is the REAL publisher (has a publisherRoutineId)"
    );
    assert_eq!(pub_sym.event_kind, "integration");
    // The resolved edge's eventId IS the real publisher's symbol id.
    assert_eq!(
        g.edges[0].event_id, pub_sym.id,
        "the resolved edge points at the real publisher's event id"
    );
}

// ---------------------------------------------------------------------------
// Invariant 3 (THE 6th-oracle-bug GUARD): two subscribers to an UNINDEXED event on
// an EXISTING object → BOTH `maybe` (NEVER one `resolved`).
// ---------------------------------------------------------------------------

#[test]
fn two_subscribers_to_unindexed_event_are_both_maybe() {
    // `Hub.OnPing` is a plain procedure (NOT an [IntegrationEvent]/[BusinessEvent]),
    // so it is NOT a real publisher — no `real_publisher_event_ids` entry. The first
    // subscriber synthesizes a "maybe" symbol; the FIXED semantics must NOT let the
    // second subscriber consult that synthesized symbol and upgrade to "resolved".
    let files = &[(
        "a.al",
        "codeunit 50100 Hub { procedure OnPing() begin end; } \
         codeunit 50101 SubA { [EventSubscriber(ObjectType::Codeunit, Codeunit::Hub, 'OnPing', '', false, false)] local procedure HandleA() begin end; } \
         codeunit 50102 SubB { [EventSubscriber(ObjectType::Codeunit, Codeunit::Hub, 'OnPing', '', false, false)] local procedure HandleB() begin end; }",
    )];
    let g = event_graph(files);

    let edges = edges_for_event_name(&g, "onping");
    assert_eq!(
        edges.len(),
        2,
        "two subscribers → two edges; got {edges:#?}"
    );
    for e in &edges {
        assert_eq!(
            e.resolution, "maybe",
            "a subscriber to an UNINDEXED event (no real publisher) is 'maybe', NEVER 'resolved' \
             (the 6th al-sem oracle bug — false-resolved on the 2nd+ subscriber); got {e:#?}"
        );
    }
    assert!(
        g.edges.iter().all(|e| e.resolution != "resolved"),
        "NO edge may be 'resolved' when no real publisher exists"
    );

    // Exactly ONE synthesized "maybe" symbol — the second subscriber DEDUPED against
    // it (did not synthesize a second), but did NOT upgrade to resolved.
    let synthesized: Vec<_> = g
        .events
        .iter()
        .filter(|s| s.publisher_routine_id.is_none())
        .collect();
    assert_eq!(
        synthesized.len(),
        1,
        "the two subscribers share ONE synthesized maybe-symbol (dedup); got {synthesized:#?}"
    );
}

// ---------------------------------------------------------------------------
// Invariant 4: target found, NO real publisher → `maybe` + a synthesized symbol with
// a CONFORMING objectId and `signatureHash == sha256Hex(raw eventId)`.
// ---------------------------------------------------------------------------

#[test]
fn target_found_no_publisher_is_maybe_with_conforming_synthesized_symbol() {
    let files = &[(
        "a.al",
        "codeunit 50100 Hub { procedure OnPing() begin end; } \
         codeunit 50101 Sub { [EventSubscriber(ObjectType::Codeunit, Codeunit::Hub, 'OnPing', '', false, false)] local procedure Handle() begin end; }",
    )];
    let g = event_graph(files);

    assert_eq!(g.edges.len(), 1);
    let edge = &g.edges[0];
    assert_eq!(
        edge.resolution, "maybe",
        "existing target, no publisher → maybe"
    );

    // The synthesized symbol: conforming objectId (the real Hub object id), kind
    // unknown, and signatureHash == sha256(raw eventId).
    let sym = g
        .events
        .iter()
        .find(|s| s.id == edge.event_id)
        .expect("the maybe edge's event symbol was synthesized");
    assert!(
        sym.publisher_routine_id.is_none(),
        "a maybe symbol has no real publisher routine"
    );
    assert_eq!(
        sym.event_kind, "unknown",
        "a synthesized symbol's kind is unknown"
    );
    // Conforming objectId: `${appGuid}/Codeunit/50100` (resolves with `/`→`:`).
    assert_eq!(
        sym.publisher_object_id,
        format!("{APP_GUID}/Codeunit/50100"),
        "the maybe symbol carries the EXISTING target object's conforming id"
    );
    assert_eq!(
        sym.signature_hash,
        sha256_hex(&sym.id),
        "synthesized signatureHash == sha256Hex(raw eventId)"
    );
}

// ---------------------------------------------------------------------------
// Invariant 5: target NOT found → `unknown` + a synthesized SENTINEL-id symbol whose
// `signatureHash == sha256Hex(raw eventId)`.
// ---------------------------------------------------------------------------

#[test]
fn target_not_found_is_unknown_with_sentinel_synthesized_symbol() {
    let files = &[(
        "a.al",
        "codeunit 50101 Sub { [EventSubscriber(ObjectType::Codeunit, Codeunit::\"Out Of World\", 'OnGone', '', false, false)] local procedure Handle() begin end; }",
    )];
    let g = event_graph(files);

    assert_eq!(g.edges.len(), 1);
    let edge = &g.edges[0];
    assert_eq!(
        edge.resolution, "unknown",
        "a subscriber to a target NOT in indexed source is 'unknown'"
    );

    let sym = g
        .events
        .iter()
        .find(|s| s.id == edge.event_id)
        .expect("the unknown edge's event symbol was synthesized");
    assert!(sym.publisher_routine_id.is_none());
    assert_eq!(sym.event_kind, "unknown");
    // The sentinel objectId is the NON-conforming `unknown/Codeunit/0:Out Of World`.
    assert_eq!(
        sym.publisher_object_id, "unknown/Codeunit/0:Out Of World",
        "an unknown target synthesizes the sentinel pseudo-object id"
    );
    assert_eq!(
        sym.signature_hash,
        sha256_hex(&sym.id),
        "synthesized signatureHash == sha256Hex(raw eventId)"
    );
    // The raw id encodes the lowercased event name on the sentinel.
    assert_eq!(
        sym.id, "unknown/Codeunit/0:Out Of World/event/ongone",
        "the sentinel raw eventId lowercases the eventName"
    );
}

// ---------------------------------------------------------------------------
// Invariant 6: publisher eventKind — [IntegrationEvent] → integration,
// [BusinessEvent] → business.
// ---------------------------------------------------------------------------

#[test]
fn publisher_event_kind_tracks_attribute() {
    let integration = &[(
        "a.al",
        "codeunit 50100 Engine { [IntegrationEvent(false, false)] procedure OnDone() begin end; }",
    )];
    let g = event_graph(integration);
    assert_eq!(g.events.len(), 1);
    assert_eq!(
        g.events[0].event_kind, "integration",
        "[IntegrationEvent] → integration"
    );
    assert!(g.events[0].publisher_routine_id.is_some());

    let business = &[(
        "a.al",
        "codeunit 50100 Biz { [BusinessEvent(false)] procedure OnBizDone() begin end; }",
    )];
    let g = event_graph(business);
    assert_eq!(g.events.len(), 1);
    assert_eq!(
        g.events[0].event_kind, "business",
        "[BusinessEvent] → business"
    );
}

// ---------------------------------------------------------------------------
// Invariant 7: isolated parsing — true / explicit-false-omitted /
// present-unparseable-conservative-true.
// ---------------------------------------------------------------------------

#[test]
fn isolated_parsing_three_cases() {
    // (a) explicit Isolated=true (index 2 of IntegrationEvent) → Some(true).
    let isolated_true = &[(
        "a.al",
        "codeunit 50100 Engine { [IntegrationEvent(false, false, true)] procedure OnDone() begin end; }",
    )];
    let g = event_graph(isolated_true);
    assert_eq!(g.events.len(), 1);
    assert_eq!(
        g.events[0].isolated,
        Some(true),
        "explicit Isolated=true → Some(true)"
    );

    // (b) explicit Isolated=false → omitted (None).
    let isolated_false = &[(
        "a.al",
        "codeunit 50100 Engine { [IntegrationEvent(false, false, false)] procedure OnDone() begin end; }",
    )];
    let g = event_graph(isolated_false);
    assert_eq!(g.events.len(), 1);
    assert_eq!(
        g.events[0].isolated, None,
        "explicit Isolated=false → omitted (None)"
    );

    // (c) Isolated arg present but NOT a boolean literal → conservative Some(true)
    //     (Rule 5: prefer exclusion over a false weave). An identifier arg is the
    //     unparseable case.
    let isolated_unparseable = &[(
        "a.al",
        "codeunit 50100 Engine { [IntegrationEvent(false, false, SomeFlag)] procedure OnDone() begin end; }",
    )];
    let g = event_graph(isolated_unparseable);
    assert_eq!(g.events.len(), 1);
    assert_eq!(
        g.events[0].isolated,
        Some(true),
        "Isolated present-but-unparseable → conservative Some(true)"
    );
}

// ---------------------------------------------------------------------------
// Invariant 8: eventName is LOWERCASED in the raw id — a mixed-case subscriber event
// resolves against the publisher whose eventName lowercases to the same id.
// ---------------------------------------------------------------------------

#[test]
fn event_name_lowercased_in_raw_id_so_mixed_case_resolves() {
    // Publisher event `OnMixedCaseEvent`; subscriber spells it `onmixedcaseevent`.
    // Both lowercase to `.../event/onmixedcaseevent` → resolved.
    let files = &[(
        "a.al",
        "codeunit 50100 Engine { [IntegrationEvent(false, false)] procedure OnMixedCaseEvent() begin end; } \
         codeunit 50101 Listener { [EventSubscriber(ObjectType::Codeunit, Codeunit::Engine, 'onmixedcaseevent', '', false, false)] local procedure Handle() begin end; }",
    )];
    let g = event_graph(files);

    assert_eq!(g.edges.len(), 1);
    assert_eq!(
        g.edges[0].resolution, "resolved",
        "a mixed-case event resolves because the eventName is lowercased in the raw id"
    );
    // The real publisher symbol's id carries the lowercased event name.
    let pub_sym = g
        .events
        .iter()
        .find(|s| s.publisher_routine_id.is_some())
        .expect("a real publisher symbol exists");
    assert!(
        pub_sym.id.ends_with("/event/onmixedcaseevent"),
        "the raw eventId lowercases the eventName; got {}",
        pub_sym.id
    );
    // The stable projection's `/`→`:` does not affect this lowercase invariant.
    assert_eq!(
        to_stable_object_id(&pub_sym.publisher_object_id),
        format!("{APP_GUID}:Codeunit:50100"),
        "the publisher object id projects to its stable form"
    );
}
