# d1 Reachability Redesign Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace d1-db-op-in-loop's exhaustive simple-path enumeration (~3 h at
Base-App 8020 density) with unbounded filtered reachability + finite-state
aggregation, emitting terminal-centric findings with structured per-loop
contexts — faster (target: seconds-to-low-minutes) AND semantically stronger
(no silent 500-node budget truncation, no DFS-order-accidental verdicts, no
canonical-loop merge accidents).

**Architecture:** Build one compact filtered graph (interned u32 nodes, CSR in
original edge order) from the existing `DetectorContext`. Per loop, run a
multi-source label search over product states `(node, param-temp-vector,
depth-bucket≤2, uncertainty-flag)`; per `(loop, terminal-op)` aggregate the
reachable verdict set, pick the max-realizable severity, and select the
strongest-confidence shortest witness REALIZING that severity. Assemble one
finding per terminal op with `contexts[]` (one per loop), severity + top-level
confidence from the same winning context. The old walker path in d1 is deleted;
`path_walker.rs` itself stays (d2/d46/d48 use it).

**Tech Stack:** Rust, existing engine modules (`src/engine/l5/*`), no new
dependencies.

## Design provenance (read before starting)

- User decisions (2026-07-19): NO output caps; byte-parity NOT required (we own
  all consumers); terminal-centric schema chosen ("format supporting best way
  forward + most use cases long term").
- Two external design reviews (gpt-5.6-sol) endorsed and refined this design.
  Load-bearing conclusions are restated inline below; the session-scratch
  verdict files are ephemeral.
- The falsified premise this replaces: "d1 is output-bound" — WRONG. Output
  never contained the ~900k enumerated paths (first-wins dedupe by
  `d1/{loop}/{routine}/{op}` at `src/engine/l5/detectors/d1.rs:1328-1336`
  discarded them). See `docs/superpowers/specs/2026-07-19-perf-optimization-handoff.md`
  §4 (now resolved) and the memory note `d1-output-bound-falsified`.

## Global Constraints

- Package name is `al-call-hierarchy` (HYPHEN) for `cargo test -p`.
- Format per-file with `rustfmt <file>`, NEVER `cargo fmt`.
- `cargo clippy --all-targets --all-features` must stay clean.
- Goldens: `scripts/check-goldens --regen` regenerates ALL five families
  together; NEVER regen one family alone. Regen is a measurement — inspect and
  TRIAGE the diff, never blind-rebless. Pre-commit hook enforces check-goldens.
- Fresh resolver (`src/program/`) untouched; north-star SHA `0a3b85bc…` must
  still reproduce (user runs `scripts/cdo-gate` — not runnable in sandbox).
- `REGEN_TEMP_GOLDENS=1 cargo test` is value-tested (`=1` exactly).
- Never pipe long gates through `| tail` (exit code lies); redirect to a log.
- Measurement builds: `cargo build --profile release-fast`; kill stale
  `alsem.exe` first; quiet machine (`Get-CimInstance Win32_Processor` load
  < ~15%); DO workspace: `U:/Git/DO.Support-SlowDOSetup/DocumentOutput/Cloud`;
  8020 corpus recipe: Wave-1 plan Global Constraints
  (`docs/superpowers/plans/2026-07-18-engine-memory-speed-wave1.md`).
- CHANGELOG.md updated (Keep a Changelog; Added/Changed/Removed/Fixed groups).
- Stage files explicitly; never `git add -A`. Never push/merge to master
  unrequested.
- DO regression bands: DO default 9.0-10.7 s / ~1.6 GB; 8020 3-det 41-45 s.

## Migration doctrine (from external review, adopted)

ONE externally visible semantic change, implemented in internal stages:
Tasks 1-4 are output-invisible (new modules + shadow oracles). Task 5 assembles
the new output behind the existing entry point in one cutover with goldens
regen + partitioned triage. Old d1 walk path is deleted at cutover — it
survives only as the shadow oracle inside tests until then.

Monotonicity oracles (the shadow contract; final finding IDs are NOT monotonic
so never assert those):

```text
old premerge (loop_id, terminal_routine_id, operation_id) ⊆ new
old rootCauseKeys ⊆ new rootCauseKeys
new severity_rank[rootCauseKey] >= old severity_rank[rootCauseKey]
```

Severity rank order: info(0) < low(1) < medium(2) < high(3) < critical(4).

## File Structure

- Create `src/engine/l5/d1_graph.rs` — compact filtered graph + terminals + seeds.
- Create `src/engine/l5/d1_temp.rs` — forward param-temp vector state + transition
  (forward-equivalent of `resolve_temp_along_path_closed_world`).
- Create `src/engine/l5/d1_reach.rs` — per-loop product-state search + per
  `(loop, terminal-op)` aggregation + witness materialization.
- Modify `src/engine/l5/finding.rs` — add `LoopContext` + `Finding.contexts` +
  stable mirror.
- Modify `src/engine/l5/detectors/d1.rs` — cutover: new pipeline replaces walk
  consumption; direct-op branch folds into the same aggregation; dead code
  removed.
- Modify `src/engine/l5/mod.rs` — register the three new modules.
- Tests: unit tests inside each new module (mirroring `d1.rs`'s existing
  `memo_tests` fixture style via `crate::engine::l5::test_support`), plus the
  shadow differential in `d1.rs` tests.

---

### Task 1: `d1_graph` — compact filtered graph, terminals, seeds

**Files:**
- Create: `src/engine/l5/d1_graph.rs`
- Modify: `src/engine/l5/mod.rs` (add `pub(crate) mod d1_graph;`)

**Interfaces:**
- Consumes: `DetectorContext` fields (`routine_by_id`, `table_by_id`,
  `summaries`, `graph.edges_by_from`, `call_site_by_id`), d1 helpers
  `edge_target_matches_callsite_callee`, `is_db_touching_class`, `classify_op`,
  `is_terminator_next`, `op_targets_virtual_system_table`, `touches_db_of`
  (make these `pub(crate)` in `d1.rs` where currently private).
- Produces (used by Tasks 3, 5):

```rust
pub(crate) type NodeIx = u32;

pub(crate) struct D1Edge<'a> {
    pub to: NodeIx,
    pub kind: &'a str,
    pub callsite_id: Option<&'a str>,
    /// `loop_depth_of_edge` semantics: call_site_by_id[cs].loop_stack.len(), else 0.
    pub loop_depth: i64,
    /// kind ∈ {direct, method, implicit-trigger} — the temp-binding allowlist.
    pub binding_ok: bool,
}

pub(crate) struct D1Terminal<'a> {
    pub op: &'a L3RecordOperation,
    pub owner: &'a L3Routine,
    pub local_depth: i64, // op.loop_stack.len()
}

pub(crate) struct D1Graph<'a> {
    pub node_ids: Vec<&'a str>,                 // NodeIx -> routine internal id
    pub node_ix: HashMap<&'a str, NodeIx>,
    pub edges: Vec<Vec<D1Edge<'a>>>,            // filtered, ORIGINAL edges_by_from order
    pub terminals: Vec<Vec<D1Terminal<'a>>>,    // per node, record_operations order
}

pub(crate) struct D1Seed<'a> {
    pub loop_routine: &'a L3Routine,
    pub loop_id: &'a str,                       // representative (innermost) loop
    pub loop_info: &'a PLoop,
    pub callsite: &'a PCallSite,
    pub entry: NodeIx,
    pub entry_edge_kind: &'a str,
    pub seed_depth: i64,                        // cs.loop_stack.len()
}

pub(crate) fn build_d1_graph<'a>(
    ctx: &'a DetectorContext,
    ws: &'a L3Workspace,
    touches_db_memo: &mut HashMap<&'a str, EffectPresence>,
) -> (D1Graph<'a>, Vec<D1Seed<'a>>);
```

Semantics locked here (each mirrors an existing rule — cite kept in doc
comments):
- Edge filter == old `D1Policy::expand` (`d1.rs:657-675`): drop
  `kind == "event-dispatch"`; drop targets with no summary or
  `touches_db == No` (memoized via the passed-in memo, same `touches_db_of`).
- Terminal filter == old `terminals_at` (`d1.rs:632-655`): db-touching class,
  not `is_terminator_next`, not `op_targets_virtual_system_table`.
- Seed ladder == old branch (b) (`d1.rs:1094-1139`): in-loop callsite,
  representative loop resolvable in `routine.loops`, edge resolved by
  callsite-id + G-18 target-name match, skip `interface`/`dynamic` kinds, skip
  calle summaries missing or `touches_db == No`. Routine gates: `body_available`
  and `!parse_incomplete` (`d1.rs:984-990`).
- Node universe = closure from all seed entries over filtered edges (BFS,
  insertion order = discovery order — deterministic).

- [ ] **Step 1: Write failing tests** (in `d1_graph.rs` `#[cfg(test)] mod tests`,
  fixtures built with `crate::engine::l5::test_support` exactly as
  `d1.rs::memo_tests` does — read that module first and reuse its
  routine/edge/summary/fact constructors):

```rust
#[test]
fn edge_filter_drops_event_dispatch_and_non_db_targets() {
    // Graph: A -> B (direct, B touches db), A -> C (event-dispatch, C touches db),
    // A -> D (direct, D touches_db == No). Seed: loop in L calls A (in-loop).
    // Expect: closure nodes {A, B}; A's edge list == [B] only.
}

#[test]
fn terminals_respect_g1_g6_filters() {
    // B has ops: [Get (db), Next-terminator (excluded G-1), Get on virtual table
    // (excluded G-6)]. Expect terminals(B) == [the plain Get] in op order.
}

#[test]
fn seed_ladder_matches_branch_b_skips() {
    // L has 3 in-loop callsites: cs1 -> resolvable direct edge to A (kept),
    // cs2 -> interface edge (skipped), cs3 -> callee with touches_db == No
    // (skipped). Expect exactly one D1Seed{entry == A, seed_depth == 1}.
}

#[test]
fn closure_is_reachable_only_and_deterministic() {
    // A -> B -> T; unrelated X -> Y never seeded. Two builds produce identical
    // node_ids ordering (discovery order).
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p al-call-hierarchy --lib d1_graph:: 2>&1 | grep -E "error|FAILED|test result"`
Expected: compile error (module/types not defined yet).

- [ ] **Step 3: Implement `d1_graph.rs`** per the Produces block. Core build loop:

```rust
pub(crate) fn build_d1_graph<'a>(
    ctx: &'a DetectorContext,
    ws: &'a L3Workspace,
    touches_db_memo: &mut HashMap<&'a str, EffectPresence>,
) -> (D1Graph<'a>, Vec<D1Seed<'a>>) {
    let mut seeds: Vec<D1Seed<'a>> = Vec::new();
    for routine in &ws.routines {
        if !routine.body_available || routine.parse_incomplete {
            continue;
        }
        let loop_by_id: HashMap<&str, &PLoop> =
            routine.loops.iter().map(|l| (l.id.as_str(), l)).collect();
        for cs in &routine.call_sites {
            if cs.loop_stack.is_empty() {
                continue;
            }
            let Some(rep) = cs.loop_stack.last().map(|s| s.as_str()) else { continue };
            let Some(loop_info) = loop_by_id.get(rep).copied() else { continue };
            let edge = ctx.graph.edges_by_from.get(&routine.id).and_then(|edges| {
                edges.iter().find(|e| {
                    e.callsite_id.as_deref() == Some(cs.id.as_str())
                        && edge_target_matches_callsite_callee(e, cs, &ctx.routine_by_id)
                })
            });
            let Some(edge) = edge else { continue };
            if edge.kind == "interface" || edge.kind == "dynamic" {
                continue;
            }
            let Some(sum) = ctx.summaries.get(&edge.to) else { continue };
            if memoized_touches_db(touches_db_memo, sum) == EffectPresence::No {
                continue;
            }
            seeds.push(D1Seed {
                loop_routine: routine,
                loop_id: rep,
                loop_info,
                callsite: cs,
                entry: NodeIx::MAX, // patched after interning below
                entry_edge_kind: edge.kind.as_str(),
                seed_depth: cs.loop_stack.len() as i64,
            });
            // remember edge.to for the closure frontier (collect alongside)
        }
    }
    // BFS closure over filtered edges from the distinct seed-entry routine ids,
    // interning nodes in discovery order; then fill edges/terminals per node and
    // patch each seed.entry from node_ix. Edge filter + terminal filter exactly
    // as the Semantics block above. loop_depth via ctx.call_site_by_id.
    // (Straight-line code; no recursion needed.)
    ...
}
```

(The `...` is the mechanical BFS + interning described immediately above it —
implement inline, ~60 lines; tests define the contract.)

- [ ] **Step 4: Run tests to verify pass**

Run: `cargo test -p al-call-hierarchy --lib d1_graph:: 2>&1 | grep "test result"`
Expected: `ok. 4 passed`

- [ ] **Step 5: rustfmt + clippy + commit**

```bash
rustfmt src/engine/l5/d1_graph.rs src/engine/l5/mod.rs
cargo clippy --all-targets --all-features 2>&1 | grep -E "^error|^warning" ; true
git add src/engine/l5/d1_graph.rs src/engine/l5/mod.rs src/engine/l5/detectors/d1.rs
git commit -m "feat(l5): d1_graph — compact filtered graph, terminals, seeds for d1 reachability"
```

---

### Task 2: `d1_temp` — forward param-temp vector, proven equivalent to the backward resolver

**Files:**
- Create: `src/engine/l5/d1_temp.rs`
- Modify: `src/engine/l5/mod.rs`

**Interfaces:**
- Consumes: `L3Routine.call_sites[*].argument_bindings`
  (`parameter_index`, `source_temp_state`, `source_parameter_index`),
  `TempStateKind`, `ClosedWorldTempParams`,
  `resolve_temp_along_path_closed_world` (oracle only, in tests).
- Produces (used by Task 3):

```rust
/// Concrete resolved temp-ness of one callee parameter, forward-composed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum ParamTemp { Temp, Physical, Unknown }

/// Sorted-by-index sparse vector; params absent => Unknown.
pub(crate) type TempVec = smallvec::SmallVec<[(u32, ParamTemp); 4]>;
// (if smallvec is not already a dependency, use Vec<(u32, ParamTemp)> — do NOT
// add a dependency for this)

/// Root frame state: every param p is Temp iff closed-world proven (routine, p),
/// else Unknown (root-PD rule).
pub(crate) fn root_state(routine_id: &str, cw: &ClosedWorldTempParams) -> TempVec;

/// Cross one hop caller->callee. `binding_ok=false` (non-allowlisted edge kind)
/// yields all-Unknown-except-proven. Mirrors resolve_temp_along_path_closed_world
/// order: proven(callee,p) checked FIRST, then binding table:
///   no binding for p        -> Unknown
///   source Known(v)         -> Temp/Physical per v
///   source PD(j)            -> caller_state[j]
///   source None/Unknown     -> Unknown
pub(crate) fn cross_hop(
    caller_state: &TempVec,
    caller: &L3Routine,
    callsite_id: &str,
    callee_id: &str,
    binding_ok: bool,
    cw: &ClosedWorldTempParams,
) -> TempVec;

/// Terminal answer for an op given the state of its owning frame.
/// op_temp_state Known(v) -> Temp/Physical; PD(i) -> state[i]; None -> Unknown.
pub(crate) fn resolve_terminal(op: &L3RecordOperation, frame_state: &TempVec,
                               owner_id: &str, cw: &ClosedWorldTempParams) -> ParamTemp;
```

Only indices that some binding or op can query need representation; the vector
is built lazily per queried index and deduped sorted — two states compare equal
iff their sorted pairs are equal.

- [ ] **Step 1: Write the differential oracle test FIRST** — the load-bearing
  test of this whole task:

```rust
/// For EVERY simple path root->terminal in a fixture graph (enumerated by a
/// tiny in-test DFS), the forward composition (root_state -> cross_hop* ->
/// resolve_terminal) must equal TempVerdict::from_resolved of
/// resolve_temp_along_path_closed_world over the equivalent EvidenceStep path.
/// Fixtures cover: Known(true)/Known(false)/PD-chain-to-root/PD-chain-to-
/// Known / missing binding / non-allowlisted hop mid-chain / closed-world
/// proven at terminal frame / proven at mid frame / op with None temp_state.
#[test]
fn forward_vec_equals_backward_resolver_on_all_simple_paths() { ... }
```

The `...` body: build 4 small fixture routines with call_sites carrying
argument_bindings for each case above (mirror the binding shapes used in
`tests/temp_state_path.rs` — read it first; it already constructs these), then
assert equality per enumerated path. This is table-driven; one fixture per
bullet, ~120 lines.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p al-call-hierarchy --lib d1_temp:: 2>&1 | grep -E "error|test result"`
Expected: compile failure (functions absent).

- [ ] **Step 3: Implement per the Produces block** (transition table is a direct
  transcription of `step_one_frame` at `src/engine/l5/path_temp_resolve.rs:213-241`
  plus the guard order of `resolve_temp_along_path_closed_world:150-194`).

- [ ] **Step 4: Run tests to verify pass**

Run: `cargo test -p al-call-hierarchy --lib d1_temp:: 2>&1 | grep "test result"`
Expected: `ok.`

- [ ] **Step 5: rustfmt + clippy + commit**

```bash
rustfmt src/engine/l5/d1_temp.rs src/engine/l5/mod.rs
git add src/engine/l5/d1_temp.rs src/engine/l5/mod.rs
git commit -m "feat(l5): d1_temp — forward param-temp vector, differential-proven vs backward resolver"
```

---

### Task 3: `d1_reach` — product-state search, aggregation, witness materialization

**Files:**
- Create: `src/engine/l5/d1_reach.rs`
- Modify: `src/engine/l5/mod.rs`

**Interfaces:**
- Consumes: Task 1's `D1Graph`/`D1Seed`, Task 2's `TempVec`/`cross_hop`/
  `root_state`/`resolve_terminal`, `ctx.uncertainties_by_node`, d1 helpers
  `severity_for`, `is_setup_singleton_get`, `FLOWFIELD_GATED_OPS`,
  `flowfield_gate_blocks_downgrade`, `TempVerdict` (make `pub(crate)` in d1.rs),
  step builders: reuse `D1Policy::build_hop_step`/`build_terminal_step` logic by
  extracting them to free `pub(crate)` fns `hop_step(...)`/`terminal_step(...)`
  in `d1.rs` (same bodies, explicit params instead of `&self`).
- Produces (used by Task 5):

```rust
/// One (loop, terminal-op) aggregate: everything Task 5 needs to build a context.
pub(crate) struct LoopTerminalAgg<'a> {
    pub loop_routine: &'a L3Routine,
    pub loop_id: &'a str,
    pub loop_info: &'a PLoop,
    pub terminal: D1Terminal<'a>,
    pub entry_callsite_id: Option<&'a str>,   // None for direct in-loop ops
    /// Winning class per the selection rule (severity already realized):
    pub severity: &'static str,
    pub verdict: TempVerdict,
    pub reachable_verdicts: Vec<TempVerdict>,  // sorted, deduped
    pub depth_bucket: i64,                     // min(2, total effective depth) of winner
    pub effective_loop_depth: i64,             // the WINNER's actual depth (reported)
    pub witness: Vec<EvidenceStep>,            // loop step + hops + terminal step
    pub uncertainties: Vec<Uncertainty>,       // union along the WITNESS path
}

