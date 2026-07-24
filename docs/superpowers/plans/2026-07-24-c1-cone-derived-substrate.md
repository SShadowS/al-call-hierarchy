# C1 — capability-cone derived substrate (plan)

Branch `feat/l4-summary-redesign`, base `a0cd348`. Companion/tail of the
l4-dbeffect-store arc (db_effects 24 GB → 0.47 GB, solver 517s → 11.3s).

**Goal.** `context.capability_cones` ~10.9 GB → sub-GB, **output byte-identical**.
Replace the per-routine `FullRoutineSummary.capability_facts_inherited:
Vec<CapabilityFact>` (~27M `retag` struct-clones) with a compact per-routine
derived substrate folded during the existing SCC cone walk, and stop
materializing the raw Vec on the `analyze` path.

**Design:** `.superpowers/sdd/C1-cone-redesign-design.md` (read §1-§5 for the
consumer inventory, the `CapabilityFact` shape, and the parity traps).
**This plan supersedes the design's §4 and §6 where they conflict** — see
Revisions below.

---

## Revisions to the design (vetting outcome — these GOVERN)

Controller source-vetting + pi panel (gpt-5.6-sol, fable-5; both read the real
source; reviews at `scratchpad/pi-{sol,fable}-c1.md`). Both CONFIRM the design's
crux — the dedup-trap byte-parity argument (§5.1) and the 15-derived/1-raw
consumer inventory (§1) — and both REJECT its §4 gating mechanism. Amendments:

- **R1 — the gate is a MODE at the cone seam, not a substrate bit alone.** The
  raw Vec is allocated *inside* `compose_inherited_cones`
  (`capability_cone.rs:1686-1691`) and returned through `compose_cone_over_graph`;
  a substrate bit checked afterwards in `build_detector_context` can only
  *discard* the allocation, never prevent it — zero memory win. Thread an explicit
  output mode through `compose_cone_over_graph` / `compose_inherited_cones`.
- **R2 — `RAW_INHERITED_FACTS` must live OUTSIDE `substrate::ALL`.** `ALL` is an
  explicit OR list (`registry.rs:60`) that non-registry callers pass verbatim
  (`gate/events.rs:391,532`, `digest_cli.rs:403`, `prove.rs:834`,
  `policy/pipeline.rs:246`). Putting the new bit in `ALL` re-materializes the
  10.9 GB on those paths. The `policy` call site passes `ALL |
  RAW_INHERITED_FACTS` explicitly. (Note: the four `requires: substrate::ALL`
  registrations in `registry.rs:678,683,717,722` are `#[cfg(test)]` fixtures —
  production detectors in `detectors/mod.rs` declare explicit bits, so the
  detector `demanded` union never contains `ALL`.)
- **R3 — fold sources are asymmetric and MUST be honoured verbatim.** Three
  distinct sources, each with its own dedup discipline:
  - self half → the routine's **raw, un-deduped** `direct_full` Vec
    (`detector_context.rs:283-290`), one fold per fact;
  - inherited, singleton path → the **key-winner reps** in `best.values()`
    (`capability_cone.rs:1449-1454`);
  - inherited, BFS path → the **`seen`-deduped reps**, where sibling-member
    facts come from the **key-deduped `direct` map**
    (`capability_cone.rs:1495-1500`), not the raw Vec.
  Folding raw reachable facts instead of key-winners flips temp-vs-physical
  whenever a temp fact wins a key (design §5.1). Fixture-pin all three.
- **R4 — representation: pooled sorted vectors, interner on the context.** Not
  four `BTreeSet`/`BTreeMap` per routine (~404k tree allocations, ~150-300 MB).
  Fold into reusable scratch sets, freeze each routine into sorted
  `Vec<u32>` / `Vec<(u32,u8)>` slices pooled into shared arrays (the
  `effect_store.rs` CSR playbook), per-routine rows holding `Range<u32>` +
  flags. Estimate ~30-80 MB. The `ResInterner` is parked on
  `DetectorContext` next to `db_effect_bundle` — **never** an `Arc` inside the
  per-routine row.
