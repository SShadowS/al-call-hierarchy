# L4 db-effect Store Redesign + Old-Solver Retirement — Design Spec (rev 4)

**Date:** 2026-07-22 (rev 4 — incorporates 3 review rounds by gpt-5.6-sol + claude-fable-5)
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
- ⟨rev3⟩ **NEVER reassign EffectIds.** EffectIds stay intern-order forever (no remap — a remap would
  rewrite every bitset word-layout, delta, via table, and hash-cons key). Instead, at universe freeze,
  compute ONE `key_rank: Vec<u32>` (`EffectId → rank under (effect_key, operation_id)`, from the
  cached keys). This resolves the round-1 order/no-remap tension: intern order and key order differ,
  so **all ordered iteration and the base∪delta ordered-merge (Step 3) sort by `key_rank`, NOT by raw
  `EffectId`.** Bitset storage/membership stays keyed by `EffectId` (word `id/64`); only the OUTPUT
  ordering uses `key_rank`. Materialization walks a set's members and orders them by `key_rank[id]`.
- ⟨rev⟩ Order correctness (confirmed by both reviewers): emitting a member's present ids in
  `key_rank` order (== `(effect_key, operation_id)` order) reproduces the legacy `Vec<DbEffect>` order
  byte-for-byte (each member's sequence is a subsequence of the global key order). `operation_id` is
  embedded in `effect_key`, so the secondary key is vacuous — keep it belt-and-braces. Preserve
  lexical `effect_key` ordering exactly (e.g. `p10 < p2` bytewise) — do NOT substitute a structured
  numeric temp comparator. The projection layer re-sorts by stable-projected keys (a second parity net).
- ⟨rev4⟩ **Universe-freeze lifecycle (explicit):** (1) during solving, build with growable
  `Vec<u64>` / sparse builders; ⟨rev4 P0⟩ (2) **complete ALL identity discovery before freeze — not
  just PD.** Explicitly collect + intern every identity source: (a) every direct base Known/Unknown
  effect of every routine, (b) every retained fixed-leaf identity, terminal AND PD, (c) every
  retained per-routine PD fact, (d) every PD→Known/Unknown terminal emission. (Terminals never revert
  to PD, so discovery terminates.) (3) freeze universe length `U`; **after freeze no `intern()` may
  create an id — shared-set/closure construction uses CHECKED lookup and a debug-assert/test fails on
  a missing identity** (guards against a terminal discovered post-freeze); (4) resize dense sets to
  `ceil(U/64)` `Box<[u64]>`, zero + validate tail bits; (5) compute cached `effect_key` per id +
  `key_rank`; (6) build dense `rank_lut`/cardinality; (7) build shared sets + hash-cons + per-set
  cached `ordered_ids`; (8) build reverse indexes. No key-order EffectId remap step exists.
- ⟨rev4⟩ **`key_rank` availability window:** through Steps 1-2 the solver still materializes per-SCC
  mid-solve, and ordering there uses the Step-1 cached-`&str` sort. `key_rank` is computed only at
  freeze (post-solve) and drives ordering ONLY for post-freeze output/projection (Step 3+, once
  feed-forward is on ids). Do not consult `key_rank` mid-solve — it does not exist yet.

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
  on cache load; build `rank_lut`/cardinality once at freeze — ⟨rev4⟩ there is NO EffectId remap, so
  no post-remap rebuild). All members of one effective SCC
  record ONE `EffectSetId` (this IS `closed_form_union`'s `C` — stop cloning per member). Per-member
  `pd_delta` = member PD facts not in the base (mandatory — `effects[v] = C ∪ member-v PD`).
- ⟨rev⟩ Invariants (test them): deltas sorted ascending + unique + exactly `delta \ base`; via arrays
  exactly parallel; sparse and dense iterators both yield ascending-`EffectId` order (storage order);
  hash-consing compares logical contents (canonical content hash over the sorted id sequence
  regardless of repr, with full equality on hash collision); threshold conversion preserves
  cardinality/order; ranges bounds-checked on deserialize. **Merged base∪delta OUTPUT iteration is an
  ORDERED merge by `key_rank`, NOT base-then-delta append and NOT raw-EffectId order** — PD and
  terminal variants of the same base effect interleave in `(effect_key, operation_id)` order (temp is
  the last key fragment).
- ⟨rev4 P1⟩ **Cache a `ordered_ids: Box<[EffectId]>` per `EffectSetId`** (the base's members sorted by
  `key_rank`), built once when the set is interned — so the 797-member base is NOT re-sorted per
  routine. Each routine's `pd_delta` is likewise stored/kept in `key_rank` order. The base∪delta
  emit is then a linear O(result) two-way merge of two already-ranked runs. Via lookup still uses the
  EffectId-based membership ordinal (storage order), independent of the emit order.
- ⟨rev3⟩ **When a routine's `terminal_base` is canonicalized to a shared `EffectSetId` in Step 3,
  REBUILD its `base_via` range** so the via bytes stay aligned to the shared set's ordinal order (a
  Step-2→3 set-id swap without rebuilding `base_via` silently misaligns provenance — the "via arrays
  exactly parallel" invariant must be re-established against the shared set).
- ⟨rev3⟩ **Fixed leaves get a singleton `EffectClassIx` each.** Effect classes = effective SCCs (fixed
  leaves/missing removed) PLUS one singleton class per RETAINED fixed leaf — normalize the leaf's own
  settled summary into `terminal_base` + `pd_delta` preserving its OWN via values, so leaves support
  down/up queries, `class_of[routine]`, and projection like any routine. Missing routines get no row.
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
- ⟨rev3⟩ **Table posting — ONE disjoint contract** (not "disjoint OR dedup"): `table_to_delta_routines[T]`
  contains R **iff** `delta(R)` touches T **AND** `base(class(R))` does NOT touch T. Then table-SCC
  expansion and delta postings are disjoint by construction; test the invariant. (At effect level
  `delta = delta \ base` already makes them disjoint.)
- **Down** (routine→effects): `base[routine_set[r]] ∪ delta[r]` ordered-merge by `key_rank`,
  O(result). "Does R touch table X?" = `table_to_sccs[X].contains(class_of[r]) ||
  table_to_delta_routines[X].contains(r)` — no set decompression. **Up** (table/effect→routines):
  iterate postings, expand via `class_members`, merge delta routines. ⟨rev4 P1, DECIDED⟩ Expanding
  sorted class postings does NOT yield globally-sorted `RoutineIx`, so for deterministic paging
  **build a temporary routine bitmap** (~12.5KB for ~100k routines), set bits while expanding classes
  + delta postings, then iterate ascending `RoutineIx` for count/top-N/paging. (One method, not a
  menu.)

### Public API
- `SummaryBundle { summaries: Vec<CompactRoutineSummary>, effects: EffectStore }` with
  `db_effects(routine) -> impl Iterator<Item = DbEffectRef<'_>>` (borrows dictionary strings). Owned
  `DbEffect` only when a legacy caller asks. aldump/fingerprint/diff **stream** their projection to
  the writer — never build 7.1M owned `PDbEffect`s in RAM.
- ⟨rev3⟩ **Fingerprints: EXACT byte-preservation of the existing sequence.** `stable_summary_fingerprint`
  today encodes each effect as `projected_effect_key + ":" + via` (verified: it hashes only that, not
  op/table/operation/temp separately — those are already inside `effect_key`). The requirement is
  byte-identical output — so STREAM that exact existing logical sequence (projected key `:` via), do
  NOT expand into an op/table/operation/temp tuple (that would change the fingerprint bytes). The
  hashed bytes must be identity-derived (stable-projected keys), NEVER raw `EffectId` (a
  per-run/per-workspace dense index — adding one earlier-sorting effect renumbers later ids and would
  change fingerprints for unchanged routines). Raw `EffectId` is for same-universe ephemeral equality
  only.
- ⟨rev4 P1⟩ **Projection sort tie-break:** the projection re-sorts by stable-projected keys; when two
  internal identities collapse to an equal `(projected_effect_key, projected_operation_id)`, tie-break
  by the internal `key_rank` — sort key `(projected_effect_key, projected_operation_id,
  key_rank[EffectId])` — to reproduce the existing stable-sort order exactly.

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
2. **Cut `run_and_project`** (`summary.rs` ~838, pipeline call ~869) off old → v2. v2 is trace-free; the R3a-2 trace
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
  re-measure (most of 517s gone); Step 2 (compact rows + u8 via) → re-measure (⟨rev3⟩ compute-time
  drops, but a chunk of the RSS win arrives only at Step 3 — through Steps 1-2 the solver still
  materializes per-SCC `Vec<DbEffect>` mid-solve and feed-forward still clones settled Strings; the
  797-member `c.clone()` + settled-map strings survive until Step 3 reads `(EffectSetId, delta)`. Do
  NOT bill Step 2 for the full 40GB); Step 3 (shared SetId + deltas + feed-forward on ids →
  re-measure: the RSS win lands here); Step 4 (inverted index). Then Part B (retirement).
- ⟨rev3⟩ Each step is differential-gated against old on FIXTURES + generated small-graphs; the CDO
  differential leg (~729s/40GB on the old solver, `CDO_WS`-gated) is opt-in per step but **MANDATORY
  before Part B** — a step "gated on fixtures only" is not proven.
- ⟨rev3⟩ Add a debug/test assertion counting present effects with NO attributed via (the `"inherited"`
  floor must never silently hide a real provenance gap).

---

## Risks & open questions ⟨rev⟩

1. ⟨rev3, RESOLVED⟩ **EffectId order** — EffectIds are NEVER reassigned (no remap); storage is
   EffectId-keyed, OUTPUT order is by a frozen `key_rank[EffectId]`. This removes the round-1
   order/no-remap contradiction without a rewrite of bitsets/deltas/via/hash-cons.
8. ⟨rev3⟩ **Fixed leaves** — each retained fixed leaf is a singleton `EffectClassIx` with a normalized
   base+delta+via row; missing routines get no row. (Do not leave leaves classless — they answer
   queries.)
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
