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

use std::collections::{BTreeMap, HashMap, VecDeque};

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
use crate::engine::l5::d1_temp::{ParamTemp, TempVec, cross_hop, resolve_terminal, root_state};
use crate::engine::l5::detector_context::DetectorContext;
use crate::engine::l5::detectors::d1::{
    TempVerdict, hop_step, is_setup_singleton_get, severity_for, terminal_step,
};
use crate::engine::l5::finding::EvidenceStep;

/// Look up `idx` in a sorted sparse [`TempVec`]; absent -> `Unknown` (mirrors
/// `d1_temp`'s own private `lookup`; the seed-entry projection needs it to read
/// the live params off `cross_hop`'s output vector).
fn temp_lookup(v: &TempVec, idx: u32) -> ParamTemp {
    v.iter()
        .find(|&&(i, _)| i == idx)
        .map(|&(_, pt)| pt)
        .unwrap_or(ParamTemp::Unknown)
}

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
            let class = temp_lookup(&entry_temp, p);
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
            .max_by_key(|c| selection_rank(c.severity, c.verdict, c.unc, c.hops, c.discovery))
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

                assert_witness_valid(s);
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
    /// step in the loop routine (a loop step), last step the terminal op, and
    /// the intermediate steps a `[call, hop*]` chain.
    fn assert_witness_valid(agg: &LoopTerminalAgg) {
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
            for step in &w[1..w.len() - 1] {
                assert!(
                    step.callsite_id.is_some(),
                    "the call + hop steps each carry a callsite"
                );
                assert!(step.loop_id.is_none(), "non-loop intermediate steps");
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
}