- **R5 — the S1 parity gate is a per-routine in-process oracle, not just
  goldens.** Findings goldens only observe routines that produce findings. While
  both representations exist, assert per routine that derived == raw-computed.
  Exact assertion list in Task 1.
- **R6 — kill the stale raw-read API after S2.** Once analyze no longer
  materializes, `FullRoutineSummary::reachable` / `reachable_iter` /
  `capability_query::find_capabilities` / `has_capability` still compile and
  silently return direct-only. Remove, privatize to the raw path, or make the
  ungated access fail loudly.
- **R7 — d44's op order is LEXICAL: `delete, insert, modify`.** `op_union` is a
  `BTreeSet<&str>` (`d44.rs:104`); the op-mask decoder must emit that order, not
  `insert, modify, delete`. (Design §5.4 states the rule but lists mask order.)

---

## Global Constraints (bind every task)

1. **Output byte-identity is the contract.** `scripts/check-goldens` (all five
   families: r4, l2_ir, cli, l3, differential) must pass with **zero golden files
   touched**, and a DO-workspace `alsem analyze` run must be byte-identical.
   **Never blind-regen a golden** — a moved golden is a defect to root-cause.
2. The l4 differential (`tests/l4_summary_differential.rs`, 17/17 vs the frozen
   baseline) must stay green — C1 must not perturb the db-effect store.
3. `cargo build --bins` and `cargo test --lib --no-run` **warning-free**;
   `cargo clippy --all-targets --all-features` **zero warnings**.
4. Format touched files with `rustfmt <file>` — **never `cargo fmt`**.
5. Stage only intended paths — **never `git add -A`**.
6. Do not touch the db-effect store (`effect_store.rs` / `db_effect_solver.rs` /
   `summary_runner.rs`) except to *read* its CSR patterns as a model.
7. The projection / `aldump` / `prove` / `digest` / `policy` paths keep RAW
   inherited facts and their exact `sort_inherited` ordering
   (`capability_cone.rs:1381-1385`) — byte-identical.
8. `CHANGELOG.md` is updated at the arc capstone (Task 4), not per task.

---

## Task 1 — `ConeDerived` substrate + fold + parity oracle (dual-run) — DONE

**Status: DONE** (`fbbb5b4` + review fix wave `c120ac9`). Historical section —
kept in the original present/future tense it was written in, describing the
task AS PLANNED. **The `C1_CONE_PARITY` oracle this task built (`l5::cone_parity`,
below) was RETIRED at Task 3** once the raw Vec stopped being materialized on
the `analyze` path (its raw side would have degraded to direct-only under
`DerivedOnly` and false-alarmed) — see Task 3's own section for the retirement.

**No behaviour change.** Every existing caller keeps receiving the raw Vec; the
derived substrate is computed alongside and proven equal.

### Build

1. **`ResInterner`** — workspace-global, lossless `String → u32` / `u32 → &str`.
   Serial (the cone walk is serial). Interning order is irrelevant to output:
   every query resolves to `String` and sorts by the **resolved string**.
2. **`ConeDerived` row** — per routine, over reachable = *own direct* ∪
   *inherited reps*:
   - `flags: u8` — `TOUCHES_TABLE` (any `resource_kind == "table"`),
     `MAY_COMMIT` (`op == "commit" && resource_kind == "transaction"`),
     `TOUCHES_HTTP` (`resource_kind == "http"`), `TOUCHES_FILE`
     (`resource_kind == "file"`). Those two IO kinds are the complete vocabulary
     (`d48.rs:50-52`).
   - `table_writes_all` — `op ∈ {insert,modify,delete}` on `resource_kind ==
     "table"` with `resource_id` present, **including known-temp**. Backs
     `writes_tables_of` (`capability_query.rs:71-86`).
   - `physical_table_writes` — same, **excluding** `fact_is_known_temp`
     (`capability_query.rs:28-35`), each id carrying a `u8` op-mask
     `{insert=1, modify=2, delete=4}`. Backs `writes_physical_tables_of` and
     d44's per-table op-union.
   - `physical_table_reads` — `op == "read"`, `resource_kind == "table"`,
     `resource_id` present, non-known-temp. Backs d44's read set.
   - `event_publishes` — `op == "publish"`, `resource_kind == "event"`,
     `resource_id` present. Backs `publishes_events_of`.
