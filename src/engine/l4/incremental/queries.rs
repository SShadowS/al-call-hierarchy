//! The tracked Salsa QUERIES — the R3b L4 query graph (Task 1, Stage 1).
//!
//! Topology (see `mod.rs`): `combined_graph` → `scc_condensation` (populates the
//! projections) → interned [`SccKey`] → `scc_for_routine` / `scc_members` /
//! `scc_successors` (early-cutting) → `scc_summaries` (the internal JACOBI over
//! `scc_members`, depending on successor `scc_summaries` — NOT the monolithic
//! condensation) → `routine_summary` → `inherited_facts` + `coverage`.
//!
//! Query BODIES reuse the R3a `src/engine/l4/*` logic (combined-graph assembly,
//! `tarjan_scc`, `run_one_scc`, `compose_inherited_cones`) — they WRAP, not
//! re-port. Stage 1 builds a fresh DB and demands the summaries; every query
//! recomputes (no incrementality yet).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use super::inputs::{AppContext, RoutineRegistry, RoutineUniverse};
use super::L4Db;
use crate::engine::l4::combined_graph::{CombinedEdge, CombinedGraph, UncertaintyEdge};
use crate::engine::l4::scc::{tarjan_scc, Scc, SccInputGraph, SccResult};
use crate::engine::l4::summary::RoutineSummary;
use crate::engine::l4::summary_runner::{run_one_scc, FieldIndex, SccComputeCtx};

// ---------------------------------------------------------------------------
// Tracked-return CARRIERS. Salsa 0.27 requires a tracked fn's return value to be
// `PartialEq` (for backdating: old == new ⇒ dependents are NOT re-fired). For
// STAGE 2 (Task 2) the carriers carry VALUE EQUALITY so an unchanged query output
// BACKDATES and does NOT propagate (the Rev-2 #3 early-cutoff). The R3a payloads
// behind these carriers are now structurally `PartialEq` (the derive was added on
// `CombinedGraph`/`SccResult`/`RoutineSummary`/`ConeResultPub` + their inner
// types), so comparing the `Arc`-pointed VALUES is exact. The compare is fast-
// pathed by `Arc::ptr_eq` (same allocation ⇒ trivially equal), then falls back to
// the byte-faithful structural compare — so a recompute that produces a
// byte-identical value backdates (no propagation), while any real change is
// detected. This is what makes Stage 3 minimality achievable; it is sound because
// the value-eq compares the exact output the from-scratch path would emit.
//
// (The carrier `PartialEq` cannot be `derive`d: the inner `Arc<T>`'s derived
// `PartialEq` compares the pointed VALUE already, but we add the `ptr_eq` fast
// path explicitly for the common same-`Arc` case and to document intent.)
macro_rules! value_eq_carrier {
    ($ty:ident, $field:ident) => {
        impl PartialEq for $ty {
            fn eq(&self, other: &Self) -> bool {
                // Fast path: the same allocation is trivially value-equal.
                Arc::ptr_eq(&self.$field, &other.$field) || *self.$field == *other.$field
            }
        }
    };
}

// VALUE equality (with an `Arc::ptr_eq` fast path). `Eq` only where the inner
// value is `Eq` (CapabilityFact carries a non-`Eq` field, so the cone/summary
// carriers that transitively hold it are `PartialEq`-only — Salsa only requires
// `PartialEq` for backdating).
value_eq_carrier!(CombinedGraphValue, graph);
impl Eq for CombinedGraphValue {}
value_eq_carrier!(CondensationValue, result);
impl Eq for CondensationValue {}
value_eq_carrier!(SccSummaries, summaries);
impl Eq for SccSummaries {}
value_eq_carrier!(SummaryValue, summary);
impl Eq for SummaryValue {}
value_eq_carrier!(ConeValue, cones);

// ===========================================================================
// Interned SccKey — semantic identity = the SORTED member StableRoutineId set.
// A merge/split mints a NEW key; an unchanged SCC re-interns to the SAME key.
// (Rev 2 #1 — NOT a Tarjan index / Vec position / reverse-topo index.)
// ===========================================================================

/// The canonical SCC identity: the interned SORTED member StableRoutineId set.
#[salsa::interned(debug)]
pub struct SccKey<'db> {
    /// Sorted StableRoutineIds of the SCC's members (the semantic identity).
    #[returns(ref)]
    pub members: Vec<String>,
}

