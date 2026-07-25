# Outstanding items

Living checklist — tick items (`- [x]` + landing commit/date) as they land; add new
items as they surface. Rewritten clean 2026-07-17 (post preflight-fresh-coverage +
outstanding-sweep runs); the full histories of completed arcs live in the Archive at
the bottom, CHANGELOG, and git log.

## Open — needs the user

- [x] `git stash drop` leftover stashes — DONE 2026-07-17: user ran `git stash clear`
  (26 accumulated scratch stashes from merged arcs, all superseded; verified 0 remain)
- [x] `/triage-wave` sharing — DONE 2026-07-17 (`79bf189`): `.claude/commands/`
  un-ignored and versioned (project doctrine as tooling); CLAUDE.md worktree note updated
- [x] **d61/d62/d64 validation** — DONE 2026-07-17 (`f3f5c85`). Corpus: Microsoft
  System App + Base App 28.0 embedded source extracted from DO's `.alpackages`
  (9.3k real files). d62: 9 findings triaged (1 real, 8 FP) → structural
  branch-exclusivity class ROOT-CAUSE FIXED via statement_tree (9→4; the
  `if Success then Log else Error` idiom no longer flags); 3 residual semantic FP
  classes documented → stays opt-in. d64: first population (8 API pages) → only FP
  class (SourceTableTemporary) fixed, 2→0 with honest skips → stays opt-in (no TP
  yet). d61: 7,367 real candidates, 0 emissions, guards hold (caveat: sliced corpus
  may hide cross-slice event pairs) → stays opt-in. Promotion wake for all three:
  a triaged true-positive population

## Open — buildable backlog (no blocker, pick up any time)

- [x] **Engine memory/speed Wave 1 (Track A)** — DONE 2026-07-18 (branch
  `worktree-design-engine-memory-speed`, commits `9c0ee77..708f000`, 10 tasks
  SDD-executed + per-task reviewed, goldens byte-stable throughout). Base App
  8k 3-detector: DNF@90min/35.8GB → **90 s / 6.1 GB**; slice-5400 236s/9.8GB →
  58s/3.4GB; DO unchanged (10.7s, byte-identical). W1.0 demand-driven substrate
  (per-detector requires + full-vs-minimal parity test), W1.1 Jacobi
  (uncertainty index, serde-free change keys w/ equivalence proof, take-based
  snapshot, dirty frontier), W1.2 SpanTemplate, W1.3+A7 move-don't-clone,
  W1.4 parallel L3 parse, W1.5 FingerprintIndex-once, A8 cross-ext hoist,
  A9' parallel diagnostics re-parse. Decision (a): substrate-skipping runs omit
  summarize cap-hit diagnostics (only permitted output change). Wave-1 outcome
  table: findings doc §7b