3. **Pooled storage per R4** — fold into reusable scratch sets, freeze each
   routine into sorted `Vec<u32>` / `Vec<(u32,u8)>` slices in shared pools;
   the per-routine row holds `Range<u32>` + `flags`. Derive whatever
   `FullRoutineSummary` derives (`Debug, Clone, PartialEq`) plus `salsa::Update`
   if the row reaches `ConeResultPub` (`capability_cone.rs:1734-1749` — salsa is
   derive-only since B1; confirm before adding).
4. **`ConeOutput` mode** — `{ DerivedOnly, RawOnly, Both }`, threaded through
   `compose_cone_over_graph` and `compose_inherited_cones`. **In this task every
   caller passes `Both`.** `DerivedOnly` must skip the `retag` clone and the
   `sort_inherited` allocation entirely (not build-then-drop).
5. **The fold** — at the three sites named in R3, replacing/paralleling
   `retag`: `capability_cone.rs:1449-1454` (singleton `best.values()`),
   `:1495-1500` and `:1515-1520` (BFS `seen` reps), plus the routine's own raw
   `direct_full` facts. Zero clones, zero sort: enum-match + set insert.

### The oracle (R5) — RETIRED at Task 3

**This whole mechanism (`l5::cone_parity`, the `C1_CONE_PARITY` env flag, and
the two `assert_cone_parity_if_enabled(...)` call sites in
`detector_context.rs`) was DELETED at Task 3**, once `build_detector_context`
started composing `DerivedOnly` by default: under that mode the oracle's raw
side degrades to direct-only, so every one of its checks would false-alarm.
Its one load-bearing test (`colliding_routine_ids_leave_summary_and_derived_
row_equally_degenerate`) was relocated, not lost — see the Task 3 report. The
description below is kept as a historical record of what Task 1 built; it does
not describe anything live in the current tree.

Behind an env flag (mirror `REGEN_TEMP_GOLDENS`'s value-test style, e.g.
`C1_CONE_PARITY=1`), immediately after cone assembly in
`build_detector_context`, assert for **every** routine that the derived
substrate equals the raw-Vec computation:

- `touches_db_of`, `may_commit` (tri-state, incl. the coverage-driven
  No-vs-Unknown arm), `reachable_coverage`
- `writes_tables_of`, `writes_physical_tables_of`, `publishes_events_of`
  (exact `Vec<String>`, order included)
- d44's write map — `BTreeMap<table, BTreeSet<op>>` — computed on the raw side
  with d44's **exact** closures (`d44.rs:80-86`), and d44's read set
  (`d44.rs:189-200`)
- `touches_io_of` vs `routine_touches_external_io` (`d48.rs:124-132`)

A mismatch panics with the routine id and the diverging predicate. Run it over
every golden corpus and over CDO (`CDO_WS`).

### Fixtures (unit tests, all three fold-source rules pinned)

- **temp/physical dedup trap** — two callees writing table T, one known-temp one
  physical, identical `(op, resource_kind, resource_id, confidence)`, reachable
  from a common ancestor: `writes_physical_tables_of` must be unchanged from the
  raw path (the *winner* decides, not "any physical").
- **BFS sibling** — a recursive SCC where a sibling member's direct facts reach
  the subject: the fold must take them from the key-deduped `direct` map, and the
  subject's own from the raw `direct_full` Vec.
- **d44 op order** — a routine writing one table with all three ops; the mask
  decoder emits `delete, insert, modify` (R7).
- Coverage tri-state: absent fact + `inherited_status == "complete"` → `No`;
  absent + `partial` → `Unknown`.

### Gate (as run at the time — the oracle no longer exists, see above)

`C1_CONE_PARITY=1` over the golden corpora + CDO green; `scripts/check-goldens`
zero files touched; differential 17/17; build/clippy/test-compile warning-free.

---

## Task 2 — migrate every derived consumer (still dual-run) — DONE

