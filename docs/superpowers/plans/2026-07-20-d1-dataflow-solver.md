# d1 Dataflow Solver Implementation Plan (Task 8 / 7b)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Replace d1's per-loop-group product-state BFS (`search_loops`/`process_group`
in `src/engine/l5/d1_reach.rs`) — which re-traverses the dense 797-member SCC once per
loop group and does NOT finish at Base App 8020 density (>1h10m, 42.8 GB, killed even
parallelized) — with a **batched per-parameter dataflow solver** that solves the SCC once
per batch of 64 loop groups, making 8020 finish in bounded time and memory.

**Architecture:** Exploit that d1's temp propagation is **unary** — each terminal reads
≤1 parameter (`d1_temp.rs::resolve_terminal`), each callee parameter is a function of ≤1
caller parameter or a constant (`d1_temp.rs::cross_hop`, `d1_temp.rs:121-169`), and graph
expansion never branches on temp values. So the joint `TempVec` (3^params multiplicity,
~1M labels/SCC-search) is unnecessary. Decompose into independent facts, each carrying a
u64 group-bitset (64 loop groups per batch):

```text
reach[node][depth 0..2][unc 0..1]                          -> u64 group mask
value[node][live_param][Temp|Physical|Unknown][depth][unc] -> u64 group mask
```

Solve the call-graph SCC condensation once per batch via a monotone least-fixpoint. Temp
VALUES are not monotone (a param can flip Temp↔Physical around a cycle), but the GROUP-SETS
realizing each fact ARE union-only-monotone (bits never removed) → a standard finite
fixpoint over the ORIGINAL call SCC, no product-SCC construction needed. Score terminals
during propagation; materialize one shortest witness per (group, terminal) via per-lane
first-arrival predecessors.

**Tech Stack:** Rust, existing engine modules. No new deps.

## Design provenance

External design (gpt-5.6-sol round 3), full text in the durable memory note
`d1-output-bound-falsified` (§2026-07-20). This plan is gpt's 9-step priority order turned
into SDD tasks. The load-bearing correctness claim (unary temp ⇒ per-parameter facts are
exact) is verified against `d1_temp.rs::cross_hop`/`resolve_terminal`.

## Global Constraints

- Package `al-call-hierarchy` (HYPHEN) for `cargo test -p`.
- rustfmt per file, NEVER `cargo fmt`. `cargo clippy --all-targets --all-features` clean.
- Goldens: `scripts/check-goldens --regen` regenerates ALL five families; regen is a
  MEASUREMENT — triage, never blind-bless. Pre-commit hook enforces check-goldens.
- Fresh resolver (`src/program/`) untouched; north-star SHA `0a3b85bc…` must reproduce.
- Never pipe a gate through `| tail` (exit code lies) — redirect to a log + grep.
- Measurement: `cargo build --profile release-fast`; kill stale `alsem.exe` first; quiet
  machine; DO workspace `U:/Git/DO.Support-SlowDOSetup/DocumentOutput/Cloud`; 8020 corpus
  at `…/scratchpad/corpus-8020` (8020 .al files, already built); detector flag is
  `--detector` (SINGULAR, comma-sep); trace `ALSEM_TRACE=1 ALSEM_TRACE_DETAIL=stages,hot`.
- CHANGELOG.md updated. Stage explicitly; never `git add -A`. Never push/merge master.
- Branch: continue on `feat/d1-reachability` (Tasks 1-5 + 7a already landed there).

## The correctness spine (read before any task)

The current per-group search (`process_group`, `d1_reach.rs`) is the ORACLE. The dataflow
solver must reproduce, for EVERY (loop, terminal-op) it emits, these SIX components
IDENTICALLY:

```text
1. coverage — the set of (loop_id, terminal_owner_id, op_id) emitted
2. reachable_verdicts (sorted set)
3. severity          4. verdict          5. depth_bucket (winner)   6. unc (winner)
```