// ===========================================================================
// combined_graph (STRUCTURAL) — reassemble the CombinedGraph from the per-routine
// edge inputs over the routine universe. (Rebuilds al-sem's `buildCombinedGraph`
// OUTPUT from the fine-grained inputs rather than re-running it monolithically.)
// ===========================================================================

/// A serde/Salsa-friendly carrier for the reassembled combined graph (it is the
/// R3a `CombinedGraph`, behind an `Arc` so the tracked return is cheap to clone).
#[derive(Clone, salsa::Update)]
pub struct CombinedGraphValue {
    pub graph: Arc<CombinedGraph>,
}

#[salsa::tracked]
pub fn combined_graph(
    db: &dyn L4Db,
    universe: RoutineUniverse,
    registry: RoutineRegistry,
) -> CombinedGraphValue {
    let ids = universe.routine_ids(db);
    let by_id = registry.by_id(db);

    // Sorted node list (the universe is already sorted, but be defensive).
    let mut nodes: Vec<String> = ids.clone();
    nodes.sort();

    // Per-`from` combined edges + the flat uncertainty list, assembled from each
    // routine's OWN outgoing-edge inputs (the fine-grained slices).
    let mut edges_by_from: HashMap<String, Vec<CombinedEdge>> = HashMap::new();
    let mut uncertainty_edges: Vec<UncertaintyEdge> = Vec::new();
    let mut typed_edges = Vec::new();

    for id in &nodes {
        let Some(ri) = by_id.get(id) else { continue };
        let ce = ri.combined_edges(db);
        if !ce.is_empty() {
            // Each routine's slice is already edgeSortKey-sorted (assembled from
            // the R3a graph), keep as-is.
            edges_by_from.insert(id.clone(), (**ce).clone());
        }
        for ue in ri.uncertainty_edges(db).iter() {
            uncertainty_edges.push(ue.clone());
        }
        for te in ri.typed_edges(db).iter() {
            typed_edges.push(te.clone());
        }
    }

    // The uncertainty edges + typed edges are emitted in the R3a global order; the
    // R3a build sorts uncertainty edges by uncertaintySortKey. Reproduce that sort
    // (the slices arrive grouped per-from; re-sort globally for parity).
    uncertainty_edges.sort_by_key(uncertainty_sort_key);

    let graph = CombinedGraph {
        nodes,
        edges_by_from,
        uncertainty_edges,
        typed_edges,
    };
    CombinedGraphValue {
        graph: Arc::new(graph),
    }
}

/// `uncertaintySortKey` — `${from}|${kind}|${ref}` (ref = callsiteId, else
/// operationId, else routineId). Mirrors the R3a combined-graph sort.
fn uncertainty_sort_key(ue: &UncertaintyEdge) -> String {
    let u = &ue.uncertainty;
    let r = u
        .callsite_id
        .clone()
        .or_else(|| u.operation_id.clone())
        .or_else(|| u.routine_id.clone())
        .unwrap_or_default();
    format!("{}|{}|{}", ue.from, u.kind, r)
}

// ===========================================================================
// scc_condensation (STRUCTURAL) — the Tarjan pass over the combined graph. Its
// output POPULATES the projection queries; `scc_summaries` does NOT depend on it
// directly. (Rev 2 #1/#3.)
// ===========================================================================

/// The condensation: the reverse-topo SCC list + member→SCC index, behind an
/// `Arc`. Consumed ONLY to populate the projection queries below.
#[derive(Clone, salsa::Update)]
pub struct CondensationValue {
    pub result: Arc<SccResult>,
}

#[salsa::tracked]
pub fn scc_condensation(
    db: &dyn L4Db,
    universe: RoutineUniverse,
    registry: RoutineRegistry,
) -> CondensationValue {
    let cg = combined_graph(db, universe, registry);
    let graph = &cg.graph;
    let mut adjacency: HashMap<String, Vec<String>> = HashMap::new();
    for (from, list) in &graph.edges_by_from {
        adjacency.insert(from.clone(), list.iter().map(|e| e.to.clone()).collect());
    }
    let result = tarjan_scc(&SccInputGraph {
        nodes: &graph.nodes,
        edges_by_from: &adjacency,
    });
    CondensationValue {
        result: Arc::new(result),
    }
}

// ===========================================================================
// PROJECTION queries — EARLY-CUT for an SCC the edit didn't touch (Rev 2 #1).
// These map the internal-id condensation onto the interned SccKey identity.
// ===========================================================================

