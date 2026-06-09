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
//!
//! d4 reads none of these; later detector waves add them as they land.
//!
//! The R4-G wave wired `reachable_roots` + `internal_reachable_externally` (D14):
//! `reachable_roots` is built via `entry_points::find_reachable_roots` over the
//! `access_modifiers` map harvested from `L3Routine.access_modifier`;
//! `internal_reachable_externally` DEFAULTS to `false` (see field doc).

use std::collections::{BTreeSet, HashMap};

use crate::engine::l2::features::PCallSite;
use crate::engine::l3::call_resolver::{
    resolve_calls, CallEdge, DeclaredDependency, UpgradedBinding,
};
use crate::engine::l3::event_graph::build_event_graph;
use crate::engine::l3::event_graph::{EventGraph, EventSymbol};
use crate::engine::l3::l3_workspace::{L3Object, L3Resolved, L3Routine, L3Table};
use crate::engine::l3::symbol_table::SymbolTable;
use crate::engine::l4::capability_cone::{
    compose_cone_over_graph, direct_facts_for_routine, CapabilityFact,
};
use crate::engine::l4::combined_graph::{build_combined_graph, CombinedGraph};
use crate::engine::l4::scc::{tarjan_scc, SccInputGraph};
use crate::engine::l4::summary::{dedupe_uncertainties, RecordRoleSummary, Uncertainty};
use crate::engine::l4::summary_runner::{compute_summaries, FieldIndex};
use crate::engine::l5::entry_points::AccessModifier;
use crate::engine::l5::event_flow::{build_event_flow_indexes, EventFlowIndexes};
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
    /// Per-node merged uncertainty set the path-walker accumulates per branch.
    /// `uncertaintiesAt(node) = core_summary.uncertainties ∪
    /// uncertainty_edges_by_from.get(node)`, deduped+sorted by `uncertainty_key`.
    /// Mirrors al-sem `walkEvidence`'s `uncertaintiesAt` (path-walker.ts:103-106).
    /// The UNION ORDER is `[...fromSummary, ...fromEdges]` — the CORE
    /// `RoutineSummary.uncertainties` first, then the combined-graph edge
    /// uncertainties — matching al-sem exactly before the dedupe. Keyed by
    /// internal routine id; `walk_evidence` reads it via this exact field.
    pub uncertainties_by_node: HashMap<String, Vec<Uncertainty>>,
    /// Every call site indexed by id.
    pub call_site_by_id: HashMap<&'a str, &'a PCallSite>,
    /// Per-routine `FullRoutineSummary` (direct + inherited facts + coverage).
    pub summaries: HashMap<String, FullRoutineSummary>,
    /// The shared event-flow indexes (publisher/subscriber lookup tables) the
    /// d43/d44/d45 event-flow detectors consume. al-sem builds this LAZILY
    /// (`ctx.getEventFlowIndexes()`, memoized); the Rust port builds it EAGERLY
    /// here — deterministic, one pass over `event_graph.events`/`.edges`, matching
    /// how `event_graph`/`transaction_spans` are already eager.
    pub event_flow_indexes: EventFlowIndexes,
    /// The CORE `RoutineSummary.parameter_roles` (`RecordRoleSummary[]`) per
    /// routine, keyed by internal RoutineId. al-sem detectors read this as
    /// `routine.summary.parameterRoles`; the Rust `FullRoutineSummary`
    /// (`ctx.summaries`) DROPPED parameter_roles, so d37/d39 read them here.
    /// Harvested from the SAME recomputed core summaries the `uncertainties_by_node`
    /// harvest uses — NOT recomputed. Absent ⇒ no record-parameter roles.
    pub parameter_roles_by_routine: HashMap<String, Vec<RecordRoleSummary>>,
    /// The post-upgrade argument bindings per callsite (the resolver's
    /// `upgradeBindings` side table). The L3 `PCallArgumentBinding` carries the
    /// SOURCE-side fields (sourceKind / sourceVariableName / sourceRecordVariableId
    /// / callerSourceParameterIsVar / argumentAnchor / parameterIndex), but NOT the
    /// upgraded `bindingResolution` / `calleeParameterIsVar` — those live here,
    /// index-aligned with `call_site.argument_bindings`. d37/d39 join the two by
    /// position to read `binding.bindingResolution` / `binding.calleeParameterIsVar`.
    pub upgraded_bindings_by_callsite: HashMap<String, Vec<UpgradedBinding>>,
    /// The D14 forward-reachability root set — entry points (trigger /
    /// event-subscriber) PLUS the procedures al-sem cannot prove app-scoped
    /// (non-`local`; `internal` only when `internal_reachable_externally`). Built
    /// by `entry_points::find_reachable_roots` over the `access_modifiers` map
    /// harvested from `L3Routine.access_modifier`. Sorted; d14 BFS-seeds from it.
    pub reachable_roots: BTreeSet<String>,
    /// al-sem `(model.identity.primaryInternalsVisibleTo?.length ?? 0) > 0` — true
    /// when some other app is granted `internal` access (so `internal` procedures
    /// stay external API surface and are NOT flaggable as dead).
    ///
    /// DEFAULTS to `false`: the Rust model does NOT carry `primaryInternalsVisibleTo`
    /// and the source-only fixtures never set `internalsVisibleTo`. This is the
    /// source-only common case (no granted consumer ⇒ `internal` is app-scoped ⇒
    /// flaggable).
    /// TODO(R4-G+): if any fixture ever sets `internalsVisibleTo`, forward
    /// `primaryInternalsVisibleTo` from the L3 identity and replace this default.
    pub internal_reachable_externally: bool,
    /// R4-F root classifications (`model.rootClassifications`), keyed by INTERNAL
    /// RoutineId — d50/d51 look these up exactly like al-sem's
    /// `model.rootClassifications.find(r => r.routineId === routine.id)`. Carried
    /// verbatim from the resolved workspace (AST classifier + roots.config
    /// overlay). Empty when the resolve path produced no classifications.
    pub root_classifications_by_routine:
        HashMap<String, crate::engine::root_classification::RootClassification>,
    // TODO(R4-F): get_ordering_facts() (D47).
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

    // D14 reachable-roots wiring. Build the RoutineId → AccessModifier map from
    // `L3Routine.access_modifier` ("local"/"internal"/"protected"/None). al-sem maps
    // "local" → Local, "internal" → Internal, "protected"/None/anything-else →
    // Public (default-access). A routine with NO entry is treated as Public by
    // `find_reachable_roots`, so we only need to insert the non-Public cases — but we
    // insert all parsed modifiers explicitly for clarity.
    let mut access_modifiers: HashMap<String, AccessModifier> = HashMap::new();
    for r in &ws.routines {
        let access = match r.access_modifier.as_deref() {
            Some("local") => AccessModifier::Local,
            Some("internal") => AccessModifier::Internal,
            // "protected" / None / any other value → public (al-sem default-access).
            _ => AccessModifier::Public,
        };
        access_modifiers.insert(r.id.clone(), access);
    }
    // See `DetectorContext::internal_reachable_externally` doc: defaults to false
    // (the Rust model carries no `primaryInternalsVisibleTo`; source-only fixtures
    // never set `internalsVisibleTo`).
    let internal_reachable_externally = false;
    let reachable_roots: BTreeSet<String> = crate::engine::l5::entry_points::find_reachable_roots(
        &ws.routines,
        &dep_routine_ids,
        &access_modifiers,
        internal_reachable_externally,
    )
    .into_iter()
    .collect();

    let transaction_spans = compute_transaction_spans(
        &ws.routines,
        &dep_routine_ids,
        &reverse_call_graph,
        &summaries,
    );

    // Event-flow indexes — built eagerly from the L3 event graph + routine set +
    // dep set (source-only ⇒ empty dep set ⇒ every routine primary). Consumes
    // `event_graph` by reference before it is moved into the struct.
    let event_flow_indexes = build_event_flow_indexes(&event_graph, &ws.routines, &dep_routine_ids);

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

    // --- Per-node uncertainty wiring (the path-walker source) --------------
    // al-sem `walkEvidence` computes
    //   uncertaintiesAt(node) = routine.summary.uncertainties ∪ uncertaintyEdgesByFrom.get(node)
    // The CORE `RoutineSummary.uncertainties` is dropped by `FullRoutineSummary`
    // (the cone path keeps only facts + coverage), so we recompute the core
    // summaries here from the SAME combined graph the cone used: Tarjan SCC over
    // `graph.edges_by_from`, then the Jacobi fixed point (`compute_summaries`).
    // This is the only place that needs the core uncertainties; the union is
    // assembled once and exposed on `uncertainties_by_node`.
    let mut scc_adjacency: HashMap<String, Vec<String>> = HashMap::new();
    for (from, list) in &graph.edges_by_from {
        scc_adjacency.insert(from.clone(), list.iter().map(|e| e.to.clone()).collect());
    }
    let scc = tarjan_scc(&SccInputGraph {
        nodes: &graph.nodes,
        edges_by_from: &scc_adjacency,
    });
    // Field-resolution index (keyed (tableId, lowercased field name)) — mirrors
    // summary.rs `run_and_project`; parameterRoles need it, uncertainties don't,
    // but `compute_summaries` takes it.
    let mut field_index: FieldIndex = HashMap::new();
    for table in &ws.tables {
        for field in &table.fields {
            field_index
                .entry((table.id.clone(), field.name.to_lowercase()))
                .or_insert_with(|| field.id.clone());
        }
    }
    let (core_summaries, _trace) = compute_summaries(
        &ws.routines,
        &graph,
        &scc,
        &calls.upgraded_bindings,
        &field_index,
        false,
    );

    // uncertaintiesAt(node) per routine: [...fromSummary, ...fromEdges], deduped.
    // Union ORDER mirrors al-sem `[...fromSummary, ...fromEdges]` — core summary
    // uncertainties FIRST, then the combined-graph edge uncertainties (converted
    // to the summary `Uncertainty` form). `dedupe_uncertainties` keeps first-seen
    // then sorts by key, matching al-sem's `dedupeUncertainties`.
    // Harvest the CORE parameter_roles per routine from the SAME recomputed core
    // summaries (d37/d39 read these as `routine.summary.parameterRoles`). Done in
    // the same pass so we never recompute the core summaries.
    let mut parameter_roles_by_routine: HashMap<String, Vec<RecordRoleSummary>> = HashMap::new();
    for r in &ws.routines {
        if let Some(s) = core_summaries.get(&r.id) {
            if !s.parameter_roles.is_empty() {
                parameter_roles_by_routine.insert(r.id.clone(), s.parameter_roles.clone());
            }
        }
    }

    let mut uncertainties_by_node: HashMap<String, Vec<Uncertainty>> = HashMap::new();
    for r in &ws.routines {
        let from_summary: &[Uncertainty] = core_summaries
            .get(&r.id)
            .map(|s| s.uncertainties.as_slice())
            .unwrap_or(&[]);
        let from_edges: Vec<Uncertainty> = uncertainty_edges_by_from
            .get(&r.id)
            .map(|edges| edges.iter().map(Uncertainty::from).collect())
            .unwrap_or_default();
        if from_summary.is_empty() && from_edges.is_empty() {
            continue;
        }
        let combined: Vec<Uncertainty> = from_summary
            .iter()
            .cloned()
            .chain(from_edges.into_iter())
            .collect();
        uncertainties_by_node.insert(r.id.clone(), dedupe_uncertainties(combined));
    }

    let mut call_site_by_id: HashMap<&str, &PCallSite> = HashMap::new();
    for r in &ws.routines {
        for cs in &r.call_sites {
            call_site_by_id.insert(cs.id.as_str(), cs);
        }
    }

    // Expose the resolver's post-upgrade bindings (the `upgradeBindings` side
    // table) keyed by callsite id — the join target for d37/d39 which read
    // `binding.bindingResolution` / `binding.calleeParameterIsVar`.
    let upgraded_bindings_by_callsite: HashMap<String, Vec<UpgradedBinding>> =
        calls.upgraded_bindings.clone();

    // R4-F root classifications — keyed by internal RoutineId for d50/d51 lookup.
    let root_classifications_by_routine: HashMap<
        String,
        crate::engine::root_classification::RootClassification,
    > = resolved
        .root_classifications
        .iter()
        .map(|rc| (rc.routine_id.clone(), rc.clone()))
        .collect();

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
        uncertainties_by_node,
        call_site_by_id,
        summaries,
        event_flow_indexes,
        parameter_roles_by_routine,
        upgraded_bindings_by_callsite,
        reachable_roots,
        internal_reachable_externally,
        root_classifications_by_routine,
    }
}