Only component 7 — the WITNESS path / entry_callsite / reported true-depth / uncertainty
vector / first-discovery tie — MAY differ (the fact abstraction may pick a different
equal-ranked witness). A change in items 1-6 is a BUG, not an improvement — debug it, do
not rebaseline. (This is TIGHTER than the Task-5 cutover, which changed semantics
deliberately; the dataflow solver changes only the ALGORITHM, so 1-6 must hold.)

`process_group` stays ALIVE as the differential oracle through Tasks D1-D4; deleted only at
the D5 cutover (like the old walker was kept for Task 4's shadow oracle).

## File Structure

- Create `src/engine/l5/d1_liveness.rs` — `Need[node]` param-liveness fixpoint + compiled
  `ParamTransfer` per edge.
- Create `src/engine/l5/d1_dataflow.rs` — the fact solver (reach/value facts, seeding,
  SCC-scheduled delta propagation, terminal scoring, witness provenance).
- Modify `src/engine/l5/d1_reach.rs` — `search_loops` becomes the batch driver;
  `process_group` retained cfg(test) as oracle until D5, then deleted with 7a's rayon.
- Modify `src/engine/l5/mod.rs` — register the two new modules.

---

### Task D1: `d1_liveness` — Need[node] fixpoint + compiled ParamTransfer

**Files:**
- Create: `src/engine/l5/d1_liveness.rs`
- Modify: `src/engine/l5/mod.rs`

**Interfaces:**
- Consumes: `D1Graph` (`d1_graph.rs`: `node_ids`, `edges` with `D1Edge{to, callsite_id,
  loop_depth, binding_ok}`, `terminals` with `D1Terminal{op, owner, local_depth}`),
  `L3Routine.call_sites[*].argument_bindings` (`parameter_index`, `source_temp_state`,
  `source_parameter_index`), `ClosedWorldTempParams`, `d1_temp::{ParamTemp}`.
- Produces:

```rust
/// The single-parameter transfer an edge applies to ONE needed callee parameter.
/// Exhaustive per d1_temp::cross_hop's per-param outcomes (d1_temp.rs:121-169):
/// closed-world-proven and constants collapse to a value; a PD binding copies one
/// caller slot; everything else is Unknown.
pub(crate) enum ParamTransfer {
    Const(ParamTemp),          // Temp (incl. proven) | Physical | Unknown
    Copy { caller_slot: u16 }, // callee param = caller param at this Need-slot
}

/// Per node: the ordered list of downstream-observable (live) parameter indices.
/// Slot i in a node's value-facts corresponds to need[node][i].
pub(crate) struct Liveness {
    pub need: Vec<Vec<u32>>,                       // NodeIx -> sorted live param indices
    pub slot_of: Vec<HashMap<u32, u16>>,           // NodeIx -> (param_index -> slot)
    /// Per edge (indexed as graph.edges[from][k]): transfers for the CALLEE's live
    /// params, in callee-slot order. `Const(Unknown)` for a callee live param with no
    /// usable binding / non-binding edge.
    pub edge_transfers: Vec<Vec<Vec<ParamTransfer>>>, // [from][k] -> per callee-slot transfer
    /// Per terminal: the single param slot its op reads, or None for a constant
    /// terminal temp state (resolve_terminal reads <=1 index).
    pub terminal_reads: Vec<Vec<Option<u16>>>,     // [node][t] -> callee-slot read by terminal t
}

pub(crate) fn compute_liveness<'a>(
    graph: &D1Graph<'a>,
    ctx: &DetectorContext,
    cw: &ClosedWorldTempParams,
) -> Liveness;
```

Backward fixpoint (gpt Phase 1): `Need[n] = {terminal PD indices read at n} ∪ {caller
param j : edge n→m binding-carrying (binding_ok), callee param p ∈ Need[m],
binding(p)=PD(j)}`. Proven/const/unknown/non-binding callee params contribute no caller
need. Iterate to fixpoint (within call SCCs). The op's read index: an op with
`temp_state = PD(i)` reads param i; `Known`/`None` reads nothing (`None` slot).

- [ ] Step 1: Write failing tests (in `d1_liveness.rs` tests; use `test_support`
  constructors mirrored from `d1_graph.rs`/`d1_reach.rs` tests — read them first):
  - `need_is_backward_closure`: chain A→B→T where T's op is PD(0) bound through B's
    param via a PD binding to A's param → Need[A] contains that param; an unrelated
    param is NOT in Need.
  - `const_and_unknown_bindings_need_no_caller`: a callee param bound to a temp literal
    (Const) or with no binding (Unknown) puts nothing into the caller's Need.
  - `proven_param_needs_no_caller`: a closed-world-proven callee param → Const(Temp), no
    caller need.
  - `transfer_matches_cross_hop_per_param`: for a small graph, EACH compiled
    `ParamTransfer` applied to a caller `ParamTemp` produces the SAME single-param result
    as `d1_temp::cross_hop` produces for that param (the per-param equivalence oracle).
- [ ] Step 2: Run RED (`cargo test -p al-call-hierarchy --lib d1_liveness:: 2>&1 | grep -E "error|test result"`).
- [ ] Step 3: Implement the fixpoint + transfer compilation.
- [ ] Step 4: GREEN. Full suite stays green (new module, no consumer yet).
- [ ] Step 5: rustfmt + clippy + commit.

---

### Task D2: `d1_dataflow` single-group fact solver, differential vs process_group

**Files:**
- Create: `src/engine/l5/d1_dataflow.rs`
- Modify: `src/engine/l5/mod.rs`

**Interfaces:**
- Consumes: `D1Graph`, `Liveness` (D1), `D1Seed`/`DirectOp` (`d1_reach.rs` — make
  `pub(crate)` if needed), `d1_temp::{root_state, resolve_terminal, ParamTemp}`,
  `d1_reach`'s selection helpers (`severity_for` via terminal, `verdict_quality`,
  `flowfield_verdict`, `TempVerdict`, `LoopTerminalAgg`) — extract the ones D2 needs to
  `pub(crate)`.
