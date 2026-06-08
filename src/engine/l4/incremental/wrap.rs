//! Stage-1 WRAP driver (R3b Task 1) — build the Salsa DB from the R3a from-scratch
//! L4 base, DEMAND the summaries + cone through the Salsa queries, and project the
//! SAME full-RoutineSummary surface. The wrapped projection is byte-identical to
//! the R3a from-scratch projection (which, via R3a parity, byte-matches the al-sem
//! goldens). NO input edits / NO incrementality yet — every query recomputes on
//! the fresh DB.
//!
//! Both entry points return the same projection type the R3a goldens use, built
//! ENTIRELY from Salsa-demanded values:
//!   - [`salsa_r3a5_cross_app`] → `R3a5FullSummaryProjection` (the R3a-5 exit-gate
//!     cross-app surface),
//!   - [`salsa_r3a3_source_only`] → `R3a3Projection` (the R3a-3 source-only cone /
//!     coverage surface).

use std::collections::HashMap;
use std::sync::Arc;

use super::inputs::{AppContext, DepStamp, RoutineInput, RoutineRegistry, RoutineUniverse};
use super::queries::{cones, scc_summaries, InternalId};
use super::{L4Database, L4Db};
use crate::engine::l3::call_resolver::{resolve_calls, DeclaredDependency};
use crate::engine::l3::event_graph::build_event_graph;
use crate::engine::l3::l3_workspace::{L3Resolved, L3Routine, L3Workspace};
use crate::engine::l3::symbol_table::SymbolTable;
use crate::engine::l4::capability_cone::{
    build_r3a5_cross_app_base, direct_facts_for_routine, project_r3a3_from_parts,
    project_r3a5_from_parts, CapabilityFact, ConeResultPub, R3a3Projection,
    R3a5FullSummaryProjection,
};
use crate::engine::l4::combined_graph::{
    build_combined_graph, CombinedGraph, TypedEdge, UncertaintyEdge,
};
use crate::engine::l4::summary::RoutineSummary;
use crate::engine::l4::summary_runner::base_intraprocedural_summary;

/// A per-routine direct-fact provider: `routine → (facts, direct_status, reasons)`.
type DirectFactsFn<'a> = dyn Fn(&L3Routine) -> (Vec<CapabilityFact>, String, Vec<String>) + 'a;

/// The fully-built DB + the input handles needed to demand the query graph.
struct WrappedDb {
    db: L4Database,
    universe: RoutineUniverse,
    registry: RoutineRegistry,
    ctx: AppContext,
}