- [ ] **Engine memory/speed Wave 2/3 (Track B)** — Wave-2a (measure-first
  root-cause + mechanical fixes) DONE 2026-07-18 (`e2e34fc` structural
  stable-id substitution in `fingerprint_of`; `136c4e2` zero-alloc
  `reachable()` iteration + memoized `touches_db` in d1; both byte-stable,
  goldens + DO diff clean). Slice-5400 full-default 2,608s→304.2s (8.6×;
  d19 988→0.23s and d12 425→0.07s effectively eliminated, d1 448→157.9s —
  2.8× but still 87.7% of the loop); 8020 3-detector 90.3s→40.9s (2.2×); DO
  unchanged (9.0s). **8020 full-default STILL DNF** (2h cap, 45.2 GB peak;
  d1 alone runs ~93 min and never finishes) — walk-graph SIZE at 846-SCC
  density, not per-step allocation cost, is now the measured limiter. The
  846-SCC's trigger-edge over-approximation hypothesis (the fusion driver)
  is source-sampling VERIFIED: 20/20 sampled intra-SCC implicit-trigger
  edges over-approximated, 97.1% (1,046/1,077) a field-collapsed OnValidate
  target-collision (Sales Header alone collapses 93 distinct field triggers
  onto one graph node). Full numbers + derivation:
  `docs/superpowers/specs/2026-07-18-wave2-measurements.md` §2/§3/§3a/§4/
  §4a/§6.

  **Wave-2b (trigger-edge builder parity) DONE 2026-07-18** — `a640815` +
  `f9ff427` (field-specific OnValidate targeting + RunTrigger gate in
  `build_implicit_trigger_edges`, mirroring `implicit_trigger_route_applicable`;
  TDD incl. quoted-field normalization guards; zero golden movement — the
  committed corpus never exercised the pathology; DO findings byte-identical
  with 65 over-approximated edges pruned, telemetry-only). **The performance
  hypothesis was FALSIFIED**: 8020 max_scc 846→797 (-5.8%), timings flat —
  the SCC is fused by direct(1067)/method(262) call cycles, and retargeted
  per-field OnValidate edges stay inside the component. The fix STANDS on
  precision/parity grounds; its perf claim is dead. Full honest numbers:
  measurements doc §7 (Wave-2b outcome).

  **Wave-2c (d1 walk_evidence memoization) DONE 2026-07-18** — `511845c`:
  per-callee memo (caller-independence proven — prefix + additive-depth
  transform; full-field memoized≡fresh test), byte-identical (goldens + DO
  clean), O(in-loop-callsites)→O(distinct callees) walk count. **The 8020
  full-default finish bar is STILL UNMET** (2h cap, 51.9 GB peak) — but this
  measurement batch is HONEST-BLIND: probes were swept and ~55% ambient
  machine load inflated even non-d1 control runs, so the batch's numbers are
  unusable in either direction (measurements doc §8).

  **ATTRIBUTION DELIVERED 2026-07-19** (permanent perf_trace layer + three
  traced d1-only runs; measurements doc §9): d1 is OUTPUT-BOUND on
  complete-path multiplicity — 69.1% of retained walk results are genuine
  complete witness paths (~126/walk, ~900k for the full census ≈ 3h),
  Jacobi/substrate CLEAN at ~24 min / 43.5 GB.
  **RESOLVED 2026-07-19 — d1-reachability redesign (Tasks 1-5,
  `feat/d1-reachability`)**: the "output-bound" premise was FALSE (the ~900k
  paths never reached output — first-wins dedupe discarded them; the cost was
  the enumeration). NO caps taken. `detect_d1` now runs an unbounded filtered
  reachability search (`d1_graph` + `d1_reach`) + terminal-centric assembly
  (one finding per `(terminal routine, op)` with per-loop `LoopContext`s); the
  old walk is deleted from d1 (survives only as the `#[cfg(test)]` shadow
  oracle). DO semantic-diff vs old: 828→908 findings (+80 budget-free), 61
  severity upgrades, 0 downgrades / 0 vanished keys, 5.2 s. Goldens rebaselined
  + triaged. Task 6 = perf re-measure at 8020 density + handoff §4 closure.
  Downstream candidates once attributed: d1 typed-receiver-§7
  guard-tag/flow-insensitive redesign; B1 interned ids + bitsets / B2
  SCC-shared cones (8.34M-cardinality summary mass, Jacobi plateau); B3
  single-substrate unification (needs detector-feature parity harness).
  SEQUENCE B1 with the `to_lowercase()` census below (same call sites, one
  churn). The change-impact wedge's effects-on-fresh fork still consumes
  B1/B2's bitset cones; the findings doc remains the evidence AGAINST making
  L3 load-bearing again
- [x] **tree-sitter-al quirks list** — WAS ALREADY DONE, stale item (live-verified
  2026-07-17 against pinned v3.2.0 `14bd55c`): `statement_block`/`argument_list`/
  `parenthesized_expression` carry ZERO fields (left/operator/right pollution gone,
  fixed 2026-06-27/28 grammar arcs), `case_else_branch` HAS the `body` field
  (asymmetry fixed), member_trigger_name landed, spaced-preproc closed at v3.2.0.
  The grammar has no documented open limitations