pub(crate) fn search_loops<'a>(
    graph: &D1Graph<'a>,
    seeds: &[D1Seed<'a>],
    direct_ops: &[DirectOp<'a>],   // branch (a) items, defined below
    ctx: &'a DetectorContext,
    cw: &ClosedWorldTempParams,
) -> Vec<LoopTerminalAgg<'a>>;

/// A direct in-loop db op (old branch (a)) folded into the same aggregation.
pub(crate) struct DirectOp<'a> {
    pub routine: &'a L3Routine,
    pub loop_id: &'a str,
    pub loop_info: &'a PLoop,
    pub op: &'a L3RecordOperation,
}
```

Locked algorithm:
1. Group seeds by `(loop_routine.id, loop_id)`. Per loop group, run ONE
   multi-source label search: initial label per seed =
   `(entry_node, cross_hop(root_state(loop_routine), seed callsite,...),
   depth = min(2, seed_depth), unc = !uncertainties_by_node[entry].is_empty())`,
   inserted in seed order.
2. Worklist = FIFO queue (BFS by hop count). Labels per node stored in a
   `HashMap<NodeIx, Vec<Label>>`; a label is
   `(temp_vec: TempVec, depth_bucket: i64, unc: bool)` + backpointer
   `(pred_label_idx, edge)` + `hops: u32` + seed index. Insert only if that
   exact label triple is NEW for the node (first discovery wins — BFS order
   makes first == shortest-then-lexicographic-by-edge-order, the deterministic
   witness tie-break).
3. Expansion: for each edge, child depth_bucket = `min(2, depth + edge.loop_depth)`,
   child temp = `cross_hop(...)` (using `edge.binding_ok`), child unc =
   `unc || !uncertainties_by_node[child].is_empty()`.
4. Scoring per (loop, terminal-op): every label at the terminal's node yields a
   candidate: verdict = `resolve_terminal` (+ FlowField gate on Temp:
   `FLOWFIELD_GATED_OPS.contains(op) && flowfield_gate_blocks_downgrade` ->
   `FlowFieldGated`); total depth = `min(2, label.depth + terminal.local_depth)`
   for severity, plus true depth (`seed_depth + Σ edge.loop_depth +
   local_depth`, recomputed along the backpointer chain) reported on the
   winner; severity = `severity_for(op, verdict, total_depth_for_scoring,
   is_setup_singleton_get(...))` — NOTE `severity_for` only distinguishes
   `>=2`, so passing the bucket is exact.
5. Selection order (external-review rule, locked): max severity rank ->
   verdict quality (Physical=FlowFieldGated > Uncertain > Temporary) ->
   `unc == false` preferred -> fewest hops -> first-discovered (BFS order).
6. Direct ops: candidate with zero hops, verdict =
   `resolve_terminal(op, root_state(routine), ...)` + FlowField gate, depth =
   `op.loop_stack.len()`, witness = `[loop_step, op_step]` exactly as built at
   `d1.rs:1052-1073` today. They enter the same per-(loop, terminal-op)
   aggregation (a loop can reach the same op directly AND transitively — the
   selection rule adjudicates).
7. Witness materialization: walk backpointers to the seed, emit
   `[loop_step, call_step]` (as built at `d1.rs:1141-1161`) + one
   `hop_step(edge)` per traversed edge + `terminal_step(op)`. Uncertainty union
   along the witness = dedupe_uncertainties over `uncertainties_by_node` of
   each node on the path (same rule the walker applied).
8. Output sorted: by loop routine id, then loop id, then terminal owner id,
   then op id (deterministic; no traversal-order dependence).

- [ ] **Step 1: Write failing tests** (fixture graphs via test_support, same
  style as Task 1):

```rust
#[test] fn finds_terminal_missed_by_budget() {
    // Star fan-out of 600 dead-end nodes plus one path to a terminal placed
    // AFTER them in edge order: old walker's 500-node budget starved it; the
    // search must find it. (This is defect D-A made into a test.)
}
#[test] fn severity_prefers_realizable_depth2_over_shorter_depth1() {
    // Two routes to the same op: 1-hop with depth bucket 1 (medium), 3-hop with
    // bucket 2 (high). Winner must be high + the 3-hop witness.
}
#[test] fn physical_route_beats_temp_route_same_severity_inputs() {
    // PD-terminal op; seed A passes temp var, seed B (same loop, second
    // callsite) passes physical. Aggregate verdict must be Physical, witness
    // through B, reachable_verdicts == [Temporary, Physical] (sorted).
}
#[test] fn cycle_terminates_and_dedupes_labels() {
    // A -> B -> A cycle with terminal on B: search terminates, one aggregate.
}
#[test] fn direct_and_transitive_same_op_adjudicated() {
    // Loop L reaches op T directly (depth 1) and transitively at bucket 2:
    // transitively-realized severity wins.
}
#[test] fn deterministic_across_runs() {
    // Build twice; Vec<LoopTerminalAgg> field-for-field equal.
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p al-call-hierarchy --lib d1_reach:: 2>&1 | grep -E "error|test result"`
Expected: compile failure.

- [ ] **Step 3: Implement** per the locked algorithm (the numbered list IS the
  implementation spec; each number maps to a function: `init_labels`,
  `expand_label`, `score_terminal`, `select_winner`, `materialize_witness`).

- [ ] **Step 4: Run tests to verify pass**

Run: `cargo test -p al-call-hierarchy --lib d1_reach:: 2>&1 | grep "test result"`
Expected: `ok. 6 passed`

- [ ] **Step 5: rustfmt + clippy + commit**

```bash
rustfmt src/engine/l5/d1_reach.rs src/engine/l5/mod.rs src/engine/l5/detectors/d1.rs
git add src/engine/l5/d1_reach.rs src/engine/l5/mod.rs src/engine/l5/detectors/d1.rs
git commit -m "feat(l5): d1_reach — product-state reachability search + witness selection"
```

---

### Task 4: Shadow differential — old walker as lower-bound oracle

**Files:**
- Modify: `src/engine/l5/detectors/d1.rs` (test module only)

**Interfaces:**
- Consumes: the CURRENT (still-live) `detect_d1` output, plus Tasks 1-3 run
  side-by-side on the same fixtures.
- Produces: the three monotonicity oracles as tests, and a manual DO recipe.

- [ ] **Step 1: Write the shadow tests** in `d1.rs`'s test module:

```rust
/// Old premerge identity set ⊆ new aggregate identity set, on every fixture
/// the existing d1 unit tests construct (reuse their fixture builders):
/// old key = (loop_id, terminal_routine_id, op_id) parsed from pre-dedupe
/// FindingRec ids (refactor: expose the pre-dedupe Vec<FindingRec> from a
/// pub(crate) detect_d1_premerge(...) helper carved out of detect_d1 — the
/// existing body up to the dedupe loop, unchanged behavior).
#[test] fn shadow_old_premerge_keys_subset_of_new() { ... }
/// severity_rank(new agg for that key) >= severity_rank(old finding), same keys.
#[test] fn shadow_severity_non_decreasing() { ... }
/// Old rootCauseKeys ⊆ new (terminal_routine_id, op_id) set.
#[test] fn shadow_root_cause_keys_subset() { ... }
```

- [ ] **Step 2: Run; fix any divergence found** — a divergence here is a REAL
  bug in Tasks 1-3 (filters or transitions drifted from the old semantics).
  Expected end state: all three pass on all fixtures.

- [ ] **Step 3: Manual DO shadow run** (documented, run it now): temporarily
  add `#[test] #[ignore]` `shadow_do_workspace()` gated on `DO_WS` env var that
  loads the DO workspace like `tests/cli` does (copy its loader call), runs the
  three oracles, prints the partitioned diff (added keys bucketed by
  "new-coverage" vs "severity-upgrade"). Run:

