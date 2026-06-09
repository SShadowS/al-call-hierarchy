//! Native oracles for the event-flow substrate. Ground-truth-free: each builds a
//! synthetic `EventGraph` directly and asserts the index/query/fan-out/chain
//! output (correct + SORTED + unresolved-excluded), mirroring al-sem's probe-style
//! soundness oracles (not a byte-diff golden).

#![cfg(test)]

use std::collections::{BTreeSet, HashMap};

use super::*;
use crate::engine::l3::event_graph::{EventEdge, EventGraph, EventSymbol, Evidence};
use crate::engine::l5::test_support::{coverage, routine, summary};

fn ev(id: &str, publisher_routine: Option<&str>, name: &str, kind: &str) -> EventSymbol {
    EventSymbol {
        id: id.to_string(),
        publisher_object_id: "app/Codeunit/1".to_string(),
        publisher_routine_id: publisher_routine.map(|s| s.to_string()),
        publisher_stable_routine_id: publisher_routine.map(|s| format!("stable::{s}")),
        event_name: name.to_string(),
        event_kind: kind.to_string(),
        element_name: None,
        signature_hash: String::new(),
        parameters: Vec::new(),
        isolated: None,
        provenance: vec![Evidence {
            source: "tree-sitter".to_string(),
            note: None,
        }],
    }
}

fn edge(event_id: &str, subscriber: &str, resolution: &str) -> EventEdge {
    EventEdge {
        event_id: event_id.to_string(),
        subscriber_routine_id: subscriber.to_string(),
        subscriber_stable_routine_id: format!("stable::{subscriber}"),
        subscriber_app_id: "app".to_string(),
        resolution: resolution.to_string(),
        provenance: vec![Evidence {
            source: "tree-sitter".to_string(),
            note: None,
        }],
    }
}

fn no_deps() -> BTreeSet<String> {
    BTreeSet::new()
}

// ---------------------------------------------------------------------------
// event_kind_of
// ---------------------------------------------------------------------------

#[test]
fn event_kind_of_passes_business_and_internal_else_integration() {
    assert_eq!(event_kind_of("business"), "business");
    assert_eq!(event_kind_of("internal"), "internal");
    assert_eq!(event_kind_of("integration"), "integration");
    assert_eq!(event_kind_of("trigger"), "integration");
    assert_eq!(event_kind_of("unknown"), "integration");
    assert_eq!(event_kind_of(""), "integration");
    assert_eq!(event_kind_of("anything-else"), "integration");
}

// ---------------------------------------------------------------------------
// build_event_flow_indexes
// ---------------------------------------------------------------------------

/// 1 publisher `P`, 2 events `E1`/`E2`; resolved edges (s2→E1, s1→E1) + an
/// unresolved edge (s3→E2). The unresolved edge must be EXCLUDED from
/// subscribers/publishers, and every list must be sorted.
fn synthetic_graph() -> EventGraph {
    EventGraph {
        events: vec![
            ev("E1", Some("P"), "OnFoo", "integration"),
            ev("E2", Some("P"), "OnBar", "business"),
        ],
        edges: vec![
            edge("E1", "s2", "resolved"),
            edge("E1", "s1", "resolved"),
            edge("E2", "s3", "maybe"), // unresolved → excluded
        ],
    }
}

#[test]
fn build_indexes_publisher_and_events_by_publisher() {
    let g = synthetic_graph();
    let rs = vec![routine("P", "event-publisher")];
    let ix = build_event_flow_indexes(&g, &rs, &no_deps());

    assert_eq!(ix.publisher_by_event.get("E1"), Some(&"P".to_string()));
    assert_eq!(ix.publisher_by_event.get("E2"), Some(&"P".to_string()));
    // events_by_publisher sorted: E1, E2.
    assert_eq!(
        ix.events_by_publisher.get("P"),
        Some(&vec!["E1".to_string(), "E2".to_string()])
    );
    assert_eq!(ix.event_name_by_event.get("E1"), Some(&"OnFoo".to_string()));
    assert_eq!(ix.event_name_by_event.get("E2"), Some(&"OnBar".to_string()));
}

