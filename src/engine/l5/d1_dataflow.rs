//! `d1_dataflow` — Task D2 of the d1 dataflow-solver redesign
//! (`.superpowers/sdd/task-d2-brief.md`,
//! `docs/superpowers/plans/2026-07-20-d1-dataflow-solver.md`): the single-group
//! FACT solver ([`solve_group`]), the correctness spine of the rewrite. It
//! reproduces [`crate::engine::l5::d1_reach::process_group`]'s six load-bearing
//! components — coverage, `reachable_verdicts`, severity, verdict,
//! `depth_bucket`, and `unc` — per (loop, terminal-op), while replacing the
//! joint-`TempVec` product-state BFS with independent per-parameter facts.
//! `process_group` stays ALIVE as the differential ORACLE (see the `tests`
//! module below); nothing wires `solve_group` into `detect_d1` yet.
//!
//! ## The fact model (single group = a 1-bit "mask"; D3 widens to u64/64 lanes)
//!
//! Because d1's temp propagation is UNARY (D1's `d1_liveness` proves it: every
//! callee param is a function of ≤1 caller param, every terminal reads ≤1
//! param), the joint `TempVec` (3^params multiplicity) is unnecessary. We track
//! two independent fact families over the filtered [`D1Graph`]:
//!
//! ```text
//! reach[node][depth 0..2][unc 0..1]                            present/absent
//! value[node][live-param-slot][Temp|Physical|Unknown][d][unc]  present/absent
//! ```
//!
//! `reach` is the transitive closure of (depth, unc) states — the SAME node set
//! and (depth, unc) reachability `process_group`'s label set realizes (its label
//! dedup is FINER — on the full `TempVec` — but the (node, depth, unc) *marginal*
//! is identical, and a terminal reads only ONE param, so the value-fact marginal
//! at the read slot equals `process_group`'s projected label set). `value`
//! carries, per live param, the resolved temp class along some realizing path.
//!
//! ## Propagation (level-synchronous → first arrival = shortest = min hops)
//!
//! Seed each seed-entry: `reach[entry][min(2, seed_depth)][entry_unc]` and, for
//! each live entry param, ONE value fact from
//! `cross_hop(root_state(loop_routine), seed callsite, …)` (identical to the
//! `process_group` seed label's own `TempVec`, projected to the live params). A
//! FIFO delta worklist then propagates to fixpoint — facts only ever GAIN
//! presence, so cycles terminate. Edge `n→m`: `d2 = min(2, d + loop_depth)`,
//! `u2 = u || node_unc[m]`. `reach` propagates directly; a callee live param's
//! value applies D1's compiled [`ParamTransfer`] — `Const(pt)` sets that class
//! (driven by the caller's REACH fact, since a constant ignores caller values),
//! `Copy{slot}` forwards the caller's value fact at that slot. FIFO order makes a
//! fact's first arrival its minimum hop count.
//!
//! ## Terminal scoring + winner selection (mirrors `process_group` exactly)
//!
//! A constant terminal (op `temp_state` non-PD) and a PROVEN PD terminal read
//! REACH (the class is caller-independent — `resolve_terminal` returns `Temp`
//! for a proven PD *without* reading any value; D1's `terminal_reads` over-reports
//! proven params, so the proven re-check here mirrors `resolve_terminal` EXACTLY);
//! a non-proven PD terminal reads the VALUE fact at its `terminal_reads` slot. Each
//! present fact yields one candidate — verdict via [`flowfield_verdict`], scoring
//! depth `min(2, fact.depth + local_depth)`, severity via `severity_for`, `hops`
//! = the fact's first-arrival hops. Direct ops enter the accumulator FIRST (as
//! today). The winner is the max under `d1_reach::selection_rank` (the identical
//! rule); its witness is materialized from the fact's first-arrival predecessor
//! chain (prepend `[loop_step, call_step]`, hop steps, terminal step; sum the
//! TRUE unclamped depth; union node uncertainties along the ONE witness).
//!
//! Only the witness / entry-callsite / reported true-depth / uncertainty vector /
//! first-discovery tie (component 7) MAY differ from `process_group` — a change
//! in components 1-6 is a BUG (proven absent by the `tests` differential below).
#![allow(dead_code)]

use std::collections::{BTreeMap, BinaryHeap, HashMap, VecDeque};

use crate::engine::l2::features::PLoop;
use crate::engine::l3::l3_workspace::{L3RecordOperation, L3Routine};
use crate::engine::l4::summary::{Uncertainty, dedupe_uncertainties};
use crate::engine::l5::closed_world_temp::ClosedWorldTempParams;
use crate::engine::l5::d1_graph::{D1Graph, D1Seed, D1Terminal, NodeIx, edge_kind_binding_ok};
use crate::engine::l5::d1_liveness::{Liveness, ParamTransfer};
use crate::engine::l5::d1_reach::{
    DirectOp, LoopTerminalAgg, call_step_ev, flowfield_verdict, loop_step_ev, node_has_uncertainty,
    selection_rank,
};
use crate::engine::l5::d1_temp::{
    ParamTemp, TempVec, cross_hop, lookup, resolve_terminal, root_state,
};
use crate::engine::l5::detector_context::DetectorContext;
use crate::engine::l5::detectors::d1::{
    TempVerdict, hop_step, is_setup_singleton_get, severity_for, terminal_step,
};
use crate::engine::l5::finding::EvidenceStep;

/// A reach fact's first-arrival predecessor (the seed it descends from, or the
/// parent reach fact + the crossed edge).
#[derive(Clone, Copy)]
enum ReachPred {
    Seed {
        seed_index: usize,
    },
    Hop {
        pred: usize,
        from_node: NodeIx,
        edge_k: usize,
    },
}

/// One `reach[node][depth][unc]` presence fact + its min-hop provenance.
struct ReachFact {
    node: NodeIx,
    depth: i64,
    unc: bool,
    hops: u32,
    pred: ReachPred,
}

/// A value fact's first-arrival predecessor. A `Const` transfer is born at a hop
/// from the caller's REACH fact (the class has no caller-value parent);
/// `Copy`/seed chain through value facts.
#[derive(Clone, Copy)]
enum ValuePred {
    Seed {
        seed_index: usize,
    },
    HopFromValue {
        pred: usize,
        from_node: NodeIx,
        edge_k: usize,
    },
    HopFromReach {
        pred: usize,
        from_node: NodeIx,
        edge_k: usize,
    },
}

/// One `value[node][slot][class][depth][unc]` presence fact + provenance.
struct ValueFact {
    node: NodeIx,
    slot: u16,
    class: ParamTemp,
    depth: i64,
    unc: bool,
    hops: u32,
    pred: ValuePred,
}

/// A worklist item — a newly-created fact whose outgoing propagation is pending.
enum WorkItem {
    Reach(usize),
    Value(usize),
}

/// A candidate's provenance — the witness source. A direct op carries its own
/// step inputs; a transitive candidate points back into a fact arena.
enum CandSource<'a> {
    Direct {
        routine: &'a L3Routine,
        loop_info: &'a PLoop,
        op: &'a L3RecordOperation,
    },
    TransReach {
        reach_fact: usize,
    },
    TransValue {
        value_fact: usize,
    },
}

/// One scored candidate for a (loop, terminal-op) bucket.
struct Candidate<'a> {
    verdict: TempVerdict,
    severity: &'static str,
    unc: bool,
    hops: u32,
    depth_bucket: i64,
    discovery: usize,
    source: CandSource<'a>,
    terminal_op: &'a L3RecordOperation,
    terminal_owner: &'a L3Routine,
    terminal_local_depth: i64,
}

/// The per-group fact solver's mutable state — the fact arenas, their dedup
/// indices, per-node fact lists (for scoring), and the delta worklist.
struct Solver {
    reach_facts: Vec<ReachFact>,
    value_facts: Vec<ValueFact>,
    reach_index: HashMap<(NodeIx, i64, bool), usize>,
    value_index: HashMap<(NodeIx, u16, ParamTemp, i64, bool), usize>,
    reach_at: Vec<Vec<usize>>,
    value_at: Vec<Vec<usize>>,
    queue: VecDeque<WorkItem>,
}

impl Solver {
    fn new(n_nodes: usize) -> Self {
        Solver {
            reach_facts: Vec::new(),
            value_facts: Vec::new(),
            reach_index: HashMap::new(),
            value_index: HashMap::new(),
            reach_at: vec![Vec::new(); n_nodes],
            value_at: vec![Vec::new(); n_nodes],
            queue: VecDeque::new(),
        }
    }

    /// Create `reach[node][depth][unc]` iff absent (first arrival wins the
    /// predecessor + min-hops), enqueueing it for propagation.
    fn ensure_reach(&mut self, node: NodeIx, depth: i64, unc: bool, hops: u32, pred: ReachPred) {
        let key = (node, depth, unc);
        if self.reach_index.contains_key(&key) {
            return;
        }
        let idx = self.reach_facts.len();
        self.reach_facts.push(ReachFact {
            node,
            depth,
            unc,
            hops,
            pred,
        });
        self.reach_index.insert(key, idx);
        self.reach_at[node as usize].push(idx);
        self.queue.push_back(WorkItem::Reach(idx));
    }

    /// Create `value[node][slot][class][depth][unc]` iff absent.
    #[allow(clippy::too_many_arguments)]
    fn ensure_value(
        &mut self,
        node: NodeIx,
        slot: u16,
        class: ParamTemp,
        depth: i64,
        unc: bool,
        hops: u32,
        pred: ValuePred,
    ) {
        let key = (node, slot, class, depth, unc);
        if self.value_index.contains_key(&key) {
            return;
        }
        let idx = self.value_facts.len();
        self.value_facts.push(ValueFact {
            node,
            slot,
            class,
            depth,
            unc,
            hops,
            pred,
        });
        self.value_index.insert(key, idx);
        self.value_at[node as usize].push(idx);
        self.queue.push_back(WorkItem::Value(idx));
    }
}

/// Walk a reach fact's first-arrival predecessor chain to its seed, collecting
/// the `(from_node, edge_k)` hops in TERMINAL->SEED order.
fn collect_reach_chain(reach_facts: &[ReachFact], start: usize) -> (Vec<(NodeIx, usize)>, usize) {
    let mut hops: Vec<(NodeIx, usize)> = Vec::new();
    let mut cur = start;
    loop {
        match reach_facts[cur].pred {
            ReachPred::Seed { seed_index } => return (hops, seed_index),
            ReachPred::Hop {
                pred,
                from_node,
                edge_k,
            } => {
                hops.push((from_node, edge_k));
                cur = pred;
            }
        }
    }
}

/// Walk a value fact's first-arrival predecessor chain. A `HopFromReach` (the
/// hop that BORN this class via a `Const` transfer) switches the walk onto the
/// caller's reach chain for the remainder.
fn collect_value_chain(
    value_facts: &[ValueFact],
    reach_facts: &[ReachFact],
    start: usize,
) -> (Vec<(NodeIx, usize)>, usize) {
    let mut hops: Vec<(NodeIx, usize)> = Vec::new();
    let mut cur = start;
    loop {
        match value_facts[cur].pred {
            ValuePred::Seed { seed_index } => return (hops, seed_index),
            ValuePred::HopFromValue {
                pred,
                from_node,
                edge_k,
            } => {
                hops.push((from_node, edge_k));
                cur = pred;
            }
            ValuePred::HopFromReach {
                pred,
                from_node,
                edge_k,
            } => {
                hops.push((from_node, edge_k));
                let (mut rest, seed_index) = collect_reach_chain(reach_facts, pred);
                hops.append(&mut rest);
                return (hops, seed_index);
            }
        }
    }
}

/// Materialize a transitive winner's witness from its first-arrival hop chain
/// (`hops` in terminal->seed order): `[loop_step, call_step]` + one `hop_step`
/// per edge (seed->terminal order) + the terminal step; the union of node
/// uncertainties along THIS path; the entry callsite; and the TRUE (unclamped)
/// effective depth `seed_depth + Σ edge.loop_depth + local_depth`.
#[allow(clippy::too_many_arguments)]
fn build_transitive_witness<'a>(
    hops: &[(NodeIx, usize)],
    seed_index: usize,
    terminal_node: NodeIx,
    terminal_owner: &'a L3Routine,
    terminal_op: &'a L3RecordOperation,
    terminal_local_depth: i64,
    graph: &D1Graph<'a>,
    ctx: &DetectorContext,
    seeds: &[D1Seed<'a>],
) -> (Vec<EvidenceStep>, Vec<Uncertainty>, Option<&'a str>, i64) {
    let seed = &seeds[seed_index];
    let mut witness: Vec<EvidenceStep> = Vec::with_capacity(hops.len() + 3);
    witness.push(loop_step_ev(seed.loop_routine, seed.loop_info));
    witness.push(call_step_ev(seed, graph, ctx));

    // hops in seed->terminal order (reverse of the terminal->seed walk).
    let mut sum_edges = 0i64;
    let mut path_nodes: Vec<NodeIx> = Vec::with_capacity(hops.len() + 1);
    for (from_node, edge_k) in hops.iter().rev() {
        let edge = &graph.edges[*from_node as usize][*edge_k];
        let from_id = graph.node_ids[*from_node as usize];
        let to_id = graph.node_ids[edge.to as usize];
        witness.push(hop_step(
            &ctx.routine_by_id,
            from_id,
            to_id,
            edge.kind,
            edge.callsite_id,
        ));
        sum_edges += edge.loop_depth;
        path_nodes.push(*from_node);
    }
    path_nodes.push(terminal_node);
    witness.push(terminal_step(
        &ctx.routine_by_id,
        &ctx.table_by_id,
        terminal_owner.id.as_str(),
        Some(terminal_op.id.as_str()),
    ));

    // Uncertainty union along the witness (seed -> terminal node order).
    let mut concat: Vec<Uncertainty> = Vec::new();
    for &n in &path_nodes {
        let nid = graph.node_ids[n as usize];
        if let Some(v) = ctx.uncertainties_by_node.get(nid) {
            concat.extend(v.iter().cloned());
        }
    }
    let uncertainties = dedupe_uncertainties(concat);
    let effective = seed.seed_depth + sum_edges + terminal_local_depth;
    (
        witness,
        uncertainties,
        Some(seed.callsite.id.as_str()),
        effective,
    )
}