```bash
DO_WS='U:/Git/DO.Support-SlowDOSetup/DocumentOutput/Cloud' \
  cargo test -p al-call-hierarchy --lib shadow_do_workspace -- --ignored --nocapture \
  > logs/d1-shadow-do.log 2>&1
grep -E "SUBSET|SEVERITY|added|upgraded" logs/d1-shadow-do.log
```

Expected: subset oracles hold; record the added/upgraded counts in the log —
they are the Task 6 triage pre-read.

- [ ] **Step 4: Commit**

```bash
git add src/engine/l5/detectors/d1.rs
git commit -m "test(l5): d1 shadow differential — old walker as lower-bound oracle"
```

---

### Task 5: Terminal-centric assembly + cutover

**Files:**
- Modify: `src/engine/l5/finding.rs` (add `LoopContext`, `Finding.contexts`,
  stable mirror in `StableFinding` + conversion)
- Modify: `src/engine/l5/detectors/d1.rs` (replace walk consumption with the
  new pipeline; delete dead code)

**Interfaces:**
- Consumes: Task 3's `Vec<LoopTerminalAgg>`, existing `build_finding` helpers
  (`severity_for`, `table_note`, `is_setup_singleton_get`, notes
  `NOTE_TEMPORARY`/`NOTE_UNCERTAIN`/`NOTE_TEMP_FLOWFIELD`, G-4 pure-transitive
  wording, `to_confidence`, `uncertainty_lites`, `pick_actionable_anchor`,
  `insert_temp_note`, d14 `provably_dead_routine_ids`), fingerprint machinery
  (unchanged inputs).
