//! `d1_witness` — Task C3 of the d1 cohort redesign
//! (`.superpowers/sdd/task-c3-brief.md`,
//! `docs/superpowers/plans/2026-07-21-d1-cohort-redesign.md`): ONE bounded
//! representative witness per `(terminal, ContextKey)`, replacing the
//! per-`(loop, terminal)` FULL witness [`d1_dataflow::build_transitive_witness`]
//! built for every winning lane (the ~28k-hop predecessor-chain walk that drove
//! the Base App 8020 ~8h run before Task C1's sink cutover — see that module's
//! doc). Task C1's [`crate::engine::l5::d1_cohort::TerminalSink`] already
//! collapses 3.2M `(loop, terminal)` aggregates down to ~34,861 `(terminal,
//! ContextKey)` cohorts; this module makes the ONE witness each cohort still
//! owes CHEAP, instead of building one PER WINNING LANE.
//!
//! ## Design
//!
//! The winner's SEED is read directly from [`BatchSolver::reach_origin`] /
//! [`BatchSolver::value_origin`] (Task C2) — an O(1) lookup, never a walk to
//! FIND it. `total_hops` is read directly from [`BatchSolver::reach_hops`] /
//! [`BatchSolver::value_hops`] — the authoritative first-arrival hop count, no
//! recompute. The hop STEPS themselves (for display) still need the actual
//! `(from_node, edge_k)` sequence, so [`representative_witness`] walks the
//! predecessor chain ONCE per cohort (`collect_reach_chain_b`/
//! `collect_value_chain_b` — option (a) from the task brief: bounded by the
//! cohort count, ~34,861, NOT the 3.2M `(loop, terminal)` pairs the old
//! per-winner witness build walked), then slices the result to the first-K +
//! last-M hop steps a human witness needs, with an `omitted_hops` count for the
//! (possibly empty) middle. The full uncertainty-vector union and the TRUE
//! (unclamped) effective-depth recompute [`d1_dataflow::build_transitive_witness`]
//! builds are DROPPED — the cohort's own `ContextKey` already carries the exact
//! `depth_bucket`/`unc` (Task C1), so this witness owes only a REPRESENTATIVE
//! realizing path, not a second derivation of those two fields.
//!
//! Nothing wires [`representative_witness`] into `detect_d1` yet — that is a
//! later cohort-redesign task (compressed report schema + consumer cutover).
#![allow(dead_code)]

use crate::engine::l3::l3_workspace::{L3RecordOperation, L3Routine};
use crate::engine::l5::d1_dataflow::{BatchSolver, collect_reach_chain_b, collect_value_chain_b};
use crate::engine::l5::d1_graph::{D1Graph, D1Seed, NodeIx};
use crate::engine::l5::d1_reach::{call_step_ev, loop_step_ev};
use crate::engine::l5::detector_context::DetectorContext;
use crate::engine::l5::detectors::d1::{hop_step, terminal_step};
use crate::engine::l5::finding::EvidenceStep;

/// A bounded representative witness for one `(terminal, ContextKey)` cohort:
/// `[loop_step, call_step]` + up to `k_first` hop steps nearest the seed, an
/// `omitted_hops` count for the (possibly empty) middle, up to `m_last` hop
/// steps nearest the terminal, and the terminal step itself.
///
/// `total_hops` is the AUTHORITATIVE first-arrival hop count
/// (`BatchSolver::reach_hops`/`value_hops`) — independent of how many hop
/// steps were actually materialized into `first_steps`/`last_steps`.
pub struct WitnessSummary {
    pub total_hops: u32,
    /// `[loop_step, call_step]` + up to `k_first` hop steps, in seed->terminal
    /// order. Never empty — always carries the two prefix steps. When the full
    /// chain fits within `k_first + m_last` hops, EVERY hop step lives here
    /// (the shallow case) and `last_steps` is empty.
    pub first_steps: Vec<EvidenceStep>,
    /// Hops skipped between `first_steps`' and `last_steps`' hop portions.
    /// `0` when the whole chain fit in `first_steps` (the shallow case).
    pub omitted_hops: u32,
    /// Up to `m_last` hop steps immediately preceding the terminal, in
    /// seed->terminal order. Empty in the shallow case.
    pub last_steps: Vec<EvidenceStep>,
    pub terminal_step: EvidenceStep,
}