/// The list of all SccKeys in reverse-topological (callee-first) order. Each key
/// is the interned SORTED-member-StableRoutineId set of one SCC. (The order is a
/// structural fact; `scc_summaries` does not depend on it — it walks successors.)
#[salsa::tracked]
pub fn all_scc_keys<'db>(
    db: &'db dyn L4Db,
    universe: RoutineUniverse,
    registry: RoutineRegistry,
    ctx: AppContext,
) -> Vec<SccKey<'db>> {
    let cond = scc_condensation(db, universe, registry);
    let stable_map = ctx.stable_map(db);
    cond.result
        .sccs
        .iter()
        .map(|scc| SccKey::new(db, stable_members(scc, stable_map)))
        .collect()
}

/// `scc_for_routine(stable_id)` — the SccKey of the SCC containing the routine.
/// EARLY-CUTS: unchanged for a routine the edit didn't move between SCCs.
#[salsa::tracked]
pub fn scc_for_routine<'db>(
    db: &'db dyn L4Db,
    universe: RoutineUniverse,
    registry: RoutineRegistry,
    ctx: AppContext,
    internal_id: InternalId<'db>,
) -> Option<SccKey<'db>> {
    let cond = scc_condensation(db, universe, registry);
    let stable_map = ctx.stable_map(db);
    let idx = cond.result.scc_id_by_routine.get(internal_id.id(db))?;
    let scc = cond.result.sccs.get(*idx)?;
    Some(SccKey::new(db, stable_members(scc, stable_map)))
}

/// `scc_members(scc_key)` — the SORTED internal member ids of the SCC. EARLY-CUTS
/// for an SCC whose member set is unchanged. The JACOBI loop iterates THIS (in
/// sorted StableRoutineId order — see [`scc_summaries`]).
#[salsa::tracked]
pub fn scc_members<'db>(
    db: &'db dyn L4Db,
    universe: RoutineUniverse,
    registry: RoutineRegistry,
    ctx: AppContext,
    scc_key: SccKey<'db>,
) -> Vec<String> {
    let cond = scc_condensation(db, universe, registry);
    let stable_map = ctx.stable_map(db);
    let want = scc_key.members(db);
    for scc in &cond.result.sccs {
        if &stable_members(scc, stable_map) == want {
            return scc.members.clone();
        }
    }
    Vec::new()
}

/// `scc_is_recursive(scc_key) -> bool` — the SCC's `recursive` flag (size > 1 OR a
/// self-edge), exposed as a VALUE-EQUAL `bool` projection. This is the PRECONDITION
/// fix (Task-1 review HIGH): `scc_summaries` reads the recursive flag from HERE, not
/// from the monolithic `scc_condensation` carrier. Because the return is a plain
/// `bool`, an SCC whose recursiveness is unchanged BACKDATES — even though this
/// query scans the (re-derived) condensation internally, its OUTPUT is value-equal,
/// so `scc_summaries` does NOT re-fire on an unrelated edit. (The condensation
/// recompute itself is a Stage-3 minimality concern; the point here is that the
/// recursive flag no longer routes the always-changed condensation VALUE into
/// `scc_summaries`.)
#[salsa::tracked]
pub fn scc_is_recursive<'db>(
    db: &'db dyn L4Db,
    universe: RoutineUniverse,
    registry: RoutineRegistry,
    ctx: AppContext,
    scc_key: SccKey<'db>,
) -> bool {
    let cond = scc_condensation(db, universe, registry);
    let stable_map = ctx.stable_map(db);
    let want = scc_key.members(db);
    for scc in &cond.result.sccs {
        if &stable_members(scc, stable_map) == want {
            return scc.recursive;
        }
    }
    // No matching SCC (should not happen for a demanded key): fall back to the
    // structural default the from-scratch path uses (size > 1 ⇒ recursive).
    want.len() > 1
}

