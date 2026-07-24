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

## Regenerating (a deliberate, engine-intended re-freeze)

```bash
REGEN_TEMP_GOLDENS=1 cargo test -p al-call-hierarchy --test l4_summary_differential
# CDO digest additionally needs the workspace:
CDO_WS=<path> REGEN_TEMP_GOLDENS=1 cargo test -p al-call-hierarchy --test l4_summary_differential cdo_
```

Regen is a **measurement**, never a blind bless — inspect the diff and
root-cause any movement before committing (the old solver is at the
`l4-pre-jacobi-deletion` tag, `f295ef8`, for re-differencing).
