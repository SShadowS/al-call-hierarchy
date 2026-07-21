# L4 Summary-Fixpoint Redesign — Design Spec

**Date:** 2026-07-21
**Status:** design — pre-implementation (revised after two adversarial external reviews)
**Goal:** Collapse `compute_summaries` from ~924s (one 797-member SCC = 729s, ~40 GB peak) to
seconds, by replacing the string-keyed, provenance-carrying Jacobi fixpoint with an interned
bitvector dataflow + closed-form SCC union + on-demand witness, WITHOUT changing the final
summaries detectors consume (differential-gated, byte-identical output).

**Architecture in one line:** partition the effect-summary domain into a *distributive* db-effect
presence lattice (solved closed-form per *effective* SCC over an interned, workspace-scoped
universe) + independent monotone side-facts (uncertainties / `has_unresolved_calls`) + a separate
`parameter_roles` fixpoint; reconstruct `via` provenance in one post-convergence pass; keep the
detector-facing summary shape via a compact, lazily-materialized representation migrated behind an
adapter.

**Reviewed by:** gpt-5.6-sol and claude-fable-5 (both source-read `summary_runner.rs`,
`summary.rs`, `effect_lattice.rs`, `cfg_walker.rs`; full reviews in `scratchpad/pi-sol-review.md`,
`scratchpad/pi-fable-review.md`). Their P0 corrections are folded in below and marked ⟨rev⟩.

---

## Global Constraints

- **Phase 1 is STRICT PARITY.** The db-effect / uncertainty / provenance redesign MUST produce
  final per-routine summaries byte-identical to today — compared over the **complete internal
  `DbEffect`** (not only the projected `effect_key`/`via`; see ⟨rev⟩ payload note), all
  uncertainties, `has_unresolved_calls`, and `parameter_roles`. Any divergence in Phase 1 is a bug.
- **Phase 2 is a SEPARATELY APPROVED soundness change.** ⟨rev⟩ Monotonizing `parameter_roles` may
  legitimately change a few facts (the legacy solver's *converged* state is trajectory-dependent —
  the change-key is deliberately lossy at `summary.rs:670` and a key-collided member does not dirty
  its dependents at `summary_runner.rs:1279`, so the old fixed point can be a non-fixpoint artifact).
  Any Phase-2 output change is an explicit, adjudicated rebaseline — NOT part of the
  performance-preserving redesign, and never silent. Do not conflate the two contracts.
- **The per-pass `RawSccTrace` is NOT a contract.** It is an internal trajectory artifact used as a
  test oracle today. The new solver will not reproduce the 58-pass Jacobi trajectory; update the
  oracle to compare *final* semantics, or retain the legacy Jacobi solver ONLY behind a
  `#[cfg(test)]`/env trace-compat flag, clearly dead on the production path.
- **Engine-never-throws.** ⟨rev⟩ No new production panics — a monotone solver terminates
  structurally; use debug-only diagnostics, never a production `assert`/panic, as the convergence
  backstop.
- **Determinism.** All output ordering stays deterministic; interned-ID assignment must reproduce
  the existing `(effect_key, operation_id)` sort at materialization (see ⟨rev⟩ ID-order note).
- **rustfmt per file; never `cargo fmt`. Stage explicitly; never `git add -A`. Never push/merge to
  master without explicit request.** (CLAUDE.md.)

---

## Background — the measured root cause

`compute_summaries` (`src/engine/l4/summary_runner.rs`) computes per-routine interprocedural
DB-effect summaries over the call graph's Tarjan SCC condensation, in reverse-topological order;
each recursive SCC runs a Jacobi fixed point (`run_one_scc`, ~line 1067).

On the real BC Base Application (8020 files), the dominant SCC measured (`logs/trace-probe.json`):

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

The dataset is tiny; the cost is representation, not volume:

1. **String-keyed domain.** `RoutineSummary.db_effects: Vec<DbEffect>`, `DbEffect.effect_key: String`
   = `"{op}|{table_id}|{operation_id}|{temp_frag}"` (`effect_lattice.rs:122`). `compose_routine`
   (`summary_runner.rs:351`) builds a `BTreeMap<String, DbEffect>` per member per pass, unions every
   callee's ~8,900 effects (O(log U) string compares), sorts by string, then `summary_change_key`
   (`summary.rs:670`) re-formats `"{effect_key}:{via}"` over the whole set. 58 passes ≈ 3×10¹¹
   char-ops.
