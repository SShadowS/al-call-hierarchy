//! The Stage-2 INPUT-EDIT surface (R3b Task 2).
//!
//! Stage 1 built a fresh Salsa DB, demanded the summaries, and threw the DB away.
//! Stage 2 makes the DB *persistent* and *editable*: a held [`EditableModel`]
//! carries the fine-grained input handles ([`RoutineUniverse`] / [`RoutineInput`] /
//! [`AppContext`] / [`DepStamp`]) and exposes a SETTER per editable input. An edit
//! mutates ONE (or a few) inputs; Salsa invalidates the dependent tracked queries
//! transitively, and a re-demand recomputes only the affected reverse cone (the
//! Stage-3 minimality claim; Stage 2 proves only that the re-demanded VALUE is
//! byte-identical to a from-scratch build over the edited inputs).
//!
//! ## The structural input model
//!
//! [`InputModel`] is the owned, serializable picture of every fine-grained input —
//! the routine universe + per-routine [`RoutineFacts`] + the shared [`CtxFacts`] +
//! the dep stamp. It is the SINGLE source both paths build from:
//!   - [`InputModel::build_incremental`] → an [`EditableModel`] holding a live DB +
//!     handles (the reused-DB path).
//!   - [`InputModel::demand_from_scratch`] → a FRESH DB built over the model,
//!     demanded once (the from-scratch oracle).
//!
//! Applying the same structural edit to the [`InputModel`] and to the
//! [`EditableModel`] (via Salsa setters) MUST yield byte-identical demanded output;
//! that is the Stage-2 incremental-equality property.
//!
//! ## Edit semantics — routine universe + id churn (the plan's section)
//!
//! - **set-fact edits** mutate ONE [`RoutineInput`] field (body facts / direct
//!   dbEffects via `base_summary` / direct capability facts / direct coverage /
//!   `body_available`) or push/drop one of its outgoing edges (call edge / typed
//!   edge). The universe is unchanged.
//! - **AddRoutine** = add the id to `routine_ids` + create its [`RoutineInput`] +
//!   register it. Repoint any edge that targeted the (previously dangling) id.
//! - **RemoveRoutine** = drop the id from `routine_ids` (its [`RoutineInput`]
//!   becomes undemanded — Salsa never re-fires a query over it) + drop every edge
//!   that names it as `from`/`to`.
//! - **RenameRoutine** (== signature-rehash; a StableRoutineId re-hash) = remove the
//!   OLD id + add the NEW id carrying the old facts + repoint every edge whose
//!   `from`/`to`/`stable_map` referenced the old id. The SccKey re-interns: the
//!   renamed routine lands in a NEW SccKey (its sorted-StableRoutineId member set
//!   changed), so its SCC's `scc_summaries` recompute — and only that cone.
//! - **no-op-at-L4** = an edit whose demanded output is unchanged (set the same
//!   value; add a dominated/duplicate edge; bump a cosmetic dep stamp). After the
//!   carrier value-equality fix these BACKDATE and do NOT propagate.

use std::collections::HashMap;
use std::sync::Arc;

use super::inputs::{AppContext, DepStamp, RoutineInput, RoutineRegistry, RoutineUniverse};
use super::queries::{cones, scc_for_routine, scc_summaries, InternalId};
use super::{L4Database, L4Db};
use crate::engine::l3::call_resolver::UpgradedBinding;
use crate::engine::l3::event_graph::EventGraph;
use crate::engine::l3::l3_workspace::{L3Object, L3Routine, L3Table};
use crate::engine::l4::capability_cone::{CapabilityFact, ConeResultPub};
use crate::engine::l4::combined_graph::{CombinedEdge, TypedEdge, UncertaintyEdge};
use crate::engine::l4::summary::RoutineSummary;
use salsa::Setter;

/// A per-routine direct-fact provider: `routine → (facts, direct_status, reasons)`.
type DirectFactsFn<'a> = dyn Fn(&L3Routine) -> (Vec<CapabilityFact>, String, Vec<String>) + 'a;

/// The per-routine fine-grained facts — the owned mirror of a [`RoutineInput`]'s
/// field values. The single structural unit an edit mutates.
#[derive(Clone)]
pub struct RoutineFacts {
    pub routine_id: String,
    pub routine: Arc<L3Routine>,
    pub combined_edges: Vec<CombinedEdge>,
    pub typed_edges: Vec<TypedEdge>,
    pub uncertainty_edges: Vec<UncertaintyEdge>,
    pub base_summary: RoutineSummary,
    pub direct_facts: Vec<CapabilityFact>,
    pub direct_coverage: (String, Vec<String>),
    pub body_available: bool,
    pub is_leaf: bool,
}