/// Build the Salsa DB from a from-scratch L4 base: the routine universe + one
/// per-routine [`RoutineInput`] (carrying its OWN edges / base summary / direct
/// facts / coverage / leaf flag) + the shared [`AppContext`]. The per-routine
/// edge slices come from the assembled combined graph (the SAME graph the
/// from-scratch path runs over).
#[allow(clippy::too_many_arguments)]
fn build_db(
    routines: &[L3Routine],
    graph: &CombinedGraph,
    objects: &[crate::engine::l3::l3_workspace::L3Object],
    tables: &[crate::engine::l3::l3_workspace::L3Table],
    event_graph: &crate::engine::l3::event_graph::EventGraph,
    upgraded_bindings: &HashMap<String, Vec<crate::engine::l3::call_resolver::UpgradedBinding>>,
    app_identity: &str,
    dep_stamp: &str,
    base_summary_for: &dyn Fn(&L3Routine) -> RoutineSummary,
    is_leaf_for: &dyn Fn(&str) -> Option<RoutineSummary>,
    direct_facts_for: &DirectFactsFn,
) -> WrappedDb {
    let db = L4Database::default();

    // Per-`from` typed/uncertainty edge slices (grouped from the global lists).
    let mut typed_by_from: HashMap<String, Vec<TypedEdge>> = HashMap::new();
    for te in &graph.typed_edges {
        typed_by_from
            .entry(te.from.clone())
            .or_default()
            .push(te.clone());
    }
    let mut uncertainty_by_from: HashMap<String, Vec<UncertaintyEdge>> = HashMap::new();
    for ue in &graph.uncertainty_edges {
        uncertainty_by_from
            .entry(ue.from.clone())
            .or_default()
            .push(ue.clone());
    }

    let mut routine_ids: Vec<String> = routines.iter().map(|r| r.id.clone()).collect();
    routine_ids.sort();

    let mut by_id: HashMap<String, RoutineInput> = HashMap::new();
    let mut stable_map: HashMap<String, String> = HashMap::new();
    for r in routines {
        stable_map.insert(r.id.clone(), r.stable_routine_id.clone());

        let combined_edges = graph.edges_by_from.get(&r.id).cloned().unwrap_or_default();
        let typed_edges = typed_by_from.get(&r.id).cloned().unwrap_or_default();
        let uncertainty_edges = uncertainty_by_from.get(&r.id).cloned().unwrap_or_default();

        let leaf = is_leaf_for(&r.id);
        let is_leaf = leaf.is_some();
        // The base summary input IS the retained leaf summary when leaf, else the
        // routine's base intraprocedural summary.
        let base_summary = leaf.unwrap_or_else(|| base_summary_for(r));

        let (facts, status, reasons) = direct_facts_for(r);

        let ri = RoutineInput::new(
            &db,
            r.id.clone(),
            Arc::new(r.clone()),
            Arc::new(combined_edges),
            Arc::new(typed_edges),
            Arc::new(uncertainty_edges),
            Arc::new(base_summary),
            Arc::new(facts),
            Arc::new((status, reasons)),
            r.body_available,
            is_leaf,
        );
        by_id.insert(r.id.clone(), ri);
    }

    let universe = RoutineUniverse::new(&db, routine_ids);
    let registry = RoutineRegistry::new(&db, Arc::new(by_id));
    let ctx = AppContext::new(
        &db,
        app_identity.to_string(),
        Arc::new(objects.to_vec()),
        Arc::new(tables.to_vec()),
        Arc::new(event_graph.clone()),
        Arc::new(upgraded_bindings.clone()),
        Arc::new(stable_map),
    );
    // The dep stamp input exists for Stage-2/3 cross-app invalidation; demanded
    // implicitly via the cross-app cone today, set here so the input graph is
    // complete.
    let _dep = DepStamp::new(&db, dep_stamp.to_string());

    WrappedDb {
        db,
        universe,
        registry,
        ctx,
    }
}

/// Demand every routine's settled CORE summary through the Salsa `scc_summaries`
/// query, returning the internal-id map (byte-identical to the from-scratch
/// `compute_summaries*` result).
fn demand_core_summaries(w: &WrappedDb) -> HashMap<String, RoutineSummary> {
    let db: &dyn L4Db = &w.db;
    let by_id = w.registry.by_id(db);
    let mut out: HashMap<String, RoutineSummary> = HashMap::new();

    // Demand each SCC's summaries via the routine's SccKey (drives the whole
    // inter-SCC dependency chain). Iterate routines; `scc_summaries` is memoized,
    // so each SCC computes once.
    for id in by_id.keys() {
        let iid = InternalId::new(db, id.clone());
        if let Some(key) = crate::engine::l4::incremental::queries::scc_for_routine(
            db, w.universe, w.registry, w.ctx, iid,
        ) {
            let s: super::queries::SccSummaries =
                scc_summaries(db, w.universe, w.registry, w.ctx, key);
            for (rid, summary) in s.summaries.iter() {
                out.entry(rid.clone()).or_insert_with(|| summary.clone());
            }
        }
    }
    // Leaves not reachable as anyone's SCC member still carry their retained
    // summary (seed from the inputs).
    for (id, ri) in by_id.iter() {
        if ri.is_leaf(db) {
            out.entry(id.clone())
                .or_insert_with(|| (**ri.base_summary(db)).clone());
        }
    }
    out
}

/// Demand the cone (`inherited_facts` + `coverage`) through the Salsa `cones`
/// query, returning the internal-id per-routine cone results.
fn demand_cones(w: &WrappedDb) -> HashMap<String, ConeResultPub> {
    let db: &dyn L4Db = &w.db;
    let c = cones(db, w.universe, w.registry);
    (*c.cones).clone()
}

// ===========================================================================
// Source-only (R3a-3) wrap.
// ===========================================================================