/// Solve ONE loop group with the fact model; returns the SAME `Vec<LoopTerminalAgg>`
/// `process_group` returns (components 1-6 identical; witness may differ).
#[allow(clippy::too_many_arguments)]
pub(crate) fn solve_group<'a>(
    graph: &D1Graph<'a>,
    liveness: &Liveness,
    seeds: &[D1Seed<'a>],
    direct_ops: &[DirectOp<'a>],
    ctx: &'a DetectorContext,
    cw: &ClosedWorldTempParams,
    loop_routine: &'a L3Routine,
    loop_id: &'a str,
    loop_info: &'a PLoop,
    seed_indices: &[usize],
    direct_indices: &[usize],
) -> Vec<LoopTerminalAgg<'a>> {
    let n_nodes = graph.node_ids.len();
    let mut solver = Solver::new(n_nodes);

    // Per-node scalars used repeatedly during propagation.
    let unc_by_node: Vec<bool> = graph
        .node_ids
        .iter()
        .map(|id| node_has_uncertainty(ctx, id))
        .collect();
    // A node with no `routine_by_id` entry carries no composable bindings, so it
    // is a dead end — never expanded (mirrors `process_group`'s defensive skip).
    let expandable: Vec<bool> = graph
        .node_ids
        .iter()
        .map(|id| ctx.routine_by_id.contains_key(id))
        .collect();

    // Rule 1: seed the frontier (in seed order). The seed's entry `TempVec` is
    // computed EXACTLY as `process_group`'s seed label — `cross_hop` of the loop
    // routine's root state across the seed callsite — then projected to the
    // entry's live params (D1's `Need[entry]`).
    let root = root_state(loop_routine.id.as_str(), cw);
    for &si in seed_indices {
        let seed = &seeds[si];
        let entry = seed.entry;
        let entry_id = graph.node_ids[entry as usize];
        let binding_ok = edge_kind_binding_ok(seed.entry_edge_kind);
        let entry_temp = cross_hop(
            &root,
            seed.loop_routine,
            seed.callsite.id.as_str(),
            entry_id,
            binding_ok,
            cw,
        );
        let depth = seed.seed_depth.min(2);
        let unc = unc_by_node[entry as usize];
        solver.ensure_reach(entry, depth, unc, 0, ReachPred::Seed { seed_index: si });
        for (slot, &p) in liveness.need[entry as usize].iter().enumerate() {
            let class = lookup(&entry_temp, p);
            solver.ensure_value(
                entry,
                slot as u16,
                class,
                depth,
                unc,
                0,
                ValuePred::Seed { seed_index: si },
            );
        }
    }

    // Rules 2-3: FIFO delta propagation to fixpoint. Reach facts drive reach +
    // `Const` value propagation; value facts drive `Copy` value propagation.
    while let Some(item) = solver.queue.pop_front() {
        match item {
            WorkItem::Reach(ri) => {
                let (node, depth, unc, hops) = {
                    let f = &solver.reach_facts[ri];
                    (f.node, f.depth, f.unc, f.hops)
                };
                if !expandable[node as usize] {
                    continue;
                }
                for (k, edge) in graph.edges[node as usize].iter().enumerate() {
                    let m = edge.to;
                    let d2 = (depth + edge.loop_depth).min(2);
                    let u2 = unc || unc_by_node[m as usize];
                    solver.ensure_reach(
                        m,
                        d2,
                        u2,
                        hops + 1,
                        ReachPred::Hop {
                            pred: ri,
                            from_node: node,
                            edge_k: k,
                        },
                    );
                    for (callee_slot, transfer) in
                        liveness.edge_transfers[node as usize][k].iter().enumerate()
                    {
                        if let ParamTransfer::Const(pt) = transfer {
                            solver.ensure_value(
                                m,
                                callee_slot as u16,
                                *pt,
                                d2,
                                u2,
                                hops + 1,
                                ValuePred::HopFromReach {
                                    pred: ri,
                                    from_node: node,
                                    edge_k: k,
                                },
                            );
                        }
                    }
                }
            }
            WorkItem::Value(vi) => {
                let (node, slot, class, depth, unc, hops) = {
                    let f = &solver.value_facts[vi];
                    (f.node, f.slot, f.class, f.depth, f.unc, f.hops)
                };
                if !expandable[node as usize] {
                    continue;
                }
                for (k, edge) in graph.edges[node as usize].iter().enumerate() {
                    let m = edge.to;
                    let d2 = (depth + edge.loop_depth).min(2);
                    let u2 = unc || unc_by_node[m as usize];
                    for (callee_slot, transfer) in
                        liveness.edge_transfers[node as usize][k].iter().enumerate()
                    {
                        if let ParamTransfer::Copy { caller_slot } = transfer
                            && *caller_slot == slot
                        {
                            solver.ensure_value(
                                m,
                                callee_slot as u16,
                                class,
                                d2,
                                u2,
                                hops + 1,
                                ValuePred::HopFromValue {
                                    pred: vi,
                                    from_node: node,
                                    edge_k: k,
                                },
                            );
                        }
                    }
                }
            }
        }
    }

    // Rules 4/6: generate candidates. Direct ops first (branch (a) precedence),
    // then transitive candidates. `discovery` is a per-group monotonic counter.
    let mut buckets: BTreeMap<(&'a str, &'a str), Vec<Candidate<'a>>> = BTreeMap::new();
    let mut discovery = 0usize;

    for &di in direct_indices {
        let d = &direct_ops[di];
        let op = d.op;
        let owner = d.routine;
        let base_pt = resolve_terminal(op, &root, owner.id.as_str(), cw);
        let verdict = flowfield_verdict(base_pt, op, &ctx.table_by_id);
        let local_depth = op.loop_stack.len() as i64;
        let depth_bucket = local_depth.min(2);
        let is_singleton = is_setup_singleton_get(op, Some(owner), &ctx.table_by_id);
        let severity = severity_for(op, verdict, depth_bucket, is_singleton);
        buckets
            .entry((owner.id.as_str(), op.id.as_str()))
            .or_default()
            .push(Candidate {
                verdict,
                severity,
                unc: false,
                hops: 0,
                depth_bucket,
                discovery,
                source: CandSource::Direct {
                    routine: owner,
                    loop_info: d.loop_info,
                    op,
                },
                terminal_op: op,
                terminal_owner: owner,
                terminal_local_depth: local_depth,
            });
        discovery += 1;
    }

    // Transitive candidates: one per present fact reaching each terminal. A
    // constant terminal (non-PD op) and a PROVEN PD terminal read REACH facts
    // (caller-independent verdict); a non-proven PD terminal reads the VALUE
    // fact at its `terminal_reads` slot. The proven re-check mirrors
    // `resolve_terminal` EXACTLY (D1's `terminal_reads` over-reports proven).
    for node in 0..n_nodes as NodeIx {
        let terminals = &graph.terminals[node as usize];
        for (ti, t) in terminals.iter().enumerate() {
            let op = t.op;
            let owner = t.owner;
            let local_depth = t.local_depth;
            let is_singleton = is_setup_singleton_get(op, Some(owner), &ctx.table_by_id);

            // Resolve read mode: `Some(slot)` = value-based; `None` = reach-based.
            let value_slot: Option<u16> = match liveness.terminal_reads[node as usize][ti] {
                None => None,
                Some(slot) => {
                    let i = liveness.need[node as usize][slot as usize];
                    if cw.contains(&(owner.id.to_string(), i)) {
                        None // proven -> constant Temp verdict, read reach
                    } else {
                        Some(slot)
                    }
                }
            };

            match value_slot {
                None => {
                    // Reach-based: verdict is caller-independent (constant op, or
                    // a proven PD -> `resolve_terminal` returns Temp without a read).
                    let verdict = flowfield_verdict(
                        resolve_terminal(op, &TempVec::new(), owner.id.as_str(), cw),
                        op,
                        &ctx.table_by_id,
                    );
                    for &ri in &solver.reach_at[node as usize] {
                        let f = &solver.reach_facts[ri];
                        let depth_bucket = (f.depth + local_depth).min(2);
                        let severity = severity_for(op, verdict, depth_bucket, is_singleton);
                        buckets
                            .entry((owner.id.as_str(), op.id.as_str()))
                            .or_default()
                            .push(Candidate {
                                verdict,
                                severity,
                                unc: f.unc,
                                hops: f.hops,
                                depth_bucket,
                                discovery,
                                source: CandSource::TransReach { reach_fact: ri },
                                terminal_op: op,
                                terminal_owner: owner,
                                terminal_local_depth: local_depth,
                            });
                        discovery += 1;
                    }
                }
                Some(slot) => {
                    let i = liveness.need[node as usize][slot as usize];
                    for &vi in &solver.value_at[node as usize] {
                        let f = &solver.value_facts[vi];
                        if f.slot != slot {
                            continue;
                        }
                        let frame: TempVec = std::iter::once((i, f.class)).collect();
                        let verdict = flowfield_verdict(
                            resolve_terminal(op, &frame, owner.id.as_str(), cw),
                            op,
                            &ctx.table_by_id,
                        );
                        let depth_bucket = (f.depth + local_depth).min(2);
                        let severity = severity_for(op, verdict, depth_bucket, is_singleton);
                        buckets
                            .entry((owner.id.as_str(), op.id.as_str()))
                            .or_default()
                            .push(Candidate {
                                verdict,
                                severity,
                                unc: f.unc,
                                hops: f.hops,
                                depth_bucket,
                                discovery,
                                source: CandSource::TransValue { value_fact: vi },
                                terminal_op: op,
                                terminal_owner: owner,
                                terminal_local_depth: local_depth,
                            });
                        discovery += 1;
                    }
                }
            }
        }
    }

    // Rules 5/7: per bucket (already (owner id, op id)-sorted by the BTreeMap),
    // select the winner and materialize its witness.
    let mut out: Vec<LoopTerminalAgg<'a>> = Vec::new();
    for (_key, cands) in buckets {
        let mut reachable_verdicts: Vec<TempVerdict> = cands.iter().map(|c| c.verdict).collect();
        reachable_verdicts.sort();
        reachable_verdicts.dedup();

        let winner = cands
            .iter()
            .max_by_key(|c| {
                selection_rank(
                    c.severity,
                    c.verdict,
                    c.unc,
                    c.hops,
                    c.depth_bucket,
                    c.discovery,
                )
            })
            .expect("a bucket is never empty");

        let (witness, uncertainties, entry_callsite_id, effective_loop_depth) = match &winner.source
        {
            CandSource::Direct {
                routine,
                loop_info,
                op,
            } => {
                let loop_step = loop_step_ev(routine, loop_info);
                let op_step = terminal_step(
                    &ctx.routine_by_id,
                    &ctx.table_by_id,
                    routine.id.as_str(),
                    Some(op.id.as_str()),
                );
                (
                    vec![loop_step, op_step],
                    Vec::new(),
                    None,
                    winner.terminal_local_depth,
                )
            }
            CandSource::TransReach { reach_fact } => {
                let terminal_node = solver.reach_facts[*reach_fact].node;
                let (hops, seed_index) = collect_reach_chain(&solver.reach_facts, *reach_fact);
                build_transitive_witness(
                    &hops,
                    seed_index,
                    terminal_node,
                    winner.terminal_owner,
                    winner.terminal_op,
                    winner.terminal_local_depth,
                    graph,
                    ctx,
                    seeds,
                )
            }
            CandSource::TransValue { value_fact } => {
                let terminal_node = solver.value_facts[*value_fact].node;
                let (hops, seed_index) =
                    collect_value_chain(&solver.value_facts, &solver.reach_facts, *value_fact);
                build_transitive_witness(
                    &hops,
                    seed_index,
                    terminal_node,
                    winner.terminal_owner,
                    winner.terminal_op,
                    winner.terminal_local_depth,
                    graph,
                    ctx,
                    seeds,
                )
            }
        };

        out.push(LoopTerminalAgg {
            loop_routine,
            loop_id,
            loop_info,
            terminal: D1Terminal {
                op: winner.terminal_op,
                owner: winner.terminal_owner,
                local_depth: winner.terminal_local_depth,
            },
            entry_callsite_id,
            severity: winner.severity,
            verdict: winner.verdict,
            reachable_verdicts,
            depth_bucket: winner.depth_bucket,
            effective_loop_depth,
            witness,
            uncertainties,
        });
    }
    out
}