2. **Provenance + roles in the fixpoint change-key.** `SummaryChangeKey` (`summary.rs:657`) carries
   `db_effects` (`"{effect_key}:{via}"`), `uncertainties`, `has_unresolved_calls`, AND
   `parameter_roles` in one key, so any sub-domain change re-dirties the expensive effect
   composition. ⟨rev⟩ **Do NOT claim `via` churn causes the 58 passes** — recomputing a caller
   *replaces* the callee `via` with the edge rank, so a callee-only `via` change does not re-propagate
   as provenance churn. The pass count's cause (presence-settle vs role-settle vs via-settle) is
   currently un-instrumented; Phase 1 adds that instrumentation before attributing it.
3. **Memory.** Jacobi keeps two `HashMap<String, RoutineSummary>` generations (`snapshot` +
   `next_pass`, `summary_runner.rs:1236,1240`) plus `key_cache` (~7.1M formatted strings). 7.1M
   `DbEffect` structs × 5 heap `String`s each ≈ 2–3 GB/generation → ~40 GB peak.

This is the classic **bitvector dataflow framework** implemented with strings + provenance.

---

## The corrected algorithm (the crux)

The db-effect transfer is `X_v = B_v ∪ ⋃_{e=(v,w)} T_e(X_w)` where `T_e` (`compose_routine:404-439`):

- `Known(_)` / `Unknown` effects transfer by **identity** (key unchanged).
- `ParameterDependent(i)` effects are **substituted per callsite** (`substitute_pd_temp_state`,
  `summary_runner.rs:691`): callee-frame index `i` → `Known(t)`/`Known(f)`/`Unknown`/`PD(j)` in the
  caller frame, and the result is **re-keyed** (`effect_key_of`, `summary_runner.rs:409`).

### Step 0 — build *effective* SCCs ⟨rev⟩ (fixed leaves + missing routines)

`run_one_scc` excludes fixed `leaf_summaries` from recomputation (`summary_runner.rs:1088,1161,1176,
1210,1246`) AND silently skips members absent from `routines_by_id` (`None => continue`,
`summary_runner.rs:1258`). Removing those from the equation graph can **split one Tarjan SCC into
several DAG components**, so you must, in order:
1. Remove fixed leaves and missing routines from the equation graph.
2. **Re-run SCC decomposition on the induced non-recomputed graph** (grouping the remainder by the
   original Tarjan SCC would over-union distinct components).
3. Treat edges from a recomputed caller to a fixed leaf / already-settled successor as
   **external-summary inputs**.
4. Ignore a fixed leaf's *outgoing* edges as dependencies (the leaf is not recomputed).
5. Process the resulting effective components in reverse-topological order.
6. Never assign the common union or reconstructed `via` back to a fixed leaf.

### Step A — solve PD as product-graph reachability ⟨rev⟩ (no Jacobi)

The PD product state MUST carry the effect identity — tokens from different originating operations
must not merge. State node:

```
(base_effect_id, routine_id, PD(parameter_index))     base_effect_id ≡ (op, table_id, operation_id)
```

The per-edge substitution is a *fixed function of the binding*, so `caller-set = ⋃ subst_edge(callee-set)`
is **distributive** → solve by a semi-naive worklist (propagate only newly discovered states). Seeds
⟨rev⟩ = each member's base PD effects **∪** the edge-substituted images of every external-callee
(settled-successor / fixed-leaf) PD effect at that member's out-edges. A transition
`PD(i) → Known/Unknown` **emits `(base_effect_id, Known/Unknown)` into the effective component's
terminal gen-set and stops** (terminal states are identity-transferred thereafter; the closed-form
union handles them). A transition `PD(i) → PD(j)` inserts a new product node. **Bound the tag
universe by the PD indices actually appearing in bases/bindings, not by declared param count** (`≤
#params+3` is per-base-effect and malformed/recovered input can carry arbitrary `u32` indices).

### Step B — closed-form terminal union per effective SCC ⟨rev⟩

For each effective SCC:

```
C =  ⋃ member terminal (Known/Unknown) base effects
   ∪ ⋃ ACTUAL-outgoing-edge successor terminal sets     (identity facts only)
   ∪ ⋃ terminal emissions from Step A
```

(Qualified: **actual outgoing edges**, terminal identity facts only, edge-substituted PD inputs — not
a blanket "all successor summaries".) Represent `C` as an interned bitset; one union, zero iteration.

### Step C — per-member result

```
effects[v] = C ∪ PD[v]          for every recomputed member v of the effective SCC
```