/// The shared (non-per-routine) context facts — the owned mirror of [`AppContext`].
#[derive(Clone)]
pub struct CtxFacts {
    pub app_identity: String,
    pub objects: Vec<L3Object>,
    pub tables: Vec<L3Table>,
    pub event_graph: EventGraph,
    pub upgraded_bindings: HashMap<String, Vec<UpgradedBinding>>,
    pub stable_map: HashMap<String, String>,
}

/// The whole fine-grained input picture for one L4 run — the SINGLE source both the
/// incremental and the from-scratch paths build from.
#[derive(Clone)]
pub struct InputModel {
    /// Sorted internal RoutineIds (the combined-graph node universe).
    pub routine_ids: Vec<String>,
    /// internal RoutineId → its facts.
    pub routines: HashMap<String, RoutineFacts>,
    pub ctx: CtxFacts,
    pub dep_stamp: String,
}

/// The demanded L4 output (the comparison surface for the proof). Both paths return
/// THIS; the property is `incremental == from_scratch` byte-for-byte.
#[derive(Clone, PartialEq)]
pub struct DemandResult {
    /// internal RoutineId → settled CORE RoutineSummary (sorted by id at compare).
    pub core: HashMap<String, RoutineSummary>,
    /// internal RoutineId → cone (inherited facts + coverage).
    pub cones: HashMap<String, ConeResultPub>,
}

impl DemandResult {
    /// A byte-faithful, order-independent fingerprint of the demanded output (sorts
    /// the maps, serializes deterministically). Two `DemandResult`s with the same
    /// fingerprint are byte-identical at the projection surface.
    pub fn fingerprint(&self) -> String {
        let mut core: Vec<(&String, &RoutineSummary)> = self.core.iter().collect();
        core.sort_by(|a, b| a.0.cmp(b.0));
        let mut cones: Vec<(&String, &ConeResultPub)> = self.cones.iter().collect();
        cones.sort_by(|a, b| a.0.cmp(b.0));
        // Render via Debug — the R3a types derive Debug, and the field order is
        // fixed, so equal values render to equal strings. (The PartialEq impls make
        // the equality authoritative; the string is for diff/diagnostics + the
        // shuffle-order assertions.)
        format!("CORE={core:?}\nCONES={cones:?}")
    }
}

impl InputModel {
    /// Build an [`InputModel`] from the same parts `wrap::build_db` consumes (the
    /// from-scratch L4 base), so the model is the faithful fine-grained picture.
    #[allow(clippy::too_many_arguments)]
    pub fn from_parts(
        routines: &[L3Routine],
        graph: &crate::engine::l4::combined_graph::CombinedGraph,
        objects: &[L3Object],
        tables: &[L3Table],
        event_graph: &EventGraph,
        upgraded_bindings: &HashMap<String, Vec<UpgradedBinding>>,
        app_identity: &str,
        dep_stamp: &str,
        base_summary_for: &dyn Fn(&L3Routine) -> RoutineSummary,
        is_leaf_for: &dyn Fn(&str) -> Option<RoutineSummary>,
        direct_facts_for: &DirectFactsFn,
    ) -> InputModel {
        let mut typed_by_from: HashMap<String, Vec<TypedEdge>> = HashMap::new();
        for te in &graph.typed_edges {
            typed_by_from
                .entry(te.from.clone())
                .or_default()
                .push(te.clone());
        }
        let mut unc_by_from: HashMap<String, Vec<UncertaintyEdge>> = HashMap::new();
        for ue in &graph.uncertainty_edges {
            unc_by_from
                .entry(ue.from.clone())
                .or_default()
                .push(ue.clone());
        }

        let mut routine_ids: Vec<String> = routines.iter().map(|r| r.id.clone()).collect();
        routine_ids.sort();

        let mut routines_map: HashMap<String, RoutineFacts> = HashMap::new();
        let mut stable_map: HashMap<String, String> = HashMap::new();
        for r in routines {
            stable_map.insert(r.id.clone(), r.stable_routine_id.clone());
            let combined_edges = graph.edges_by_from.get(&r.id).cloned().unwrap_or_default();
            let typed_edges = typed_by_from.get(&r.id).cloned().unwrap_or_default();
            let uncertainty_edges = unc_by_from.get(&r.id).cloned().unwrap_or_default();
            let leaf = is_leaf_for(&r.id);
            let is_leaf = leaf.is_some();
            let base_summary = leaf.unwrap_or_else(|| base_summary_for(r));
            let (facts, status, reasons) = direct_facts_for(r);
            routines_map.insert(
                r.id.clone(),
                RoutineFacts {
                    routine_id: r.id.clone(),
                    routine: Arc::new(r.clone()),
                    combined_edges,
                    typed_edges,
                    uncertainty_edges,
                    base_summary,
                    direct_facts: facts,
                    direct_coverage: (status, reasons),
                    body_available: r.body_available,
                    is_leaf,
                },
            );
        }

        InputModel {
            routine_ids,
            routines: routines_map,
            ctx: CtxFacts {
                app_identity: app_identity.to_string(),
                objects: objects.to_vec(),
                tables: tables.to_vec(),
                event_graph: event_graph.clone(),
                upgraded_bindings: upgraded_bindings.clone(),
                stable_map,
            },
            dep_stamp: dep_stamp.to_string(),
        }
    }