- Produces:

```rust
/// Solve ONE loop group with the fact model; returns the SAME Vec<LoopTerminalAgg>
/// process_group returns (components 1-6 identical; witness may differ).
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
) -> Vec<LoopTerminalAgg<'a>>;
```

Single-group facts use a 1-bit "mask" (this group only) — the batching in D3 widens the
mask to u64. Solve: seed init (reach + value facts at each seed entry, from `root_state`
+ the seed callsite transfer), level-synchronous propagation over the filtered graph
(BFS by hops → first arrival = shortest), delta worklist to fixpoint (cycles terminate
because facts only gain — never lose — presence). Score each terminal from reach (const
terminal state) or the read value-fact (PD terminal) + FlowField gate + local depth →
severity; select winner by the existing rule; materialize witness via first-arrival
predecessors. Direct ops enter the accumulator first (as today).

- [ ] Step 1: Write the DIFFERENTIAL test FIRST — the spine. On every fixture the
  `d1_reach` tests build (reuse them), assert `solve_group` and `process_group` agree on
  components 1-6 for every emitted (loop, terminal-op): coverage set equal;
  reachable_verdicts equal; severity/verdict/depth_bucket/unc equal. Assert the witness
  is a VALID path (first step in the loop routine, last step the terminal op, hop count
  == reported). Cover: the budget-buster fanout, depth-2-beats-depth-1, physical-beats-
  temp multi-seed, cycle, direct+transitive adjudication. Non-vacuous (assert >0
  aggregates per fixture).
- [ ] Step 2: RED.
- [ ] Step 3: Implement `solve_group`.
- [ ] Step 4: GREEN — the differential passes on all fixtures. Full suite green.
- [ ] Step 5: rustfmt + clippy + commit.

---

### Task D3: batch driver — 64-lane group bitsets + call-SCC scheduler

