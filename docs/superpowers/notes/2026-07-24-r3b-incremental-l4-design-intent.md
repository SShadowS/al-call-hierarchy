# R3b L4 incremental — preserved design intent (spec Part B.3)

**Status:** the R3b Salsa incrementality experiment (`src/engine/l4/incremental/`
+ `tests/r3/r3b_*`) was DELETED in the old-Jacobi retirement arc (Task B1). It
was Stage-1-through-3 scaffolding built over the OLD `run_one_scc` Jacobi loop
and the OLD per-pass `RawSccTrace`; both are gone. This note preserves the
**reusable test intent** so a future incremental path over the NEW interned
columnar `EffectStore` can be rebuilt without re-deriving the hard parts.

**Forensic recovery:** the full R3b code is one `git show` away — it existed
through commit `64322bc` (`test(l4): freeze complete-internal baseline …`), the
parent of the deletion commit. e.g.
`git show 64322bc:src/engine/l4/incremental/queries.rs`.

**Why it was NOT ported to the new store:** R3b's `scc_summaries` query body
called the OLD `run_one_scc` directly (the Jacobi loop) and its minimality proof
categorized the OLD query families incl. `scc_trace` (the per-pass `RawSccTrace`,
which the closed-form v2 solver has no analogue for). A future incremental path
would be a **redesign** — SCC-condensation-level invalidation over the new store,
not a re-port. The intent below is what carries over.

## Reusable query TOPOLOGY (the crux — externally reviewed at R3b Task 1)

Fine-grained INPUTS (never a monolithic `resolved_model`), so invalidation is
non-vacuous:
- `RoutineUniverse` — `routine_ids : BTreeSet<StableRoutineId>` (the authoritative
  universe the combined graph enumerates).
- `RoutineInput` (per StableRoutineId) — own body facts (`L3Routine`), resolved
  call edges, typed edges, direct db-effects (base summary), direct capability
  facts + coverage, `is_leaf` + retained leaf summary, `body_available`.
- `AppContext` — `app_identity` + the L3-from-scratch shared context the
  combined-graph rebuild needs (objects/tables/event graph/upgraded bindings/field
  index). **L0–L3 stay from-scratch INPUTS to L4** — L4 is the only incremental
  layer.
- `DepStamp` — the dep-artifact stamp (cross-app invalidation key).

TRACKED query data-flow:
`combined_graph` (structural, over `routine_ids`)
→ `scc_condensation` (Tarjan; its output POPULATES the projections, is NOT a
  direct dependency of `scc_summaries`)
→ an **INTERNED `SccKey`** = the interned SORTED member `StableRoutineId` set —
  **an unchanged SCC re-interns to the SAME key** (the identity rule that makes
  early-cutoff work)
→ projection queries `scc_for_routine` / `scc_members` / `scc_successors` (these
  EARLY-CUT for an unchanged SCC)
→ `scc_summaries(scc_key)` — the intra-SCC solve over `scc_members` in SORTED
  order; depends on `scc_members` / `scc_successors` / the members' inputs /
  **successor** `scc_summaries` — NOT the monolithic condensation.
→ `routine_summary(stable_id)` → cone `inherited_facts` + `coverage`.

Salsa handles only INTER-SCC incrementality (an SCC query depends on its
successor SCCs' queries); the intra-SCC solve is a plain function call in the
query body. (R3b used the proven `run_one_scc`; a rebuild calls the new
`EffectStore` solver instead.) Do NOT use Salsa's cycle-recovery API — prefer the
internal-loop pattern.

## SCC-identity rules (carry over verbatim)

- The SCC identity is the **interned sorted member `StableRoutineId` set**. An SCC
  whose membership is unchanged must re-intern to the SAME `SccKey` so its summary
  query backdates (early-cut) after an unrelated edit.
- Member iteration inside the solve is the **SORTED `StableRoutineId` order**
  (asserted at the `scc_summaries` entry), not visitation order — so the fixed
  point is schedule-invariant.

## Fixed-leaf successor handling

- A retained fixed leaf enters as a `RoutineInput` with `is_leaf=true` + its
  retained leaf summary; its SCC is a singleton whose `scc_summaries` returns the
  seeded summary (never recomputed). Successor SCCs read it through the normal
  `successor scc_summaries` dependency. (Mirror the new store's
  `seed_fixed_leaf_rows` path.)

## Deterministic member-order / nondeterminism audit (Rev 2 #4)

The incremental output must be byte-identical to from-scratch REGARDLESS of:
1. **demand order** — demanding summaries/cones in shuffled routine orders must be
   byte-identical (Salsa memoizes; the solve iterates members in canonical sorted
   order).
2. **DB provenance** — the same edit over {fresh / reused-Salsa /
   reused-Salsa-with-different-prior-demand-order} must match.
3. **recursive-SCC fixpoint schedule** — a recursive SCC's settled result is
   order-invariant under any demand schedule.
4. **`RUST_HASH_SEED`** — no internal `HashMap`/`HashSet` iteration order may leak
   into output. Re-run the suite under varied seeds (`RUST_HASH_SEED=0/1/999`).
   One latent leak was fixed at source during R3b
   (`cfg_walker::param_field_accesses` now iterates position keys sorted) — that
   fix SHIPPED and is unaffected by the R3b deletion.

## Minimal-invalidation fixtures (the Stage-3 EXIT GATE)

Categorize the `WillExecute` log BY QUERY FAMILY (use an instrumented DB event
callback):
- **STRUCTURAL** (`combined_graph` / `scc_condensation`) — a topology edit may
  recompute these broadly; NEVER assert them as bounded (accounted separately).
- **PROJECTION** (`scc_members` / `scc_successors` / `scc_is_recursive` /
  `scc_for_routine` / `all_scc_keys` / per-routine edge/leaf projections) — these
  EARLY-CUT (value-equal backdate) for untouched routines/SCCs.
- **SUMMARY** (`scc_summaries` / `routine_summary` / `inherited_facts` /
  `coverage` / `cones`) — **THE BOUNDED SET**: the recomputed SUMMARY set ⊆ the
  reverse dependency cone of the changed inputs.

Curated strict-subset fixture: a **localized NON-topology edit** (change one leaf
routine's direct db-effects) in a graph with UNRELATED SCCs ⇒ only that routine's
SCC + its caller cone recompute — a strict subset of all SCCs (the unrelated SCCs
early-cut). Edit kinds to exercise (Stage 2): add/remove a call edge; change a
routine's direct db-effects / facts / coverage / `body_available`; change
`app_identity` / bump `dep_stamp`; routine ADD / REMOVE / RENAME (== signature
rehash ⇒ `StableRoutineId` re-hash); and NO-OP-at-L4 edits (set same value; add a
dominated edge; cosmetic dep-stamp bump) which MUST early-cut AND stay byte-equal.

## The correctness spine (reusable)

`salsa_incremental(base, edit) == from_scratch(base+edit)` BYTE-FOR-BYTE, where
`from_scratch` is itself pinned to the byte-exact from-scratch baseline (now the
v2 frozen baseline in `tests/l4-summary-baseline/`). Equality then ties the
incremental result to ground truth transitively — no tolerance layer.
