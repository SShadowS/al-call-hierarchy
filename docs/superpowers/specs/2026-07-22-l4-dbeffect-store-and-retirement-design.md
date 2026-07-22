# L4 db-effect Store Redesign + Old-Solver Retirement — Design Spec

**Date:** 2026-07-22
**Status:** design — pre-implementation
**Follows:** `2026-07-21-l4-summary-fixpoint-redesign-design.md` (Phase 1, MERGED-on-branch: the
closed-form v2 db-effect solver, byte-identical to the old Jacobi, is done). This is the Phase-1.5
sub-arc that (A) retires the old solver so there is ONE path, and (B) redesigns the db-effect
**representation** so the (correct) v2 solver is fast (seconds) and compact (sub-GB) and queryable.

**Goal:** Keep every db-effect fact fully computed and queryable, but store it as a shared,
interned, columnar structure so `compute_summaries` drops from **517s / ~40GB** to **seconds /
<300MB**, and expose a bidirectional (down: routine→effects; up: table/effect→routines) query index
for a future VSCode hover. Simultaneously delete the old Jacobi solver and its dead incrementality
experiment so no dual path is maintained.

**Reviewed inputs:** two independent, source-read model consultations (gpt-5.6-sol, claude-fable-5),
which converged on the design below (`scratchpad/pi-sol-repr.md`, `scratchpad/pi-fable-repr.md`).

---

## Motivation (measured, 8020-file real BC Base Application)

Phase-split attribution (instrumented, `compute_summaries_v2_phase_split`):
- **db-effect solver: 517s** ← the cost. roles fixpoint: **0.8s** (a non-issue).
- peak RSS: **~40GB** (unchanged by the Phase-1 solver redesign).
- 82,716 Tarjan SCCs; dominant one = **797 mutually-recursive routines**; effect universe = **9,137**;
  mean Jaccard between the 797 members' effect sets = **0.99997**; total memberships = **7,122,286**.

The cost is NOT algorithmic (no Jacobi, no O(N²) — those are fixed). It is **per-effect String
constant-factors on materialization**: `materialize_member_db_effects` sorts each routine's effects
with a `format!`-per-comparison key; `reconstruct_via` builds a String-keyed
`HashMap<(String,EffectId),String>` with ~7.1M entries; each of 797 SCC-members holds its own private
~9,000-element `Vec<DbEffect>` (5 heap Strings each) even though `closed_form_union` builds them as
`c.clone()` + a tiny per-member PD delta. **The sharing exists in the solver and is destroyed at the
last line.**

**Why not just skip db_effects on the analyze path** (it is provably unread there today): the product
needs it — future detectors will read per-routine db_effects, and a planned VSCode hover answers "is
table/record X touched by any DB action transitively, up or down the call stack." So db_effects must
stay computed AND become cheaply queryable in both directions.

---

## Prior art (this is a solved shape — we are not first)

- **SCC-condensed bitvector dataflow** (textbook): collapse SCCs, process the condensation DAG in
  reverse-topo, union successor bitvectors, share ONE closure among SCC-equivalent nodes.
- **Andersen inclusion-based points-to with SCC/cycle collapse** — Fähndrich et al. PLDI'98 (*Partial
  Online Cycle Elimination*), Hardekopf & Lin PLDI'07 (*The Ant and the Grasshopper*, HVN/HRU). Core
  insight = ours: nodes in a cycle have identical solution sets → store one set per equivalence class
  + a class-representative pointer. Steensgaard unification does NOT apply (it trades precision for
  speed; we need exact parity).
