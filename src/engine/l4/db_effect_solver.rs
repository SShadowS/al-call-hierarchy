//! L4 summary-fixpoint redesign (Phase 1) — the interned-bitvector db-effect solver.
//!
//! Step 0: effective-SCC re-decomposition. `run_one_scc` (the old Jacobi solver, see
//! `summary_runner.rs`) excludes fixed leaves AND routines missing from
//! `routines_by_id` when it builds a Tarjan SCC's per-member equation graph. Removing
//! those nodes from a strongly-connected component's induced subgraph can SPLIT one
//! cycle into several DAG-shaped pieces — e.g. `a -> b -> c -> a` with `b` excluded
//! degrades to `c -> a`, no cycle at all. The old solver never re-ran Tarjan on that
//! induced subgraph; the new solver does, via [`effective_sccs`].
//!
//! Step A: PD product-graph reachability ([`solve_pd_reachability`]). The ONLY
//! genuinely per-member db-effect flow is `ParameterDependent` substitution —
//! `Known`/`Unknown` effects transfer by identity and are handled by the later
//! closed-form union task (Task 5). Step A solves PD substitution as semi-naive
//! reachability over `(base_effect_id, routine, PD(param_index))` product nodes,
//! reusing `summary_runner::substitute_pd_temp_state` (the SAME per-edge
//! transition `compose_routine`'s JACOBI fold already uses) so both solvers agree
//! on PD semantics by construction, not by re-derivation.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::engine::l3::call_resolver::UpgradedBinding;
use crate::engine::l3::l3_workspace::L3Routine;
use crate::engine::l4::combined_graph::{CombinedEdge, CombinedGraph};
use crate::engine::l4::effect_lattice::TempStateKind;
use crate::engine::l4::effect_universe::{EffectId, EffectIdentity, EffectUniverse};
use crate::engine::l4::scc::{Scc, SccInputGraph, tarjan_scc};
use crate::engine::l4::summary::{DbEffect, RoutineSummary, TempState};
use crate::engine::l4::summary_runner::{
    FieldIndex, base_intraprocedural_summary, substitute_pd_temp_state,
};

/// Re-decompose one Tarjan `Scc` into its *effective* SCCs: the strongly-connected
/// components of the subgraph induced by keeping only members for which
/// `is_recomputed` is true (neither a fixed leaf nor a routine missing from
/// `routines_by_id`) and only edges between two such members.
///
/// A member excluded by `is_recomputed` contributes no node and no edge to the
/// induced subgraph — its outgoing/incoming edges are external inputs to whichever
/// effective SCC touches it, not intra-component dependencies, so the caller must
/// account for them separately (fixed-leaf substitution, not re-decomposition).
///
/// Returned in reverse-topological order (callees first), matching `tarjan_scc`'s
/// own contract — `effective_sccs` re-runs `tarjan_scc` and returns its `.sccs`
/// verbatim, so callers can fold over the result exactly like a normal SCC list.
pub fn effective_sccs(
    scc_entry: &Scc,
    graph: &CombinedGraph,
    is_recomputed: &dyn Fn(&str) -> bool,
) -> Vec<Scc> {
    // 1. Filter to recomputed members only. Keep `scc_entry.members`' own order (it
    //    is already sorted — `tarjan_scc` sorts every member list — so the induced
    //    node list is sorted too, matching `SccInputGraph::nodes`'s deterministic-DFS-
    //    roots contract).
    let nodes: Vec<String> = scc_entry
        .members
        .iter()
        .filter(|m| is_recomputed(m))
        .cloned()
        .collect();

    if nodes.is_empty() {
        return Vec::new();
    }

    let recomputed: std::collections::HashSet<&str> = nodes.iter().map(|s| s.as_str()).collect();

    // 2. Project edges: for each recomputed member, keep only `to`s that are ALSO
    //    recomputed members of THIS Scc. Edges to fixed leaves, missing routines, or
    //    nodes outside this Scc entirely are dropped — they are external inputs, not
    //    intra-component dependencies.
    let mut edges_by_from: HashMap<String, Vec<String>> = HashMap::new();
    for m in &nodes {
        let tos: Vec<String> = graph
            .edges_by_from
            .get(m)
            .into_iter()
            .flatten()
            .map(|e| e.to.as_str())
            .filter(|to| recomputed.contains(to))
            .map(|to| to.to_string())
            .collect();
        edges_by_from.insert(m.clone(), tos);
    }

    // 3. Re-run Tarjan on the induced subgraph and hand back its SCCs verbatim —
    //    already reverse-topological, already deterministic-sorted per component.
    let input = SccInputGraph {
        nodes: &nodes,
        edges_by_from: &edges_by_from,
    };
    tarjan_scc(&input).sccs
}

// ---------------------------------------------------------------------------
// Step A: PD product-graph reachability.
// ---------------------------------------------------------------------------

/// A terminal (`Known`/`Unknown`) db-effect discovered while solving `PD`
/// substitution for one effective SCC. Keyed by the member routine it landed
/// on plus the BASE effect identity — `(op, table_id, operation_id)`, the same
/// via-free/temp-state-free triple `effect_key_of` folds in as its first three
/// fragments.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TerminalEmission {
    pub routine_id: String,
    /// `(op, table_id, operation_id)`.
    pub base: (String, String, String),
    /// `Known(_)` or `Unknown` only — a `ParameterDependent` outcome never
    /// reaches here (it inserts a new product node instead; see
    /// [`solve_pd_reachability`]).
    pub temp: TempStateKind,
}

/// A `ParameterDependent` fact retained on a member: the base effect identity,
/// carried at THIS member's own caller-frame parameter index — reached after
/// zero or more re-symbolizing substitution hops from wherever the base effect
/// originated (a member's own record op, or an external successor's PD effect
/// substituted at this member's out-edge).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PdFact {
    pub routine_id: String,
    /// `(op, table_id, operation_id)`.
    pub base: (String, String, String),
    pub param_index: u32,
}

/// One product-graph state: `(routine, base identity, param_index)` — a
/// `PdFact` pre-materialization. Plain tuple internally so the worklist/visited
/// set never allocates a `PdFact` just to check membership.
type PdState = (String, (String, String, String), u32);

/// Apply one `substitute_pd_temp_state` outcome to the worklist: a
/// `ParameterDependent(j)` result inserts (and, if unseen, enqueues) a new
/// state at `at_routine`; a `Known`/`Unknown` result emits a [`TerminalEmission`]
/// and stops (terminal states transfer by identity thereafter — Task 5).
fn apply_pd_transition(
    at_routine: &str,
    base: &(String, String, String),
    outcome: TempState,
    visited: &mut HashSet<PdState>,
    worklist: &mut VecDeque<PdState>,
    terminals: &mut HashSet<TerminalEmission>,
) {
    match outcome {
        TempState::ParameterDependent(j) => {
            let state: PdState = (at_routine.to_string(), base.clone(), j);
            if visited.insert(state.clone()) {
                worklist.push_back(state);
            }
        }
        TempState::Known(v) => {
            terminals.insert(TerminalEmission {
                routine_id: at_routine.to_string(),
                base: base.clone(),
                temp: TempStateKind::Known(v),
            });
        }
        TempState::Unknown => {
            terminals.insert(TerminalEmission {
                routine_id: at_routine.to_string(),
                base: base.clone(),
                temp: TempStateKind::Unknown,
            });
        }
    }
}

