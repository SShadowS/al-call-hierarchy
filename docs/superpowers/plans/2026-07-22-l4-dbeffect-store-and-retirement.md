# L4 db-effect Store Redesign + Old-Solver Retirement Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the (correct) v2 db-effect solver fast (517s→seconds) and compact (~40GB→<300MB) by
replacing per-routine `Vec<DbEffect>` materialization with a shared, interned, columnar `EffectStore`
+ a bidirectional query index; then retire the old Jacobi solver so one path remains.

**Architecture:** Per the design spec `docs/superpowers/specs/2026-07-22-l4-dbeffect-store-and-retirement-design.md`
(rev 4, review-converged by gpt-5.6-sol + claude-fable-5). Prior art: SCC-condensed bitvector dataflow
+ Andersen SCC-collapse (Fähndrich PLDI'98 / Hardekopf-Lin PLDI'07) + Heintze CLA shared-set interning.
Staged lowest-risk-first; each Part-A step is independently landable, **differential-gated against the
retained old solver** (fixtures + generated + CDO), and re-measured on 8020. Retirement is LAST.

**Tech Stack:** Rust, existing `src/engine/l4/`. No new deps (optionally `roaring` for reverse
postings — decide at Step 4 by measurement; a plain bitmap is the fallback).

## Global Constraints

- **THE SPEC IS THE REQUIREMENTS.** Each task cites the spec section it implements; the spec's ⟨rev⟩
  notes are binding (they encode the review corrections). Read the spec section before implementing.
- **Representation-only change ⇒ EXACT output preservation.** The live differential
  (`tests/l4_summary_differential.rs`, currently v2-vs-old) MUST stay green throughout Part A: the
  lazy `DbEffect` view over the new store reproduces byte-identical `Vec<DbEffect>` (effects,
  temp_state, via, record_variable_id, in `(effect_key, operation_id)` order) per routine vs the OLD
  solver. Any divergence is a bug, never a rebaseline.
- **Old solver stays as the differential oracle through all of Part A.** Do NOT touch/retire it until
  Part B.
- **CDO differential leg** (`CDO_WS`-gated, ~729s/40GB on old) is opt-in per step but **MANDATORY
  before Part B** (a step gated on fixtures only is not proven).
- **Goldens byte-identical** (`scripts/check-goldens`, no regen) + **DO byte-identical** at every step.
- **Never reassign EffectIds.** Storage is EffectId-keyed; OUTPUT order is a frozen `key_rank`. No remap.
- **rustfmt per file (never `cargo fmt`). Stage only intended paths (never `git add -A`). Never
  push/merge to master without explicit request.** Package is `al-call-hierarchy` (hyphen).
