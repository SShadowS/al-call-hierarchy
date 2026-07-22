# L4 db-effect Store Redesign + Old-Solver Retirement — Design Spec (rev 2)

**Date:** 2026-07-22 (rev 2 — incorporates round-1 review by gpt-5.6-sol + claude-fable-5)
**Status:** design — pre-implementation
**Follows:** `2026-07-21-l4-summary-fixpoint-redesign-design.md` (Phase 1, done-on-branch: the
closed-form v2 db-effect solver, byte-identical to the old Jacobi). This Phase-1.5 sub-arc (A)
redesigns the db-effect **representation** so the (correct) v2 solver is fast (seconds) and compact
(sub-GB) and bidirectionally queryable, then (B) retires the old solver so one path remains.

**Goal:** Keep every db-effect fact fully computed and queryable, but store it as a shared, interned,
columnar structure so `compute_summaries` drops from **517s / ~40GB** to **seconds / <300MB**, and
expose a bidirectional query index (down: routine→effects; up: table/effect→routines) for a future
VSCode hover. Then delete the old Jacobi solver and its dead R3b incrementality experiment.

**Sequencing decision (rev 2):** the redesign lands FIRST with the old solver retained ONLY as the
v2-vs-old differential oracle; retirement is the FINAL task, after parity is proven. Rationale (both
reviewers): this change alters solver *structure* (universe-discovery order, PD-via accumulation), so
it is NOT purely representational — keeping an independent-algorithm differential (against new,
generated, and CDO inputs) through the biggest change is a materially stronger net than a
frozen-fixture replay. End state is still no-dual-paths.

**Reviewed inputs:** two source-read model consultations (design: `scratchpad/pi-{sol,fable}-repr.md`;
spec round-1: `scratchpad/pi-{sol,fable}-specreview.md`). Round-1 corrections are folded in and
marked ⟨rev⟩.

---

## Motivation (measured, 8020-file real BC Base Application)

Phase-split attribution (`compute_summaries_v2_phase_split`, instrumented): **db-effect solver 517s**
(roles fixpoint 0.8s — a non-issue); peak RSS ~40GB (unchanged by Phase 1). 82,716 Tarjan SCCs;
dominant one = 797 mutually-recursive routines; universe = 9,137 effects; mean Jaccard between the 797
members' sets = 0.99997; total memberships = 7,122,286.

The cost is NOT algorithmic (no Jacobi, no O(N²) — fixed). It is **per-effect String constant-factors
on materialization**: `materialize_member_db_effects` sorts each routine's effects with a
`format!`-per-comparison key; `reconstruct_via` builds a String-keyed `HashMap<(String,EffectId),
String>` (~7.1M entries); each of 797 SCC-members holds its own private ~9,000-element `Vec<DbEffect>`
(5 heap Strings each) even though `closed_form_union` builds them as `c.clone()` + a tiny PD delta.
**The sharing exists in the solver and is destroyed at the last line.** ⟨rev⟩ Note the ~40GB is not
100% db_effects — the `settled` map's cloned `RoutineSummary` uncertainties/roles are a residual to
measure separately (see RSS target).

**Why not skip db_effects on the analyze path** (provably unread there today): the product needs it —
future detectors read per-routine db_effects, and a planned VSCode hover answers "is table/record X
touched by any DB action transitively, up or down the call stack." db_effects must stay computed AND
become cheaply queryable both directions.

---

## Prior art (a solved shape — not first)

- **SCC-condensed bitvector dataflow** (textbook): collapse SCCs, reverse-topo over the condensation,
  union successor bitvectors, share ONE closure among SCC-equivalent nodes.
- **Andersen inclusion-based points-to with SCC/cycle collapse** — Fähndrich et al. PLDI'98 (*Partial
  Online Cycle Elimination*), Hardekopf & Lin PLDI'07 (HVN/HRU): cycle nodes have identical solution
  sets → one set per class + a rep pointer. Steensgaard unification does NOT apply (loses precision;
  we need exact parity).