    /// Build a FRESH Salsa DB over this model and demand the L4 output once. This is
    /// the FROM-SCRATCH oracle: a clean DB built over the (possibly edited) inputs,
    /// with no reuse — byte-identical to the R3a from-scratch path (Stage-1 parity).
    pub fn demand_from_scratch(&self) -> DemandResult {
        let editable = self.build_incremental();
        editable.demand()
    }

    /// Build a persistent, EDITABLE Salsa DB over this model (the reused-DB path).
    pub fn build_incremental(&self) -> EditableModel {
        self.build_with_db(L4Database::default(), None)
    }

    /// Build an INSTRUMENTED editable DB whose Salsa event callback records every
    /// query recompute into the returned [`super::RecomputeLog`] — the early-cutoff
    /// oracle for the no-op edits.
    pub fn build_incremental_instrumented(&self) -> (EditableModel, super::RecomputeLog) {
        let (db, log) = L4Database::instrumented();
        let em = self.build_with_db(db, Some(log.clone()));
        (em, log)
    }

    /// Build an editable model over the GIVEN database (default or instrumented).
    fn build_with_db(&self, db: L4Database, log: Option<super::RecomputeLog>) -> EditableModel {
        let mut by_id: HashMap<String, RoutineInput> = HashMap::new();
        for (id, f) in &self.routines {
            let ri = RoutineInput::new(
                &db,
                f.routine_id.clone(),
                f.routine.clone(),
                Arc::new(f.combined_edges.clone()),
                Arc::new(f.typed_edges.clone()),
                Arc::new(f.uncertainty_edges.clone()),
                Arc::new(f.base_summary.clone()),
                Arc::new(f.direct_facts.clone()),
                Arc::new(f.direct_coverage.clone()),
                f.body_available,
                f.is_leaf,
            );
            by_id.insert(id.clone(), ri);
        }

        let universe = RoutineUniverse::new(&db, self.routine_ids.clone());
        let registry = RoutineRegistry::new(&db, Arc::new(by_id.clone()));
        let ctx = AppContext::new(
            &db,
            self.ctx.app_identity.clone(),
            Arc::new(self.ctx.objects.clone()),
            Arc::new(self.ctx.tables.clone()),
            Arc::new(self.ctx.event_graph.clone()),
            Arc::new(self.ctx.upgraded_bindings.clone()),
            Arc::new(self.ctx.stable_map.clone()),
        );
        let dep_stamp = DepStamp::new(&db, self.dep_stamp.clone());

        EditableModel {
            db,
            universe,
            registry,
            ctx,
            dep_stamp,
            by_id,
            model: self.clone(),
            log,
        }
    }
}

/// A held, editable Salsa DB + the input handles. Setters mutate the inputs in
/// place; [`EditableModel::demand`] re-demands the (incrementally recomputed) L4
/// output. The mirrored [`InputModel`] is kept in sync so a from-scratch oracle can
/// be rebuilt at any point for the equality assertion.
pub struct EditableModel {
    db: L4Database,
    universe: RoutineUniverse,
    registry: RoutineRegistry,
    ctx: AppContext,
    dep_stamp: DepStamp,
    by_id: HashMap<String, RoutineInput>,
    /// The structural mirror — kept in sync with every edit (the from-scratch oracle
    /// rebuilds from THIS).
    pub model: InputModel,
    /// The recompute log when this DB is instrumented (else `None`).
    log: Option<super::RecomputeLog>,
}