/// Solve `ParameterDependent` substitution as semi-naive reachability over
/// `(base_effect_id, routine, PD(param_index))` product nodes, for ONE
/// effective SCC (`eff` — see [`effective_sccs`]).
///
/// Seeds:
///   1. Each member's OWN base (intraprocedural) PD effects — a record op on
///      the member itself whose temp-state is `ParameterDependent(i)`.
///   2. Edge-substituted images of external-callee PD effects at each
///      member's OUT-edges: an edge to a routine NOT in `eff` is resolved
///      through `settled` (the predecessor final map — already-processed
///      successor SCCs AND fixed leaves both live there, exactly like
///      `compose_routine`'s `lookup` falling through to `final_map`), and any
///      `ParameterDependent` effect on that settled summary is substituted
///      through the edge immediately (it can only be seeded once — the
///      external callee's summary never changes while this SCC is solved).
///
/// Transition: popping a discovered state `(base, w, i)`, for every
/// intra-effective-SCC edge `caller -> w` (caller ALSO a member of `eff`),
/// `substitute_pd_temp_state` is applied using the CALLER's own binding for
/// callee-param `i`. A `ParameterDependent(j)` result inserts the new state
/// `(base, caller, j)` (continuing the worklist if unseen); a `Known`/
/// `Unknown` result emits a [`TerminalEmission`] and stops.
///
/// The visited set is bounded by the OBSERVED `(base, routine, param_index)`
/// triples actually reached — never by an assumed `param_index <= #params`
/// (malformed/recovered input can carry an arbitrary `u32`), so a PD chasing
/// itself around a cycle re-hits an already-visited state and the worklist
/// terminates (the same monotone-fixed-point argument
/// `substitute_pd_temp_state`'s own doc makes for the JACOBI fold).
///
/// `_upgraded_bindings` is accepted (not read): `substitute_pd_temp_state`
/// resolves purely from `PCallArgumentBinding.source_temp_state` on the
/// caller's own call site, never consulting the upgraded-bindings side table
/// (that table only feeds the SEPARATE cross-call `parameter_roles`
/// composition in `compose_routine`) — kept in the signature for parity with
/// the plan's other Step functions / Task 8's uniform per-SCC ctx wiring.
pub fn solve_pd_reachability(
    eff: &Scc,
    graph: &CombinedGraph,
    routines_by_id: &HashMap<String, &L3Routine>,
    settled: &HashMap<String, RoutineSummary>,
    _upgraded_bindings: &HashMap<String, Vec<UpgradedBinding>>,
) -> (Vec<PdFact>, Vec<TerminalEmission>) {
    let member_set: HashSet<&str> = eff.members.iter().map(|s| s.as_str()).collect();

    // Intra-effective-SCC caller edges, indexed by CALLEE: for member `w`, the
    // `(caller, edge)` pairs where `caller` is ALSO an effective-SCC member and
    // `caller -> w` is a real combined-graph edge. Multiple edges for the same
    // (caller, w) pair (distinct callsites) are each kept independently — they
    // can carry different bindings and so substitute differently (the
    // multi-callsite-same-callee shape `compose_routine` already handles).
    let mut callers_by_callee: HashMap<&str, Vec<(&str, &CombinedEdge)>> = HashMap::new();
    for v in &eff.members {
        for e in graph.edges_by_from.get(v).into_iter().flatten() {
            if member_set.contains(e.to.as_str()) {
                callers_by_callee
                    .entry(e.to.as_str())
                    .or_default()
                    .push((v.as_str(), e));
            }
        }
    }

    let mut visited: HashSet<PdState> = HashSet::new();
    let mut worklist: VecDeque<PdState> = VecDeque::new();
    let mut terminals: HashSet<TerminalEmission> = HashSet::new();

    // Seed 1: each member's OWN base PD effects. `fields` never affects
    // `db_effects` (only `parameter_roles` reads it), so deriving the base
    // summary here with an EMPTY `FieldIndex` reproduces byte-identical
    // `db_effects` to whatever the real workspace-wide `base_summaries` map
    // would carry for this member — no separate derivation logic to drift.
    let empty_fields = FieldIndex::new();
    for m in &eff.members {
        let Some(routine) = routines_by_id.get(m) else {
            continue; // effective_sccs already excludes missing members; defensive only.
        };
        let base = base_intraprocedural_summary(routine, routines_by_id, &empty_fields);
        for e in &base.db_effects {
            if let TempState::ParameterDependent(i) = &e.temp_state {
                let base_id = (e.op.clone(), e.table_id.clone(), e.operation_id.clone());
                let state: PdState = (m.clone(), base_id, *i);
                if visited.insert(state.clone()) {
                    worklist.push_back(state);
                }
            }
        }
    }

    // Seed 2: edge-substituted images of external-callee (already-settled
    // successor / fixed-leaf) PD effects at each member's OUT-edges.
    for m in &eff.members {
        let Some(caller_routine) = routines_by_id.get(m) else {
            continue;
        };
        for e in graph.edges_by_from.get(m).into_iter().flatten() {
            if member_set.contains(e.to.as_str()) {
                continue; // intra-effective-SCC; handled by the worklist below.
            }
            let Some(callee_summary) = settled.get(&e.to) else {
                continue; // unresolved / not (yet) settled — no PD image to seed.
            };
            for callee_effect in &callee_summary.db_effects {
                if let TempState::ParameterDependent(j) = &callee_effect.temp_state {
                    let base_id = (
                        callee_effect.op.clone(),
                        callee_effect.table_id.clone(),
                        callee_effect.operation_id.clone(),
                    );
                    let outcome = substitute_pd_temp_state(e, *j, caller_routine);
                    apply_pd_transition(
                        m,
                        &base_id,
                        outcome,
                        &mut visited,
                        &mut worklist,
                        &mut terminals,
                    );
                }
            }
        }
    }

    // Semi-naive worklist: pop a discovered state; for every intra-effective-
    // SCC caller edge INTO its routine, substitute through the CALLER's own
    // binding for the callee param this state's index refers to.
    while let Some((w, base_id, idx)) = worklist.pop_front() {
        if let Some(callers) = callers_by_callee.get(w.as_str()) {
            for (v, edge) in callers {
                let Some(caller_routine) = routines_by_id.get(*v) else {
                    continue;
                };
                let outcome = substitute_pd_temp_state(edge, idx, caller_routine);
                apply_pd_transition(
                    v,
                    &base_id,
                    outcome,
                    &mut visited,
                    &mut worklist,
                    &mut terminals,
                );
            }
        }
    }

    // Materialize + sort for determinism. `PdFact`'s derived `Ord` compares
    // fields in declaration order — (routine_id, base, param_index) — exactly
    // the requested sort key, so a plain `.sort()` suffices.
    let mut pd_facts: Vec<PdFact> = visited
        .into_iter()
        .map(|(routine_id, base, param_index)| PdFact {
            routine_id,
            base,
            param_index,
        })
        .collect();
    pd_facts.sort();

    // `TerminalEmission` can't derive `Ord` (`TempStateKind` doesn't), so sort
    // explicitly by (routine_id, base, temp's key fragment).
    let mut terminal_emissions: Vec<TerminalEmission> = terminals.into_iter().collect();
    terminal_emissions.sort_by(|a, b| {
        (&a.routine_id, &a.base, a.temp.key_fragment()).cmp(&(
            &b.routine_id,
            &b.base,
            b.temp.key_fragment(),
        ))
    });

    (pd_facts, terminal_emissions)
}