#[test]
fn build_indexes_subscribers_sorted_and_unresolved_excluded() {
    let g = synthetic_graph();
    let rs = vec![routine("P", "event-publisher")];
    let ix = build_event_flow_indexes(&g, &rs, &no_deps());

    // E1 has resolved subs s1, s2 — SORTED (input was s2, s1).
    assert_eq!(
        ix.subscribers_by_event.get("E1"),
        Some(&vec!["s1".to_string(), "s2".to_string()])
    );
    // E2's only edge was unresolved → NOT present.
    assert_eq!(ix.subscribers_by_event.get("E2"), None);
}

#[test]
fn build_indexes_publishers_by_subscriber_unresolved_excluded() {
    let g = synthetic_graph();
    let rs = vec![routine("P", "event-publisher")];
    let ix = build_event_flow_indexes(&g, &rs, &no_deps());

    assert_eq!(
        ix.publishers_by_subscriber.get("s1"),
        Some(&vec!["P".to_string()])
    );
    assert_eq!(
        ix.publishers_by_subscriber.get("s2"),
        Some(&vec!["P".to_string()])
    );
    // s3's edge was unresolved → no publisher mapping.
    assert_eq!(ix.publishers_by_subscriber.get("s3"), None);
}

#[test]
fn primary_routines_excludes_dep_set() {
    let g = synthetic_graph();
    let rs = vec![
        routine("P", "event-publisher"),
        routine("s1", "event-subscriber"),
        routine("dep1", "procedure"),
    ];
    let mut deps = BTreeSet::new();
    deps.insert("dep1".to_string());
    let ix = build_event_flow_indexes(&g, &rs, &deps);

    assert!(ix.primary_routines.contains("P"));
    assert!(ix.primary_routines.contains("s1"));
    assert!(!ix.primary_routines.contains("dep1"));
}

// ---------------------------------------------------------------------------
// query helpers
// ---------------------------------------------------------------------------

#[test]
fn get_subscribers_of_publisher_unions_sorted() {
    // P publishes E1 {s1,s2} and E2 {s2,s4}; union sorted dedup = s1,s2,s4.
    let g = EventGraph {
        events: vec![
            ev("E1", Some("P"), "OnFoo", "integration"),
            ev("E2", Some("P"), "OnBar", "integration"),
        ],
        edges: vec![
            edge("E1", "s2", "resolved"),
            edge("E1", "s1", "resolved"),
            edge("E2", "s4", "resolved"),
            edge("E2", "s2", "resolved"),
        ],
    };
    let rs = vec![routine("P", "event-publisher")];
    let ix = build_event_flow_indexes(&g, &rs, &no_deps());
    assert_eq!(
        get_subscribers_of_publisher("P", &ix),
        vec!["s1".to_string(), "s2".to_string(), "s4".to_string()]
    );
    // Unknown publisher → empty.
    assert!(get_subscribers_of_publisher("nope", &ix).is_empty());
}

#[test]
fn get_publishers_for_subscriber_and_of_event() {
    let g = synthetic_graph();
    let rs = vec![routine("P", "event-publisher")];
    let ix = build_event_flow_indexes(&g, &rs, &no_deps());

    assert_eq!(
        get_publishers_for_subscriber("s1", &ix),
        vec!["P".to_string()]
    );
    assert!(get_publishers_for_subscriber("s3", &ix).is_empty());

    assert_eq!(
        get_subscribers_of_event("E1", &ix),
        vec!["s1".to_string(), "s2".to_string()]
    );
    assert!(get_subscribers_of_event("E2", &ix).is_empty());

    assert_eq!(get_publisher_of_event("E1", &ix), Some("P".to_string()));
    assert_eq!(get_publisher_of_event("missing", &ix), None);
}

// ---------------------------------------------------------------------------
// compute_fanout
// ---------------------------------------------------------------------------