/// Build the Salsa DB over a SOURCE-ONLY resolved workspace and project the
/// R3a-3 cone/coverage surface ENTIRELY from Salsa-demanded values. Byte-identical
/// to `project_r3a3`.
pub fn salsa_r3a3_source_only(resolved: &L3Resolved) -> R3a3Projection {
    let ws: &L3Workspace = &resolved.workspace;
    let symbols = SymbolTable::build(&ws.objects, &ws.tables, &ws.routines);
    let no_deps: Vec<DeclaredDependency> = Vec::new();
    let no_fetched: Vec<String> = Vec::new();
    let calls = resolve_calls(ws, &symbols, &no_deps, &no_fetched);
    let event_graph = build_event_graph(&ws.routines, &symbols);
    let graph = build_combined_graph(ws, &calls, &event_graph);

    // Field index (parameterRoles + the uncertainty-reason fixed point need it).
    let mut field_index: crate::engine::l4::summary_runner::FieldIndex = HashMap::new();
    for table in &ws.tables {
        for field in &table.fields {
            field_index
                .entry((table.id.clone(), field.name.to_lowercase()))
                .or_insert_with(|| field.id.clone());
        }
    }
    let routines_by_id: HashMap<String, &L3Routine> =
        ws.routines.iter().map(|r| (r.id.clone(), r)).collect();

    // Publisher events for the direct-fact injection.
    let mut pub_by_routine: HashMap<String, Vec<&crate::engine::l3::event_graph::EventSymbol>> =
        HashMap::new();
    for evt in &event_graph.events {
        if let Some(pr) = &evt.publisher_routine_id {
            pub_by_routine.entry(pr.clone()).or_default().push(evt);
        }
    }

    // Source-only direct coverage carries the uncertainty-derived reasons (the
    // R3a-3 path folds them in). Reuse the SAME fixed-point the from-scratch path
    // runs so the coverage matches byte-for-byte: derive it via the public
    // `project_r3a3` uncertainty source — but that is private, so re-derive here
    // by running the core summaries (Salsa) and mapping the four uncertainty kinds.
    let w = build_db(
        &ws.routines,
        &graph,
        &ws.objects,
        &ws.tables,
        &event_graph,
        &calls.upgraded_bindings,
        "r3a3-source-only",
        "",
        &|r| base_intraprocedural_summary(r, &routines_by_id, &field_index),
        &|_id| None, // no leaves in the source-only model
        &|r| {
            let empty: Vec<&crate::engine::l3::event_graph::EventSymbol> = Vec::new();
            let pubs = pub_by_routine.get(&r.id).unwrap_or(&empty);
            direct_facts_for_routine(r, pubs)
        },
    );

    // Core summaries (Salsa) → the four uncertainty-derived coverage reasons.
    let core = demand_core_summaries(&w);
    let uncertainty_reasons = uncertainty_coverage_reasons(&core);

    // Re-build the DB with coverage that folds the uncertainty reasons (so the
    // cone's directStatus downgrade + reason forwarding matches the from-scratch
    // `project_r3a3`). The direct FACTS are unchanged; only `direct_coverage`.
    let w2 = build_db(
        &ws.routines,
        &graph,
        &ws.objects,
        &ws.tables,
        &event_graph,
        &calls.upgraded_bindings,
        "r3a3-source-only",
        "",
        &|r| base_intraprocedural_summary(r, &routines_by_id, &field_index),
        &|_id| None,
        &|r| {
            let empty: Vec<&crate::engine::l3::event_graph::EventSymbol> = Vec::new();
            let pubs = pub_by_routine.get(&r.id).unwrap_or(&empty);
            let (facts, mut status, reasons) = direct_facts_for_routine(r, pubs);
            let mut reason_set: std::collections::BTreeSet<String> =
                reasons.iter().cloned().collect();
            let base_len = reason_set.len();
            if let Some(extra) = uncertainty_reasons.get(&r.id) {
                for rr in extra {
                    reason_set.insert(rr.clone());
                }
            }
            let final_reasons: Vec<String> = if reason_set.len() > base_len {
                if status == "complete" {
                    status = "partial".to_string();
                }
                reason_set.into_iter().collect()
            } else {
                reasons
            };
            (facts, status, final_reasons)
        },
    );

    let cone_map = demand_cones(&w2);

    // Project (R3a-3 tail). Only routines with a cone entry are emitted.
    let map: HashMap<String, String> = ws
        .routines
        .iter()
        .map(|r| (r.id.clone(), r.stable_routine_id.clone()))
        .collect();
    let direct_full: HashMap<String, Vec<CapabilityFact>> = ws
        .routines
        .iter()
        .map(|r| {
            let empty: Vec<&crate::engine::l3::event_graph::EventSymbol> = Vec::new();
            let pubs = pub_by_routine.get(&r.id).unwrap_or(&empty);
            (r.id.clone(), direct_facts_for_routine(r, pubs).0)
        })
        .collect();

    project_r3a3_from_parts(&ws.routines, &cone_map, &direct_full, &event_graph, &map)
}