// ===========================================================================
// Task D3 — the BATCH driver: 64-lane group bitsets + call-SCC condensation
// scheduler. `solve_batch` solves up to 64 loop groups sharing ONE traversal of
// the (dense 797-member) call SCC: the D2 single-group fact model widened from a
// 1-bit "mask" to a `u64` group-mask (group i in the batch owns bit i). This is
// where the dataflow win materializes — the SCC is threaded once per BATCH, not
// once per group.
//
// ## Correctness (why the shared traversal is exact)
//
// Per gpt's design (memory note `d1-output-bound-falsified` §2026-07-20): d1's
// temp propagation is UNARY, so the GROUP-SETS realizing each per-parameter fact
// are union-only-monotone (bits are only ever OR-ed in, never cleared) even
// though temp VALUES are not. A standard least-fixpoint over the ORIGINAL call
// SCC therefore reproduces, PER LANE, the identical (node, depth, unc, slot,
// class) fact set — hence identical coverage / reachable_verdicts / severity /
// verdict / depth_bucket / unc — that `solve_group` (and thus `process_group`)
// produces for that group. Only the WITNESS / entry-callsite / equal-ranked tie
// (component 7) may differ, exactly as at D2.
//
// ## Scheduling + provenance
//
// Facts propagate FORWARD (caller -> callee). We process the call-graph SCC
// condensation in TOPOLOGICAL order (callers before callees); cross-SCC
// arrivals are buffered per downstream SCC and, within each SCC, drained by a
// min-hops-first (level-synchronous) delta worklist so a lane's FIRST arrival at
// a fact is its minimum hop count (= shortest witness). Provenance is per
// (fact, lane): the first-arrival predecessor (the seed it descends from, or the
// parent fact + crossed edge), stored in per-lane arrays and walked at scoring
// time to materialize that lane's winning witness. The whole batch arena is
// dropped after emitting its aggregates (the memory bound — serial batches, no
// concurrent arenas).

/// Batch width — how many loop groups share one condensation pass. Group `i` in
/// the batch owns bit `i` of the `u64` lane masks. (D6 may lower this — 32/16 —
/// if the dense-SCC batch arena exceeds the RSS headroom.)
pub(crate) const BATCH_WIDTH: usize = 64;

/// One loop group in a batch: the (loop routine, loop id) identity plus the seed
/// and direct-op indices (into the shared `seeds`/`direct_ops` slices) that
/// belong to it. `search_loops` assigns groups to lanes in the existing sorted
/// `(loop_routine_id, loop_id)` order.
pub(crate) struct GroupSpec<'a> {
    pub loop_routine: &'a L3Routine,
    pub loop_id: &'a str,
    pub loop_info: &'a PLoop,
    pub seed_indices: Vec<usize>,
    pub direct_indices: Vec<usize>,
}

/// The call-graph SCC condensation over the filtered [`D1Graph`]. Deterministic:
/// node iteration (0..n) and per-node edge order are both fixed, so Tarjan's
/// emission order — and hence every field here — is a pure function of the graph.
pub(crate) struct CallScc {
    /// `NodeIx` -> its SCC id (an index into `members`/`topo_order`, in
    /// topological numbering: a caller SCC has a lower id than its callees').
    pub scc_of: Vec<u32>,
    /// SCC id -> its member nodes, sorted ascending by `NodeIx`.
    pub members: Vec<Vec<NodeIx>>,
    /// SCC ids in TOPOLOGICAL order (callers before callees). Since ids are
    /// assigned in topological order, this is simply `0..members.len()`, but it
    /// is materialized so a caller iterates schedule order without assuming the
    /// numbering convention.
    pub topo_order: Vec<u32>,
}

/// One explicit Tarjan work-stack frame (node + next-child cursor) — no
/// recursion (AL call graphs can be deep). Mirrors `l4::scc::Frame`.
struct SccFrame {
    node: NodeIx,
    child: usize,
}

/// Tarjan's SCC over the filtered [`D1Graph`], iterative + deterministic.
///
/// This is a DENSE (`NodeIx`-indexed) port of the engine's existing iterative
/// Tarjan, `crate::engine::l4::scc::tarjan_scc` — same explicit-work-stack
/// shape, same reverse-topological emission — but that one operates on
/// `String`-keyed `SccInputGraph`s (it runs over al-sem's combined graph), which
/// would force a full String-adjacency rebuild + a re-map back to `NodeIx` on
/// every call; the `D1Graph` is already interned to dense `NodeIx`, so a direct
/// port keeps the index alignment the solver relies on. Tarjan emits SCCs in
/// REVERSE-topological order (callees before callers); we reverse that to number
/// SCCs (and `topo_order`) callers-first — the order forward fact propagation
/// wants.
pub(crate) fn condense(graph: &D1Graph) -> CallScc {
    let n = graph.node_ids.len();
    const UNVISITED: u32 = u32::MAX;
    let mut index = vec![UNVISITED; n];
    let mut lowlink = vec![0u32; n];
    let mut on_stack = vec![false; n];
    let mut tarjan_stack: Vec<NodeIx> = Vec::new();
    let mut next_index = 0u32;
    // Raw SCCs in Tarjan's natural REVERSE-topological emission order.
    let mut raw_sccs: Vec<Vec<NodeIx>> = Vec::new();

    for start in 0..n as NodeIx {
        if index[start as usize] != UNVISITED {
            continue;
        }
        let mut work: Vec<SccFrame> = vec![SccFrame {
            node: start,
            child: 0,
        }];
        while !work.is_empty() {
            let top = work.len() - 1;
            let node = work[top].node;
            let child = work[top].child;

            if child == 0 {
                index[node as usize] = next_index;
                lowlink[node as usize] = next_index;
                next_index += 1;
                tarjan_stack.push(node);
                on_stack[node as usize] = true;
            }

            let edges = &graph.edges[node as usize];
            if child < edges.len() {
                work[top].child += 1;
                let to = edges[child].to;
                if index[to as usize] == UNVISITED {
                    work.push(SccFrame { node: to, child: 0 });
                } else if on_stack[to as usize] {
                    let cur = lowlink[node as usize];
                    lowlink[node as usize] = cur.min(index[to as usize]);
                }
                continue;
            }

            // All children explored — settle this node.
            if lowlink[node as usize] == index[node as usize] {
                let mut members: Vec<NodeIx> = Vec::new();
                loop {
                    let w = tarjan_stack.pop().expect("Tarjan stack underflow");
                    on_stack[w as usize] = false;
                    members.push(w);
                    if w == node {
                        break;
                    }
                }
                raw_sccs.push(members);
            }
            work.pop();
            if let Some(parent) = work.last() {
                let pn = parent.node;
                let cur = lowlink[pn as usize];
                lowlink[pn as usize] = cur.min(lowlink[node as usize]);
            }
        }
    }

    // raw_sccs is in reverse-topological order (callees before callers); reverse
    // it so SCC id / topo_order run callers-first (forward propagation order).
    let mut scc_of = vec![u32::MAX; n];
    let mut members: Vec<Vec<NodeIx>> = Vec::with_capacity(raw_sccs.len());
    for raw in raw_sccs.iter().rev() {
        let scc_id = members.len() as u32;
        let mut sorted = raw.clone();
        sorted.sort_unstable();
        for &m in &sorted {
            scc_of[m as usize] = scc_id;
        }
        members.push(sorted);
    }
    let topo_order: Vec<u32> = (0..members.len() as u32).collect();
    CallScc {
        scc_of,
        members,
        topo_order,
    }
}

/// A reach fact's per-lane first-arrival predecessor (the seed it descends from,
/// or the parent reach fact + the crossed edge). `None` = the lane is not
/// present at this fact.
#[derive(Clone, Copy)]
enum ReachPredB {
    None,
    Seed {
        seed_index: u32,
    },
    Hop {
        pred: u32,
        from_node: NodeIx,
        edge_k: u32,
    },
}

/// A value fact's per-lane first-arrival predecessor. `HopFromReach` is the hop
/// that BORN this class via a `Const` transfer (its parent is a reach fact);
/// `HopFromValue` / `Seed` chain through value facts (mirrors D2's `ValuePred`).
#[derive(Clone, Copy)]
enum ValuePredB {
    None,
    Seed {
        seed_index: u32,
    },
    HopFromValue {
        pred: u32,
        from_node: NodeIx,
        edge_k: u32,
    },
    HopFromReach {
        pred: u32,
        from_node: NodeIx,
        edge_k: u32,
    },
}

/// One `reach[node][depth][unc]` fact shared across the batch's ≤64 lanes: the
/// key + the lane presence `mask`. Per-lane provenance (hops + predecessor)
/// lives in the parallel `reach_hops`/`reach_pred` arrays (indexed by fact id).
struct ReachFactB {
    node: NodeIx,
    depth: i64,
    unc: bool,
    mask: u64,
}

/// One `value[node][slot][class][depth][unc]` fact shared across the lanes.
struct ValueFactB {
    node: NodeIx,
    slot: u16,
    class: ParamTemp,
    depth: i64,
    unc: bool,
    mask: u64,
}

/// A pending arrival to a fact key: the newly-arriving lane `mask` at `hops`,
/// plus the predecessor to record for those lanes. Routed to the target SCC's
/// worklist (same SCC) or its pending buffer (a downstream SCC).
enum Proposal {
    Reach {
        node: NodeIx,
        depth: i64,
        unc: bool,
        mask: u64,
        hops: u32,
        pred: ReachPredB,
    },
    Value {
        node: NodeIx,
        slot: u16,
        class: ParamTemp,
        depth: i64,
        unc: bool,
        mask: u64,
        hops: u32,
        pred: ValuePredB,
    },
}

impl Proposal {
    fn node(&self) -> NodeIx {
        match self {
            Proposal::Reach { node, .. } | Proposal::Value { node, .. } => *node,
        }
    }
    fn hops(&self) -> u32 {
        match self {
            Proposal::Reach { hops, .. } | Proposal::Value { hops, .. } => *hops,
        }
    }
}

/// A min-hops-first worklist item for the current SCC's level-synchronous
/// drain. Ordered by `(hops, seq)` ascending (via a reversed `Ord` over the
/// max-heap); `seq` is a monotonic push counter making the within-hop order
/// deterministic (it only affects an equal-ranked tie — component 7).
struct HeapItem {
    hops: u32,
    seq: u64,
    prop: Proposal,
}

impl PartialEq for HeapItem {
    fn eq(&self, other: &Self) -> bool {
        self.hops == other.hops && self.seq == other.seq
    }
}
impl Eq for HeapItem {}
impl PartialOrd for HeapItem {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for HeapItem {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // `BinaryHeap` is a MAX-heap; reverse so the SMALLEST (hops, seq) pops
        // first (min-hops-first = level-synchronous).
        other
            .hops
            .cmp(&self.hops)
            .then_with(|| other.seq.cmp(&self.seq))
    }
}

/// The batch fact solver's shared arenas (across all lanes in the batch), their
/// dedup indices, per-node fact lists (for scoring), and per-lane provenance.
struct BatchSolver {
    reach_facts: Vec<ReachFactB>,
    value_facts: Vec<ValueFactB>,
    reach_hops: Vec<[u32; BATCH_WIDTH]>,
    reach_pred: Vec<[ReachPredB; BATCH_WIDTH]>,
    value_hops: Vec<[u32; BATCH_WIDTH]>,
    value_pred: Vec<[ValuePredB; BATCH_WIDTH]>,
    reach_index: HashMap<(NodeIx, i64, bool), usize>,
    value_index: HashMap<(NodeIx, u16, ParamTemp, i64, bool), usize>,
    reach_at: Vec<Vec<usize>>,
    value_at: Vec<Vec<usize>>,
}

impl BatchSolver {
    fn new(n_nodes: usize) -> Self {
        BatchSolver {
            reach_facts: Vec::new(),
            value_facts: Vec::new(),
            reach_hops: Vec::new(),
            reach_pred: Vec::new(),
            value_hops: Vec::new(),
            value_pred: Vec::new(),
            reach_index: HashMap::new(),
            value_index: HashMap::new(),
            reach_at: vec![Vec::new(); n_nodes],
            value_at: vec![Vec::new(); n_nodes],
        }
    }

    /// Commit an incoming reach arrival: create the fact key if absent, then set
    /// the lanes that are NEWLY present (`mask & !fact.mask`) — first arrival
    /// wins, recording each new lane's `hops` + `pred`. Returns `(fact idx, new
    /// bits)`; `new bits == 0` means nothing to propagate.
    fn commit_reach(
        &mut self,
        node: NodeIx,
        depth: i64,
        unc: bool,
        mask: u64,
        hops: u32,
        pred: ReachPredB,
    ) -> (usize, u64) {
        let key = (node, depth, unc);
        let idx = match self.reach_index.get(&key) {
            Some(&i) => i,
            None => {
                let i = self.reach_facts.len();
                self.reach_facts.push(ReachFactB {
                    node,
                    depth,
                    unc,
                    mask: 0,
                });
                self.reach_hops.push([0u32; BATCH_WIDTH]);
                self.reach_pred.push([ReachPredB::None; BATCH_WIDTH]);
                self.reach_at[node as usize].push(i);
                self.reach_index.insert(key, i);
                i
            }
        };
        let fact = &mut self.reach_facts[idx];
        let new_bits = mask & !fact.mask;
        if new_bits == 0 {
            return (idx, 0);
        }
        fact.mask |= new_bits;
        let hops_arr = &mut self.reach_hops[idx];
        let pred_arr = &mut self.reach_pred[idx];
        let mut m = new_bits;
        while m != 0 {
            let lane = m.trailing_zeros() as usize;
            hops_arr[lane] = hops;
            pred_arr[lane] = pred;
            m &= m - 1;
        }
        (idx, new_bits)
    }