#[test]
fn compute_fanout_counts_and_coverage_states() {
    let g = synthetic_graph();
    let rs = vec![routine("P", "event-publisher")];
    let ix = build_event_flow_indexes(&g, &rs, &no_deps());

    // Summaries: s1 complete, s2 partial → E1 capabilityComposition = partial.
    let mut summaries: HashMap<String, FullRoutineSummary> = HashMap::new();
    summaries.insert(
        "s1".to_string(),
        summary("s1", vec![], vec![], Some(coverage("complete"))),
    );
    summaries.insert(
        "s2".to_string(),
        summary("s2", vec![], vec![], Some(coverage("partial"))),
    );

    let fan = compute_fanout(&g, &ix, &summaries);
    assert_eq!(fan.len(), 2);

    // Sorted by (publisher, eventName, eventId): OnBar(E2) before OnFoo(E1).
    let e2 = &fan[0];
    assert_eq!(e2.event_id, "E2");
    assert_eq!(e2.event_name, "OnBar");
    assert_eq!(e2.event_kind, "business");
    // E2: only an unresolved edge → 0 resolved subscribers.
    assert_eq!(e2.direct_subscriber_count, 0);
    // 1 total edge, 1 unresolved → dispatch partial; 0 subs → capability unknown.
    assert_eq!(e2.coverage.dispatch_edges, "partial");
    assert_eq!(e2.coverage.subscriber_discovery, "partial");
    assert_eq!(e2.coverage.capability_composition, "unknown");

    let e1 = &fan[1];
    assert_eq!(e1.event_id, "E1");
    assert_eq!(e1.event_name, "OnFoo");
    assert_eq!(e1.event_kind, "integration");
    assert_eq!(e1.direct_subscriber_count, 2);
    // 2 edges all resolved → dispatch complete; one subscriber partial → partial.
    assert_eq!(e1.coverage.dispatch_edges, "complete");
    assert_eq!(e1.coverage.capability_composition, "partial");
}

#[test]
fn compute_fanout_missing_summary_is_unknown() {
    // E1 has resolved subs but NO summary for one → capabilityComposition unknown.
    let g = EventGraph {
        events: vec![ev("E1", Some("P"), "OnFoo", "integration")],
        edges: vec![edge("E1", "s1", "resolved")],
    };
    let rs = vec![routine("P", "event-publisher")];
    let ix = build_event_flow_indexes(&g, &rs, &no_deps());
    let summaries: HashMap<String, FullRoutineSummary> = HashMap::new(); // none
    let fan = compute_fanout(&g, &ix, &summaries);
    assert_eq!(fan[0].coverage.capability_composition, "unknown");

    // With a summary that HAS coverage=complete → complete.
    let mut summaries2: HashMap<String, FullRoutineSummary> = HashMap::new();
    summaries2.insert(
        "s1".to_string(),
        summary("s1", vec![], vec![], Some(coverage("complete"))),
    );
    let fan2 = compute_fanout(&g, &ix, &summaries2);
    assert_eq!(fan2[0].coverage.capability_composition, "complete");

    // Summary present but NO coverage record → undefined status → unknown.
    let mut summaries3: HashMap<String, FullRoutineSummary> = HashMap::new();
    summaries3.insert("s1".to_string(), summary("s1", vec![], vec![], None));
    let fan3 = compute_fanout(&g, &ix, &summaries3);
    assert_eq!(fan3[0].coverage.capability_composition, "unknown");
}

#[test]
fn compute_fanout_report_scope_and_summary() {
    let g = synthetic_graph();
    let rs = vec![routine("P", "event-publisher")];
    let ix = build_event_flow_indexes(&g, &rs, &no_deps());
    let summaries: HashMap<String, FullRoutineSummary> = HashMap::new();

    let rep = compute_fanout_report(&g, &ix, &summaries, Scope::All);
    assert_eq!(rep.total_events, 2);
    assert_eq!(rep.total_publishers, 1);
    // E2 has 0 resolved subs → zeroSubscriberEvents counts it.
    assert_eq!(rep.zero_subscriber_events, 1);
    assert_eq!(rep.hot_events, 0);
    // E2 dispatch partial → coverage_partial_events counts it.
    assert_eq!(rep.coverage_partial_events, 1);
}

