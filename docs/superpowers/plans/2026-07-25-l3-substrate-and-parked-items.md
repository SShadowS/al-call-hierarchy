# L3 substrate + the two parked items (plan)

Branch `feat/l3-substrate-and-parked-items`, base `e5cb27f` (master).

Three items, scoped read-only before planning. **Every number below is measured**, and
each scoping report records how:

- `.superpowers/sdd/scope-routine-id-collision.md`
- `.superpowers/sdd/scope-l3-substrate.md`
- `.superpowers/sdd/scope-reverse-index-consumer.md`

---

## What the scoping changed about the premise

1. **The routine-id collision is a live production defect, not a parked nicety.**
   ~1 in 5 routines: DO 1,157 of 4,842 **erased** (23.9%), 8020 16,906 of 100,941
   (16.7%), largest group 100 routines on one id. `cone_derived.rs:323-324` calls this
   "a handful of routines per workspace" — off by three orders of magnitude, and that
   wrong number is why it was parked as small.
2. **Sub-GB is not reachable, and L3 is not the blocker.** L3's own floor is ~1.2 GB
   (the real program at ~12 KB/routine). A default-preset run peaks at **11,825 MB**,
   set by `d1` (~4.3 GB), not by L3. Realistic outcome: **~11.8 GB → ~5–6 GB**.
3. **`ReverseEffectIndex` is incomplete, not merely unwired.** Its up-query is
   workspace-global — a median of 377 of 3,685 DO routines per table, a haystack. The
   ancestor-scoped query its own header calls mandatory was never implemented.

## Global Constraints (bind every task)

1. **Byte-identical output is the default contract.** `scripts/check-goldens` (r4,
   l2_ir, cli, l3, differential) passes with **zero golden files touched**, and a
   DO-workspace `alsem analyze` run stays byte-identical — *except* in Tasks 3-4, which
   are a deliberate, isolated id-schema change (see their own gates).
   **Never blind-regen a golden.**
2. The l4 differential stays 17/17 vs the frozen baseline; the forensic tag
   `l4-pre-jacobi-deletion` (`f295ef8`) is the only route back to an independent oracle.
3. `cargo check --all-targets` and `cargo clippy --all-targets --all-features` **zero
   warnings**. `cargo build --bins` is NOT sufficient — it skips test targets.
   **`touch` changed files before trusting a clean `cargo check`**; a cached run prints
   only "Finished" and proves nothing.
4. `rustfmt <file>` per file, never `cargo fmt`. Stage only intended paths, never
   `git add -A`.
5. **No caps, sampling, or lossy truncation** — explicitly rejected by the user in a
   prior arc.
6. The IDE's rust-analyzer diagnostics in this repo are stale and have been wrong nine
   times. Trust `cargo check`.
7. Measurement shape must be stated with every number: the 8020 corpus with
   `--detector d8-commit-in-transaction` is the established like-for-like probe; a
   default-preset run is a different, larger number. Never compare across shapes.

---

## Task 1 — (c′) non-destructive cone drain + measure what it uncovers

**Three lines, zero expected golden movement, and it buys a number nothing else can.**

`build_detector_context` drains its cone maps with `remove()`, so the second routine on
a colliding id gets a fully degenerate summary. `build_detector_context_cross_app`
already reads with `get()` and never has the accident — the two builders disagree, which
is itself evidence the drain is an accident.

1. Make the source-only builder non-destructive, matching the cross-app builder.
   `ConeDerivedStore::forget` and its call site must be re-examined: with a
   non-destructive read, the degenerate-summary path may no longer arise. Do not delete
   `forget` blindly — establish from source whether anything still reaches it.
2. **Measure the delta.** Capture DO's full finding set before and after
   (`alsem analyze --format json --deterministic`). The difference **is** the
   false-negative population the drain was hiding — findings that exist on real code
   today and are silently dropped. Report counts by detector.
3. If goldens move, that movement is the same population — triage it finding by finding
   against real AL source, do not bless it. Report before regenerating anything.

**Gate:** goldens + DO diff, both explained finding-by-finding. Expected: zero golden
movement (the corpora are small and may contain no collisions) with a non-zero DO delta.

---

## Task 2 — L-1: `SymbolTable` borrows instead of cloning the workspace

**The single biggest memory lever found, with zero output risk.**

`symbol_table.rs:84-86` does `objects.to_vec()` / `tables.to_vec()` / `routines.to_vec()`
— deep-cloning the entire workspace. Measured cost of one clone: **+1,690 MB** on 8020
(two runs: +1,686.8, +1,702.2). Every public accessor already returns a *reference*
(`&L3Object` / `&L3Table` / `&L3Routine`), so the type never needed ownership.

Convert to borrows with an explicit lifetime. Expect ~1.5 GB removed from every span
after `context.symbols_resolve_calls` and ~1.4 GB off `l3.assemble_resolve`'s peak.

**Gate:** representation-only — goldens must not move and DO must stay byte-identical.
Re-measure 8020 (d8-only shape) and report the span table before/after.

---

## Task 3 — (a2) conditional member discriminator on the INTERNAL id