### Step D — reconstruct `via` in one post-pass ⟨rev⟩

The fold *replaces* callee `via` with `via_for_edge_kind(edge.kind)` (`summary_runner.rs:394,413,421`)
— via never propagates transitively. So:

```
via(m, k) = max(  base_via(m,k) if k ∈ base(m)      [direct today],
                  via_for_edge_kind(e) for every ACTUAL edge e=(m,c) and callee fact k'∈set(c) with T_e(k')=k )
```

Caveats: leave fixed-leaf summaries untouched; init recomputed-member ranks from base effects; apply
each *actual* outgoing edge's rank after transforming its final callee set; resolve collisions by rank
max; and **preserve the legacy collision-winner semantics for non-key metadata** (see payload note).
`merge_via` keeps the FIRST arg on equal rank and any non-canonical via string ranks 0 alongside
`inherited` (`effect_lattice.rs:147`) — add a canonicalization/invariant check so max-by-rank picks a
defined representative. Compute compact rank data **once** (a `u8`/membership or rank bitplanes), not
per-detector-query.

### ⟨rev⟩ DbEffect payload beyond the key

`DbEffect` carries `record_variable_id: Option<String>` (`summary.rs:62`) which `effect_key_of`
**excludes**. The legacy fold is order-dependent: base insertion is last-write-wins; an inherited
collision keeps the *existing* payload and only raises `via`
(`DbEffect { via: merged_via, ..existing.clone() }`, `summary_runner.rs:429`). Two-part resolution,
BOTH required:
1. **Proof it is out of the output contract:** the stable projection `PDbEffect` (`summary.rs:211`)
   drops `record_variable_id`, and every l5 occurrence is a `None`-constructor, never a reader — so
   goldens/fingerprints/detectors cannot observe it. (Verify by a repo sweep in the plan.)
2. **Differential over the complete internal `DbEffect`** anyway (not just projected key/via), so a
   future reader or malformed-input collision cannot silently diverge.

### Side-facts (uncertainties + `has_unresolved_calls`) ⟨rev⟩

`walk_param` (the roles walker) does NOT consume DB effects, uncertainties, or `has_unresolved_calls`.
These currently accrue *inside* `compose_routine`'s effect loop (`summary_runner.rs:442-501`) and sit
in the change-key. Split them into their **own monotone solvers**, not the roles fixpoint and not the
effect union: `has_unresolved_calls` is boolean-OR reachability; uncertainties are a per-SCC set union
that is monotone and independent BUT **not a plain universal union** — callsite-local kinds
(`member-not-found`/`external-target`/`ambiguous-overload`/`interface-open-world`) are filtered at
`summary_runner.rs:443`, so the union carries only the non-filtered, edge-independent kinds plus each
member's own callsite-local ones. Closed-form per effective SCC once the filter is respected.

---

## Part 1 — db_effects redesign (STRICT PARITY)

**Files:** `summary_runner.rs`, `summary.rs`, `effect_lattice.rs`; new `effect_universe.rs`
(interner) and `db_effect_solver.rs` (Steps 0–D). Detector-facing: `src/engine/l5/full_summary.rs`,
`detector_context.rs`, and every `summary.db_effects` reader.

### Internal sequencing ⟨rev⟩ (do NOT combine solver + API migration atomically)

1. Implement the new solver (Steps 0–D + side-facts) **behind the existing materialization adapter**
   — it still produces today's `Vec<DbEffect>`.
2. Differential-test complete old-vs-new internal summaries to green (see Correctness spine).
3. Freeze the universe/ID model and **prove ordering** (below).
4. Migrate retained storage to compact IDs/ranks.
5. Migrate detector readers to borrowed views / direct queries.
6. Remove full materialization **only after all consumers are enumerated**. Transient compat
   materialization is fine during testing; not in production.

### Interning ⟨rev⟩ (universe ownership + ID order)

- **Workspace-scoped, frozen universe** — NOT per-SCC (a per-SCC interner needs remapping at every
  SCC edge and is meaningless for Salsa-cached / cross-SCC summaries). Compact `u32` IDs are only
  valid relative to their universe generation; record that ownership explicitly.
- **Resolve the lazy-vs-lexical-ID tension** (they conflict: PD substitution creates variants
  mid-solve): discover variants with temporary handles, then **freeze + sort the universe and remap
  once** before building final bitsets — OR assign arbitrary deterministic internal IDs and keep an
  ID→sorted-key permutation applied only at materialization. Pick ONE in the plan; do not ship the
  "or".
