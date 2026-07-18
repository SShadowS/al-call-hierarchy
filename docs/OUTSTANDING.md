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

  **Perf queue after Wave-2b** (re-ranked by the falsification): (1) §7
  flow-insensitive d1-walker redesign — NOW THE TOP LEVER (d1 = the sole
  full-default blocker at 8020, ~93 min alone; its 500-node DFS per in-loop
  callsite over the dense 797-SCC is the measured cost; the fix shape is a
  precomputed per-routine reachability/effect answer instead of per-candidate
  path walking — needs its own design pass, witness-fidelity is the risk);
  (2) B1 interned id universe + bitsets (output-stable) / B2 SCC-shared lazy
  cones, for the summary mass (8.34M cardinalities on one SCC) and the
  Jacobi plateau. B3 single-substrate unification still needs a
  detector-feature parity harness first. SEQUENCE with the `to_lowercase()`
  census below — B1 rewrites the same `src/engine/l2`-`l5` call sites; do
  the fold-primitive swap as part of (or immediately before) B1's interning
  pass, never as separate churn. Also feeds the change-impact wedge's Q1/Q2
  fork (effects-on-fresh): the wedge's cone substrate should be B1/B2's
  bitset cones, and the findings doc is the evidence AGAINST making L3
  load-bearing again
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

## Parked — deferred WITH evidence; do NOT start without the wake condition

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