**Files:**
- Modify: `src/engine/l5/d1_dataflow.rs` (widen masks to u64; add batch solve + SCC
  condensation scheduler), `src/engine/l5/d1_reach.rs` (`search_loops` → batch driver).

**Interfaces:**
- Produces:

```rust
/// Solve a BATCH of up to 64 groups sharing one SCC-condensation pass. Group i in the
/// batch owns bit i. Returns aggregates for all groups in the batch.
pub(crate) fn solve_batch<'a>(
    graph: &D1Graph<'a>, liveness: &Liveness, scc: &CallScc,
    seeds: &[D1Seed<'a>], direct_ops: &[DirectOp<'a>], ctx: &'a DetectorContext,
    cw: &ClosedWorldTempParams, batch: &[GroupSpec<'a>],   // <=64 groups
) -> Vec<LoopTerminalAgg<'a>>;

/// Call-graph SCC condensation over the filtered D1Graph (Tarjan; deterministic).
pub(crate) struct CallScc { /* node->scc, scc topo order, scc members */ }
pub(crate) fn condense(graph: &D1Graph) -> CallScc;
```

Facts become `u64` masks (`reach[node][depth*2+unc] -> u64`; value facts
`node × slot × {T,P,U} × depth × unc -> u64`). Seed each batch group into its lane. Solve
SCCs in topological order; within an SCC run a delta worklist over newly-set bits to
least-fixpoint (monotone: bits only added). Per-lane first-arrival predecessor for
witness. `search_loops`: assign deterministic `GroupIx` from the existing sorted
`(loop_routine_id, loop_id)` order, chunk into 64-lane batches, solve each batch SERIALLY,
concatenate, keep the final total-order sort. **Disable 7a's rayon `par_iter`** (batches
run serially — the RSS fix).

- [ ] Step 1: Write failing tests: (a) `batch_equals_per_group` — for a multi-group
  fixture (>64 groups to force >1 batch, or a small batch-width override for the test),
  `solve_batch` results equal `solve_group` per group on components 1-6; (b)
  `condensation_deterministic` — `condense` gives identical SCC ids/topo order across
  runs; (c) `search_loops_matches_process_group` — the full new `search_loops` equals a
  reference that runs `process_group` per group (components 1-6), on the existing
  fixtures.
- [ ] Step 2: RED.
- [ ] Step 3: Implement `condense`, `solve_batch`, rewire `search_loops`.
- [ ] Step 4: GREEN. Full suite green. Goldens: run `scripts/check-goldens` — components
  1-6 identical means goldens should move ONLY in witness/path fields (if at all). If a
  golden's severity/verdict/coverage moved → STOP, it's a bug (violates the spine).
- [ ] Step 5: rustfmt + clippy + commit.

---

### Task D4: witness materialization parity + provenance memory bound

**Files:** Modify `src/engine/l5/d1_dataflow.rs`.

Fold into D3 if witness materialization already lands there; otherwise this task hardens
it: per (fact, lane) first-arrival `PackedPred{predecessor_fact: u32, edge_or_seed: u32}`;
materialize by walking the chain, prepend the winning seed's `[loop_step, call_step]`,
append hop steps + terminal step, sum TRUE (unclamped) edge depths, union actual node
uncertainties along the ONE witness (preserving the search's bool `unc` vs materialized
uncertainty-vector distinction). Provenance bounded by `reached_facts × 64 × 8 bytes`;
drop the batch arena after emitting its aggregates.

- [ ] Step 1: Test: witness of every emitted aggregate is a valid realizing path AND its
  reported true-depth = sum of true edge depths along it (not the clamped bucket);
  uncertainty vector = deduped union along the witness. On a fixture with a bucket-2
  winner whose true depth is >2, assert the reported depth is the true value.
- [ ] Steps 2-5: RED → implement → GREEN → full suite → commit.

---

### Task D5: cutover — delete per-group BFS + 7a rayon; goldens; DO diff

