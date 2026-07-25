# L4 summary frozen baseline (Task B1 — old-Jacobi retirement)

These files are the **complete-internal-surface** `RoutineSummary` baseline that
`tests/l4_summary_differential.rs` asserts the v2 interned-columnar
`EffectStore` solver against, after the old Jacobi solver was retired (spec
`docs/superpowers/specs/2026-07-22-l4-dbeffect-store-and-retirement-design.md`,
Part B).

## What is frozen

Each `<fixture>.baseline.txt` is the pretty-`Debug` (`{:#?}`) rendering of the
fixture's v2 output `HashMap<String, RoutineSummary>`, with the outer map sorted
by `routine_id` for determinism. This captures **every** `RoutineSummary`
field — `db_effects` (sequence + order, `effect_key`, op/table/operation, temp,
via, `record_variable_id`), `uncertainties`, `has_unresolved_calls`,
`parameter_roles`, `in_recursive_cycle` — i.e. the complete internal surface,
NOT the `stable_summary_fingerprint` (which omits internal fields).

`cdo-whole-program-digest.txt` is the SHA-256 (hex) over the same canonical
`{:#?}` rendering of the CDO whole-program v2 output (the DO source-only
workspace's ~3685-routine population — too large to commit readably, so frozen
as a digest).

## Provenance — captured at parity with the old Jacobi solver

- **Pre-deletion tag (old oracle one `git checkout` away):** `l4-pre-jacobi-deletion`
  (an annotated tag pointing at `f295ef8`, `docs(l4): 8020/CDO re-measure note —
  Part A store redesign`), the HEAD of `feat/l4-summary-redesign` immediately
  before Part B began. Citing the tag rather than the bare SHA keeps this
  instruction reachable across a squash-merge, which would otherwise leave
  `f295ef8` unreferenced by any branch and eligible for GC. `git checkout
  l4-pre-jacobi-deletion -- src/engine/l4/summary_runner.rs` restores the old
  `compute_summaries_with_leaves` for forensic re-differencing.
- The baselines were regenerated from **v2** while the old solver still existed.
  In the same working tree the differential's `cross_check_v2_equals_old_*`
  tests (fixtures) and `cdo_whole_program_v2_parity` (CDO) were **green** — so
  at capture `baseline == old == v2`. Those two v2==old cross-checks were kept
  live through the R3b/aldump-cut steps and deleted only in the final commit
  that deletes the old solver, leaving `v2 == frozen-baseline` as the permanent
  regression anchor.

## Re-freeze log — `cdo-whole-program-digest.txt`

Every regeneration of the CDO digest is recorded here, with the evidence that
root-caused the movement. The frozen baseline's independent oracle (the old
Jacobi solver) was retired, so a bare "regenerated, tests green" entry proves
nothing — see the warning at the end of this file.

### 2026-07-25 — `d3fc4f0e…d42d401` → `d9eac0c7…37def214` (3685 → 4842 routines)

**Cause:** the conditional enclosing-member discriminator on the INTERNAL routine
id (commit `c4c3d03`, `feat/l3-substrate-and-parked-items` task 3). `canonical()`
sorts the summary map by `routine_id` and the map is KEYED by it, so this moves
both the ordering and the population: member triggers that previously collided on
one id now get one entry each.

**Evidence — exact single-variable attribution, not a masked-diff argument.**
Disabling the one behaviour this task added (short-circuiting the conditional
7th-key-part append in `ids::encode_canonical_routine_key`, leaving the rest of
the tree at `c4c3d03`) reproduces the OLD frozen digest `d3fc4f0e…` **byte for
byte**, with the routine count back at 3685. Nothing else in the tree contributes
to the movement — an unrelated regression could not have hidden, because it would
have moved that run too. This is strictly stronger than the "mask the ids and
compare the remainder" method used for the committed goldens, which is available
only when you cannot toggle the schema.

**Characterization** (canonical `{:#?}` renderings dumped with and without the
discriminator, then compared):

| measurement | value |
|---|---|
| routine ids unchanged (member-less: procedures, object-level triggers) | 3231 |
| member-bearing ids replaced | 454 |
| member-discriminated ids minted | 1611 |
| population delta | **+1157** |
| ids violating the `{modelInstanceId}/{64 lowercase hex}` shape | **0** |
| lines retaining a BARE 64-hex run after masking `r0/`-prefixed ids | **0** (both dumps) |
| unchanged-id records byte-identical after masking | 3182 / 3231 |
| unchanged-id records that differ | 49 |
| new struct/variant shapes, new `via` / `op` / uncertainty-`kind` values | **none** — the value domains are identical |

- `+1157` matches, exactly, the independently-measured figure recorded in
  `src/engine/l5/detector_context.rs` ("1 157 of 4 842 DO routines (23.9 %) are
  erased by it"), taken by a different route before this change existed.
- The `454 → 1611` split reconciles with the CHANGELOG's independently-measured
  "DO 262 collision groups → 0": 454 member-bearing ids moved, of which 262
  actually COLLIDED (a member-bearing id with no same-named sibling moves without
  ever having collided); `1611 − 454 = 1157 = 1419 − 262` is consistent with 262
  groups holding 1419 routines and the remaining 192 ids being singletons.
- The 49 differing records change ONLY `DbEffect` membership (net 2277 lines
  removed vs 1471 added — a cone SHRINK, mostly `via: "implicit-trigger"`
  entries) and `has_unresolved_calls`. That is the documented mechanism: a
  colliding id's cone was `(one sibling's direct facts) ∪ (cone over ALL
  siblings' callees)`, so de-colliding removes cross-body splices. No field, no
  shape, and no value domain moved.

**Note on what runs this.** `scripts/cdo-gate` does NOT execute
`--test l4_summary_differential` — it runs `program_resolve_harness` and the
`lsp` CDO tests only — so this digest was never on that runner's path. It is
reached by `scripts/check-goldens` (which gained `--test l4_summary_differential`
in the same fix wave) whenever `CDO_WS` is set, and by the explicit command
below.

## Regenerating (a deliberate, engine-intended re-freeze)

```bash
REGEN_TEMP_GOLDENS=1 cargo test -p al-call-hierarchy --test l4_summary_differential
# CDO digest additionally needs the workspace:
CDO_WS=<path> REGEN_TEMP_GOLDENS=1 cargo test -p al-call-hierarchy --test l4_summary_differential cdo_
```

Regen is a **measurement**, never a blind bless — inspect the diff and
root-cause any movement before committing (the old solver is at the
`l4-pre-jacobi-deletion` tag, `f295ef8`, for re-differencing).