    /// Commit an incoming value arrival (see [`Self::commit_reach`]).
    #[allow(clippy::too_many_arguments)]
    fn commit_value(
        &mut self,
        node: NodeIx,
        slot: u16,
        class: ParamTemp,
        depth: i64,
        unc: bool,
        mask: u64,
        hops: u32,
        pred: ValuePredB,
    ) -> (usize, u64) {
        let key = (node, slot, class, depth, unc);
        let idx = match self.value_index.get(&key) {
            Some(&i) => i,
            None => {
                let i = self.value_facts.len();
                self.value_facts.push(ValueFactB {
                    node,
                    slot,
                    class,
                    depth,
                    unc,
                    mask: 0,
                });
                self.value_hops.push([0u32; BATCH_WIDTH]);
                self.value_pred.push([ValuePredB::None; BATCH_WIDTH]);
                self.value_at[node as usize].push(i);
                self.value_index.insert(key, i);
                i
            }
        };
        let fact = &mut self.value_facts[idx];
        let new_bits = mask & !fact.mask;
        if new_bits == 0 {
            return (idx, 0);
        }
        fact.mask |= new_bits;
        let hops_arr = &mut self.value_hops[idx];
        let pred_arr = &mut self.value_pred[idx];
        let mut m = new_bits;
        while m != 0 {
            let lane = m.trailing_zeros() as usize;
            hops_arr[lane] = hops;
            pred_arr[lane] = pred;
            m &= m - 1;
        }
        (idx, new_bits)
    }
}

/// Route a generated proposal to the current SCC's min-hops worklist (target in
/// the SAME SCC) or the target SCC's pending buffer (a downstream SCC — the call
/// SCC condensation is a DAG once condensed, so a proposal never targets an
/// already-settled upstream SCC).
fn route(
    prop: Proposal,
    current_scc: u32,
    scc_of: &[u32],
    heap: &mut BinaryHeap<HeapItem>,
    pending: &mut [Vec<Proposal>],
    seq: &mut u64,
) {
    let target = scc_of[prop.node() as usize];
    if target == current_scc {
        let hops = prop.hops();
        let s = *seq;
        *seq += 1;
        heap.push(HeapItem { hops, seq: s, prop });
    } else {
        // Topological invariant: a cross-SCC edge only ever targets a STRICTLY
        // downstream (not-yet-drained) SCC (`scc_of[caller] < scc_of[callee]`).
        // If a future `condense` regression broke the numbering, this would
        // otherwise SILENTLY drop the proposal into an already-drained upstream
        // pending buffer — fail loudly instead.
        debug_assert!(
            target > current_scc,
            "route: proposal targets SCC {target} <= current {current_scc} — \
             condense produced a non-topological order (upstream pending is drained)"
        );
        pending[target as usize].push(prop);
    }
}

/// Walk a reach fact's per-lane first-arrival predecessor chain to its seed,
/// collecting the `(from_node, edge_k)` hops in TERMINAL->SEED order (the batch
/// analogue of [`collect_reach_chain`], reading the per-lane predecessor).
fn collect_reach_chain_b(
    reach_pred: &[[ReachPredB; BATCH_WIDTH]],
    lane: usize,
    start: usize,
) -> (Vec<(NodeIx, usize)>, usize) {
    let mut hops: Vec<(NodeIx, usize)> = Vec::new();
    let mut cur = start;
    loop {
        match reach_pred[cur][lane] {
            ReachPredB::Seed { seed_index } => return (hops, seed_index as usize),
            ReachPredB::Hop {
                pred,
                from_node,
                edge_k,
            } => {
                hops.push((from_node, edge_k as usize));
                cur = pred as usize;
            }
            ReachPredB::None => unreachable!("a present lane always has a reach predecessor"),
        }
    }
}

/// Walk a value fact's per-lane first-arrival predecessor chain (the batch
/// analogue of [`collect_value_chain`]); a `HopFromReach` switches the walk onto
/// the caller's reach chain for the remainder.
fn collect_value_chain_b(
    value_pred: &[[ValuePredB; BATCH_WIDTH]],
    reach_pred: &[[ReachPredB; BATCH_WIDTH]],
    lane: usize,
    start: usize,
) -> (Vec<(NodeIx, usize)>, usize) {
    let mut hops: Vec<(NodeIx, usize)> = Vec::new();
    let mut cur = start;
    loop {
        match value_pred[cur][lane] {
            ValuePredB::Seed { seed_index } => return (hops, seed_index as usize),
            ValuePredB::HopFromValue {
                pred,
                from_node,
                edge_k,
            } => {
                hops.push((from_node, edge_k as usize));
                cur = pred as usize;
            }
            ValuePredB::HopFromReach {
                pred,
                from_node,
                edge_k,
            } => {
                hops.push((from_node, edge_k as usize));
                let (mut rest, seed_index) = collect_reach_chain_b(reach_pred, lane, pred as usize);
                hops.append(&mut rest);
                return (hops, seed_index);
            }
            ValuePredB::None => unreachable!("a present lane always has a value predecessor"),
        }
    }
}

