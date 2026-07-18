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
    CallEdge, DeclaredDependency, UpgradedBinding, resolve_calls,
};
use crate::engine::l3::event_graph::build_event_graph;
use crate::engine::l3::event_graph::{EventGraph, EventSymbol};
use crate::engine::l3::l3_workspace::{L3Object, L3Resolved, L3Routine, L3Table};
use crate::engine::l3::symbol_table::SymbolTable;
use crate::engine::l4::capability_cone::{
    CapabilityFact, compose_cone_over_graph, direct_facts_for_routine,
};
use crate::engine::l4::combined_graph::{CombinedGraph, build_combined_graph};
use crate::engine::l4::scc::{SccInputGraph, tarjan_scc};
use crate::engine::l4::summary::{RecordRoleSummary, Uncertainty, dedupe_uncertainties};
use crate::engine::l4::summary_runner::{FieldIndex, compute_summaries};
use crate::engine::l5::entry_points::AccessModifier;
use crate::engine::l5::event_flow::{EventFlowIndexes, build_event_flow_indexes};
use crate::engine::l5::full_summary::FullRoutineSummary;
use crate::engine::l5::reverse_call_graph::{ReverseCallGraph, build_reverse_call_graph};
use crate::engine::l5::transaction_spans::{TransactionSpan, compute_transaction_spans};

/// A declared workspace dependency (`model.identity.primaryDependencies[]`): the
/// `appGuid` / `name` / `minVersion` triple d17 iterates. Mirrors al-sem's
/// `ManifestDependency` (the d17-relevant subset). Source-only runs leave this empty.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclaredDep {
    pub app_guid: String,
    pub name: String,
    pub min_version: String,
}

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
    /// The routines whose app_guid ∈ the fetched dependency set (cross-app runs).
    /// `roleOf(caller)` = `dep_routine_ids.contains(caller.id) ? "dependency" :
    /// "primary"` — the d13/d16/d17 cross-app gate. EMPTY for source-only runs (every
    /// routine primary), matching al-sem's source-only `analysisRole` default.
    pub dep_routine_ids: BTreeSet<String>,
    /// The DECLARED workspace dependencies (`model.identity.primaryDependencies`),
    /// `{appGuid, name, minVersion}` per the primary app.json `dependencies[]`. d17
    /// iterates these. EMPTY for source-only runs (no deps declared / read).
    pub declared_dependencies: Vec<DeclaredDep>,
    /// Resolved dependency `.app` versions keyed by appGuid (`model.apps[].version`).
    /// d17 looks up the resolved version to compare against the declared minVersion.
    /// EMPTY for source-only runs (no dep .app parsed).
    pub app_versions: HashMap<String, String>,
    /// R4-F Stage-5b — the L4.5 ordering facts the d47/d49/d51 detectors consume,
    /// keyed by `StableRoutineId`. Computed LAZILY on first `get_ordering_facts()`
    /// access and memoized — exactly al-sem's `ctx.getOrderingFacts()` semantics.
    /// Only d47/d49/d51 (opt-in detectors) read it, so a default `analyze` run
    /// never pays the snapshot→digest→ordering cost (measured 43.6 s+ on CDO —
    /// the "alsem never completes" hang; see
    /// `.superpowers/sdd/alsem-parallel/investigation.md`).
    pub ordering_facts:
        std::sync::OnceLock<HashMap<String, crate::engine::l5::ordering_facts::OrderingFacts>>,
    /// The resolved model `get_ordering_facts()` computes from. `None` for the
    /// cross-app context (whose ordering facts are ALWAYS empty — d13/d16/d17
    /// never read them; matches the previous eager `HashMap::new()`).
    pub ordering_source: Option<&'a L3Resolved>,
    /// G-19 — the closed-world proven-temp `(routineId, paramIndex)` set: a
    /// keyword-less by-var record param of a `local` procedure ALL of whose
    /// resolved callers (and the routine's complete, fully-resolved same-object
    /// call surface) prove a `Known(true)` temporary argument. The d1/d3/d10
    /// temp gates treat such a param exactly like a `Known(true)` temp record.
    /// Built by `closed_world_temp::prove_closed_world_temp_params`; EVERY
    /// uncertainty fails the proof (the firing direction) — see module docs.
    pub closed_world_temp_params: crate::engine::l5::closed_world_temp::ClosedWorldTempParams,
    /// L4 summarize-stage diagnostics — presently just the JACOBI fixed-point
    /// cap-hit (`summary_runner::run_one_scc`). Harvested from the SAME
    /// `compute_summaries*` call this module already makes for
    /// `uncertainties_by_node`/`parameter_roles_by_routine` — not recomputed.
    /// Empty for every workspace whose SCCs converge, which is the overwhelming
    /// common case (additive: `run_detectors` folds this into the "summarize"
    /// slot of the analyze/detect diagnostics envelope).
    pub summarize_diagnostics: Vec<crate::engine::l4::summary_runner::SummarizeDiagnostic>,
    /// The shared finding-fingerprint index (routine/object id maps + the
    /// internal→stable routine-id substitution map). Built ONCE per run —
    /// previously every detector rebuilt it (54 × ~2 String clones per routine).
    pub fingerprint_index: crate::engine::l5::fingerprint::FingerprintIndex<'a>,
    /// event id → cross-extension subscriber routine ids (subscribers living in a
    /// DIFFERENT app than the publisher object). Previously rebuilt identically by
    /// d43, d44 AND d45 each run; built once here (same sharing pattern as
    /// `event_flow_indexes`).
    pub cross_extension_subscribers: std::collections::BTreeMap<String, Vec<String>>,
}