- **Shared-set interning** — Heintze CLA (PLDI'01), hash-consed set constructor cache.
- **Compressed bitmap indexes** (Roaring) — for the reverse posting lists over the 82k/100k
  SCC/routine domain, NOT for the dense 9,137-bit forward sets (a plain `[u64; 143]` is 1.1KB).
- **Explicitly NOT adopted as the primary structure:** BDD/bddbddb (overkill for one-level,
  class-shaped redundancy; variable-ordering pain), full Datalog/Soufflé (useful as an oracle, not a
  smaller store), GRAIL/PathTree/2-hop reachability labels (answer node-pair reachability — the wrong
  predicate; our reverse query is a relation transpose, not pairwise reachability).

---

## Part A — Old-solver retirement (FIRST, per user directive "no dual paths")

The old Jacobi solver survives only as:
1. the **v2-vs-old differential oracle** (`tests/l4_summary_differential.rs`),
2. **`run_and_project`** (`summary.rs:795`) → aldump R3a-2 trace projection,
3. the **R3b Salsa incrementality experiment** (`src/engine/l4/incremental/`, `run_one_scc` via
   `scc_summaries`) — consumed ONLY by `tests/r3/r3b_*` (verified: NO bin/lsp/server references it).

Retirement (before the repr redesign, so the redesign is done on a single path):

1. **Freeze a v2 self-baseline oracle.** Before deleting old, serialize v2's current output on the 10
   differential fixtures (+ the CDO-gated whole-program case when `CDO_WS` is set) to a committed
   golden. The differential test flips from `v2 == old` to `v2 == frozen-baseline` — preserving a
   regression anchor for the repr redesign WITHOUT the old solver. (Phase 1 already proved v2 == old;
   the frozen baseline captures that proven-correct output as the new anchor.)
2. **Cut `run_and_project` off the old solver.** Assess aldump's R3a-2 trace consumers: v2 is
   trace-free (no `RawSccTrace`), so either (a) re-point the R3a-2 projection at v2's final summaries
   (trace goldens that encode the 58-pass trajectory retire → regenerate to final-semantics form,
   inspected), or (b) if aldump's `--l3`/r3a2 trace output is itself obsolete, retire that dump mode.
   Decide at implementation time from the actual golden diff; NEVER blind-regen.
3. **Delete the R3b Salsa incrementality experiment** (`src/engine/l4/incremental/` + `tests/r3/r3b_*`).
   It is built on the old `run_one_scc`, unused by shipping code, and architecturally superseded (the
   workspace-shared store in Part B does not decompose into Salsa's per-SCC memoized values; a future
   incremental LSP path, if wanted, would be redesigned against the new store). **This is a whole
   experimental subsystem deletion — called out explicitly for approval.**
4. **Delete the old Jacobi solver:** `compose_routine`'s db_effects fold + uncertainty fold,
   `run_one_scc`, `compute_summaries` / `compute_summaries_with_leaves`, the `RawSccTrace` machinery,
   the `Detail::Jacobi` db_effects instrumentation. **KEEP** `compose_roles_only` / `run_one_scc_roles`
   (the roles path v2 uses) and `solve_side_facts`.
5. Gate: goldens byte-identical (except deliberately-retired trace goldens, inspected), lib + the
   reframed differential green, DO byte-identical.

**Risk the user accepted:** retiring the old oracle before the biggest change removes the v2-vs-old
net. Mitigated by the frozen v2 baseline (step 1), which is a stronger anchor for a
representation-only change (the logical output must not move at all).

---

## Part B — db-effect Store redesign (Steps 1-4)

A workspace-level `EffectStore` replaces per-routine `Vec<DbEffect>`. `Vec<DbEffect>` becomes a
projection/view type only (built lazily at the serde/detector boundary).

### Interned identities (Step 1)
- `RoutineIx(u32)`, `EffectId(u32)`, `TableId`/`OperationId`/`OpId` dictionaries. Intern routine ids
  ONCE (kills String-hashing in `presence.by_member`, `PdState`, the via map, feed-forward).
- Freeze the effect universe once; generate each effect's legacy `effect_key` at most once; **sort the
  9,137 identities once and assign `EffectId` in that output order.** Materialization then iterates in
  EffectId order with ZERO per-comparison `format!` and no per-routine sort. `effect_key` is a pure
  function of the identity — never stored per membership (regenerated only in `project_db_effect`).
  `record_variable_id` stays keyed by `operation_id` (existing `build_rvid_by_opid`).

### Compact rows + u8 via (Step 2) — "removes almost all of the 40GB/517s"
- Per-routine row = `{ terminal_base: EffectSetId, pd_delta: Range, base_via: Range, delta_via: Range }`.
- `via` → `u8 ViaRank` (5 canonical values; `Inherited=0..Direct=4`), stored parallel to the base/delta
  enumeration (~7.1MB at one byte), NOT a String-keyed HashMap. O(1) lookup: base membership ordinal
  (dense: sampled popcounts + ≤7 `count_ones`; sparse: binary search) → `base_via[ordinal]`.
- `temp_state` lives in the interned `EffectIdentity` — never stored per membership.
- Build via with bitmap ops, not edge×effect hash insertion: per routine, group callee terminal sets
  by the 5 edge ranks, OR into masks, evaluate present effects against masks (highest rank wins),
  overlay direct as rank 4; PD-effect via accumulated during the integer product-graph transitions.

### SCC-shared base + per-member delta (Step 3)
- `EffectSetId` arena of hash-consed sets (`HybridEffectSet`: `Sparse(Box<[EffectId]>)` for the long
  tail, `Dense([u64; 143]+rank_lut+cardinality)` above a ~256-entry threshold). All members of one
  effective SCC record ONE `EffectSetId` (this IS `closed_form_union`'s `C` — stop cloning it per
  member). Per-member `pd_delta` = the member's PD facts not in the base (mandatory: PD facts differ
  per member — `effects[v] = C ∪ member-v PD`). Feed-forward reads settled callees' `(EffectSetId,
  delta)`, not materialized Strings — eliminating per-edge re-intern and the `settled.clone()` in the
  multi-effective-SCC path.
- 7.1M memberships collapse to ~82k SetId refs + tiny deltas.

### Inverted index for the hover (Step 4)
- `ReverseEffectIndex`: `effect_to_sccs[EffectId] → PostingList<EffectiveSccIx>`,
  `scc_members[SccIx] → [RoutineIx]` (CSR), `effect_to_delta_routines[EffectId] → [RoutineIx]`,
  and table-level aggregates `table_to_sccs` / `table_to_delta_routines`. Posting lists adapt
  sorted-vec / dense-bitmap / Roaring by cardinality. Built in one transpose pass while iterating each
  SCC base.
- **Down** (routine→effects): `base[routine_set[r]] ∪ delta[r]`, O(result). "Does R touch table X?" =
  `table_to_sccs[X].contains(routine_to_scc[r]) || table_to_delta_routines[X].contains(r)` — no set
  decompression. **Up** (table/effect→routines): iterate `effect_to_sccs[E]`, expand via `scc_members`,
  merge `effect_to_delta_routines[E]`. Ancestor-scoped ("callers of R that touch X") = reverse-DAG BFS
  from r's SCC ∩ `table_to_sccs[X]`, memoized. Return count/top-N/paged (a 50k-routine result is not
  itself low-latency regardless of index speed).

### Public API
- `SummaryBundle { summaries: Vec<CompactRoutineSummary>, effects: EffectStore }` with
  `db_effects(routine) -> impl Iterator<Item = DbEffectRef<'_>>` (borrows dictionary strings). Owned
  `DbEffect` built only when a legacy caller explicitly asks. aldump/fingerprint/diff **stream** their
  projection to the writer — never build 7.1M owned `PDbEffect`s in RAM; the JSON output-size cost is
  the writer's, not `compute_summaries`'s RSS. Fingerprints hash ordered `(EffectId, ViaRank)` +
  universe/version metadata, not millions of formatted strings.

