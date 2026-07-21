# L4 Summary-Fixpoint Redesign — Design Spec

**Date:** 2026-07-21
**Status:** design — pre-implementation
**Goal:** Collapse `compute_summaries` from ~924s (one 797-member SCC = 729s, ~40 GB peak) to
seconds, by replacing the string-keyed, provenance-carrying Jacobi fixpoint with an interned
bitvector dataflow + closed-form SCC union + on-demand witness, WITHOUT changing the final
summaries detectors consume (differential-gated, byte-identical output).

**Architecture in one line:** partition the effect-summary domain into a *distributive* db-effect
presence lattice (solved closed-form per SCC over an interned universe) and a separate
`parameter_roles` fixpoint (monotonized so its convergence cap can be dropped); reconstruct `via`
provenance in one post-convergence pass; keep the detector-facing summary shape via a compact,
lazily-materialized representation.

---

## Global Constraints

- **Final summaries byte-identical.** The real contract is the settled per-routine
  `FullRoutineSummary`/`RoutineSummary` that `DetectorContext.summaries` exposes and every detector
  reads (`d1` reads `ctx.summaries.get(edge.to)` etc.). All existing goldens, CDO north-star metrics,
  and detector outputs MUST be preserved. Divergence = bug, not a rebaseline.
- **The per-pass `RawSccTrace` is NOT part of the contract.** It is an internal fixpoint-trajectory
  artifact used as a test oracle today (`summary.rs:678` explicitly notes the change-key drives the
  trajectory). The new solver will not reproduce the 58-pass Jacobi trajectory; the trace oracle is
  updated to compare *final* semantics, or the legacy Jacobi path is retained ONLY behind a
  trace-compat test flag.
- **Engine-never-throws.** No new panics; fail-closed on malformed input as today.
- **Determinism.** All output ordering stays deterministic (interned-ID order must reproduce the
  existing `(effect_key, operation_id)` sort; `via` reconstruction is a pure function of final sets).
- **rustfmt per file; never `cargo fmt`. Stage explicitly; never `git add -A`. Never push/merge to
  master without explicit request.** (CLAUDE.md.)

---

## Background — the measured root cause

`compute_summaries` (`src/engine/l4/summary_runner.rs`) computes per-routine interprocedural
DB-effect summaries over the call graph's Tarjan SCC condensation, in reverse-topological order;
each recursive SCC runs a Jacobi fixed point (`run_one_scc`, ~line 1067).

On the real BC Base Application (8020 files), the dominant SCC measured (trace
`logs/trace-probe.json`, `largest_scc_effect_universe` + `jacobi.pass` series):

| Metric | Value |
|--------|------:|
| SCC members (mutually-recursive routines, rooted at a central BC manager codeunit) | **797** |
| Distinct effects (universe) | **9,137** |
| total_memberships (Σ per-member `db_effects.len()`) | **7,122,286** (~8,900/member) |
| mean Jaccard between members' effect sets | **0.99997** |
| passes to converge | **58** (not cap-bound) |
| wall time, this one SCC | **729 s** |
| `compute_summaries` total | **924 s** |
| peak RSS | **~40 GB** |

The dataset is tiny (9,137 effects, 797 routines). The cost is representation, not volume:

1. **String-keyed domain.** `RoutineSummary.db_effects: Vec<DbEffect>` where
   `DbEffect.effect_key: String` = `"{op}|{table_id}|{operation_id}|{temp_frag}"` (~60 chars,
   `effect_lattice.rs:122`). `compose_routine` (`summary_runner.rs:351`) builds a
   `BTreeMap<String, DbEffect>` per member per pass, unions every callee's ~8,900 effects into it
   (O(log U) string compares), sorts by string, then `summary_change_key` (`summary.rs:670`)
   re-formats `"{effect_key}:{via}"` over the whole set for change detection. Per pass ≈ 7.1M
   string-map ops + sorts + change-key formats. 58 passes ≈ 3×10¹¹ char-ops.
2. **Provenance in the fixpoint domain.** `via` (`summary.rs:658`, one of 5 ranks,
   `effect_lattice.rs:147`) is folded into the change-key. The effect *set* stabilizes fast but
   `via` churn drags convergence to 58 passes.
3. **Domain coupling.** `SummaryChangeKey` (`summary.rs:657`) carries `db_effects` AND
   `parameter_roles` in one key, so any role change re-dirties the (expensive) effect composition.
4. **Memory.** Jacobi keeps two `HashMap<String, RoutineSummary>` generations live (`snapshot` +
   `next_pass`, `summary_runner.rs:1236,1240`) plus `key_cache` (~7.1M formatted strings). 7.1M
   `DbEffect` structs × 5 heap `String`s each ≈ 2–3 GB/generation → ~40 GB peak.