- Produces:

```rust
// finding.rs
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoopContext {
    pub loop_id: String,
    pub loop_routine_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry_callsite_id: Option<String>,
    pub verdict: String,                 // temporary|physical|uncertain|flowfield-on-temp
    pub reachable_verdicts: Vec<String>,
    pub depth_class: String,             // "single-loop" | "nested-loop"
    pub severity: String,
    pub confidence: FindingConfidence,
    pub witness: Vec<EvidenceStep>,
}
// Finding gains:
//   #[serde(skip_serializing_if = "Option::is_none")]
//   pub contexts: Option<Vec<LoopContext>>,
// StableFinding gains the StableEvidenceStep-mirrored equivalent.
```

Locked assembly semantics:
- Group aggregates by `(terminal_routine_id, op_id)`. One Finding per group.
- `id = root_cause_key = "d1/{terminal_routine_id}/{op_id}"` (terminal-based
  identity — the SCHEMA change; old `d1/{loop}/...` ids disappear, which the
  Task 4 oracles already treat as non-monotonic by design).
- Context order: severity rank desc, then verdict quality, then loop routine
  id, then loop id (deterministic). `contexts[0]` is the WINNER.
- Finding severity, confidence, `evidence_path`, temp/setup notes, G-4
  pure-transitive wording: all from the winner (severity and confidence from
  the SAME context — fixes the best-confidence-across-loops mismatch,
  `path_merge.rs:105-148`).