impl DetectorContext<'_> {
    /// The L4.5 ordering facts, keyed by `StableRoutineId`. Lazily computed on
    /// first access (memoized via `OnceLock` — thread-safe for future parallel
    /// detector runs). d47/d49/d51 look up their reportable routine's facts here
    /// exactly as al-sem's `ctx.getOrderingFacts()`.
    pub fn get_ordering_facts(
        &self,
    ) -> &HashMap<String, crate::engine::l5::ordering_facts::OrderingFacts> {
        self.ordering_facts
            .get_or_init(|| match self.ordering_source {
                Some(resolved) => {
                    crate::engine::l5::ordering_facts::compute_ordering_facts(resolved)
                }
                None => HashMap::new(),
            })
    }
}

/// Build the shared context. Runs the SOURCE-ONLY L3→L4 substrate (symbols →
/// resolve_calls → event_graph → combined_graph → cone) to assemble the combined
/// graph + per-routine `FullRoutineSummary`, then the eager indexes + transaction
/// spans (which consume the reverse graph + summaries).
pub fn build_detector_context(resolved: &L3Resolved) -> DetectorContext<'_> {
    let ws = &resolved.workspace;

    // --- L3→L4 substrate (source-only: no deps) ----------------------------
    let symbols = SymbolTable::build(&ws.objects, &ws.tables, &ws.routines);
    crate::stage_probe::stage("l4:symbol_table:end");
    let no_deps: Vec<DeclaredDependency> = Vec::new();
    let no_fetched: Vec<String> = Vec::new();
    let calls = resolve_calls(ws, &symbols, &no_deps, &no_fetched);
    crate::stage_probe::stage("l4:resolve_calls:end");
    let event_graph = build_event_graph(&ws.routines, &symbols);
    crate::stage_probe::stage("l4:event_graph:end");
    let graph = build_combined_graph(ws, &calls, &event_graph);
    crate::stage_probe::stage("l4:combined_graph:end");

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
    crate::stage_probe::stage("l4:cones:end");

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
    // G-5: REAL table wins an id collision with a tableextension stub (the stub's
    // id reuses the extension's own object number) — otherwise rootCause text
    // renders the EXTENSION's name for ops on the real table.
    let table_by_id: HashMap<&str, &L3Table> =
        crate::engine::l3::l3_workspace::table_by_id_preferring_real(&ws.tables);

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

    // G-19 — closed-world proven-temp params for `local` procedures (consumed
    // by the d1/d3/d10 temp gates). Pure lookup-table build over the routines +
    // combined graph + reverse graph; entry points are proof-disqualifying.
    crate::stage_probe::stage("l4:summaries_indexes:end");
    let closed_world_temp_params =
        crate::engine::l5::closed_world_temp::prove_closed_world_temp_params(
            &ws.routines,
            &graph,
            &reverse_call_graph,
            &entry_points,
        );
    crate::stage_probe::stage("l4:closed_world_temp:end");

    let transaction_spans = compute_transaction_spans(
        &ws.routines,
        &dep_routine_ids,
        &reverse_call_graph,
        &summaries,
    );
    crate::stage_probe::stage("l4:transaction_spans:end");

    // Event-flow indexes — built eagerly from the L3 event graph + routine set +
    // dep set (source-only ⇒ empty dep set ⇒ every routine primary). Consumes
    // `event_graph` by reference before it is moved into the struct.
    let event_flow_indexes = build_event_flow_indexes(&event_graph, &ws.routines, &dep_routine_ids);

    // Cross-extension subscriber lookup, shared by d43/d44/d45 — previously each
    // rebuilt this identically from `ctx.event_graph` + `ws.objects` per run.
    let cross_extension_subscribers =
        crate::engine::l5::event_flow::build_cross_extension_subscribers(&event_graph, &ws.objects);

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
    if std::env::var("ALSEM_STAGE_TIMING").as_deref() == Ok("1") {
        let max_scc = scc.sccs.iter().map(|s| s.members.len()).max().unwrap_or(0);
        let rec = scc.sccs.iter().filter(|s| s.recursive).count();
        let rec_members: usize = scc
            .sccs
            .iter()
            .filter(|s| s.recursive)
            .map(|s| s.members.len())
            .sum();
        eprintln!(
            "SCCSTATS nodes={} sccs={} recursive_sccs={} recursive_members={} max_scc={}",
            graph.nodes.len(),
            scc.sccs.len(),
            rec,
            rec_members,
            max_scc
        );
        if std::env::var("ALSEM_EXIT_AFTER_SCCSTATS").as_deref() == Ok("1") {
            std::process::exit(0);
        }
    }
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
    let (core_summaries, _trace, summarize_diagnostics) = compute_summaries(
        &ws.routines,
        &graph,
        &scc,
        &calls.upgraded_bindings,
        &field_index,
        false,
    );
    crate::stage_probe::stage("l4:compute_summaries:end");

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
        if let Some(s) = core_summaries.get(&r.id)
            && !s.parameter_roles.is_empty()
        {
            parameter_roles_by_routine.insert(r.id.clone(), s.parameter_roles.clone());
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
        let combined: Vec<Uncertainty> = from_summary.iter().cloned().chain(from_edges).collect();
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

    // R4-F Stage-5b ordering facts — computed lazily on first
    // `get_ordering_facts()` access (see field doc). Keyed by StableRoutineId;
    // d47/d49/d51 read it via `get_ordering_facts()`.

    let fingerprint_index =
        crate::engine::l5::fingerprint::FingerprintIndex::build(&ws.routines, &ws.objects);

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
        // Source-only: no deps → every routine primary, no declared deps, no versions.
        dep_routine_ids: BTreeSet::new(),
        declared_dependencies: Vec::new(),
        app_versions: HashMap::new(),
        root_classifications_by_routine,
        ordering_facts: std::sync::OnceLock::new(),
        ordering_source: Some(resolved),
        closed_world_temp_params,
        summarize_diagnostics,
        fingerprint_index,
        cross_extension_subscribers,
    }
}