**Files:** Modify `src/engine/l5/d1_reach.rs`, CHANGELOG.md, goldens.

- [ ] Step 1: Generate DO baseline with the CURRENT (pre-D5) binary:
  `cargo build --profile release-fast`; kill stale alsem; run
  `alsem analyze <DO> --format json > logs/d1-do-preD5.json`.
- [ ] Step 2: Delete `process_group` (the per-group BFS), its `LabelRec`/`seen`
  machinery, and 7a's `par_iter` (now unused). Keep `solve_group` as the cfg(test)
  differential oracle IF the D2/D3 tests still reference it; else delete. `search_loops`
  is now purely the batch driver.
- [ ] Step 3: Full suite. `scripts/check-goldens --regen` → TRIAGE with the 7-component
  partition: verify components 1-6 UNCHANGED for every retained (loop, terminal-op) (this
  is the spine — a 1-6 change is a STOP-and-debug, never a rebaseline); accept only
  witness/path (item 7) diffs, and inspect a sample against fixture source. Then
  `scripts/check-goldens` green.
- [ ] Step 4: DO semantic diff vs `logs/d1-do-preD5.json`: components 1-6 must be
  byte-identical modulo witness fields; runtime in the DO band. Any severity/coverage/
  verdict move = STOP.
- [ ] Step 5: CHANGELOG (Changed: d1 search replaced by batched dataflow solver — same
  findings, scalable; Removed: per-group BFS + 7a rayon). rustfmt + clippy + commit.

---

### Task D6: 8020 measurement — the finish bar

**Files:** Modify `docs/superpowers/specs/2026-07-19-perf-optimization-handoff.md`.

- [ ] Step 1: Quiet machine; kill stale alsem; `cargo build --profile release-fast`.
- [ ] Step 2: d1-only 8020 with `ALSEM_TRACE_DETAIL=stages,hot`, run in background, poll
  the log; record whether d1 FINISHES (census emitted + process exits + non-empty JSON),
  d1 wall, peak RSS, and the new fact-count/reach checkpoints. Compare to the killed
  serial (>4h) and 7a (>1h10m/42.8GB) baselines.
- [ ] Step 3: Full-default 8020 — the arc finish bar. Background; record FINISHED y/n,
  total wall, peak RSS, per-detector tail.
- [ ] Step 4: Regression bands: DO default (band 9-11s; Task 5 measured 5.23s) + 8020
  3-det (`--detector d61-ishandled-bypasses-critical-write,d62-telemetry-before-success,d64-api-page-write-surface`).
- [ ] Step 5: Update the handoff doc (§0/§2 with the after-numbers; §5 queue: d1 done,
  B1-narrow next) + commit. If d1 STILL doesn't finish or RSS exceeds the substrate
  headroom, report the fact-count checkpoints and reduce batch width (32→16) before any
  further design — measure-before-build.

---

### Post-plan user gates (not tasks)

- `scripts/cdo-gate <CDO_WS>` — north-star SHA `0a3b85bc…` reproduces (fresh resolver
  untouched).
- CDO d1 finding-level triage: since components 1-6 are preserved, expect the CDO d1
  findings identical to the Task-5 cutover modulo witness fields.

## Self-review notes

- The spine (components 1-6 identical) is asserted at D2 (single-group), D3 (batched), and
  D5 (goldens + DO) — three independent gates before the old code is deleted.
- Unary-temp is the load-bearing premise (per-param facts are exact); D1's
  `transfer_matches_cross_hop_per_param` and D2's differential are its proof.
- Memory: serial batches (no rayon) + per-batch arena drop + u64 lanes bound RSS by batch
  width; D6 Step 5 has the 32→16 fallback if the dense-SCC batch is too fat.
- Parallelism deliberately ABSENT (7a's rayon removed) — reintroduce at most 2 concurrent
  batches ONLY if D6 shows d1 CPU-bound with safe RSS (a future task, licensed by
  measurement).