`L3Routine.enclosing_member` (`l3_workspace.rs:421`, from `al_syntax`
`RoutineDecl.enclosing_member`) is already populated at every `compute_routine_id` call
site and is **measurably sufficient**: DO residual collisions **0**, 8020 residual 15
groups / 19 routines (0.019%, all preproc union-read + XMLport nesting — record both as
the honest residual).

Apply it **conditionally**: routines with no enclosing member keep byte-identical ids,
so encoder vectors and every dep-side id are untouched and the cross-app join stays
symmetric by construction.

**THE TRAP — read before writing code.** The *shape* must not change, only the hash
input. A `#member` suffix or an extra `/`-segment silently breaks
`substitute_stable_ids`' 64-hex scan and `stable_sub_id`'s two-part split; the failure
mode is **every fingerprint in the product moves** — the opposite of the intent. Add a
test that pins the shape independently of the content.

**Gate:** this task MAY move goldens, and every moved file must be explained as
id-shaped. Any movement that is not id-shaped is a defect. Differential stays 17/17.

---

## Task 4 — (a3) the discriminator on the STABLE id + fingerprints

Without this, two sibling `OnAction` findings still hash to one fingerprint and one
baseline entry still suppresses both — demonstrated on DO today
(`b2d1b142f0577a38`, `47500c86760f3f93`).

Measured user cost: **81 of 2,384 DO fingerprints move (3.4%)**; the other 96.6% of a
user's baseline keeps matching. Licensed by this project's standing rule — pre-1.0,
correctness over compatibility, all downstream consumers are ours to change.

Also in this task: correct `cone_derived.rs:323-324`'s "a handful of routines per
workspace" and the `docs/OUTSTANDING.md` entry to the measured numbers.

**Gate:** goldens + baselines regenerated **only** after the diff is triaged and every
moved fingerprint is confirmed id-shaped. Report the exact count moved vs total.

---

## Task 5 — L-3: stop replicating object globals per routine

`variables.global` holds **2,997,353 copies of 53,186 distinct (object,name) pairs — a
56.4× replication**, costing **442.8 MB per workspace copy**; `record_variables.global`
replicates **65.4×**. This is L4 pattern (a) recurring in L3.

Share them (engine-internal API only). Expect ~700 MB.

> **Outcome (2026-07-25), and a correction to the numbers above.** Landed at `234e549`;
> measured **~560 MB** off the 8020 peak, not ~700 MB — the lever was scoped as
> *never-build* but implemented as *build-then-discard* (`ir_variables` still
> materializes every object global per routine and `project_file` filters them back
> out), so the retained cost is gone while the allocations are not. Recorded in
> `docs/OUTSTANDING.md` with a wake condition.
>
> **Every factor in this section is corpus-shaped and must be quoted with its corpus.**
> Re-measured on the real CDO/DO workspace: **16.8×** (record variables **20.2×**), so
> the fractional payload recovered there is ~9% against 8020's ~25%. Artifacts:
> `census-t5fix-{8020,cdo}.txt`, reproduced byte-identically on a fresh run.

**Gate:** goldens + DO byte-identical. Re-measure; L-3 changes the slack term, so any
later sizing must be redone after it.

---

## Task 6 — `ReverseEffectIndex`: complete it, wire it, prove it

1. **Implement the missing ancestor-scoped up-query** (~120 lines, `GraphSccIx`
   reverse-DAG BFS) — the module header already calls it mandatory. Without it the up
   direction returns a haystack.
2. **A thin facade** answering the real question: *is table X touched by a DB action,
   transitively, up or down from here?* — with the `RoutineIx → L3Routine` join for
   user-facing identity (`name`, `object_type`, `source_anchor`), which belongs in the
   facade, not the index.
3. **Demand-driven construction** — must cost `analyze` nothing when unused. The
   `RAW_INHERITED_FACTS` substrate bit (deliberately outside `substrate::ALL`) is the
   working precedent; the scoping report flags a trap in that shape — read §3.2.
4. **A consumer that runs in CI**, so this cannot rot again while the VSCode hover is
   months away.
5. **Correctness evidence**: a **differential against the same answer computed the slow
   way** (walking `SummaryBundle::db_effects` directly). Its 7 self-consistency tests are
   not evidence for something that has never run on real data.
6. **Fix the four defects** the scoping found: the untested dense `PostingList` branch
   (real data hits it — 4 of 61 DO table postings exceed the 256 threshold, largest
   650), `touches_effect`'s doc-vs-implementation asymptotics mismatch, and the
   `"unknown"` `table_id` bucket (1,334 DO routines — the largest posting of all; decide
   and state what it means rather than returning it as a table).

**Gate:** goldens + DO byte-identical (the index is additive and demand-gated); the
differential green; `analyze` cost unchanged when the bit is unset — prove it.

---

## Task 7 — re-measure, capstone docs

1. Re-measure 8020 in **both** shapes (d8-only for like-for-like against this arc's
   history; default-preset for the honest whole-process number) and record both, clearly
   labelled.
2. `CHANGELOG.md` + `docs/OUTSTANDING.md`: the collision item closed with its real
   numbers; L3 levers landed vs deferred; `ReverseEffectIndex` unparked.
3. State the ceiling honestly: sub-GB was **not** reached and why — after L3, `d1`
   (~4.3 GB) and the cone substrate (2.2 GB) own the peak, which is a separate arc.