impl EditableModel {
    /// Demand the L4 output through the Salsa query graph (incrementally, on the
    /// reused DB). Byte-identical to [`InputModel::demand_from_scratch`] over the
    /// CURRENT (edited) [`Self::model`].
    pub fn demand(&self) -> DemandResult {
        self.demand_in_order(&self.demand_order())
    }

    /// Clear the recompute log (call AFTER the initial demand, BEFORE applying the
    /// edit, so the log captures ONLY the edit-driven recomputes). No-op when the DB
    /// is not instrumented.
    pub fn clear_log(&self) {
        if let Some(log) = &self.log {
            if let Ok(mut g) = log.lock() {
                g.clear();
            }
        }
    }

    /// Snapshot the recompute log — the query-execution (NON-backdated) names since
    /// the last [`Self::clear_log`].
    pub fn take_log(&self) -> Vec<String> {
        match &self.log {
            Some(log) => log.lock().map(|g| g.clone()).unwrap_or_default(),
            None => Vec::new(),
        }
    }

    /// The default demand order (sorted ids — deterministic).
    fn demand_order(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.model.routine_ids.clone();
        ids.sort();
        ids
    }

    /// Demand the L4 output, visiting routines in the GIVEN order (the
    /// nondeterminism oracle: the demanded VALUE must be order-invariant). Salsa
    /// memoizes, so each SCC/cone computes once regardless of visitation order.
    pub fn demand_in_order(&self, order: &[String]) -> DemandResult {
        let db: &dyn L4Db = &self.db;

        // CORE summaries — demand each routine's SccKey (drives the inter-SCC chain).
        let mut core: HashMap<String, RoutineSummary> = HashMap::new();
        for id in order {
            let iid = InternalId::new(db, id.clone());
            if let Some(key) = scc_for_routine(db, self.universe, self.registry, self.ctx, iid) {
                let s = scc_summaries(db, self.universe, self.registry, self.ctx, key);
                for (rid, summary) in s.summaries.iter() {
                    core.entry(rid.clone()).or_insert_with(|| summary.clone());
                }
            }
        }
        // Undemanded leaves still carry their retained summary.
        for id in order {
            if let Some(ri) = self.by_id.get(id) {
                if ri.is_leaf(db) {
                    core.entry(id.clone())
                        .or_insert_with(|| (**ri.base_summary(db)).clone());
                }
            }
        }

        // The cone (single query over the universe).
        let c = cones(db, self.universe, self.registry);
        let cones_map = (*c.cones).clone();

        DemandResult {
            core,
            cones: cones_map,
        }
    }

    // -----------------------------------------------------------------------
    // SET-FACT edits — mutate ONE RoutineInput field. The universe is unchanged.
    // -----------------------------------------------------------------------

    /// Set a routine's body facts (the resolved `L3Routine`).
    pub fn set_routine_body(&mut self, id: &str, routine: Arc<L3Routine>) {
        let Some(ri) = self.by_id.get(id).copied() else {
            return;
        };
        ri.set_routine(&mut self.db).to(routine.clone());
        if let Some(f) = self.model.routines.get_mut(id) {
            f.routine = routine;
        }
    }

    /// Set a routine's DIRECT dbEffects (carried by its base summary).
    pub fn set_base_summary(&mut self, id: &str, base: RoutineSummary) {
        let Some(ri) = self.by_id.get(id).copied() else {
            return;
        };
        ri.set_base_summary(&mut self.db).to(Arc::new(base.clone()));
        if let Some(f) = self.model.routines.get_mut(id) {
            f.base_summary = base;
        }
    }

    /// Set a routine's DIRECT capability facts.
    pub fn set_direct_facts(&mut self, id: &str, facts: Vec<CapabilityFact>) {
        let Some(ri) = self.by_id.get(id).copied() else {
            return;
        };
        ri.set_direct_facts(&mut self.db)
            .to(Arc::new(facts.clone()));
        if let Some(f) = self.model.routines.get_mut(id) {
            f.direct_facts = facts;
        }
    }

    /// Set a routine's DIRECT coverage `(status, reasons)`.
    pub fn set_direct_coverage(&mut self, id: &str, coverage: (String, Vec<String>)) {
        let Some(ri) = self.by_id.get(id).copied() else {
            return;
        };
        ri.set_direct_coverage(&mut self.db)
            .to(Arc::new(coverage.clone()));
        if let Some(f) = self.model.routines.get_mut(id) {
            f.direct_coverage = coverage;
        }
    }