---

## Correctness spine

- **Representation-only change ⇒ exact output preservation.** The reframed differential (v2 ==
  frozen-baseline) must stay green: the lazy `DbEffect` view over the new store must reproduce
  byte-identical `Vec<DbEffect>` (same effects, temp_state, via, record_variable_id, in the same
  `(effect_key, operation_id)` order) per routine, on the 10 fixtures + the CDO whole-program case.
- **Goldens byte-identical** (`scripts/check-goldens`) except the deliberately-retired trace goldens
  (Part A step 2), which are inspected, not blind-regenerated.
- **DO byte-identical** on all identity fields + detectorStats.
- **Perf gate** (`tests/perf_bounds.rs`): the existing `compute_summaries` bound, tightened to the new
  linear/compact reality; add a memory assertion if feasible.
- **8020 re-measure** with the phase-split instrument: db-solver seconds, peak RSS <1GB (target
  <300MB), total substantially down from 1322s.
- **Staged, lowest-risk-first** (both consultations): (1) integer identities + kill comparator
  formatting → re-measure (expected: most of 517s gone); (2) compact rows + u8 via → re-measure
  (expected: most of 40GB gone); (3) SCC-shared SetId + deltas; (4) inverted index. Each step is
  differential-gated and independently landable.

---

## Risks & open questions

1. **aldump trace retirement (Part A step 2)** — the R3a-2 trace goldens encode the 58-pass trajectory;
   v2 is trace-free. Resolve by inspecting the golden diff (re-point to final-semantics vs retire the
   dump mode). Never blind-regen.
2. **R3b experiment deletion** — a whole subsystem + its tests. Approved-for-deletion gate: confirm
   (re-grep) zero shipping consumers before removing.
3. **Universe grows during PD solving** — assign EffectIds in sorted order requires either a
   freeze-then-sort-then-remap pass after PD discovery, or a post-solve global sort before
   materialization. Pick one (both consultations note the tension; the post-solve global freeze is
   cleanest).
4. **`via` bitmap-build parity** — the grouped-by-rank mask approach must reproduce the old
   per-effect `merge_via` winner exactly (rank max, first-wins on tie); differential-gated.
5. **Hybrid sparse/dense threshold** — measure before tuning; a dense-everywhere fallback is <1GB and
   acceptable as v1.

---

## Sequencing

**Part A (retirement) FIRST**, then **Part B steps 1→2→3→4**, each differential-gated + re-measured.
The frozen v2 baseline (A.1) is the correctness anchor throughout Part B.