// ---------------------------------------------------------------------------
// walk_event_chain
// ---------------------------------------------------------------------------

#[test]
fn walk_event_chain_two_hop() {
    // P --E1--> s1 ; s1 --E2--> s2. Chain: root(P) -> dispatch(E1) -> sub(s1)
    //                                      -> dispatch(E2) -> sub(s2).
    let g = EventGraph {
        events: vec![
            ev("E1", Some("P"), "OnFoo", "integration"),
            ev("E2", Some("s1"), "OnBar", "integration"),
        ],
        edges: vec![edge("E1", "s1", "resolved"), edge("E2", "s2", "resolved")],
    };
    let rs = vec![
        routine("P", "event-publisher"),
        routine("s1", "event-publisher"),
    ];
    let ix = build_event_flow_indexes(&g, &rs, &no_deps());
    let tree = walk_event_chain("P", &ix, &ChainWalkOptions::default());

    assert_eq!(tree.kind, "root");
    assert_eq!(tree.routine_id, Some("P".to_string()));
    assert_eq!(tree.children.len(), 1);

    let disp_e1 = &tree.children[0];
    assert_eq!(disp_e1.kind, "event-dispatch");
    assert_eq!(disp_e1.event_id, Some("E1".to_string()));
    assert_eq!(disp_e1.event_name, Some("OnFoo".to_string()));
    assert_eq!(disp_e1.children.len(), 1);

    let sub_s1 = &disp_e1.children[0];
    assert_eq!(sub_s1.kind, "subscriber");
    assert_eq!(sub_s1.routine_id, Some("s1".to_string()));
    assert_eq!(sub_s1.children.len(), 1);

    let disp_e2 = &sub_s1.children[0];
    assert_eq!(disp_e2.kind, "event-dispatch");
    assert_eq!(disp_e2.event_id, Some("E2".to_string()));
    assert_eq!(disp_e2.children.len(), 1);

    let sub_s2 = &disp_e2.children[0];
    assert_eq!(sub_s2.kind, "subscriber");
    assert_eq!(sub_s2.routine_id, Some("s2".to_string()));
    assert!(sub_s2.children.is_empty());
}

#[test]
fn walk_event_chain_cycle_terminates() {
    // P --E1--> Q ; Q --E2--> P. Walking from P must detect the cycle when E2's
    // subscriber P is already on the active path — no infinite loop.
    let g = EventGraph {
        events: vec![
            ev("E1", Some("P"), "OnFoo", "integration"),
            ev("E2", Some("Q"), "OnBar", "integration"),
        ],
        edges: vec![edge("E1", "Q", "resolved"), edge("E2", "P", "resolved")],
    };
    let rs = vec![
        routine("P", "event-publisher"),
        routine("Q", "event-publisher"),
    ];
    let ix = build_event_flow_indexes(&g, &rs, &no_deps());
    let tree = walk_event_chain("P", &ix, &ChainWalkOptions::default());

    // root(P) -> dispatch(E1) -> sub(Q) -> dispatch(E2) -> sub(P){cycleDetected}
    let q = &tree.children[0].children[0];
    assert_eq!(q.routine_id, Some("Q".to_string()));
    let p_again = &q.children[0].children[0];
    assert_eq!(p_again.routine_id, Some("P".to_string()));
    assert!(p_again.cycle_detected);
    assert!(p_again.children.is_empty());
}