- **Shared-set interning** — Heintze CLA (PLDI'01), hash-consed set constructor cache.
- **Compressed bitmap indexes** (Roaring) — for the reverse posting lists over the 82k/100k domain,
  NOT the dense 9,137-bit forward sets (a plain word-array is ~1.1KB).
- **NOT adopted as primary:** BDD/bddbddb (overkill for one-level class-shaped redundancy), full
  Datalog/Soufflé (oracle-worthy, not a smaller store), GRAIL/PathTree/2-hop (answer node-pair
  reachability — wrong predicate; our reverse query is a relation transpose).

---

## Part A — db-effect Store redesign (Steps 1-4, old solver kept as oracle throughout)

`Vec<DbEffect>` becomes a lazy projection/view type only; the internal summary holds a compact handle
into a workspace-level `EffectStore`.

### Step 1 — interned identities + cached keys ⟨rev: made truly independent⟩
Lowest-risk, independently landable, removes the 10⁸ `format!` allocs WITHOUT any universe
reassignment:
- Intern `RoutineIx(u32)` once workspace-wide (kills String-hashing in `presence.by_member`,
  `PdState`, the via map, feed-forward, and `solve_side_facts`'s per-member String clone — fold that
  in here too). `EffectId(u32)` already exists.
- **Cache each effect's `effect_key` once** (a `Vec<String>` parallel to the universe's `by_id`, or
  interned `Box<str>`), computed at first sight — NOT recomputed per comparison. `materialize_member_db_effects`
  then sorts a member's present ids by the cached `&str` (+ `operation_id` tie-break), O(k log k)
  `&str` compares, zero allocation. This alone removes almost all of the 517s.
- ⟨rev⟩ Do NOT reassign EffectIds to sorted order in Step 1 — that requires a post-solve global
  freeze and a remap that rewrites every stored bitset word-layout, delta list, via table, and the
  hash-consed set arena (dedup hashes are id-dependent). If adopted at all, that remap is a Step-3
  post-solve pass with that stated blast radius. Step 1 keeps intern-order ids + cached-key sort.