/// `scc_successors(scc_key)` — the SORTED-by-StableRoutineId-members SccKeys of
/// the condensation-DAG successors (callees) of this SCC. EARLY-CUTS. The
/// inter-SCC deps are a clean DAG (no Salsa cycle-detection trip).
#[salsa::tracked]
pub fn scc_successors<'db>(
    db: &'db dyn L4Db,
    universe: RoutineUniverse,
    registry: RoutineRegistry,
    ctx: AppContext,
    scc_key: SccKey<'db>,
) -> Vec<SccKey<'db>> {
    let cond = scc_condensation(db, universe, registry);
    let cg = combined_graph(db, universe, registry);
    let stable_map = ctx.stable_map(db);
    let result = &cond.result;

    // Find this SCC's index.
    let want = scc_key.members(db);
    let Some(my_idx) = result
        .sccs
        .iter()
        .position(|scc| &stable_members(scc, stable_map) == want)
    else {
        return Vec::new();
    };
    let my_scc = &result.sccs[my_idx];

    // Distinct successor SCC indices (cross-SCC combined edges only).
    let mut succ: BTreeSet<usize> = BTreeSet::new();
    let empty: Vec<CombinedEdge> = Vec::new();
    for m in &my_scc.members {
        for e in cg.graph.edges_by_from.get(m).unwrap_or(&empty) {
            if let Some(j) = result.scc_id_by_routine.get(&e.to) {
                if *j != my_idx {
                    succ.insert(*j);
                }
            }
        }
    }
    // Mint the successor keys, sorted by their (sorted-member) StableRoutineId set.
    let mut keys: Vec<Vec<String>> = succ
        .iter()
        .filter_map(|j| result.sccs.get(*j))
        .map(|scc| stable_members(scc, stable_map))
        .collect();
    keys.sort();
    keys.into_iter().map(|m| SccKey::new(db, m)).collect()
}

/// Internal-RoutineId carrier so a per-routine projection query can be keyed by an
/// interned id (Salsa tracked-fn args must be Salsa entities or `Copy` scalars).
#[salsa::interned(debug)]
pub struct InternalId<'db> {
    #[returns(ref)]
    pub id: String,
}

// ===========================================================================
// PER-ROUTINE edge queries (Stage 3 re-granularization, the CRUX). These replace
// the monolithic `combined_graph` read inside `scc_summaries`: each member reads
// ONLY its OWN outgoing edge slice + uncertainty slice. The value is exactly the
// per-`from` slice the structural `combined_graph` would emit (the inputs are the
// same per-routine `combined_edges`/`uncertainty_edges` slices), so it BACKDATES
// when a routine's own edges are unchanged — an edit to a DIFFERENT routine's
// edges leaves THIS routine's slice value-equal. `scc_summaries(scc_key)` builds a
// per-SCC mini combined graph from ITS MEMBERS' slices (+ the members' edge
// targets' `body_available`), so it no longer reads the whole graph.
// ===========================================================================

/// A value-equal carrier for one routine's outgoing combined-edge slice (the
/// `edgeSortKey`-sorted slice the combined graph groups under `from`). Behind an
/// `Arc` so the tracked return is cheap; `PartialEq` is the structural slice compare
/// (with the `Arc::ptr_eq` fast path) so an unchanged slice backdates.
#[derive(Clone, salsa::Update)]
pub struct RoutineCombinedEdges {
    pub edges: Arc<Vec<CombinedEdge>>,
}
value_eq_carrier!(RoutineCombinedEdges, edges);
impl Eq for RoutineCombinedEdges {}

/// `routine_combined_edges(internal_id)` — one routine's OUTGOING combined edges,
/// read straight from its own `RoutineInput::combined_edges`. Depends ONLY on that
/// one routine's edge input (NOT the whole graph), so editing another routine's
/// edges leaves this value-equal ⇒ this query backdates.
#[salsa::tracked]
pub fn routine_combined_edges<'db>(
    db: &'db dyn L4Db,
    registry: RoutineRegistry,
    internal_id: InternalId<'db>,
) -> RoutineCombinedEdges {
    let by_id = registry.by_id(db);
    let edges = by_id
        .get(internal_id.id(db))
        .map(|ri| ri.combined_edges(db).clone())
        .unwrap_or_else(|| Arc::new(Vec::new()));
    RoutineCombinedEdges { edges }
}

/// A value-equal carrier for one routine's outgoing uncertainty-edge slice.
#[derive(Clone, salsa::Update)]
pub struct RoutineUncertaintyEdges {
    pub edges: Arc<Vec<UncertaintyEdge>>,
}
value_eq_carrier!(RoutineUncertaintyEdges, edges);
impl Eq for RoutineUncertaintyEdges {}