// ---------------------------------------------------------------------------
// Steps B/C: closed-form terminal union + per-member presence assembly.
// ---------------------------------------------------------------------------

/// OR one interned [`EffectId`] into a presence bitset, growing the backing
/// `Vec<u64>` if the id's word isn't yet allocated. Bit `n % 64` of word
/// `n / 64` for `EffectId(n)` — the layout the Task 5 brief specifies, shared
/// by every later task (`reconstruct_via`, `solve_scc_db_effects`) that reads
/// or writes a [`SccPresence`] bitset.
pub(crate) fn set_bit(bits: &mut Vec<u64>, id: EffectId) {
    let word = (id.0 / 64) as usize;
    if bits.len() <= word {
        bits.resize(word + 1, 0);
    }
    bits[word] |= 1u64 << (id.0 % 64);
}

/// True iff `id`'s bit is set in `bits`. An id past the end of `bits` is
/// simply absent (never grows `bits` — read-only). Not yet read by production
/// code (Task 5 only WRITES `SccPresence`; `reconstruct_via`/
/// `solve_scc_db_effects` — Tasks 6/8 — are the intended readers) — kept
/// `pub(crate)` and exercised directly by this task's own tests.
#[allow(dead_code)]
pub(crate) fn has_bit(bits: &[u64], id: EffectId) -> bool {
    let word = (id.0 / 64) as usize;
    word < bits.len() && (bits[word] & (1u64 << (id.0 % 64))) != 0
}

/// Per-member db-effect PRESENCE sets for one effective SCC — a bitset over
/// the shared [`EffectUniverse`], indexed by [`EffectId`]`.0` (see [`set_bit`]).
pub struct SccPresence {
    pub by_member: HashMap<String, Vec<u64>>,
}

/// Intern a TERMINAL (`Known`/`Unknown` only) [`DbEffect`](crate::engine::l4::summary::DbEffect)
/// and OR its id into `bits`. A `ParameterDependent` effect is silently
/// skipped — Step A already accounts for it (as a retained [`PdFact`] or a
/// [`TerminalEmission`]); folding it here under its UNSUBSTITUTED index would
/// double-count under the wrong identity.
fn intern_terminal_db_effect(
    universe: &mut EffectUniverse,
    bits: &mut Vec<u64>,
    op: &str,
    table_id: &str,
    operation_id: &str,
    temp_state: &TempState,
) {
    let temp = match temp_state {
        TempState::Known(v) => TempStateKind::Known(*v),
        TempState::Unknown => TempStateKind::Unknown,
        TempState::ParameterDependent(_) => return,
    };
    let identity = EffectIdentity {
        op: op.to_string(),
        table_id: table_id.to_string(),
        operation_id: operation_id.to_string(),
        temp,
    };
    let id = universe.intern(&identity);
    set_bit(bits, id);
}

