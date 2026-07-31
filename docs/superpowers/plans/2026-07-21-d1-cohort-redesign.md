# d1 Cohort Redesign Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Make d1 finish on Base App 8020 (currently ~8 h, killed) by eliminating
the per-`(loop, terminal)` witness/context explosion — replace the 3.2M
`emit_lane_aggregates` witness builds with terminal **bitmap-cohort** emission +
**one bounded representative witness per verdict-class** + a **compressed
terminal-centric report**. Output components `(loop, terminal, verdict,
depth_bucket, unc)` stay exact; witness becomes representative; output shape
becomes compressed (USER-APPROVED 2026-07-21).

**Architecture:** Specialized IFDS tabulation, Datalog semi-naive style (gpt r5/r6
co-design). KEEP the existing 64-lane `solve_batch` fixpoint + `TerminalPlan`
running-best scan UNCHANGED — they are fast (ms/batch) and correct. Change ONLY
the emission + output: (1) a run-global terminal sink `Terminal → ContextKey →
loop-bitmap`; (2) an `origin_seed` propagated per fact/lane so witnesses need no
28k-hop predecessor walk to find the seed; (3) one bounded representative witness
per `(terminal, context-class)`; (4) a compressed report with hash-consed
loop-sets + a loop catalog. Defer the full global-bitmap solver (build only if
this doesn't hit target).

**Tech Stack:** Rust, existing engine. No new deps.

## Design provenance

gpt-5.6-sol rounds 5-6 (memory note `d1-output-bound-falsified` §2026-07-21) +
IFDS/IDE (Reps-Horwitz-Sagiv) + reachability-indexing research. Root cause
(measured): fixpoint fast; `emit_lane_aggregates` (d1_dataflow.rs) builds a full
witness per `(loop, terminal)` (3.2M), each walking ~28k-hop predecessor chains.

## Global Constraints

- Package `al-call-hierarchy` (HYPHEN) for `cargo test -p`.
- rustfmt per file, NEVER `cargo fmt`. `cargo clippy --all-targets --all-features` clean.
- Goldens: `scripts/check-goldens --regen` regenerates ALL five families; regen is
  a MEASUREMENT — triage, never blind-bless. This redesign CHANGES output shape →
  goldens WILL move → regen + triage confirming the DECOMPRESSED tuples match.
- Fresh resolver (`src/program/`) untouched; north-star SHA `0a3b85bc…` reproduces.
- Never pipe a gate through `| tail` (exit code lies) — redirect to a log + grep.
- Measurement: `cargo build --profile release-fast`; kill stale `alsem.exe`; DETACHED
  run via `logs/run-det-d1only.ps1` (Start-Process survives harness reaping) +
  sentinel; **`ALSEM_TRACE_DETAIL=hot` ALONE** (NOT `stages,hot` — parse falls to
  Stages, gating off Hot counters; perf_trace.rs:58-61); span/counter names are
  BARE not cat-prefixed; `serde_json` sorts object keys so grep individual keys.
  8020 corpus at `…/scratchpad/corpus-8020`; DO at
  `U:/Git/DO.Support-SlowDOSetup/DocumentOutput/Cloud`; substrate ~19 min.
- CHANGELOG.md updated. Stage explicitly; never `git add -A`. Branch:
  `feat/d1-reachability` (continue). Commit trailer:
  `Claude-Session: https://claude.ai/code/session_016wtDZDfc9Gcyjz1GjZpPhQ`

## The correctness spine (read before any task)

The current `solve_batch` already computes, per terminal, `best[lane]` = the
winner `(verdict, severity, depth_bucket, unc, hops, source)` for each loop-lane
(d1_dataflow.rs, the running-best scan). This redesign changes ONLY how that
winner is EMITTED and how witnesses are built — NOT which winner is selected. So
the set of `(loop, terminal, verdict, depth_bucket, unc)` tuples and
`reachable_verdicts` per `(loop, terminal)` MUST be byte-identical to the current
output. The DIFFERENTIAL: `decompress(new cohorts)` == the tuples the current
`emit_lane_aggregates` would emit, on components verdict / depth_bucket / unc /
coverage / reachable_verdicts. Witness is NOT compared (representative, bounded).
The current `batch_equals_per_group` / `search_loops_matches_process_group`
differentials + `solve_group`/`process_group` remain the small-graph oracles.

## File Structure

- Create `src/engine/l5/d1_cohort.rs` — the terminal sink (`Terminal → ContextKey →
  GroupBitmap`), `GroupBitmap` (hybrid), cohort finalization (bitmap subtraction →
  winner cohorts), loop-set interning.
- Create `src/engine/l5/d1_witness.rs` — representative bounded witness per
  `(terminal, context-class)` from `origin_seed` + bounded predecessor tail.
- Modify `src/engine/l5/d1_dataflow.rs` — replace `emit_lane_aggregates` with sink
  insertion; add `origin_seed` to `BatchSolver` + `commit_reach`/`commit_value`;
  `solve_batch` returns/contributes to the sink instead of `Vec<LoopTerminalAgg>`;
  add the `ΣNeed`/static-fact/reached-fact census.
- Modify `src/engine/l5/finding.rs` — the compressed report types (`LoopContextCohort`,
  interned `LoopSetId`, `LoopCatalogEntry`, representative-witness ref) + stable mirror.
- Modify `src/engine/l5/detectors/d1.rs` — `detect_d1` consumes the sink → compressed
  report; `assemble_findings` builds cohort findings.
- Modify consumers: `src/lsp/lens.rs`, `src/lsp/diagnostics.rs`,
  `src/engine/gate/format_sarif.rs`, `src/engine/gate/format_html.rs`,
  `src/engine/gate/projection.rs` — render cohort findings (representative witness +
  loop count + a few loop anchors; decompress on demand).

---

### Task 1: terminal sink + cohort emission (replace emit_lane_aggregates) + census

**Files:** Create `src/engine/l5/d1_cohort.rs`; modify `src/engine/l5/d1_dataflow.rs`,
`src/engine/l5/mod.rs`.

**Interfaces — Produces:**

```rust
pub(crate) type GroupIx = u32;

/// Hybrid loop-set bitmap over GroupIx (up to ~6178). Start dense-lazy.
pub(crate) enum GroupBitmap { Empty, Dense(Box<[u64]>) } // n_words = ceil(groups/64)
impl GroupBitmap { fn set(&mut self, g: GroupIx); fn or_with(&mut self, other:&GroupBitmap);
    fn and_not(&self, other:&GroupBitmap)->GroupBitmap; fn is_empty(&self)->bool;
    fn iter(&self)->impl Iterator<Item=GroupIx>; fn count(&self)->u64; }

/// The per-(terminal, semantic-class) context identity (excludes loop).
#[derive(PartialEq, Eq, Hash, Clone)]
pub(crate) struct ContextKey {
    pub severity: &'static str, pub verdict: TempVerdict,
    pub depth_bucket: i64, pub unc: bool,
    // representative witness source (one BestRef per class, first-seen)
}

/// Run-global sink: terminal -> context-class -> which loops realize it, +
/// a per-(terminal, loop) verdict mask (for reachable_verdicts).
pub(crate) struct TerminalSink { /* dense by TerminalIx */ }
impl TerminalSink {
    fn new(n_terminals: usize, n_groups: usize) -> Self;
    /// Called per winning lane (batch_base+lane = GroupIx). Sets the loop bit in
    /// the (terminal, ctx) cohort; ORs verdict into the per-(terminal) verdict map
    /// for that loop; records the first-seen BestRef for the class (representative).
    fn insert(&mut self, terminal: TerminalIx, group: GroupIx, ctx: ContextKey, rep: BestRefLite);
    /// Finalize: per terminal, the cohorts are ALREADY per-winner (best[lane] gave
    /// one ctx per (terminal, loop)), so no cross-cohort subtraction is needed —
    /// each loop appears in exactly ONE ctx per terminal (its winner). Verify with
    /// an assertion. Produce Vec<(TerminalIx, Vec<(ContextKey, GroupBitmap)>)>.
    fn finalize(self) -> Vec<TerminalCohorts>;
}
```

NOTE (correctness simplification): because `best[lane]` already picks ONE winner
per `(terminal, loop)`, each loop lands in exactly one `ContextKey` per terminal —
so the sink is just `set the loop bit in the winner's ctx cohort`; the general
"bitmap subtraction across candidate cohorts" (gpt r5) is NOT needed here (that was
for the un-pre-selected candidate case). Assert disjointness (a loop appears in ≤1
ctx per terminal) as the invariant.

- Modify `solve_batch`: replace the `emit_lane_aggregates` call with, per terminal,
  per present lane: `sink.insert(terminal_ix, batch_base + lane, ctx_from(best[lane]),
  best_ref_lite)`. `batch_base` = `bi * BATCH_WIDTH` (thread the batch index in).
  Do NOT build witnesses. Keep the running-best scan exactly.
- Census (Hot): emit `ΣNeed`, `max_need_per_node`, `nodes_with_need`,
  `static_reach_facts` (6·nodes), `static_value_facts` (18·ΣNeed), and after the run
  `union_reached_facts`, `total_cohorts`, `unique_loopsets`.

- [ ] Step 1: Write the DIFFERENTIAL test FIRST — the spine. On every fixture the
  `d1_dataflow`/`d1_reach` tests build, run the sink path AND the current
  `emit_lane_aggregates` path; assert `decompress(sink)` (every (loop, terminal)
  with its ctx verdict/depth_bucket/unc + reachable_verdicts) equals the current
  aggregates' tuples EXACTLY. Non-vacuous.
- [ ] Steps 2-5: RED → implement `d1_cohort` + wire `solve_batch` → GREEN → full
  suite (goldens will move at cutover, NOT here — solve_batch still returns
  aggregates via a compatibility shim until Task 6, OR gate the new path behind a
  flag; keep goldens green in Task 1 by having detect_d1 still use the old path) →
  rustfmt/clippy → commit.

---

### Task 2: origin_seed propagation

**Files:** Modify `src/engine/l5/d1_dataflow.rs`.

Add `reach_origin: Vec<[u32; BATCH_WIDTH]>` + `value_origin: Vec<[u32; BATCH_WIDTH]>`
to `BatchSolver` (parallel to `reach_hops`/`value_hops`). In `commit_reach`/
`commit_value`, for each newly-admitted lane bit (the loop already iterating for
hops/pred): set origin = the seed index for a `Seed` predecessor, else COPY the
parent fact's origin[lane]. So a fact's origin[lane] = the seed that first reached
it on lane — WITHOUT walking the predecessor chain.

- [ ] Step 1: Test: for a multi-hop fixture, `reach_origin[terminal_fact][lane]`
  equals the seed index that `collect_reach_chain_b` would terminate at. (Compare
  against the existing chain walk on fixtures.)
- [ ] Steps 2-5: RED → implement → GREEN → full suite green (additive) → commit.

---

### Task 3: representative bounded witness

**Files:** Create `src/engine/l5/d1_witness.rs`; modify `d1_dataflow.rs`.

Per `(terminal, ContextKey)` (from the sink's representative `BestRef` + a
representative lane), build ONE bounded witness: `[loop_step, call_step]` (from
`origin_seed`) + up to first-K hop steps + `…N omitted…` marker + last-M hop steps
(walk `reach_pred`/`value_pred` only M steps from the terminal) + terminal step;
plus `total_hops` (from `reach_hops`/`value_hops`) and exact `depth_bucket`/`unc`.
Drop full true-depth recompute + full uncertainty-vector.

```rust
pub struct WitnessSummary {
    pub total_hops: u32, pub first_steps: Vec<EvidenceStep>,
    pub omitted_hops: u32, pub last_steps: Vec<EvidenceStep>,
    pub terminal_step: EvidenceStep,
}
pub(crate) fn representative_witness(...) -> WitnessSummary;
```

- [ ] Step 1: Test: on a deep fixture (hops > K+M), the witness has first-K +
  last-M + correct total_hops + omitted = total-(K+M+prefix); on a shallow fixture
  (hops ≤ K+M), the full path with omitted=0. Witness is a valid realizing path
  (first step in loop routine, last = terminal op).
- [ ] Steps 2-5: RED → implement → GREEN → full suite → commit.

---

### Task 4: compressed report schema + loop-set interning

**Files:** Modify `src/engine/l5/finding.rs`; modify `d1_cohort.rs` (interning).

Compressed report types (internal + `Stable*` serialized mirror):

```rust
pub struct LoopCatalogEntry { pub loop_ix: u32, pub loop_routine_id: String,
    pub loop_id: String, pub anchor: SourceAnchor, pub entry_callsite_id: Option<String> }
pub struct LoopSetId(pub u32); // interned bitmap
pub struct D1CohortContext { pub severity: String, pub verdict: String,
    pub depth_bucket: i64, pub uncertain: bool, pub loop_set: LoopSetId,
    pub loop_count: u64, pub witness: WitnessSummary }
// Finding gains: contexts_compressed: Option<Vec<D1CohortContext>> (+ the run-level
// loop catalog + loop-set registry live on DetectorOutput, not per-finding).
```

Hash-cons loop-set bitmaps → `LoopSetId` (many terminals share reaching sets).
Loop catalog + loop-set registry attach to `DetectorOutput`.

- [ ] Step 1: Tests: interning round-trips (same bitmap → same id; decompress(id) ==
  bitmap); a finding's compressed contexts decompress to the exact (loop, verdict,
  depth, unc) tuples; stable-serialize round-trip.
