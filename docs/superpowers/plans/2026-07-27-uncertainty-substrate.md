# Uncertainty substrate — hash-cons `ctx.uncertainties_by_node` (plan)

Branch `feat/uncertainty-substrate`, base `6e136e2` (master).

Scoped read-only first. The scoping doc is local scratch (`.superpowers/sdd/` is
gitignored), so every load-bearing figure is restated here and at its use site.

## What the scoping corrected

- **729.1 MiB in 7,428,267 allocations is CORRECT** — reproduced to the byte by an
  independently written census. With ~24 B/allocation of Windows LFH overhead the true
  RSS footprint is **≈ 899 MiB**.
- **The vocabulary is 19,311 distinct values, not 3,073.** The 3,073 figure — which this
  repo currently states in `docs/OUTSTANDING.md` and `d1_cohort.rs`'s `UncertaintyTable`
  doc — is the distinct-note count in the *downstream* winner-cohort evidence, a subset.
  Duplication is still **192×**, so the arc holds; but anything sized against
  "3,073 × 200 B" mis-prices the residual by ~5×. **Correcting those two statements is
  part of this task.**
- **The blast radius is 6 production call sites in 4 files, not ~51** — and
  `to_confidence`'s signature does not change at all. That argument parked this work
  twice; it was wrong.
- **On a real customer workspace (DO) this structure is 2.8 MiB.** This is a BC Base App
  improvement. **Nobody should later read this arc as a customer-workspace win.**

## The measurement trap, stated before anyone measures

The `CORE_SUMMARIES` stage *retains* 1,602.7 MiB but adds only 871.1 MiB to the
**context-build** peak, because the `nocore` run's peak is set inside the cone stage
(~1,071 MiB of cone transient) and `CORE_SUMMARIES` then builds partly into heap that
stage freed. **Measured by context-build peak alone, this arc would appear capped at
~871 MiB and would hit the cone stage as its new floor.** It is not: the win comes off
the **whole-run** peak, which occurs later (inside `d1/assemble_cohort_findings`) with
this structure still alive. Measure the whole-run peak, on a default preset.

## Global Constraints (bind the task)

1. **Byte-identical output.** `scripts/check-goldens` (29 dirs / 9 targets) with **zero**
   golden files touched, plus a DO byte-identity run. **Never blind-regen.**
2. **No caps, sampling, or truncation.**
3. `cargo check --all-targets` + `cargo clippy --all-targets --all-features` zero
   warnings; **`touch` changed files first** or a cached check proves nothing.
4. **A test must pin the USE, not just the helper, and be proven able to fail**
   (CLAUDE.md Testing Philosophy). This session has caught five instances of the inverse,
   plus one fixture that looked like coverage and was none.
5. `rustfmt <file>` per file, never `cargo fmt`; stage only intended paths, never
   `git add -A`.
6. **State the probe shape with every number.** A d1-only probe, a context-build probe and
   a default preset are three different measurements.

---

## Task 1 (the whole arc) — hash-cons the per-node vectors

Hash-cons `ctx.uncertainties_by_node`'s vectors into `Arc<[Uncertainty]>` in **both**
`build_detector_context` variants; change the field type and `walk_evidence`/`visit`'s two
parameter types; fix the six test fixtures. **No detector edits, no output-shape risk.**

Expected: **729.1 → 102.2 MiB live**, 7,428,267 → ~1,052,387 allocations, **≈ −780 MiB of
RSS** on 8020.

Take the two free wins along the way, both identified in scoping:
- `d2.rs:559-563`'s pointless deep clone;
- the clone-then-drain of `core_summaries`.

**Do NOT touch `RoutineSummary.uncertainties`** — the r3a2 golden family lives there and
the retained win does not need it.

Also correct the two wrong vocabulary statements (`docs/OUTSTANDING.md`,
`d1_cohort.rs`'s `UncertaintyTable` doc) to 19,311, and record the DO 2.8 MiB figure
beside the 8020 one.

### Gate
Goldens zero-moved; DO byte-identical; census before/after (live bytes **and**
allocations); a **default-preset whole-run** peak measurement, not a context-build one.

---

## Deliberately NOT in this task

**Step 2 — the further shrink to ~14 MiB live** (interning to ids, per scoping §3's
option C). It is worth ~−105 MiB more, but **it is the part the golden suite currently
cannot see**: add a `d2` `confidence.evidence` golden first, exactly as the d1 arc had to
add `ws-d1-uncertain-path` mid-flight before it could trust its own work. Recorded in
`docs/OUTSTANDING.md` with that precondition.

**`FindingConfidence`-as-ids (−78 MiB)** — the other recorded follow-on. It shares this
table; scoping §6 covers the interaction. Sequence it after Step 2, not before.