- `additional_paths = Some(non-winner witnesses in context order)` (kept
  populated so SARIF/HTML/projection consumers render unchanged shapes).
- Multi-context annotation: append the existing `annotate_root_cause` wording
  with `contexts.len()` (reuse the fn from path_merge).
- `affected_objects` = sorted union over contexts' loop-routine object ids +
  terminal object id; `affected_tables` = terminal op table (unchanged rule).
- G-7 dead-routine down-confidence: roots = every context witness's first step
  routine; unchanged criteria, applied per finding.
- Stats preserved with identical keys where semantics survive
  (`candidatesConsidered`, `skipped_*`, `downgradedToInfo` still counted in
  the direct-op enumeration, `downgradedSetupSingleton` counted post-assembly
  by root-cause text as today); `mergedPathGroups`-style stats that described
  the old merge die with it — remove their keys, note in CHANGELOG.
- DELETE at end of this task: `D1Policy` walk usage + `walk_memo` +
  `apply_seed_transform` + branch-(b) walk consumption + `reconcile_merge_tie`
  + d1's `merge_by_terminal` call (grep: if d1 was `merge_by_terminal`'s only
  caller, move the fn + its tests into d1-history or delete; `path_merge`'s
  `path_sort_key`/`annotate_root_cause` survive if still referenced). The d1
  Hot-tier trace counters are REPLACED by new ones: `d1.reach` census
  (nodes/edges/seeds/labels/aggregates) via the same `pt::` API — cap-durable
  flushes are no longer needed (runs finish in seconds).