- [x] **Multi-root LSP workspaces** — DONE 2026-07-17 (`6470e3e`). Per-root
  `ServerState` map (`Workspace`/`RootState`, each root gets its own `LspSnapshot`/
  updater/watcher/`DiagnosticsState`) + URI→root routing (`route_uri`, longest-prefix)
  for `dispatch_request`/`handle_notification`; `incomingCalls`/`outgoingCalls` route
  via a stamped `CallHierarchyItem.data` root marker instead (required, not cosmetic —
  `RoutineNodeId.AppRef` is a raw per-snapshot index, so the same id value can name a
  different routine in a different root). Single-root byte-identical (no marker/
  warnings ever emitted; the pre-existing dispatch test's assertions untouched). New
  follow-up surfaced by this work: `workspace/didChangeWorkspaceFolders` is NOT
  implemented — safe root removal needs an `AlFileWatcher` cancellation signal that
  doesn't exist yet (see `server.rs`'s module doc); the notification now warns loudly
  instead of being silently swallowed. Report: `.superpowers/sdd/multiroot-report.md`
- [x] **Snapshot-scoped LineTable cache** — DONE 2026-07-17. `ParsedFileEntry` gained
  a `OnceLock<LineTable>`-backed cache (rides the existing Arc-forwarding
  invalidation architecture, no new bookkeeping); `LineTable` moved from
  borrowing `&'t str` to owning `Arc<str>` so it can be stored. `incoming`
  ~5.82ms → ~4.30ms median on the 999-way-fan-in synthetic corpus (noisy
  machine — see `.superpowers/sdd/linetable-cache-report.md`); `dep_texts`
  (dependency-embedded-source) deliberately left uncached (smaller, rarer
  population). All perf_bounds gates still pass.