- [ ] Steps 2-5: RED → implement → GREEN → full suite → commit.

---

### Task 5: consumer adapters

**Files:** Modify `src/engine/l5/detectors/d1.rs` (`assemble_findings` → cohort
report), `src/lsp/lens.rs`, `src/lsp/diagnostics.rs`,
`src/engine/gate/format_sarif.rs`, `src/engine/gate/format_html.rs`,
`src/engine/gate/projection.rs`.

Each consumer renders a cohort finding: the terminal + severity + reachable
verdicts + per-class `loop_count` + a few representative loop anchors (from the
catalog) + ONE representative witness per class. SARIF: one representative
code-flow per class + `total_hops`/`loop_count` metadata. Decompress the loop-set
only when a consumer needs the full loop list.

- [ ] Step 1 (per consumer): a test that the adapter renders a cohort finding
  without panics and surfaces loop_count + representative witness + verdicts.
- [ ] Steps 2-5: implement each adapter; full suite (goldens still via old path
  until Task 6) → commit (may be several commits, one per consumer).

---

### Task 6: cutover + goldens regen/triage + DO diff

**Files:** Modify `d1.rs` (wire detect_d1 to the sink → compressed report; delete
`emit_lane_aggregates` + the per-loop `LoopTerminalAgg` witness path; keep
`solve_group`/`process_group` cfg(test) oracle), CHANGELOG, goldens.

