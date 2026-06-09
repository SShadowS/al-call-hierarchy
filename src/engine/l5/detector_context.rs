//! `DetectorContext` — port of al-sem `src/detectors/detector-context.ts`.
//!
//! The shared, eager indexes + derived graphs detectors read from, built once at
//! the top of `run_detectors`. This R4-A wave builds the EAGER indexes the task
//! enumerates: routine_by_id / objects_by_id / table_by_id / reverse_call_graph /
//! entry_points / transaction_spans / resolved_call_edge_by_callsite /
//! uncertainty_edges_by_from / call_site_by_id, plus the combined graph + the
//! per-routine `FullRoutineSummary` map (transaction_spans needs it).
//!
//! DEFERRED to later waves (TODO):
//!   - the lazy `get_event_flow_indexes()` hook (D43/D44/D45)
//!   - the lazy `get_ordering_facts()` hook (D47)
//!   - `reachable_roots` + `internal_reachable_externally` (D14)
//!
//! d4 reads none of these; later detector waves add them as they land.

use std::collections::{BTreeSet, HashMap};

use crate::engine::l2::features::PCallSite;
use crate::engine::l3::call_resolver::{resolve_calls, CallEdge, DeclaredDependency};
use crate::engine::l3::event_graph::build_event_graph;
use crate::engine::l3::event_graph::{EventGraph, EventSymbol};
use crate::engine::l3::l3_workspace::{L3Object, L3Resolved, L3Routine, L3Table};
use crate::engine::l3::symbol_table::SymbolTable;
use crate::engine::l4::capability_cone::{
    compose_cone_over_graph, direct_facts_for_routine, CapabilityFact,
};
use crate::engine::l4::combined_graph::{build_combined_graph, CombinedGraph};
use crate::engine::l5::full_summary::FullRoutineSummary;
use crate::engine::l5::reverse_call_graph::{build_reverse_call_graph, ReverseCallGraph};
use crate::engine::l5::transaction_spans::{compute_transaction_spans, TransactionSpan};

/// Shared context threaded into every detector.
pub struct DetectorContext<'a> {
    /// The combined graph (al-sem passes this as the detector's `graph` arg;
    /// detectors read it from the ctx here).
    pub graph: CombinedGraph,
    /// The raw L3 event graph (al-sem `model.eventGraph`). d12/d38 read its
    /// `events`/`edges`; the combined-graph build already constructs it, so it is
    /// captured here rather than recomputed.
    pub event_graph: EventGraph,
    pub routine_by_id: HashMap<&'a str, &'a L3Routine>,
    pub objects_by_id: HashMap<&'a str, &'a L3Object>,
    pub table_by_id: HashMap<&'a str, &'a L3Table>,
    pub reverse_call_graph: ReverseCallGraph,
    /// Trigger + event-subscriber roots — transaction-span boundaries.
    pub entry_points: BTreeSet<String>,
    pub transaction_spans: Vec<TransactionSpan>,
    /// Resolved CallEdges keyed by callsiteId (first edge per callsite wins).
    pub resolved_call_edge_by_callsite: HashMap<String, CallEdge>,
    /// Uncertainty edges grouped by source routine.
    pub uncertainty_edges_by_from:
        HashMap<String, Vec<crate::engine::l4::combined_graph::Uncertainty>>,
    /// Every call site indexed by id.
    pub call_site_by_id: HashMap<&'a str, &'a PCallSite>,
    /// Per-routine `FullRoutineSummary` (direct + inherited facts + coverage).
    pub summaries: HashMap<String, FullRoutineSummary>,
    // TODO(R4-C/D/G): reachable_roots + internal_reachable_externally (D14),
    // get_event_flow_indexes() (D43/D44/D45), get_ordering_facts() (D47).
}