    /// Set a routine's `body_available` flag.
    pub fn set_body_available(&mut self, id: &str, body_available: bool) {
        let Some(ri) = self.by_id.get(id).copied() else {
            return;
        };
        ri.set_body_available(&mut self.db).to(body_available);
        if let Some(f) = self.model.routines.get_mut(id) {
            f.body_available = body_available;
        }
    }

    /// Set a routine's WHOLE outgoing combined-edge slice (a call-edge add/remove is
    /// expressed as a new slice; the slice stays edgeSortKey-sorted).
    pub fn set_combined_edges(&mut self, id: &str, mut edges: Vec<CombinedEdge>) {
        edges.sort_by_key(crate::engine::l4::combined_graph::edge_sort_key);
        let Some(ri) = self.by_id.get(id).copied() else {
            return;
        };
        ri.set_combined_edges(&mut self.db)
            .to(Arc::new(edges.clone()));
        if let Some(f) = self.model.routines.get_mut(id) {
            f.combined_edges = edges;
        }
    }

    /// Set a routine's WHOLE outgoing typed-edge slice (the cone substrate).
    pub fn set_typed_edges(&mut self, id: &str, edges: Vec<TypedEdge>) {
        let Some(ri) = self.by_id.get(id).copied() else {
            return;
        };
        ri.set_typed_edges(&mut self.db).to(Arc::new(edges.clone()));
        if let Some(f) = self.model.routines.get_mut(id) {
            f.typed_edges = edges;
        }
    }

    // -----------------------------------------------------------------------
    // CTX edits.
    // -----------------------------------------------------------------------

    /// Set the app identity.
    pub fn set_app_identity(&mut self, identity: &str) {
        self.ctx
            .set_app_identity(&mut self.db)
            .to(identity.to_string());
        self.model.ctx.app_identity = identity.to_string();
    }

    /// Bump the dep-artifact stamp (the cross-app invalidation key).
    pub fn set_dep_stamp(&mut self, stamp: &str) {
        self.dep_stamp.set_stamp(&mut self.db).to(stamp.to_string());
        self.model.dep_stamp = stamp.to_string();
    }

    // -----------------------------------------------------------------------
    // ID CHURN — add / remove / rename a routine.
    // -----------------------------------------------------------------------

    /// ADD a routine: add its id to `routine_ids`, create its [`RoutineInput`],
    /// register it, and extend the stable_map.
    pub fn add_routine(&mut self, facts: RoutineFacts) {
        let id = facts.routine_id.clone();
        let stable = self
            .model
            .ctx
            .stable_map
            .get(&id)
            .cloned()
            .unwrap_or_else(|| facts.routine.stable_routine_id.clone());

        let ri = RoutineInput::new(
            &self.db,
            facts.routine_id.clone(),
            facts.routine.clone(),
            Arc::new(facts.combined_edges.clone()),
            Arc::new(facts.typed_edges.clone()),
            Arc::new(facts.uncertainty_edges.clone()),
            Arc::new(facts.base_summary.clone()),
            Arc::new(facts.direct_facts.clone()),
            Arc::new(facts.direct_coverage.clone()),
            facts.body_available,
            facts.is_leaf,
        );
        self.by_id.insert(id.clone(), ri);
        self.model.routines.insert(id.clone(), facts);
        if !self.model.routine_ids.contains(&id) {
            self.model.routine_ids.push(id.clone());
            self.model.routine_ids.sort();
        }
        self.model.ctx.stable_map.insert(id.clone(), stable);

        self.repush_universe_and_registry();
    }

    /// REMOVE a routine: drop its id from `routine_ids` (its input becomes
    /// undemanded) and drop every edge that names it as `from`/`to`.
    pub fn remove_routine(&mut self, id: &str) {
        self.model.routine_ids.retain(|x| x != id);
        self.model.routines.remove(id);
        self.by_id.remove(id);
        self.model.ctx.stable_map.remove(id);
        self.drop_edges_referencing(id);
        self.repush_universe_and_registry();
    }