- `path_walker.rs` untouched (d2/d46/d48).

- [ ] **Step 1: Write failing assembly tests** (d1.rs test module):

```rust
#[test] fn one_finding_per_terminal_with_contexts() { ... }
#[test] fn winner_drives_severity_confidence_and_wording() { ... }
#[test] fn terminal_based_id_and_stable_fingerprint_inputs() {
    // fingerprint inputs (detector, terminal primary location, affected tables,
    // root_cause_key) unchanged vs old pipeline on the same fixture.
}
#[test] fn dead_routine_downconfidence_spans_all_contexts() { ... }
```

Each `...` body: construct via the same fixtures Task 4 used, assert the
locked-semantics bullets above (they are the assertions, one per bullet).

- [ ] **Step 2: Run to verify failure**, implement finding.rs then the d1
  assembly, re-run to green:

Run: `cargo test -p al-call-hierarchy --lib d1 2>&1 | grep "test result"`
Expected: `ok.`

- [ ] **Step 3: Full test suite — expect golden failures ONLY**

Run: `cargo test 2>&1 | tee logs/d1-cutover-tests.log | grep -E "FAILED|test result"`
Expected: unit/integration green except golden byte-compares touching d1 output.

- [ ] **Step 4: Regen ALL goldens + TRIAGE**