This is the classic **bitvector dataflow framework** (Kildall / Gen-Kill) implemented with strings
and provenance. Two independent external analyses (gpt-5.6-sol, claude-fable-5), each having read
`summary_runner.rs`/`summary.rs`/`effect_lattice.rs`/`cfg_walker.rs`, converged on the same fix and
corrected two overstatements in the initial plan (recorded below). Full analyses:
`scratchpad/pi-sol-answer.md`, `scratchpad/pi-fable-answer.md`.

---

## The corrected algorithm (the crux)

The db-effect transfer is NOT a plain union — it is `X_v = B_v ∪ ⋃_{e=(v,w)} T_e(X_w)` where `T_e` is
the per-callsite transfer (`compose_routine:404-439`):

- `Known(_)` / `Unknown` effects transfer by **identity** (key unchanged).
- `ParameterDependent(i)` effects are **substituted per callsite** (`substitute_pd_temp_state`,
  `summary_runner.rs:691`): the callee-frame index `i` is remapped through the caller's argument
  binding, yielding `Known(t)` / `Known(f)` / `Unknown` / `PD(j)` in the caller frame. The result is
  **re-keyed** (`effect_key_of`, `summary_runner.rs:409`).

### Correction 1 — PD substitution feeds the plain-key universe, so it must be solved FIRST

`mean_jaccard ≈ 1.0` proves the *settled* sets barely differ, NOT that PD work is negligible — many
plain (terminal) keys in the settled set may have been *generated* by PD substitution during
propagation. So "plain union is closed-form, zero iteration" is unsound as ordered. The exact
algorithm (both models, independently):

**Step A — solve PD as product-graph reachability (no Jacobi).**
For each PD-carrying base effect, a routine's token ∈ `{Known(t), Known(f), Unknown} ∪ {PD(j) :
j∈params(r)}`. `substitute_pd_temp_state` is a *fixed function of the binding* per edge, so
`caller-set = ⋃ subst_edge(callee-set)` — a union of images under fixed maps = **distributive**.
Solve by BFS/worklist over `(routine, token)` product nodes (semi-naive: propagate only newly
discovered tokens). Domain per routine ≤ `#params + 3`. This is a "miniature IDE that degenerates to
reachability" — the ONLY genuinely per-member effect flow.

**Step B — closed-form plain-key union per SCC.**
Gen-set = base plain keys of all members ∪ all `Known/Unknown` keys generated by Step A. In a
strongly-connected component every member reaches every other, so the plain (identity-transferred)
union is **identical for all members** = `⋃ gen-set ∪ ⋃ successor-SCC plain summaries`. One union over
the interned universe (bitset OR); assign to every member. Zero iteration.

**Step C — per-member result** = uniform plain union (B) ∪ that member's own PD-keyed facts (A).