- ⟨rev⟩ Order correctness (confirmed by both reviewers): iterating a member's present ids in
  `(effect_key, operation_id)` order reproduces the legacy `Vec<DbEffect>` order byte-for-byte (each
  member's sequence is a subsequence of the global key order). `operation_id` is embedded in
  `effect_key`, so the secondary key is vacuous — keep it belt-and-braces. Preserve lexical
  `effect_key` ordering exactly (e.g. `p10 < p2` bytewise) — do NOT substitute a structured numeric
  temp comparator. The projection layer re-sorts by stable-projected keys (a second parity net).

### Step 2 — compact rows + u8 via
- Per-routine row: `{ terminal_base: EffectSetId, pd_delta: Range, base_via: Range, delta_via: Range }`
  — pooled CSR ranges into global arrays (⟨rev⟩ NOT a per-row `SmallVec`: that multiplies inline
  capacity across ~100k routines and defeats columnar pooling; use `SmallVec` only as a transient
  builder, then sort/dedup/append to CSR).
- `via` → `u8 ViaRank` (5 canonical values `Inherited=0..Direct=4`), stored parallel to the
  base/delta enumeration (~7.1MB at one byte — ⟨rev⟩ this does NOT collapse with sharing; state that;
  per-class default + per-member exceptions is a later optimization only if it profiles). O(1)
  lookup: base-set membership ordinal (dense: sampled popcounts + ≤7 `count_ones`; sparse: binary
  search) → `base_via[ordinal]`; else the tiny `delta_via`. ⟨rev⟩ Keep the defensive `"inherited"`
  floor for any present effect with no attributed via.
- `temp_state` lives in the interned identity — never per membership. `effect_key` never stored
  per membership (regenerated only in `project_db_effect`). `record_variable_id` stays keyed by
  `operation_id`.
- ⟨rev⟩ **Staging vs Step 3:** Step 2's row references `EffectSetId`. To make Step 2 independently
  landable, Step 2 may create ONE set-arena entry per routine (no hash-consing yet) or use temporary
  compact rows; Step 3 canonicalizes exact SCC bases and replaces row refs with shared ids. State
  this so Step 2 is buildable alone.
- **via build ⟨rev⟩:** the TERMINAL half is a rank-group mask-OR and reproduces `merge_via` exactly
  (the 5 ranks are a bijection with the 5 strings, so an equal-rank tie is an identical string and
  "first-wins" is vacuous; max-rank = `merge_via`). Must reproduce: the `has_bit` presence gate
  (evaluate masks against FINAL presence), the base seed applying to PD-typed base effects too (they
  carry `via=Direct`), and the `"inherited"` floor. The **PD half is NOT a mask-OR** — the produced
  target bit depends on the callsite's argument binding. Keep the existing
  `attribute_pd_substituted_via` structure, integerized (`RoutineIx`, `u8`). ⟨rev⟩ If instead
  accumulating during Step A transitions: record a via candidate for EVERY traversed transition
  (PD→PD and PD→Known/Unknown), max-merged, EVEN when the destination product state was already
  visited (else a later higher-rank edge to a known PD state is lost) — Step A's `TerminalEmission`
  HashSet currently drops the edge kind, so carry `(routine, produced-id, via_rank)` through
  `apply_pd_transition`; and confirm coverage matches (Step A covers intra-SCC + seed-2 external
  edges; `attribute_pd_substituted_via` iterates ALL out-edges incl settled-successor PD). Gate all
  accumulated vias on final presence. Simplest safe v1: keep the integerized post-pass. Add fixtures:
  two callsites producing the same PD state at different ranks; first-transition rank-0 then
  later-transition rank-2/3; PD→Known colliding with a direct terminal; PD→PD colliding with a direct
  PD; a duplicate transition where `visited.insert` is false.

### Step 3 — SCC-shared base + per-member delta
- `EffectSetId` arena of hash-consed sets (`HybridEffectSet`: `Sparse(Box<[EffectId]>)` below a
  ~256-entry threshold, `Dense{ words: Box<[u64]>, rank_lut, cardinality }` above — ⟨rev⟩ `words`
  length = `ceil(frozen_universe_len / 64)`, NEVER a hardcoded `[u64;143]`; zero + validate tail bits
  on cache load; rebuild `rank_lut`/cardinality after any remap). All members of one effective SCC
  record ONE `EffectSetId` (this IS `closed_form_union`'s `C` — stop cloning per member). Per-member
  `pd_delta` = member PD facts not in the base (mandatory — `effects[v] = C ∪ member-v PD`).
- ⟨rev⟩ Invariants (test them): deltas sorted ascending + unique + exactly `delta \ base`; via arrays
  exactly parallel; sparse and dense iterators both yield ascending-EffectId order; hash-consing
  compares logical contents (canonical content hash over the sorted id sequence regardless of repr,
  with full equality on hash collision); threshold conversion preserves cardinality/order; ranges
  bounds-checked on deserialize. **Merged base∪delta iteration is an ORDERED merge by global key
  rank, NOT base-then-delta append** — PD and terminal variants of the same base effect interleave in
  `(effect_key, operation_id)` order (temp is the last key fragment).
- Feed-forward reads settled callees' `(EffectSetId, delta)`, not materialized Strings — eliminates
  per-edge re-intern and the `settled.clone()` in the multi-effective-SCC path.
- 7.1M memberships collapse to ~82k SetId refs + tiny deltas (⟨rev⟩ presence collapses; the u8 via
  table stays ~7.1MB).

### Step 4 — inverted index for the hover
- `ReverseEffectIndex`: `effect_to_sccs[EffectId] → PostingList<EffectClassIx>`,
  `class_members[EffectClassIx] → [RoutineIx]` (CSR), `effect_to_delta_routines[EffectId] →
  [RoutineIx]`, and table aggregates `table_to_sccs` / `table_to_delta_routines`. Posting lists adapt
  sorted-vec / dense-bitmap / Roaring by cardinality. One transpose pass over each SCC base.
- ⟨rev⟩ **Two SCC notions:** `EffectClassIx` = the effect-sharing effective SCC (fixed leaves/missing
  removed, can split a Tarjan SCC); `GraphSccIx` = the ORIGINAL call-graph SCC condensation. The
  effect transpose uses `EffectClassIx`. **Ancestor-scoped hover** ("callers of R that touch X") uses
  `GraphSccIx` for the reverse-DAG BFS — effective-SCC leaf-removal changes reachability semantics,
  so ancestor traversal must not use the effect-sharing DAG. Maintain both.
- ⟨rev⟩ **Table posting disjointness:** at effect level `delta = delta \ base` keeps SCC-expansion
  and delta-routines disjoint. At TABLE level a routine may hit table X via a base effect AND a
  distinct PD delta effect — so add to `table_to_delta_routines[X]` ONLY when the base does not
  already touch X, OR dedup during up-query merge. State the invariant.
- **Down** (routine→effects): `base[routine_set[r]] ∪ delta[r]` ordered-merge, O(result). "Does R
  touch table X?" = `table_to_sccs[X].contains(class_of[r]) || table_to_delta_routines[X].contains(r)`
  — no set decompression. **Up** (table/effect→routines): iterate postings, expand via
  `class_members`, merge delta routines. Return count/top-N/paged (a 50k-routine result is not
  low-latency regardless of index speed).

### Public API
- `SummaryBundle { summaries: Vec<CompactRoutineSummary>, effects: EffectStore }` with
  `db_effects(routine) -> impl Iterator<Item = DbEffectRef<'_>>` (borrows dictionary strings). Owned
  `DbEffect` only when a legacy caller asks. aldump/fingerprint/diff **stream** their projection to
  the writer — never build 7.1M owned `PDbEffect`s in RAM.
- ⟨rev⟩ **Fingerprints hash STABLE-projected key material** (op/table/operation as stable ids,
  projected `effect_key`, temp, canonical via) exactly as `stable_summary_fingerprint` does today —
  NOT raw `EffectId` (a per-run/per-workspace dense index: adding one earlier-sorting effect
  renumbers later ids and would change fingerprints for unchanged routines, breaking cross-run/version
  stability and byte-identity). Streaming is fine; the hashed bytes must be identity-derived, not
  index-derived. Raw `EffectId` is for same-universe ephemeral equality only.

---

## Part B — Old-solver retirement (FINAL task, after Part-A parity is proven)

The old Jacobi solver stays as the **v2-vs-old differential oracle** through all of Part A. Retire it
once the compact store is proven at parity on: the 10 fixtures + ⟨rev⟩ **committed generated
small-graph fixtures** (collision / freeze-order / multi-callsite shapes) + the CDO whole-program
case. Then, in one final task:

1. ⟨rev⟩ **Freeze the complete-internal-surface baseline** (all `RoutineSummary` fields: effect
   sequence + order, `effect_key`, op/table/operation, temp, via, `record_variable_id`, uncertainties,
   has_unresolved, roles, in_recursive_cycle) on the fixtures + **CDO (non-optional in the freeze)**.
   NOT `stable_summary_fingerprint` (it omits internal fields). This becomes the post-retirement
   regression anchor. ⟨rev⟩ Record the pre-deletion commit/tag in the spec so the old oracle is one
   `git checkout` away for forensic re-differencing.
2. **Cut `run_and_project`** (`summary.rs:795`) off old → v2. v2 is trace-free; the R3a-2 trace
   goldens encoding the 58-pass trajectory retire → inspect the diff (re-point to final-semantics or
   retire the dump mode). Never blind-regen.
3. **Delete the R3b Salsa incrementality experiment** (`src/engine/l4/incremental/` + `tests/r3/r3b_*`)
   — consumed only by those tests (re-grep to confirm zero shipping consumers before removal). ⟨rev⟩
   Do NOT cascade-delete the shared `salsa::Update` derives on `RoutineSummary`/`DbEffect` if other
   Salsa users exist. ⟨rev⟩ Before deletion, preserve (as design notes / a branch tag) the reusable
   edit/minimal-invalidation fixtures, SCC-identity rules, fixed-leaf successor handling, and
   deterministic member-order tests — a future incremental path over the new store would want
   SCC-condensation-level invalidation (a redesign, but the test intent is reusable).
4. **Delete the old Jacobi:** `compose_routine`'s db_effects + uncertainty folds, `run_one_scc`,
   `compute_summaries` / `compute_summaries_with_leaves`, `RawSccTrace`, the `Detail::Jacobi`
   db_effects instrumentation. KEEP `compose_roles_only` / `run_one_scc_roles` + `solve_side_facts`.
5. Flip the differential test from `v2 == old` to `v2 == frozen-baseline`. Gate: goldens byte-identical
   (except inspected trace goldens), lib green, DO byte-identical.

---

## Correctness spine

- **Through Part A: live v2-vs-old differential** on the 10 fixtures + generated small-graph fixtures
  (the collision/freeze-order/multi-callsite shapes above) + the CDO whole-program parity test — an
  independent-algorithm net, not a replay. The lazy `DbEffect` view over the new store must reproduce
  byte-identical `Vec<DbEffect>` (effects, temp, via, record_variable_id, order) per routine.
- **Goldens byte-identical** (`scripts/check-goldens`); **DO byte-identical** (identity fields +
  detectorStats).
- **Perf gate** (`tests/perf_bounds.rs`): tighten the `compute_summaries` bound to the new reality;
  add a memory assertion if feasible.
- **8020 re-measure** (phase-split instrument): db-solver seconds, peak RSS <1GB (target <300MB) —
  ⟨rev⟩ if the residual is uncertainties/roles cloning in `settled`, name and address it, don't assume
  db_effects was 100% of the 40GB.
- **Staged, lowest-risk-first & each independently landable:** Step 1 (intern + cached-key sort) →
  re-measure (most of 517s gone); Step 2 (compact rows + u8 via) → re-measure (most of 40GB gone);
  Step 3 (shared SetId + deltas + feed-forward on ids); Step 4 (inverted index). Then Part B
  (retirement). Each step differential-gated against old.

---

## Risks & open questions ⟨rev⟩

1. **Step-1/Step-3 remap** — if EffectIds are ever reassigned to sorted order, the remap rewrites all
   bitsets/deltas/via/hash-cons entries; the design avoids this by using cached-key sort in Step 1.
2. **PD-via accumulation** — the mask trick is terminal-only; the PD half must record every transition
   and gate on final presence (keep the integerized post-pass unless proven equivalent).
3. **Fingerprint identity** — must be stable-projected keys, never raw EffectId.
4. **Dense set sizing** — `Box<[u64]>` at freeze; universe grows with PD variants mid-solve.
5. **Two SCC notions** — `EffectClassIx` (sharing) vs `GraphSccIx` (ancestry); don't conflate.
6. **RSS residual** — measure uncertainties/roles `settled` cloning after db_effects shrinks.
7. **R3b deletion** — re-grep zero shipping consumers; preserve reusable invalidation-test intent.

---

## Sequencing

Part A steps 1→2→3→4 (each differential-gated vs the retained old solver + re-measured), THEN Part B
retirement (freeze complete-internal baseline incl CDO → cut aldump → delete R3b → delete old Jacobi →
flip differential to baseline). Task 13 (Salsa migration) is MOOT once R3b is deleted.