/// Map a routine's L4 fixed-point uncertainties to the four coverage reasons.
/// Mirrors the source-only `compute_uncertainty_coverage_reasons`.
fn uncertainty_coverage_reasons(
    core: &HashMap<String, RoutineSummary>,
) -> HashMap<String, std::collections::BTreeSet<String>> {
    let mut out: HashMap<String, std::collections::BTreeSet<String>> = HashMap::new();
    for (rid, summary) in core {
        let mut reasons: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for u in &summary.uncertainties {
            match u.kind.as_str() {
                "ambiguous-overload"
                | "member-not-found"
                | "external-target"
                | "interface-open-world" => {
                    reasons.insert(u.kind.clone());
                }
                _ => {}
            }
        }
        if !reasons.is_empty() {
            out.insert(rid.clone(), reasons);
        }
    }
    out
}

// ===========================================================================
// Cross-app (R3a-5) wrap — the EXIT-GATE surface.
// ===========================================================================

/// Build the Salsa DB over the cross-app L4 base and project the R3a-5 full
/// cross-app summary surface ENTIRELY from Salsa-demanded values. Byte-identical
/// to `project_r3a5_cross_app`. Engine-never-throws: a fail-closed / dep-less
/// workspace yields an empty projection.
pub fn salsa_r3a5_cross_app(
    workspace: &std::path::Path,
    model_instance_id: &str,
    fixture_name: &str,
) -> R3a5FullSummaryProjection {
    let empty = R3a5FullSummaryProjection {
        fixture_name: fixture_name.to_string(),
        summaries: Vec::new(),
        primary_routines_with_inherited_dep_facts: 0,
        primary_routines_with_dep_db_effects: 0,
        coverages_with_opaque_apps_reason: 0,
        total_cross_app_inherited_facts: 0,
    };
    let Some(base) = build_r3a5_cross_app_base(workspace, model_instance_id) else {
        return empty;
    };

    let leaf_summaries = base.leaf_summaries.clone();
    let direct_full = base.direct_full.clone();
    let direct_coverage = base.direct_coverage.clone();
    let routines_by_id: HashMap<String, &L3Routine> =
        base.ws_routines.iter().map(|r| (r.id.clone(), r)).collect();
    let field_index = &base.field_index;

    let w = build_db(
        &base.ws_routines,
        &base.graph,
        &base.objects,
        &base.tables,
        &base.event_graph,
        &base.upgraded_bindings,
        &base.app_guid,
        &base.app_guid, // dep stamp (cosmetic for Stage 1)
        // Non-leaf seed: the routine's base intraprocedural summary (parity with
        // `compute_summaries_with_leaves`, which skips the base for leaves).
        &|r| base_intraprocedural_summary(r, &routines_by_id, field_index),
        &|id| leaf_summaries.get(id).cloned(),
        &|r| {
            let facts = direct_full.get(&r.id).cloned().unwrap_or_default();
            let (status, reasons) = direct_coverage
                .get(&r.id)
                .cloned()
                .unwrap_or_else(|| ("unknown".to_string(), Vec::new()));
            (facts, status, reasons)
        },
    );

    let core = demand_core_summaries(&w);
    let cone_map = demand_cones(&w);

    project_r3a5_from_parts(
        &base.ws_routines,
        &base.dep_routine_ids,
        &core,
        &cone_map,
        &base.direct_full,
        &base.event_graph,
        fixture_name,
    )
}