- [ ] Step 1: DO baseline JSON with the PRE-cutover binary.
- [ ] Step 2: Wire detect_d1 to the new path. Full suite. `scripts/check-goldens --regen`.
- [ ] Step 3: TRIAGE — the goldens MOVE (compressed shape). Verify: DECOMPRESSED
  cohorts == the old per-(loop, terminal) (verdict, depth_bucket, unc) tuples +
  reachable_verdicts (a decompress-and-compare test/script). Severity/verdict/
  coverage per (loop, terminal) MUST match; only the SHAPE (cohorts vs contexts) +
  witness (representative vs full) changed. A per-(loop,terminal) tuple change =
  STOP.
- [ ] Step 4: DO semantic diff vs baseline: decompressed tuples match; runtime in band.
- [ ] Step 5: CHANGELOG + commit.

---

### Task 7: 8020 measurement — the finish bar

**Files:** Modify `docs/superpowers/specs/2026-07-19-perf-optimization-handoff.md`.

- [ ] Quiet machine; kill stale alsem; `cargo build --profile release-fast`.
- [ ] d1-only 8020 detached (`run-det-d1only.ps1`, `ALSEM_TRACE_DETAIL=hot`): record
  d1 wall, the census (ΣNeed / static+reached facts / cohorts / unique loopsets),
  peak RSS. Compare to the ~8 h / killed baselines.
- [ ] Full-default 8020: FINISHED y/n, total wall, peak RSS, per-detector tail.
- [ ] Regression bands: DO default (5.23 s), 8020 3-det.
- [ ] Update handoff doc; commit. If STILL above target, the census tells whether to
  build the deferred global-arrival-cohort solver (Task R-full).

---

### Post-plan user gates
- `scripts/cdo-gate <CDO_WS>` — north-star SHA reproduces (fresh resolver untouched).
- CDO d1 finding triage (decompressed tuples vs prior).

## Self-review notes
- Spine (decompressed tuples == old, per-(loop,terminal)) gated at Task 1
  (differential), Task 6 (goldens + DO). Witness is representative (not compared).
- The "bitmap subtraction" from gpt r5 is unnecessary HERE (best[lane] pre-selects
  one winner per (loop, terminal)); the sink just sets the winner's bit. Disjointness
  asserted.
- Fixpoint + running-best scan UNCHANGED (fast, correct). Only emission + output +
  witness change. Full global solver DEFERRED (Task 7 census decides).
- Output shape change USER-APPROVED (compressed cohorts + loop catalog + representative
  witness).