- Do NOT pipe `scripts/check-goldens` through `tail` (exit code becomes tail's) — redirect + grep.

## Existing interfaces (consume; do not redefine)

```rust
// src/engine/l4/effect_universe.rs (Phase 1)
pub struct EffectId(pub u32);
pub struct EffectIdentity { pub op:String, pub table_id:String, pub operation_id:String, pub temp:TempStateKind }
pub struct EffectUniverse { /* intern, get, identity, len, sorted_order, effect_key(id)->String(format!) */ }

// src/engine/l4/db_effect_solver.rs (Phase 1)
pub fn effective_sccs(scc:&Scc, graph:&CombinedGraph, is_recomputed:&dyn Fn(&str)->bool) -> Vec<Scc>;
pub fn solve_pd_reachability(...) -> (Vec<PdFact>, Vec<TerminalEmission>);
pub fn closed_form_union(...) -> SccPresence;              // builds C as bitset; c.clone() per member TODAY
pub fn reconstruct_via(...) -> HashMap<(String,EffectId),String>;   // String-keyed TODAY
fn attribute_pd_substituted_via(...);                      // the PD-via post-pass
pub fn solve_scc_db_effects(...) -> HashMap<String,(Vec<DbEffect>,Vec<Uncertainty>,bool)>;
fn materialize_member_db_effects(...) -> Vec<DbEffect>;    // sorts by effect_key() format!-per-compare TODAY

// src/engine/l4/summary_runner.rs (Phase 1)
pub fn compute_summaries_v2_with_leaves(...) -> HashMap<String,RoutineSummary>;   // + tuple wrappers compute_summaries_v2*
// (old, oracle only, delete in Part B) compute_summaries / compute_summaries_with_leaves / run_one_scc / compose_routine
// (keep) compose_roles_only / run_one_scc_roles ; solve_side_facts

// src/engine/l4/summary.rs
pub struct DbEffect { effect_key,operation_id,op,table_id,record_variable_id:Option<String>,temp_state:TempState,via:String }
pub fn stable_summary_fingerprint(&PRoutineSummaryCore) -> String;   // encodes each effect as "{proj_effect_key}:{via}"
```

Phase-split instrument (`compute_summaries_v2_phase_split`, emits `db_solver_ms`/`roles_ms`) is live —
use it for per-step re-measure.

---

## Part A — the EffectStore (old solver retained as oracle)

### Task A1 — intern RoutineIx + cache effect_key + integer-cached sort (spec Part A Step 1)

**Files:** Modify `src/engine/l4/effect_universe.rs` (cache `effect_key`), `src/engine/l4/db_effect_solver.rs`
(`materialize_member_db_effects` sort; `PdState`/via map `RoutineIx`), `src/engine/l4/summary_runner.rs`
(`solve_side_facts` String-clone), plus a `RoutineIx` interner (new small module or in `effect_store.rs`
stub). Test: `tests/l4_summary_differential.rs` (unchanged assertion), inline unit tests.

**Interfaces (produce):**
- `EffectUniverse`: add `fn effect_key_cached(&self, id: EffectId) -> &str` (compute once on first
  intern, store a `Vec<String>` parallel to `by_id`; `effect_key()` stays for compat).
- A `RoutineIx(u32)` interner: `RoutineInterner { intern(&str)->RoutineIx, get, name(RoutineIx)->&str }`,
  **canonical deterministic id assignment** (sorted stable-routine-id — spec ⟨rev4⟩).

- [ ] **Step 1 (test):** the differential (`cargo test -p al-call-hierarchy --test l4_summary_differential`)
  is the gate — it must stay 10/10 after the change. Add a unit test asserting `effect_key_cached`
  equals the old `effect_key` for a sample, and that `materialize_member_db_effects` produces the same
  `Vec<DbEffect>` order as before (a fixture with ≥2 effects whose keys sort non-trivially).
- [ ] **Step 2 (run, expect current green baseline):** `cargo test -p al-call-hierarchy --test l4_summary_differential` → 10/10.
- [ ] **Step 3 (implement):** cache `effect_key` in the universe; change `materialize_member_db_effects`
  to sort present ids by the cached `&str` (+ `operation_id` tie-break) — NO `format!` in the comparator.
  Intern `RoutineIx` and use it in `presence.by_member`, `PdState`, the via map key, and
  `solve_side_facts`'s per-member map (kill the String clone). Do NOT introduce `key_rank`/freeze yet
  (that's A3) — mid-solve ordering stays the cached-`&str` sort (spec ⟨rev4⟩ availability window).
- [ ] **Step 4 (run):** differential 10/10; `cargo test -p al-call-hierarchy --lib db_effect_solver` green; clippy clean.
- [ ] **Step 5 (re-measure):** rebuild `release-fast` + the trimmed 8020 split-probe
  (`logs/run-probe-v2-split.ps1`); record `db_solver_ms` (expect most of 517s gone). Note the number
  in the report; peak RSS likely still high (structs unchanged until A2).
- [ ] **Step 6 (commit):** `perf(l4): intern RoutineIx + cache effect_key; integer-cached materialization sort`.

### Task A2 — compact rows + u8 ViaRank + lazy DbEffect view (spec Part A Step 2)

**Files:** Create `src/engine/l4/effect_store.rs` (`CompactRoutineSummary`, `ViaRank`, `SummaryBundle`,
`DbEffectRef`, the interim per-routine set store — NO sharing yet), modify `db_effect_solver.rs`
(materialize into compact rows + `u8` via instead of `Vec<DbEffect>` + String via map), `summary_runner.rs`
(v2 returns the bundle; a compat shim rebuilds `HashMap<String,RoutineSummary>` via the lazy view for
current callers). Test: `l4_summary_differential.rs`, inline.

**Interfaces (produce):**
```rust
#[repr(u8)] pub enum ViaRank { Inherited=0, Dynamic=1, EventSubscriber=2, ImplicitTrigger=3, Direct=4 }
impl ViaRank { pub fn as_str(self)->&'static str; pub fn from_str(&str)->Self; }  // 5 canonical, byte-parity
pub struct CompactRoutineSummary { pub terminal_base: SetRef, pub pd_delta: Range<u32>,
    pub base_via: Range<u32>, pub delta_via: Range<u32>, /* + roles/unc/hu handles or kept on RoutineSummary */ }
pub struct SummaryBundle { /* rows + store */ }
impl SummaryBundle { pub fn db_effects(&self, r:RoutineIx) -> impl Iterator<Item=DbEffectRef<'_>>; }
```
(A2 uses an interim per-routine `SetRef` — one arena entry per routine, no hash-consing; A3 canonicalizes.)

- [ ] **Step 1 (test):** differential stays the gate. Add unit tests: `ViaRank` round-trips the 5
  canonical strings; the lazy `db_effects(r)` view yields byte-identical `DbEffect`s (incl
  `record_variable_id`, via) to the old materialization for a fixture.
- [ ] **Step 2 (run):** differential 10/10 (pre-change).
- [ ] **Step 3 (implement):** materialize into compact rows; store via as `u8` parallel to base/delta
  enumeration (spec Part A Step 2 — mask-OR for TERMINAL via, keep `attribute_pd_substituted_via`
  integerized for PD, record via for every transition; base seed applies to PD-typed base effects;
  `"inherited"` floor kept). Drop the String-keyed via `HashMap`. Provide the lazy `DbEffect` view;
  a compat shim reconstructs `RoutineSummary.db_effects` for existing callers via that view.
- [ ] **Step 4 (run):** differential 10/10; goldens `GOLDENS_CLEAN` (no regen); lib green; clippy clean.
- [ ] **Step 5 (re-measure):** 8020 split-probe — record `db_solver_ms` + peak RSS. Expect compute
  down; note (spec ⟨rev4⟩) the FULL RSS win lands at A3 (per-SCC `Vec<DbEffect>`/settled-strings survive).
- [ ] **Step 6 (commit):** `perf(l4): compact CompactRoutineSummary rows + u8 ViaRank + lazy DbEffect view`.

### Task A3 — FrozenEffectUniverse + key_rank + SCC-shared SetId store + delta + feed-forward on ids (spec Part A Step 3)

**Files:** `effect_universe.rs` (`GrowingEffectUniverse::freeze() -> FrozenEffectUniverse` typestate,
`key_rank`, complete pre-freeze identity collection), `effect_store.rs` (`HybridEffectSet` arena,
hash-cons, per-`EffectSetId` cached `ordered_ids`, fixed-leaf singleton classes), `db_effect_solver.rs`
(stop `c.clone()` per member — share `EffectSetId`; feed-forward reads `(EffectSetId, delta)`;
`base_via` rebuild on canonicalization), `summary_runner.rs` (the freeze lifecycle placement). Test: as above.

**Interfaces (produce):** `GrowingEffectUniverse`/`FrozenEffectUniverse` (no `intern` on frozen);
`EffectSetId`, `HybridEffectSet { Sparse(Box<[EffectId]>), Dense{words:Box<[u64]>,rank_lut,cardinality} }`,
`EffectStore::intern_set(bits)->EffectSetId`, `ordered_ids(EffectSetId)->&[EffectId]` (key_rank-sorted).

- [ ] **Step 1 (test):** differential is the gate. Unit tests for the spec's Step-3 invariants: delta
  = `key_rank`-sorted + unique-by-EffectId + exactly `delta\base`; hash-cons dedups content-equal sets
  (sparse≡dense same hash); `ordered_ids` is `key_rank` order; a fixed-leaf singleton class round-trips
  its own via; the 8-step freeze lifecycle collects ALL identity sources pre-freeze and a post-freeze
  `get` of an un-interned identity fails (typestate: no `intern` method exists on `FrozenEffectUniverse`).
- [ ] **Step 2 (run):** differential 10/10 (pre-change).
- [ ] **Step 3 (implement):** per the spec's Step-3 + the freeze lifecycle (§ "Universe-freeze
  lifecycle"). Members of one effective SCC share one `EffectSetId`; per-member `pd_delta`; feed-forward
  reads `(EffectSetId, delta)` (kills `settled.clone()` + string re-intern); fixed-leaf singleton
  classes; base∪delta emit = O(result) two-way merge of `ordered_ids` + `key_rank`-sorted delta.
- [ ] **Step 4 (run):** differential 10/10; goldens CLEAN; lib green; clippy clean.
- [ ] **Step 5 (re-measure):** 8020 split-probe — record `db_solver_ms` + **peak RSS (expect <1GB,
  target <300MB)**. ⟨rev4⟩ If residual RSS remains, attribute it (uncertainties/roles `settled`
  cloning) — do not assume db_effects was 100%.
- [ ] **Step 6 (commit):** `perf(l4): SCC-shared EffectSetId store + per-member delta + frozen key_rank`.

### Task A4 — ReverseEffectIndex (bidirectional hover index) (spec Part A Step 4)

**Files:** Create `src/engine/l4/reverse_index.rs`, wire a build pass in `summary_runner.rs`/`effect_store.rs`.
Test: inline unit tests (down/up queries on a fixture); differential unaffected (additive).

**Interfaces (produce):** `EffectClassIx` (sharing class) + `GraphSccIx` (call-graph SCC condensation,
SEPARATE — for ancestor BFS); `ReverseEffectIndex { effect_to_sccs, class_members(CSR),
effect_to_delta_routines, table_to_sccs, table_to_delta_routines }`; `down(RoutineIx)->effects`,
`touches_table(RoutineIx, TableId)->bool`, `up_table(TableId)->routine bitmap (ascending RoutineIx)`.

- [ ] **Step 1 (test):** unit tests — down(r) == the routine's computed set; `touches_table` true/false
  without decompression; up_table(X) returns exactly the routines whose base-class OR disjoint delta
  touches X, as an ascending-`RoutineIx` result bitmap; table-posting disjoint invariant
  (`table_to_delta_routines[T]` ⟺ delta touches T AND base-class does not).
- [ ] **Step 2 (run):** new tests fail (index absent).
- [ ] **Step 3 (implement):** one transpose pass over each SCC base (spec Part A Step 4); result-bitmap
  up-query; two SCC notions kept distinct.
- [ ] **Step 4 (run):** new tests green; differential 10/10 (additive); goldens CLEAN; clippy clean.
- [ ] **Step 5 (commit):** `feat(l4): ReverseEffectIndex — bidirectional effect/table<->routine queries`.

### Task A5 — CDO whole-program differential + full 8020 re-measure (gate before Part B)

**Files:** `tests/l4_summary_differential.rs` (ensure the `CDO_WS`-gated whole-program v2-vs-old parity
test exercises the new store's lazy view), a re-measure note in `docs/`.

- [ ] **Step 1:** run `CDO_WS=<path> scripts/cdo-gate` (or the gated test) — v2 (new store) vs old,
  complete-`RoutineSummary` parity for every routine. (User runs locally; document the command.)
- [ ] **Step 2:** full (non-trimmed) 8020 probe (`logs/run-probe-v2.ps1`) — record total, `db_solver_ms`,
  peak RSS, EXITCODE. Confirm the target (db-solver seconds, RSS <1GB).
- [ ] **Step 3:** tighten `tests/perf_bounds.rs`'s `compute_summaries` bound to the new reality; add a
  memory assertion if feasible.
- [ ] **Step 4 (commit):** `perf(l4): 8020/CDO re-measure + tightened compute_summaries perf gate`.

---

## Part B — retire the old solver (LAST, after A5 parity proven)

### Task B1 — freeze complete-internal baseline + delete R3b + delete old Jacobi + flip differential

**Files:** `tests/l4_summary_differential.rs` (freeze baseline + flip oracle), delete
`src/engine/l4/incremental/` + `tests/r3/r3b_*` (+ `mod incremental;` in `src/engine/l4/mod.rs`, r3
umbrella members), `src/engine/l4/summary.rs`/`summary_runner.rs` (delete old Jacobi), `src/bin/aldump.rs`
+ `summary.rs` `run_and_project` (cut to v2), any `RawSccTrace`/`Detail::Jacobi` remnants.

- [ ] **Step 1:** Freeze the **complete-internal-surface** baseline (all `RoutineSummary` fields, spec
  Part B.1) on the 10 fixtures + generated small-graph fixtures + **CDO (non-optional in the freeze)**.
  Record the pre-deletion commit sha in the test doc for forensic re-differencing.
- [ ] **Step 2:** Flip the differential from `v2 == old` to `v2 == frozen-baseline`; confirm green.
- [ ] **Step 3:** Cut `run_and_project` (aldump) off old → v2; inspect the R3a-2 trace golden diff
  (re-point to final-semantics OR retire the dump mode — never blind-regen); regen only with a
  root-caused justification recorded.
- [ ] **Step 4:** Re-grep to confirm `l4::incremental` has zero shipping consumers; preserve the R3b
  edit/invalidation test *intent* as design notes (spec Part B.3); delete `src/engine/l4/incremental/`
  + `tests/r3/r3b_*` + module decls. Do NOT cascade-delete shared `salsa::Update` derives if other
  Salsa users exist (re-grep).
- [ ] **Step 5:** Delete the old Jacobi: `compose_routine` db_effects+uncertainty folds, `run_one_scc`,
  `compute_summaries`/`compute_summaries_with_leaves`, `RawSccTrace`, `Detail::Jacobi` db_effects
  instrumentation. KEEP `compose_roles_only`/`run_one_scc_roles` + `solve_side_facts`.
- [ ] **Step 6 (gate):** `cargo build` warning-free; lib green; differential (vs baseline) green;
  `scripts/check-goldens` CLEAN (except inspected trace goldens); DO byte-identical.
- [ ] **Step 7 (commit):** may be several commits (baseline+flip; aldump cut; R3b delete; Jacobi delete)
  — stage each group explicitly. `refactor(l4): retire old Jacobi solver + R3b experiment; one path`.
- [ ] **Step 8:** Update `CHANGELOG.md` (the arc capstone entry — Added: EffectStore + hover index;
  Changed: compute_summaries perf; Removed: old Jacobi solver + R3b incremental experiment) and
  `docs/OUTSTANDING.md`.

---

## Self-Review (author checklist — completed)

- **Spec coverage:** Step 1→A1, Step 2→A2, Step 3→A3 (incl freeze lifecycle + fixed-leaf classes +
  key_rank + shared SetId), Step 4→A4 (incl EffectClassIx/GraphSccIx split + table disjointness +
  result-bitmap), the CDO+re-measure gate→A5, Part B retirement (baseline+R3b+old Jacobi+aldump+flip)→B1.
  Every ⟨rev⟩ correction has a home (key_rank no-remap in A1/A3; PD-via integerized in A2;
  fingerprint byte-parity in B/A2 via-string; Box<[u64]> + CSR + invariants in A3).
- **Placeholders:** interfaces are exact (from the spec); bodies are TDD-discovered against the live
  differential (the correct granularity for a representation redesign whose oracle is the old solver —
  same approach as the Phase-1 solver plan). The spec carries the detailed algorithm; the plan
  sequences + gates + names interfaces.
- **Type consistency:** `RoutineIx`/`EffectId`/`EffectSetId`/`EffectClassIx`/`GraphSccIx`/`ViaRank`/
  `key_rank`/`ordered_ids`/`SummaryBundle`/`DbEffectRef`/`CompactRoutineSummary` used consistently and
  match the spec's names.
- **Gate discipline:** every Part-A task keeps the live v2-vs-old differential green + goldens CLEAN;
  A5 makes CDO mandatory before B1; B1 flips to the frozen baseline only after parity is proven.