**Status: DONE** (`4232a5e` + review fix wave `fc9e509` + residual pins
`c45586a`). Historical section, same caveat as Task 1: the oracle referenced
below ("Oracle still green") was retired at Task 3.

The raw Vec is still built (`Both`), but **no detector reads it**.

1. Rewrite the `capability_query.rs` helpers over `ConeDerived` + `coverage`
   + `capability_facts_direct`, resolving ids through the ctx-parked
   `ResInterner`. Signatures may gain a `&ResInterner` / `&DetectorContext`
   parameter — pick the seam with least call-site churn and apply it uniformly.
   Add `physical_table_reads_of`, `physical_table_write_ops_of`, `touches_io_of`.
2. Migrate all consumers: `d1` (`:697,845,1099`), `d1_graph` (`:208,258` — the
   design's §1 table omits this file; same `touches_db_of` predicate), `d2`
   (`:147,403,435`), `d8` (`:37,125`), `d34` (`:115`), `d35` (`:52`), `d43`
   (`:186,427,444`), `d44` (`:80,182,195` → the new op-mask/read-set queries),
   `d45` (`:64,97,65,91`), `d48` (`:130` → `touches_io_of`; its **direct** reads
   at `:80-117` stay as-is), `d50` (`:70`), `transaction_spans`
   (`:136,139,142`), `event_flow::compute_fanout` (`:350`, coverage only).
   `event_flow::publisher_branch_facts` (`:903-906`) reads **direct** facts —
   unchanged.
3. Migrate `build_detector_context_cross_app` (`:687-712`) the same way — it
   additionally `clone()`s each routine's inherited Vec instead of moving it;
   the fold removes that cost too.
4. `test_support::summary(id, direct, inherited, cov)` (`test_support.rs:191-203`)
   folds its `inherited` argument into a `ConeDerived` internally, so the
   ~dozens of `d1_*` / detector fixtures keep their current call shape.

### Gate

Oracle still green; `scripts/check-goldens` zero files touched; a DO-workspace
`alsem analyze` run byte-identical to the pre-task run (capture both, `cmp`);
differential 17/17; build/clippy warning-free.

---

## Task 3 — stop materializing on analyze (the memory win) — DONE

**Status: DONE** (`39e861e` + `42b48a6`; review fix wave: this commit). Once
`build_detector_context` composes `DerivedOnly` by default, the Task 1 parity
oracle's raw side degrades to direct-only under that mode, so every one of its
checks would false-alarm — **this task deleted `src/engine/l5/cone_parity.rs`
in full** (the module, its `pub mod cone_parity;` re-export, and both
`assert_cone_parity_if_enabled(...)` call sites in `detector_context.rs`), per
R6's "kill the stale raw-read API" mandate. Its one load-bearing test
(`colliding_routine_ids_leave_summary_and_derived_row_equally_degenerate`) was
relocated to `detector_context.rs`'s own `mod tests`, not lost. **The
correctness net from here on is goldens + the DO `analyze`/`policy check`
byte-identity runs only** — see the Task 3 report §6.3. The review's finding
I-1 closed the one remaining gap this left (no in-repo fixture pinned the
`policy` path's inherited-fact *content*): `tests/fixtures/cli-c-policy/
ws-policy-commit-inherited-trigger` now does.

1. Add `substrate::RAW_INHERITED_FACTS`, **outside** `ALL` (R2), documented as
   policy-only. Amend `ALL`'s doc comment to say so.
2. `build_detector_context` / `_cross_app` pass `ConeOutput::DerivedOnly` unless
   the bit is set, `Both` when it is. `policy/pipeline.rs:246` passes
   `substrate::ALL | substrate::RAW_INHERITED_FACTS`.
3. Classify **every** other `build_detector_context(..., ALL)` caller explicitly
   in the diff (`gate/events.rs:391,532`, `digest_cli.rs:403`, `prove.rs:834`,
   plus test callers): each is derived-only unless proven otherwise by a source
   read — state the evidence per caller.
4. The direct cone callers (`capability_cone.rs:1803` `project_r3a3`, `:2582`,
   `:2850`, `:3003`) keep `RawOnly`/`Both` — projection bytes and
   `sort_inherited` order unchanged (Global Constraint 7).
5. `capability_facts_inherited` becomes absent-by-default (an `Option`/side
   field or moved onto the raw path's own type). Apply R6: remove or privatize
   `reachable` / `reachable_iter` / `find_capabilities` / `has_capability` so no
   caller can silently read a direct-only view. `has_capability` has zero
   production call sites — delete it.
6. `policy_engine::select_facts` must fail loudly (not silently return empty) if
   it is handed a context built without the bit.

### Gate

`scripts/check-goldens` zero files touched; DO run byte-identical; differential
17/17; a `policy`-subcommand run byte-identical to pre-task; then the **8020 RSS
measure** — `context.capability_cones` `rss_delta` ~10 GB → sub-GB, whole-process
peak reported. (Controller runs the 8020 probe.)

---

## Task 4 — cone-lifetime fixes + `CapabilityFact` struct shrink + capstone

**Scope expanded on measured evidence.** After Task 3 the cone span still held
2,151 MB. A deterministic byte census (`C1_CONE_CENSUS=1`, report at
`.superpowers/sdd/c1-residual-census.md`) attributed it and **falsified both
standing hypotheses**: the structures retained after the cone build total only
157.74 MB, of which `capability_facts_direct` — the target of the original Task 4
below — is just **71.56 MB**, and `CoverageRecord.unknown_targets` is **0 bytes,
0 entries** on this corpus. Items 0a/0b below are the real levers and come first.

### 0a. Free a root SCC's own cone (~1,599 MB — 74% of the residual)

`compose_inherited_cones` never frees `fact_cones[i]` / `cov_cones[i]` for an SCC
that is a call-graph **root**. `remaining_uses[y]` (`capability_cone.rs:1710-1715`)
counts how many SCCs have `y` as a *successor* — i.e. how many predecessors will
consume `y`'s cone — and the only removal site (`:1800-1806`) sits inside
`for y in succ_ids`, so removal fires only when some *predecessor* finishes. A root
has no predecessors: its counter starts at 0, it never appears as any `y`, and its
fully-populated cone survives until the function returns. On 8020 that is **17,864
root SCCs** holding their cones for the whole walk.

Fix: after emitting SCC `i`'s members, free `i`'s own cone when
`remaining_uses[i] == 0`. **Output-neutral by construction** — SCCs are processed
callee-before-caller, so a cone with no remaining consumers can never be read
again. This is the same "materialized cone held too long" pathology Task 3 fixed,
one level up.

### 0b. Drop the two transient direct-facts duplicates (~143 MB)

`direct_in` (79.66 MB) and the walk's own dedup-keyed `direct` map (63.70 MB) are
both dead well before their enclosing scope ends. Drop them at last use.

### 1. `CapabilityFact` struct shrink (~71.56 MB retained, plus transient reps)

1. Convert the five closed-vocabulary `String` fields to `&'static str`:
   `op`, `resource_kind`, `confidence`, `provenance`, `via` — every producer
   already starts from a `&'static str` (`map_table_op`,
   `object_type_to_resource_kind`, `confidence_from_source`,
   `capability_via_for_edge_kind`). **Do not drop** `subject`,
   `resource_arg_source`, `witness_operation_id` — they feed `rep_key`
   (`capability_cone.rs:1317-1328`) and the projection.
   This shrinks the transient cone reps (build-time peak) and the raw
   policy/projection Vecs.
### 2. Capstone docs

1. `CHANGELOG.md` — one entry for the whole C1 change under `Changed`/`Performance`.
2. `docs/2026-07-24-l4-dbeffect-store-8020-remeasure.md` — it still describes the
   db_effects consumer-migration RSS win as *deferred*; that win LANDED (24 GB →
   0.47 GB at B1). Correct it, and add the C1 cone numbers.

### Gate

`scripts/check-goldens` zero files touched; differential 17/17; DO `analyze` AND DO
`policy check` both byte-identical; build/clippy warning-free. The controller
re-measures 8020 — do not attempt it.