/// `routine_uncertainty_edges(internal_id)` — one routine's OUTGOING uncertainty
/// edges (to-less callsites), read from its own `RoutineInput::uncertainty_edges`.
/// Per-routine ⇒ backdates when that routine's slice is unchanged.
#[salsa::tracked]
pub fn routine_uncertainty_edges<'db>(
    db: &'db dyn L4Db,
    registry: RoutineRegistry,
    internal_id: InternalId<'db>,
) -> RoutineUncertaintyEdges {
    let by_id = registry.by_id(db);
    let edges = by_id
        .get(internal_id.id(db))
        .map(|ri| ri.uncertainty_edges(db).clone())
        .unwrap_or_else(|| Arc::new(Vec::new()));
    RoutineUncertaintyEdges { edges }
}

/// A value-equal carrier for one routine's `body_available` flag (a plain `bool`
/// projection so an edge target's bodyAvailable feeds `scc_summaries` per-routine,
/// not via the monolithic `by_id` scan).
#[salsa::tracked]
pub fn routine_body_available<'db>(
    db: &'db dyn L4Db,
    registry: RoutineRegistry,
    internal_id: InternalId<'db>,
) -> bool {
    let by_id = registry.by_id(db);
    by_id
        .get(internal_id.id(db))
        .map(|ri| ri.body_available(db))
        .unwrap_or(false)
}

/// `routine_leaf_summary(internal_id)` — `Some(retained summary)` when the routine
/// is a FIXED LEAF (an R3a-5 dep routine), else `None`. Per-routine (reads only
/// THIS routine's `is_leaf` + `base_summary`). The from-scratch path pre-seeds EVERY
/// leaf's retained summary into the global `final_map`; per-SCC, `scc_summaries`
/// seeds the retained summary of each of its members' EDGE-TARGET leaves into the
/// `predecessor_final_map` (a leaf's own SCC `scc_summaries` returns empty, so the
/// retained summary must be sourced here, not via the successor recursion).
#[salsa::tracked]
pub fn routine_leaf_summary<'db>(
    db: &'db dyn L4Db,
    registry: RoutineRegistry,
    internal_id: InternalId<'db>,
) -> Option<SummaryValue> {
    let by_id = registry.by_id(db);
    by_id.get(internal_id.id(db)).and_then(|ri| {
        if ri.is_leaf(db) {
            Some(SummaryValue {
                summary: ri.base_summary(db).clone(),
            })
        } else {
            None
        }
    })
}

// ===========================================================================
// scc_summaries(scc_key) — the internal JACOBI loop over `scc_members` (in SORTED
// StableRoutineId order). Depends on `scc_members` / `scc_successors` / the
// members' inputs / successor `scc_summaries` — NOT the monolithic condensation.
// Reuses the PROVEN R3a `run_one_scc` (no re-port).
// ===========================================================================

/// One SCC's settled summaries (internal RoutineId → RoutineSummary), behind an
/// `Arc`. The tracked return value is byte-identical to the from-scratch
/// per-SCC result.
#[derive(Clone, salsa::Update)]
pub struct SccSummaries {
    pub summaries: Arc<BTreeMap<String, RoutineSummary>>,
}