    /// RENAME (== signature-rehash): remove the OLD id, add the NEW id carrying the
    /// old facts, and repoint every edge whose `from`/`to`/stable_map referenced the
    /// old id. The StableRoutineId re-hashes (a new stable id), so the renamed
    /// routine re-interns into a NEW SccKey.
    pub fn rename_routine(&mut self, old_id: &str, new_id: &str, new_stable_id: &str) {
        let Some(mut facts) = self.model.routines.remove(old_id) else {
            return;
        };
        facts.routine_id = new_id.to_string();
        // The body's stable id re-hashes too (signature change).
        let mut routine = (*facts.routine).clone();
        routine.id = new_id.to_string();
        routine.stable_routine_id = new_stable_id.to_string();
        facts.routine = Arc::new(routine);
        facts.base_summary.routine_id = new_id.to_string();

        // Remove the old input/id.
        self.by_id.remove(old_id);
        self.model.routine_ids.retain(|x| x != old_id);
        self.model.ctx.stable_map.remove(old_id);

        // Repoint edges everywhere (from/to) old_id → new_id.
        self.repoint_edges(old_id, new_id);
        // Repoint this routine's own outgoing edges' `from`.
        for e in &mut facts.combined_edges {
            if e.from == old_id {
                e.from = new_id.to_string();
            }
            if e.to == old_id {
                e.to = new_id.to_string();
            }
        }
        for e in &mut facts.typed_edges {
            if e.from == old_id {
                e.from = new_id.to_string();
            }
            if e.to.as_deref() == Some(old_id) {
                e.to = Some(new_id.to_string());
            }
        }
        for ue in &mut facts.uncertainty_edges {
            if ue.from == old_id {
                ue.from = new_id.to_string();
            }
        }

        // Add the new id carrying the (repointed) facts + the re-hashed stable id.
        self.model.ctx.stable_map.remove(new_id);
        self.add_routine(facts);
        self.model
            .ctx
            .stable_map
            .insert(new_id.to_string(), new_stable_id.to_string());
        self.repush_universe_and_registry();
    }

    // -----------------------------------------------------------------------
    // Internal: re-push the universe + registry + ctx after id churn.
    // -----------------------------------------------------------------------

    /// After id churn the universe + registry + stable_map changed — re-push them as
    /// whole-value input sets (Salsa invalidates the structural queries; the per-SCC
    /// projections early-cut for untouched SCCs).
    fn repush_universe_and_registry(&mut self) {
        self.model.routine_ids.sort();
        self.universe
            .set_routine_ids(&mut self.db)
            .to(self.model.routine_ids.clone());
        self.registry
            .set_by_id(&mut self.db)
            .to(Arc::new(self.by_id.clone()));
        self.ctx
            .set_stable_map(&mut self.db)
            .to(Arc::new(self.model.ctx.stable_map.clone()));
    }

    /// Drop every edge naming `id` as `from`/`to` (REMOVE semantics).
    fn drop_edges_referencing(&mut self, id: &str) {
        for f in self.model.routines.values_mut() {
            f.combined_edges.retain(|e| e.from != id && e.to != id);
            f.typed_edges
                .retain(|e| e.from != id && e.to.as_deref() != Some(id));
            f.uncertainty_edges.retain(|ue| ue.from != id);
        }
        // Re-push each mutated routine's edge inputs.
        let ids: Vec<String> = self.model.routines.keys().cloned().collect();
        for rid in ids {
            self.push_routine_edges(&rid);
        }
    }

    /// Repoint every edge `from`/`to` old → new (RENAME semantics) across all OTHER
    /// routines, then re-push their edge inputs.
    fn repoint_edges(&mut self, old: &str, new: &str) {
        for f in self.model.routines.values_mut() {
            for e in &mut f.combined_edges {
                if e.from == old {
                    e.from = new.to_string();
                }
                if e.to == old {
                    e.to = new.to_string();
                }
            }
            for e in &mut f.typed_edges {
                if e.from == old {
                    e.from = new.to_string();
                }
                if e.to.as_deref() == Some(old) {
                    e.to = Some(new.to_string());
                }
            }
            for ue in &mut f.uncertainty_edges {
                if ue.from == old {
                    ue.from = new.to_string();
                }
            }
        }
        let ids: Vec<String> = self.model.routines.keys().cloned().collect();
        for rid in ids {
            self.push_routine_edges(&rid);
        }
    }

    /// Re-push one routine's edge inputs from the model (after a churn rewrite).
    fn push_routine_edges(&mut self, id: &str) {
        let Some(ri) = self.by_id.get(id).copied() else {
            return;
        };
        let Some(f) = self.model.routines.get(id) else {
            return;
        };
        ri.set_combined_edges(&mut self.db)
            .to(Arc::new(f.combined_edges.clone()));
        ri.set_typed_edges(&mut self.db)
            .to(Arc::new(f.typed_edges.clone()));
        ri.set_uncertainty_edges(&mut self.db)
            .to(Arc::new(f.uncertainty_edges.clone()));
    }
}