- Intern the structured identity `(op_id, table_id, operation_id)` + temp-state tag; only the
  temp-state fragment changes under PD substitution.

### Compact detector-facing representation

Retaining `Vec<DbEffect>` (5 `String`s each) for 7.1M entries is ~1 GiB+ regardless of how it's
built. For sub-GB: store `CompactEffect { effect_id: u32, temp_state, via_rank: u8 }` +
`record_variable_id` handling per the payload note + the global interner; expose a borrowed
`DbEffectView` (lazy `effect_key`); materialize full `DbEffect`s only for *reported* effects.

**Expected:** the 797-SCC effect fixpoint 729s → sub-second; peak RSS 40 GB → sub-GB.

**Phase-1 exit criteria** ⟨rev⟩: (a) complete-internal-summary differential green on fixtures + CDO;
(b) a **roles-only re-measure** on the 8020 corpus (the `walk_param` walker, `summary_runner.rs:625`,
is the real per-pass CPU once strings are gone — confirm Phase 1 actually moved the needle and isn't
bottlenecked on roles still riding a shared loop); (c) pass-attribution instrumentation showing
presence-settle vs role-settle vs via-settle rounds.

---

## Part 2 — parameter_roles monotonization + cap (SEPARATELY APPROVED SOUNDNESS CHANGE)

**Files:** `cfg_walker.rs` (`apply_call` ~1168, exit-effect block ~1295-1332, `join_dirty:105`,
`LOOP_BOUND:344`), `summary_runner.rs` (`MAX_FIXED_POINT_ITERATIONS:36`), `summary.rs` role domains.

`apply_call` is **genuinely non-monotone** — but ⟨rev⟩ the fix is NOT the lattice edits I first
proposed:

- **Do NOT reorder `Dirty`.** `join_dirty` (`cfg_walker.rs:105`) is already a consistent join
  semilattice (`Pristine⊔Persisted=Unknown`, `_⊔DirtyV=DirtyV`). A linear
  `Pristine<Persisted<DirtyV<Unknown` chain would change branch semantics and output.
- **Do NOT join `current_loaded_fields`.** `out.current_loaded_fields = field_list_to_loaded(…)`
  (`cfg_walker.rs:1302`) is a *sequential strong update* (a definite callee load); joining with the
  pre-call value loses precision and is not byte-identical.

The real non-monotonicity is the transfer's **strong updates keyed on callee-summary values that
themselves grow across the fixpoint** (the equality-guarded `Dirty` transitions at
`cfg_walker.rs:1313-1331`; `Loaded`'s flat domain over `EffectPresence`'s linear order). Correct
approach:

1. Solve the monotone c1b may-facts first (`persists_current_record`, `validates_param`,
   `copies_into_param`, … — pure joins) and **freeze** them.
2. Test whether the remaining path-summary transfer is monotone under the existing flat domains.
3. If not, represent procedures as **monotone abstract transformers / finite input→output
   relations** (compose transformers at calls, union at branches; derive the tri-state presentation
   only after convergence).
4. Derive the existing `RecordRoleSummary` presentation *after* convergence.
5. Add exhaustive **monotonicity property tests** for `apply_call`, the CFG joins, and bounded-loop
   behavior.