#[salsa::tracked]
pub fn scc_summaries<'db>(
    db: &'db dyn L4Db,
    universe: RoutineUniverse,
    registry: RoutineRegistry,
    ctx: AppContext,
    scc_key: SccKey<'db>,
) -> SccSummaries {
    let by_id = registry.by_id(db);
    let stable_map_arc = ctx.stable_map(db);

    // The members, in INTERNAL-id order from the projection query.
    let members = scc_members(db, universe, registry, ctx, scc_key);

    // --- DEMAND every SUCCESSOR SCC's summaries (the inter-SCC dependency) and
    //     fold them into the predecessor `final_map` (callees settle first). ---
    let mut predecessor_final_map: HashMap<String, RoutineSummary> = HashMap::new();
    for succ in scc_successors(db, universe, registry, ctx, scc_key) {
        let s = scc_summaries(db, universe, registry, ctx, succ);
        for (id, summary) in s.summaries.iter() {
            predecessor_final_map.insert(id.clone(), summary.clone());
        }
    }
    // Fixed leaves whose summary the members fold in must also be visible. A leaf
    // is its own singleton SCC (no outgoing edges), so it is reached as a
    // successor above — EXCEPT a leaf with no caller-visible edge. To be safe,
    // seed any member-referenced leaf that is itself NOT a member.
    // (Leaves are pre-seeded into final_map in the from-scratch path; here they
    // arrive via the successor recursion. Their retained `base_summary` is used.)

    // === RE-GRANULARIZATION (Stage 3, the CRUX) ============================
    // Build a PER-SCC mini combined graph + per-SCC lookup maps from ONLY this
    // SCC's MEMBERS' per-routine inputs (NOT the whole `combined_graph` / `by_id`
    // scan). For each member `m` we read m's OWN `routine_combined_edges(m)` +
    // `routine_uncertainty_edges(m)` + m's `routine`/`base_summary`/`is_leaf`/
    // `body_available` inputs. The JACOBI's `compose_routine` additionally reads
    // each edge TARGET's `body_available` (the opaque-callee guard) — we read THOSE
    // per-target via `routine_body_available(target)`. The callee SUMMARIES arrive
    // via `predecessor_final_map` (successor `scc_summaries`), NOT via the ctx maps.
    //
    // RESULT: this query depends ONLY on {its members' inputs + edges} ∪ {its edge
    // targets' body_available} ∪ {successor `scc_summaries`} ∪ {scc_members /
    // scc_successors / scc_is_recursive(this key)}. An edit isolated to an unrelated
    // SCC's input leaves all of those value-equal ⇒ this query BACKDATES. The
    // monolithic `combined_graph`/`by_id.iter()` reads are GONE.
    // =======================================================================

    // Per-member edge slices → the mini combined graph (only members as `from`).
    let mut edges_by_from: HashMap<String, Vec<CombinedEdge>> = HashMap::new();
    let mut mini_uncertainty: Vec<UncertaintyEdge> = Vec::new();
    // The set of edge TARGETS referenced by members (need their body_available).
    let mut edge_targets: BTreeSet<String> = BTreeSet::new();
    for m in &members {
        let iid = InternalId::new(db, m.clone());
        let me = routine_combined_edges(db, registry, iid);
        if !me.edges.is_empty() {
            for e in me.edges.iter() {
                edge_targets.insert(e.to.clone());
            }
            edges_by_from.insert(m.clone(), (*me.edges).clone());
        }
        for ue in routine_uncertainty_edges(db, registry, iid).edges.iter() {
            mini_uncertainty.push(ue.clone());
        }
    }
    // `compose_routine` filters `graph.uncertainty_edges` by `from == member`, so
    // the per-SCC list (members' own slices) is exactly what it consumes; the
    // global uncertaintySortKey sort is irrelevant under that filter, but keep a
    // deterministic order for stability.
    mini_uncertainty.sort_by_key(uncertainty_sort_key);

    // The mini-graph `nodes` list only needs to cover the members + their targets
    // for any `nodes`-based read; `compose_routine` reads only `edges_by_from` and
    // `uncertainty_edges`, so an exact node list is not required — supply members +
    // targets for completeness/determinism.
    let mut mini_nodes: BTreeSet<String> = members.iter().cloned().collect();
    mini_nodes.extend(edge_targets.iter().cloned());
    let mini_graph = CombinedGraph {
        nodes: mini_nodes.into_iter().collect(),
        edges_by_from,
        uncertainty_edges: mini_uncertainty,
        typed_edges: Vec::new(),
    };

    // Per-member lookup maps (routines_by_id / base_summaries / leaf_summaries) —
    // ONLY this SCC's members (compose_routine is called for members only).
    let mut routine_arcs: HashMap<String, Arc<crate::engine::l3::l3_workspace::L3Routine>> =
        HashMap::new();
    let mut base_summaries: HashMap<String, RoutineSummary> = HashMap::new();
    let mut leaf_summaries: HashMap<String, RoutineSummary> = HashMap::new();
    let mut body_avail_by_id: HashMap<String, bool> = HashMap::new();
    for m in &members {
        let Some(ri) = by_id.get(m) else { continue };
        routine_arcs.insert(m.clone(), ri.routine(db).clone());
        body_avail_by_id.insert(m.clone(), ri.body_available(db));
        let base = ri.base_summary(db);
        if ri.is_leaf(db) {
            leaf_summaries.insert(m.clone(), (**base).clone());
        } else {
            base_summaries.insert(m.clone(), (**base).clone());
        }
    }
    // Edge TARGETS' body_available (read per-target — the opaque-callee guard in
    // `compose_routine` does `body_avail_by_id.get(&edge.to)`). A target that is
    // itself a member is already present; only fill the non-member targets, each
    // via its own per-routine `routine_body_available` query (NOT the global scan).
    //
    // ALSO seed each edge-target FIXED LEAF's retained summary into the
    // `predecessor_final_map`: the from-scratch path pre-seeds EVERY leaf into the
    // global `final_map`, so a member that calls a dep leaf folds in its retained
    // summary. A leaf's OWN `scc_summaries` returns empty (run_one_scc short-circuits
    // a leaf), so the retained summary must be sourced per-target here — NOT via the
    // successor recursion. (`routine_leaf_summary` is per-routine ⇒ granular.)
    for t in &edge_targets {
        let tid = InternalId::new(db, t.clone());
        body_avail_by_id
            .entry(t.clone())
            .or_insert_with(|| routine_body_available(db, registry, tid));
        if let Some(leaf) = routine_leaf_summary(db, registry, tid) {
            predecessor_final_map
                .entry(t.clone())
                .or_insert_with(|| (*leaf.summary).clone());
        }
    }
    let routines_by_id: HashMap<String, &crate::engine::l3::l3_workspace::L3Routine> = routine_arcs
        .iter()
        .map(|(id, a)| (id.clone(), a.as_ref()))
        .collect();

    // Leaves must also be in the predecessor map (the from-scratch path pre-seeds
    // them into final_map). Add any member-leaf not already present from successors.
    for (id, s) in &leaf_summaries {
        predecessor_final_map
            .entry(id.clone())
            .or_insert_with(|| s.clone());
    }

    let upgraded = ctx.upgraded_bindings(db);

    // Build the from-scratch `Scc` entry ENTIRELY from the VALUE-EQUAL projection
    // queries: `scc_members` (the sorted internal member ids) + `scc_is_recursive`
    // (the value-equal `bool` flag). The PRECONDITION fix (Task-1 review HIGH):
    // `scc_summaries` no longer reads the monolithic `scc_condensation` carrier —
    // its SCC deps are `scc_members` / `scc_is_recursive` / `scc_successors` / the
    // members' per-routine inputs / successor `scc_summaries`. An edit that does not
    // touch THIS SCC's members / recursiveness / successors / inputs leaves all of
    // those value-equal, so this query BACKDATES (no re-fire) even though the
    // condensation always recomputes.
    let want = scc_key.members(db);
    let recursive = scc_is_recursive(db, universe, registry, ctx, scc_key);
    let scc_entry = Scc {
        members: members.clone(),
        recursive,
    };

    // ASSERTION (Rev 2 #4): the SCC member iteration order is the SORTED
    // StableRoutineId set — the JACOBI loop iterates members canonically. The R3a
    // `Scc.members` is already sorted by INTERNAL id; assert the projection of
    // that order is the sorted-StableRoutineId order we interned the key from.
    debug_assert_eq!(
        &stable_members(&scc_entry, stable_map_arc),
        want,
        "scc_summaries: member iteration order must be the sorted StableRoutineId set"
    );

    let sctx = SccComputeCtx {
        routines_by_id: &routines_by_id,
        base_summaries: &base_summaries,
        upgraded_bindings: upgraded,
        graph: &mini_graph,
        body_avail_by_id: &body_avail_by_id,
        stable_map: stable_map_arc,
        leaf_summaries: &leaf_summaries,
    };

    let out = run_one_scc(&scc_entry, &predecessor_final_map, &sctx, false);
    let mut map: BTreeMap<String, RoutineSummary> = BTreeMap::new();
    for (id, s) in out.summaries {
        map.insert(id, s);
    }
    SccSummaries {
        summaries: Arc::new(map),
    }
}