- [x] **Unicode-fold moat task** — DONE 2026-07-18. New choke point
  `al_syntax::{fold_identifier, eq_fold_identifier, IdentifierFoldExt}`
  (`crates/al-syntax/src/casing.rs`, `is_ascii()`-guarded simple 1:1 Unicode
  fold — byte-identical to `to_ascii_lowercase` for all-ASCII input, never
  `str::to_lowercase()`'s 1:n `İ`→`i̇`). Mechanically swapped every SEMANTIC
  identifier fold across `crates/al-syntax`'s lowerer, `src/program/`
  (production+lookup sides together, one commit), and `src/engine`+`src/lsp`
  — 3 commits, one per layer, each landing green. New fixture
  `tests/r0-corpus/ws-unicode-fold/` proves cross-case non-ASCII identifiers
  (Danish `Løbenr Mgt.`/`LØBENR MGT.`, German `Prüfung`/`PRÜFUNG`) now
  resolve via `Evidence::Source` — verified they would NOT under the old
  ASCII-only fold. North-star SHA guard: **unchanged**
  (`0a3b85bc832ff0a3e77acee118d203edbf62827dc37617c8d9315fe52d5cb7d0`, exactly
  as the investigation predicted — DO's primary source is 100% ASCII).
  Report: `.superpowers/sdd/unicode-fold-report.md`
- [x] **r3a4 source-bearing-dep pin hardening** — DONE 2026-07-17 (`8b5b4ec`):
  closure-membership assert added; the pin can no longer be vacated by a fixture edit

- [ ] **Multi-root follow-ups** (from the 6470e3e review): (a)
  `workspace/didChangeWorkspaceFolders` deferred — safe root REMOVAL needs an
  `AlFileWatcher` cancellation signal that doesn't exist (warns loudly today); (b)
  nested-root diagnostics overlap — two nested AL app roots can both publish
  diagnostics for the same URI (last-write-wins clobber); routing handles nesting,
  the publish side lacks URI-ownership arbitration. Both narrow; build when a real
  client hits them

- [ ] **`str::to_lowercase()` census in the advisory engine** (surfaced by the
  unicode-fold arc): ~364 sites across `src/engine/l2`-`l5` use full Unicode
  `to_lowercase()` (the 1:n-hazard primitive) as their own pre-existing convention —
  inconsistent with the new `fold_identifier` simple-fold choke point. One live
  interaction traced neutral-to-improving; population of divergent inputs is empty
  today. Migrate to `eq_fold_identifier`/`fold_identifier` layer-by-layer for
  consistency (low priority; advisory engine only)

- [ ] **perf_bounds `compute_all_within_bound` CI flake** (seen once, 2026-07-18,
  docs-only push; adjacent runs of the same code passed): magnitude bound lost to
  shared-runner load variance — the exact class the T3 arc fixed for rung bounds via
  interleaved complexity-class assertions. Give compute_all the same load-stable
  treatment if it flakes again

- [x] **L4 db-effect RSS consumer-migration — remove the analyze-path
  materialization shim** — DONE 2026-07-24 (`a0cd348`, Part B B1-migrate). The
  analyze path (`build_detector_context` + the cross-app builder) now consumes
  the LEAN bundle entry points, and the compact `SummaryBundle` rides on
  `DetectorContext.db_effect_bundle` so `db_effects` stays queryable without the
  per-routine `Vec<DbEffect>` expansion. MEASURED (8020, `release-fast`, full
  detector set): `context.compute_summaries` `rss_delta` 24 250 MB → **477 MB**
  (24 GB → 0.47 GB — the sub-GB target), span wall 87 s → 15.7 s, whole-process
  peak 39.9 GB → 18.1 GB, `analyze.total` 620 s → 366 s. The shim itself is
  UNCHANGED and still serves the projection + differential. Original item:
  (`src/engine/l4/summary_runner.rs`
  `compute_summaries_v2_bundle_with_leaves` → `_core`; the follow-up the L4 store
  redesign B1 explicitly deferred). B1 deleted the old Jacobi solver and flipped
  the differential to a frozen baseline, but the shim that re-expands the shared
  `EffectStore` into an owned `Vec<DbEffect>` per routine (so the returned
  `HashMap<String, RoutineSummary>` keeps the legacy shape) STAYS — the projection
  (`summary.rs::project_r3a2`) and the differential still need materialized
  `db_effects`. Measured cost (8020,
  `docs/2026-07-24-l4-dbeffect-store-8020-remeasure.md`): `context.compute_summaries`
  ~87 s + ~24 GB peak RSS is dominated by this shim, and the analyze path never
  READS `RoutineSummary.db_effects` (detectors consume only `.uncertainties` /
  `.parameter_roles` / capability facts — verified grep). Migrate the analyze path
  (`detector_context` / `gate`) to the bundle's borrowing view + the A4
  `ReverseEffectIndex`, keeping a materializing path ONLY for the projection +
  differential. Expected: −24 GB, `compute_summaries` ~87 s → ~13 s. No wake
  condition — buildable now.
- [x] **C1 — `context.capability_cones` base-assembly RSS** — DONE 2026-07-24
  (Tasks 1–4, plan `docs/superpowers/plans/2026-07-24-c1-cone-derived-substrate.md`;
  see the CHANGELOG `Changed` entry for the full shape). Diagnosis:
  `.superpowers/sdd/C1-cones-diagnosis.md`; residual attribution:
  `.superpowers/sdd/c1-residual-census.md`. The compact `ConeDerivedStore`
  replaced the per-routine raw inherited-fact Vec on the analyze path
  (`ConeOutput::DerivedOnly`), and the SCC walk stopped materializing cones no
  predecessor will ever read — it is Task 4's code change (not building a root
  SCC's cone at all) that takes the root-SCC residual to 0; the
  `C1_CONE_CENSUS=1` byte census only MEASURES that it stayed at 0, it did not
  cause it. MEASURED (8020, `release-fast`, `d8-commit-in-transaction`-only
  shape, EXITCODE=0): span `rss_delta` 10 941 MB (pre-C1) → 2 151 MB (Task 3) →
  2 195 MB (Task 4); whole-process peak 17 055 MB → 9 593 MB (Task 3) →
  **7 787 MB (Task 4)**; wall 213 s → 196 s → 127 s. The span's own `rss_delta`
  barely moved Task 3→4 despite the ~1.8 GB peak drop: `rss_delta` is working
  set at span end minus span start, and the root cones were always freed
  inside the span either way (when `compose_inherited_cones` returned) — they
  lived in the PEAK, which is where Task 4's saving shows up, not in the
  delta. Output byte-identical throughout (five golden families + l4
  differential 17/17 + DO `analyze` and `policy check`). Post-Task-4 the
  largest remaining spans are all OUTSIDE this arc — `l3.assemble_resolve`
  3 381 MB, `l3.parse_project_parallel` 2 770 MB, `context.symbols_resolve_calls`
  1 723 MB, `gate.coverage` 1 157 MB — so the cone span is no longer the
  dominant consumer, but the arc did NOT reach sub-GB whole-process: the
  remaining ~7.8 GB peak is L3-substrate work, not reachable by B1 + C1 alone.
- [ ] **A future incremental L4 path over the new `EffectStore`** (a redesign, NOT
  a re-port of the deleted R3b). The reusable design intent — fine-grained Salsa
  query topology, SCC-identity rules (interned sorted-member `SccKey`), fixed-leaf
  successor handling, deterministic sorted member-order, the demand-order /
  DB-provenance / fixpoint-schedule / `RUST_HASH_SEED` nondeterminism invariants,
  and the strict-subset minimal-invalidation fixtures — is preserved in
  `docs/superpowers/notes/2026-07-24-r3b-incremental-l4-design-intent.md`. Wake: a
  real incremental-analyze consumer.
- [ ] **`salsa` derive-only dependency** — after the R3b `incremental/` deletion
  (Task B1), nothing in the crate consumes `#[salsa::db]`/`#[salsa::input]`/
  `#[salsa::tracked]`/`#[salsa::interned]` any more; the only surviving usage is
  `#[derive(salsa::Update)]` on a handful of `l4` types (`summary.rs`,
  `combined_graph.rs`, `scc.rs`, `capability_cone.rs`). A full salsa-ectomy
  (drop the derives + the `Cargo.toml` dep) is a legitimate future cleanup — not
  urgent, no correctness or perf stake, just dead-weight-dependency hygiene.
  Wake: convenient piggyback on an unrelated touch of those types, or a
  dependency-audit pass.

## Parked — deferred WITH evidence; do NOT start without the wake condition

- [ ] **`compute_routine_id` member-discriminator gap — colliding same-name
  triggers share ONE id** — `compute_routine_id` (`src/engine/l2/scope.rs`)
  keys app/object-type/number/kind/name/signature with NO member
  discriminator, so two same-name same-signature triggers in one object (e.g.
  any page with two actions each declaring `trigger OnAction()` — ordinary in
  real BC) collide on one routine id. This is the SAME collision family as
  `docs/engine-gaps.md`'s **G-18** (which fixed a different symptom — d1's
  cross-body loop misattribution — and correctly remains marked FIXED).

  **MEASURED, 2026-07-25** (`.superpowers/sdd/scope-routine-id-collision.md`),
  correcting this entry's own earlier "handful of routines" framing by three
  orders of magnitude:

  | corpus | routines | collision groups | routines erased by the collapse |
  |---|---:|---:|---:|
  | DO (`DocumentOutput/Cloud`) | 4 842 | 262 | **1 157 (23.9 %)** |
  | 8020 (BC Base App) | 100 941 | 3 058 | **16 906 (16.7 %)** |

  Largest group: 17 routines on one id (DO), 100 (8020). Every colliding
  routine is a member trigger; 98 % of groups carry real call graph.
  `enclosing_member` is already in the model at every `compute_routine_id`
  call site and closes DO to **0** residual groups, 8020 to 15 groups / 19
  routines (preproc `#if` alternatives + XMLport same-name elements at
  different nesting paths).

  **DONE (T1, 2026-07-25): the cone-LOSS symptom is fixed.**
  `build_detector_context` drained its cone maps with `remove()`, so a later
  occurrence of a colliding id wrote a fully degenerate summary over the real
  one and `ConeDerivedStore::forget` dropped the matching derived row — the
  whole cone of an id shared by N routines was erased.
  (`build_detector_context_cross_app` reads with `get()` and never had the
  accident, which is what identified the drain as an accident rather than a
  decision.) The builder now skips the later occurrence instead; `forget` is
  deleted. Measured on DO: **+4 `d8-commit-in-transaction` findings, −1
  `d9-transaction-span-summary`, 20 findings changed in place** — see
  `.superpowers/sdd/task-1-report.md`. Zero golden movement.

  **STILL OPEN — the id schema itself.** With one id per N siblings the
  surviving answer is still ONE arbitrary sibling's, and the derived
  (`cs`/`op`/`loop`) ids, the merged call edges, `routine_by_id`'s last-wins
  `collect()`, the shared **stable** id (⇒ shared fingerprint ⇒ one baseline
  entry suppresses N findings), and G-19's collision disqualifier all remain.
  It is visible in T1's own output: the new `CDO Move Logs` d8 finding anchors
  on line 212 (`UpdateStatusAction`'s trigger) while its `Commit()` is at line
  188 in the sibling `StartmovinglogsAction` — `routine_by_id` resolved the
  last sibling and the op-site lookup fell back to its declaration anchor.
  Recommended fix is the conditional member discriminator on both the internal
  and stable id, keeping the `{mid}/{64hex}` / `{stableObjectId}#{64hex}`
  SHAPE intact (§3.2 of the scope doc: `substitute_stable_ids` scans for
  exactly 64 lowercase-hex bytes, and `stable_sub_id`/`DepIdStabilizer` assume
  a two-part split — a `#member` suffix or an extra segment silently moves
  every fingerprint in the product). **Wake:** an isolated arc whose only
  expected diff is id-shaped (~561 committed goldens carry a raw internal id;
  regenerating them alongside unrelated movement is strictly worse than doing
  it alone).
- [ ] **`ReverseEffectIndex` (779 lines, `src/engine/l4/reverse_index.rs`) is
  built and tested but has zero production callers** — built at A4 with
  wiring explicitly deferred to B1 ("the hover consumer"); B1 ran (retire +
  migrate) and deliberately did NOT wire it (eager construction would add
  unconsumed cost to every `analyze` run — the right call), so the stated
  wake condition passed without being met. Every `ReverseEffectIndex::build`
  call site is inside its own `#[cfg(test)]` module; its only other mentions
  in the codebase are two doc comments (`summary_runner.rs`,
  `detector_context.rs`). It has never executed against real data — no
  golden, no DO run, no CDO run reaches it — so its 7 self-consistency tests
  are its entire correctness evidence. **Wake:** the first db-effect-reading
  consumer (originally planned: VSCode hover) or a future `finding`/query
  surface that needs an effect/table ↔ routine lookup.
- [ ] **Preflight shared parse** — measured 2026-07-17: duplicated work is the PRIMARY
  app's parse only (deps parse once in the fresh pass); on DO that's 407 files of a
  dep-dominated 4.8 s resolve → sub-second saving. Live BOM divergence (DO has 4
  BOM-carrying `.al` files; snapshot keeps BOM, L3 strips) makes naive sharing
  behavior-changing. Investigation: `.superpowers/sdd/shared-parse-investigation.md`.
  **Wake:** analyze latency becomes user-facing pain, dep-parse caching lands, or BOM
  handling gets unified anyway
- [ ] **FreshCoverage ABI-error / missing-dep reporting** (+ serde-default-empty
  exemption hardening) — population-less on DO (0 ingest failures, 0 declared-but-
  missing; real ingest failures already surface as could-not-verify). **Wake:** the
  first real failing-ingest or missing-declared-dep population, or a SymbolReference
  emitter shape change
- [ ] **Number-less object identity collision (engine-wide)** — `o.id.unwrap_or(0)`
  (`src/engine/l2/l2_workspace.rs:355/414/593`) gives every Interface/ControlAddIn in
  an app the id `{guid}/{type}/0`. Harness symptom fixed; harm latent (DO: 5
  interfaces share one id, zero shared routine names → no routine-id collapse
  observed). Fix is a stable-id earthquake (fingerprints/baselines/digests/cache).
  **Wake:** two same-app number-less objects sharing a routine name, a misattributed
  production finding on an interface, or the next planned stable-id break (piggyback)

## Parked — call-graph roadmap (doctrine-deferred, population-less)

- [ ] ProvenAbsent — wake: a real proven-absence population (MemberNotFound is 0)
- [ ] Implicit conversions — wake: nonzero `ambiguousResolved` (currently 0)
- [ ] Full ParseStatus gate — wake: the first absence-claiming consumer
- [ ] Protected `Variables[]` — wake: an extension routine consuming a base protected var
- [ ] Preproc-symbol fidelity — wake: a real consumer
- [ ] Sender param-TYPE drift analysis — wake: a version-drifted-closure corpus

## Product direction (post-1.0 — needs a brainstorm session, not a dispatch)

- [ ] **Change-impact wedge** — the charter's headline product feature ("what breaks
  if I change X" over the zero-unknown whole-program graph). Brainstorm input +
  substrate map + the 8 open design forks:
  `docs/superpowers/notes/2026-07-18-change-impact-wedge-brainstorm-input.md`
  (its file:line substrate map is a `b7da82d` snapshot — re-verify after any refactor;
  the product framing is refactor-independent). Biggest architecture fork: effects-on-
  fresh vs re-consuming the advisory L4 layer

## Separate track

- BC-Brain — its own product backlog (`SShadowS/bc-brain`), never mixed into this list.

---

## Archive — completed (compressed; details in CHANGELOG + git log)

2026-07-17, outstanding-sweep run:
- [x] Push master to origin (113 commits, `e6b1283..d695392`; then continuously)
- [x] Differential-harness identity keying + wrong IEmpty fingerprint golden
  (`fix/outstanding-test-bugs`; "flaky" claim falsified — was deterministic-wrong)
- [x] gate_sarif regen-mode anti-degenerate bypass (`819790d`)
- [x] condition_references consumer audit — CLEAN, no consumer bitten
  (`.superpowers/sdd/condref-audit-report.md`)
- [x] d56 re-promotion OPT-IN → DEFAULT via keyRemappedClone analysis (`752a496`;
  DO: 0 findings, both real key-remap sites verified excluded)
- [x] MERGE-TIME CRLF re-materialization on master (552 files; detection law: use
  `file`/`od`, never grep — MSYS grep strips CR)
- [x] Stale-section corrections: deep-review T0-T4 ALL merged long ago (T2 `542740e`,
  T3 = the LSP-migration arc incl. legacy-pipeline deletion, T4 `d99c65e`); both
  Recovered-parse grammar defects fixed at grammar-defects-and-repin
  (`recoveredFiles` re-measured 0 on CDO)

2026-07-17, preflight-fresh-coverage arc (`d14cf84`):
- [x] §1 preflight fix — analyze preflight re-keyed to the fresh resolver
  (FreshCoverage + could-not-verify state + fail-closed hole + empty-ABI exemption);
  DO warning gone, totalFindings 2307 exact, north-star SHA byte-identical

2026-07-16/17, BCQuality detector wave (`8bb9756`):
- [x] 13 detectors d52–d64 + `bcquality` preset; FP triage on DO; root-cause fixes for
  d53/d56/d60/d63 (only d56 was temporarily opt-in, since re-promoted)

## d1-db-op-in-loop cohort redesign — RESOLVED 2026-07-21 (merged ee3aa45)

The perf-optimization-handoff §4 "open decision" (d1 output semantics / no-caps)
is CLOSED. d1's exhaustive path-enumeration walk — which took ~8h on Base App
8020 and was always killed — was rewritten as an IFDS/reachability-indexing
COHORT DATAFLOW (co-designed with gpt-5.6-sol; see memory `d1-output-bound-falsified`
§2026-07-21 for the full design + the 24-commit arc C1-C9 + cleanup):

- Per-loop reachability computed once as bitmap COHORTS (Terminal -> ContextKey
  -> loop-bitmap), not per-loop re-traversal. Compressed terminal-centric output
  (interned loop-sets + loop catalog + ONE bounded representative witness per
  verdict-class). Reuses d1_graph/d1_liveness/d1_temp; the old walk path is now
  cfg(test) as the differential oracle.
- **8020 now FINISHES** (~26min total, d1 detector ~140-250s, machine-noise-
  dependent; pure compute floor 9.35s). DO (real customer ws) BYTE-IDENTICAL on
  all identity fields at 6-7s. Correctness proven: decompressed cohorts == old
  (loop,terminal,verdict,depth_bucket,unc) tuples + reachable_verdicts (differential
  + regenerated goldens + DO diff). Whole-branch review clean.
- Output shape CHANGED (user-approved): compressed cohorts vs old per-loop contexts;
  pathCount now counts verdict-classes.

### Deferred d1 follow-ups (non-blocking; 8020 already finishes)
1. **Witness/uncertainty polish (~130s residual)**: `build_cohort_rep`'s full-chain
   `path_uncertainties` walk for UNCERTAIN cohorts is the residual (certain cohorts
   already skipped, ee07983). Eliminate by accumulating uncertainty-KIND-SETS in the
   fixpoint (no walk) — output-identical, targets d1 ~10-30s. NEEDS A QUIET MACHINE
   to validate (these detached 8020 runs swing +/-80s; sub-fixes unmeasurable against
   that noise).
2. **`affected_objects` bitmap-partition** (d1.rs ~1807): a `bm.iter()` loop over the
   ~3.2M (loop,terminal) population, same shape C9 bitmap-partitioned for `by_rv`.
3. **`finding.rs` LoopContext/StableLoopContext cleanup**: the superseded Task-5
   per-loop schema is dead-in-practice but referenced from the generic
   `project_finding` — removing needs a Finding/StableFinding schema change.
4. **Global-arrival-cohort solver** (the "full" redesign, deferred): only if the
   FIXPOINT ever becomes the bottleneck (currently 9.35s, fine).
5. **Depth-semantics**: the 22,511 reached terminals include deep-chain findings
   (arguably spurious SCC artifacts from no-depth-bound); a depth bound would cut
   count + improve precision, but the user chose no-caps. Revisit only if the deep
   findings prove noise on real triage.

MEASUREMENT GOTCHAS (recorded so a fresh session doesn't re-learn them):
`ALSEM_TRACE_DETAIL=hot` ALONE (not `stages,hot` — parse falls to Stages, gating
off Hot counters). Detached `Start-Process`+sentinel (logs/run-det-d1only.ps1)
survives the harness reaping background bash at ~30-90min. span names are BARE not
cat-prefixed; `serde_json` sorts object keys (grep individual keys).