/// Compute per-member db-effect PRESENCE sets for one effective SCC (`eff`):
///
/// ```text
/// C =  union(member terminal (Known/Unknown) base effects)
///    ∪ union(ACTUAL outgoing-edge successor terminal sets)   // successors in `settled`
///    ∪ union(terminal emissions from Step A / solve_pd_reachability)
/// effects[v] = C ∪ (member-v PD facts from Step A)   // PD facts are per-member, NOT shared
/// ```
///
/// The exploit: within a strongly-connected effective SCC every member
/// reaches every other, so the TERMINAL (identity-transferred) union `C` is
/// IDENTICAL for every member — computed ONCE, over the whole membership, as
/// a single bitset — rather than folded edge-by-edge per member the way the
/// old JACOBI `compose_routine` fold does it. Only the `ParameterDependent`
/// facts differ per member (Step A already computed them per-member, because
/// PD re-symbolizes along the SPECIFIC chain of caller bindings reaching that
/// member — it does not transfer by identity).
///
/// `C`'s three sources (all TERMINAL — `Known`/`Unknown` — effects; a `PD`
/// effect is skipped wherever it's seen, whether in a member's own base
/// summary or a settled successor's, because Step A already turned every PD
/// outcome into either a per-member [`PdFact`] or a shared [`TerminalEmission`]):
///   1. Each member's OWN base ([`base_summaries`], falling back to
///      [`base_intraprocedural_summary`] if a member is missing from it —
///      mirrors Step A's own Seed 1 fallback).
///   2. TERMINAL effects of successors reached over an ACTUAL out-edge
///      (`graph.edges_by_from[member]`) from ANY member of `eff`, where the
///      successor is already `settled` (a previously-processed effective SCC
///      or a fixed leaf) — an edge to another member of `eff` itself is
///      intra-component, not yet settled, and is skipped here (source 1 +
///      Step A already cover it via the strong-connectivity closure).
///   3. Every `terminal_emissions` entry from Step A, regardless of which
///      member it landed on — by the same strong-connectivity argument, once
///      any member's PD chain resolves to a terminal value the whole SCC
///      shares it.
#[allow(clippy::too_many_arguments)]
pub fn closed_form_union(
    eff: &Scc,
    graph: &CombinedGraph,
    routines_by_id: &HashMap<String, &L3Routine>,
    settled: &HashMap<String, RoutineSummary>,
    base_summaries: &HashMap<String, RoutineSummary>,
    pd_facts: &[PdFact],
    terminal_emissions: &[TerminalEmission],
    universe: &mut EffectUniverse,
) -> SccPresence {
    let member_set: HashSet<&str> = eff.members.iter().map(|s| s.as_str()).collect();
    let empty_fields = FieldIndex::new();

    let mut c: Vec<u64> = Vec::new();

    // Source 1: each member's own base terminal effects. `computed_base`
    // only gets initialized (and only lives) on the fallback path — the
    // common case (`base_summaries` has the member) borrows straight from
    // the map, never cloning a whole `RoutineSummary` just to read its
    // `db_effects`.
    for m in &eff.members {
        let computed_base;
        let db_effects: &[DbEffect] = if let Some(b) = base_summaries.get(m) {
            &b.db_effects
        } else if let Some(r) = routines_by_id.get(m.as_str()) {
            computed_base = base_intraprocedural_summary(r, routines_by_id, &empty_fields);
            &computed_base.db_effects
        } else {
            continue; // effective_sccs already excludes missing members; defensive only.
        };
        for e in db_effects {
            intern_terminal_db_effect(
                universe,
                &mut c,
                &e.op,
                &e.table_id,
                &e.operation_id,
                &e.temp_state,
            );
        }
    }

    // Source 2: terminal effects of settled successors, reached over an
    // actual out-edge from any member of `eff`.
    for m in &eff.members {
        for e in graph.edges_by_from.get(m).into_iter().flatten() {
            if member_set.contains(e.to.as_str()) {
                continue; // intra-effective-SCC; not settled yet.
            }
            let Some(callee) = settled.get(&e.to) else {
                continue; // unresolved / not (yet) settled.
            };
            for effect in &callee.db_effects {
                intern_terminal_db_effect(
                    universe,
                    &mut c,
                    &effect.op,
                    &effect.table_id,
                    &effect.operation_id,
                    &effect.temp_state,
                );
            }
        }
    }

    // Source 3: Step A's terminal emissions, shared across the whole SCC.
    for t in terminal_emissions {
        let identity = EffectIdentity {
            op: t.base.0.clone(),
            table_id: t.base.1.clone(),
            operation_id: t.base.2.clone(),
            temp: t.temp.clone(),
        };
        let id = universe.intern(&identity);
        set_bit(&mut c, id);
    }

    // effects[v] = C ∪ member-v's own retained PD facts (NOT shared).
    let mut by_member: HashMap<String, Vec<u64>> = HashMap::with_capacity(eff.members.len());
    for m in &eff.members {
        let mut bits = c.clone();
        for f in pd_facts.iter().filter(|f| &f.routine_id == m) {
            let identity = EffectIdentity {
                op: f.base.0.clone(),
                table_id: f.base.1.clone(),
                operation_id: f.base.2.clone(),
                temp: TempStateKind::ParameterDependent(f.param_index),
            };
            let id = universe.intern(&identity);
            set_bit(&mut bits, id);
        }
        by_member.insert(m.clone(), bits);
    }

    SccPresence { by_member }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::l4::combined_graph::CombinedEdge;

    /// Build a minimal `CombinedGraph` for SCC-decomposition tests: every edge is a
    /// plain "direct" call with a synthetic callsite id, no other L4 machinery
    /// (uncertainty/typed edges, event dispatch) attached.
    fn build_cycle_graph(nodes: &[&str], edges: &[(&str, &str)]) -> CombinedGraph {
        let mut sorted_nodes: Vec<String> = nodes.iter().map(|n| n.to_string()).collect();
        sorted_nodes.sort();

        let mut edges_by_from: HashMap<String, Vec<CombinedEdge>> = HashMap::new();
        let mut edges_from_order: Vec<String> = Vec::new();
        for (from, to) in edges {
            let from = from.to_string();
            let to = to.to_string();
            if !edges_by_from.contains_key(&from) {
                edges_from_order.push(from.clone());
            }
            edges_by_from
                .entry(from.clone())
                .or_default()
                .push(CombinedEdge {
                    from,
                    to,
                    kind: "direct".to_string(),
                    callsite_id: Some("cs".to_string()),
                    operation_id: None,
                    event_id: None,
                    subscriber_app_id: None,
                    resolution: "resolved".to_string(),
                });
        }

        CombinedGraph {
            nodes: sorted_nodes,
            edges_by_from,
            edges_from_order,
            uncertainty_edges: Vec::new(),
            typed_edges: Vec::new(),
        }
    }

    #[test]
    fn fixed_leaf_splits_cycle_into_dag_parts() {
        // Tarjan SCC {a,b,c} with edges a->b->c->a. Mark `b` as NOT recomputed (fixed leaf).
        // Induced graph over {a,c}: a-> (b removed), c->a  => edges: c->a only. No cycle.
        let graph = build_cycle_graph(&["a", "b", "c"], &[("a", "b"), ("b", "c"), ("c", "a")]);
        let scc = Scc {
            members: vec!["a".into(), "b".into(), "c".into()],
            recursive: true,
        };
        let eff = effective_sccs(&scc, &graph, &|id| id != "b");
        // b excluded; a and c are now acyclic (c->a). Two non-recursive singletons.
        let members: Vec<Vec<String>> = eff.iter().map(|s| s.members.clone()).collect();
        assert_eq!(eff.len(), 2, "leaf removal splits the cycle");
        assert!(eff.iter().all(|s| !s.recursive));
        // reverse-topo (callees before callers): c calls a, so a (the callee) settles
        // and is emitted BEFORE c (the caller) — this is the opposite pairing from the
        // brief's inline comment, which had caller/callee backwards; verified against
        // `tarjan_scc`'s actual output (see task-3-report.md) and invariant under DFS
        // root order (Tarjan settles a node's SCC only after every SCC it can reach).
        assert_eq!(members, vec![vec!["a".to_string()], vec!["c".to_string()]]);
    }

    #[test]
    fn missing_routine_excluded_same_as_leaf() {
        let graph = build_cycle_graph(&["a", "b"], &[("a", "b"), ("b", "a")]);
        let scc = Scc {
            members: vec!["a".into(), "b".into()],
            recursive: true,
        };
        let eff = effective_sccs(&scc, &graph, &|id| id != "b"); // b missing
        assert_eq!(eff.len(), 1);
        assert_eq!(eff[0].members, vec!["a".to_string()]);
        assert!(!eff[0].recursive);
    }

    // -----------------------------------------------------------------------
    // Step A: PD product-graph reachability (Task 4).
    //
    // Local fixture builders — NOT `crate::engine::l5::test_support` (that
    // module is `mod test_support;`, private to `l5`; `l4` is a sibling, not a
    // descendant, so it is unreachable even under `#[cfg(test)]` — the SAME
    // constraint `cfg_walker.rs`'s own test module already documents on its
    // `minimal_routine` helper).
    // -----------------------------------------------------------------------

    use crate::engine::l2::features::{
        PAnchor, PCallArgumentBinding, PCallSite, PCallee, PTempState,
    };
    use crate::engine::l3::call_resolver::UpgradedBinding;
    use crate::engine::l3::l3_workspace::{L3RecordOperation, L3Routine};
    use crate::engine::l4::effect_lattice::effect_key_of;
    use crate::engine::l4::summary::{DbEffect, RoutineSummary, TempState};
    use std::collections::HashSet;

    fn pd_anchor() -> PAnchor {
        PAnchor {
            source_unit_id: "ws:test".to_string(),
            start_line: 0,
            start_column: 0,
            end_line: 0,
            end_column: 0,
            syntax_kind: "test".to_string(),
        }
    }

    /// A minimal, body-available `L3Routine` with just an id; callers push
    /// `record_operations`/`call_sites` onto it (mirrors
    /// `cfg_walker::tests::minimal_routine` / the differential harness's own
    /// `routine` ctor — the SAME from-scratch pattern, unavoidable here since
    /// `l5::test_support` is private to `l5`).
    fn pd_routine(id: &str) -> L3Routine {
        L3Routine {
            id: id.to_string(),
            stable_routine_id: format!("stable::{id}"),
            object_id: "app/Codeunit/1".to_string(),
            object_type: "Codeunit".to_string(),
            name: id.to_string(),
            kind: "procedure".to_string(),
            attributes_parsed: Vec::new(),
            app_guid: "app".to_string(),
            object_number: 1,
            normalized_signature_hash: String::new(),
            body_available: true,
            parse_incomplete: false,
            record_variables: Vec::new(),
            record_operations: Vec::new(),
            field_accesses: Vec::new(),
            variables: Vec::new(),
            parameters: Vec::new(),
            access_modifier: None,
            return_type: None,
            call_sites: Vec::new(),
            operation_sites: Vec::new(),
            statement_tree: None,
            loops: Vec::new(),
            source_anchor: pd_anchor(),
            identifier_references: Vec::new(),
            unreachable_statements: Vec::new(),
            has_branching: false,
            var_assignments: Vec::new(),
            condition_references: Vec::new(),
            enclosing_member: None,
            originating_object: None,
            enclosing_member_range: None,
            entry_temp_guard_receiver: None,
        }
    }

    fn pd_ts_known(value: bool) -> PTempState {
        PTempState {
            kind: "known".to_string(),
            value: Some(value),
            parameter_index: None,
        }
    }

    fn pd_ts_pd(idx: u32) -> PTempState {
        PTempState {
            kind: "parameter-dependent".to_string(),
            value: None,
            parameter_index: Some(idx),
        }
    }

    /// One db-touching record operation with the given (always-PD-or-known)
    /// temp state; `record_variable_name` is always `"Rec"` — no fixture below
    /// needs `RecordRoleSummary` field precision.
    fn pd_record_op(
        id: &str,
        op: &str,
        table_id: &str,
        temp_state: PTempState,
    ) -> L3RecordOperation {
        L3RecordOperation {
            id: id.to_string(),
            op: op.to_string(),
            record_variable_name: "Rec".to_string(),
            record_variable_id: None,
            table_id: Some(table_id.to_string()),
            temp_state: Some(temp_state),
            field_arguments: None,
            source_anchor: pd_anchor(),
            loop_stack: Vec::new(),
            field_argument_infos: None,
            in_until_condition: false,
            run_trigger: None,
        }
    }

    /// One argument binding of callee param `parameter_index` to
    /// `source_temp_state` — the shape `substitute_pd_temp_state` reads.
    fn pd_arg_binding(
        parameter_index: u32,
        source_temp_state: Option<PTempState>,
    ) -> PCallArgumentBinding {
        PCallArgumentBinding {
            parameter_index,
            source_kind: "variable".to_string(),
            source_variable_name: Some("arg".to_string()),
            source_record_variable_id: None,
            source_parameter_index: None,
            caller_source_parameter_is_var: None,
            source_temp_state,
            argument_anchor: pd_anchor(),
        }
    }

    /// A bare call site `id` calling `callee_name`, with the given argument
    /// bindings.
    fn pd_call_site(
        id: &str,
        callee_name: &str,
        argument_bindings: Vec<PCallArgumentBinding>,
    ) -> PCallSite {
        PCallSite {
            id: id.to_string(),
            operation_id: format!("{id}/op"),
            callee_text: callee_name.to_string(),
            callee: PCallee::Bare {
                name: callee_name.to_string(),
            },
            argument_texts: Vec::new(),
            argument_infos: Vec::new(),
            argument_bindings,
            loop_stack: Vec::new(),
            source_anchor: pd_anchor(),
            result_consumed: None,
            object_run_return_used: None,
            under_asserterror: None,
            control_context: None,
            order: None,
            in_statement_position: false,
        }
    }

    /// A resolved "direct" combined edge `from -> to` with a real callsite id
    /// (every Task-4 fixture below uses only direct, binding-carrying edges).
    fn pd_edge(from: &str, to: &str, callsite_id: &str) -> CombinedEdge {
        CombinedEdge {
            from: from.to_string(),
            to: to.to_string(),
            kind: "direct".to_string(),
            callsite_id: Some(callsite_id.to_string()),
            operation_id: None,
            event_id: None,
            subscriber_app_id: None,
            resolution: "resolved".to_string(),
        }
    }

    /// A `CombinedGraph` from a node list + flat edge list, grouped by `from`.
    fn pd_graph(nodes: &[&str], edges: Vec<CombinedEdge>) -> CombinedGraph {
        let mut node_vec: Vec<String> = nodes.iter().map(|n| n.to_string()).collect();
        node_vec.sort();
        let mut edges_by_from: HashMap<String, Vec<CombinedEdge>> = HashMap::new();
        let mut edges_from_order: Vec<String> = Vec::new();
        for e in edges {
            if !edges_by_from.contains_key(&e.from) {
                edges_from_order.push(e.from.clone());
            }
            edges_by_from.entry(e.from.clone()).or_default().push(e);
        }
        CombinedGraph {
            nodes: node_vec,
            edges_by_from,
            edges_from_order,
            uncertainty_edges: Vec::new(),
            typed_edges: Vec::new(),
        }
    }

    fn pd_routines_by_id(routines: &[L3Routine]) -> HashMap<String, &L3Routine> {
        routines.iter().map(|r| (r.id.clone(), r)).collect()
    }

    #[test]
    fn pd_to_pd_chain_two_hops_then_known() {
        // c (base PD(0)) <- b (forwards its own param1, PD(1)) <- a (binds
        // param1 to Known(true)): a two-hop re-symbolizing chain that
        // terminates at `a` as a Known(true) TerminalEmission.
        let mut c = pd_routine("c");
        c.record_operations
            .push(pd_record_op("c_op1", "Insert", "t1", pd_ts_pd(0)));

        let mut b = pd_routine("b");
        b.call_sites.push(pd_call_site(
            "b_cs1",
            "C",
            vec![pd_arg_binding(0, Some(pd_ts_pd(1)))],
        ));

        let mut a = pd_routine("a");
        a.call_sites.push(pd_call_site(
            "a_cs1",
            "B",
            vec![pd_arg_binding(1, Some(pd_ts_known(true)))],
        ));

        let routines = vec![a, b, c];
        let rbid = pd_routines_by_id(&routines);
        let graph = pd_graph(
            &["a", "b", "c"],
            vec![pd_edge("b", "c", "b_cs1"), pd_edge("a", "b", "a_cs1")],
        );
        let eff = Scc {
            members: vec!["a".into(), "b".into(), "c".into()],
            recursive: true,
        };
        let settled: HashMap<String, RoutineSummary> = HashMap::new();
        let ub: HashMap<String, Vec<UpgradedBinding>> = HashMap::new();

        let (pd_facts, terminals) = solve_pd_reachability(&eff, &graph, &rbid, &settled, &ub);

        let base = ("Insert".to_string(), "t1".to_string(), "c_op1".to_string());
        let expected_facts: HashSet<PdFact> = [
            PdFact {
                routine_id: "c".to_string(),
                base: base.clone(),
                param_index: 0,
            },
            PdFact {
                routine_id: "b".to_string(),
                base: base.clone(),
                param_index: 1,
            },
        ]
        .into_iter()
        .collect();
        let expected_terminals: HashSet<TerminalEmission> = [TerminalEmission {
            routine_id: "a".to_string(),
            base: base.clone(),
            temp: TempStateKind::Known(true),
        }]
        .into_iter()
        .collect();

        assert_eq!(
            pd_facts.iter().cloned().collect::<HashSet<_>>(),
            expected_facts
        );
        assert_eq!(
            terminals.iter().cloned().collect::<HashSet<_>>(),
            expected_terminals
        );
    }

    #[test]
    fn pd_to_known_emission() {
        let mut callee = pd_routine("callee");
        callee
            .record_operations
            .push(pd_record_op("callee_op1", "Insert", "t1", pd_ts_pd(0)));

        let mut caller = pd_routine("caller");
        caller.call_sites.push(pd_call_site(
            "caller_cs1",
            "Callee",
            vec![pd_arg_binding(0, Some(pd_ts_known(true)))],
        ));

        let routines = vec![callee, caller];
        let rbid = pd_routines_by_id(&routines);
        let graph = pd_graph(
            &["callee", "caller"],
            vec![pd_edge("caller", "callee", "caller_cs1")],
        );
        let eff = Scc {
            members: vec!["callee".into(), "caller".into()],
            recursive: true,
        };
        let settled: HashMap<String, RoutineSummary> = HashMap::new();
        let ub: HashMap<String, Vec<UpgradedBinding>> = HashMap::new();

        let (pd_facts, terminals) = solve_pd_reachability(&eff, &graph, &rbid, &settled, &ub);

        let base = (
            "Insert".to_string(),
            "t1".to_string(),
            "callee_op1".to_string(),
        );
        let expected_facts: HashSet<PdFact> = [PdFact {
            routine_id: "callee".to_string(),
            base: base.clone(),
            param_index: 0,
        }]
        .into_iter()
        .collect();
        let expected_terminals: HashSet<TerminalEmission> = [TerminalEmission {
            routine_id: "caller".to_string(),
            base: base.clone(),
            temp: TempStateKind::Known(true),
        }]
        .into_iter()
        .collect();

        assert_eq!(
            pd_facts.iter().cloned().collect::<HashSet<_>>(),
            expected_facts
        );
        assert_eq!(
            terminals.iter().cloned().collect::<HashSet<_>>(),
            expected_terminals
        );
    }

    #[test]
    fn pd_to_unknown_emission() {
        // Same shape as `pd_to_known_emission`, but the binding carries no
        // captured source temp state (e.g. an unresolved/non-record argument)
        // — substitutes to `Unknown`.
        let mut callee = pd_routine("callee");
        callee
            .record_operations
            .push(pd_record_op("callee_op1", "Insert", "t1", pd_ts_pd(0)));

        let mut caller = pd_routine("caller");
        caller.call_sites.push(pd_call_site(
            "caller_cs1",
            "Callee",
            vec![pd_arg_binding(0, None)],
        ));

        let routines = vec![callee, caller];
        let rbid = pd_routines_by_id(&routines);
        let graph = pd_graph(
            &["callee", "caller"],
            vec![pd_edge("caller", "callee", "caller_cs1")],
        );
        let eff = Scc {
            members: vec!["callee".into(), "caller".into()],
            recursive: true,
        };
        let settled: HashMap<String, RoutineSummary> = HashMap::new();
        let ub: HashMap<String, Vec<UpgradedBinding>> = HashMap::new();

        let (pd_facts, terminals) = solve_pd_reachability(&eff, &graph, &rbid, &settled, &ub);

        let base = (
            "Insert".to_string(),
            "t1".to_string(),
            "callee_op1".to_string(),
        );
        let expected_facts: HashSet<PdFact> = [PdFact {
            routine_id: "callee".to_string(),
            base: base.clone(),
            param_index: 0,
        }]
        .into_iter()
        .collect();
        let expected_terminals: HashSet<TerminalEmission> = [TerminalEmission {
            routine_id: "caller".to_string(),
            base: base.clone(),
            temp: TempStateKind::Unknown,
        }]
        .into_iter()
        .collect();

        assert_eq!(
            pd_facts.iter().cloned().collect::<HashSet<_>>(),
            expected_facts
        );
        assert_eq!(
            terminals.iter().cloned().collect::<HashSet<_>>(),
            expected_terminals
        );
    }

    #[test]
    fn external_successor_pd_seed_resymbolizes() {
        // `ext` is an already-settled successor (NOT a member of `eff`)
        // carrying a ParameterDependent(0) effect. `a`'s out-edge to `ext`
        // forwards its OWN param 2 — the seed re-symbolizes to a NEW PdFact
        // retained on `a`, not a terminal (demonstrating a settled-derived
        // seed feeds into the same worklist machinery as an intra-eff hop).
        let mut a = pd_routine("a");
        a.call_sites.push(pd_call_site(
            "a_ext_cs",
            "Ext",
            vec![pd_arg_binding(0, Some(pd_ts_pd(2)))],
        ));

        let routines = vec![a];
        let rbid = pd_routines_by_id(&routines);
        let graph = pd_graph(&["a", "ext"], vec![pd_edge("a", "ext", "a_ext_cs")]);
        let eff = Scc {
            members: vec!["a".into()],
            recursive: false,
        };

        let base = (
            "Insert".to_string(),
            "t1".to_string(),
            "ext_op1".to_string(),
        );
        let ext_effect_key = effect_key_of(
            "Insert",
            "t1",
            "ext_op1",
            &TempStateKind::ParameterDependent(0),
        );
        let mut settled: HashMap<String, RoutineSummary> = HashMap::new();
        settled.insert(
            "ext".to_string(),
            RoutineSummary {
                routine_id: "ext".to_string(),
                db_effects: vec![DbEffect {
                    effect_key: ext_effect_key,
                    operation_id: "ext_op1".to_string(),
                    op: "Insert".to_string(),
                    table_id: "t1".to_string(),
                    record_variable_id: None,
                    temp_state: TempState::ParameterDependent(0),
                    via: "direct".to_string(),
                }],
                in_recursive_cycle: false,
                has_unresolved_calls: false,
                uncertainties: Vec::new(),
                parameter_roles: Vec::new(),
            },
        );
        let ub: HashMap<String, Vec<UpgradedBinding>> = HashMap::new();

        let (pd_facts, terminals) = solve_pd_reachability(&eff, &graph, &rbid, &settled, &ub);

        let expected_facts: HashSet<PdFact> = [PdFact {
            routine_id: "a".to_string(),
            base: base.clone(),
            param_index: 2,
        }]
        .into_iter()
        .collect();

        assert_eq!(
            pd_facts.iter().cloned().collect::<HashSet<_>>(),
            expected_facts
        );
        assert!(
            terminals.is_empty(),
            "expected no terminal emissions, got {terminals:?}"
        );
    }

    #[test]
    fn self_loop_pd_to_known() {
        // `a` calls itself; its own base op is PD(0), and its self-callsite
        // binds param 0 to Known(true) — the seed IS `a`'s own retained
        // PdFact (compose_routine seeds base db_effects verbatim, unsubstituted)
        // AND the self-edge substitution independently emits a Known terminal.
        let mut a = pd_routine("a");
        a.record_operations
            .push(pd_record_op("a_op1", "Insert", "t1", pd_ts_pd(0)));
        a.call_sites.push(pd_call_site(
            "a_cs1",
            "A",
            vec![pd_arg_binding(0, Some(pd_ts_known(true)))],
        ));

        let routines = vec![a];
        let rbid = pd_routines_by_id(&routines);
        let graph = pd_graph(&["a"], vec![pd_edge("a", "a", "a_cs1")]);
        let eff = Scc {
            members: vec!["a".into()],
            recursive: true,
        };
        let settled: HashMap<String, RoutineSummary> = HashMap::new();
        let ub: HashMap<String, Vec<UpgradedBinding>> = HashMap::new();

        let (pd_facts, terminals) = solve_pd_reachability(&eff, &graph, &rbid, &settled, &ub);

        let base = ("Insert".to_string(), "t1".to_string(), "a_op1".to_string());
        let expected_facts: HashSet<PdFact> = [PdFact {
            routine_id: "a".to_string(),
            base: base.clone(),
            param_index: 0,
        }]
        .into_iter()
        .collect();
        let expected_terminals: HashSet<TerminalEmission> = [TerminalEmission {
            routine_id: "a".to_string(),
            base: base.clone(),
            temp: TempStateKind::Known(true),
        }]
        .into_iter()
        .collect();

        assert_eq!(
            pd_facts.iter().cloned().collect::<HashSet<_>>(),
            expected_facts
        );
        assert_eq!(
            terminals.iter().cloned().collect::<HashSet<_>>(),
            expected_terminals
        );
    }

    // -----------------------------------------------------------------------
    // Steps B/C: closed-form terminal union + per-member presence (Task 5).
    // -----------------------------------------------------------------------

    #[test]
    fn recursive_members_share_terminal_union_plus_own_pd() {
        // A<->B; A has base Known(true) effect e1; B has base Unknown effect
        // e2; B has a PD fact (from Step A) on b only. Expect: A and B both
        // contain {e1,e2}; only B additionally carries its PD-keyed effect.
        let a_effect_key = effect_key_of("Insert", "t1", "a_op1", &TempStateKind::Known(true));
        let b_effect_key = effect_key_of("Modify", "t2", "b_op1", &TempStateKind::Unknown);

        let mut base_summaries: HashMap<String, RoutineSummary> = HashMap::new();
        base_summaries.insert(
            "a".to_string(),
            RoutineSummary {
                routine_id: "a".to_string(),
                db_effects: vec![DbEffect {
                    effect_key: a_effect_key,
                    operation_id: "a_op1".to_string(),
                    op: "Insert".to_string(),
                    table_id: "t1".to_string(),
                    record_variable_id: None,
                    temp_state: TempState::Known(true),
                    via: "direct".to_string(),
                }],
                in_recursive_cycle: true,
                has_unresolved_calls: false,
                uncertainties: Vec::new(),
                parameter_roles: Vec::new(),
            },
        );
        base_summaries.insert(
            "b".to_string(),
            RoutineSummary {
                routine_id: "b".to_string(),
                db_effects: vec![DbEffect {
                    effect_key: b_effect_key,
                    operation_id: "b_op1".to_string(),
                    op: "Modify".to_string(),
                    table_id: "t2".to_string(),
                    record_variable_id: None,
                    temp_state: TempState::Unknown,
                    via: "direct".to_string(),
                }],
                in_recursive_cycle: true,
                has_unresolved_calls: false,
                uncertainties: Vec::new(),
                parameter_roles: Vec::new(),
            },
        );

        let routines = vec![pd_routine("a"), pd_routine("b")];
        let rbid = pd_routines_by_id(&routines);
        let graph = pd_graph(
            &["a", "b"],
            vec![pd_edge("a", "b", "a_cs1"), pd_edge("b", "a", "b_cs1")],
        );
        let eff = Scc {
            members: vec!["a".into(), "b".into()],
            recursive: true,
        };
        let settled: HashMap<String, RoutineSummary> = HashMap::new();
        let pd_facts = vec![PdFact {
            routine_id: "b".to_string(),
            base: ("Delete".to_string(), "t3".to_string(), "b_op3".to_string()),
            param_index: 0,
        }];
        let terminal_emissions: Vec<TerminalEmission> = Vec::new();
        let mut universe = EffectUniverse::new();

        let presence = closed_form_union(
            &eff,
            &graph,
            &rbid,
            &settled,
            &base_summaries,
            &pd_facts,
            &terminal_emissions,
            &mut universe,
        );

        let e1 = universe.intern(&EffectIdentity {
            op: "Insert".to_string(),
            table_id: "t1".to_string(),
            operation_id: "a_op1".to_string(),
            temp: TempStateKind::Known(true),
        });
        let e2 = universe.intern(&EffectIdentity {
            op: "Modify".to_string(),
            table_id: "t2".to_string(),
            operation_id: "b_op1".to_string(),
            temp: TempStateKind::Unknown,
        });
        let epd = universe.intern(&EffectIdentity {
            op: "Delete".to_string(),
            table_id: "t3".to_string(),
            operation_id: "b_op3".to_string(),
            temp: TempStateKind::ParameterDependent(0),
        });

        let a_bits = presence.by_member.get("a").expect("a present");
        let b_bits = presence.by_member.get("b").expect("b present");

        assert!(has_bit(a_bits, e1), "a must carry e1 (own base)");
        assert!(has_bit(a_bits, e2), "a must carry e2 (shared SCC closure)");
        assert!(!has_bit(a_bits, epd), "a must NOT carry b's own PD fact");

        assert!(has_bit(b_bits, e1), "b must carry e1 (shared SCC closure)");
        assert!(has_bit(b_bits, e2), "b must carry e2 (own base)");
        assert!(has_bit(b_bits, epd), "b must carry its own PD fact");
    }

    #[test]
    fn settled_successor_terminal_contributes_to_c_but_its_pd_effect_does_not() {
        // `a` (a singleton effective SCC) calls an already-settled successor
        // `ext` over an actual out-edge. `ext`'s Known effect must join `a`'s
        // presence set (transfers by identity); `ext`'s ParameterDependent
        // effect must NOT — Step A already owns PD outcomes, and re-adding it
        // here (under its unsubstituted callee-frame index) would be wrong.
        let known_key = effect_key_of("Insert", "t4", "ext_op1", &TempStateKind::Known(true));
        let mut settled: HashMap<String, RoutineSummary> = HashMap::new();
        settled.insert(
            "ext".to_string(),
            RoutineSummary {
                routine_id: "ext".to_string(),
                db_effects: vec![
                    DbEffect {
                        effect_key: known_key,
                        operation_id: "ext_op1".to_string(),
                        op: "Insert".to_string(),
                        table_id: "t4".to_string(),
                        record_variable_id: None,
                        temp_state: TempState::Known(true),
                        via: "direct".to_string(),
                    },
                    DbEffect {
                        effect_key: effect_key_of(
                            "Delete",
                            "t5",
                            "ext_op2",
                            &TempStateKind::ParameterDependent(0),
                        ),
                        operation_id: "ext_op2".to_string(),
                        op: "Delete".to_string(),
                        table_id: "t5".to_string(),
                        record_variable_id: None,
                        temp_state: TempState::ParameterDependent(0),
                        via: "direct".to_string(),
                    },
                ],
                in_recursive_cycle: false,
                has_unresolved_calls: false,
                uncertainties: Vec::new(),
                parameter_roles: Vec::new(),
            },
        );

        let routines = vec![pd_routine("a")];
        let rbid = pd_routines_by_id(&routines);
        let graph = pd_graph(&["a", "ext"], vec![pd_edge("a", "ext", "a_cs1")]);
        let eff = Scc {
            members: vec!["a".into()],
            recursive: false,
        };
        let base_summaries: HashMap<String, RoutineSummary> = HashMap::new(); // "a" falls back to base_intraprocedural_summary (no own effects).
        let pd_facts: Vec<PdFact> = Vec::new();
        let terminal_emissions: Vec<TerminalEmission> = Vec::new();
        let mut universe = EffectUniverse::new();

        let presence = closed_form_union(
            &eff,
            &graph,
            &rbid,
            &settled,
            &base_summaries,
            &pd_facts,
            &terminal_emissions,
            &mut universe,
        );

        let known_id = universe.intern(&EffectIdentity {
            op: "Insert".to_string(),
            table_id: "t4".to_string(),
            operation_id: "ext_op1".to_string(),
            temp: TempStateKind::Known(true),
        });
        let a_bits = presence.by_member.get("a").expect("a present");
        assert!(
            has_bit(a_bits, known_id),
            "a must carry ext's settled terminal effect"
        );

        let pd_identity = EffectIdentity {
            op: "Delete".to_string(),
            table_id: "t5".to_string(),
            operation_id: "ext_op2".to_string(),
            temp: TempStateKind::ParameterDependent(0),
        };
        assert_eq!(
            universe.get(&pd_identity),
            None,
            "ext's PD effect must never be interned by closed_form_union at all"
        );
    }

    #[test]
    fn base_pd_effect_is_skipped_from_c() {
        // A member's OWN base summary carrying a raw ParameterDependent
        // db_effect entry (as opposed to a Step-A-retained PdFact) must NOT
        // contribute to `C` — only its Known/Unknown siblings do.
        let known_key = effect_key_of("Insert", "t1", "a_op1", &TempStateKind::Known(true));
        let mut base_summaries: HashMap<String, RoutineSummary> = HashMap::new();
        base_summaries.insert(
            "a".to_string(),
            RoutineSummary {
                routine_id: "a".to_string(),
                db_effects: vec![
                    DbEffect {
                        effect_key: known_key,
                        operation_id: "a_op1".to_string(),
                        op: "Insert".to_string(),
                        table_id: "t1".to_string(),
                        record_variable_id: None,
                        temp_state: TempState::Known(true),
                        via: "direct".to_string(),
                    },
                    DbEffect {
                        effect_key: effect_key_of(
                            "Delete",
                            "t9",
                            "a_op9",
                            &TempStateKind::ParameterDependent(3),
                        ),
                        operation_id: "a_op9".to_string(),
                        op: "Delete".to_string(),
                        table_id: "t9".to_string(),
                        record_variable_id: None,
                        temp_state: TempState::ParameterDependent(3),
                        via: "direct".to_string(),
                    },
                ],
                in_recursive_cycle: false,
                has_unresolved_calls: false,
                uncertainties: Vec::new(),
                parameter_roles: Vec::new(),
            },
        );

        let routines = vec![pd_routine("a")];
        let rbid = pd_routines_by_id(&routines);
        let graph = pd_graph(&["a"], Vec::new());
        let eff = Scc {
            members: vec!["a".into()],
            recursive: false,
        };
        let settled: HashMap<String, RoutineSummary> = HashMap::new();
        let pd_facts: Vec<PdFact> = Vec::new();
        let terminal_emissions: Vec<TerminalEmission> = Vec::new();
        let mut universe = EffectUniverse::new();

        let presence = closed_form_union(
            &eff,
            &graph,
            &rbid,
            &settled,
            &base_summaries,
            &pd_facts,
            &terminal_emissions,
            &mut universe,
        );

        let known_id = universe.intern(&EffectIdentity {
            op: "Insert".to_string(),
            table_id: "t1".to_string(),
            operation_id: "a_op1".to_string(),
            temp: TempStateKind::Known(true),
        });
        let a_bits = presence.by_member.get("a").expect("a present");
        assert!(
            has_bit(a_bits, known_id),
            "a must carry its own Known effect"
        );

        let pd_identity = EffectIdentity {
            op: "Delete".to_string(),
            table_id: "t9".to_string(),
            operation_id: "a_op9".to_string(),
            temp: TempStateKind::ParameterDependent(3),
        };
        assert_eq!(
            universe.get(&pd_identity),
            None,
            "the member's own base PD effect must never be interned by closed_form_union"
        );
    }

    #[test]
    fn terminal_emission_lands_in_every_member_presence() {
        // A<->B; no base effects, no settled successors, no PD facts — a
        // single `TerminalEmission` (Step A's output) is the ONLY source
        // contributing to `C`. Its `EffectId` bit must be set in EVERY
        // member's presence bitset (source 3's "shared across the whole
        // SCC" contract), even though it's recorded against just one
        // `routine_id` ("a") and its base identity matches nothing else in
        // scope (no base effect, no settled-successor effect).
        let routines = vec![pd_routine("a"), pd_routine("b")];
        let rbid = pd_routines_by_id(&routines);
        let graph = pd_graph(
            &["a", "b"],
            vec![pd_edge("a", "b", "a_cs1"), pd_edge("b", "a", "b_cs1")],
        );
        let eff = Scc {
            members: vec!["a".into(), "b".into()],
            recursive: true,
        };
        let base_summaries: HashMap<String, RoutineSummary> = HashMap::new();
        let settled: HashMap<String, RoutineSummary> = HashMap::new();
        let pd_facts: Vec<PdFact> = Vec::new();
        let terminal_emissions = vec![TerminalEmission {
            routine_id: "a".to_string(),
            base: ("Insert".to_string(), "t7".to_string(), "op7".to_string()),
            temp: TempStateKind::Known(true),
        }];
        let mut universe = EffectUniverse::new();

        let presence = closed_form_union(
            &eff,
            &graph,
            &rbid,
            &settled,
            &base_summaries,
            &pd_facts,
            &terminal_emissions,
            &mut universe,
        );

        let emitted_id = universe.intern(&EffectIdentity {
            op: "Insert".to_string(),
            table_id: "t7".to_string(),
            operation_id: "op7".to_string(),
            temp: TempStateKind::Known(true),
        });

        let a_bits = presence.by_member.get("a").expect("a present");
        let b_bits = presence.by_member.get("b").expect("b present");
        assert!(
            has_bit(a_bits, emitted_id),
            "terminal emission must land on the member it was recorded against"
        );
        assert!(
            has_bit(b_bits, emitted_id),
            "terminal emission must be shared into EVERY member's presence, not just the one it landed on"
        );
    }
}