/// `routine_summary(stable_id)` — the settled CORE summary for one routine,
/// pulled from its SCC's `scc_summaries`. (The cone `inherited_facts` + `coverage`
/// are computed in `wrap.rs` over the full typed-edge graph, mirroring R3a's
/// `compose_inherited_cones`; this query exposes the R3a-2 CORE per routine.)
#[salsa::tracked]
pub fn routine_summary<'db>(
    db: &'db dyn L4Db,
    universe: RoutineUniverse,
    registry: RoutineRegistry,
    ctx: AppContext,
    internal_id: InternalId<'db>,
) -> Option<SummaryValue> {
    let key = scc_for_routine(db, universe, registry, ctx, internal_id)?;
    let s = scc_summaries(db, universe, registry, ctx, key);
    s.summaries
        .get(internal_id.id(db))
        .cloned()
        .map(|summary| SummaryValue {
            summary: Arc::new(summary),
        })
}

/// A per-routine settled summary carrier.
#[derive(Clone, salsa::Update)]
pub struct SummaryValue {
    pub summary: Arc<RoutineSummary>,
}

// ===========================================================================
// The cone — `inherited_facts` + `coverage`. Wraps R3a `compose_cone_over_graph`
// (the typed-edge SCC fused bottom-up pass). For Stage 1 a single `cones` tracked
// query computes all routines' cones; the per-routine accessors below expose the
// `inherited_facts(stable_id)` + `coverage(stable_id)` surface. (The cone's own
// early-cutting decomposition is a Stage-2/3 refinement; the VALUES are byte-
// identical to R3a here.)
// ===========================================================================