/// Solve a BATCH of up to [`BATCH_WIDTH`] loop groups sharing ONE call-SCC
/// condensation pass. Group `i` in `batch` owns bit `i` of the `u64` lane masks.
/// Returns one [`LoopTerminalAgg`] per (group, terminal-op) — components 1-6
/// identical to `solve_group` per group; witness (component 7) may differ.
#[allow(clippy::too_many_arguments)]
pub(crate) fn solve_batch<'a>(
    graph: &D1Graph<'a>,
    liveness: &Liveness,
    scc: &CallScc,
    seeds: &[D1Seed<'a>],
    direct_ops: &[DirectOp<'a>],
    ctx: &'a DetectorContext,
    cw: &ClosedWorldTempParams,
    batch: &[GroupSpec<'a>],
) -> Vec<LoopTerminalAgg<'a>> {
    assert!(
        batch.len() <= BATCH_WIDTH,
        "a batch holds at most {BATCH_WIDTH} lanes"
    );
    let n_nodes = graph.node_ids.len();
    let mut solver = BatchSolver::new(n_nodes);

    // Per-node scalars used repeatedly during propagation (mirrors solve_group).
    let unc_by_node: Vec<bool> = graph
        .node_ids
        .iter()
        .map(|id| node_has_uncertainty(ctx, id))
        .collect();
    let expandable: Vec<bool> = graph
        .node_ids
        .iter()
        .map(|id| ctx.routine_by_id.contains_key(id))
        .collect();

    // Per-SCC pending arrivals from upstream SCCs (+ seeds). Drained when that
    // SCC is reached in topological order.
    let mut pending: Vec<Vec<Proposal>> = (0..scc.members.len()).map(|_| Vec::new()).collect();
    let mut seq: u64 = 0;

    // Rule 1: seed every lane's frontier into its entry node's SCC pending
    // buffer. The seed entry `TempVec` is `cross_hop` of the group's loop-routine
    // root across the seed callsite (EXACTLY solve_group's seed label), projected
    // to the entry's live params.
    for (lane, group) in batch.iter().enumerate() {
        let bit = 1u64 << lane;
        let root = root_state(group.loop_routine.id.as_str(), cw);
        for &si in &group.seed_indices {
            let seed = &seeds[si];
            let entry = seed.entry;
            let entry_id = graph.node_ids[entry as usize];
            let binding_ok = edge_kind_binding_ok(seed.entry_edge_kind);
            let entry_temp = cross_hop(
                &root,
                seed.loop_routine,
                seed.callsite.id.as_str(),
                entry_id,
                binding_ok,
                cw,
            );
            let depth = seed.seed_depth.min(2);
            let unc = unc_by_node[entry as usize];
            let entry_scc = scc.scc_of[entry as usize] as usize;
            pending[entry_scc].push(Proposal::Reach {
                node: entry,
                depth,
                unc,
                mask: bit,
                hops: 0,
                pred: ReachPredB::Seed {
                    seed_index: si as u32,
                },
            });
            for (slot, &p) in liveness.need[entry as usize].iter().enumerate() {
                let class = lookup(&entry_temp, p);
                pending[entry_scc].push(Proposal::Value {
                    node: entry,
                    slot: slot as u16,
                    class,
                    depth,
                    unc,
                    mask: bit,
                    hops: 0,
                    pred: ValuePredB::Seed {
                        seed_index: si as u32,
                    },
                });
            }
        }
    }

    // Rules 2-3: process SCCs in topological order. Within each SCC, a min-hops
    // (level-synchronous) delta worklist drains to least-fixpoint — masks only
    // OR-in bits, so cycles terminate; min-hops-first pops make a lane's first
    // arrival its minimum hop count.
    for &scc_id in &scc.topo_order {
        let mut heap: BinaryHeap<HeapItem> = BinaryHeap::new();
        for prop in std::mem::take(&mut pending[scc_id as usize]) {
            let hops = prop.hops();
            let s = seq;
            seq += 1;
            heap.push(HeapItem { hops, seq: s, prop });
        }
        while let Some(item) = heap.pop() {
            match item.prop {
                Proposal::Reach {
                    node,
                    depth,
                    unc,
                    mask,
                    hops,
                    pred,
                } => {
                    let (idx, new_bits) = solver.commit_reach(node, depth, unc, mask, hops, pred);
                    if new_bits == 0 || !expandable[node as usize] {
                        continue;
                    }
                    for (k, edge) in graph.edges[node as usize].iter().enumerate() {
                        let m = edge.to;
                        let d2 = (depth + edge.loop_depth).min(2);
                        let u2 = unc || unc_by_node[m as usize];
                        route(
                            Proposal::Reach {
                                node: m,
                                depth: d2,
                                unc: u2,
                                mask: new_bits,
                                hops: hops + 1,
                                pred: ReachPredB::Hop {
                                    pred: idx as u32,
                                    from_node: node,
                                    edge_k: k as u32,
                                },
                            },
                            scc_id,
                            &scc.scc_of,
                            &mut heap,
                            &mut pending,
                            &mut seq,
                        );
                        for (callee_slot, transfer) in
                            liveness.edge_transfers[node as usize][k].iter().enumerate()
                        {
                            if let ParamTransfer::Const(pt) = transfer {
                                route(
                                    Proposal::Value {
                                        node: m,
                                        slot: callee_slot as u16,
                                        class: *pt,
                                        depth: d2,
                                        unc: u2,
                                        mask: new_bits,
                                        hops: hops + 1,
                                        pred: ValuePredB::HopFromReach {
                                            pred: idx as u32,
                                            from_node: node,
                                            edge_k: k as u32,
                                        },
                                    },
                                    scc_id,
                                    &scc.scc_of,
                                    &mut heap,
                                    &mut pending,
                                    &mut seq,
                                );
                            }
                        }
                    }
                }
                Proposal::Value {
                    node,
                    slot,
                    class,
                    depth,
                    unc,
                    mask,
                    hops,
                    pred,
                } => {
                    let (idx, new_bits) =
                        solver.commit_value(node, slot, class, depth, unc, mask, hops, pred);
                    if new_bits == 0 || !expandable[node as usize] {
                        continue;
                    }
                    for (k, edge) in graph.edges[node as usize].iter().enumerate() {
                        let m = edge.to;
                        let d2 = (depth + edge.loop_depth).min(2);
                        let u2 = unc || unc_by_node[m as usize];
                        for (callee_slot, transfer) in
                            liveness.edge_transfers[node as usize][k].iter().enumerate()
                        {
                            if let ParamTransfer::Copy { caller_slot } = transfer
                                && *caller_slot == slot
                            {
                                route(
                                    Proposal::Value {
                                        node: m,
                                        slot: callee_slot as u16,
                                        class,
                                        depth: d2,
                                        unc: u2,
                                        mask: new_bits,
                                        hops: hops + 1,
                                        pred: ValuePredB::HopFromValue {
                                            pred: idx as u32,
                                            from_node: node,
                                            edge_k: k as u32,
                                        },
                                    },
                                    scc_id,
                                    &scc.scc_of,
                                    &mut heap,
                                    &mut pending,
                                    &mut seq,
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    // Rules 4-7: score terminals + select winner + materialize witness PER LANE.
    let mut out: Vec<LoopTerminalAgg<'a>> = Vec::new();
    for (lane, group) in batch.iter().enumerate() {
        let bit = 1u64 << lane;
        let root = root_state(group.loop_routine.id.as_str(), cw);
        let mut buckets: BTreeMap<(&'a str, &'a str), Vec<Candidate<'a>>> = BTreeMap::new();
        let mut discovery = 0usize;

        // Direct ops first (branch (a) precedence) — identical to solve_group.
        for &di in &group.direct_indices {
            let d = &direct_ops[di];
            let op = d.op;
            let owner = d.routine;
            let base_pt = resolve_terminal(op, &root, owner.id.as_str(), cw);
            let verdict = flowfield_verdict(base_pt, op, &ctx.table_by_id);
            let local_depth = op.loop_stack.len() as i64;
            let depth_bucket = local_depth.min(2);
            let is_singleton = is_setup_singleton_get(op, Some(owner), &ctx.table_by_id);
            let severity = severity_for(op, verdict, depth_bucket, is_singleton);
            buckets
                .entry((owner.id.as_str(), op.id.as_str()))
                .or_default()
                .push(Candidate {
                    verdict,
                    severity,
                    unc: false,
                    hops: 0,
                    depth_bucket,
                    discovery,
                    source: CandSource::Direct {
                        routine: owner,
                        loop_info: d.loop_info,
                        op,
                    },
                    terminal_op: op,
                    terminal_owner: owner,
                    terminal_local_depth: local_depth,
                });
            discovery += 1;
        }

        // Transitive candidates: one per present fact (this lane's bit set)
        // reaching each terminal. Read mode + proven re-check mirror solve_group.
        for node in 0..n_nodes as NodeIx {
            let terminals = &graph.terminals[node as usize];
            for (ti, t) in terminals.iter().enumerate() {
                let op = t.op;
                let owner = t.owner;
                let local_depth = t.local_depth;
                let is_singleton = is_setup_singleton_get(op, Some(owner), &ctx.table_by_id);

                let value_slot: Option<u16> = match liveness.terminal_reads[node as usize][ti] {
                    None => None,
                    Some(slot) => {
                        let i = liveness.need[node as usize][slot as usize];
                        if cw.contains(&(owner.id.to_string(), i)) {
                            None
                        } else {
                            Some(slot)
                        }
                    }
                };

                match value_slot {
                    None => {
                        let verdict = flowfield_verdict(
                            resolve_terminal(op, &TempVec::new(), owner.id.as_str(), cw),
                            op,
                            &ctx.table_by_id,
                        );
                        for &ri in &solver.reach_at[node as usize] {
                            let f = &solver.reach_facts[ri];
                            if f.mask & bit == 0 {
                                continue;
                            }
                            let depth_bucket = (f.depth + local_depth).min(2);
                            let severity = severity_for(op, verdict, depth_bucket, is_singleton);
                            buckets
                                .entry((owner.id.as_str(), op.id.as_str()))
                                .or_default()
                                .push(Candidate {
                                    verdict,
                                    severity,
                                    unc: f.unc,
                                    hops: solver.reach_hops[ri][lane],
                                    depth_bucket,
                                    discovery,
                                    source: CandSource::TransReach { reach_fact: ri },
                                    terminal_op: op,
                                    terminal_owner: owner,
                                    terminal_local_depth: local_depth,
                                });
                            discovery += 1;
                        }
                    }
                    Some(slot) => {
                        let i = liveness.need[node as usize][slot as usize];
                        for &vi in &solver.value_at[node as usize] {
                            let f = &solver.value_facts[vi];
                            if f.slot != slot || f.mask & bit == 0 {
                                continue;
                            }
                            let frame: TempVec = std::iter::once((i, f.class)).collect();
                            let verdict = flowfield_verdict(
                                resolve_terminal(op, &frame, owner.id.as_str(), cw),
                                op,
                                &ctx.table_by_id,
                            );
                            let depth_bucket = (f.depth + local_depth).min(2);
                            let severity = severity_for(op, verdict, depth_bucket, is_singleton);
                            buckets
                                .entry((owner.id.as_str(), op.id.as_str()))
                                .or_default()
                                .push(Candidate {
                                    verdict,
                                    severity,
                                    unc: f.unc,
                                    hops: solver.value_hops[vi][lane],
                                    depth_bucket,
                                    discovery,
                                    source: CandSource::TransValue { value_fact: vi },
                                    terminal_op: op,
                                    terminal_owner: owner,
                                    terminal_local_depth: local_depth,
                                });
                            discovery += 1;
                        }
                    }
                }
            }
        }

        // Rules 5/7: per bucket, select the winner and materialize its witness.
        for (_key, cands) in buckets {
            let mut reachable_verdicts: Vec<TempVerdict> =
                cands.iter().map(|c| c.verdict).collect();
            reachable_verdicts.sort();
            reachable_verdicts.dedup();

            let winner = cands
                .iter()
                .max_by_key(|c| {
                    selection_rank(
                        c.severity,
                        c.verdict,
                        c.unc,
                        c.hops,
                        c.depth_bucket,
                        c.discovery,
                    )
                })
                .expect("a bucket is never empty");

            let (witness, uncertainties, entry_callsite_id, effective_loop_depth) =
                match &winner.source {
                    CandSource::Direct {
                        routine,
                        loop_info,
                        op,
                    } => {
                        let loop_step = loop_step_ev(routine, loop_info);
                        let op_step = terminal_step(
                            &ctx.routine_by_id,
                            &ctx.table_by_id,
                            routine.id.as_str(),
                            Some(op.id.as_str()),
                        );
                        (
                            vec![loop_step, op_step],
                            Vec::new(),
                            None,
                            winner.terminal_local_depth,
                        )
                    }
                    CandSource::TransReach { reach_fact } => {
                        let terminal_node = solver.reach_facts[*reach_fact].node;
                        let (hops, seed_index) =
                            collect_reach_chain_b(&solver.reach_pred, lane, *reach_fact);
                        build_transitive_witness(
                            &hops,
                            seed_index,
                            terminal_node,
                            winner.terminal_owner,
                            winner.terminal_op,
                            winner.terminal_local_depth,
                            graph,
                            ctx,
                            seeds,
                        )
                    }
                    CandSource::TransValue { value_fact } => {
                        let terminal_node = solver.value_facts[*value_fact].node;
                        let (hops, seed_index) = collect_value_chain_b(
                            &solver.value_pred,
                            &solver.reach_pred,
                            lane,
                            *value_fact,
                        );
                        build_transitive_witness(
                            &hops,
                            seed_index,
                            terminal_node,
                            winner.terminal_owner,
                            winner.terminal_op,
                            winner.terminal_local_depth,
                            graph,
                            ctx,
                            seeds,
                        )
                    }
                };

            out.push(LoopTerminalAgg {
                loop_routine: group.loop_routine,
                loop_id: group.loop_id,
                loop_info: group.loop_info,
                terminal: D1Terminal {
                    op: winner.terminal_op,
                    owner: winner.terminal_owner,
                    local_depth: winner.terminal_local_depth,
                },
                entry_callsite_id,
                severity: winner.severity,
                verdict: winner.verdict,
                reachable_verdicts,
                depth_bucket: winner.depth_bucket,
                effective_loop_depth,
                witness,
                uncertainties,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::l3::l3_workspace::L3Workspace;
    use crate::engine::l4::combined_graph::CombinedEdge;
    use crate::engine::l5::d1_graph::build_d1_graph;
    use crate::engine::l5::d1_liveness::compute_liveness;
    use crate::engine::l5::d1_reach::process_group;
    use crate::engine::l5::full_summary::FullRoutineSummary;
    use crate::engine::l5::test_support::{
        call_site, coverage, edge_kind, fact, loop_def, minimal_ctx, record_op, routine, summary,
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

    fn ws(routines: &[L3Routine]) -> L3Workspace {
        L3Workspace {
            objects: vec![],
            tables: vec![],
            routines: routines.to_vec(),
        }
    }

    /// Group `seeds`/`direct_ops` EXACTLY as `search_loops` does, run BOTH
    /// `process_group` (oracle) and `solve_group` per group, and assert the six
    /// load-bearing components agree per (loop, terminal-op). Also assert every
    /// `solve_group` aggregate carries a structurally-valid realizing witness.
    /// Panics if the fixture yields zero aggregates (non-vacuity).
    #[allow(clippy::type_complexity)]
    fn assert_agrees(
        graph: &D1Graph,
        seeds: &[D1Seed],
        direct_ops: &[DirectOp],
        ctx: &DetectorContext,
        cw: &ClosedWorldTempParams,
    ) {
        let liveness = compute_liveness(graph, ctx, cw);

        struct Group<'a> {
            loop_routine: &'a L3Routine,
            loop_info: &'a PLoop,
            seed_indices: Vec<usize>,
            direct_indices: Vec<usize>,
        }
        let mut groups: BTreeMap<(&str, &str), Group> = BTreeMap::new();
        for (i, seed) in seeds.iter().enumerate() {
            groups
                .entry((seed.loop_routine.id.as_str(), seed.loop_id))
                .or_insert_with(|| Group {
                    loop_routine: seed.loop_routine,
                    loop_info: seed.loop_info,
                    seed_indices: Vec::new(),
                    direct_indices: Vec::new(),
                })
                .seed_indices
                .push(i);
        }
        for (i, d) in direct_ops.iter().enumerate() {
            groups
                .entry((d.routine.id.as_str(), d.loop_id))
                .or_insert_with(|| Group {
                    loop_routine: d.routine,
                    loop_info: d.loop_info,
                    seed_indices: Vec::new(),
                    direct_indices: Vec::new(),
                })
                .direct_indices
                .push(i);
        }

        let mut total = 0usize;
        for (key, group) in &groups {
            let loop_id = key.1;
            let oracle = process_group(
                graph,
                seeds,
                direct_ops,
                ctx,
                cw,
                group.loop_routine,
                loop_id,
                group.loop_info,
                &group.seed_indices,
                &group.direct_indices,
            );
            let solved = solve_group(
                graph,
                &liveness,
                seeds,
                direct_ops,
                ctx,
                cw,
                group.loop_routine,
                loop_id,
                group.loop_info,
                &group.seed_indices,
                &group.direct_indices,
            );

            // Component 1 (coverage): the (owner, op) key sets are identical.
            let ok: std::collections::BTreeSet<(&str, &str)> = oracle
                .iter()
                .map(|a| (a.terminal.owner.id.as_str(), a.terminal.op.id.as_str()))
                .collect();
            let sk: std::collections::BTreeSet<(&str, &str)> = solved
                .iter()
                .map(|a| (a.terminal.owner.id.as_str(), a.terminal.op.id.as_str()))
                .collect();
            assert_eq!(
                ok, sk,
                "coverage (component 1) diverged for group {key:?}: oracle {ok:?} vs solved {sk:?}"
            );

            let oracle_by: HashMap<(&str, &str), &LoopTerminalAgg> = oracle
                .iter()
                .map(|a| ((a.terminal.owner.id.as_str(), a.terminal.op.id.as_str()), a))
                .collect();
            for s in &solved {
                let k = (s.terminal.owner.id.as_str(), s.terminal.op.id.as_str());
                let o = oracle_by[&k];
                assert_eq!(
                    s.reachable_verdicts, o.reachable_verdicts,
                    "reachable_verdicts (2) diverged for {key:?}/{k:?}"
                );
                assert_eq!(
                    s.severity, o.severity,
                    "severity (3) diverged for {key:?}/{k:?}"
                );
                assert_eq!(
                    s.verdict, o.verdict,
                    "verdict (4) diverged for {key:?}/{k:?}"
                );
                assert_eq!(
                    s.depth_bucket, o.depth_bucket,
                    "depth_bucket (5) diverged for {key:?}/{k:?}"
                );
                let s_unc = winner_unc(s);
                let o_unc = winner_unc(o);
                assert_eq!(s_unc, o_unc, "unc (6) diverged for {key:?}/{k:?}");

                assert_witness_valid(s, ctx);
            }
            total += solved.len();
        }
        assert!(total > 0, "fixture must produce at least one aggregate");
    }

    /// The winner's `unc` is not stored directly on `LoopTerminalAgg`; recover it
    /// from the materialized uncertainty union (empty iff the winning path
    /// crossed no uncertain node — the same bit `unc` tracks). Both oracle and
    /// solved use the same rule, so comparing the derived bit is faithful.
    ///
    /// This is EXACT for component 6, not a mere proxy: both `process_group` and
    /// `solve_group` derive `unc` as the OR of `node_has_uncertainty` over the
    /// winning path's nodes, and the materialized union is non-empty under
    /// exactly that same condition — so `!uncertainties.is_empty()` recovers the
    /// winner's `unc` bit identically on both sides. (The uncertainty VECTOR
    /// itself may still differ — that is component 7.)
    fn winner_unc(agg: &LoopTerminalAgg) -> bool {
        !agg.uncertainties.is_empty()
    }

    /// A `solve_group` aggregate's witness must be a valid realizing path: first
    /// step in the loop routine (a loop step), last step the terminal op, the
    /// intermediate steps a `[call, hop*]` chain, and — the brief's "hop count ==
    /// reported" + edge-contiguity requirements — every consecutive `(from, to)`
    /// pair of intermediate/terminal steps a REAL graph edge (`from`'s callsite
    /// resolving `from -> to` in `ctx.graph.edges_by_from`). Because the walk
    /// crosses exactly one real edge per step and lands precisely on the terminal
    /// owner, the witness's hop count is the true path length — no gap, no repeat,
    /// no phantom hop (the "reported" count).
    fn assert_witness_valid(agg: &LoopTerminalAgg, ctx: &DetectorContext) {
        let w = &agg.witness;
        assert!(!w.is_empty(), "witness must be non-empty");
        // First step: the loop step, in the loop routine, naming the loop.
        assert_eq!(
            w[0].routine_id, agg.loop_routine.id,
            "witness first step must be in the loop routine"
        );
        assert_eq!(
            w[0].loop_id.as_deref(),
            Some(agg.loop_info.id.as_str()),
            "witness first step must be the loop step"
        );
        // Last step: the terminal op.
        let last = w.last().unwrap();
        assert_eq!(
            last.operation_id.as_deref(),
            Some(agg.terminal.op.id.as_str()),
            "witness last step must be the terminal op"
        );
        assert_eq!(
            last.routine_id, agg.terminal.owner.id,
            "witness last step must be owned by the terminal routine"
        );
        assert!(
            last.callsite_id.is_none(),
            "the terminal step carries no callsite"
        );
        if w.len() == 2 {
            // Direct op: [loop_step, terminal_step], no entry callsite.
            assert_eq!(
                agg.entry_callsite_id, None,
                "a direct-op winner has no entry callsite"
            );
        } else {
            // Transitive: [loop, call, hop*, terminal]. The call + every hop
            // step carries a callsite; the entry callsite is recorded.
            assert!(w.len() >= 3, "a transitive witness has loop+call+terminal");
            assert!(
                agg.entry_callsite_id.is_some(),
                "a transitive winner records its entry callsite"
            );
            // Edge contiguity + hop count: each `(from, to)` in the [call, hop*,
            // terminal] tail is a real edge `from --from.callsite--> to`. `from`
            // ranges over the call + hop steps (all carry a callsite, none a loop
            // id); `to` ranges over the hops + terminal step.
            for pair in w[1..].windows(2) {
                let from = &pair[0];
                let to = &pair[1];
                assert!(
                    from.callsite_id.is_some(),
                    "the call + hop steps each carry a callsite"
                );
                assert!(from.loop_id.is_none(), "non-loop intermediate steps");
                let cs = from.callsite_id.as_deref();
                let is_real_edge =
                    ctx.graph
                        .edges_by_from
                        .get(&from.routine_id)
                        .is_some_and(|edges| {
                            edges
                                .iter()
                                .any(|e| e.callsite_id.as_deref() == cs && e.to == to.routine_id)
                        });
                assert!(
                    is_real_edge,
                    "witness step {} --{:?}--> {} must be a real graph edge (contiguity)",
                    from.routine_id, cs, to.routine_id
                );
            }
        }
    }

    // === Fixture 1: budget-buster fanout ================================
    #[test]
    fn agrees_on_budget_buster_fanout() {
        let mut r = routine("R", "procedure");
        r.loops = vec![loop_def("R/loop0")];
        r.call_sites = vec![call_site("R/cs0", "A", vec!["R/loop0".to_string()])];
        let a = routine("A", "procedure");

        let mut routines = vec![r, a];
        let mut a_edges: Vec<CombinedEdge> = Vec::new();
        let mut summaries: HashMap<String, FullRoutineSummary> = HashMap::new();
        summaries.insert("A".to_string(), db_summary("A", "t/A"));
        for i in 0..600 {
            let did = format!("D{i}");
            routines.push(routine(&did, "procedure"));
            a_edges.push(edge_kind("A", &did, &format!("A/cs{i}"), "direct"));
            summaries.insert(did.clone(), db_summary(&did, &format!("t/{did}")));
        }
        let mut t = routine("T", "procedure");
        t.record_operations = vec![record_op(
            "T/op0",
            "Modify",
            "Rec",
            Some("t/T"),
            vec![],
            false,
        )];
        routines.push(t);
        a_edges.push(edge_kind("A", "T", "A/csT", "direct"));
        summaries.insert("T".to_string(), db_summary("T", "t/T"));

        let mut graph_edges: HashMap<String, Vec<CombinedEdge>> = HashMap::new();
        graph_edges.insert(
            "R".to_string(),
            vec![edge_kind("R", "A", "R/cs0", "direct")],
        );
        graph_edges.insert("A".to_string(), a_edges);

        let ctx = minimal_ctx(&routines, graph_edges, summaries);
        let workspace = ws(&routines);
        let mut memo = HashMap::new();
        let (graph, seeds) = build_d1_graph(&ctx, &workspace, &mut memo);
        assert!(graph.node_ids.len() > 500);
        let cw = ClosedWorldTempParams::new();
        assert_agrees(&graph, &seeds, &[], &ctx, &cw);
    }

    // === Fixture 2: depth-2 route beats a shorter depth-1 route ==========
    #[test]
    fn agrees_on_depth2_beats_depth1() {
        let mut r = routine("R", "procedure");
        r.loops = vec![loop_def("R/loop0")];
        r.call_sites = vec![call_site("R/cs0", "A", vec!["R/loop0".to_string()])];
        let mut a = routine("A", "procedure");
        a.call_sites = vec![
            call_site("A/csT", "T", vec![]),
            call_site("A/csX", "X", vec!["A/loop0".to_string()]),
        ];
        let x = routine("X", "procedure");
        let y = routine("Y", "procedure");
        let mut t = routine("T", "procedure");
        t.record_operations = vec![record_op(
            "T/op0",
            "FindSet",
            "Rec",
            Some("t/T"),
            vec![],
            false,
        )];
        let routines = vec![r, a, x, y, t];

        let mut graph_edges: HashMap<String, Vec<CombinedEdge>> = HashMap::new();
        graph_edges.insert(
            "R".to_string(),
            vec![edge_kind("R", "A", "R/cs0", "direct")],
        );
        graph_edges.insert(
            "A".to_string(),
            vec![
                edge_kind("A", "T", "A/csT", "direct"),
                edge_kind("A", "X", "A/csX", "direct"),
            ],
        );
        graph_edges.insert(
            "X".to_string(),
            vec![edge_kind("X", "Y", "X/csY", "direct")],
        );
        graph_edges.insert(
            "Y".to_string(),
            vec![edge_kind("Y", "T", "Y/csT", "direct")],
        );

        let summaries: HashMap<String, FullRoutineSummary> = ["A", "X", "Y", "T"]
            .iter()
            .map(|id| (id.to_string(), db_summary(id, &format!("t/{id}"))))
            .collect();

        let ctx = minimal_ctx(&routines, graph_edges, summaries);
        let workspace = ws(&routines);
        let mut memo = HashMap::new();
        let (graph, seeds) = build_d1_graph(&ctx, &workspace, &mut memo);
        let cw = ClosedWorldTempParams::new();
        assert_agrees(&graph, &seeds, &[], &ctx, &cw);
    }

    // === Fixture 2b: a winner whose realizing path crosses an UNCERTAIN
    // node — the only fixture that drives component 6 (`unc`) to `true`, so the
    // fact solver's uncertainty propagation is genuinely differentiated (all the
    // other fixtures leave `uncertainties_by_node` empty → `unc` trivially
    // false). The deep A->X->Y->T route wins on severity; node X carries an
    // uncertainty, so BOTH engines must report the winner `unc == true`.
    #[test]
    fn agrees_on_uncertain_winner() {
        use crate::engine::l4::summary::Uncertainty;

        let mut r = routine("R", "procedure");
        r.loops = vec![loop_def("R/loop0")];
        r.call_sites = vec![call_site("R/cs0", "A", vec!["R/loop0".to_string()])];
        let mut a = routine("A", "procedure");
        a.call_sites = vec![
            call_site("A/csT", "T", vec![]),
            call_site("A/csX", "X", vec!["A/loop0".to_string()]),
        ];
        let x = routine("X", "procedure");
        let y = routine("Y", "procedure");
        let mut t = routine("T", "procedure");
        t.record_operations = vec![record_op(
            "T/op0",
            "FindSet",
            "Rec",
            Some("t/T"),
            vec![],
            false,
        )];
        let routines = vec![r, a, x, y, t];

        let mut graph_edges: HashMap<String, Vec<CombinedEdge>> = HashMap::new();
        graph_edges.insert(
            "R".to_string(),
            vec![edge_kind("R", "A", "R/cs0", "direct")],
        );
        graph_edges.insert(
            "A".to_string(),
            vec![
                edge_kind("A", "T", "A/csT", "direct"),
                edge_kind("A", "X", "A/csX", "direct"),
            ],
        );
        graph_edges.insert(
            "X".to_string(),
            vec![edge_kind("X", "Y", "X/csY", "direct")],
        );
        graph_edges.insert(
            "Y".to_string(),
            vec![edge_kind("Y", "T", "Y/csT", "direct")],
        );

        let summaries: HashMap<String, FullRoutineSummary> = ["A", "X", "Y", "T"]
            .iter()
            .map(|id| (id.to_string(), db_summary(id, &format!("t/{id}"))))
            .collect();

        let mut ctx = minimal_ctx(&routines, graph_edges, summaries);
        // Inject an uncertainty on node X — on the winning deep route only.
        ctx.uncertainties_by_node.insert(
            "X".to_string(),
            vec![Uncertainty {
                kind: "dynamic-dispatch".to_string(),
                callsite_id: None,
                operation_id: None,
                routine_id: Some("X".to_string()),
                interface_name: None,
            }],
        );

        let workspace = ws(&routines);
        let mut memo = HashMap::new();
        let (graph, seeds) = build_d1_graph(&ctx, &workspace, &mut memo);
        let cw = ClosedWorldTempParams::new();

        // Cross-check that this fixture actually reaches `unc == true` on the
        // winner (guards against a vacuous always-false component 6).
        let liveness = compute_liveness(&graph, &ctx, &cw);
        let seed_indices: Vec<usize> = (0..seeds.len()).collect();
        let solved = solve_group(
            &graph,
            &liveness,
            &seeds,
            &[],
            &ctx,
            &cw,
            seeds[0].loop_routine,
            seeds[0].loop_id,
            seeds[0].loop_info,
            &seed_indices,
            &[],
        );
        assert_eq!(solved.len(), 1);
        assert_eq!(solved[0].severity, "high", "the deep route wins");
        assert!(
            !solved[0].uncertainties.is_empty(),
            "the winning path crosses the uncertain node X — unc must be true"
        );

        assert_agrees(&graph, &seeds, &[], &ctx, &cw);
    }

    // === Fixture 3: physical route beats temp route (multi-seed) =========
    fn temp_vs_physical_fixture() -> Fixture {
        use crate::engine::l5::test_support::{arg_binding, ts_known, ts_pd};

        let mut r = routine("R", "procedure");
        r.loops = vec![loop_def("R/loop0")];
        let mut cs0 = call_site("R/cs0", "H", vec!["R/loop0".to_string()]);
        cs0.argument_bindings = vec![arg_binding(0, Some(ts_known(true)))];
        let mut cs1 = call_site("R/cs1", "H", vec!["R/loop0".to_string()]);
        cs1.argument_bindings = vec![arg_binding(0, Some(ts_known(false)))];
        r.call_sites = vec![cs0, cs1];

        let mut h = routine("H", "procedure");
        let mut op0 = record_op("H/op0", "Modify", "Rec", Some("t/H"), vec![], false);
        op0.temp_state = Some(ts_pd(0));
        h.record_operations = vec![op0];

        let routines = vec![r, h];
        let mut graph_edges: HashMap<String, Vec<CombinedEdge>> = HashMap::new();
        graph_edges.insert(
            "R".to_string(),
            vec![
                edge_kind("R", "H", "R/cs0", "direct"),
                edge_kind("R", "H", "R/cs1", "direct"),
            ],
        );
        let summaries: HashMap<String, FullRoutineSummary> =
            [("H".to_string(), db_summary("H", "t/H"))]
                .into_iter()
                .collect();
        (routines, graph_edges, summaries)
    }

    #[test]
    fn agrees_on_physical_beats_temp_multi_seed() {
        let (routines, graph_edges, summaries) = temp_vs_physical_fixture();
        let ctx = minimal_ctx(&routines, graph_edges, summaries);
        let workspace = ws(&routines);
        let mut memo = HashMap::new();
        let (graph, seeds) = build_d1_graph(&ctx, &workspace, &mut memo);
        assert_eq!(seeds.len(), 2);
        let cw = ClosedWorldTempParams::new();
        assert_agrees(&graph, &seeds, &[], &ctx, &cw);
    }

    // === Fixture 3b: the FlowFieldGated-vs-Physical DISCOVERY TIE =========
    // The one shape where a pure first-discovery tie decides component 4
    // (verdict): a `CalcFields` PD-terminal reachable by two in-loop seeds — one
    // passing a TEMP record (-> `FlowFieldGated`, since the FlowField gate BLOCKS
    // the info-downgrade — here via a missing `table_id`, `flowfield_gate_blocks_
    // downgrade` d1.rs:143-148), one PHYSICAL (-> `Physical`). Both share
    // verdict-quality rank 3 AND the SAME severity ("high", `CalcFields` is a
    // heavy-read op) at equal unc(false)/hops(0), so the winner is decided by
    // `-discovery` ALONE. `reachable_verdicts` (a SET) holds BOTH; only the single
    // WINNER verdict is at stake — a binding component-4 constraint. This asserts
    // `solve_group` and `process_group` pick the SAME winner verdict.
    fn flowfield_tie_fixture() -> Fixture {
        use crate::engine::l5::test_support::{arg_binding, ts_known, ts_pd};

        let mut r = routine("R", "procedure");
        r.loops = vec![loop_def("R/loop0")];
        let mut cs0 = call_site("R/cs0", "H", vec!["R/loop0".to_string()]);
        cs0.argument_bindings = vec![arg_binding(0, Some(ts_known(true)))]; // temp
        let mut cs1 = call_site("R/cs1", "H", vec!["R/loop0".to_string()]);
        cs1.argument_bindings = vec![arg_binding(0, Some(ts_known(false)))]; // physical
        r.call_sites = vec![cs0, cs1];

        let mut h = routine("H", "procedure");
        // CalcFields with NO table_id -> the FlowField gate blocks the temp
        // downgrade (unresolvable table is conservative), so a Temp-resolved read
        // is `FlowFieldGated`, not `Temporary`.
        let mut op0 = record_op("H/op0", "CalcFields", "Rec", None, vec![], false);
        op0.temp_state = Some(ts_pd(0)); // PD on param 0 — resolves per caller
        h.record_operations = vec![op0];

        let routines = vec![r, h];
        let mut graph_edges: HashMap<String, Vec<CombinedEdge>> = HashMap::new();
        graph_edges.insert(
            "R".to_string(),
            vec![
                edge_kind("R", "H", "R/cs0", "direct"),
                edge_kind("R", "H", "R/cs1", "direct"),
            ],
        );
        let summaries: HashMap<String, FullRoutineSummary> =
            [("H".to_string(), db_summary("H", "t/H"))]
                .into_iter()
                .collect();
        (routines, graph_edges, summaries)
    }

    #[test]
    fn agrees_on_flowfield_vs_physical_discovery_tie() {
        use crate::engine::l5::detectors::d1::TempVerdict;

        let (routines, graph_edges, summaries) = flowfield_tie_fixture();
        let ctx = minimal_ctx(&routines, graph_edges, summaries);
        let workspace = ws(&routines);
        let mut memo = HashMap::new();
        let (graph, seeds) = build_d1_graph(&ctx, &workspace, &mut memo);
        assert_eq!(seeds.len(), 2, "one temp seed + one physical seed into H");
        let cw = ClosedWorldTempParams::new();

        // Confirm the tie is real: BOTH verdicts reach the terminal at the same
        // severity, and the winner is genuinely FlowFieldGated (not Temporary,
        // not Physical) — else the fixture would not exercise the tie path.
        let liveness = compute_liveness(&graph, &ctx, &cw);
        let seed_indices: Vec<usize> = (0..seeds.len()).collect();
        let solved = solve_group(
            &graph,
            &liveness,
            &seeds,
            &[],
            &ctx,
            &cw,
            seeds[0].loop_routine,
            seeds[0].loop_id,
            seeds[0].loop_info,
            &seed_indices,
            &[],
        );
        assert_eq!(solved.len(), 1);
        assert_eq!(solved[0].severity, "high", "CalcFields heavy-read => high");
        assert_eq!(
            solved[0].reachable_verdicts,
            vec![TempVerdict::Physical, TempVerdict::FlowFieldGated],
            "both verdicts reach the terminal (the SET holds both)"
        );

        // The differential decides the tie: solve_group and process_group must
        // agree on the single WINNER verdict (component 4) despite the tie.
        assert_agrees(&graph, &seeds, &[], &ctx, &cw);
    }

    // === Fixture 4: cycle ================================================
    #[test]
    fn agrees_on_cycle() {
        let mut r = routine("R", "procedure");
        r.loops = vec![loop_def("R/loop0")];
        r.call_sites = vec![call_site("R/cs0", "A", vec!["R/loop0".to_string()])];
        let a = routine("A", "procedure");
        let mut b = routine("B", "procedure");
        b.record_operations = vec![record_op(
            "B/op0",
            "Modify",
            "Rec",
            Some("t/B"),
            vec![],
            false,
        )];
        let routines = vec![r, a, b];

        let mut graph_edges: HashMap<String, Vec<CombinedEdge>> = HashMap::new();
        graph_edges.insert(
            "R".to_string(),
            vec![edge_kind("R", "A", "R/cs0", "direct")],
        );
        graph_edges.insert(
            "A".to_string(),
            vec![edge_kind("A", "B", "A/cs0", "direct")],
        );
        graph_edges.insert(
            "B".to_string(),
            vec![edge_kind("B", "A", "B/cs0", "direct")],
        );

        let summaries: HashMap<String, FullRoutineSummary> = [
            ("A".to_string(), db_summary("A", "t/A")),
            ("B".to_string(), db_summary("B", "t/B")),
        ]
        .into_iter()
        .collect();

        let ctx = minimal_ctx(&routines, graph_edges, summaries);
        let workspace = ws(&routines);
        let mut memo = HashMap::new();
        let (graph, seeds) = build_d1_graph(&ctx, &workspace, &mut memo);
        let cw = ClosedWorldTempParams::new();
        assert_agrees(&graph, &seeds, &[], &ctx, &cw);
    }

    // === Fixture 5: direct + transitive adjudication =====================
    #[test]
    fn agrees_on_direct_and_transitive() {
        let mut r = routine("R", "procedure");
        r.loops = vec![loop_def("R/loop0")];
        r.call_sites = vec![call_site("R/cs0", "A", vec!["R/loop0".to_string()])];
        r.record_operations = vec![record_op(
            "R/T",
            "FindSet",
            "Rec",
            Some("t/R"),
            vec!["R/loop0".to_string()],
            false,
        )];
        let mut a = routine("A", "procedure");
        a.call_sites = vec![call_site("A/csR", "R", vec!["A/loop0".to_string()])];
        let routines = vec![r, a];

        let mut graph_edges: HashMap<String, Vec<CombinedEdge>> = HashMap::new();
        graph_edges.insert(
            "R".to_string(),
            vec![edge_kind("R", "A", "R/cs0", "direct")],
        );
        graph_edges.insert(
            "A".to_string(),
            vec![edge_kind("A", "R", "A/csR", "direct")],
        );

        let summaries: HashMap<String, FullRoutineSummary> = [
            ("A".to_string(), db_summary("A", "t/A")),
            ("R".to_string(), db_summary("R", "t/R")),
        ]
        .into_iter()
        .collect();

        let ctx = minimal_ctx(&routines, graph_edges, summaries);
        let workspace = ws(&routines);
        let mut memo = HashMap::new();
        let (graph, seeds) = build_d1_graph(&ctx, &workspace, &mut memo);
        let direct_ops = vec![DirectOp {
            routine: &routines[0],
            loop_id: routines[0].loops[0].id.as_str(),
            loop_info: &routines[0].loops[0],
            op: &routines[0].record_operations[0],
        }];
        let cw = ClosedWorldTempParams::new();
        assert_agrees(&graph, &seeds, &direct_ops, &ctx, &cw);
    }

    // === Fixture 6: multi-group (overlapping closure + a direct group) ===
    fn multi_group_fixture() -> Fixture {
        let mut r1 = routine("R1", "procedure");
        r1.loops = vec![loop_def("R1/loop0")];
        r1.call_sites = vec![call_site("R1/cs0", "A1", vec!["R1/loop0".to_string()])];
        let mut a1 = routine("A1", "procedure");
        a1.call_sites = vec![call_site("A1/csT", "T", vec![])];

        let mut r2 = routine("R2", "procedure");
        r2.loops = vec![loop_def("R2/loop0")];
        r2.call_sites = vec![call_site("R2/cs0", "A2", vec!["R2/loop0".to_string()])];
        let mut a2 = routine("A2", "procedure");
        a2.call_sites = vec![
            call_site("A2/csT", "T", vec![]),
            call_site("A2/csX", "X2", vec!["A2/loop0".to_string()]),
        ];
        let mut x2 = routine("X2", "procedure");
        x2.call_sites = vec![call_site("X2/csY", "Y2", vec![])];
        let mut y2 = routine("Y2", "procedure");
        y2.call_sites = vec![call_site("Y2/csT", "T", vec![])];

        let mut r3 = routine("R3", "procedure");
        r3.loops = vec![loop_def("R3/loop0")];
        r3.call_sites = vec![call_site("R3/cs0", "A3", vec!["R3/loop0".to_string()])];
        let mut a3 = routine("A3", "procedure");
        a3.call_sites = vec![call_site("A3/csT", "T", vec![])];

        let mut r4 = routine("R4", "procedure");
        r4.loops = vec![loop_def("R4/loop0")];
        r4.record_operations = vec![record_op(
            "R4/opD",
            "Modify",
            "Rec",
            Some("t/R4"),
            vec!["R4/loop0".to_string()],
            false,
        )];

        let mut t = routine("T", "procedure");
        t.record_operations = vec![record_op(
            "T/op0",
            "FindSet",
            "Rec",
            Some("t/T"),
            vec![],
            false,
        )];

        let routines = vec![r1, a1, r2, a2, x2, y2, r3, a3, r4, t];

        let mut graph_edges: HashMap<String, Vec<CombinedEdge>> = HashMap::new();
        graph_edges.insert(
            "R1".to_string(),
            vec![edge_kind("R1", "A1", "R1/cs0", "direct")],
        );
        graph_edges.insert(
            "A1".to_string(),
            vec![edge_kind("A1", "T", "A1/csT", "direct")],
        );
        graph_edges.insert(
            "R2".to_string(),
            vec![edge_kind("R2", "A2", "R2/cs0", "direct")],
        );
        graph_edges.insert(
            "A2".to_string(),
            vec![
                edge_kind("A2", "T", "A2/csT", "direct"),
                edge_kind("A2", "X2", "A2/csX", "direct"),
            ],
        );
        graph_edges.insert(
            "X2".to_string(),
            vec![edge_kind("X2", "Y2", "X2/csY", "direct")],
        );
        graph_edges.insert(
            "Y2".to_string(),
            vec![edge_kind("Y2", "T", "Y2/csT", "direct")],
        );
        graph_edges.insert(
            "R3".to_string(),
            vec![edge_kind("R3", "A3", "R3/cs0", "direct")],
        );
        graph_edges.insert(
            "A3".to_string(),
            vec![edge_kind("A3", "T", "A3/csT", "direct")],
        );

        let summaries: HashMap<String, FullRoutineSummary> = [
            ("A1", "t/A1"),
            ("A2", "t/A2"),
            ("X2", "t/X2"),
            ("Y2", "t/Y2"),
            ("A3", "t/A3"),
            ("T", "t/T"),
        ]
        .into_iter()
        .map(|(id, table)| (id.to_string(), db_summary(id, table)))
        .collect();

        (routines, graph_edges, summaries)
    }

    #[test]
    fn agrees_on_multi_group() {
        let (routines, graph_edges, summaries) = multi_group_fixture();
        let ctx = minimal_ctx(&routines, graph_edges, summaries);
        let workspace = ws(&routines);
        let mut memo = HashMap::new();
        let (graph, seeds) = build_d1_graph(&ctx, &workspace, &mut memo);
        assert_eq!(seeds.len(), 3);
        let r4_idx = routines.iter().position(|r| r.id == "R4").unwrap();
        let direct_ops = vec![DirectOp {
            routine: &routines[r4_idx],
            loop_id: routines[r4_idx].loops[0].id.as_str(),
            loop_info: &routines[r4_idx].loops[0],
            op: &routines[r4_idx].record_operations[0],
        }];
        let cw = ClosedWorldTempParams::new();
        assert_agrees(&graph, &seeds, &direct_ops, &ctx, &cw);
    }

    // === Task D3: condensation determinism + real-SCC identification ======
    #[test]
    fn condensation_deterministic() {
        use crate::engine::l5::d1_graph::D1Edge;
        // A<->B is a real 2-member cycle; C->D is a chain of two singletons.
        let node_ids = vec!["A", "B", "C", "D"];
        let mut node_ix = HashMap::new();
        for (i, id) in node_ids.iter().enumerate() {
            node_ix.insert(*id, i as u32);
        }
        let mk = |to: u32| D1Edge {
            to,
            kind: "direct",
            callsite_id: None,
            loop_depth: 0,
            binding_ok: true,
        };
        let graph = D1Graph {
            node_ids: node_ids.clone(),
            node_ix,
            edges: vec![
                vec![mk(1)], // A -> B
                vec![mk(0)], // B -> A (closes the cycle)
                vec![mk(3)], // C -> D
                vec![],      // D (leaf)
            ],
            terminals: vec![vec![], vec![], vec![], vec![]],
        };

        let s1 = condense(&graph);
        let s2 = condense(&graph);
        assert_eq!(s1.scc_of, s2.scc_of, "scc ids identical across runs");
        assert_eq!(
            s1.topo_order, s2.topo_order,
            "topo order identical across runs"
        );

        let a = graph.node_ix["A"];
        let b = graph.node_ix["B"];
        let c = graph.node_ix["C"];
        let d = graph.node_ix["D"];
        // The cycle is one SCC whose members are exactly {A, B}.
        assert_eq!(
            s1.scc_of[a as usize], s1.scc_of[b as usize],
            "A and B belong to the same (cyclic) SCC"
        );
        assert_eq!(
            s1.members[s1.scc_of[a as usize] as usize],
            vec![a.min(b), a.max(b)],
            "the cyclic SCC's members are exactly {{A, B}}"
        );
        // C and D are distinct singleton SCCs.
        assert_ne!(
            s1.scc_of[c as usize], s1.scc_of[d as usize],
            "C and D are separate singleton SCCs"
        );
        assert_eq!(s1.members[s1.scc_of[c as usize] as usize], vec![c]);
        assert_eq!(s1.members[s1.scc_of[d as usize] as usize], vec![d]);
        // Topological order: the caller C precedes its callee D.
        assert!(
            s1.scc_of[c as usize] < s1.scc_of[d as usize],
            "C (caller) precedes D (callee) in topological SCC order"
        );
        assert_eq!(
            s1.topo_order,
            (0..s1.members.len() as u32).collect::<Vec<_>>(),
            "topo_order is the identity permutation over topologically-numbered SCCs"
        );
    }

    // === Task D3: solve_batch == solve_group per lane, across >1 batch ====
    // A fixture with > BATCH_WIDTH independent loop groups (forcing >=2 batches),
    // all overlapping on a SHARED recursive SCC (the C<->D cycle) and a shared
    // terminal T. For every group, `solve_batch`'s per-lane aggregates must equal
    // `solve_group`'s on components 1-6, with a structurally-valid witness.
    #[allow(clippy::type_complexity)]
    fn many_groups_fixture(n: usize) -> Fixture {
        // Shared recursive SCC: C <-> D, with the terminal op on D.
        let c = routine("C", "procedure");
        let mut c_only = c;
        c_only.call_sites = vec![call_site("C/csD", "D", vec![])];
        let mut d = routine("D", "procedure");
        d.call_sites = vec![call_site("D/csC", "C", vec![])];
        d.record_operations = vec![record_op(
            "D/op0",
            "Modify",
            "Rec",
            Some("t/D"),
            vec![],
            false,
        )];
        // Shared plain terminal.
        let mut t = routine("T", "procedure");
        t.record_operations = vec![record_op(
            "T/op0",
            "FindSet",
            "Rec",
            Some("t/T"),
            vec![],
            false,
        )];

        let mut routines: Vec<L3Routine> = Vec::new();
        let mut graph_edges: HashMap<String, Vec<CombinedEdge>> = HashMap::new();
        let mut summaries: HashMap<String, FullRoutineSummary> = HashMap::new();

        graph_edges.insert(
            "C".to_string(),
            vec![edge_kind("C", "D", "C/csD", "direct")],
        );
        graph_edges.insert(
            "D".to_string(),
            vec![edge_kind("D", "C", "D/csC", "direct")],
        );
        summaries.insert("C".to_string(), db_summary("C", "t/C"));
        summaries.insert("D".to_string(), db_summary("D", "t/D"));
        summaries.insert("T".to_string(), db_summary("T", "t/T"));

        for i in 0..n {
            let rid = format!("R{i}");
            let aid = format!("A{i}");
            let mut r = routine(&rid, "procedure");
            r.loops = vec![loop_def(&format!("{rid}/loop0"))];
            r.call_sites = vec![call_site(
                &format!("{rid}/cs0"),
                &aid,
                vec![format!("{rid}/loop0")],
            )];
            let a = routine(&aid, "procedure");
            graph_edges.insert(
                rid.clone(),
                vec![edge_kind(&rid, &aid, &format!("{rid}/cs0"), "direct")],
            );
            graph_edges.insert(
                aid.clone(),
                vec![
                    edge_kind(&aid, "T", &format!("{aid}/csT"), "direct"),
                    edge_kind(&aid, "C", &format!("{aid}/csC"), "direct"),
                ],
            );
            summaries.insert(aid.clone(), db_summary(&aid, &format!("t/{aid}")));
            routines.push(r);
            routines.push(a);
        }
        routines.push(c_only);
        routines.push(d);
        routines.push(t);
        (routines, graph_edges, summaries)
    }

    /// Group `seeds`/`direct_ops` EXACTLY as `search_loops` does, chunk the sorted
    /// groups into `BATCH_WIDTH` lanes, and assert every lane of every batch's
    /// `solve_batch` output equals `solve_group`'s for that group on the six
    /// load-bearing components + a valid witness. Panics unless it exercises >1
    /// batch (the chunking boundary) and produces aggregates.
    fn assert_batch_agrees(
        graph: &D1Graph,
        seeds: &[D1Seed],
        direct_ops: &[DirectOp],
        ctx: &DetectorContext,
        cw: &ClosedWorldTempParams,
    ) {
        let liveness = compute_liveness(graph, ctx, cw);
        let scc = condense(graph);

        let mut groups: BTreeMap<(&str, &str), GroupSpec> = BTreeMap::new();
        for (i, seed) in seeds.iter().enumerate() {
            groups
                .entry((seed.loop_routine.id.as_str(), seed.loop_id))
                .or_insert_with(|| GroupSpec {
                    loop_routine: seed.loop_routine,
                    loop_id: seed.loop_id,
                    loop_info: seed.loop_info,
                    seed_indices: Vec::new(),
                    direct_indices: Vec::new(),
                })
                .seed_indices
                .push(i);
        }
        for (i, d) in direct_ops.iter().enumerate() {
            groups
                .entry((d.routine.id.as_str(), d.loop_id))
                .or_insert_with(|| GroupSpec {
                    loop_routine: d.routine,
                    loop_id: d.loop_id,
                    loop_info: d.loop_info,
                    seed_indices: Vec::new(),
                    direct_indices: Vec::new(),
                })
                .direct_indices
                .push(i);
        }
        let group_vec: Vec<GroupSpec> = groups.into_values().collect();
        assert!(
            group_vec.len() > BATCH_WIDTH,
            "fixture must exceed one batch ({} groups) to exercise chunking",
            group_vec.len()
        );

        let mut n_batches = 0usize;
        let mut total = 0usize;
        for chunk in group_vec.chunks(BATCH_WIDTH) {
            n_batches += 1;
            let batch_out = solve_batch(graph, &liveness, &scc, seeds, direct_ops, ctx, cw, chunk);
            for group in chunk {
                let solo = solve_group(
                    graph,
                    &liveness,
                    seeds,
                    direct_ops,
                    ctx,
                    cw,
                    group.loop_routine,
                    group.loop_id,
                    group.loop_info,
                    &group.seed_indices,
                    &group.direct_indices,
                );
                let batch_g: Vec<&LoopTerminalAgg> = batch_out
                    .iter()
                    .filter(|a| {
                        a.loop_routine.id == group.loop_routine.id && a.loop_id == group.loop_id
                    })
                    .collect();

                let gk = (group.loop_routine.id.as_str(), group.loop_id);
                let solo_keys: std::collections::BTreeSet<(&str, &str)> = solo
                    .iter()
                    .map(|a| (a.terminal.owner.id.as_str(), a.terminal.op.id.as_str()))
                    .collect();
                let batch_keys: std::collections::BTreeSet<(&str, &str)> = batch_g
                    .iter()
                    .map(|a| (a.terminal.owner.id.as_str(), a.terminal.op.id.as_str()))
                    .collect();
                assert_eq!(
                    solo_keys, batch_keys,
                    "coverage (1) diverged for group {gk:?}"
                );

                let solo_by: HashMap<(&str, &str), &LoopTerminalAgg> = solo
                    .iter()
                    .map(|a| ((a.terminal.owner.id.as_str(), a.terminal.op.id.as_str()), a))
                    .collect();
                for b in &batch_g {
                    let k = (b.terminal.owner.id.as_str(), b.terminal.op.id.as_str());
                    let s = solo_by[&k];
                    assert_eq!(
                        b.reachable_verdicts, s.reachable_verdicts,
                        "reachable_verdicts (2) diverged for {gk:?}/{k:?}"
                    );
                    assert_eq!(
                        b.severity, s.severity,
                        "severity (3) diverged for {gk:?}/{k:?}"
                    );
                    assert_eq!(
                        b.verdict, s.verdict,
                        "verdict (4) diverged for {gk:?}/{k:?}"
                    );
                    assert_eq!(
                        b.depth_bucket, s.depth_bucket,
                        "depth_bucket (5) diverged for {gk:?}/{k:?}"
                    );
                    assert_eq!(
                        winner_unc(b),
                        winner_unc(s),
                        "unc (6) diverged for {gk:?}/{k:?}"
                    );
                    assert_witness_valid(b, ctx);
                }
                total += batch_g.len();
            }
        }
        assert!(n_batches >= 2, "the fixture must span more than one batch");
        assert!(total > 0, "fixture must produce at least one aggregate");
    }

    #[test]
    fn batch_equals_per_group() {
        // 80 independent loop groups -> two batches (64 + 16), all overlapping on
        // the shared recursive C<->D SCC and the shared terminal T.
        let (routines, graph_edges, summaries) = many_groups_fixture(80);
        let ctx = minimal_ctx(&routines, graph_edges, summaries);
        let workspace = ws(&routines);
        let mut memo = HashMap::new();
        let (graph, seeds) = build_d1_graph(&ctx, &workspace, &mut memo);
        assert_eq!(seeds.len(), 80, "one in-loop seed per group");
        let cw = ClosedWorldTempParams::new();
        assert_batch_agrees(&graph, &seeds, &[], &ctx, &cw);
    }

    // === Task D3 fix: the depth_bucket straddle (saturating severity) ======
    // A `LockTable` terminal — base severity "low", which `severity_for` does
    // NOT bump on depth>=2 (it SATURATES: only high/medium promote) — reached by
    // two EQUAL-hop paths whose summed loop_depth straddles the nested-loop
    // threshold: R->A->B->T carries edge loop_depth 1 (bucket 2), R->A->C->T
    // carries 0 (bucket 1). All of (severity, verdict, unc, hops) tie, so ONLY
    // `depth_bucket` distinguishes the two candidates — and the canonical
    // `selection_rank` must pick the HIGHER (nested-loop) bucket in
    // `process_group`, `solve_group` AND `solve_batch` deterministically, closing
    // the discovery-order divergence in the reported `depth_class`.
    fn straddle_fixture() -> Fixture {
        let mut r = routine("R", "procedure");
        r.loops = vec![loop_def("R/loop0")];
        r.call_sites = vec![call_site("R/cs0", "A", vec!["R/loop0".to_string()])];
        // A has NO loops (it never seeds); its callsites' loop_stacks give the
        // A->B edge loop_depth 1 and the A->C edge loop_depth 0 (via
        // call_site_by_id) — the Task-1 non-zero edge-loop_depth pattern.
        let mut a = routine("A", "procedure");
        a.call_sites = vec![
            call_site("A/csB", "B", vec!["A/loop0".to_string()]),
            call_site("A/csC", "C", vec![]),
        ];
        let b = routine("B", "procedure");
        let c = routine("C", "procedure");
        let mut t = routine("T", "procedure");
        t.record_operations = vec![record_op(
            "T/op0",
            "LockTable",
            "Rec",
            Some("t/T"),
            vec![],
            false,
        )];
        let routines = vec![r, a, b, c, t];

        let mut graph_edges: HashMap<String, Vec<CombinedEdge>> = HashMap::new();
        graph_edges.insert(
            "R".to_string(),
            vec![edge_kind("R", "A", "R/cs0", "direct")],
        );
        graph_edges.insert(
            "A".to_string(),
            vec![
                edge_kind("A", "B", "A/csB", "direct"),
                edge_kind("A", "C", "A/csC", "direct"),
            ],
        );
        graph_edges.insert(
            "B".to_string(),
            vec![edge_kind("B", "T", "B/csT", "direct")],
        );
        graph_edges.insert(
            "C".to_string(),
            vec![edge_kind("C", "T", "C/csT", "direct")],
        );
        let summaries: HashMap<String, FullRoutineSummary> = ["A", "B", "C", "T"]
            .iter()
            .map(|id| (id.to_string(), db_summary(id, &format!("t/{id}"))))
            .collect();
        (routines, graph_edges, summaries)
    }

    #[test]
    fn depth_bucket_straddle_prefers_nested_loop() {
        let (routines, graph_edges, summaries) = straddle_fixture();
        let ctx = minimal_ctx(&routines, graph_edges, summaries);
        let workspace = ws(&routines);
        let mut memo = HashMap::new();
        let (graph, seeds) = build_d1_graph(&ctx, &workspace, &mut memo);
        assert_eq!(seeds.len(), 1, "only R/cs0 -> A seeds");
        let cw = ClosedWorldTempParams::new();
        let liveness = compute_liveness(&graph, &ctx, &cw);
        let scc = condense(&graph);
        let seed_indices: Vec<usize> = (0..seeds.len()).collect();

        let oracle = process_group(
            &graph,
            &seeds,
            &[],
            &ctx,
            &cw,
            seeds[0].loop_routine,
            seeds[0].loop_id,
            seeds[0].loop_info,
            &seed_indices,
            &[],
        );
        let solo = solve_group(
            &graph,
            &liveness,
            &seeds,
            &[],
            &ctx,
            &cw,
            seeds[0].loop_routine,
            seeds[0].loop_id,
            seeds[0].loop_info,
            &seed_indices,
            &[],
        );
        let group = GroupSpec {
            loop_routine: seeds[0].loop_routine,
            loop_id: seeds[0].loop_id,
            loop_info: seeds[0].loop_info,
            seed_indices: seed_indices.clone(),
            direct_indices: Vec::new(),
        };
        let batch = solve_batch(
            &graph,
            &liveness,
            &scc,
            &seeds,
            &[],
            &ctx,
            &cw,
            std::slice::from_ref(&group),
        );

        for (name, aggs) in [
            ("process_group", &oracle),
            ("solve_group", &solo),
            ("solve_batch", &batch),
        ] {
            assert_eq!(aggs.len(), 1, "{name}: one (loop, LockTable) aggregate");
            let agg = &aggs[0];
            assert_eq!(
                agg.terminal.op.id, "T/op0",
                "{name}: the LockTable terminal"
            );
            assert_eq!(
                agg.severity, "low",
                "{name}: LockTable base severity, saturated (no depth>=2 bump)"
            );
            assert_eq!(
                agg.depth_bucket, 2,
                "{name}: the HIGHER (nested-loop) bucket wins the straddle, deterministically"
            );
        }

        // The two dataflow engines agree with the oracle on all six components
        // (now including the canonical, discovery-independent depth_bucket).
        assert_agrees(&graph, &seeds, &[], &ctx, &cw);
    }
}