```bash
scripts/check-goldens --regen > logs/d1-goldens-regen.log 2>&1
git diff --stat tests/ | tail -5
```

TRIAGE the diff (this is the semantic cutover — never blind-bless). Partition
every changed finding into: (a) id migration `d1/{loop}/... -> d1/{routine}/{op}`
with same terminal — expected, mass; (b) NEW findings — must trace to budget
removal (Task 4's DO log predicts the population); (c) severity changes — must
be UPGRADES justified by a realizable witness (spot-check the witness path in
the golden against fixture source); (d) contexts/additional_paths shape churn —
expected. ANY severity DOWNGRADE or vanished rootCauseKey = STOP, it violates
the Task 4 oracles — debug before proceeding.

Then: `scripts/check-goldens > logs/d1-goldens-verify.log 2>&1; tail -2 logs/d1-goldens-verify.log`
Expected: all green.

- [ ] **Step 5: DO byte-diff (semantic diff, not byte-identity)**

```bash
cargo build --profile release-fast 2>&1 | tail -2
./target/release-fast/alsem analyze 'U:/Git/DO.Support-SlowDOSetup/DocumentOutput/Cloud' \
  --format json > logs/d1-do-after.json 2> logs/d1-do-after.err
```

Compare against a pre-cutover JSON (generate from master with the same command
before starting Step 1 — save as `logs/d1-do-before.json`). Assert: rootCauseKey
subset oracle holds, severity non-decreasing, runtime within the DO band
(9-11 s). Record added/upgraded counts; they must match Task 4's shadow log.

- [ ] **Step 6: CHANGELOG + docs + commit**

CHANGELOG.md under Changed (d1 semantics: terminal-centric findings, contexts,
budget removal, verdict adjudication — cite defect fixes D-A/D-B), Removed (old
walk consumption, reconcile_merge_tie, stats keys), Added (LoopContext schema).
Update `docs/superpowers/specs/2026-07-19-perf-optimization-handoff.md` §4
(decision: resolved — no caps, reachability redesign) and `docs/OUTSTANDING.md`.

```bash
rustfmt src/engine/l5/finding.rs src/engine/l5/detectors/d1.rs
cargo clippy --all-targets --all-features 2>&1 | grep -E "^error|^warning" ; true
git add src/engine/l5/finding.rs src/engine/l5/detectors/d1.rs CHANGELOG.md \
  docs/superpowers/specs/2026-07-19-perf-optimization-handoff.md docs/OUTSTANDING.md \
  tests/r4-goldens tests/ir-l2-goldens tests/cli-a-goldens tests/r2c-goldens
git commit -m "feat(l5)!: d1 terminal-centric reachability findings — budget-free coverage, order-independent verdicts"
```

---

### Task 6: Perf measurement + handoff doc closure

**Files:**
- Modify: `docs/superpowers/specs/2026-07-19-perf-optimization-handoff.md`

- [ ] **Step 1: Rebuild the 8020 corpus** per the Wave-1 recipe (Global
  Constraints pointer), quiet machine, kill stale alsem.exe.

- [ ] **Step 2: d1-only 8020 run with tracing**

```bash
ALSEM_TRACE=1 ALSEM_TRACE_DETAIL=stages ALSEM_TRACE_STDERR=1 \
  ./target/release-fast/alsem analyze <corpus-8020> --detectors d1-db-op-in-loop \
  > logs/d1-8020-after.log 2> logs/d1-8020-after.err
grep -E "STAGE|detector:d1" logs/d1-8020-after.err | tail -20
```

Expected: d1 detector stage completes in seconds-to-low-minutes (was: never —
3 h extrapolated). If d1 exceeds 5 min, profile before optimizing further
(measure-before-build); the design headroom (rayon over loops) is licensed
ONLY by that measurement.

- [ ] **Step 3: Full-default 8020 run** — the arc's finish bar. Detached run
  (>10 min possible: substrate is still ~24 min):

Expected: run FINISHES; substrate remains the wall (~24 min / ~43-52 GB);
record total wall + peak RSS + per-detector tail in the handoff doc.

- [ ] **Step 4: Regression bands** — DO default and 8020 3-det re-run; must
  stay within Global Constraints bands.

- [ ] **Step 5: Update handoff doc** (§0 TL;DR, §2 facts, §4 marked RESOLVED,
  §5 queue: B1-narrow now top) + commit:

```bash
git add docs/superpowers/specs/2026-07-19-perf-optimization-handoff.md logs/.gitignore
git commit -m "docs(specs): d1 reachability redesign measured — 8020 full-default finish bar"
```

---

### Post-plan user gates (not tasks)

- `scripts/cdo-gate <CDO_WS>` — north-star SHA `0a3b85bc…` must reproduce
  (fresh resolver untouched by this plan; this is the proof).
- CDO finding-level triage of the d1 diff (same partition as Task 5 Step 4) —
  the `triage-findings` skill; new-coverage findings are candidate REAL wins,
  treat >30% FP among them as a defect in the coverage change.
- B1-narrow is the next queue item after this plan lands.

## Self-review notes

- Spec coverage: no-caps decision (no budget, no cap anywhere: Task 3 has no
  node/depth bound — cycle safety comes from label dedup, not budgets);
  terminal-centric schema (Task 5); order-independent verdicts (Task 3 rule 5);
  multi-source per loop incl. seed-hop temp participation (Task 3 rule 1, Task
  2 cross_hop at seed); monotonicity oracles (Task 4); one gated cutover
  (Task 5); measurement closure (Task 6). Parallelism deliberately ABSENT
  (YAGNI — licensed only by Task 6 Step 2 evidence).
- Depth semantics: `severity_for` needs only the >=2 threshold — bucket is
  exact for scoring; the winner's TRUE depth is recomputed for reporting
  (Task 3 rule 4) so output never shows a clamped number.
- Known deliberate divergences from old output (user-approved semantics
  change, all surfaced in Task 5 triage): terminal-based ids; witness ≠ old
  DFS-first path; severity upgrades from D-A/D-B; contexts field; removed
  merge-era stats keys.

---

## Task 7 — search_loops scaling (added 2026-07-20, licensed by Task 6)

**Why:** Task 6 measured d1-only 8020 at >4h STILL inside `search_loops`
(census never emitted), while DO ran in 5.23s. Root cause (verified
`d1_reach.rs:617` + `:362`): `search_loops` runs ONE product-state BFS per loop
group with a FRESH per-group `seen` map — zero cross-group sharing. On Base
Application's dense 797-member SCC, thousands of loop groups each re-traverse the
same overlapping closure. Cost = (num groups) × (dense-SCC traversal), serial.
Two levers, sequenced by measure-before-build:

### Task 7a: parallelize the group loop (output-IDENTICAL, low risk)

**Files:** Modify `src/engine/l5/d1_reach.rs` (`search_loops`).

Groups are independent; `process_group`'s only mutable output is `&mut out`
(append), and `search_loops` ends with a TOTAL-order `out.sort_by` (loop routine
id, loop id, terminal owner id, op id — `d1_reach.rs:634`), so call order does
not affect output. Refactor `process_group` to RETURN `Vec<LoopTerminalAgg>`;
collect the `groups` BTreeMap into a `Vec<(key, Group)>` (deterministic from
sorted BTreeMap iteration); `par_iter().flat_map(process_group).collect()` via
rayon (already a dep, `Cargo.toml:22`); keep the existing final sort verbatim.

- [ ] Step 1: Verify `DetectorContext`, `D1Graph`, `ClosedWorldTempParams`, and
  everything the group closure borrows are `Sync` (grep for `RefCell`/`Cell`/
  `Rc` reachable through the borrowed fields — the D1Policy walk memo used
  RefCell but that path is now cfg(test); ctx must be clean). If any accessed
  field is not Sync, report NEEDS_CONTEXT before proceeding.
- [ ] Step 2: Add a determinism test: `search_loops` over a multi-group fixture
  returns byte-identical `Vec<LoopTerminalAgg>` across two calls (already
  implied, but assert it), AND assert the parallel result equals a serial
  reference computed inline in the test.
- [ ] Step 3: Implement the rayon refactor.
- [ ] Step 4: Full suite + `scripts/check-goldens` — MUST stay byte-stable
  (this change is output-identical; a golden move = a bug). Report exit codes.
- [ ] Step 5: rustfmt + clippy + commit.

### Task 7b: cross-group start-label memo (ONLY if 7a insufficient)

Gated on the Task 7a re-measure. A search from a start-label
`(entry_node, temp_vec, depth_bucket, unc)` is caller-independent (same property
that licensed Wave-2c's canonical-callee memo): its reachable-terminal set +
per-terminal {verdict, suffix witness, suffix hops, suffix depth-delta, suffix
uncertainty set} depend only on the label state, not the seed/prefix. Memoize on
that key; each group prepends its own `[loop_step, call_step]` prefix + true
depth. Collapses `× num_groups` → `× distinct start-labels`. CAUTION: with a
shared multi-source `seen`, the current code lets the first source's prefix win a
shared label's whole downstream; a per-label memo that unions candidates across
sources then applies the selection rule is DETERMINISTIC and >= as correct, but
MAY change witnesses vs 7a's output → goldens regen + shadow-oracle re-triage
(subset holds: superset of candidates; severity monotonic: selection takes max).
Only build this if 7a's 8020 number is unacceptable; write it as its own plan
with the witness-under-sharing design worked out first.

### Task 7 measurement gate

After 7a: re-run d1-only 8020 (Hot-tier) + full-default 8020, quiet machine.
Record whether the finish bar is met and d1's own `d1.reach` census + wall. If
met → finalize. If not → Task 7b.