**Step D — reconstruct `via` in one post-pass.** The fold *replaces* callee `via` with
`via_for_edge_kind(edge.kind)` (`summary_runner.rs:394,413,421`) — via never propagates transitively.
So `via(m,k) = max( base_via if k∈base(m),  max over edges m→c with T_e(k')=k, k'∈set(c) of
via_for_edge_kind(edge.kind) )`. One scan over final sets + out-edges. `via` leaves the fixpoint
domain entirely (validates original proposal #2).

**Correctness caveat — fixed leaves in Tarjan SCCs.** `run_one_scc` excludes fixed `leaf_summaries`
from recomputation (`summary_runner.rs:1088,1161,1176,1210,1246`). If a Tarjan SCC contains a fixed
leaf, the *effective* equation graph over recomputed members is no longer strongly connected — the
"identical union for all members" property fails. **Derive effective SCCs over non-leaf members only
(or treat fixed leaves as external seeds)** before applying Step B.

### Correction 2 — the pass count is (partly) `parameter_roles`-driven, not effects-driven

`SummaryChangeKey` couples `db_effects` and `parameter_roles` (`summary.rs:657-665`), and
`compose_routine` runs the full `walk_param` CFG walk + cross-call role composition EVERY pass. The
per-pass *cost* is effects (strings); the pass *count* is roles/PD-chain length. Therefore the fix
**requires domain-splitting**: `db_effects` gets its own settle (now closed-form → zero passes),
independent of the roles fixpoint. Roles keep iterating but cheaply (no 8,900-effect string work).

---

## Part 1 — db_effects redesign

**Files:** `src/engine/l4/summary_runner.rs` (compose/run_one_scc), `src/engine/l4/summary.rs`
(`DbEffect`, `SummaryChangeKey`, projection), `src/engine/l4/effect_lattice.rs` (interning),
new `src/engine/l4/effect_universe.rs` (interner), new `src/engine/l4/db_effect_solver.rs`
(the closed-form + PD-BFS solver). Detector-facing: `src/engine/l5/full_summary.rs`,
`src/engine/l5/detector_context.rs`, `src/engine/l5/detectors/d1.rs` (+ any detector reading
`summary.db_effects`).

1. **Interned effect universe.** Intern a *structured* effect identity, not the string. Key insight
   (sol): `op`/`table_id`/`operation_id` are fixed during PD substitution; only the temp-state
   fragment changes. Intern `(op_id, table_id, operation_id)` → a base `u32`; the temp-state tag is a
   small enum. Assign final effect IDs in the existing `(effect_key, operation_id)` sort order (or
   keep an ID→key permutation) so deterministic materialization needs no string re-sort. Do NOT
   assume the 9,137-key universe is known up front — PD substitution creates variants; intern lazily.
2. **Bitset presence domain.** Per-routine plain-key presence = `Vec<u64>` bitset over the interned
   universe (~143 words for this SCC). `compose`/union = bitwise OR; change-detect = bitset equality.
3. **PD product-graph BFS** (Step A) in `db_effect_solver.rs`: semi-naive worklist over
   `(routine, token)`; PD→Known/Unknown results feed the plain gen-set.
4. **Closed-form SCC union** (Step B/C) over effective (leaf-excluded) SCCs.
5. **via post-pass** (Step D): produce the per-effect via-rank as a compact `u8` array or reconstruct
   on demand for reported effects only.
6. **Domain-split the change-key.** `db_effects` no longer participates in the recursive change-key
   (it is closed-form). The remaining change-key (roles/uncertainties) drives Part 2's fixpoint only.
7. **Compact / lazy detector-facing representation.** Retaining `Vec<DbEffect>` (5 `String`s each) for
   7.1M entries is ~1 GiB+ regardless of how it's built (both models corrected the original "sub-GB"
   claim). To actually reach sub-GB: store `CompactEffect { effect_id: u32, via_rank: u8 }` +
   the global interner, expose a borrowed `DbEffectView` iterator (lazy `effect_key`), and let
   detectors query presence/via directly — materialize full `DbEffect`s only for *reported* effects.
   This touches the detector-facing summary API (the d1-cohort-redesign precedent: consumers are ours
   to change).

**Expected:** the 797-SCC effect fixpoint 729s → sub-second; peak RSS 40 GB → sub-GB.

---

## Part 2 — parameter_roles monotonization + cap removal

**Files:** `src/engine/l4/cfg_walker.rs` (`apply_call` ~1168, exit-effect block ~1295-1332),
`src/engine/l4/summary_runner.rs` (split roles fixpoint from effects; `MAX_FIXED_POINT_ITERATIONS`),
`src/engine/l4/summary.rs` (`Dirty`/`Loaded`/`LoadedFields` domains).

`apply_call` is **genuinely non-monotone** (verified in source):
- `Loaded` is a flat domain (`No⊔Yes=Unknown`) over `EffectPresence`'s linear `no<unknown<yes`
  (`effect_lattice.rs:20-68`) → a *larger* callee input yields a *smaller* caller output.
- `out.current_loaded_fields = field_list_to_loaded(cr…)` (`cfg_walker.rs:1302`) is a destructive
  overwrite, not a join.
- `Dirty` transitions (`cfg_walker.rs:1313-1331`) are equality-guarded with no consistent partial
  order over `{Pristine, Persisted, DirtyV, Unknown}`.

A finite non-monotone deterministic system has **no** finite convergence bound (only eventual
periodicity — the 1000-cap catches cycles). So the cap CANNOT be removed for the combined solver; it
is not a residual bug from an earlier fix. Path to removal:

1. Give `Dirty` a real lattice (e.g. `Pristine < Persisted < DirtyV < Unknown-top`, or a small
   powerset), replace equality guards with joins, and **join** (not overwrite) `current_loaded_fields`.
2. Optionally separate may/must dimensions so "unknown-whether" and "definitely" stop colliding.
3. Solve monotone c1b role facts first, freeze, then path facts over stable call effects
   (stratification).

Then the roles fixpoint has bound = lattice-height × SCC-diameter and the cap becomes an assert.
**Precision:** a few facts may degrade to `Unknown`/joined lists — differential-gate to confirm the
change is acceptable (final summaries must stay byte-identical, or any diff must be adjudicated as a
soundness improvement, per project doctrine). **Interim (if Part 2 lands after Part 1):** keep the cap
but add repeated-global-state (2-cycle) detection so oscillation is diagnosed, not silently run to
1000.

---

## Part 3 — capability_cones

**Files:** `src/engine/l4/capability_cone.rs`.