/// Build the shared context. Runs the SOURCE-ONLY L3→L4 substrate (symbols →
/// resolve_calls → event_graph → combined_graph → cone) to assemble the combined
/// graph + per-routine `FullRoutineSummary`, then the eager indexes + transaction
/// spans (which consume the reverse graph + summaries).
pub fn build_detector_context(resolved: &L3Resolved) -> DetectorContext<'_> {
    let ws = &resolved.workspace;

    // --- L3→L4 substrate (source-only: no deps) ----------------------------
    let symbols = SymbolTable::build(&ws.objects, &ws.tables, &ws.routines);
    let no_deps: Vec<DeclaredDependency> = Vec::new();
    let no_fetched: Vec<String> = Vec::new();
    let calls = resolve_calls(ws, &symbols, &no_deps, &no_fetched);
    let event_graph = build_event_graph(&ws.routines, &symbols);
    let graph = build_combined_graph(ws, &calls, &event_graph);

    // Per-routine direct facts + direct coverage, then the inherited cone over
    // the combined graph — the same assembly project_r3a3 does inline, here via
    // the reusable `compose_cone_over_graph` seam.
    let mut publisher_events_by_routine: HashMap<String, Vec<&EventSymbol>> = HashMap::new();
    for evt in &event_graph.events {
        if let Some(pr) = &evt.publisher_routine_id {
            publisher_events_by_routine
                .entry(pr.clone())
                .or_default()
                .push(evt);
        }
    }
    let empty_pub: Vec<&EventSymbol> = Vec::new();
    let mut direct_full: HashMap<String, Vec<CapabilityFact>> = HashMap::new();
    let mut direct_in: HashMap<String, Vec<CapabilityFact>> = HashMap::new();
    let mut coverage_in: HashMap<String, (String, Vec<String>)> = HashMap::new();
    let nodes: Vec<String> = ws.routines.iter().map(|r| r.id.clone()).collect();
    for r in &ws.routines {
        let pubs = publisher_events_by_routine.get(&r.id).unwrap_or(&empty_pub);
        let (facts, status, reasons) = direct_facts_for_routine(r, pubs);
        direct_in.insert(r.id.clone(), facts.clone());
        coverage_in.insert(r.id.clone(), (status, reasons));
        direct_full.insert(r.id.clone(), facts);
    }
    let cones = compose_cone_over_graph(&graph, &nodes, &direct_in, &coverage_in);

    let empty_facts: Vec<CapabilityFact> = Vec::new();
    let mut summaries: HashMap<String, FullRoutineSummary> = HashMap::new();
    for r in &ws.routines {
        let cone = cones.get(&r.id);
        let inherited = cone.map(|c| c.inherited.clone()).unwrap_or_default();
        let coverage = cone.map(|c| c.coverage.clone());
        summaries.insert(
            r.id.clone(),
            FullRoutineSummary {
                routine_id: r.id.clone(),
                capability_facts_direct: direct_full.get(&r.id).unwrap_or(&empty_facts).clone(),
                capability_facts_inherited: inherited,
                coverage,
            },
        );
    }

    // --- Eager indexes -----------------------------------------------------
    let routine_by_id: HashMap<&str, &L3Routine> =
        ws.routines.iter().map(|r| (r.id.as_str(), r)).collect();
    let objects_by_id: HashMap<&str, &L3Object> =
        ws.objects.iter().map(|o| (o.id.as_str(), o)).collect();
    let table_by_id: HashMap<&str, &L3Table> =
        ws.tables.iter().map(|t| (t.id.as_str(), t)).collect();

    let reverse_call_graph = build_reverse_call_graph(&graph);

    // Source-only: no dep routines.
    let dep_routine_ids: BTreeSet<String> = BTreeSet::new();
    let entry_points: BTreeSet<String> =
        crate::engine::l5::entry_points::find_entry_points(&ws.routines, &dep_routine_ids)
            .into_iter()
            .collect();

    let transaction_spans = compute_transaction_spans(
        &ws.routines,
        &dep_routine_ids,
        &reverse_call_graph,
        &summaries,
    );

    let mut resolved_call_edge_by_callsite: HashMap<String, CallEdge> = HashMap::new();
    for ce in &calls.edges {
        if ce.to.is_none() {
            continue;
        }
        resolved_call_edge_by_callsite
            .entry(ce.callsite_id.clone())
            .or_insert_with(|| ce.clone());
    }

    let mut uncertainty_edges_by_from: HashMap<
        String,
        Vec<crate::engine::l4::combined_graph::Uncertainty>,
    > = HashMap::new();
    for ue in &graph.uncertainty_edges {
        uncertainty_edges_by_from
            .entry(ue.from.clone())
            .or_default()
            .push(ue.uncertainty.clone());
    }

    let mut call_site_by_id: HashMap<&str, &PCallSite> = HashMap::new();
    for r in &ws.routines {
        for cs in &r.call_sites {
            call_site_by_id.insert(cs.id.as_str(), cs);
        }
    }

    DetectorContext {
        graph,
        event_graph,
        routine_by_id,
        objects_by_id,
        table_by_id,
        reverse_call_graph,
        entry_points,
        transaction_spans,
        resolved_call_edge_by_callsite,
        uncertainty_edges_by_from,
        call_site_by_id,
        summaries,
    }
}