/// One graph hop, `(from_node, edge_k)` into `D1Graph::edges[from_node]`.
type HopSlice<'h> = &'h [(NodeIx, usize)];

/// Render one graph hop `(from_node, edge_k)` into its [`EvidenceStep`] — the
/// same per-hop [`hop_step`] call `d1_dataflow::build_transitive_witness` makes.
fn render_hop(
    graph: &D1Graph,
    ctx: &DetectorContext,
    from_node: NodeIx,
    edge_k: usize,
) -> EvidenceStep {
    let edge = &graph.edges[from_node as usize][edge_k];
    let from_id = graph.node_ids[from_node as usize];
    let to_id = graph.node_ids[edge.to as usize];
    hop_step(
        &ctx.routine_by_id,
        from_id,
        to_id,
        edge.kind,
        edge.callsite_id,
    )
}

/// Build ONE bounded representative witness for a winning (lane, fact) pair.
///
/// `fact_ix`/`is_value_fact` select the winning reach or value fact (indexed
/// into `BatchSolver::reach_facts`/`value_facts`, the same way
/// `d1_dataflow::BestSource::Reach`/`Value` do); `terminal_node`/
/// `terminal_owner`/`terminal_op` are the terminal's own identity (the caller
/// already has these when it has a fact to build a witness for — mirrors
/// `build_transitive_witness`'s parameters).
///
/// Algorithm: the seed is read via [`BatchSolver::reach_origin`]/
/// [`BatchSolver::value_origin`] (Task C2) — O(1), NOT a walk to find it. The
/// hop STEPS are collected by one bounded walk of the predecessor chain
/// (`collect_reach_chain_b`/`collect_value_chain_b`), reversed to seed->terminal
/// order, then sliced: if the chain fits within `k_first + m_last` hops, the
/// WHOLE chain goes into `first_steps` (`last_steps` empty, `omitted_hops` 0);
/// otherwise the first `k_first` hops go into `first_steps`, the last `m_last`
/// into `last_steps`, and the count in between is reported as `omitted_hops`.
/// A `debug_assert` cross-checks the walk's own seed against the O(1) origin
/// lookup — the correctness invariant (Task C2) this witness's cheap seed
/// lookup relies on.
#[allow(clippy::too_many_arguments)]
pub(crate) fn representative_witness<'a>(
    solver: &BatchSolver,
    graph: &D1Graph<'a>,
    ctx: &DetectorContext,
    seeds: &[D1Seed<'a>],
    lane: usize,
    fact_ix: usize,
    is_value_fact: bool,
    terminal_node: NodeIx,
    terminal_owner: &'a L3Routine,
    terminal_op: &'a L3RecordOperation,
    k_first: usize,
    m_last: usize,
) -> WitnessSummary {
    // The seed: O(1) via Task C2's origin array — never a chain walk to find it.
    let origin_seed_index = (if is_value_fact {
        solver.value_origin[fact_ix][lane]
    } else {
        solver.reach_origin[fact_ix][lane]
    }) as usize;
    debug_assert_ne!(
        origin_seed_index,
        u32::MAX as usize,
        "a present (fact, lane) always carries a resolved origin seed"
    );

    // total_hops: authoritative, straight off the fixpoint's own first-arrival
    // record — never recomputed.
    let total_hops = if is_value_fact {
        solver.value_hops[fact_ix][lane]
    } else {
        solver.reach_hops[fact_ix][lane]
    };

    // ONE bounded walk of the predecessor chain, for the hop STEPS only
    // (option (a) from the task brief — bounded per cohort, not per-(loop,
    // terminal)). Returned in TERMINAL->SEED order; reverse for display.
    let (hops_terminal_to_seed, walked_seed_index) = if is_value_fact {
        collect_value_chain_b(&solver.value_pred, &solver.reach_pred, lane, fact_ix)
    } else {
        collect_reach_chain_b(&solver.reach_pred, lane, fact_ix)
    };
    debug_assert_eq!(
        walked_seed_index, origin_seed_index,
        "origin_seed (Task C2) must equal the chain-walk seed — the correctness \
         invariant this witness's O(1) seed lookup relies on"
    );
    debug_assert_eq!(
        hops_terminal_to_seed.len() as u32,
        total_hops,
        "reach_hops/value_hops must equal the chain-walk's own hop count"
    );
    let mut hops = hops_terminal_to_seed;
    hops.reverse(); // now seed -> terminal order

    let seed = &seeds[origin_seed_index];

    // Path validity: the chain's terminal-most hop must land on terminal_node
    // (a zero-hop witness's seed entry must BE the terminal node instead).
    match hops.last() {
        Some(&(from_node, edge_k)) => {
            debug_assert_eq!(
                graph.edges[from_node as usize][edge_k].to, terminal_node,
                "the witness's terminal-most hop must land on the terminal node"
            );
        }
        None => {
            debug_assert_eq!(
                seed.entry, terminal_node,
                "a zero-hop witness's seed entry must BE the terminal node"
            );
        }
    }

    let n = hops.len();
    let (first_hops, omitted_hops, last_hops): (HopSlice, u32, HopSlice) = if n <= k_first + m_last
    {
        // Shallow: the whole chain fits — no omission, nothing in last_steps.
        (&hops[..], 0, &[])
    } else {
        let omitted = (n - k_first - m_last) as u32;
        (&hops[..k_first], omitted, &hops[n - m_last..])
    };

    let mut first_steps: Vec<EvidenceStep> = Vec::with_capacity(2 + first_hops.len());
    first_steps.push(loop_step_ev(seed.loop_routine, seed.loop_info));
    first_steps.push(call_step_ev(seed, graph, ctx));
    first_steps.extend(
        first_hops
            .iter()
            .map(|&(from_node, edge_k)| render_hop(graph, ctx, from_node, edge_k)),
    );

    let last_steps: Vec<EvidenceStep> = last_hops
        .iter()
        .map(|&(from_node, edge_k)| render_hop(graph, ctx, from_node, edge_k))
        .collect();

    let terminal_ev = terminal_step(
        &ctx.routine_by_id,
        &ctx.table_by_id,
        terminal_owner.id.as_str(),
        Some(terminal_op.id.as_str()),
    );

    WitnessSummary {
        total_hops,
        first_steps,
        omitted_hops,
        last_steps,
        terminal_step: terminal_ev,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::engine::l3::l3_workspace::{L3Routine, L3Workspace};
    use crate::engine::l4::combined_graph::CombinedEdge;
    use crate::engine::l5::closed_world_temp::ClosedWorldTempParams;
    use crate::engine::l5::d1_dataflow::{GroupSpec, condense, run_batch_fixpoint_for_test};
    use crate::engine::l5::d1_graph::build_d1_graph;
    use crate::engine::l5::d1_liveness::compute_liveness;
    use crate::engine::l5::full_summary::FullRoutineSummary;
    use crate::engine::l5::test_support::{
        arg_binding, call_site, coverage, edge_kind, fact, loop_def, minimal_ctx, record_op,
        routine, summary, ts_known, ts_pd,
    };

    type Fixture = (
        Vec<L3Routine>,
        HashMap<String, Vec<CombinedEdge>>,
        HashMap<String, FullRoutineSummary>,
    );

    fn db_summary(id: &str, table: &str) -> FullRoutineSummary {
        summary(
            id,
            vec![fact("read", "table", Some(table))],
            vec![],
            Some(coverage("complete")),
        )
    }

    /// A throwaway `L3Workspace` wrapping a CLONE of `routines` — `build_d1_graph`
    /// needs an owned `&L3Workspace` sharing `ctx`'s own lifetime; mirrors
    /// `d1_dataflow::tests::ws`.
    fn ws(routines: &[L3Routine]) -> L3Workspace {
        L3Workspace {
            objects: vec![],
            tables: vec![],
            routines: routines.to_vec(),
        }
    }

    /// A chain of `depth` intermediate routines between the seed entry `A` and
    /// the terminal `T`: `R --(loop, seed)--> A -> C1 -> C2 -> ... -> C{depth}
    /// -> T`. Every hop is a plain direct call (no argument bindings), so `T`'s
    /// op (no `temp_state`, i.e. non-PD/constant) is read via a REACH fact —
    /// `depth + 1` hops from `A` to `T`.
    fn deep_reach_chain_fixture(depth: usize) -> Fixture {
        let mut r = routine("R", "procedure");
        r.loops = vec![loop_def("R/loop0")];
        r.call_sites = vec![call_site("R/cs0", "A", vec!["R/loop0".to_string()])];

        let mut names: Vec<String> = vec!["A".to_string()];
        for i in 1..=depth {
            names.push(format!("C{i}"));
        }
        names.push("T".to_string());

        let mut routines = vec![r];
        let mut graph_edges: HashMap<String, Vec<CombinedEdge>> = HashMap::new();
        graph_edges.insert(
            "R".to_string(),
            vec![edge_kind("R", "A", "R/cs0", "direct")],
        );
        let mut summaries: HashMap<String, FullRoutineSummary> = HashMap::new();

        for i in 0..names.len() {
            let name = &names[i];
            summaries.insert(name.clone(), db_summary(name, &format!("t/{name}")));
            if i + 1 < names.len() {
                let next = &names[i + 1];
                let cs_id = format!("{name}/cs0");
                let mut rt = routine(name, "procedure");
                rt.call_sites = vec![call_site(&cs_id, next, vec![])];
                routines.push(rt);
                graph_edges.insert(name.clone(), vec![edge_kind(name, next, &cs_id, "direct")]);
            } else {
                // T: the terminal, a plain Modify with no temp_state (reads REACH).
                let mut t = routine(name, "procedure");
                t.record_operations = vec![record_op(
                    &format!("{name}/op0"),
                    "Modify",
                    "Rec",
                    Some(&format!("t/{name}")),
                    vec![],
                    false,
                )];
                routines.push(t);
            }
        }
        (routines, graph_edges, summaries)
    }

    /// `R --(loop, seed)--> A -> B -> C -> D -> H` (a VALUE-fact chain): `B`'s
    /// call to `C` binds `C`'s param 0 to a KNOWN literal (`ts_known`, a Const
    /// transfer — this is where the value chain switches onto the REACH chain,
    /// `ValuePredB::HopFromReach`, since a Const value has no caller-value
    /// parent); `C`'s and `D`'s calls forward their own param 0 (`ts_pd(0)`, a
    /// Copy transfer) onward, and `H` reads `ts_pd(0)` directly on its own op.
    /// 4 hops (A->B->C->D->H) from the seed entry `A` to the terminal `H`, with
    /// the HopFromReach transition mid-chain (at B's call to C).
    fn deep_value_chain_fixture() -> Fixture {
        let mut r = routine("R", "procedure");
        r.loops = vec![loop_def("R/loop0")];
        r.call_sites = vec![call_site("R/cs0", "A", vec!["R/loop0".to_string()])];

        let mut a = routine("A", "procedure");
        a.call_sites = vec![call_site("A/csB", "B", vec![])];

        let mut b = routine("B", "procedure");
        let mut b_cs = call_site("B/csC", "C", vec![]);
        b_cs.argument_bindings = vec![arg_binding(0, Some(ts_known(true)))];
        b.call_sites = vec![b_cs];

        let mut c = routine("C", "procedure");
        let mut c_cs = call_site("C/csD", "D", vec![]);
        c_cs.argument_bindings = vec![arg_binding(0, Some(ts_pd(0)))];
        c.call_sites = vec![c_cs];

        let mut d = routine("D", "procedure");
        let mut d_cs = call_site("D/csH", "H", vec![]);
        d_cs.argument_bindings = vec![arg_binding(0, Some(ts_pd(0)))];
        d.call_sites = vec![d_cs];

        let mut h = routine("H", "procedure");
        let mut op0 = record_op("H/op0", "Modify", "Rec", Some("t/H"), vec![], false);
        op0.temp_state = Some(ts_pd(0));
        h.record_operations = vec![op0];

        let routines = vec![r, a, b, c, d, h];

        let mut graph_edges: HashMap<String, Vec<CombinedEdge>> = HashMap::new();
        graph_edges.insert(
            "R".to_string(),
            vec![edge_kind("R", "A", "R/cs0", "direct")],
        );
        graph_edges.insert(
            "A".to_string(),
            vec![edge_kind("A", "B", "A/csB", "direct")],
        );
        graph_edges.insert(
            "B".to_string(),
            vec![edge_kind("B", "C", "B/csC", "direct")],
        );
        graph_edges.insert(
            "C".to_string(),
            vec![edge_kind("C", "D", "C/csD", "direct")],
        );
        graph_edges.insert(
            "D".to_string(),
            vec![edge_kind("D", "H", "D/csH", "direct")],
        );
        let summaries: HashMap<String, FullRoutineSummary> = ["A", "B", "C", "D", "H"]
            .iter()
            .map(|id| (id.to_string(), db_summary(id, &format!("t/{id}"))))
            .collect();
        (routines, graph_edges, summaries)
    }

    /// A real graph edge `from --from.callsite--> to_routine_id` exists —
    /// mirrors `d1_dataflow::tests::assert_witness_valid`'s contiguity check.
    fn assert_edge(ctx: &DetectorContext, from: &EvidenceStep, to_routine_id: &str) {
        assert!(
            from.callsite_id.is_some(),
            "an intermediate/call step must carry a callsite"
        );
        assert!(from.loop_id.is_none(), "non-loop intermediate step");
        let cs = from.callsite_id.as_deref();
        let is_real_edge = ctx
            .graph
            .edges_by_from
            .get(&from.routine_id)
            .is_some_and(|edges| {
                edges
                    .iter()
                    .any(|e| e.callsite_id.as_deref() == cs && e.to == to_routine_id)
            });
        assert!(
            is_real_edge,
            "step {} --{:?}--> {} must be a real graph edge (contiguity)",
            from.routine_id, cs, to_routine_id
        );
    }

    /// A `WitnessSummary` is a valid realizing path PREFIX (`first_steps`
    /// contiguous, `last_steps` contiguous where present, no gap-crossing
    /// contiguity claimed across an `omitted_hops` middle): first step is the
    /// loop step, second is the call step, the terminal step matches, and
    /// every consecutive pair within each contiguous run crosses a real edge.
    fn assert_witness_valid(
        w: &WitnessSummary,
        ctx: &DetectorContext,
        loop_routine_id: &str,
        loop_id: &str,
        terminal_owner_id: &str,
        terminal_op_id: &str,
    ) {
        assert!(
            w.first_steps.len() >= 2,
            "first_steps always carries [loop_step, call_step]"
        );
        assert_eq!(
            w.first_steps[0].routine_id, loop_routine_id,
            "first step must be in the loop routine"
        );
        assert_eq!(
            w.first_steps[0].loop_id.as_deref(),
            Some(loop_id),
            "first step must be the loop step"
        );
        assert!(
            w.first_steps[1].loop_id.is_none(),
            "second step (call step) is not a loop step"
        );
        assert!(
            w.first_steps[1].callsite_id.is_some(),
            "the call step carries a callsite"
        );

        assert_eq!(
            w.terminal_step.routine_id, terminal_owner_id,
            "terminal step must be owned by the terminal routine"
        );
        assert_eq!(
            w.terminal_step.operation_id.as_deref(),
            Some(terminal_op_id),
            "terminal step must be the terminal op"
        );
        assert!(
            w.terminal_step.callsite_id.is_none(),
            "the terminal step carries no callsite"
        );

        for pair in w.first_steps[1..].windows(2) {
            assert_edge(ctx, &pair[0], &pair[1].routine_id);
        }
        if w.last_steps.is_empty() {
            assert_edge(
                ctx,
                w.first_steps.last().unwrap(),
                &w.terminal_step.routine_id,
            );
        } else {
            for pair in w.last_steps.windows(2) {
                assert_edge(ctx, &pair[0], &pair[1].routine_id);
            }
            assert_edge(
                ctx,
                w.last_steps.last().unwrap(),
                &w.terminal_step.routine_id,
            );
        }
    }

    // === Deep fixture: hops > K+M — first-K + last-M + omitted ==============
    // (Setup is inlined per test, not factored into a shared helper: `graph`/
    // `seeds` borrow BOTH `ctx` and `workspace`, so a helper returning them
    // would have to return a self-referential struct — mirrors
    // `d1_dataflow::tests::origin_propagation_matches_chain_walk`'s own inline
    // setup for the same reason.)
    #[test]
    fn deep_reach_witness_first_k_last_m_omitted() {
        let (routines, graph_edges, summaries) = deep_reach_chain_fixture(9);
        let ctx = minimal_ctx(&routines, graph_edges, summaries);
        let workspace = ws(&routines);
        let mut memo = HashMap::new();
        let (graph, seeds) = build_d1_graph(&ctx, &workspace, &mut memo);
        assert_eq!(seeds.len(), 1, "fixture must have exactly one seed");
        let cw = ClosedWorldTempParams::new();
        let liveness = compute_liveness(&graph, &ctx, &cw);
        let scc = condense(&graph);
        let group = GroupSpec {
            loop_routine: seeds[0].loop_routine,
            loop_id: seeds[0].loop_id,
            loop_info: seeds[0].loop_info,
            seed_indices: vec![0],
            direct_indices: Vec::new(),
        };
        let solver = run_batch_fixpoint_for_test(
            &graph,
            &liveness,
            &scc,
            &seeds,
            &ctx,
            &cw,
            std::slice::from_ref(&group),
        );

        let t_node = graph.node_ix["T"];
        let reach_facts = &solver.reach_at[t_node as usize];
        assert_eq!(reach_facts.len(), 1, "one reach fact at T");
        let fact_ix = reach_facts[0];

        let t_routine = ctx.routine_by_id["T"];
        let t_op = &t_routine.record_operations[0];

        const K: usize = 3;
        const M: usize = 2;
        let w = representative_witness(
            &solver, &graph, &ctx, &seeds, 0, fact_ix, false, t_node, t_routine, t_op, K, M,
        );

        assert_eq!(w.total_hops, 10, "A -> C1..C9 -> T is 10 hops");
        // prefix (2) + K hop steps.
        assert_eq!(w.first_steps.len(), 2 + K);
        assert_eq!(w.first_steps[2].routine_id, "A");
        assert_eq!(w.first_steps[3].routine_id, "C1");
        assert_eq!(w.first_steps[4].routine_id, "C2");
        assert_eq!(w.last_steps.len(), M);
        assert_eq!(w.last_steps[0].routine_id, "C8");
        assert_eq!(w.last_steps[1].routine_id, "C9");
        assert_eq!(w.omitted_hops, (w.total_hops as usize - K - M) as u32);
        assert_eq!(w.omitted_hops, 5);

        assert_witness_valid(&w, &ctx, "R", "R/loop0", "T", "T/op0");
    }

    // === Shallow fixture: hops <= K+M — whole chain, omitted = 0 ============
    #[test]
    fn shallow_reach_witness_whole_chain_no_omission() {
        let (routines, graph_edges, summaries) = deep_reach_chain_fixture(1);
        let ctx = minimal_ctx(&routines, graph_edges, summaries);
        let workspace = ws(&routines);
        let mut memo = HashMap::new();
        let (graph, seeds) = build_d1_graph(&ctx, &workspace, &mut memo);
        assert_eq!(seeds.len(), 1, "fixture must have exactly one seed");
        let cw = ClosedWorldTempParams::new();
        let liveness = compute_liveness(&graph, &ctx, &cw);
        let scc = condense(&graph);
        let group = GroupSpec {
            loop_routine: seeds[0].loop_routine,
            loop_id: seeds[0].loop_id,
            loop_info: seeds[0].loop_info,
            seed_indices: vec![0],
            direct_indices: Vec::new(),
        };
        let solver = run_batch_fixpoint_for_test(
            &graph,
            &liveness,
            &scc,
            &seeds,
            &ctx,
            &cw,
            std::slice::from_ref(&group),
        );

        let t_node = graph.node_ix["T"];
        let reach_facts = &solver.reach_at[t_node as usize];
        assert_eq!(reach_facts.len(), 1, "one reach fact at T");
        let fact_ix = reach_facts[0];

        let t_routine = ctx.routine_by_id["T"];
        let t_op = &t_routine.record_operations[0];

        const K: usize = 3;
        const M: usize = 2;
        let w = representative_witness(
            &solver, &graph, &ctx, &seeds, 0, fact_ix, false, t_node, t_routine, t_op, K, M,
        );

        assert_eq!(w.total_hops, 2, "A -> C1 -> T is 2 hops");
        // Whole chain (2 hops) fits in first_steps: prefix (2) + 2 hop steps.
        assert_eq!(w.first_steps.len(), 4);
        assert_eq!(w.first_steps[2].routine_id, "A");
        assert_eq!(w.first_steps[3].routine_id, "C1");
        assert!(w.last_steps.is_empty(), "shallow case: last_steps empty");
        assert_eq!(w.omitted_hops, 0);

        assert_witness_valid(&w, &ctx, "R", "R/loop0", "T", "T/op0");
    }

    // === Value-fact terminal via a HopFromReach transition ===================
    #[test]
    fn deep_value_witness_crosses_hop_from_reach() {
        let (routines, graph_edges, summaries) = deep_value_chain_fixture();
        let ctx = minimal_ctx(&routines, graph_edges, summaries);
        let workspace = ws(&routines);
        let mut memo = HashMap::new();
        let (graph, seeds) = build_d1_graph(&ctx, &workspace, &mut memo);
        assert_eq!(seeds.len(), 1, "fixture must have exactly one seed");
        let cw = ClosedWorldTempParams::new();
        let liveness = compute_liveness(&graph, &ctx, &cw);
        let scc = condense(&graph);
        let group = GroupSpec {
            loop_routine: seeds[0].loop_routine,
            loop_id: seeds[0].loop_id,
            loop_info: seeds[0].loop_info,
            seed_indices: vec![0],
            direct_indices: Vec::new(),
        };
        let solver = run_batch_fixpoint_for_test(
            &graph,
            &liveness,
            &scc,
            &seeds,
            &ctx,
            &cw,
            std::slice::from_ref(&group),
        );

        let h_node = graph.node_ix["H"];
        let value_facts = &solver.value_at[h_node as usize];
        assert_eq!(value_facts.len(), 1, "one value fact at H (slot 0)");
        let fact_ix = value_facts[0];

        let h_routine = ctx.routine_by_id["H"];
        let h_op = &h_routine.record_operations[0];

        const K: usize = 2;
        const M: usize = 1;
        let w = representative_witness(
            &solver, &graph, &ctx, &seeds, 0, fact_ix, true, h_node, h_routine, h_op, K, M,
        );

        assert_eq!(w.total_hops, 4, "A -> B -> C -> D -> H is 4 hops");
        // prefix (2) + K=2 hop steps: A->B, B->C (the HopFromReach-crossing hop).
        assert_eq!(w.first_steps.len(), 2 + K);
        assert_eq!(w.first_steps[2].routine_id, "A");
        assert_eq!(w.first_steps[3].routine_id, "B");
        assert_eq!(w.last_steps.len(), M);
        assert_eq!(w.last_steps[0].routine_id, "D");
        assert_eq!(w.omitted_hops, 1, "the C->D hop is the omitted middle");

        assert_witness_valid(&w, &ctx, "R", "R/loop0", "H", "H/op0");
    }
}