**Cap:** after a proven-monotone reformulation the solver terminates structurally; the bound is
"successful worklist updates ≤ global product-lattice height" (⟨rev⟩ NOT "lattice-height ×
SCC-diameter", which is not a general bound). Replace the 1000-cap with **debug-only diagnostics**,
not a production panic (engine-never-throws). `LOOP_BOUND=3` (`cfg_walker.rs:344`) is the walker's
SEPARATE local precision cap — removing the outer 1000 cap does not touch it; address separately.

**Interim (if Phase 2 lands after Phase 1):** keep the 1000-cap, but ⟨rev⟩ note that *early* cycle
detection can change capped output — the legacy solver emits the state at iteration 1000 plus a
`fixpoint-capped` uncertainty; stopping at the first repeated state returns a different cycle phase and
breaks byte-identity. For parity either detect the *full* repeated solver state and jump to the state
at `iteration 1000 mod cycle_length`, or keep current behavior and use detection for diagnostics only.
A `t == t-2` check catches only 2-cycles; general repetition needs stored state hashes.

---

## Part 3 — capability_cones (pending profiling confirm)

**Files:** `src/engine/l4/capability_cone.rs`.

Same *shape* as db_effects: `ConeFacts = BTreeMap<String, ConeFactEntry>` (`capability_cone.rs:1338`),
`merge_cone` keeps min-hop-distance + canonical-rep witness (`:1342`), `retag` replaces `via` per
first-hop edge (`:1360`), ~50s. ⟨rev⟩ **Neither reviewer independently confirmed these line-cites** —
treat "same disease" as pending a short profiling + source confirm before planning. Note the cone
closure is a **shortest-path (min-distance) tropical closure**, NOT a plain reachability union, so the
condensation carries an interned key → min-dist map; design accordingly, reconstruct the witness
(min-dist rep + via) on demand.

---

## Part 4 — substrate cache (optional gravy)

After Parts 1–3 the run drops ~25 min → ~2–3 min with NO cache, so this is optional. When wanted:
content-address the L4 substrate (workspace-scoped interned universe + per-routine bitsets + rank
data + roles + cones) by a hash of the app's source set; reload instead of recompute when unchanged.
`run_one_scc`'s doc names the incremental seam (each SCC summary depends only on `members +
successor-summaries + inputs`). ⟨rev⟩ The compact `u32` IDs are only meaningful within their universe
generation — a cache entry must carry/verify its universe identity, or remap on load. Keyed by BC
base-app version + content hash (the original instinct, now aimed at the substrate, not the parse).

---

## Freebie — redundant `fresh_coverage`

`run_analyze_with_exit` (`src/engine/gate/run.rs:206`) runs `program::resolve::full::fresh_coverage`
— a *second, full* program resolve (58s on 8020) only for coverage stats. **Investigate** reusing the
L3 assembly already built for the detector path, or skipping when coverage is not consumed (the JSON
envelope + preflight warning DO surface coverage, so this is a reuse/derive, not an unconditional
skip). Independent small task, gated on confirming output is unaffected.

---

## Correctness spine

- **Differential-gate the new DB solver vs the old over the COMPLETE internal summary** ⟨rev⟩ (all
  `DbEffect` fields incl. `record_variable_id`, all uncertainties, `has_unresolved_calls`,
  `parameter_roles`) on generated SCCs covering every flagged shape: PD-index permutations,
  PD→Known, PD→Unknown, multiple callsites to the same callee, edge-kind `via` collisions, external
  successor PD seeds, self-loops, **fixed leaves inside Tarjan SCCs**, and **missing routines inside a
  Tarjan SCC**.
- **Final-summary byte-identity** on the fixture corpus + CDO north-star metrics unchanged
  (`scripts/cdo-gate`) + all detector goldens (`scripts/check-goldens`).
- **Trace oracle** updated to compare final semantics (or legacy Jacobi retained behind a test-only
  flag).
- **Pass-attribution instrumentation** ⟨rev⟩ (presence-settle vs role-settle vs via-settle) added
  before crediting any lever with the 58-pass depth.
- **Perf gate** so `compute_summaries` on the synthetic corpus can't regress an order of magnitude
  (mirrors `tests/perf_bounds.rs`).

---

## Risks & open questions

1. **Roles precision drift (Part 2)** — separately approved, explicitly rebaselined; never silent.
2. **Interned-ID determinism** — final ID assignment must reproduce the existing sort; test explicitly.
3. **Detector API change (Part 1 step 5)** — the largest blast radius; enumerate every
   `summary.db_effects` reader before removing full materialization.
4. **Effective-SCC re-decomposition (Step 0)** — must actually re-run Tarjan on the induced graph,
   not reuse original SCC grouping.
5. **PD tag-universe bound** — driven by indices in bases/bindings, robust to malformed `u32`.
6. **Cone closure is shortest-path, not reachability (Part 3)** — pending profiling confirm.
7. **Cap-parity if Phase 2 slips** — cycle detection must not change capped output.

---

## Sequencing (phased plan)

1. **Phase 1 — db_effects** (Steps 0–D + side-facts, behind the materialization adapter; then the
   compact-API migration as its own internal sub-sequence). STRICT parity. Drive to green + perf win
   before Phase 2. Independent of the cap.
2. **Phase 2 — parameter_roles** monotonization (stratify → test → transformer/relation) + cap →
   debug diagnostic. Separately approved soundness change with explicit rebaseline.
3. **Phase 3 — capability_cones** (after profiling confirm).
4. **Phase 4 — substrate cache** (optional; decide after Phases 1–3 re-measure).
5. **Freebie — fresh_coverage** reuse/skip (independent; any time).

Each phase is differential-gated and ends with an 8020 re-measure + the CDO north-star ratchets.