Confirmed same disease: `ConeFacts = BTreeMap<String, ConeFactEntry>` (`capability_cone.rs:1338`),
`merge_cone` keeps min-hop-distance + canonical-rep witness (`:1342`), `retag` replaces `via` per
first-hop edge (`:1360`) — the identical string-keyed-dedup-with-witness shape, ~50s. Same medicine:
intern the cone universe, compute the cone closure over the SCC condensation (min-distance is a
tropical-semiring closure, not a plain union — carry an interned key → min-dist map; still avoids
string BTreeMaps and per-pass re-sorting), reconstruct the witness (min-dist rep + via) on demand.
Scope detail to be finalized in the plan after a short profiling confirm (the min-distance tie-break
makes this a shortest-path closure, not pure reachability — design the condensation accordingly).

---

## Part 4 — substrate cache (optional gravy)

**Files:** new cache module under `src/engine/l4/` or `src/engine/gate/`; the existing dep-cache
convention (`~/.al-sem/cache/`, `cache_prune.rs`) is the sibling to follow.

After Parts 1–3 the run drops ~25 min → ~2–3 min with NO cache, so this is optional. When wanted:
content-address the L4 substrate (interned universe + per-routine bitsets + roles + cones) by a hash
of the app's source set; reload instead of recompute when unchanged. The `run_one_scc` doc already
names the incremental seam: each SCC summary depends only on `members + successor-summaries + inputs`,
"NOT a monolithic condensation" — so a base-app substrate can be cached and only changed/added apps
recomputed. Keyed by BC base-app version + content hash (the user's original instinct, now aimed at
the substrate rather than the parse).

---

## Freebie — redundant `fresh_coverage`

`run_analyze_with_exit` (`src/engine/gate/run.rs:206`) runs `program::resolve::full::fresh_coverage`
— a *second, full* program resolve (58s on the 8020 corpus) — only to produce coverage stats for the
report envelope/preflight. **Investigate** whether it can reuse the L3 assembly already built for the
detector path, or be skipped when coverage is not consumed (note: the JSON envelope and preflight
warning DO surface coverage, so this is a "reuse/derive", not an unconditional skip). Sized as a
small independent task, gated behind confirming output is unaffected.

---

## Correctness spine

The d1-cohort-redesign discipline applies verbatim:

- **Differential-gate the new DB solver against the old** on generated SCCs covering every transfer
  shape both models flagged: PD-index permutations, PD→Known, PD→Unknown, multiple callsites to the
  same callee, edge-kind `via` collisions, external successor PD seeds, self-loops, and **fixed leaves
  inside Tarjan SCCs**. Assert new `db_effects` (with reconstructed via) == old, per routine.
- **Final-summary byte-identity** on the fixture corpus (existing goldens) + CDO north-star metrics
  unchanged (`scripts/cdo-gate`), + all detector goldens (`scripts/check-goldens`).
- **Trace oracle:** update `RawSccTrace`-based tests to compare final semantics, or retain the legacy
  Jacobi solver behind a `#[cfg(test)]`/env flag purely for trace-compat, clearly marked dead on the
  production path.
- **Perf gate:** add a bound so `compute_summaries` on the synthetic corpus can't regress by an order
  of magnitude (mirrors `tests/perf_bounds.rs`).

---

## Risks & open questions

1. **Roles precision drift (Part 2).** Monotonizing `apply_call` may degrade a few role facts to
   `Unknown`. Must be differential-checked and adjudicated (soundness-improving diffs are acceptable
   per doctrine; silent detector-output changes are not).
2. **Interned-ID determinism.** Final effect-ID assignment MUST reproduce the existing sort so
   materialized output stays byte-identical. Test explicitly.
3. **Detector API change (Part 1.7).** Moving detectors from `Vec<DbEffect>` to a compact/lazy view
   is the largest blast radius; every `summary.db_effects` reader must migrate. Enumerate them in the
   plan (`d1` confirmed; sweep for others).
4. **Cone closure is shortest-path, not pure reachability (Part 3)** — the min-distance tie-break
   needs a tropical closure over the condensation, designed carefully.
5. **Cap removal ordering.** If Part 2 slips, Part 1 still ships behind the retained cap (effects no
   longer iterate; only roles do), so Part 1's win is independent of the cap.

---

## Sequencing (phased plan)

1. **Phase 1 — db_effects** (interned universe + PD-BFS + closed-form union + via post-pass +
   domain-split + compact repr + differential harness). Drive to green (byte-identical + perf win)
   before Phase 2. Biggest lever; independent of the cap.
2. **Phase 2 — parameter_roles** monotonization + cap removal (+ interim oscillation detection).
3. **Phase 3 — capability_cones** (after a short profiling confirm).
4. **Phase 4 — substrate cache** (optional; decide after Phases 1–3 re-measure).
5. **Freebie — fresh_coverage** reuse/skip (independent; can land any time).

Each phase is differential-gated and preserves final output; each ends with a re-measure on the 8020
corpus and the CDO north-star ratchets.