/// All routines' cone results (internal RoutineId → inherited facts + coverage),
/// behind an `Arc`.
#[derive(Clone, salsa::Update)]
pub struct ConeValue {
    pub cones: Arc<HashMap<String, crate::engine::l4::capability_cone::ConeResultPub>>,
}

#[salsa::tracked]
pub fn cones(db: &dyn L4Db, universe: RoutineUniverse, registry: RoutineRegistry) -> ConeValue {
    let cg = combined_graph(db, universe, registry);
    let by_id = registry.by_id(db);
    let nodes = &cg.graph.nodes;

    let mut direct_in: HashMap<String, Vec<crate::engine::l4::capability_cone::CapabilityFact>> =
        HashMap::new();
    let mut coverage_in: HashMap<String, (String, Vec<String>)> = HashMap::new();
    for id in nodes {
        let Some(ri) = by_id.get(id) else { continue };
        direct_in.insert(id.clone(), (**ri.direct_facts(db)).clone());
        coverage_in.insert(id.clone(), (**ri.direct_coverage(db)).clone());
    }

    let result = crate::engine::l4::capability_cone::compose_cone_over_graph(
        &cg.graph,
        nodes,
        &direct_in,
        &coverage_in,
    );
    ConeValue {
        cones: Arc::new(result),
    }
}

/// `inherited_facts(stable_id)` — one routine's inherited capability facts
/// (internal-id form), pulled from the `cones` query.
#[salsa::tracked]
pub fn inherited_facts<'db>(
    db: &'db dyn L4Db,
    universe: RoutineUniverse,
    registry: RoutineRegistry,
    internal_id: InternalId<'db>,
) -> Vec<crate::engine::l4::capability_cone::CapabilityFact> {
    let c = cones(db, universe, registry);
    c.cones
        .get(internal_id.id(db))
        .map(|r| r.inherited.clone())
        .unwrap_or_default()
}

/// `coverage(stable_id)` — one routine's coverage record (internal-id form).
#[salsa::tracked]
pub fn coverage<'db>(
    db: &'db dyn L4Db,
    universe: RoutineUniverse,
    registry: RoutineRegistry,
    internal_id: InternalId<'db>,
) -> Option<crate::engine::l4::capability_cone::CoverageRecord> {
    let c = cones(db, universe, registry);
    c.cones.get(internal_id.id(db)).map(|r| r.coverage.clone())
}

// ===========================================================================
// Helpers.
// ===========================================================================

/// Project an SCC's members to the SORTED StableRoutineId set (the interned key
/// identity). Mirrors the R3a projection's member sort.
fn stable_members(scc: &Scc, stable_map: &HashMap<String, String>) -> Vec<String> {
    let mut m: Vec<String> = scc
        .members
        .iter()
        .map(|id| stable_map.get(id).cloned().unwrap_or_else(|| id.clone()))
        .collect();
    m.sort();
    m
}

/// Build the field-resolution index from the AppContext tables (shared by the
/// JACOBI parameterRoles + the cone). Exposed for `wrap.rs`.
pub fn field_index_from_ctx(db: &dyn L4Db, ctx: AppContext) -> FieldIndex {
    let mut field_index: FieldIndex = HashMap::new();
    for table in ctx.tables(db).iter() {
        for field in &table.fields {
            field_index
                .entry((table.id.clone(), field.name.to_lowercase()))
                .or_insert_with(|| field.id.clone());
        }
    }
    field_index
}