/// Build the shared context for a CROSS-APP run from a pre-assembled
/// `R3a5CrossAppBase` (the merged workspace+dep model + cross-app combined graph +
/// `dep_routine_ids`). Mirrors `build_detector_context` but reads every substrate
/// from `base` instead of recomputing source-only, and threads `dep_routine_ids`
/// into the entry-point / reachable-root / transaction-span / event-flow builders so
/// dep routines are NOT treated as primary roots. d13/d16/d17 read
/// `dep_routine_ids` (the roleOf gate), `declared_dependencies` + `app_versions`
/// (d17), and the eager indexes; the path-walker substrate (uncertainties /
/// summaries) is built identically for any future cross-app detector.
///
/// `root_classifications` are EMPTY here; `ordering_source` is `None` here (ordering
/// facts lazily resolve to EMPTY — d13/d16/d17 never read them; the base does not
/// carry the resolved-model classifier inputs). A future cross-app ordering detector
/// would thread them additively.
pub(crate) fn build_detector_context_cross_app(
    base: &crate::engine::l4::capability_cone::R3a5CrossAppBase,
) -> DetectorContext<'_> {
    use crate::engine::l4::summary_runner::compute_summaries_with_leaves;

    let ws_routines = &base.ws_routines;
    let dep_routine_ids = &base.dep_routine_ids;
    let graph = base.graph.clone();

    // Cone over the merged graph (direct facts/coverage already assembled in `base`).
    let cones = compose_cone_over_graph(
        &base.graph,
        &base.nodes,
        &base.direct_full,
        &base.direct_coverage,
    );
    let empty_facts: Vec<CapabilityFact> = Vec::new();
    let mut summaries: HashMap<String, FullRoutineSummary> = HashMap::new();
    for r in ws_routines {
        let cone = cones.get(&r.id);
        let inherited = cone.map(|c| c.inherited.clone()).unwrap_or_default();
        let coverage = cone.map(|c| c.coverage.clone());
        summaries.insert(
            r.id.clone(),
            FullRoutineSummary {
                routine_id: r.id.clone(),
                capability_facts_direct: base
                    .direct_full
                    .get(&r.id)
                    .unwrap_or(&empty_facts)
                    .clone(),
                capability_facts_inherited: inherited,
                coverage,
            },
        );
    }

    // --- Eager indexes (over the merged routine/object/table sets) ---------
    let routine_by_id: HashMap<&str, &L3Routine> =
        ws_routines.iter().map(|r| (r.id.as_str(), r)).collect();
    let objects_by_id: HashMap<&str, &L3Object> =
        base.objects.iter().map(|o| (o.id.as_str(), o)).collect();
    // G-5: REAL table wins an id collision with a tableextension stub.
    let table_by_id: HashMap<&str, &L3Table> =
        crate::engine::l3::l3_workspace::table_by_id_preferring_real(&base.tables);

    let reverse_call_graph = build_reverse_call_graph(&graph);

    let entry_points: BTreeSet<String> =
        crate::engine::l5::entry_points::find_entry_points(ws_routines, dep_routine_ids)
            .into_iter()
            .collect();

    let mut access_modifiers: HashMap<String, AccessModifier> = HashMap::new();
    for r in ws_routines {
        let access = match r.access_modifier.as_deref() {
            Some("local") => AccessModifier::Local,
            Some("internal") => AccessModifier::Internal,
            _ => AccessModifier::Public,
        };
        access_modifiers.insert(r.id.clone(), access);
    }
    let internal_reachable_externally = false;
    let reachable_roots: BTreeSet<String> = crate::engine::l5::entry_points::find_reachable_roots(
        ws_routines,
        dep_routine_ids,
        &access_modifiers,
        internal_reachable_externally,
    )
    .into_iter()
    .collect();

    // G-19 — closed-world proven-temp params (see the source-only builder).
    // Dep routines carry `access_modifier: None` (the ABI does not expose it),
    // so they can never be proven; primary `local` procedures still can.
    let closed_world_temp_params =
        crate::engine::l5::closed_world_temp::prove_closed_world_temp_params(
            ws_routines,
            &graph,
            &reverse_call_graph,
            &entry_points,
        );

    let transaction_spans = compute_transaction_spans(
        ws_routines,
        dep_routine_ids,
        &reverse_call_graph,
        &summaries,
    );

    let event_flow_indexes =
        build_event_flow_indexes(&base.event_graph, ws_routines, dep_routine_ids);

    // Cross-extension subscriber lookup, from the SAME inputs as `event_flow_indexes`
    // above — `base.event_graph` + `base.objects` (the merged cross-app event graph
    // + object set), consistent with how `fingerprint_index` below anchors to `base`.
    let cross_extension_subscribers =
        crate::engine::l5::event_flow::build_cross_extension_subscribers(
            &base.event_graph,
            &base.objects,
        );

    // Resolved-call-edge-by-callsite index: EMPTY for the cross-app context. The
    // cross-app build does not retain the raw resolver `calls.edges`, and d13/d16/d17
    // read edges directly off `ctx.graph` (the combined graph). Future cross-app
    // detectors that need this index would thread `calls` through `R3a5CrossAppBase`.
    let resolved_call_edge_by_callsite: HashMap<
        String,
        crate::engine::l3::call_resolver::CallEdge,
    > = HashMap::new();

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

    // Core summaries (JACOBI WITH dep leaves) for the path-walker uncertainty union +
    // parameter roles — same as project_r3a5_cross_app's core.
    let (core_summaries, _trace, summarize_diagnostics) = compute_summaries_with_leaves(
        ws_routines,
        &graph,
        &base.combined_scc,
        &base.upgraded_bindings,
        &base.field_index,
        false,
        &base.leaf_summaries,
    );

    let mut parameter_roles_by_routine: HashMap<String, Vec<RecordRoleSummary>> = HashMap::new();
    for r in ws_routines {
        if let Some(s) = core_summaries.get(&r.id)
            && !s.parameter_roles.is_empty()
        {
            parameter_roles_by_routine.insert(r.id.clone(), s.parameter_roles.clone());
        }
    }

    let mut uncertainties_by_node: HashMap<String, Vec<Uncertainty>> = HashMap::new();
    for r in ws_routines {
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
        let combined: Vec<Uncertainty> = from_summary.iter().cloned().chain(from_edges).collect();
        uncertainties_by_node.insert(r.id.clone(), dedupe_uncertainties(combined));
    }

    let mut call_site_by_id: HashMap<&str, &PCallSite> = HashMap::new();
    for r in ws_routines {
        for cs in &r.call_sites {
            call_site_by_id.insert(cs.id.as_str(), cs);
        }
    }

    let upgraded_bindings_by_callsite: HashMap<String, Vec<UpgradedBinding>> =
        base.upgraded_bindings.clone();

    // Build the fingerprint index from `base.ws_routines`/`base.objects` — NOT from
    // the throwaway `merged_workspace_view` (registry.rs's `run_detectors_cross_app`
    // clones the merged sets into a local `L3Resolved` it builds AFTER calling this
    // function, so that clone doesn't exist yet at this point and can't be borrowed
    // from here anyway). `base: &'a R3a5CrossAppBase` is already the ctx's own
    // borrow source for every other eager index above, so anchoring the fingerprint
    // index to it too keeps the lifetime honest — the same 'a the whole ctx uses.
    let fingerprint_index =
        crate::engine::l5::fingerprint::FingerprintIndex::build(&base.ws_routines, &base.objects);

    let app_versions: HashMap<String, String> = base.resolved_app_versions.clone();
    let declared_dependencies: Vec<DeclaredDep> = base
        .declared_dependencies
        .iter()
        .map(|d| DeclaredDep {
            app_guid: d.app_guid.clone(),
            name: d.name.clone(),
            min_version: d.min_version.clone(),
        })
        .collect();

    DetectorContext {
        graph,
        event_graph: base.event_graph.clone(),
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
        dep_routine_ids: dep_routine_ids.clone(),
        declared_dependencies,
        app_versions,
        root_classifications_by_routine: HashMap::new(),
        ordering_facts: std::sync::OnceLock::new(),
        ordering_source: None,
        closed_world_temp_params,
        summarize_diagnostics,
        fingerprint_index,
        cross_extension_subscribers,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Laziness contract: `build_detector_context` must NOT compute ordering facts
    /// (the OnceLock starts empty); first `get_ordering_facts()` call computes and
    /// memoizes a map EQUAL to a direct `compute_ordering_facts(resolved)` run.
    #[test]
    fn ordering_facts_are_lazy_and_parity_with_direct_compute() {
        // Empty workspace: cheap, and exercises the full lazy path end-to-end.
        let resolved = crate::engine::l3::l3_workspace::L3Resolved {
            workspace: crate::engine::l3::l3_workspace::L3Workspace {
                objects: Vec::new(),
                tables: Vec::new(),
                routines: Vec::new(),
            },
            root_classifications: Vec::new(),
            primary_app: None,
            infra_diagnostics: Vec::new(),
        };
        let ctx = build_detector_context(&resolved);
        assert!(
            ctx.ordering_facts.get().is_none(),
            "ordering facts must not be computed eagerly"
        );
        let via_ctx = ctx.get_ordering_facts();
        let direct = crate::engine::l5::ordering_facts::compute_ordering_facts(&resolved);
        assert_eq!(via_ctx.len(), direct.len());
        assert!(
            ctx.ordering_facts.get().is_some(),
            "first access must memoize"
        );
    }
}