#[test]
fn walk_event_chain_depth_bound_truncates() {
    // P --E1--> s1 ; s1 --E2--> s2. With max_depth = 1, the subscribers of E1 would
    // land at depth 1 (>= max_depth) → the dispatch node is depth_truncated and has
    // NO subscriber children.
    let g = EventGraph {
        events: vec![
            ev("E1", Some("P"), "OnFoo", "integration"),
            ev("E2", Some("s1"), "OnBar", "integration"),
        ],
        edges: vec![edge("E1", "s1", "resolved"), edge("E2", "s2", "resolved")],
    };
    let rs = vec![
        routine("P", "event-publisher"),
        routine("s1", "event-publisher"),
    ];
    let ix = build_event_flow_indexes(&g, &rs, &no_deps());
    let tree = walk_event_chain(
        "P",
        &ix,
        &ChainWalkOptions {
            max_depth: Some(1),
            max_nodes: None,
        },
    );
    let disp = &tree.children[0];
    assert_eq!(disp.event_id, Some("E1".to_string()));
    assert!(disp.depth_truncated);
    assert!(disp.children.is_empty());
}

#[test]
fn walk_event_chain_node_budget_bounds() {
    // max_nodes = 1: root consumes the only node; expand sees budget <= 0 and emits
    // nothing.
    let g = EventGraph {
        events: vec![ev("E1", Some("P"), "OnFoo", "integration")],
        edges: vec![edge("E1", "s1", "resolved")],
    };
    let rs = vec![routine("P", "event-publisher")];
    let ix = build_event_flow_indexes(&g, &rs, &no_deps());
    let tree = walk_event_chain(
        "P",
        &ix,
        &ChainWalkOptions {
            max_depth: None,
            max_nodes: Some(1),
        },
    );
    assert!(tree.children.is_empty());
}

#[test]
fn compute_chain_report_stats() {
    // P --E1--> Q ; Q --E2--> P (cycle). Stats: 1 root, max depth, 1 cycle.
    let g = EventGraph {
        events: vec![
            ev("E1", Some("P"), "OnFoo", "integration"),
            ev("E2", Some("Q"), "OnBar", "integration"),
        ],
        edges: vec![edge("E1", "Q", "resolved"), edge("E2", "P", "resolved")],
    };
    let rs = vec![
        routine("P", "event-publisher"),
        routine("Q", "event-publisher"),
    ];
    let ix = build_event_flow_indexes(&g, &rs, &no_deps());
    let rep = compute_chain_report(&ix, &ChainWalkOptions::default(), Scope::All);

    // Two publisher roots (P, Q) each produce a chain.
    assert_eq!(rep.total_roots, 2);
    assert_eq!(rep.roots_with_events, 2);
    // At least one cycle detected across the two roots.
    assert!(rep.cycles_detected >= 1);
}

#[test]
fn compute_chain_report_primary_scope_filters() {
    // P primary, dep1 a dependency publisher with a dep-only subtree. scope=Primary
    // keeps only chains touching a primary routine.
    let g = EventGraph {
        events: vec![
            ev("E1", Some("P"), "OnFoo", "integration"),
            ev("E2", Some("dep1"), "OnBar", "integration"),
        ],
        edges: vec![
            edge("E1", "P_sub", "resolved"),
            edge("E2", "dep2", "resolved"),
        ],
    };
    let rs = vec![
        routine("P", "event-publisher"),
        routine("P_sub", "event-subscriber"),
        routine("dep1", "event-publisher"),
        routine("dep2", "event-subscriber"),
    ];
    let mut deps = BTreeSet::new();
    deps.insert("dep1".to_string());
    deps.insert("dep2".to_string());
    let ix = build_event_flow_indexes(&g, &rs, &deps);

    let all = compute_chain_report(&ix, &ChainWalkOptions::default(), Scope::All);
    assert_eq!(all.total_roots, 2);

    let primary = compute_chain_report(&ix, &ChainWalkOptions::default(), Scope::Primary);
    // dep1's chain touches no primary routine → dropped; P's chain kept.
    assert_eq!(primary.total_roots, 1);
    assert_eq!(primary.chains[0].routine_id, Some("P".to_string()));
}
