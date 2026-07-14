# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed
- `diagnostics::rung1_cover` (final tier-2 whole-branch review finding): the
  cover resolved each affected `RoutineNodeId` to a SINGLE file via
  `decl_by_id`, whose winner for a cross-file-duplicate id is explicitly
  unspecified — so when a rung-1 edit flipped a duplicated procedure's
  `effective_incoming_count`, only one of its declaring files was recomputed
  and the other's unused-procedure/high-fan-in verdict stayed stale until the
  next rung-2/3 swap. The cover now scans `decls_by_file` and inserts EVERY
  declaring file of every affected id (one hash probe per workspace decl,
  microseconds). New regression test
  `rung1_cover_includes_all_declaring_files_of_duplicate_id` (verified to fail
  against the old lookup).
- Merge-identity effect-fact dedup (previous entry, below): the skip was not
  provably output-neutral for an identity's FIRST duplicate — a duplicate-free
  entry from the fresh-insert branch stores its paths in raw BFS order, not the
  MERGE branch's `(projected_len, query_hops_json)`-sorted + json-deduped order,
  so a later duplicate that used to trigger a real merge (and thus normalize the
  entry) previously just vanished, silently leaving the entry un-normalized.
  `digest_one_root` now re-normalizes the target accumulator entry (via a new
  shared `merge_normalize_via_paths` helper, used by both the live MERGE branch
  and this path) on an identity's FIRST duplicate — with no new paths appended,
  since the duplicate's own path set is a proven subset of what's already stored
  — and skips outright from the SECOND duplicate onward (a normalized list is a
  fixed point). `tempState` is recomputed for real (cheap, non-BFS) rather than
  assumed unchanged, since an unrelated identity sharing the same `dedupe_key`
  may have mutated it in between. Byte-identical on CDO (fc-verified); previously
  this was corpus luck, not a proven invariant. transaction-integrity preset:
  ~24.7 s → ~24.9 s (no measurable regression from the fix).
- `alsem analyze` no longer hangs (10+ min, single core pegged) on large workspaces: the
  L4.5 ordering-facts pass (43.6 s+ on CDO, superlinear witness reconstruction) ran eagerly
  in `build_detector_context` although only the OPT-IN d47/d49/d51 detectors read it. It is
  now computed lazily on first `get_ordering_facts()` access (al-sem's own memoized
  `ctx.getOrderingFacts()` semantics), so the default detector set never pays it. Default
  `analyze` on CDO: never-completes → ~6.3 s. Output byte-identical.

### Changed
- **Tier-2 latency wave close-out** (docs `docs/perf-regression-t3-vs-0.9.3.md`
  §13): final independent median-of-5 CDO measurement pass confirms the
  per-task numbers below hold together as a whole — rung-1 save end-to-end
  (apply + diagnostics) ≈5.37 ms, rung-2 641.7 ms (unaffected by this wave,
  within noise of the arc-start baseline), cold start 3.069 s, `aldump`
  wall time 3.511 s, steady-state peak RSS ~1,584 MB (unchanged — this wave
  landed zero RSS movement, see below). North-star SHA-256
  `0a3b85bc832ff0a3e77acee118d203edbf62827dc37617c8d9315fe52d5cb7d0` held
  byte-identical across all 5 trials. Item A (dep-node RSS, ~49 MB) is
  **deferred** under both of its known designs: the investigation's original
  three-segment-façade variant (too invasive to the north-star resolver
  core) and Task 4's drop-and-rebuild variant (measured ~1.49 s added to
  rung-2, ~4× the brief's own regression ceiling — see Task 4's report,
  no code committed). Item C (`RoutineNodeId` interning) confirmed NO-GO
  standalone (18.7 MB measured, not the originally-claimed 40-80 MB). Tier-2
  is re-labelled the **incremental-latency wave** (Tasks 1-3); the RSS work
  (item A) is carried forward as its own future arc — see docs §13.5 for the
  two candidate future routes (widening the frozen `DeclSurface` tier, or an
  M4 disk-cached dep layer).
- The three serial per-file resolve loops (`resolve_full_program_from_parts`'s
  Phase-1 loop in `src/program/resolve/full.rs` — the `aldump`
  north-star/cold-start path; `LspSnapshot::from_context`'s per-file loop in
  `src/lsp/snapshot.rs`; `Updater::apply_rung2`'s per-file loop in
  `src/lsp/updater.rs`) are now parallelized (Tier-2 latency wave, Task 3 /
  item E / F7). Each `resolve_file_obligations`/`recompute_file` call reads
  only immutable shared borrows (`&graph`/`&index`/`&surface`/
  `&obj_node_map`) and returns its own per-file result — embarrassingly
  parallel. Each site now collects the ordered, already-filtered file list
  first, resolves it with an INDEXED `par_iter`/`collect()` (which preserves
  iteration order), then folds the results in the ORIGINAL file order in a
  sequential pass — byte-identical to the old serial loop's accumulator
  inserts. Runs on a dedicated `crate::big_stack::big_stack_pool()` (32 MiB
  worker stacks), not the rayon global pool: the resolver's
  receiver/extraction walk recurses over the AL expression tree and can
  overflow rayon's default ~1 MiB worker stack on real BC files — the same
  hazard `snapshot::parse::parse_snapshot` already guards against for the
  lowerer. Measured on CDO
  (`u:\Git\DO.Support-SlowDOSetup\DocumentOutput\Cloud`, best of 3 runs):
  CLI cold start (`al-call-hierarchy --project`) 3.36 s → 3.00 s (−11%);
  `aldump --program-call-graph-stats` wall time 3.73 s → 3.42 s (−8%).
  North-star JSON on CDO remains byte-identical, SHA-256
  `0a3b85bc832ff0a3e77acee118d203edbf62827dc37617c8d9315fe52d5cb7d0` (fc-verified,
  no resolver change). `cargo bench --bench lsp_pipeline`'s synthetic corpus:
  `build_full/100_files` −6.3%, `build_full/1000_files` −22.0%.
- `src/lsp/diagnostics.rs`/`src/lsp/updater.rs`/`src/server.rs`: diagnostics recompute
  is now scoped to the swap that triggered it (Tier-2 latency wave, Task 2 / item D).
  `spawn_updater`'s `on_swap` callback signature changed from
  `Fn(&LspSnapshot, &LspSnapshot)` (old, new) to `Fn(&LspSnapshot, &SwapScope)`, where
  the new `SwapScope` enum is `Full` (rung 2/3 — every file is recomputed, unchanged
  behavior) or `Rung1(Rung1Delta)` (rung 1 — only a restricted cover is recomputed). A
  new `compute_for_files` (restricted key set) shares a single per-file `compute_file`
  helper with `compute_all` so the full and partial recompute paths can never drift.
  `DiagnosticsState::diff_partial` diffs only the supplied uris, leaving every other
  uri's last-published state untouched (unlike `diff`, it never clears an absent uri —
  a rung-1 swap never adds/removes workspace files). The rung-1 recompute cover
  (`diagnostics::rung1_cover`) is `delta.files` (the edited files) UNION every
  `virtual_path` declaring a decl in `delta.affected_ids` (Task 1's per-file
  `incoming`-delta) — complete because both cross-file diagnostic rules
  (unused-procedure AND high-fan-in) depend on exactly `incoming` + `publisher_fanout`,
  and rung 1
  never changes `publisher_fanout` (Task 1 Arc-forwards it unchanged). `Updater` gains
  a public `rung1_context` accessor so a caller outside `updater.rs` can drive
  `apply_batch_scoped` without reaching into a private field. Measured on CDO
  (`u:\Git\DO.Support-SlowDOSetup\DocumentOutput\Cloud`, median of 3 runs, new
  `rung1_diagnostics_wall_clock_on_cdo` test): per-save diagnostics cost `compute_all`
  11.8 ms → `rung1_cover` + `compute_for_files` 0.21 ms (−98%). End-to-end rung-1 save
  (apply + diagnostics), measured through this task's own harness (`apply_batch_scoped`,
  which pays real `classify`/fs-read/re-parse overhead Task 1's own direct-call harness
  bypassed): apply 11.35 ms + diagnostics 11.8 ms ≈ 23.15 ms → apply 11.35 ms +
  diagnostics 0.21 ms ≈ 11.56 ms (measured 11.73 ms). Combined with Task 1's own
  warm-context apply number (4.83 ms, `rung1_rung2_wall_clock_on_cdo`): ≈16.6 ms →
  ≈5.04 ms end-to-end, matching this arc's original ~5-6 ms target. North-star
  `aldump --program-call-graph-stats` JSON on CDO remains byte-identical, SHA-256
  `0a3b85bc832ff0a3e77acee118d203edbf62827dc37617c8d9315fe52d5cb7d0` (no resolver
  change).
- Rung-1 (`apply_rung1_core`, `src/lsp/updater.rs`) no longer rebuilds `decl_by_id` and
  `incoming` wholesale on every 1-file body-only edit. Both are now patched
  incrementally: only the touched file's OLD/NEW declarations and edge targets are
  diffed against the current indexes. `decl_by_id` uses a new duplicate-safe
  multiplicity refcount (`build_decl_multiplicity`, `Updater::decl_multiplicity`) so a
  `RoutineNodeId` declared identically in two workspace files (a real, if rare,
  cross-file duplicate) is never wrongly evicted when only one of its declaring files
  is edited — the winner is re-derived from `decls_by_file` on eviction instead of
  assumed. `incoming` is patched via a new `edge_targets`/`push_edge_targets` pair
  operating on the touched file's edge diff only. `LspSnapshot::publisher_fanout` is now
  `Arc<HashMap<...>>` and forwarded (not rebuilt) at rung 1. `EdgeRef.file` changed from
  `String` to `Arc<str>` (one alloc per distinct file per rebuild instead of one alloc
  per edge). `apply_rung1_core`/`apply_batch_scoped` now additionally return a
  `Rung1Delta { files, affected_ids }` describing exactly what changed, for a future
  incremental diagnostics patch. The H-10 doc contract (`src/lsp/snapshot.rs` module
  doc) is amended: `decl_by_id`/`incoming`/`publisher_fanout` are wholesale-rebuilt at
  rung 2/3/`build_full`, but patched in place at rung 1.
  Measured on CDO (`u:\Git\DO.Support-SlowDOSetup\DocumentOutput\Cloud`, median of 3
  runs via `rung1_rung2_wall_clock_on_cdo`): rung 1 (warm context, swap excluded)
  21.19 ms → 4.83 ms. `cargo bench --bench lsp_pipeline` corroborates on the synthetic
  1000-file corpus: `rung1_body_edit_scoped_1000_files` 6.35–6.72 ms (−63–66%),
  `rung1_body_edit_1000_files` (full `apply_batch`) 26.9–28.3 ms (−26–31%). Rung 2 is
  unaffected (~760 ms before/after — `apply_rung2` still rebuilds `decl_multiplicity`
  wholesale, as documented). North-star `aldump --program-call-graph-stats` JSON on CDO
  remains byte-identical, SHA-256
  `0a3b85bc832ff0a3e77acee118d203edbf62827dc37617c8d9315fe52d5cb7d0`.
- digest per-root loop now skips duplicate effect facts (merge-identity dedup, measured
  ~2.3× duplication on CDO): a fact identical in every consumed field to an earlier one
  contributes a provable no-op to the effect map, so its witness BFS + projection +
  merge are skipped entirely. Byte-identical (fc-verified). transaction-integrity
  preset: 63.6 s → 24.7 s.
- digest per-root merge loop de-quadratized: O(1) HashMap index over the insertion-ordered
  effect map (was an O(F) scan per fact → O(F²) on 1500-fact Page roots) + `query_hops_json`
  memoized per path instead of re-serialized on every merge sort/dedupe. Byte-identical
  (fc-verified on CDO). transaction-integrity preset: 94.1 s → 63.6 s.
- `digest_query` (the L5 witness/ordering digest behind `--preset transaction-integrity`'s
  d47/d48/d49 and the digest CLI) now processes roots in parallel (rayon). Each root's
  witness reconstruction + ordering pass reads only immutable inputs (`snap`/`idx`/context
  maps) and is fully independent; the existing final sort by `routineId` keeps output
  byte-identical regardless of scheduling order. CDO `transaction-integrity` preset:
  ~1061 s (sequential, Task-1 commit `f71b8d1`) → ~88.7 s (parallel, this commit) — ~12x.
  Verified byte-identical JSON output aside from the (expected, non-deterministic)
  `generatedAt` timestamp.
- `compose_snapshot`'s capability-fact materialization parallelized per routine and its
  canonical sort switched to `sort_by_cached_key` (key built once per fact instead of per
  comparison; ~128k facts on CDO). Byte-identical (fc-verified). Full CDO
  transaction-integrity preset: 24.88 s → 23.09 s.
- witness/digest optimization close-out (`docs/perf-regression-t3-vs-0.9.3.md` §12):
  final CDO measurement at branch tip, `transaction-integrity` preset 94.12 s → 22.90 s
  (median of 3 fresh-process runs), default `analyze` unchanged (7.31 s → 6.53 s, within
  noise). Byte-identical (fc-verified vs the pre-branch `--deterministic` baseline).
  Candidate F (inner-root parallelism over the giant Page roots) is a NO-GO: the <30 s
  target was met with ~7 s of headroom, so its added complexity isn't justified; it
  remains a documented backlog lever if a future workload regresses the preset.
- The rung-1 bench + release perf gate now also measure the PRODUCTION
  scoped-context path (`Rung1Context` + `Updater::apply_batch_scoped`,
  extracted from `spawn_updater`'s hot loop so bench and server share one
  code path); the old `apply_batch` bench remains as the worst-case.
- Deleted `LspSnapshot.dep_decl_by_id` — dependency decl lookups
  (`decl_and_text`) are now served directly from the `dep_meta` frozen tier
  via a borrowed `DeclView`, removing a fully redundant ~126k-entry map
  (~103 MB steady-state RSS on a CDO-scale workspace) and the O(all-dep-decls)
  `build_dep_indexes` decl pass (~150-200 ms of every cold start / rung-3
  rebuild). LSP responses are byte-identical (both maps were built from the
  same frozen `RoutineMeta` source).
- `callHierarchy/incomingCalls` now groups call sites by caller in a single
  pass (previously O(refs²) re-filtering with a string-hashed edge lookup per
  caller×ref pair) — measured ~21.46 ms → ~4.02 ms on the 999-way fan-in
  bench; output unchanged.
- Cold-start regression from the owned-DeclSurface arena drop DIAGNOSED and FIXED (docs/perf-regression-t3-vs-0.9.3.md §9): phase instrumentation attributed the +0.5s to the SYNCHRONOUS drop of the ~10,727 dependency parse arenas on the critical path (~500ms — the old pipeline retained them, never dropping at startup) plus back-to-back build+freeze_dep_tier of ~127k entries (~305ms). Fix: (1) drop the dependency arenas on a detached background thread off the critical path (~500ms -> ~50µs handoff); (2) fuse DeclSurface::build+freeze_dep_tier into single-pass DeclSurface::build_split (~305ms -> ~190ms). Same-session A/B: LSP cold start 3.44s -> 2.82s (-18%), restoring the pre-branch base (~2.78s); steady-state RSS unchanged (~750 MB). Zero goldens; parity suite green.
- LSP steady state no longer retains dependency parse arenas: the updater keeps only the workspace ParsedUnit; the frozen dep DeclSurface tier, dep_decl_by_id and dep_texts are Arc-forwarded across rungs 1/2 and rebuilt only at rung 3.
- Resolution decl lookups migrated from the borrowed BodyMap<'a> to the owned DeclSurface; BodyMap deleted. No behavioral change (goldens unchanged).
- Measured impact of the owned-DeclSurface arena drop (docs/perf-regression-t3-vs-0.9.3.md §8): LSP steady-state RSS -54% (1,584 MB -> ~726 MB) on the DO.Support-SlowDOSetup Cloud workspace, at the cost of a +19% cold-start regression (2.87s -> 3.42s) initially hypothesized as the eager RoutineMeta projection build; rung2_signature_edit bench -25% (~149.93ms -> ~113ms), rung1_body_edit flat after accounting for machine variance. (The cold-start regression was later re-attributed to the synchronous dep-arena drop and FIXED — see §9 and the top Changed bullet.)

### Added
- DeclSurface: owned two-tier routine-decl metadata surface (workspace tier +
  Arc-frozen dependency tier), groundwork for dropping dependency parse
  arenas from LSP steady state.
- **`tests/lsp_differential.rs`: adjudicated legacy-vs-new differential parity
  harness (T3 LSP-migration arc, Task 14) — the DELETION LICENSE for the
  legacy LSP pipeline (Task 17): runs BOTH backends in-process over identical
  request scripts (`prepareCallHierarchy`/`incomingCalls`/`outgoingCalls`/
  `codeLens`, plus the `unused-procedure` diagnostic) driven by the union of
  legacy's `CallGraph::iter_definitions()` and new's `decls_by_file`
  identities, normalized to UTF-8 byte columns on both sides (legacy already
  serves bytes; new driven with `PositionEncoding::Utf8`). Every divergence
  is classified: `Match`; `Regression`/`NewUnexplained` (gates, must be 0);
  or one of 11 mechanically-justified `NewBetter` classes — the brief's 9
  (`CaseFoldHit`, `CrossAppTarget`, `DepSourceSpan`, `EventDirectionMoved`,
  `AbiSymbolShape`, `OutgoingCardinality`, `R2Precision`,
  `R6InterfaceExclusion`, `ObjectIdAdditive` — pinned at 0, out of this
  driver's scope) plus 2 discovered and adjudicated during implementation:
  `UnqualifiedCallResolved` (legacy's `outgoing_calls` renders EVERY
  unqualified call — same-object bare call or a global/builtin bareword
  call — through an unconditional, self-documented `"(local)"` placeholder,
  `data: None`, positioned at the call site, never actually attempting
  resolution; new correctly resolves or correctly omits) and
  `OverloadIdentityCollapsed` (legacy's `QualifiedName`-keyed
  `definitions`/`incoming_calls`/`outgoing_calls` have no signature
  component at all — an overload set collapses to ONE last-write-wins slot,
  silently merging every overload's callers together and sometimes pointing
  at the WRONG overload's position entirely; new's `RoutineNodeId`
  distinguishes every overload via `params_count`/`sig_fp`). CDO run
  (env-gated, `CDO_WS`/`ENFORCE_CDO_WS`) additionally exercises the H-10
  edit scenario: legacy `reindex_file` of one file loses cross-file
  incoming edges to it while new's `apply_batch` of the identical no-op
  save keeps them (`NewBetter(H10Repair)`, the 11th class,
  edit-scenario-only). Always-on fixture corpora: the existing
  `tests/fixtures/lsp-incr/` (Task 10's fixture — overloads, events,
  Unicode) plus two new ones built for this task —
  `tests/fixtures/lsp-diff-core/` (an interface, a case-mismatched call, a
  misdirected event subscriber, a well-formed pub/sub pair, an overload
  set, a two-call-site caller) and `tests/fixtures/lsp-diff-deps/` (a real,
  committed `.alpackages/` pair reused from `tests/r2-5a-fixtures/`: a
  SymbolOnly dependency and a genuine embedded-source dependency, each
  called from the workspace). `src/handlers.rs`'s private `code_lens`
  widened to `pub` (same T0.5 precedent as `prepare_call_hierarchy`/
  `incoming_calls`/`outgoing_calls`) so the harness can drive it directly.
  Diagnostics comparison is scoped to `unused-procedure` only — legacy's
  code-quality diagnostics live in a private function in the binary-only
  `src/server.rs`, structurally unreachable from an integration-test crate,
  and relocating it purely for a scaffolding test that dies with legacy at
  Task 17 was judged not worth the effort.
  **CDO fix-wave** (the controller's first real CDO run, 23 REGRESSION/
  unexplained findings, decomposed into exactly 2 mechanical classes):
  (1) `OverloadIdentityCollapsed` renamed and GENERALIZED to
  `LegacyIdentityCollapse` — legacy's `object_types`/`definitions`/
  `incoming_calls`/`outgoing_calls` are keyed by bare `(object NAME text,
  routine NAME text)` only, no object KIND or enclosing-member component at
  all, so the collision isn't limited to same-file arg-count overloads (the
  original, narrower scope) — CDO's `PAGE 6175343 "CDO E-Mail"` and
  `CODEUNIT 6175280 "CDO E-Mail"` sharing routine names collide too, across
  entirely different files and object kinds. Detection is now workspace-
  GLOBAL: every identity is queried against legacy independently (the prior
  per-file "skip the non-primary overload, never query legacy for it"
  construction-time shortcut is gone), and the classifier cross-references
  a mismatched answer against every OTHER declaration sharing the same
  legacy identity key, anywhere in the workspace. New fixture:
  `tests/fixtures/lsp-diff-identity/` (a same-named page+codeunit pair, and
  a table with two different fields' same-named `OnValidate` triggers).
  (2) `DepSourceSpan`'s predicate widened: it required legacy's and new's
  reported APP NAMES to agree, which real CDO data (`LogMessage`/
  `ToBase64`/`FromBase64`, 7 findings) showed doesn't always hold — legacy's
  and new's independent app-attribution logic can disagree on which
  declaring app "owns" a transitively-visible symbol even for the identical
  target; the robust check is the ROUTINE NAME instead (already known to
  match at that point in the classifier). A genuine double-classification
  bug surfaced and was fixed along the way: `classify_incoming`'s raw
  per-caller item-count heuristic (`OutgoingCardinality`'s incoming-axis
  counterpart) was ALSO firing on a collided identity's inflated legacy
  count, misreporting an already-`LegacyIdentityCollapse`-explained
  divergence under the wrong class — it now skips entirely for any
  identity `is_legacy_identity_collision` reports as collided. CDO class-
  count pins are TODO named constants in `cdo_workspace_has_zero_regressions_and_zero_unexplained`
  pending the controller's next CDO re-run (the old 23-finding
  decomposition is stale — the classification mechanism changed).
  **CDO layer-2 fix-wave** (re-run after the above: the previous 23 stayed
  correctly classified plus the H-10 scenario, but 31 NEW incoming/codeLens
  findings surfaced): added `NewBetter::ImplicitTriggerEdge` — a record
  operation with a statically-`true` run-trigger argument (e.g.
  `Rec.Insert(true)`) implicitly fires the target table's own trigger; the
  new resolver models this as a real `EdgeKind::ImplicitTrigger` edge
  (`RecordOpCtx`/`RunTrigger::True` in `src/program/resolve/applicability.rs`),
  which legacy structurally never models at all (`Insert`/`Modify`/`Delete`/
  `Rename` are builtin record methods, never user `Definition`s legacy's
  call-site resolution can connect to a trigger). Mechanical predicate:
  the incoming/codeLens site is backed by an `EdgeKind::ImplicitTrigger`
  edge, checked directly against `LspSnapshot::incoming`/`edges_by_file`
  (the LSP wire shape itself carries no edge-kind marker for this, unlike
  EventFlow's `[EventPublisher]` tag). New fixture arm in
  `tests/fixtures/lsp-diff-nested/` (`ImplicitTrigger.al`/
  `ImplicitTriggerCaller.al`). The layer-2 brief's OTHER hypothesized class,
  `NestedTriggerCaller` ("legacy's `ParsedFile` projection never captured
  nested field/action/dataitem-scoped triggers as caller definitions"), was
  built and run against 4 reproduction shapes (table field, page action,
  page field, and report dataitem triggers, each with a bareword call
  inside — `NestedTable.al`/`NestedPage.al`/`NestedPageField.al`/
  `NestedReport.al`) and DID NOT REPRODUCE: legacy's `collect_routines`/
  `parse_file_ir` walk the object subtree unconditionally regardless of
  nesting depth and correctly attribute calls inside every one of these
  trigger bodies. Per the project's own "measure the population before
  building taxonomy for it" doctrine, no predicate/class was added for a
  hypothesis that didn't reproduce; the fixture is kept as the permanent
  falsification record (and a regression guard for the 4 shapes it proves
  correct). Some of the CDO layer-2's 31 findings may be genuinely
  DIFFERENT, unexplained phenomena (e.g. a plain-procedure caller like
  `gethtml`/`getplaintext`) not covered by either class here — flagged for
  the controller's own individual investigation, not stretched into an
  existing predicate.
  **CDO layer-2b fix-wave** (the controller confirmed a concrete repro for
  the residual, reconciling the layer-2 falsification: CDO `Page 6175306
  "CDO E-Mail Template Lines"`, `SourceTable "CDO E-Mail Templ. Line
  Report"` — an action's `OnAction`/a page trigger's bareword call whose
  TARGET lives on the bound SourceTable, not the page itself —
  CROSS-OBJECT, unlike the layer-2 falsification fixtures' SAME-OBJECT bare
  calls, which is exactly why those matched and this doesn't): added
  `NewBetter::ImplicitRecResolved` — a `Page`/`PageExtension`/`Report`/
  `ReportExtension` trigger's bare (or explicit `Rec.`/`xRec.`-qualified)
  call resolved cross-object via the caller's IMPLICIT SourceTable binding.
  Legacy's bare-call resolution (`src/graph.rs`'s `resolve_call`) is
  structurally same-object-only (`QualifiedName{object: caller_qname.object,
  ..}`, unconditionally); for the qualified form, `Rec`/`xRec` are a
  LANGUAGE-level implicit binding `lookup_variable_type` never sees as a
  declared local variable for a page/report scope — so this is invisible to
  legacy's incoming-call index regardless of call syntax. Mechanical
  predicate: a Call-kind (not `ImplicitTrigger`) new-only incoming/codeLens
  site whose caller's object differs from the callee's, the caller's
  object KIND is Page/PageExtension/Report/ReportExtension, and the
  call-site TEXT (read from the caller's own source, since `LspSnapshot`
  carries no dedicated marker for this) is bare or `Rec.`/`xRec.`-qualified.
  A companion diagnostics-axis fix: `classify_diagnostics`'s `CaseFoldHit`
  check now defers to `ImplicitTriggerEdge`/`ImplicitRecResolved` first,
  closing a misclassification the layer-2 fix-wave's own codeLens fix
  (same root cause) had already caught for that axis but not yet for
  `unused-procedure`. New fixture arm `ImplicitRecTable.al`/
  `ImplicitRecPage.al`, kept in the SAME `tests/fixtures/lsp-diff-nested/`
  directory as the falsification fixtures for direct contrast (same-object
  bare calls match; cross-object ones don't).
  **CDO layer-3 fix-wave** (41 NEW incoming/codeLens/diagnostics findings,
  all plain-procedure callers with RECEIVER-QUALIFIED calls; concretely
  confirmed against CDO `Codeunit 6175274 "CDO Continia Online PDF Mgt"`'s
  `procedure MergePdf(var DOFile: Record "CDO File"; ...)` calling
  `DOFile.IsPdf()`, where `DOFile` is a `var` PARAMETER): added
  `NewBetter::VariableReceiverResolved` — an ordinary receiver-qualified
  call (any object kind, unlike `ImplicitRecResolved` above) whose receiver
  is a `var` parameter (or another local/temp shape legacy's variable
  tracking misses), resolved cross-object by the new engine. Root cause
  confirmed by reading `src/parser.rs`/`src/indexer.rs`: legacy's
  `variable_bindings` map is populated EXCLUSIVELY from a routine's
  `var`-section LOCALS (`push_variables_ir(&mut result, &r.locals, ...)`,
  `src/parser.rs:293`, driven by `src/indexer.rs`'s
  `add_variable_binding` loop) — the routine's PARAMETER list (`r.params`,
  a structurally separate IR field used elsewhere only to compute
  `parameter_count`) never flows into it at all, so `lookup_variable_type`
  can never type a parameter receiver, while a LOCAL `var`-section variable
  receiver of the identical record type resolves correctly today (verified
  empirically, not assumed — see the fixture below). Mechanical predicate:
  a Call-kind new-only incoming/codeLens/diagnostics site whose caller's
  object differs from the callee's, is receiver-qualified, and whose
  receiver token is NEITHER `Rec`/`xRec` (already claimed by
  `ImplicitRecResolved`) NOR (case-insensitive, quote-normalized) the
  callee's own object display name (an object-name-qualified call legacy
  CAN resolve via `object_types`, so it never reaches this class at all).
  A companion span-shape correctness fix: the call-site text-sniffing
  helper previously assumed a call's span always starts exactly at the
  callee identifier (true for bare calls, per layer-2b) — real CDO data
  showed a member call's span covers the WHOLE reference expression
  INCLUDING the receiver (`DOFile.IsPdf()`'s span was 14 columns, not
  just `IsPdf()`'s 5). `call_site_receiver` now parses the site's own
  text directly rather than inferring shape from what precedes it,
  correctly handling both shapes without needing to know which one a
  given site is in advance; `ImplicitRecResolved`'s bare-call check was
  re-verified against this fix and needed no change. New fixture arm
  `VariableReceiverTable.al`/`VariableReceiverCaller.al` in the SAME
  `tests/fixtures/lsp-diff-nested/` directory, deliberately pairing a
  `var` parameter receiver (diverges) with a local `var`-section variable
  receiver of the same record type (matches cleanly) to empirically prove
  the gap is parameter-specific, not "any variable receiver."
  **CDO layer-4 fix-wave** (re-run after layer 3: 41→35 findings; the
  predicate itself was CORRECT but OVER-RESTRICTED): GENERALIZED
  `VariableReceiverResolved` by dropping its `caller object != callee
  object` requirement entirely, per 3 concrete same-object CDO
  counterexamples the controller verified: `Codeunit 6175324 "CDO XML
  Node"`'s `AddNode(var NewXmlNode: Codeunit "CDO XML Node" ...)` calling
  `NewXmlNode.SetXmlNode(...)` (a codeunit's `var` parameter of its OWN
  type calling itself); `Table 6175301 "CDO File"`'s `MergeWithPdf`
  calling `PDFDocument.IsPdf()` where `PDFDocument` is `Record "CDO File"`
  (same shape, record-typed); and `Table 6175330`'s `GetPlainText` calling
  `Rec.GetHTML()` (a table's OWN implicit `Rec.`-qualified self-call —
  `Rec`/`xRec` are no longer excluded from this class either, since the
  mechanism is the receiver TOKEN legacy never modeled, not object
  identity; a cross-object Page/Report bare-or-`Rec.`-qualified call is
  still claimed by `ImplicitRecResolved` FIRST in the classifier chain, so
  there is no double-classification). Also independently confirmed (via a
  temporary probe, BEFORE writing any layer-4 code) that a `var`
  Codeunit-typed variable's `.Run()` dispatching to `OnRun` — the
  controller's 4th flagged row, CDO's `.dependencies/cdo/.../
  cdoqueuemanagement.codeunit.al::cdo queue management.onrun: new
  caller=sendqueue` — needed NO dedicated `EdgeKind::Run` handling at all:
  `resolve_member`'s `Run`-on-Codeunit special case
  (`src/program/resolve/resolver.rs`) produces an ordinary `EdgeKind::Call`/
  Member-shape edge, so the existing (even pre-generalization,
  cross-object) receiver check already caught it. New fixture arms in the
  SAME `tests/fixtures/lsp-diff-nested/` directory:
  `SameObjectVariableReceiver.al` (the codeunit self-call),
  `VariableReceiverTable.al` extended with `MergeWithSelf`/`GetPlainText`
  (the two same-object table shapes), and `RunDispatchTarget.al`/
  `RunDispatchCaller.al` (the Run-dispatch regression guard — proven to
  need no fix, kept as a permanent guard). TDD calibration confirmed the
  generalization is what fixes the 3 same-object cases: temporarily
  restoring the old cross-object restriction reproduced exactly those 3 as
  `NewUnexplained`, nothing more or less.
  **CDO GATE HOLDS (capstone)**: the controller re-ran the full suite on
  the real 551-file Continia (CDO) workspace at commit `ebad1b9` (the
  layer-4 generalization above) — **8/8 differential tests green,
  `REGRESSION=0`, `NEW_UNEXPLAINED=0`, H-10 edit scenario green.** This is
  the deletion license Task 14 exists to produce: every one of 55,216
  total findings on a real production BC workspace is either an exact
  `Match` (12,089) or mechanically justified by a named, fixture-proven
  class — `UnqualifiedCallResolved` (36,971 — legacy's blanket
  `"(local)"` placeholder for every unqualified call, i.e. legacy never
  even attempts resolution for these), `VariableReceiverResolved` (2,169
  — calls through a variable/parameter receiver legacy's
  `variable_bindings` never bound), `LegacyIdentityCollapse` (1,625 —
  legacy's bare name-only keying collides same-named routines across
  different objects into ONE slot, producing WRONG answers, not just
  missing ones), `ImplicitTriggerEdge` (989), `ImplicitRecResolved`
  (778), `OutgoingCardinality` (430), `R2Precision` (68),
  `EventDirectionMoved` (47), `R6InterfaceExclusion` (14),
  `CaseFoldHit`/`CrossAppTarget`/`DepSourceSpan` (12 each), with
  `AbiSymbolShape` and `ObjectIdAdditive` both confirmed at 0. All
  `CDO_PINS` entries in `cdo_workspace_has_zero_regressions_and_zero_unexplained`
  are now exact ratchets (`Some(n)`, no `None` left) at these values — a
  future change moving any of them must be explained, never
  blind-updated.
  **Review fix-wave (Opus adversarial audit, 6 required fixes + 3 LOW +
  a DOC gap)**: the audit judged the license SOUND but conditional.
  **HIGH-1, arc-critical**: `run_sweep` queried legacy with the
  LOWERCASED cross-engine matching key (`relativize`), but legacy's own
  `path_cache` (`graph.rs`'s `get_shared_path`) indexes
  `Definition.file` under its REAL case on any platform where
  `protocol::normalize_path` isn't itself lowercasing (Linux, CI's
  `ubuntu-latest`) — every fixture file here is capitalized, so a
  lowercased query path missed the exact-match `path_cache` lookup
  entirely, `get_definitions_in_file` silently returned `&[]` for EVERY
  identity, and the whole differential run panicked on CI, meaning the
  "always-on CI arm" backing this deletion license never actually ran
  there. Invisible on Windows: `normalize_path` already lowercases the
  whole path at legacy's OWN index time there, erasing the case
  difference before either code path could observe it — which is
  exactly why this slipped past every fixture run in this dev
  environment across the whole task. Fix: added `relativize_case_preserving`
  (a `relativize_raw` shared core, minus the final lowercase step) and
  a new `RoutineIdentity.file_rel_case` field carrying the real
  case-preserving path (identity KEYING still uses lowercased
  `file_rel` only); `run_sweep`'s per-identity query loop and the CDO
  H-10 test (which had the identical bug independently) now build their
  legacy-query path from it. **HIGH-2**: `R2Precision` was an
  unconditional blanket over EVERY new-only diagnostics finding — now
  gated on positive evidence (`routine_has_event_subscriber_attribute`,
  reading the owned IR's `RoutineDecl.attributes` directly) that the
  flagged routine actually carries a real `[EventSubscriber(...)]`
  attribute. **MED-1**: `EventDirectionMoved`'s sibling cross-reference
  matched by BARE NAME workspace-wide (endemic same-named BC handlers
  could launder a genuinely lost subscription) — now also requires the
  sibling's own object to match the object legacy's own detail string
  names. **MED-2**: `classify_outgoing_pair`'s `LegacyIdentityCollapse`
  never constrained NEW's own resolved target object, so an unrelated
  new answer could be laundered by an unrelated same-named collision
  elsewhere in the workspace — now requires new's target object to
  equal legacy's own claimed object. **MED-3**: outgoing
  `OutgoingCardinality` fired on a raw item-COUNT mismatch with no
  content check, pairing same-count items by blind positional zip
  regardless of target identity — rewritten to group by target name and
  compare per-target sets (an honest coverage gap noted: no existing
  fixture exercises this rewritten branch directly — `Zeta.CallTwice`'s
  pinned finding turned out to come from the already-correctly-scoped
  INCOMING-axis counterpart instead). **MED-4**: the CDO H-10 test's
  "new keeps them" assertion was near-vacuous (`x == max(y, x)`, always
  true unless new got STRICTLY worse) and compared different UNITS (raw
  `EdgeRef` count vs. grouped LSP item count) — replaced with a
  same-unit equality assertion against a captured pre-edit raw count;
  target selection now also requires a genuinely CROSS-FILE caller,
  matching the test's own name. **3 LOW fixes**: `is_new_event_derived_outgoing`
  no longer treats an item with EMPTY `from_ranges` as vacuously
  event-derived (`Iterator::all` on an empty iterator is `true`);
  `classify_incoming`'s per-site maps are now `Vec`-valued multimaps
  instead of silently dropping same-site duplicates via `.insert()`;
  `lens_key` uses `args.first()` instead of an unguarded `args[0]`
  (panicked on `Some(vec![])`). **DOC**: added the license's actual
  load-bearing argument (module doc + task report) — legacy's OUTGOING
  handler never attempts resolution for an unqualified call, but its
  INCOMING index (`add_call_site`/`resolve_call`, index time) resolves
  it EAGERLY and independently, so a genuine new-side resolution bug
  among the 36,971 `UnqualifiedCallResolved` calls cannot hide behind
  that class's size — it surfaces as a real `Regression` on the
  INCOMING axis instead. Verified via a structural invariant test
  (`relativize_is_lowercased_relativize_case_preserving`) plus temporary
  TDD-calibration probes for HIGH-1/HIGH-2/MED-1/MED-2 (each forced
  wrong, confirmed to fail loudly for the right reason, reverted); all
  9 differential tests (8 pre-existing + the new invariant test) stayed
  green with byte-identical pins throughout, confirming zero regression
  against any currently-covered legitimate case. CDO pins deliberately
  left unchanged pending the controller's next re-run.
  **CDO re-run findings (2 of 7 tests failed; the hardening worked)**:
  **Finding A** — 8 diagnostics-axis findings were misclassified as
  `NewUnexplained`/would-be `R2Precision`, when they're actually
  `LegacyIdentityCollapse` (legacy's collapsed (object, name) identity
  credits a genuinely-unused declaration with a COLLIDING sibling
  declaration's real callers, staying silent) — two confirmed CDO
  sub-shapes: `Page 6175343 "CDO E-Mail"`/`Codeunit 6175280 "CDO E-Mail"`
  sharing a display name (only the codeunit's procedures are really
  called), and `Table 6175301 "CDO File"`.`SetBackgroundPDF`'s two
  overloads (only one really called). Fix: `classify_diagnostics` now
  consults `Sweep::is_legacy_identity_collision` FIRST, before the
  `[EventSubscriber]`-gated `R2Precision` check, via a new
  `routine_legacy_identity_collision` helper. Two new fixture arms
  (`SharedNameTwo.al`/`SharedPageTwo.al`/`IdentityCallerTwo.al` and
  `OverloadProbeTable.al`/`OverloadProbeCaller.al`, `tests/fixtures/
  lsp-diff-identity/`) pin the diagnostics-axis collapse always-on:
  `LegacyIdentityCollapse` 8→17. **Finding B** — the H-10 test's now-honest
  `assert_eq!` failed for real: `apply_batch`'s post-edit raw incoming
  count for a no-op save came back HIGHER than the pre-edit count.
  Root-caused (NOT reclassified as a test bug) to a genuine engine gap in
  `classify_path` (`src/lsp/updater.rs`): unlike `resolve_virtual_path`
  (`src/lsp/handlers.rs`), which already falls back to a case-insensitive
  scan when an inbound URI's case doesn't match the already-indexed
  `virtual_path` key, `classify_path` had NO such fallback — a
  case-mismatched `FileSaved`/`FileRemoved` path silently produced a
  SECOND, separate map entry for the SAME physical file (`apply_rung1_core`'s
  `edges_by_file.insert` only overwrites on an exact key match;
  `apply_rung2`'s `splice_file` only replaces on an exact string match),
  and `build_incoming` then double-counted every edge whose caller lived
  in that file. Reproduced LOCALLY (no CDO needed) with a new unit test,
  `classify_path_resolves_case_mismatched_path_to_the_existing_key`
  (platform-independent, TDD RED confirmed the bug before the fix) plus a
  full-pipeline integration test using the exact same-file-caller-plus-
  cross-file-caller shape the CDO finding needed,
  `case_mismatched_no_op_save_does_not_duplicate_incoming_edges` (which,
  before the fix, reproduced the CDO report's exact numeric shape:
  `parsed.len()` 3 vs. 2). Fixed `classify_path` to match
  `resolve_virtual_path`'s existing case-insensitive-fallback pattern —
  an engine fix (Task 9), not a test change. Both new tests added to
  `src/lsp/updater.rs`'s own test module. CDO pins deliberately left
  unchanged; the controller re-runs and hands back updated numbers.
  **CDO re-run: final residual (8/9 green)** — after Findings A/B landed,
  the controller's H-10 test PASSED for real on CDO (confirming the
  `classify_path` engine fix) and the diagnostics-axis collapse closed;
  ONE residual remained: 7 new-only OUTGOING items at a single site
  (`Table 6175283 "CDO E-Mail Template Header"`.`UpdateTemplateLines`'s
  `EMailTemplateLine.Validate("Dimension Code", "Dimension Code")`) — the
  engine's implicit-trigger machinery fans a same-field `Validate` call
  out to a MULTICAST candidate set (7 distinct field-scoped `OnValidate`
  routines across the base table and its extensions, all legitimately
  firing — real AL semantics, not a duplicate-emission bug), which
  legacy never models (`Validate` is just a builtin record method to
  it). This is `ImplicitTriggerEdge` — already wired into
  `classify_incoming`/`classify_code_lens` — but never into the
  OUTGOING axis's new-only arms. Fix: `run_sweep` now ALSO records this
  routine's own file's `edges_by_file` entries backed by
  `EdgeKind::ImplicitTrigger` where `edge.from` matches this exact
  routine (no reverse "outgoing" index exists on `LspSnapshot`, unlike
  `incoming`), into a new `new_outgoing_implicit_trigger_sites` set;
  `classify_outgoing`'s two new-only arms (`([], _)` and the MED-3
  per-target-name `(0, _)`) both route through a new shared
  `push_new_only_outgoing` helper that checks this set before defaulting
  to `NewUnexplained`. New fixture arm in `tests/fixtures/lsp-diff-nested/`:
  `ImplicitTriggerTableExt.al` extends `ImplicitTrigger.al`'s table with
  a SECOND `"Amount".OnValidate` via `modify("Amount")`, and
  `ImplicitTriggerCaller.al`'s new `DoValidate` calls
  `Rec.Validate(Amount, ...)` — reproducing a genuine 2-candidate
  multicast locally (mirroring CDO's 7-candidate shape exactly).
  `ImplicitTriggerEdge` 2→8 on `lsp-diff-nested` (every finding
  individually probed before pinning — 2 outgoing + 2 incoming + 2
  codeLens for the new multicast pair, on top of the unchanged
  `Rec.Insert(true)` pair). TDD-calibrated: forced the new outgoing
  check off, confirmed the exact 2 multicast findings correctly fail as
  `NewUnexplained`, reverted. This is expected to be the LAST layer —
  the controller re-runs CDO and, if green, hands back the FINAL pin
  numbers for every class.
  **CDO GATE HOLDS — FINAL (t3.14 closes)**: the controller's final
  re-run, at commit `f5e441e`, held `REGRESSION=0`/`NEW_UNEXPLAINED=0`
  under the FULLY HARDENED predicates (positive-evidence gates, not
  blanket fallbacks — the whole point of the Opus adversarial review).
  **12,284 identical answers; 44,422 mechanically-justified divergences
  (every one a documented legacy defect); ZERO regressions; ZERO
  unexplained** (56,706 total findings) — final counts: `Match` 12,284,
  `UnqualifiedCallResolved` 37,066, `VariableReceiverResolved` 2,326,
  `ImplicitTriggerEdge` 2,034, `LegacyIdentityCollapse` 1,700,
  `ImplicitRecResolved` 825, `OutgoingCardinality` 335,
  `EventDirectionMoved` 47, `R2Precision` 39, `R6InterfaceExclusion` 14,
  `CaseFoldHit`/`CrossAppTarget`/`DepSourceSpan` 12 each,
  `AbiSymbolShape`/`ObjectIdAdditive` both 0. Deltas vs. the
  pre-hardening capstone (`ebad1b9`), with cause — this audit trail
  proves the hardening changed REAL classifications, not just prose:
  `Match` 12,089→12,284 (+195: the `classify_path` engine fix restoring
  correctly-attributed edges + the `classify_incoming` multimap fix
  restoring comparisons the old `.insert()` had silently dropped);
  `R2Precision` 68→39 (−29: the `[EventSubscriber]`-positive gate plus
  the new diagnostics-axis `LegacyIdentityCollapse` check moved these —
  29 findings had been LAUNDERED by the blanket fallback the review
  flagged); `LegacyIdentityCollapse` 1,625→1,700 (+75: the diagnostics
  axis, new); `ImplicitTriggerEdge` 989→2,034 (+1,045: the outgoing
  axis, new); `OutgoingCardinality` 430→335 (−95: target-SET comparison
  replaced the raw-count-plus-positional-zip check); `ImplicitRecResolved`
  778→825, `VariableReceiverResolved` 2,169→2,326, `UnqualifiedCallResolved`
  36,971→37,066 (real workspace re-attribution from the same review
  fix-wave's other tightenings, MED-1/MED-2); `EventDirectionMoved`/
  `R6InterfaceExclusion`/`CaseFoldHit`/`CrossAppTarget`/`DepSourceSpan`
  unchanged (untouched by this fix-wave). **Two REAL bugs this harness
  caught in the process of hardening itself**: (1) the `classify_path`
  duplicate-file-entry engine bug (`src/lsp/updater.rs`, Finding B) — a
  genuine Task-9 production defect that would have shipped; (2) the
  `R2Precision` blanket-fallback laundering gap (HIGH-2/Finding A) — a
  genuine hole in THIS harness's own deletion-license evidence, closed
  before it could hide a real divergence. All `CDO_PINS` entries in
  `cdo_workspace_has_zero_regressions_and_zero_unexplained` are exact
  ratchets at these FINAL values. Task 14 is DONE.
- **`tests/lsp_incremental_parity.rs`: dep-bearing fixture arm (T3
  LSP-migration arc, Task 14 Step 5, plan-amended)** —
  `tests/fixtures/lsp-diff-deps/` exercises the incremental-vs-batch gate
  through a rung-1 (body-only) then rung-2 (signature-change) transition on
  a workspace with a REAL embedded-source dependency, giving
  `LspSnapshot::dep_decl_by_id`/`dep_texts`/`workspace_root` (widened into
  this gate's equivalence key in the Task 10/11 review fix-waves, but
  trivially vacuous on the dep-less `lsp-incr` fixture) non-vacuous
  coverage for the first time: a real `dep_decl_by_id` entry for `Source
  Mgt.DoWork`, asserted byte-identical across both rungs (the dep layer is
  never touched by either). A hand-built `.app` zip was judged infeasible
  to construct correctly from scratch within scope; reusing the
  already-committed, already-proven `tests/r2-5a-fixtures/` `.app` fixtures
  made the disk-based, plan-preferred arm feasible instead of the brief's
  in-memory `two_app` fallback.
- **`src/lsp/custom.rs`: engine-backed custom LSP requests (T3 LSP-migration
  arc, Task 13) — `dependency_document_symbol`, `event_publishers_in_file`,
  and `event_reference_at_position`, the program-engine replacements for
  `src/handlers.rs`'s `dependency_document_symbol`/`event_publishers_in_file`/
  `event_reference_at_position` (cut over at Task 15). `fieldProperties`/
  `actionProperties`/`telemetryStatus` are already graph-independent and were
  not touched. `dependency_document_symbol`/`event_reference_at_position`
  read `LspSnapshot::snap.apps[].abi: Option<ParsedAppPackage>` — the SAME
  `.app`-derived data (`crate::app_package`/`crate::dependencies::
  load_all_apps`) legacy's `graph.dependency_objects` was built from —
  rather than `ProgramGraph`'s `RoutineNode`/`ObjectNode` nodes, giving
  byte-identical, full-parameter-name signatures for every dependency
  regardless of trust tier (a `RoutineNode`'s ABI-tier parameter metadata
  deliberately drops parameter names — `AbiParamRetained`'s "MINUS
  name/is_temporary" — and carries none at all for an embedded-source
  dependency, so building from graph nodes would have meant a reduced-fidelity
  synthesized signature). `event_publishers_in_file` reads a workspace file's
  retained `AlFile` IR directly (`ParsedFileEntry.file`, no re-parsing, no
  disk I/O) and renders each publisher's signature via a local
  string-literal-aware raw-text header scan (byte-identical output to
  `parser.rs`'s `signature_ir`, duplicated rather than shared since
  `parser.rs` is a documented Task-17 deletion target). Two additive,
  parity-safe deltas from legacy: numbered-object lookup via `object_id` now
  actually resolves (legacy parses but never reads that field), and no disk
  I/O (reads the in-memory snapshot text instead of re-reading the file from
  disk, so it can't race a live unsaved editor buffer).
- **`src/lsp/lens.rs` + `src/lsp/diagnostics.rs`: codeLens + a diffing
  diagnostics engine on the program engine (T3 LSP-migration arc, Task 12) —
  the engine-backed replacements for `src/handlers.rs`'s legacy graph-backed
  `code_lens`/`get_unused_procedure_diagnostics` and `src/server.rs`'s
  publish-once-stale `get_code_quality_diagnostics` (cut over at Task 15).**
  `code_lenses` emits one lens per declaration in a file (procedures,
  triggers, everything `decls_by_file` carries — unfiltered by kind, mirroring
  legacy's `get_definitions_in_file`), delegating complexity/parameter-count
  to the SAME owned-IR walker the `--analyze` CLI path uses
  (`analysis::routine_complexity_ir`) rather than re-deriving metrics.
  `diagnostics::compute_all` ports the full unused-procedure rule set
  (`src/indexer.rs:159-218` + `src/graph.rs:865-905`) onto engine data — the
  new `effective_incoming_count` helper generalizes legacy's
  `get_incoming_call_count` (direct calls + event-subscription count) onto
  the engine's edge model, which SUBSUMES the old `[EventSubscriber]`
  attribute-blanket exclusion entirely: a subscriber is "used" because a real
  `EventFlow` edge targets it, so a subscription whose publisher/event name
  doesn't resolve to anything real is now correctly flagged (a precision
  improvement over legacy's unconditional attribute exclusion, proven both
  directions by tests) — while `[IntegrationEvent]`/`[BusinessEvent]`
  publishers stay unconditionally excluded (their real subscribers typically
  live in a downstream app this workspace never loads), `[InternalEvent]`
  stays un-excluded, flagged unless subscribed-or-raised, exactly as legacy
  behaved, and a NEW rule (R6, review fix-wave) excludes any routine whose
  enclosing object is `ObjectKind::Interface` — an interface method's own
  signature can never itself be a call target (dispatch always resolves to
  an IMPLEMENTING object's own routine instead), so it structurally always
  showed zero incoming under EITHER engine; legacy shared this exact
  false-positive class. Plus `DiagnosticsState::diff`, the
  recompute-diff-publish-clear engine Task 15 wires to `on_swap`: diffs a
  fresh `compute_all` result against the last published set and emits ONLY
  what changed, INCLUDING a uri whose findings dropped to zero — the clear
  behavior legacy's `publish_all_diagnostics` never had (a fixed procedure's
  stale "unused" hint would otherwise linger until the next unrelated
  finding in that file happened to overwrite it). Widened three existing
  helpers to `pub(crate)` for reuse rather than duplication:
  `lsp::handlers::{resolve_virtual_path, object_name_for, origin_to_range}`.
  **Review fix-wave:** `routine_complexity_ir`/`is_framework_invocation_attribute`
  (+ their private IR-walking helpers) were relocated from `parser.rs` (a
  Task-17 deletion target) into `src/analysis.rs`, which was itself promoted
  from a binary-only `main.rs` module to a proper library module (`pub mod
  analysis;` in `lib.rs`) so these permanent LSP modules never depend on code
  scheduled for deletion; `parser.rs` re-exports both so its own existing
  call sites keep compiling unchanged. (`program::resolve::event::is_event_publisher`,
  separately named in the same review finding, was already correctly placed
  in the permanent program-engine module and needed no change.)
- **`src/lsp/handlers.rs`: core call-hierarchy handlers on the program engine
  (T3 LSP-migration arc, Task 11) — `prepare`/`incoming`/`outgoing`, the
  engine-backed replacements for `src/handlers.rs`'s legacy graph-backed
  `prepare_call_hierarchy`/`incoming_calls`/`outgoing_calls` (cut over at
  Task 15).** `ItemData { node: RoutineNodeId }` is the `item.data` JSON
  round-trip payload; `incoming` groups `EdgeRef`s by `edge.from` into one
  `CallHierarchyIncomingCall` per DISTINCT caller (a deliberate improvement
  over legacy's ungrouped per-call-site shape); `outgoing` walks the
  routine's own edge bucket (`Call`/`Run`/`ImplicitTrigger`) plus
  `event_edges` filtered `edge.from == id` (a publisher's subscribers
  surface as outgoing targets — the design doc's "natural direction"
  decision), emitting one item per route: `RouteTarget::Routine(id)` → a
  real item (workspace OR dependency-with-embedded-source — both now get
  REAL navigable spans, legacy never could); `conditionalResolved`/
  `ambiguousResolved` candidate sets → one item per candidate;
  `RouteTarget::AbiSymbol` → a zero-range item at a synthesized
  `al-preview://` URI (matching legacy's external-def fallback SHAPE —
  identity-bearing detail + an `external`-flagged `data` blob — but
  deliberately NOT reusing the caller's own file/range as legacy did);
  `Builtin`/`Unresolved` → no item. Both non-negotiable live-span rules
  (resolver-read audit §6.1, and its Task-10 EventFlow extension) are
  enforced structurally: every position-bearing surface is re-derived from
  `LspSnapshot::decl_and_text` at query time, never from a stored
  `Route::Witness`/`SiteId` span — proven by two dedicated rung-1-edit
  regression tests asserting the returned ranges match an independent fresh
  batch build, not the pre-edit witness. **Review fix-wave:** the
  `al-preview://` URI `RouteTarget::AbiSymbol` items emit was found to only
  wear legacy's scheme by eye — legacy's own `parse_al_preview_uri`
  (`src/handlers.rs:1452-1499`, widened to `pub(crate)` for direct testing)
  structurally requires the OBJECT-level 5-segment layout
  `al-preview:///allang/{App}/{Type}/{Id}/{Name}.dal` (it anchors on `Type`
  parsing as a known `ObjectType`), which the original 3-segment
  `allang/{App}/{ObjectDisplay}/{Routine}` shape could never satisfy —
  `abi_symbol_uri` now emits the conformant 5-segment layout (falling back
  to the object number's own text for the `Name` segment when a numbered
  ABI object carries no raw display name at all), proven by a dedicated
  test that calls legacy's parser directly on the emitted URI.
- **`LspSnapshot::dep_decl_by_id`/`dep_texts`/`workspace_root` (T3 Task 11,
  extending Task 8's `LspSnapshot`)** — closes a real gap the handlers work
  surfaced: `decl_by_id` was workspace-only, so a `RouteTarget::Routine(id)`
  pointing into an embedded-source DEPENDENCY had nowhere to resolve a real
  span, contradicting the migration design doc's explicit §5 promise ("a dep
  with embedded source gets REAL navigable spans"). `build_dep_indexes`
  (new, shared by `LspSnapshot::from_context` and `Updater::apply_rung2`)
  walks `graph.routines` for every non-primary app and records each one
  `BodyMap` can still resolve, plus its file's text (keyed `(AppRef,
  virtual_path)`, since a dependency's `virtual_path` is only unique within
  its own app); rung 1 `Arc::clone`s both forward unchanged (dependency
  source cannot change on a rung that only touches workspace files).
  `workspace_root` (normalized like `uri_to_path`) lets `prepare` turn an
  inbound URI into the same `virtual_path` key `decls_by_file`/`parsed` use,
  with a case-insensitive fallback scan for `virtual_path`'s case-preserving
  keys under Windows's case-insensitive filesystem semantics.
- **Additive `Serialize`/`Deserialize` derives on `RoutineNodeId` and its
  identity chain (`src/program/node.rs`: `AppRef`, `ObjKey`,
  `ObjectNodeId`)** — needed for `ItemData`'s JSON round-trip through
  `CallHierarchyItem.data`. `ObjectNodeId.kind: al_syntax::ir::ObjectKind` is
  a foreign type in a crate that deliberately carries no serde dependency
  (`al-syntax` stays minimal by design), so a local `ObjectKindDef` mirror
  uses serde's standard `#[serde(remote = "...")]` idiom rather than adding
  serde to `al-syntax` or hand-rolling a shadow enum with manual conversion.
  Zero behavior change to any resolution path — verified by the full
  existing test suite staying green. **Review fix-wave:** `RoutineNodeId::
  sig_fp: u64` was found to serialize as a bare JSON NUMBER — an FNV-1a hash
  spans the FULL `u64` range, but JSON numbers are IEEE-754
  double-precision (exactly representable only up to 2^53), so a
  JavaScript-based LSP client's `JSON.parse(item.data)` (the near-universal
  case) would silently ROUND `sig_fp` for essentially every multi-param
  routine, corrupting `ItemData` before the follow-up incoming/outgoing
  request ever reaches this engine — a decode-then-lookup miss that fails
  closed to an empty result in every real editor, never a loud error. Now
  serialized through a decimal string (`#[serde(with = "sig_fp_as_string")]`,
  a small local module) instead, which carries the exact value losslessly
  regardless of the receiving language's number type; a dedicated test
  asserts the JSON field is a string AND a value past 2^53 round-trips
  exactly.
- **`tests/lsp_incremental_parity.rs`: the PERMANENT incremental-vs-batch
  differential gate (T3 LSP-migration arc, Task 10 — the arc's H-10
  insurance policy; outlives the arc, runs in CI forever).** 9 scripts, each
  copying the new fixture workspace (`tests/fixtures/lsp-incr/` — 2
  codeunits with an overload set and an event pub/sub pair, 1 table, 1
  tableextension, 1 page, a `Løbenr` identifier and an `æøå` line) into a
  fresh tempdir, driving `Updater::apply_batch` through one or more scripted
  disk edits, and asserting AFTER EVERY EDIT that the incrementally-produced
  `LspSnapshot` is EQUIVALENT to a completely independent `LspSnapshot::
  build_full` of the same on-disk state: a body-edit chain (3 consecutive
  rung-1 saves), a signature change, a routine rename, a brand-new file, a
  file delete, a body-only edit that flips which of two overloads a call
  site resolves to (stays rung 1 — proves arg-type dispatch re-runs
  correctly against the touched file's fresh parse, not a stale cached
  `BodyMap`), an `EventSubscriber` attribute edit, and one mixed 6-edit batch
  (add + delete + rename + signature change + 2 body-only edits, all
  coalesced into ONE `apply_batch` call). Plus a dedicated non-vacuity probe
  (`gate_non_vacuity_rung1_and_rung2_are_both_exercised`) proving, in
  isolation from every other script's state, that this suite's applies
  really do take both `Rung::One` and `Rung::Two` — a suite that silently
  rung-3'd everywhere would pass every check for a trivial, uninteresting
  reason. The equivalence key (one `canon_edges`/`canon_decls`/
  `canon_incoming` helper set every script shares) compares, per file: the
  edge MULTISET as `(ObligationId, EdgeKind, DispatchShape, SetCompleteness,
  sorted Vec<(RouteTarget, EvidenceKind, sorted Vec<Condition>)>)` tuples
  (widened in the review fix-wave below — see that entry); `event_edges`,
  same rule; `incoming`, dereferenced through each `EdgeRef` to its owning
  edge's `ObligationId` rather than compared as raw `(file, idx)` pairs;
  `decls_by_file`, as `(RoutineNodeId, name, origin, name_origin)` tuples
  (`Origin`'s EPHEMERAL `ts_id`/`kind_text` fields projected away —
  `al_syntax::ir::Origin`'s own doc: "NEVER compare across parses,
  tree-sitter recycles ids"). `Route::witness`, `Route::receiver_tier`, and
  `generation` are excluded by design (stale-witness-span /
  diagnostic-only-per-its-own-doc / monotonic-counter reasons, all
  documented in the test file's header, which now gives an exhaustive
  accounting of every compared-vs-excluded field). **Building the gate
  exactly as the
  brief's `(SiteId, ...)` key specified surfaced a REAL false-positive on
  the very first script**: an `EventFlow` edge's `SiteId` — per
  `resolver.rs`'s `emit_event_flow_edges`, explicitly "anchored at the
  publisher routine's name-origin span" (a position stand-in, since an event
  has no call expression) — goes stale after ANY rung-1 edit that shifts
  line numbers in a file declaring a publisher, because `apply_rung1_core`
  never recomputes `event_edges` (unconditional `Arc::clone`). Root-caused
  (not papered over): switched the equivalence key from raw `Edge.site` to
  `ClassifiedEdge.obligation_id` — `ObligationId::CallSite` mirrors `SiteId`
  field-for-field (zero loss for real call sites, whose spans rung 1 always
  keeps fresh) while `ObligationId::Publisher(RoutineNodeId)` carries no span
  at all, exactly matching the cosmetic-vs-identity distinction the engine's
  own coverage-contract type was already designed around. Calibration
  (binding TDD step): deliberately compared a post-edit snapshot against the
  WRONG (pre-edit) oracle on the signature-change script — confirmed the
  gate fails loudly, naming the exact diverging file and edge — before
  reverting to the correct comparison. Also widens `LspSnapshot::
  build_full_with_parsed` from `pub(crate)` to `pub` (Task 9 had it
  crate-only; this gate is an external integration-test crate and needs it
  to construct an `Updater` the same way `server.rs` eventually will).
  **Review fix-wave:** the equivalence key omitted `Route::conditions` and
  `Edge::kind`/`shape`/`completeness` entirely — an unjustified exclusion
  for a permanent CI gate (all 4 are real semantics: `conditions` gates
  `Route::fires_by_default`/`Edge::default_reachable_routes`;
  `kind`/`shape`/`completeness` are exactly what `classify_obligation`/
  `real_unknown_rate` read). Widened `CanonEdge`/`canon_edge` to include all
  4 (a new `CanonRoute` type carries each route's own sorted `Condition`
  set), and rewrote the module doc's equivalence-key section into an
  exhaustive accounting — every `Edge`/`Route` field is now either compared
  or excluded with a stated, engine-doc-grounded reason (`Route::
  receiver_tier`'s exclusion, previously implicit, is now stated explicitly
  too: its own doc already marks it diagnostic-only/never-goldens-compared).
  Added a new permanent meta-test,
  `canon_edge_distinguishes_kind_shape_completeness_and_conditions`, proving
  the widened key's discriminating power directly (4 hand-constructed
  edge pairs, each differing in exactly one of the 4 newly-added
  dimensions, must canonicalize unequal) — calibrated by temporarily
  narrowing `canon_edge` back to the pre-fix-wave shape and confirming all 4
  assertions fail (collapse to equal) before restoring. All 9 original
  scripts stayed green under the widened key (no active divergence existed
  on these dimensions; the fix closes an exclusion gap, not an active bug).
  Also made the overload-flip script (`overload_flip_body_only_edit_
  stays_rung1_and_equivalent`) prove its own claim directly rather than
  leaning on the trailing full-equivalence check: a new `calc_target_at_line`
  helper names each specific overload's `RoutineNodeId` by source line
  before AND after the edit, asserting the Text-literal call site now names
  `Calc(Text)`'s id (previously the Integer-literal site's id) and
  vice versa. **Review fix-wave (T3 Task 11):** widened the equivalence key
  a second time to ALSO compare `LspSnapshot::dep_decl_by_id`/`dep_texts`/
  `workspace_root` — the three fields Task 11 added for dependency-source
  real-span coverage, after this gate was already written, would otherwise
  have been an invisible-divergence hole in a PERMANENT gate. Trivially
  equal on this dep-less fixture today (no dependency apps); a dedicated
  dependency-bearing fixture arm giving these three comparisons real,
  non-vacuous coverage is planned as Task 14's Step 5 (plan-amended, commit
  `9e4006e`).
- **`src/lsp/updater.rs`: the incremental updater — debounced queue, the
  rung-1/rung-2/(degenerate)rung-3 soundness ladder, and atomic
  `Arc`-swap publication (T3 LSP-migration arc, Task 9 — the arc's CRUX).**
  `SharedSnapshot` (`RwLock<Arc<LspSnapshot>>`, swap-only — a query thread's
  `get()` is a cheap `Arc` clone, never blocked) + `ChangeEvent`
  (`FileSaved`/`FileRemoved`/`DepsChanged`/`Overflow`) + `Updater` (owns the
  mutable working `Vec<ParsedUnit>` — every source-bearing app, workspace
  AND embedded-source deps) + `Updater::apply_batch` (the synchronous,
  unit-tested core the brief asks for; returns `(LspSnapshot, Rung)` — the
  "expose the rung taken" test hook, more directly than the
  originally-suggested `Cell<Rung>`) + `spawn_updater` (the REAL hot-path
  thread wrapper: 100ms debounce, per-path last-wins coalescing, apply,
  swap, `on_swap` notify hook for a future diagnostics consumer). Implements
  the plan's now-MANDATORY contingency from Task 3's CDO measurement
  (`ResolveIndex`+`BodyMap` cost 200-350ms — 2-3.5x rung 1's entire 100ms
  budget): `spawn_updater`'s loop builds a `ResolveIndex`/`BodyMap`/
  `obj_node_map` context ONCE right after each swap and REUSES it across
  every consecutive rung-1 batch, rebuilding only when a rung-2/3 event
  actually changes the graph. Getting this cache-reuse to compile in safe
  Rust surfaced a real ownership conflict (a first draft that spliced each
  rung-1 edit straight into `Updater::parsed` failed to build once caching
  was wired through `spawn_updater`, because a cached `BodyMap` borrows
  `parsed` and can't coexist with a later `&mut` splice into it) — the fix
  is a `pending: HashMap<String, ParsedFile>` overlay field, DISJOINT from
  `parsed`, that rung 1 (via the new free function `apply_rung1_core`,
  deliberately NOT a `&mut self` method, so the two fields' borrows stay
  provably disjoint to the borrow checker) writes into instead;
  `Updater::flush_pending` folds it into `parsed` whenever a rung-2/3
  rebuild needs `parsed` to be current, or immediately after every
  `apply_batch` call (which always builds its OWN fresh, uncached context,
  since it's the simple/correctness-first path, not the optimized one).
  Soundness argument (recorded in the module doc): resolving a rung-1
  touched file against a STALE cached `BodyMap` is sound because the ONLY
  fields any resolution path reads through it (`RoutineDecl::params`/
  `by_ref`/`parse_incomplete`, plus a witness span never trusted stale
  anyway) are EXACTLY what rung 1's own fingerprint-equal gate already
  guarantees are unchanged. Rung 2 (a definition-surface change, file
  add/delete, or a `Recovered` parse — fail-closed, never trusted for rung
  1) flushes any pending rung-1 backlog first, then rebuilds the workspace
  layer via `assemble_program_graph` over the cached, unchanged `DepLayer`
  and re-resolves EVERY workspace file (a signature change anywhere can
  change how any OTHER file's call sites resolve). Rung 3
  (`DepsChanged`/`Overflow`, or a path outside the workspace source set
  entirely — e.g. under `.alpackages/`, the Task-4-review dep-file-boundary
  scenario) is a full rebuild via the new `LspSnapshot::build_full_with_parsed`
  (returns a SECOND, fully independent `parse_snapshot` pass alongside the
  snapshot — `AlFile` has no `Clone` impl, so the snapshot's own owned parse
  and the updater's mutable working parse can never share one `AlFile`
  instance; this second pass runs only at startup and on the rare rung-3
  path, never on the rung-1/rung-2 hot path). Batch semantics: any rung-2 (or
  rung-3) event in a coalesced batch escalates the WHOLE batch — one rebuild
  serves every file named in it. `LspSnapshot` gained `Arc`-shared `graph`
  and `snap` fields (were plain owned `ProgramGraph`/`AppSetSnapshot` —
  neither is cheaply cloneable at CDO scale) and `decls_by_file`'s per-file
  values became `Arc<Vec<DeclEntry>>` (was a plain `Vec`) so an incremental
  rebuild can share every untouched file's decl list instead of deep-cloning
  the whole map; `decl_by_id` is now explicitly documented as a DERIVED
  index (like `incoming`) always rebuilt wholesale via the new
  `build_decl_by_id`, never cloned-then-patched. `LspSnapshot::from_context`
  and the new `recompute_file` helper (used by both the Task-8 batch build
  and this task's rung 1/2 per-file recompute — the ONE place "what a file
  contributes to a snapshot" is defined) factor the Task-8 batch-build loop
  so it can never drift from the incremental path. 9 unit tests over an
  in-memory fixture workspace cover the brief's binding Step-1 scenarios
  (a)-(e) (body edit → rung 1 with the `Arc::ptr_eq` sibling-untouched
  proof; signature edit → rung 2 with the caller's route flipping to
  `Evidence::Unknown(UnknownReason::ArityMismatch)`; file delete → rung 2
  with its bucket/incoming entries gone; a `Recovered`-parse save escalating
  past rung 1; a `.alpackages`-shaped path escalating to rung 3) plus a
  fail-closed build-failure test (prev snapshot AND the updater's working
  state both survive untouched), a mixed-batch test (one rung-2 file forces
  the whole batch), a dedicated caching-property test (`apply_rung1_core`
  called TWICE with the SAME `index`/`body_map`, built only once, before
  either edit — the exact arrangement `spawn_updater`'s hot loop relies on),
  and the Step-3 debounce/coalesce test (5 rapid saves of one file via
  `spawn_updater`'s real background thread → exactly 1 apply, proven via a
  counting `on_swap` wrapper). Step 3b re-measured rung 1/rung 2 against this
  REAL code path on CDO (`.superpowers/sdd/t3-stage-split.md` addendum,
  `apply_rung1_core`/`Updater::apply_rung2` exercised directly with
  in-memory-only `ParsedFile`s — zero disk mutation to the real workspace):
  rung 1 measures ~10.5ms (10x under the 100ms budget, holding the
  cached-context design's whole point) and rung 2 measures ~1.46s — FASTER
  than Task 3's ~1.9s pre-implementation upper-bound estimate, because the
  real rung 2 only re-parses the changed file(s), never the whole workspace.
  Also fixes a Task 8 review carry-over: `build_incoming` could push a
  duplicate `EdgeRef` when one edge's `routes` named the same target more
  than once (a pathological ambiguous-overload shape) — the new
  `push_edge_targets` helper dedups per-edge (routes from a DIFFERENT edge
  naming the same target are untouched — those are genuinely distinct
  callers), pinned by a hand-constructed-`Edge` unit test (plus a mirror
  test proving 2 DIFFERENT edges to the same target stay 2 `EdgeRef`s, from
  the review fix-wave below). **Review fix-wave:** `apply_rung3` now
  `log::warn!`s (matching `server.rs`'s existing `log` idiom) when a rung-3
  rebuild fails, so a broken `app.json` isn't a silently-dropped event;
  added `spawn_updater_rebuilds_context_after_rung2_escalation`, an e2e test
  driving the real background thread through rung 1 → rung 2 → rung 1 and
  proving the final snapshot resolves correctly against the POST-rung-2
  graph; corrected a narrative overclaim in the Step 3b write-up (`Updater::
  apply_rung2`'s snapshot-copy construction redundantly re-parses every
  workspace file, not just the touched one — included in the measured
  ~1.46s, flagged as a deferred `Arc<AlFile>`-sharing optimization rather
  than fixed now).
- **`src/lsp/snapshot.rs`: `LspSnapshot`, the immutable batch-built
  program-engine snapshot the migrated LSP server will serve queries from
  (T3 LSP-migration arc, Task 8 — the arc's structural centerpiece).**
  `LspSnapshot::build_full(workspace_root)` composes every engine primitive
  landed by earlier T3 tasks into one self-contained, `Arc`-shareable value:
  `SnapshotBuilder` → `parse_snapshot` → `build_dep_layer`/
  `assemble_program_graph` (Task 5) → per-file `resolve_file_obligations`
  (Task 6) → `def_surface_fingerprint` (Task 7) → `emit_event_flow_edges`,
  then derives `incoming` (an O(E) wholesale rebuild, never incrementally
  edited — spec §3 / H-10 lesson). New public types: `EdgeRef` (an index-based
  `(file, idx)` edge handle — never a borrow), `DeclEntry` (owned routine
  identity + `origin`/`name_origin` spans), `ParsedFileEntry` (owned
  `AlFile`+text+`DefSurface`), and `EVENT_EDGES_KEY` (the reserved
  NUL-prefixed `EdgeRef.file` bucket for whole-program `EventFlow` edges, kept
  separate from the workspace-scoped per-file `Call`/`Run`/`ImplicitTrigger`
  buckets in `edges_by_file`). `LspSnapshot::decl_at` does position lookup
  (file + 0-based line + UTF-8 byte col → the routine whose `name_origin`
  contains it, preferred, else whose whole-decl `origin` contains it, using
  `Origin`'s own byte-column semantics directly — no encoding conversion
  needed) and `LspSnapshot::edge` resolves an `EdgeRef` back to its
  `ClassifiedEdge`. `ResolveIndex`/`BodyMap`/the `ObjectNodeId → &ObjectNode`
  map are built TRANSIENTLY inside `build_full` and never stored on the
  struct (they borrow `graph`/`parsed`, which would make `LspSnapshot`
  self-referential) — `AlFile` has no `Clone` impl, so `build_full` is
  structured in two phases: a borrow phase (index/body_map alive) that
  resolves every file and computes each `DefSurface`, then an ownership-move
  phase (after the borrows drop) that consumes the parsed workspace unit by
  value to build `parsed: HashMap<String, Arc<ParsedFileEntry>>` without
  cloning any `AlFile`. Enabling change to `program::resolve::full`: `
  ProgramContext`/`build_context` widened to `pub(crate)`, and `build_context`
  now inlines `build_dep_layer` + `assemble_program_graph` directly (rather
  than calling the `build_program_graph_from_parsed` wrapper) so the
  `DepLayer` it assembles from survives into `ProgramContext` — behavior-
  preserving (the wrapper does exactly these two calls, in this order),
  verified by the pre-existing `assemble_program_graph_matches_build_
  program_graph_field_by_field` characterization test plus the whole `cargo
  test --lib` suite staying green. 4 unit tests over an in-memory fixture
  workspace (a cross-file call via a declared `Codeunit "Beta"` local var, a
  same-name overload pair, an event publisher/subscriber pair, and a
  non-ASCII Danish identifier, `Løbenr`, to exercise UTF-8 byte-column
  handling): `build_full`'s `edges_by_file` + `event_edges` union equals a
  direct `resolve_full_program` run (order-insensitive, via `Edge`'s derived
  `Ord`); determinism across two independent builds (`generation` excluded);
  `decl_at`'s name-hit/whole-decl-fallback/none paths; `build_incoming`
  finding both the cross-file caller and the event subscriber's publisher.
- **`src/lsp/def_surface.rs`: the definition-surface fingerprint,
  `DefSurface`/`def_surface_fingerprint` (T3 LSP-migration arc, Task 7).** A
  blake3 hash — canonically encoded, length-prefixed strings/lists, no
  `format!`-glue — of every field Task 4's resolver-read audit
  (`docs/superpowers/specs/2026-07-12-t3-def-surface-audit.md` §4) found
  reachable from another file's call-graph resolution: per-object identity/
  `extends_target`/`implements`/`SourceTable`+`temporary`/`TableNo`/
  `page_controls`/`fields`/`dataitems`/file-level `parse_incomplete`, and
  per-routine identity/`access`/`event_subscribers`/
  `subscriber_instance_manual`/`publisher_kind`/`include_sender`/
  `return_type`/`param_sig_key`/per-parameter `(ty, by_ref)`/routine-level
  `parse_incomplete`. Reuses `program::extract_nodes` (the SAME extractor
  `program::build` calls) rather than a second hand-rolled IR walk, so the
  fingerprint's notion of "the surface" can never drift from the graph's
  own. One deliberate ADDITION beyond the audit's literal §4 text: an
  object's `name` (lowercased) is also hashed per object, even for a
  NUMBERED object whose §4 identity key is `ObjKey::Id` — the audit's own
  §2.2 read-table already lists `ObjectNode::name` as consulted
  (`graph.rs`'s `ObjectIndex::build` keys `graph.resolve_object`'s by-name
  lookup on it for every object kind), so omitting it from §4's derived list
  would have been a false-negative gap (a numbered-object rename resolving
  differently elsewhere without moving the fingerprint) — flagged back for
  the audit doc, and CONFIRMED by an independent review (which verified
  `graph.rs`'s `ObjectIndex::build` keys by-name unconditionally and
  `resolve_object` never consults `declared_id`); the audit doc itself
  (`docs/superpowers/specs/2026-07-12-t3-def-surface-audit.md` §4) was
  patched in the same review fix-wave to record object `name` in its
  per-object field list, with the confirmed evidence. 33 unit tests: 1
  parse-twice determinism check, 24 NOT-EQUAL pairs (one per audited
  field-change class: object add/re-id/rename, `extends_target`,
  `implements`, `SourceTable`+`temporary`, `TableNo`, `page_controls`, table
  `fields` add/type-change, report `dataitems`, file-level
  `parse_incomplete`, routine add/rename/re-arity/param-type-change,
  `by_ref` flip, `access`, `return_type`, `event_subscribers`,
  `subscriber_instance_manual`, `publisher_kind`, `include_sender`), 5
  EXCLUSION pairs proving an out-of-scope field never moves the fingerprint
  (body-only statement, local variable, comment/whitespace-only span shift,
  an added enum value, a parameter's NAME), and 3 review fix-wave additions
  deliberately constructed to isolate a field the review found NO existing
  test actually exercised (verified by a temporary deletion-probe on each:
  neutering the write made ONLY the new test fail, confirmed then
  restored) — a case-only param type-text change (isolates the raw
  per-parameter `ty` read from `sig_fp`/`param_sig_key`, both of which
  normalize case away), a routine-level `parse_incomplete` flip held
  against a permanently-`Recovered` file (isolates the per-routine flag
  from the file-level one), and an id-less object rename (exercises
  `ObjKey::Name`, previously only `ObjKey::Id` was covered).
- **`benches/engine_stages.rs`: program-engine stage-split Criterion bench +
  a CDO-gated stage-split unit test (T3 LSP-migration arc, Task 3 —
  MEASUREMENT ONLY, no engine behavior changed).** Splits the program
  engine's pipeline (`aldump --program-call-graph-stats`'s path) into
  timed stages — snapshot (`SnapshotBuilder::build`), parse
  (`parse_snapshot`), graph build (`build_program_graph`),
  `ResolveIndex::build`, `BodyMap::build`, and the obligation-resolution
  inner loop (derived by subtraction, since `resolve_full_program_from_
  parts` is a private fn invisible to the external-crate bench) — over the
  synthetic 100/1000-file perf corpus. `src/program/resolve/full.rs` gained
  a matching `#[ignore]`d unit test, `stage_split_wall_clock_on_cdo`
  (`CDO_WS=<path> cargo test --release stage_split -- --ignored
  --nocapture`), placed as a `#[cfg(test)] mod` inside `full.rs` itself
  (not under `tests/`) specifically so it can see the private inner-loop
  fn with zero visibility widening. On the real CDO workspace (median of 3
  runs, release binary): snapshot 727ms, parse 1.23s (of which ~1.19s is
  dependency-app source — only ~44ms is the workspace's own files),
  `build_program_graph` graph-build-only ~926ms, `ResolveIndex::build`
  153ms, `BodyMap::build` 185ms, resolve inner loop ~582ms. **`ResolveIndex
  ::build` + `BodyMap::build` together measure ~240-340ms on CDO scale —
  an order of magnitude over the ~30ms threshold the T3 plan's Task 9
  documents a contingency for** (an incremental single-file "rung 1" update
  cannot afford to rebuild both transiently within its 100ms budget; Task 9
  must take the cached/keyed-by-generation branch, not the transient
  rebuild). Full numbers, methodology, and the derived rung-1/rung-2 budget
  pins are in `.superpowers/sdd/t3-stage-split.md` (arc scratch, gitignored,
  not part of this commit).
- **`src/lsp/encoding.rs`: LSP `positionEncoding` negotiation (LSP 3.17) + a
  byte<->UTF-16 `LineTable` converter (H-12 infrastructure, Tier-3
  LSP-migration arc, Task 2).** The engine's `Definition`/`CallSite` ranges
  are UTF-8 byte columns throughout, but LSP's mandatory fallback encoding is
  UTF-16 code units — every response has been silently miscolumned for any
  client that doesn't negotiate `"utf-8"`, on any line containing non-ASCII
  text or an astral character (e.g. emoji) before the reported column.
  `negotiate()` reads the client's `general.positionEncodings` capability and
  picks `"utf-8"` iff the client offers it, else the LSP-mandatory `"utf-16"`
  fallback; `server.rs`'s `initialize` now negotiates this and advertises the
  result in `ServerCapabilities.position_encoding`. `LineTable` performs the
  actual per-line byte<->UTF-16 column conversion on demand (AL lines are
  short, so a `char_indices()`/`len_utf16()` walk per call is plenty fast —
  no fancier memoization); both `col_out`/`col_in` clamp out-of-range
  columns/lines to the line's end rather than panicking (fail-closed).
  **Legacy handlers keep serving byte columns THIS task** — for a
  utf-8-negotiating client behavior becomes correct NOW; utf-16 clients stay
  unchanged-broken until the Task-15 cutover wires conversion into
  `handlers.rs`.
- **`docs/superpowers/specs/2026-07-12-t3-def-surface-audit.md`: resolver-read
  audit — the definition-surface fingerprint field list (T3 LSP-migration arc,
  Task 4, DOCUMENTATION ONLY, no code changed).** The soundness spine of rung
  1's "body-only edit in F can only change F's own edges" claim: enumerates
  every data read reachable from `resolve_call_site_obligation`/
  `emit_event_flow_edges`, classifies each CALLER-side vs. SURFACE-side, and
  answers the load-bearing question — does resolution ever read another
  file's routine BODY? **No** (verified: exactly 3 non-test
  `BodyMap::get`/`get_with_path` call sites exist in the whole
  `src/program/resolve/` tree, reading only `origin`/`name_origin` byte-spans
  and `params`/`parse_incomplete` — never `.body`/`.locals`/`.return_name`).
  One real subtlety surfaced along the way (not a rung-1 blocker, but a
  BINDING constraint on Task 9): `RoutineDecl.origin` spans the whole
  declaration INCLUDING its body, so a stored edge's `Witness::SourceSpan`
  for a cross-file target goes byte-stale the instant that target's body is
  edited — handlers must always re-derive position data live from the
  current `BodyMap`/`decl_index`, never trust a baked-in span, or rung 1
  will silently serve stale ranges. Also falsified one expected fingerprint
  class from the design doc: "enum values" turned out not to be a real
  resolver read at all (enum-value dispatch is a static MS-Learn-sourced
  catalog keyed only on the enum TYPE's identity, never per-value data) and
  was dropped from the derived field list.
- **`program::resolve::full::resolve_file_obligations`: per-file resolve
  entry point (T3 LSP-migration arc, Task 6 — additive, byte-identical
  output).** `resolve_full_program_from_parts`'s Phase-1 `for pf in
  &unit.files` loop body — extract site-obligation resolution for every
  object/routine/call-site in one workspace file — is now a standalone
  `pub(crate)` fn returning a new `FileResolution { edges, flagged,
  indeterminate }`, extracted VERBATIM (identical obligation-id construction,
  identical iteration order). `resolve_full_program_from_parts` becomes a
  thin caller: build `obj_node_map`/`index`/`body_map` as before, then for
  each workspace file call `resolve_file_obligations` and fold its
  `FileResolution` into the whole-run accumulators (`obligation_id_set` is
  now populated by reading each returned edge's `obligation_id` rather than
  inline per-site — an identical-by-construction set, since every obligation
  that used to insert inline now produces exactly one returned edge carrying
  the same id). This is the engine primitive a future incremental LSP
  updater's "rung 1" needs: re-resolving ONE saved file's obligations without
  re-walking the whole workspace. Proven behavior-preserving by a new test
  (`full.rs`) asserting per-file output equals the full run's Phase-1 edges
  filtered to that file, AND that concatenating every file's output in file
  order equals the full run's Phase-1 edge list exactly (order included).
  CDO SHA gate re-verified byte-identical
  (`0a3b85bc832ff0a3e77acee118d203edbf62827dc37617c8d9315fe52d5cb7d0`).

### Removed
- **The legacy tree-sitter-only LSP pipeline — `src/graph.rs` (2477 lines),
  `src/indexer.rs` (1247), `src/parser.rs` (1202), `src/handlers.rs` (2383)
  — is DELETED (T3 LSP-migration arc, Task 17, the capstone).** This is the
  pipeline the program-engine-backed LSP surface (Tasks 8-16, above and in
  `### Changed`/`### Added`) fully replaced: `CallGraph`'s `QualifiedName`-
  keyed (name-only, no signature/kind/enclosing-member discriminator)
  `Definition`/`CallSite` model, the naive text-matching call resolver in
  `indexer.rs`, and every handler in `handlers.rs` that read them
  (`prepare_call_hierarchy`/`incoming_calls`/`outgoing_calls`/`code_lens`/
  `get_unused_procedure_diagnostics`/`dependency_document_symbol`/
  `event_publishers_in_file`/`event_reference_at_position`) — all now dead
  code, unreachable since Task 15's server cutover pointed `server.rs`'s
  dispatcher entirely at `lsp::handlers`/`lsp::lens`/`lsp::diagnostics`/
  `lsp::custom`.
  **Deletion license:** `tests/lsp_differential.rs` (the adjudicated
  legacy-vs-new differential parity harness, Task 14 — also deleted here,
  a scaffolding test that structurally cannot compile once one of the two
  engines it drove is gone) proved, on real CDO source: 12,284 identical
  answers across `prepare`/`incoming`/`outgoing`/`codeLens`/
  `unused-procedure` diagnostics, ~44,000 additional divergences every one
  mechanically classified into a named, justified `NewBetter` class (never
  a bare "different, who knows why"), `Regression = 0` and
  `NewUnexplained = 0` held across five escalating CDO fix-wave layers
  (`LegacyIdentityCollapse`, `ImplicitTriggerEdge`, `ImplicitRecResolved`,
  `VariableReceiverResolved` generalized off object identity, and a widened
  `DepSourceSpan` predicate), plus a real H-10 edit-scenario regression
  legacy carried (losing cross-file incoming edges after a same-file
  no-op reindex) that the new engine provably does not. Its differential-
  only fixtures (`tests/fixtures/lsp-diff-core/`, `lsp-diff-identity/`,
  `lsp-diff-nested/`) are deleted alongside it; `tests/fixtures/lsp-diff-deps/`
  is KEPT — `tests/lsp_incremental_parity.rs`'s (the PERMANENT incremental
  gate) `dep_bearing_rung1_then_rung2_stay_equivalent_with_nonvacuous_dep_indexes`
  script reuses it for real dependency-index coverage — and
  `tests/fixtures/lsp-incr/` is likewise KEPT (the incremental gate's
  primary fixture).
  **`tests/parser-ir-goldens/`** (the r0-corpus `parser.rs` projection
  golden) retires together with `parser.rs` — nothing else in the repo
  reads it.
  **Relocated, not deleted:** the three handlers legacy `handlers.rs` still
  owned that never touched `graph`/`indexer` at all — `fieldProperties`/
  `actionProperties` (+ their `SymbolProperties*` types and
  `read_source_from_uri`/`to_symbol_properties_result` helpers) and the
  `al-preview://` URI parser `parse_al_preview_uri` (+ its `urldecode`
  helper) — move verbatim into `src/lsp/custom.rs`; Task 15's cutover had
  already pointed `server.rs`'s dispatcher at their legacy implementations
  unchanged, so this is a pure relocation, not a behavior change.
  `telemetryStatus` has no handler function to relocate (`server.rs` calls
  `crate::telemetry::status()` directly).
  **Coverage-completeness fix found during deletion verification:**
  `src/types.rs`'s `ObjectType` (a surviving, actively-used type) had its
  only direct unit tests (`TryFrom<&str>`'s valid/case-insensitive/invalid
  cases, `Display`'s exact-capitalization output) living in `graph.rs`,
  which only ever re-exported the type — deleting `graph.rs` outright
  would have left `ObjectType` with zero direct unit test coverage of its
  own. Ported the 4 tests verbatim into `src/types.rs` itself.
  **`tests/perf_support_smoke.rs`** — its `Indexer`-dependent correctness
  checks (the corpus's 999-way fan-in / 3-way fan-out contract, and the
  rung-1/rung-2 body-edit/signature-edit definition-count deltas) are
  rewritten onto `LspSnapshot`/`lsp::handlers`/`lsp::updater` rather than
  deleted outright: `tests/perf_bounds.rs`'s equivalent assertions only
  compile under `#[cfg(not(debug_assertions))]` (a release-only gate), so
  this file remains the only place the corpus's contract is pinned under a
  plain `cargo test` (every debug-profile run, not just
  `cargo test --release --test perf_bounds`).
  `src/lib.rs`/`src/main.rs` dropped the `graph`/`handlers`/`indexer`/
  `parser` module declarations and re-exports. CLAUDE.md's Architecture
  section is rewritten from "two pipelines" to "one engine, two consumers"
  (the LSP surface and the CLI/`aldump`), its Key Modules list and
  Performance Targets table updated to the surviving `src/lsp/*` modules
  and Task 16's measured numbers, and its stale `QualifiedName`/
  `Definition`/`CallSite` "Key Data Structures" block replaced with the
  current `RoutineNodeId`/`DeclEntry`/`EdgeRef` shapes.

### Changed
- **`load_all_apps` is now manifest-first (perf safe-wins plan, Task 3)** —
  discovered `.app` files are GUID-deduped (H-2) on a cheap manifest-only
  read (`extract_app_metadata`, KB-sized `NavxManifest.xml` only) BEFORE any
  `SymbolReference.json` (MB-sized, sometimes 100 MB) is parsed; only the
  per-GUID version winners then pay `extract_app_symbols`. Duplicate Base
  App / System App copies discovered across ancestor `.alpackages` folders
  no longer each pay a full SymbolReference parse — only the highest-version
  survivor does. `dedup_by_guid_keep_highest_version` is retyped from
  `Vec<ResolvedDependency>` to a new manifest-level `DiscoveredApp{app_path,
  meta}`; `load_all_apps`'s own public signature and its trailing
  deterministic sort are unchanged. **Known, accepted behavior change:** a
  duplicated-GUID `.app` whose version-losing copy has a corrupt
  `SymbolReference.json` is now dropped as a reported dedup loser (named in
  `load_all_apps`'s second return value) instead of the old order's silent
  extraction failure — identity comes from the manifest, not from whether a
  large symbol blob happens to parse, so a corrupt package is a properly
  surfaced drop either way.
  - **Fix (availability regression, same task):** the first cut of this
    reorder introduced a real regression — if the manifest-first dedup
    WINNER (highest version) had a corrupt `SymbolReference.json`, the
    dependency vanished entirely, where the old symbols-first order would
    have tried every physically-discovered copy and loaded whichever one
    actually parsed. Fixed: `load_all_apps`'s symbol-extraction phase now
    walks each GUID's candidates highest-version-first and falls back to
    the next-highest copy when a candidate's symbols fail to parse,
    repeating until one succeeds or the group is exhausted (matching, and
    in the corrupt-winner case bettering, the old order's effective
    resilience). The corrupt ex-winner is warned about (not reported as a
    dedup drop — it never lost a version comparison, it was simply
    unreadable); the promoted good copy is the new kept version, and only
    candidates ranked below IT are reported as dedup drops.
- **Embedded/workspace source text is now ONE shared `Arc<str>` allocation per
  file across the whole snapshot/parse/LSP pipeline (perf safe-wins plan,
  Task 1)** — kills the ~228 MB duplicate-text overhead on the reference
  workspace documented in `docs/perf-regression-t3-vs-0.9.3.md` §3.1, where
  `AppSetSnapshot`'s `SourceFile.text`, `ParsedFile.text` (previously cloned
  in `parse_snapshot`), and `LspSnapshot::dep_texts` (previously a fresh copy
  built in `build_dep_indexes`) each held an independent copy of the same
  embedded dependency source (~114 MB on a real BC workspace, tripled).
  `SourceFile.text`/`ParsedFile.text`/`ParsedFileEntry.text` are now
  `std::sync::Arc<str>`, and `dep_texts` shares that SAME allocation via
  `Arc::clone` instead of copying it; `Arc<str>` derefs to `&str` so almost
  every call site kept compiling unchanged. Enabled serde's `rc` feature so
  `SourceFile`'s content-addressed cache (`src/snapshot/cache.rs`) still
  serializes `Arc<str>` as a plain string — the on-disk JSON format, and
  every existing cache entry, is unchanged.
- **`tests/perf_bounds.rs`/`benches/lsp_pipeline.rs` rewritten onto the
  ENGINE-BACKED LSP surface (T3 LSP-migration arc, Task 16) — the last thing
  standing before Task 17 deletes the legacy `Indexer`/`graph`/`handlers`
  pipeline these two files used to pin.** Both now build `LspSnapshot`
  (`build_full`/`build_full_with_parsed`) and drive `lsp::handlers::{prepare,
  incoming,outgoing}` + `lsp::updater::Updater::apply_batch` instead of
  `Indexer::index_directory`/`Arc<RwLock<Indexer>>`/`handlers::{prepare_call_hierarchy,
  incoming_calls,outgoing_calls}`. New rows: `build_full` (100/1000 files, same
  <500ms/<2s CLAUDE.md targets as the old `initial_index` rows), and TWO NEW
  incremental-update rows — rung-1 body-edit `apply_batch` (<100ms target, 3x
  CI bound 300ms) and rung-2 signature-edit `apply_batch` (~1.5s target, 3x CI
  bound 4.5s; both targets are the T3 Task 9 Step-3b RE-MEASUREMENT against the
  real `Updater` on the real CDO workspace — `.superpowers/sdd/t3-stage-split.md`'s
  addendum — which REPLACES Task 3's earlier ~1.9s algebraic upper-bound
  estimate for rung 2, since that estimate predated `Updater`/
  `assemble_program_graph` entirely). `incomingCalls` under this corpus's
  deliberate 999-way real fan-in gets its OWN, separately-reasoned CI bound
  (25ms target / 75ms bound) rather than inheriting the shared 1ms/3ms query
  bound `prepare`/`outgoing` keep: every distinct caller's position is now
  re-derived LIVE from that caller's own current file text (the live-span
  audit's correctness rule — never serve a stale stored witness span), so a
  999-way-fan-in query is genuinely O(distinct callers) in a way legacy's
  precomputed-byte-range design never was; measured 20.3ms on this machine,
  comfortably inside the new bound.
  **A root-cause fix to the shared corpus generator was required to keep the
  999-way fan-in assertion meaningful**: `tests/perf_support`'s cross-file
  "hub" call previously read `HubObjectName.Proc0()` — a bare object-display-name
  call with no declared receiver, which is not valid AL and only "worked"
  because the legacy pipeline's naive text-matching call resolution
  (`callee_object` is whatever raw text sits left of the dot, matched directly
  against object display names when no variable binding exists) tolerated it.
  Confirmed empirically against the fresh resolver (a 2-file probe workspace
  via `aldump --program-call-graph-stats`): the bare form classifies as
  `Unknown`/`UntrackedReceiver`, 0% resolved, which would have made the
  999-way fan-in assertion vacuous (0, not 999) under the new backend. Fixed
  by declaring a local `Hub: Codeunit "..."` variable and calling through it —
  valid AL both engines resolve correctly (confirmed 0 unknown / 7-of-7
  resolved on the probe workspace; `tests/perf_support_smoke.rs`'s existing
  legacy-correctness assertions are unaffected, still exactly 999/3).
  perf_support also gained `body_only_comment_edit` (a new pub helper: inserts
  one comment line as `Proc0`'s first statement — no routine identity/
  signature change, so the file's `DefSurface` fingerprint stays byte-identical,
  which is exactly rung 1's own gate condition) plus a new smoke test pinning
  that contract (`body_only_comment_edit_adds_no_definitions`).
  **Review fix-wave (t3.16):** the rung-1/rung-2 rows above now assert TWO
  bounds each, not one — the CDO-anchored absolute bound (300ms/4.5s) carries
  15-30x headroom against what actually runs on this SYNTHETIC corpus
  (~20ms/~150ms), which meant a genuine 10x regression on the corpus alone
  could sail through unnoticed; `RUNG1_SYNTHETIC_BOUND`/`RUNG2_SYNTHETIC_BOUND`
  (100ms/750ms — 5x today's measured synthetic-corpus baseline) close that
  gap, asserted alongside (never instead of) the absolute bound. Separately,
  `.github/workflows/ci.yml`'s Lint step was missing `--all-targets` (it
  already had `--release`, since `f323d6d` — the earlier framing in this
  task's own report that blamed the `#[cfg(not(debug_assertions))]` gate was
  WRONG and is corrected here): without `--all-targets`, `cargo clippy` never
  compiles ANY test or bench crate in ANY profile, so `tests/perf_bounds.rs`
  and `benches/lsp_pipeline.rs` (and every other `tests/*.rs`/`benches/*.rs`
  file) had NEVER been linted by CI at all, regardless of profile. Fixed to
  `cargo clippy --release --all-targets --all-features -- -D warnings`,
  matching CLAUDE.md's documented bar; re-running that exact command against
  the WHOLE repo surfaced zero new findings (confirmed via a genuine forced
  recompile, not a stale cache hit).
- **One `Arc<al_syntax::ir::AlFile>` per parse — the duplicate whole-program
  parse at LSP startup, and every rung-1/rung-2 per-file re-parse, are gone
  (perf safe-wins plan, Task 2).** `ParsedFile.file`/`ParsedFileEntry.file`
  are now `Arc<AlFile>` (mirroring Task 1's `Arc<str>` text sharing) —
  sound because nothing in the engine mutates an `AlFile` after
  `al_syntax::parse` returns; every update REPLACES a whole `ParsedFile`/
  `ParsedUnit` value (rung-1's `pending` splice, rung-2's `splice_file`,
  rung-3's wholesale `Vec` replacement), so two owners of the same
  `Arc<AlFile>` can never observe a torn or stale-relative-to-each-other
  view. `LspSnapshot::from_context` no longer CONSUMES the one parse it's
  handed — it clones `Arc`s into the published snapshot's
  `ParsedFileEntry`s and returns the intact `Vec<ParsedUnit>` alongside it,
  so `build_full_with_parsed` no longer needs (and no longer runs) a SECOND,
  fully independent `parse_snapshot` pass over the whole workspace + every
  embedded-source dependency at server startup. `Updater::apply_rung1_core`
  and `Updater::apply_rung2` (`src/lsp/updater.rs`) previously re-parsed
  each touched/every workspace file a SECOND time just to populate the
  snapshot's own `ParsedFileEntry` copy; both now `Arc::clone` the already-
  parsed file instead — rung 2 used to re-parse EVERY workspace file on
  every signature-change edit and now re-parses none. Per
  `docs/perf-regression-t3-vs-0.9.3.md` §2.3/§3.2, this eliminates roughly
  1s of duplicate cold-start parsing plus one whole extra dependency
  text+IR-arena set held in memory on the reference workspace. Zero LSP-served
  data changed: `tests/lsp_incremental_parity.rs` (the permanent
  incremental-vs-batch differential gate) passes unmodified plus a new
  sharing-proof test
  (`build_full_with_parsed_shares_one_parse_between_snapshot_and_updater`)
  asserting `Arc::ptr_eq` between the published snapshot's and the
  updater's `AlFile`/text allocations; `cargo test --release --test
  perf_bounds` (the CI rung-budget gate) stays green.

  arc, Task 15 — the cutover).** `src/server.rs` no longer holds
  `Arc<RwLock<Indexer>>`; its state is `Arc<lsp::updater::SharedSnapshot>` (the
  published, immutable `LspSnapshot`) plus an `mpsc::Sender<ChangeEvent>`
  feeding a background `spawn_updater` thread. Every request handler only
  ever `Arc`-clones the current snapshot — no parsing or graph rebuild ever
  happens under a request-facing lock; the updater thread owns all rebuild
  work and publishes a fresh snapshot by atomic swap.
  `textDocument/prepareCallHierarchy` / `callHierarchy/{incoming,outgoing}Calls`
  / `textDocument/codeLens` / the three engine-backed custom requests
  (`dependencyDocumentSymbol`/`eventPublishersInFile`/`eventReferenceAtPosition`)
  now dispatch straight to the Tasks 11-13 functions
  (`lsp::handlers`/`lsp::lens`/`lsp::custom`) with the NEGOTIATED position
  encoding threaded through every call (H-12 is now fully wired end to end —
  `lsp::diagnostics::compute_all` no longer hardcodes `PositionEncoding::Utf16`
  internally, closing the two `TODO(t3.15)`s Task 12 left). `al-call-hierarchy/
  {fieldProperties,actionProperties,telemetryStatus}` are graph-independent
  and stay on their existing `crate::handlers` implementations (widened from
  private to `pub` so the new dispatcher can call them directly) — they
  survive Task 17's legacy deletion untouched.
  **Multi-root workspaces: a deliberate, documented capability narrowing.**
  A client offering more than one `workspace_folders` entry now gets exactly
  ONE app served (the first, with a clear warning) instead of legacy's
  silent multi-folder accumulation into one `Indexer` graph — see
  `server.rs`'s `primary_workspace_root` doc comment for the full decision
  record (legacy's multi-root was a `LegacyIdentityCollapse`-shaped
  collision hazard, not a working feature, and the tracked follow-up design
  for real multi-root support lives there too). Diagnostics now follow
  "recompute-diff-publish-clear" on EVERY snapshot swap (including the very
  first, batch-built one), via one shared `lsp::diagnostics::DiagnosticsState`:
  publishing only what changed and CLEARING a uri whose findings dropped to
  zero — the legacy publish-once-at-startup path never cleared a fixed
  finding until an unrelated one happened to overwrite the same file's
  bucket. The file watcher (`src/watcher.rs`) is widened to also forward
  `.alpackages` dependency-file changes (previously filtered out entirely by
  its `.al`-only extension check — dependency add/update/remove never
  reached the index until a server restart) and a backend-reported
  event-buffer overflow/rescan (`notify`'s `Flag::Rescan`, via a new
  `FileChange::Overflow` variant) — both map onto `ChangeEvent::DepsChanged`/
  `ChangeEvent::Overflow` in `server.rs`, forcing a full rebuild rather than
  silently trusting stale state. `didSave` notifications and watcher events
  now feed ONE coalesced channel (previously each independently reindexed the
  same save). The program engine's snapshot model requires a single AL-app
  workspace root with a readable `app.json` (unlike the legacy `Indexer`,
  which tolerated zero indexed files) — a missing/invalid workspace degrades
  to every request returning an empty result rather than the server
  refusing to start. `main.rs`'s `--project` CLI
  index-and-report mode is re-pointed at `LspSnapshot::build_full`:
  `definitions` is now the workspace `decls_by_file` entry count and
  `call sites` is the sum of `edges_by_file` bucket lengths — NEITHER number
  is directly comparable to the legacy `CallGraph::definition_count`/
  `call_site_count` this replaces (different identity/dedup rules); the
  legacy "external definitions" line is replaced by a count of dependency
  routines with embedded source (`dep_decl_by_id`). `--analyze` is untouched
  (`analysis.rs` reads the owned IR directly, no snapshot involved). Smoke-
  tested via a new in-binary unit test (`server::tests`, `cargo test` runs
  binary-target unit tests too — no existing `tests/` crate could reach these
  binary-private symbols) driving `dispatch_request`/`handle_notification`
  over a real `Connection::memory()` pair: prepare + outgoingCalls resolve
  through the new handlers, and a `didSave` round-trips into a published
  generation bump plus a diagnostics republish that clears a since-fixed
  unused-procedure hint, and (T3 Task 15 review fix-wave) a named unit test
  pinning `ChangeEvent::Overflow`'s own escalation to rung 3
  (`overflow_event_escalates_to_rung3` in `lsp/updater.rs` — it shared
  `DepsChanged`'s match arm from Task 9 onward, structurally covered but
  never pinned on its own until now). Full test suite green (2679 passed, 0
  failed, legacy unit tests unaffected — legacy code is unwired, not
  deleted); `tests/lsp_differential.rs` and `tests/lsp_incremental_parity.rs`
  (the Task 14 deletion-license gates) stay green, unaffected by the
  cutover. A real shutdown-hang bug was found and fixed along the way, via a
  manual end-to-end stdio smoke test — see **Fixed**, below.
- **`program::build`: layered dep/workspace graph assembly + a single-parse
  program-engine pipeline (T3 LSP-migration arc, Task 5 — additive,
  byte-identical output).** `build_program_graph` used to do one monolithic
  pass over the WHOLE snapshot (every app, dep and workspace alike); it is
  now a thin wrapper over two new primitives: `build_dep_layer` (everything
  derived from NON-primary apps — object/routine extraction, ABI ingest,
  dependency topology, `internalsVisibleTo` friend wiring — into a `DepLayer`)
  and `assemble_program_graph` (merges a `DepLayer` with a freshly-extracted
  PRIMARY/workspace `ParsedUnit` into the full `ProgramGraph`, re-running sort
  + dedup + the synthetic platform-event-publisher injection). This is the
  primitive a future incremental LSP updater's "rung 2" needs: rebuild only
  the workspace layer over an UNCHANGED dep layer, without re-parsing or
  re-extracting a single dependency file. Also kills a real production
  double-parse T3 Task 3 measured on CDO (~1.19s dependency-parse alone):
  `resolve::full::build_context` used to call `build_program_graph` (which
  parses the whole snapshot internally) AND separately run its own
  standalone `parse_snapshot` for the resolver's body-walk; it now parses
  ONCE and calls the new `build_program_graph_from_parsed(snap, abi_cache,
  &parsed)` entry point instead. `AppRegistry` and `DependencyGraph` gained
  `Clone` (needed so `assemble_program_graph` can clone a borrowed
  `DepLayer`'s shared fields into each assembled graph). Every existing
  caller of `build_program_graph` (aldump, `engine/l4`/`l5`/`gate`, tests) is
  unaffected — the public signature and output are unchanged; verified via a
  new characterization test (`build.rs`) proving the split composes to the
  identical graph field-by-field, plus the frozen CDO SHA-256 gate
  (`0a3b85bc832ff0a3e77acee118d203edbf62827dc37617c8d9315fe52d5cb7d0`,
  byte-identical).
- **L5 detectors now return `Result<DetectorOutput, DetectorError>` instead of
  `DetectorOutput` — the abort-safe detector-isolation contract (Task T2.3, Tier-2
  crash/DoS arc).** `[profile.release] panic = "abort"` makes `catch_unwind`
  (`registry.rs`) INERT in every shipped binary, so the documented "a detector that
  panics becomes a Diagnostic and the rest still run" guarantee never actually held
  outside `cargo test` (which unwinds) — one panicking detector aborted the whole
  `alsem analyze` run. All 41 registered detectors (`d1`..`d51`) were converted
  mechanically: the signature change plus wrapping each existing return in `Ok(...)`;
  no detector logic changed. `run_each` now maps a detector `Err` to the identical
  `Detector "<name>" threw: <msg>` warning diagnostic the panic path already used, so
  the message format consumers rely on (possibly golden-pinned) is unchanged.
  `catch_unwind` is KEPT around the `Result` call as debug-build-only
  defense-in-depth (still catches an errant `panic!` under `cargo test`) but the doc
  contract on `run_detectors`/`Detector` now states plainly that it is inert under
  `panic = "abort"` and must never be relied on as the real guarantee.

### Fixed
- **The LSP server never actually exited after a clean `shutdown`/`exit`
  exchange (T3 LSP-migration arc, Task 15 review fix-wave) — found via a
  manual end-to-end stdio smoke test (a Python LSP client driving the real
  compiled binary through `initialize`→`prepareCallHierarchy`→
  `outgoingCalls`→`shutdown`→`exit`) that neither the in-binary unit test
  nor any prior differential/parity harness could have caught (all of them
  call handler functions in-process, never through a real `Connection::
  stdio()` round trip).** Two independent layers, both root-caused against
  `lsp_server` 0.7.9's own source:
  1. `main_loop` took `connection: &Connection` (a borrow). `IoThreads::
     join()` (called right after) blocks until `lsp_server`'s writer thread
     sees every `Sender<Message>` clone dropped — but a BORROWED
     `connection` is never dropped before that call, since it's still a
     live local in `run_server`'s own scope. **This exact shape already
     existed in the pre-cutover legacy `server.rs`** — an identical bug,
     just never exercised end to end before this task's own verification
     step. Fixed by moving `connection` into `main_loop` by value, matching
     `lsp_server`'s own `examples/minimal_lsp.rs` pattern exactly (drops
     `connection` — and its `sender` — at `main_loop`'s own closing brace,
     before `io_threads.join()` runs).
  2. A SECOND, cutover-introduced layer surfaced once (1) was fixed: the
     updater thread's diagnostics `on_swap` hook holds its own
     `connection.sender` clone for its entire (deliberately unbounded)
     lifetime, and the file watcher thread separately holds its own clone
     of the updater's INPUT channel forever too (no stop signal — matching
     legacy's watcher thread, which also never exits early). So the
     updater never naturally terminates, its `sender` clone never drops,
     and unconditionally joining it (or waiting unconditionally on
     `io_threads.join()`) would still hang forever. Fixed with a bounded
     wait + detach at both points — the SAME idiom `telemetry::shutdown`
     already used elsewhere in this same file for the identical class of
     problem (a background thread that legitimately never stops on its own
     must never be joined unconditionally at shutdown): everything
     meaningful is already flushed to stdout by the time `main_loop`
     returns (verified — the `shutdown` response itself is received by the
     client before the hang), so giving up on a background thread after a
     short grace period loses nothing. Verified via 4+ consecutive clean
     `PROCESS EXIT CODE: 0` runs of the manual stdio smoke test after the
     fix, plus the full `cargo test` suite re-run green.
- **`path_to_uri` now percent-encodes URIs correctly on `percent_encoding` instead of a
  hand-picked ~5-character subset (H-13, Tier-3 LSP-migration arc, Task 1).**
  `protocol.rs`'s encoder only escaped space/`(`/`)`/`[`/`]`, so any other byte the
  `lsp-types` URI parser rejects — non-ASCII text (e.g. a workspace path containing
  `Løsninger`), `#`, raw `%`, `+`, `@`, emoji — produced an unparseable URI string and
  silently fell back to the sentinel `file:///unknown`, breaking every LSP response
  (definitions, call sites, code lens) for any file under such a path. Each path segment
  is now escaped independently with `utf8_percent_encode` against an RFC 3986
  `pchar`-complement `AsciiSet` (plus `%` itself and `\`), so arbitrary Unicode and
  reserved-character filenames round-trip through `path_to_uri`/`uri_to_path` losslessly.
  The existing Windows drive-letter normalization and forward-slash join are unchanged;
  only the per-segment escaping changed. Signature unchanged (`&Path -> Uri`). **Wire-format
  note:** `(` and `)` are no longer percent-encoded (the old encoder escaped them as
  `%28`/`%29`); RFC 3986 allows literal parens unescaped in a path segment (they're
  `sub-delims`), and the round trip still holds byte-for-byte — this is a deliberate,
  intentional narrowing of what gets escaped, not an oversight.
- **Decompression caps on every zip/gzip site — a hostile `.app` (or a hostile
  `.cbor.gz` snapshot fed to `alsem diff`) could OOM-kill the whole process
  (Task T2.2, crash/DoS arc).** Every zip entry and gzip stream in the
  ingestion paths was read via an unbounded `read_to_end`: a few KB of
  DEFLATE expanding to an attacker-chosen number of gigabytes, and release
  builds are `panic=abort`, so even the resulting allocation failure aborts
  the whole LSP/CLI process, not just the one request. There is no zip-slip
  vector on any of these paths (extraction never writes entry-derived paths).
  - **New `src/capped_io.rs`**: one shared `read_capped(reader, cap) ->
    Result<Vec<u8>, CapReadError>` (bounds the allocation via `Read::take(cap
    + 1)`, so a hostile stream forces at most a `cap + 1`-byte allocation, not
    its full expanded size) + `check_declared_size(declared, cap)` (a
    belt-and-suspenders pre-check against a zip entry's central-directory
    `size()`, rejecting before decompressing when the archive doesn't lie
    about it). `CapReadError` implements `std::error::Error`, so `?` composes
    into every call site's existing `anyhow::Result` unchanged.
  - **Six sites capped**, each ceiling grounded in a real size measured
    against the CDO reference workspace (10 real BC apps incl. Microsoft
    BaseApp/System Application) and given generous headroom — see
    `src/capped_io.rs` for the per-cap justification comments:
    `SYMBOL_REFERENCE_JSON_CAP` (512 MB; BaseApp measures ~58.3 MB),
    `NAVX_MANIFEST_XML_CAP` (4 MB; BaseApp measures ~6.3 KB),
    `EMBEDDED_AL_SOURCE_CAP` (16 MB; the largest real `.al` entry observed is
    ~789 KB), `SNAPSHOT_GZ_CAP` (1 GB; reasoned, not directly measured — see
    the module doc). Wired into `app_package::{parse_manifest,parse_symbols}`,
    `program::abi_ingest::read_symbol_reference_from_app`,
    `snapshot::embedded::extract_embedded_source`,
    `engine::deps::app_package_zip::extract_entry_bytes` (now cap-parameterized
    — shared by the manifest and symbol-reference callers, each passing its
    own cap), `engine::deps::dep_artifact_l4::iterate_embedded_source`, and
    `engine::gate::snapshot_deserialize::gunzip`.
  - **Failure semantics preserved per surface**: sites that already return
    `Result`/`anyhow::Result` (app_package, abi_ingest, snapshot::embedded,
    snapshot_deserialize) surface a cap-exceeded overage through the SAME
    error path as any other read failure — never a panic, never a silent
    empty result. Sites with an established fail-closed-to-`None`/skip
    contract (`app_package_zip::extract_entry_bytes`,
    `dep_artifact_l4::iterate_embedded_source`'s per-entry skip) keep that
    contract — cap-exceeded joins the SAME `None`/skip path corruption
    already used, rather than introducing a new failure mode.
  - CDO byte-identity verified: every real CDO artifact is well under its
    cap, so resolution output is unchanged; all 1398 lib tests pass
    (up from 1378 pre-task), including new crafted-oversized-entry fixtures
    at every site (a small-compressed/huge-declared-size zip entry, a
    genuinely-over-cap DEFLATE/gzip stream of compressible zeros, and a
    normal-sized real `.app`/gz payload round-tripping byte-identical).
- **Stack-overflow hardening everywhere the `al_syntax` lowerer and the L4 CFG
  walker run (Task T2.1, crash/DoS arc).** `src/snapshot/parse.rs` had
  documented an OBSERVED stack overflow lowering real BaseApp source and
  worked around it with a local 32 MiB rayon pool — the ONLY hardened
  `al_syntax::parse` call site in the repo. Every other site running the same
  recursive lowerer (the LSP indexer, `didSave` on the LSP main thread — ~1
  MiB on Windows — the file-watcher thread, CLI `--analyze`, and the engine's
  sequential per-workspace parse loops used by `aldump`/`alsem`) was
  unhardened, and release builds are `panic=abort`, so a deep AL expression or
  hostile file could SIGSEGV/abort the whole process uncatchably.
  - **New `src/big_stack.rs`** generalizes the one working mitigation:
    `run_with_big_stack` (a scoped 32 MiB thread — borrows freely, no
    `'static` bound, so an owned value's DROP never lands on the small
    thread) for sequential call sites, `big_stack_pool` (a local
    `rayon::ThreadPool`) for parallel ones. Wired into `snapshot::parse_snapshot`
    (refactored onto the shared helper), `Indexer::index_directory` (parallel
    pool) and `Indexer::reindex_file` (covers both `didSave` and the watcher
    thread), CLI `run_analysis`, and the engine's `engine::snapshot::snapshot_workspace`,
    `engine::l2::l2_workspace::project_workspace`,
    `engine::l3::l3_workspace::{assemble_workspace,assemble_workspace_units}`,
    and `engine::gate::workspace_diagnostics::compute_workspace_diagnostics`
    sequential parse loops (one big-stack thread per WHOLE loop, not per file).
  - **Depth budget in the lowerer** (`crates/al-syntax/src/lower/mod.rs`):
    `lower_stmt`/`lower_expr` (mutually recursive with `lower_branch`,
    `lower_code_block`, `lower_stmt_seq`, `lower_block_child`, `lower_case_body`
    and its helpers, `lower_opt_field`, `lower_branch_field`) had no bound of
    their own — a generated `x := 1 + 1 + … 50k terms` or 50k-deep nested `if`
    recursed the native stack proportionally to input size. A `depth: u32`
    counter now threads through the whole family (plumbing helpers forward it
    unchanged; only `lower_stmt`/`lower_expr` increment, so it tracks AL
    syntactic nesting depth, not raw native-frame count) and fails closed past
    `MAX_LOWER_DEPTH` to a `SyntaxIssue` + `ExprKind::Unknown`/`StmtKind::Unknown`
    — never crashes. 128, not the in-repo `MAX_CBOR_DEPTH` precedent of 256:
    measured empirically against the actual red fixture (a 50k-term binary
    chain lowered on a 1 MiB thread, simulating the real Windows LSP
    main-thread stack) — 256 crashed an unoptimized debug build, 224 passed;
    128 gives a ~2x margin, still nowhere near real AL nesting.
  - **`walk_cfg` depth bound** (`src/engine/l4/cfg_walker.rs`): the single
    self-recursive branch-aware CFG walker gained the same `depth: usize`
    treatment (mutual helper `apply_condition_leaves` forwards it, `walk_cfg`
    itself increments and checks). Past `MAX_CFG_WALK_DEPTH` it degrades to the
    SAME conservative `saturate_unknown` path already used for bounded-loop
    overshoot — never recurses further; T1.1's LoopFrame/Reach machinery is
    untouched. `walk_cfg`'s own frame proved heavier than the lowerer's (a
    single 12-arm-match function with several `PerParamState` clones per arm):
    measured empirically the same way, 96 crashed, 64 passed; 32 gives a ~2x
    margin. `PCFNNode` is `Deserialize`, so this input can arrive from a
    cache/snapshot, not only from fresh lowering — the budget is not
    contingent on the lowerer's own cap.
  - Both budgets were proven genuinely red-first: temporarily disabling each
    (`MAX_LOWER_DEPTH = u32::MAX` / `MAX_CFG_WALK_DEPTH = usize::MAX`)
    reproduced a real `STATUS_STACK_OVERFLOW` crash on the small-stack test
    fixture before the fix, confirmed by-hand with `cargo test`.
  - CDO byte-identity verified: real-code resolution and L4 facts are
    unchanged (the budgets only fire past pathological, non-real-AL depth);
    all 1378 lib tests + 42 `al-syntax` tests pass.
- **Four small reachable panics/corruption sites, each confirmed and fixed
  with a red-then-green fixture (Task T2.4, Tier-2 crash/DoS arc).**
  - **`unquote_path` lone-quote panic** (`diff_parser.rs`): a one-character
    `"` token passes both `starts_with('"')` and `ends_with('"')` (the same
    char), then `trimmed[1..trimmed.len()-1]` underflowed to `[1..0]` and
    panicked. Reachable via `alsem digest --diff <file>` on a diff truncated
    mid-header (`--- "`, `rename from "`, `rename to "`) — the surrounding
    code already has a graceful `ChangedRootsDiagnostic::DiffParseError`
    path that the panic bypassed. Fixed with a `trimmed.len() < 2` guard that
    degrades to returning the raw token.
  - **CBOR map-16 header `debug_assert!`** (`cbor.rs`): the comment claimed
    the >65535-key guard was "an invariant we ENFORCE, not hope for" but
    enforced it only via `debug_assert!`, which compiles out under
    `[profile.release]` and lets `entries.len() as u16` silently wrap,
    corrupting the stream. Promoted to a release-alive `assert!` (`encode`
    is infallible-by-signature and called from dozens of snapshot-building
    sites, so threading a `Result` through wasn't the shallow fix here).
    Verified empirically both ways: the new test FAILS under
    `cargo test --release` on the pre-fix code (proving the release-mode
    corruption is real) and PASSES after the fix.
  - **`strip_trailing_temporary` Unicode slice bug**, present verbatim in
    both `engine::l3::record_types` and `program::resolve::receiver`: sliced
    the ORIGINAL string with a byte offset computed from a `to_lowercase()`
    copy — panics on `ẞ` (U+1E9E, whose lowercase byte length differs from
    its own) and silently fails to recognize a real `İ Temporary` (Turkish
    dotted capital I, U+0130) table name as temporary, leaving " Temporary"
    stuck on the parsed name. Both copies rewritten to walk `char_indices()`
    on the original string directly and ASCII-fold-compare against the
    literal word "temporary", so every slice offset is provably a valid char
    boundary. **Not deduplicated into one shared helper**: a grep-guard test
    (`resolve_module_has_no_stray_engine_l3_l2_imports`) enforces that
    `src/program/resolve` stays L3-independent except `builtins.rs`'s one
    sanctioned exception, so both copies are fixed independently with a
    cross-referencing comment instead.
  - **Sweep for `debug_assert!` siblings** (`rg -n 'debug_assert' src/`):
    found two more guarding a genuinely input-size-triggerable invariant
    (silently truncates/corrupts on release, mirroring the CBOR fix above)
    and promoted both: `cbor.rs`'s `encode_uint_header` (>32-bit
    length/magnitude truncates via `(n as u32)`, reachable from
    `encode_text`/`encode_array_header` on a >4GB string/array) got the same
    `assert!` treatment; `snapshot_full/to_cbor.rs`'s `MapSer::serialize_key`
    non-text-key guard was a bare `debug_assert!(false, ..)` whose comment
    claimed it "ENFORCES that invariant rather than silently stringifying a
    non-text key" — false in release (the `format!()` fallback ran
    regardless) AND it PANICKED even in debug (an unwind `to_cbor_value`'s
    `unwrap_or` can't catch), worse than either intended behavior; converted
    to a real `Err` since `serialize_key` is already `Result`-returning
    (the shallow, more-correct fix per this task's own precedent for the
    infallible-signature cases). The remaining `debug_assert!` sites
    (`fingerprint.rs`'s SHA-256-length self-check, `format_sarif.rs`'s
    zip-length precondition, `l4/incremental/queries.rs`'s SCC-order
    self-check, `graphify_export.rs`'s/`edge.rs`'s/`receiver.rs`'s
    unreachable-arm tripwires) are legitimate: each is either a
    mathematical/structural invariant no external input can violate, or its
    release-mode fallback is already the same fail-closed behavior the code
    would take anyway (no correctness difference between profiles) — none
    qualify as the wrap/truncate/corrupt shape this sweep targets.
  - `scripts/cdo-gate` PASS (`program_resolve_harness` 187/187,
    `program_graph` + `snapshot_robustness` 2/2, `--release`,
    `ENFORCE_CDO_WS=1`); CDO's `--program-call-graph-stats` SHA-256
    re-confirmed BYTE-IDENTICAL to the frozen baseline
    (`0a3b85bc832ff0a3e77acee118d203edbf62827dc37617c8d9315fe52d5cb7d0`) —
    all four fixes are pathological-input paths real CDO source never hits.
    `cargo test --workspace` (160 binaries) and
    `cargo clippy --all-targets --all-features -D warnings` both clean.
- **`finding.rs`'s `map_table_id` doc comment falsely claimed detector-run
  `catch_unwind` covered it (Task T2.3).** `map_table_id` runs in `project_finding`,
  AFTER `run_detectors` has already returned — it was never inside any per-detector
  isolation boundary (neither the old `catch_unwind` nor the new `Result` contract).
  The comment now states the true safety characterization: a malformed TableId here
  is an engine bug (every TableId a detector emits is internally constructed), so it
  remains a hard panic, matching al-sem's uncaught throw.
- **The never-written failing-detector test (Task T2.3).** No test anywhere
  constructed a panicking or failing detector to exercise the isolation contract;
  `registry.rs` now has `err_returning_detector_degrades_to_warning_others_still_run`
  and `panicking_detector_degrades_to_warning_others_still_run`, asserting the exact
  diagnostic message/severity/stage and that the other registered detector's finding
  still appears.
- **`EnumExtensionTypes` was missing from the engine ABI ingestion's BARE table
  (Task T4-C medium (b), Tier-4 hygiene arc).** `symbol_reference.rs`'s `BARE`
  lookup table (`EnumTypes`/`ControlAddIns`/`PermissionSets`/etc.) lacked an
  `("EnumExtensionTypes", "EnumExtension")` entry, so a dependency `.app`'s
  enum-extension objects never entered `objects` at all — the legacy
  `app_package.rs` parser has always read this field. `program::abi_ingest`
  already normalized both `"enumextension"`/`"enumextensiontype"` strings to
  `ObjectKind::EnumExtension`, so the consumer side was always ready; only the
  producer was missing the entry. Added the 7th BARE tuple + a regression test.
- **A caller-scope param/local lookup used raw first-match-wins `Vec::iter().
  find()` with no duplicate-name awareness at all (Task T4-C medium (e),
  Tier-4 hygiene arc).** `program::resolve::receiver::caller_scope_symbol` (the
  ONE shared param→local→named-return→global lookup `receiver.rs`'s Step 2 and
  `arg_dispatch.rs`'s `type_one_arg` both use) picked whichever param/local
  happened to be first in declaration order when a name matched more than
  once — including a genuine `#if`/`#else` union-read where each branch
  declared the SAME name with a DIFFERENT type (al-syntax does not evaluate
  preproc conditions, so both branches' declarations survive into the IR).
  This silently returned an arbitrary, possibly-wrong type instead of
  declining, unlike its established siblings (`ResolveIndex::field_in_table`,
  `resolve_dataitem_source_table`), which already dedupe identical duplicates
  and decline on a genuine conflict. Added `dedupe_type_hits` (mirrors
  `field_in_table`'s provenance-dedup pattern) and applied it to params,
  locals, AND globals; a genuinely conflicting duplicate now returns
  `CallerScopeSymbol::MalformedDuplicate` (decline) instead of guessing.
  3 regression tests (identical-dedup resolves; conflicting param/local
  declines).
- **L4 JACOBI fixed-point cap-hit shipped partial summaries as silently
  definite (Task T4-C, Tier-4 hygiene arc).** `summary_runner::run_one_scc`'s
  cap-hit branch (`MAX_FIXED_POINT_ITERATIONS = 1000`) only `eprintln!`'d a
  warning and returned the last, unconverged in-progress summaries with no
  marker distinguishing them from settled ones — a real risk given the
  transfer function is non-monotone via `apply_call`. Fixed two ways,
  additively: (1) every capped SCC member's `RoutineSummary` now gets a
  `fixpoint-capped` `Uncertainty` (the SAME mechanism detectors already read
  via `uncertainties_by_node` — no parallel channel), and (2) a new
  `SummarizeDiagnostic` (severity "warning", stage "summarize") threads from
  `run_one_scc` through `compute_summaries`/`compute_summaries_with_leaves`
  (now 3-tuple returns) into `DetectorContext.summarize_diagnostics`, then
  `RunOutput.summarize_diagnostics`, filling the "summarizeDiagnostics" TS-order
  slot 3 that `gate/run.rs` had tracked as an explicit gap since the al-sem
  6-source diagnostic concat was ported. Empty on every SCC that converges
  (the overwhelming common case), so this is CDO byte-identity-preserving.
- **d44/d45 computed their per-event/per-publisher output-cap truncation count
  and threw it away (Task T4-C, Tier-4 hygiene arc).** Both detectors called
  `group_and_cap` and destructured `(kept, _truncated)`, discarding the second
  element. Now surfaced via `stats.add_skip("outputCapped", truncated)` — the
  existing present-iff-nonzero `DetectorStats.skipped` mechanism, so output is
  unchanged whenever no event/publisher exceeds its cap.
- **`tests/r3b_incremental_nondeterminism.rs`'s corpus sweep used the exact
  shipped silent-skip shape this whole remediation program exists to kill
  (Task T4-B, Tier-4 hygiene arc).** `sweep_fixtures()` read the ~40-fixture
  discovered slice from `tests/r3a3-goldens` via `if let Ok(rd) =
  std::fs::read_dir(...)` — if that directory ever moved or was renamed, the
  discovery step would silently no-op instead of failing, leaving only the 6
  hardcoded fixtures. The two floors gating on `checked` asserted only `>= 5`,
  which the 6 hardcoded fixtures satisfy alone — so an evaporated corpus sweep
  would pass green with 88% less coverage and no signal anything had changed.
  Replaced the silent `if let Ok` with `.expect("read r3a3 goldens")`
  (matching siblings `r3b_minimality.rs` and `r3b_incremental_equality.rs`,
  which already hard-require) and raised both floors from `>= 5` to `>= 30` —
  well above the 6-fixture evaporation case, below the ~46 the current corpus
  discovers. Verified both directions: the full suite passes at 46 fixtures
  swept; temporarily renaming `tests/r3a3-goldens` now hard-panics with a clear
  message instead of silently passing.
- **Three CLI flag-honesty defects across `aldump`/`alsem`/the main binary
  (Task T4-B, Tier-4 hygiene arc): a mutual-exclusion array missing three mode
  flags, two dead `alsem prove` flags plus a fingerprint flag that flipped
  classification behavior but never updated its own honesty field, and the
  main binary silently blocking on stdin instead of erroring.**
  - **`aldump`'s mode mutual-exclusion array omitted `--graphify-export` /
    `--graphify-export-fragments` / `--integration-points`.** Each guards its
    own dedicated `if`-block exactly like the other 25 mode flags, but a combo
    like `aldump --graphify-export --l3-call-graph ws` was silently accepted
    and ran whichever block's `if` happened to come first in source order,
    dropping the other flag with no diagnostic. Added all three to the array
    and the error message.
  - **`alsem prove --no-roots-config` and `--alpackages` were parsed but never
    read, and had no possible effect even if wired: `run_prove_pipeline` never
    consults `resolved.root_classifications` or any cross-app dependency
    path**, unlike `fingerprint` (whose whole output is a walk over root
    classifications). Implementing "skip roots.config.json" or "load
    .alpackages deps" for `prove` would be invisible theater, not a real
    feature — removed both flags (clap now rejects them as unknown arguments,
    per the "unknown flag beats silent no-op" policy) instead of faking
    support.
  - **`alsem fingerprint --no-roots-config` was parsed but never read, and its
    own `rootsConfigIgnored` output field was hardcoded `false` — actively
    contradicting the flag whenever a `roots.config.json` existed.** Unlike
    `prove`, `fingerprint`'s root-classification pool IS the primary output
    surface, so this one has a real, obvious, small implementation:
    `assemble_and_resolve_workspace` gained a `skip_roots_config: bool`
    parameter (threaded through as `false` from its 7 other call sites,
    preserving their behavior exactly) that skips the `roots.config.json`
    overlay when set; `fingerprint_query` gained a `roots_config_ignored: bool`
    parameter so its caller (which knows both the flag and whether the file
    existed) can report the truth instead of a hardcoded value. Verified live:
    with a fixture's `roots.config.json` present, `--no-roots-config` now
    correctly emits `inputsMetadata.rootsConfigIgnored: true` (previously
    always absent) and the config-sourced root classification it names
    disappears from the AST-only pool, as expected.
  - **The main `al-call-hierarchy` binary's `--lsp` flag was parsed but never
    consulted**, so `--project X --lsp` silently ran CLI/analyze mode instead
    of the LSP server it explicitly asked for. **`--analyze` without
    `--project` fell through to the default branch and silently blocked
    forever as an LSP server reading stdin**, with no indication anything was
    wrong. Fixed both in `main.rs`'s dispatch: `--analyze` without `--project`
    now hard-errors immediately (`--analyze requires --project <path>`,
    checked before any mode dispatch so it fires even if `--lsp` is also set);
    `--lsp` now has real, unconditional top precedence — it always starts the
    LSP server regardless of `--project`/`--analyze`. Verified live: both new
    behaviors observed end-to-end (hard error; genuine LSP-mode startup with
    `--project` present).
- **`fingerprintQuery`'s `SelectorAmbiguous.candidates` reordered run-to-run
  because the index feeding it was rebuilt from `HashMap` iteration (Task T4-B,
  Tier-4 hygiene arc).** `fingerprint_query.rs` cloned the shared
  `routine_display_by_id: HashMap<String, String>` and iterated it directly to
  build the display→stable-ids selector bucket — both the bucket order and the
  per-bucket id order were process-random, so an ambiguous-selector diagnostic's
  printed candidate list (truncated at `MAX_AMBIGUOUS_CANDIDATES=16`) reordered
  between runs, and past the truncation point the displayed SET could change.
  `digest_cli.rs::build_selector_indexes` already built the equivalent index
  correctly, from the identity table's source `Vec` (deterministic by
  construction) — the two implementations had drifted. Extracted the shared
  logic (`build_selector_indexes` / `resolve_selector` / `normalize_display_key`
  / the 5-form `resolveSelector` cascade) into a new `selector_index` module and
  pointed both call sites at it, so they cannot re-diverge. Added a
  process-deterministic regression test asserting the KNOWN correct
  (identity-table insertion) order for a 4-way-ambiguous selector.
- **The `al-syntax` lowerer silently dropped preproc-split procedures, case
  branches, and statement-position `#if`/guarded constructs, and let comments
  pollute positional argument reads — including silently unregistering a whole
  `[EventSubscriber(...)]` (Task T1.4, Tier-1 remediation arc).** The corpus had
  ZERO `#if` fixtures before this task, so three fleet-confirmed lowering
  defects (grammar-verified with `tree-sitter parse`, not just read from
  `grammar.js`) went undetected: lost code produced no edge AND no `unknown` —
  it silently ceased to exist. All work is in `crates/al-syntax/src/lower/mod.rs`
  (+ one engine-side test in `src/program/resolve/event.rs`, the only site
  outside `al-syntax` this task touched).
  - **H-6 (`preproc_split_procedure` dropped entirely).** `collect_routines`
    matched only `Procedure|TriggerDeclaration|InterfaceProcedure` — the
    catch-all recursed without ever creating a `RoutineDecl` for a procedure
    whose HEADER differs across `#if`/`#else` but whose BODY (compiled into
    every build) is shared. Fixed by adding `PreprocSplitProcedure` /
    `PreprocSplitProcedurePreamble` to the match (both grammar-INLINE their
    header/body fields onto the wrapper node, so `.field()` resolves correctly
    — VERIFIED with `tree-sitter parse`, first-branch-wins for the
    per-branch name/parameters, mirroring `PreprocSplitDeclaration`'s
    established policy); `lower_routine`'s body extraction gained a dedicated
    fallback for `_preamble`'s BARE trailing `code_block` (no `body` field at
    all, unlike the plain variant).
  - **H-7 (case/statement preproc variants + `#region` markers).**
    `lower_case_body` matched only plain `CaseBranch`, silently skipping
    `preproc_conditional_case` / `preproc_split_case_extended` entirely and
    fabricating an EMPTY branch for `preproc_split_case_branch` (whose
    `pattern`/`body` fields live on a NESTED node the grammar still wraps in an
    outer `case_branch` — verified empirically, not assumed). Refactored into
    `collect_case_branches`/`push_case_branch`/`case_patterns`/
    `lower_case_else_branch`: branches union additively (mirrors the
    `implements`-list union-read policy already established for `#if`), the
    singular `else_block` is first-match-wins. `lower_stmt`'s `_` catch-all
    recorded an issue and returned bare `Unknown` for the nine
    `preproc_split_if*`/`preproc_guarded_statement` grammar shapes without
    descending — new `lower_unmodelled_stmt` reconstructs a REAL `StmtKind::If`
    for the three shapes with clean `condition`/`then_branch`/`else_branch`
    fields (`preproc_split_if_statement`, `preproc_guarded_statement`,
    `preproc_split_if_else_statement`) and a real `StmtKind::Call` for
    `preproc_split_call_statement` (unambiguous by construction); the
    remaining flat/fragmented shapes and any leftover content (guard
    statements, extra `#elif`/`#else` arms, a `preproc_fragmented_else_tail`)
    are recovered generically through the SAME `lower_block_child` dispatcher
    real block content uses, wrapped in `StmtKind::Block` — never a silent
    drop, never a fabricated empty block. `#region`/`#endregion` markers
    (previously absent from `lower_block_child`'s statement-position skip-list)
    were producing a phantom `Unknown` statement for a construct with zero
    content; added to the skip-list.
  - **H-8 (trivia as named extras).** A `comment`/`multiline_comment` is a
    legal named child almost anywhere, so a bare `named_children()` scan let
    it silently occupy a real positional slot: as the sole child of a
    `parenthesized_expression` (replacing the real inner expression), as a
    phantom argument in a `call_expression`'s arguments (breaking arity-exact
    dispatch), and — most consequentially — in an `[EventSubscriber(...)]`'s
    `attribute_argument_list`, shifting every later positional read in
    `parse_event_subscriber_ir` and silently unregistering the WHOLE
    subscriber. Fixed with ONE shared `structural_children` helper (filters
    `Class::Trivia`) applied at all three al-syntax sites; the root cause of
    the `EventSubscriber` case was upstream in al-syntax's attribute-arg
    lowering, not `event.rs` itself, so no engine-side fix was needed there —
    only a regression test proving the pipeline end-to-end.
  - 9 new fixtures (8 `al-syntax` unit tests + 1 engine-side
    `event.rs` test), each proven red-then-green against the pre-fix code (a
    representative H-6 spot-check is recorded in the test module; the rest
    follow the same `tree-sitter parse`-verified methodology). A test-only
    `call_reachable` walker mirrors `extract.rs`'s real `walk_block_v2`/
    `walk_stmt_v2` `Block`/`Stmt` LINKAGE (not a bare arena scan) to prove
    recovered calls are actually DISCOVERABLE by the call-graph walker, not
    just present-but-orphaned in the IR arena.
  - Blast radius: the full workspace `cargo test` (2502 tests, 158 binaries)
    is byte-identical — ZERO existing golden/differential fixture touches any
    of these constructs (the corpus's `#if` coverage was zero before this
    task), so this change is purely additive for all existing content.
    `cargo clippy --all-targets --all-features` clean.
  - CDO verification (`aldump --program-call-graph-stats`): `primaryScoped`
    moved `total` 18108→18113 (+5), all five landing in `resolvedSource`
    (8279→8284); `wholeProgram` moved `total` 43408→43414 (+6: +5
    `resolvedSource`, +1 `honestEmpty`). `unknown` held at 0/0 and
    `ambiguousResolved` held at 0 in both scopes throughout — the move is
    entirely NEW resolved edges, never a regression. Fully attributed: CDO's
    `Codeunit 6175280 CDO E-Mail.al` (`SetFromAddress`/`GetDefaultFromAddress`
    area) has a real `preproc_split_case_extended` (`#if DOSMTP` inside a
    `case`) whose extra branch (`SetFromAddressWithDOSMTPSetup`,
    `SetDOSMTPCode`) and shared body (`EmailAccountCU.IsAnyAccountRegistered`,
    `EmailScenarios.GetDefaultEmailAccount`, `SetBCEmailAccount`) were
    entirely invisible to the call graph pre-fix — exactly 5 calls, exactly
    matching the `primaryScoped.resolvedSource` delta. `scripts/cdo-gate`
    (`ENFORCE_CDO_WS=1`) green.
  - **T1.4 review follow-up: two more sibling-field gaps in these same shapes**
    (found by re-verifying live against `tree-sitter parse` output rather than
    trusting a grammar read). `preproc_split_procedure_body` /
    `preproc_split_complete_body` are NAMED (non-inlined) choice arms of
    `procedure`/`trigger_declaration`'s BODY position — unlike
    `_routine_regular_body`'s inlined `field('body', code_block)`, neither
    wrapper's own `field('body', ..)` flattens onto the routine node itself,
    so `lower_routine`'s existing `_preamble`-only fallback never caught them:
    a plain procedure whose var/begin section is `#if`-split lowered to
    `RoutineDecl.body == None`, zero issues, its call silently gone.
    `lower_routine` gained a third fallback
    (`lower_preproc_split_routine_body`): `preproc_split_complete_body`'s
    mutually-exclusive `#if`/`#elif`/`#else` arms use first-branch-wins
    (`.field(Body)`, single match, mirroring `PreprocSplitDeclaration`'s
    established policy); `preproc_split_procedure_body`'s `#if`-branch content
    and its trailing SHARED tail are NOT alternatives — both inlined
    sub-rules' `body` fields flatten onto the SAME wrapper node (grammar-
    VERIFIED with `tree-sitter parse`: two separate `body:`-tagged
    `statement_block` children), so they are union-read via
    `children_by_field(Body)` instead, preserving both. Separately,
    `preproc_split_code_block_end` (the closing `end` of a `code_block` itself
    split across `#if`/`#else`) is a SIBLING of the `code_block`'s own `body`
    field, never nested inside it — `lower_code_block`'s old
    `cb.field(Body).unwrap_or(cb)` trick only ever exposed this sibling when
    `body` was ABSENT; a `code_block` with real LEADING content before the
    split took the `Some(body)` arm and never looked at the sibling again,
    silently dropping its content (including, grammar-verified, a following
    unconditional `else` clause the grammar folds into the SAME node).
    `lower_code_block` now always checks for the sibling and recovers it
    through the same generic `lower_block_child` dispatcher, folded flat into
    the SAME block. A misleading doc comment on `lower_unmodelled_stmt`
    (claiming this shape was "recovered generically through
    `lower_block_child`" — only ever true when `body` was absent) is
    corrected to point at `lower_code_block` instead. 4 new red-then-green
    `al-syntax` fixtures (proven red against the pre-fix code, matching the
    parent task's methodology); full workspace `cargo test` and
    `cargo clippy --all-targets --all-features` both clean, `rustfmt` applied.
- **L4 dataflow walker only saw the FIRST statement of every `repeat…until`
  body, and `break`/`continue` lowered to an inert `other` CFN kind that
  silently threaded state through statements they actually skip (Task T1.1,
  Tier-1 remediation arc).** Two coupled unsoundness bugs in
  `src/engine/l4/cfg_walker.rs`'s branch-aware `walk_cfg`:
  1. The `"repeat"` arm took `node.children.first()` — but `repeat` bodies are
     lowered FLAT (`src/engine/l2/ir_walk.rs`'s `Repeat` case), unlike
     `while`/`for`/`foreach`'s single wrapped block child, so any multi-
     statement `repeat` body was silently truncated to its first statement.
     Fixed by wrapping the flat children in a synthetic `"block"` CFN node
     (mirroring the same pattern already used by
     `control_context.rs::walk_loop_node` and
     `operation_order.rs::walk_loop_node` for this exact shape) so the walk
     reuses the `"block"` arm's sequential/field-interleave logic over ALL
     statements.
  2. `break`/`continue` now lower to their own CFN kinds (`"break"`/
     `"continue"`, `src/engine/l2/ir_walk.rs`) instead of the inert `"other"`.
     `walk_cfg` gained a `Reach` signal (`Normal` / `Abrupt`) threaded through
     every arm: a `break`/`continue` leaf pushes its at-break/at-continue
     state onto a new per-loop `LoopFrame` (a stack scoped to the innermost
     enclosing `while`/`for`/`foreach`/`repeat`) and returns `Abrupt`, which
     propagates up through `if`/`case`/`block` so statements after an
     unconditional break/continue in the same block are correctly treated as
     dead for that path. Each loop arm folds its frame's `breaks` into the
     loop's own exit state (once, after the bounded fixed-point settles) and
     its `continues` into the loop-head join (cleared every iteration,
     mirroring `continue` jumping straight to the condition re-check). The
     break/continue defect was previously MASKED inside `repeat` bodies by
     defect 1 (truncation meant a body-final `break`/`continue` was often
     invisible entirely) and would have gone live unsoundly the moment defect
     1 was fixed in isolation — the two fixes landed as one change.
  New TDD fixtures in `tests/r3a2_branch_aware.rs` (all confirmed red on the
  pre-fix code, green after): a multi-statement `repeat` body where a later
  statement resets `dirtyAtExit` (bug 1 in isolation, no break involved); a
  `while` loop with a conditional `break` between two state-changing
  statements exercising the exit-join (bug 2 in isolation, no repeat
  involved); and a `repeat` body combining both. Full `cargo test` green (157
  binaries); `REGEN_TEMP_GOLDENS=1` regen touched exactly one Rust-owned
  golden family with a real content diff — see the T1.1 report for the full
  blast-radius table. CDO verification: `aldump --program-call-graph-stats`
  SHA unchanged (`67910e99...13f4f`, byte-identical — this fix touches L4
  dataflow, not call resolution) and `scripts/cdo-gate` green.

### Added
- **Golden-regen completeness + value-gated `REGEN_TEMP_GOLDENS` (Task T0.6,
  Tier-0 remediation arc).** Doctrine says every Rust-owned golden regenerates
  via `REGEN_TEMP_GOLDENS=1 cargo test`; review proved 10 golden-writing
  functions across 8 families had no regen path at all (R0 identity, R2c L3
  event-graph, r3a2-trace, cli-c policy [4 sub-tests], cli-c cache [2
  sub-tests], and all 5 R4-F families [digest-effects, ordering-facts,
  return-summaries, root-classifications, scoped-guarantees]) and that the
  regen trigger itself was presence-tested everywhere (`.is_ok()`/`.is_err()`),
  so `REGEN_TEMP_GOLDENS=0 cargo test` silently rewrote every golden while
  reporting green. New `tests/common/regen.rs` (the `#[path]`-included pattern
  established by `tests/common/cdo.rs`, Task T0.2) provides one shared,
  unit-tested `regen_mode()`: only the exact value `"1"` regenerates. Wired
  into all ~30 golden-writing call sites across ~38 test files (replacing
  every ad-hoc `std::env::var("REGEN_TEMP_GOLDENS").is_ok()`/`.is_err()`), plus
  a colocated mirror in `src/parser.rs`'s own `#[cfg(test)]` module (a lib
  unit test can't reach across the `src/`/`tests/` boundary via `#[path]`).
  Added the 10 missing regen paths, each proven to reproduce its committed
  golden byte-for-byte from the unchanged engine (R1). Wired all 9 previously-
  decorative `manifest.json`/`suppress-baseline-manifest.json` oracle files
  (read by zero tests — a silently deleted golden passed unnoticed) into a new
  floor-check test per family (`discovered >= manifest's fixtureCount/
  totalGoldens`; `>=` not `==` since several corpora have legitimately grown
  past their frozen al-sem-era snapshot count). `tests/r0-goldens/README.md`
  rewritten to describe the real mechanism (previously documented a regen path
  for R0 identity that did not exist in code).
- **Performance targets: measured + CI-asserted generous bounds (Task T0.5,
  Tier-0 remediation arc).** CLAUDE.md's Performance Targets table (initial index
  <500ms/100 files, <2s/1000 files; prepareCallHierarchy/incomingCalls/outgoingCalls
  <1ms; file change update <50ms) was measured NOWHERE: `benches/telemetry_hot_path.rs`
  registered only `bench_disabled`, CI ran fmt/clippy/test/build and no bench, and no
  test asserted any timing bound against the legacy LSP pipeline. New
  `tests/perf_support/` is a deterministic (index-driven, no RNG) synthetic AL corpus
  generator — N codeunits with a real cross-file call topology (every non-hub file's
  `Proc0` makes one qualified call into a designated hub codeunit plus 2 local calls),
  giving `incomingCalls`/`outgoingCalls` genuine fan-in/fan-out rather than an
  all-isolated corpus. New `benches/lsp_pipeline.rs` (Criterion, `cargo bench --bench
  lsp_pipeline`) measures initial index of 100/1000-file corpora, the 3 call-hierarchy
  query handlers against a 1000-file indexed graph, and a single-file reindex — all
  in-process, no LSP stdio loop. New `tests/perf_bounds.rs`, compiled for real only
  under `#[cfg(not(debug_assertions))]` (a debug-build timing assert is meaningless;
  an always-present marker test guarantees the binary never silently reports 0 tests),
  asserts every operation stays within 3x its CLAUDE.md target (USER DECISION, binding:
  generous margins accept occasional flake on loaded CI runners in exchange for
  catching real order-of-magnitude regressions), using a warm-up pass plus a median of
  3-5 timed runs. `.github/workflows/ci.yml` gained a `cargo test --release --test
  perf_bounds` step reusing the existing release build. CLAUDE.md's perf table now
  carries measured numbers alongside each target (all with wide headroom: e.g.
  1000-file initial index ~15.9ms against a 2s target).
  **Enabling refactor:** `graph.rs`/`indexer.rs`/`handlers.rs`/`parser.rs`/`protocol.rs`
  were bin-only modules (declared in `main.rs`), invisible to bench/test targets that
  only link the library crate — benching them required exposing them. Moved module
  ownership to `lib.rs` (`pub mod`) and re-exported from `main.rs` via `pub use
  al_call_hierarchy::{...}`, extending the pattern the repo already used for
  `config`/`telemetry`/`app_package`/`dependencies` (whose own doc comment already said
  this was "so library consumers \[i.e. benches\] can use them"). Fixed the one
  self-crate-reference this exposed (`graph.rs`'s `ObjectType` re-export) and widened
  one `pub(crate)` function (`parser::routine_complexity_ir`) to `pub`, since `main.rs`
  now consumes it across a real crate boundary. The 3 handler functions
  (`prepare_call_hierarchy`/`incoming_calls`/`outgoing_calls`) are now `pub fn` (were
  private) so benches/tests can call them directly. Zero behavior change: all 1340 lib
  tests + 24 bin tests pass unchanged (92 of the 1340 are the graph/indexer/handlers
  suites, now running as part of the lib target instead of the bin target).
- **Builtin-dispatch justification audit — pinned-baseline ratchet (Task T0.3,
  Tier-0 remediation arc).** The north-star real-`unknown` rate cannot see a
  missed dispatch edge that lands in `builtin` instead: `Page.RunModal(Page::"X")`
  (a keyword receiver + `DatabaseReference` argument) and a declared
  Page/Report-typed variable's `.RunModal()` both currently resolve as an
  ordinary `Evidence::Catalog` `Builtin` route (`PageInstance::runmodal` /
  `ReportInstance::runmodal`) instead of an entry-trigger `Run` edge into the
  named target — two separate classifier gaps (`extract::classify_call`'s
  `ObjectRun` check only recognizes method `"run"`, never `"runmodal"`, for a
  keyword receiver; `resolver::resolve_member_with_args`'s `Object{kind,
  name_lc}` arm never special-cases a declared Page/Report variable's
  `Run`/`RunModal` as an entry dispatch at all — only `Codeunit.Run` has that
  special case). New `member_catalog::ENTRY_DISPATCH_BUILTIN_IDS` names the 4
  flagged catalog entries (`PageInstance`/`ReportInstance` × `run`/`runmodal`;
  Codeunit/XmlPort/Query excluded with documented MS-Learn-grounded reasoning).
  `resolve_full_program`'s `ProgramReport` gained an ADDITIVE
  `builtin_dispatch_audit: BuiltinDispatchAudit` field (sorted `flagged: Vec<
  FlaggedBuiltinDispatchSite>` + `indeterminate: Vec<IndeterminateBuiltinDispatchSite>`),
  computed inline in `resolve_call_site_obligation`'s `CalleeShape::Member` arm
  from data already in scope (the resolved `ReceiverType` + the call's raw
  argument expressions) — no change to `Route`/`Edge`/`CalleeShape`, no change
  to any resolution outcome or histogram (CDO `aldump
  --program-call-graph-stats` SHA-256 confirmed byte-identical to the frozen
  baseline). Fail-closed: a flagged method whose target cannot be PROVEN
  static (e.g. a runtime-variable `RunModal` argument) is reported separately
  as `indeterminate`, never guessed into `flagged`. `extract::classify_call`'s
  existing `DatabaseReference`-target extraction was factored into a shared
  `static_database_reference_target` helper so the audit and the `ObjectRun`
  classifier check can never drift. New fixture `tests/r0-corpus/
  ws-builtin-dispatch-audit` proves the audit flags exactly its 3 statically-named
  RunModal sites (both populations) and marks 1 dynamic-target call
  indeterminate, with zero `CDO_WS` dependency. New CDO-gated ratchet test
  `cdo_builtin_dispatch_audit_flagged_count_is_pinned` (via the shared
  `cdo::cdo_ws_or_enforce()` helper) pins the measured real population —
  **94 flagged sites, 13 indeterminate** — as a binding, user-decided
  pinned-baseline ratchet (mirrors the `ambiguousResolved=56` precedent): the
  pin holds the gate green until Task T1.3 lands the classifier fix, at which
  point it drops (verified stable across 2 consecutive CDO runs, byte-identical
  flagged-site lists both times).
- **`scripts/cdo-gate` — the local release-gate runner for the CDO ratchet
  (Task T0.2, Tier-0 remediation arc).** A Git-Bash-compatible shell script
  that requires a CDO workspace path (positional arg or `CDO_WS` env var —
  refuses with exit 2 and a clear message if neither is set or the path
  doesn't exist; never hardcodes a machine-specific default), exports
  `ENFORCE_CDO_WS=1`, runs `cargo test --release --test
  program_resolve_harness -- --test-threads=1` followed by `cargo test
  --release --test program_graph --test snapshot_robustness`, and exits
  non-zero with a one-line `cdo-gate: PASS`/`cdo-gate: FAIL` summary if
  either step failed. CI cannot reach the CDO workspace, so this is meant to
  be scheduled locally (cron / Task Scheduler) — see the new CLAUDE.md
  testing note. `.gitattributes` gained a `scripts/* text eol=lf` rule so
  `core.autocrlf=true` checkouts don't corrupt the shebang line.
- **ABI param-type retention — the SymbolOnly arg-type dispatch lift, behind
  a structural guard (Task 2, roadmap-closure plan).** `AbiParameter`
  (`engine/deps/symbol_reference.rs`) already carried full parameter Subtype
  fidelity (`type_text` + `is_var` + `subtype_id`/`subtype_raw_name`/
  `subtype_tag`, from the sigfp-and-ambiguous-reclassification plan), but
  `abi_ingest.rs` folded it into `sig_fp` and then discarded it
  (`param_sig_key: String::new()`) — the argtype-dispatch-and-page-catalog
  plan's fail-closed overload pick (`arg_dispatch::pick_candidate`) was
  therefore gated `obj_tier != TrustTier::SymbolOnly`, permanently inert for
  every ABI (dependency-boundary) overload set. New `RoutineNode.abi_params:
  AbiParams` — a STRUCTURAL enum (`Complete(Vec<AbiParamRetained>) | Missing
  | CollapsedUntrusted`), not a plain `Option`, so a collapse-marked
  survivor's parameters are impossible to read BY TYPE: `abi_ingest::
  retain_abi_params` populates `Complete`/`Missing` at ingestion (tri-state
  arity — `Missing` mirrors "arity unknown", never a false empty list), and
  `build::dedup_routines_preserving_genuine_overloads` demotes to
  `CollapsedUntrusted` in lockstep with the existing `abi_overload_collapsed`
  marker (the SAME survivor, the SAME collapse condition). New
  `arg_dispatch::candidate_param_infos_abi` + `abi_param_canonical`: the
  ABI-AWARE canonicalization route resolves an object-typed parameter's
  Subtype via the SAME semantic object identity a source parameter's
  declared text resolves through (`ResolveIndex::resolve_object_ref`) —
  `Record 36` and `Record "Customer"` canonicalize identically iff they
  resolve to the SAME table, reaching PAST a degraded `type_text` (an
  `id_only`/`name_quoted` Subtype) via the raw discriminator fields instead
  of guessing from text; a genuinely unresolvable/absent Subtype degrades
  that parameter to untyped, which degrades the WHOLE call (never a partial
  or filtered read). The `resolver.rs` gate lifts from "SOURCE tier only" to
  a per-candidate `candidate_param_infos_either` (BodyMap first, the
  ABI-AWARE route only when BodyMap has no entry for that candidate) — "no
  BodyMap entry", not `rid.object`'s tier, is the trigger, so the two routes
  can never disagree about which one applies. `is_var` carries through as
  real `by_ref` fidelity, so the pre-existing ByRef-EXACT rule now also
  protects ABI candidates. 8 new unit fixtures (`arg_dispatch.rs`): distinct
  -scalar-type ABI overloads pick correctly; a `var` ABI param eliminates a
  literal argument; a `CollapsedUntrusted` survivor declines unconditionally
  (the enum makes the read impossible, regardless of how discriminating the
  original raw params might have looked); a `Missing`-metadata candidate
  declines; a lookup-miss declines without panicking; Record-id-vs-name
  Subtype equality; an unresolvable Subtype degrades; a scalar keyword's
  ordinary base-keyword route is unchanged. Plus 2 new fixtures
  (`abi_ingest.rs`): one REAL generated `SymbolReference.json` fragment
  (method `RegisterAssistedSetup`, genuinely declared on `Table 6192869
  "CSC Temp. Assisted Setup"` — extracted verbatim from
  `Continia Software_Continia Core_29.0.0.94574.app` in the CDO workspace's
  own `.alpackages`, including its real extra `ModuleId` field on
  `Subtype` — proving the parser tolerates real-world JSON shapes, not just
  hand-authored text; the fixture wraps it in a fabricated Codeunit
  wrapper, since only the Methods[] content carries test signal); and the
  tri-state-arity sibling (`parameters_known == false` retains as
  `Missing`, never a false empty `Complete`). 3 new end-to-end fixtures
  (`resolver.rs`):
  a `Missing`-metadata ABI candidate in an otherwise-pickable set degrades
  the whole call rather than resolving the `Complete` sibling confidently;
  a real-source-plus-hand-injected-ABI "mixed" candidate set (proving the
  per-candidate helper's contract directly, since one `ObjectNode` cannot
  legitimately carry two tiers at once) picks correctly when both sides are
  complete, and declines on the ABI side alone when it is not (the
  no-filtering rule). Full CDO harness BYTE-IDENTICAL to the frozen
  `.superpowers/sdd/cdo-baseline-plan13.md` baseline — CDO has ZERO
  SymbolOnly routines with retained ABI parameters exercised by this path,
  so the lift is fixture-proven but CDO-dormant, exactly as the plan
  predicted.

### Changed
- **`default-run = "al-call-hierarchy"` restores bare `cargo run` (Task T4-A
  review fix).** The crate grew multiple `[[bin]]` targets, so bare `cargo run`
  errored on ambiguous binary selection — contradicting the very first command
  the docs show. `default-run` pins the LSP binary as documented.
- **CLAUDE.md + README rewritten against the real tree (Task T4-A, Tier-4
  hygiene arc).** Onboarding docs still described a retired system: "Adding New
  AL Constructs" pointed at `language.rs` tree-sitter query consts with zero
  execution repo-wide (owned-IR migration retired them); "Key Modules" named a
  nonexistent top-level `resolver.rs` and omitted `src/engine/`, `src/program/`,
  `crates/al-syntax/`, `src/bin/` entirely; a "V2 grammar" section directly
  contradicted the v3 section below it and instructed using
  `node_util::block_statements` — a deleted function; both README and
  CLAUDE.md documented a `--no-lsp` CLI flag that `clap` now hard-errors on;
  README pointed the grammar submodule at `../tree-sitter-al` (outside the
  repo) instead of the real in-repo `tree-sitter-al/` submodule; the
  Resolution Coverage table ("Record methods: Partial") predated the entire
  resolution program. Rewrote: Architecture/Data Flow now documents BOTH
  pipelines honestly (the LSP surface — `graph.rs`/`handlers.rs`/etc., now lib
  modules — and the program engine — `snapshot` → al-syntax IR →
  `program::resolve` → `Histogram` report); Key Modules lists the real tree
  including `src/engine/{l2,l3,l4,l5,gate,deps}`, `src/program/{resolve,...}`,
  `crates/al-syntax`, `src/bin/{aldump,alsem,mint-goldens}`, plus a testing
  note on `tests/common/{cdo,regen}.rs` + `scripts/cdo-gate`; the Grammar
  section is now one coherent v3.2.0-reality section (V1→V2→V3 history kept,
  explicitly marked non-actionable for engine code since `al-syntax`'s
  lowerer now absorbs all grammar-shape handling); "Adding New AL Constructs"
  documents the real workflow (grammar → al-syntax lowerer → IR consumers →
  `REGEN_TEMP_GOLDENS=1 cargo test`); Resolution Coverage replaced with the
  honest taxonomy (`resolvedSource`/`resolvedCatalog`/`resolvedAbiExternal`/
  `conditionalResolved`/`honestDynamic`/`honestEmpty`/`unknown`/
  `ambiguousResolved`) and the CDO numbers last measured immediately after the
  Tier-1 merge (commit `f171d0f`; both scopes `unknown`=0, `ambiguousResolved`=0,
  `realUnknownRate`=0.0000%; JSON SHA-256
  `0a3b85bc832ff0a3e77acee118d203edbf62827dc37617c8d9315fe52d5cb7d0` — sourced
  from the coordinating session's own post-merge measurement, not
  independently re-run in this worktree, which lacks `CDO_WS` access); the two
  distinct "legacy" metric axes (engine axis: fresh resolver vs. legacy L3;
  definition axis: `realUnknownRate` vs.
  `realUnknownRateLegacyIncludingAmbiguous`) now get one explicit paragraph so
  neither gets conflated with the other again. Every documented command
  (`cargo run` flag set, `aldump --program-call-graph-stats`,
  `cargo bench --bench lsp_pipeline`/`cargo check --bench lsp_pipeline`) was
  run or built against this worktree before being written down; the CLI flag
  list was read from `src/main.rs`'s `Args` derive, not guessed. Docs-only —
  zero code changes.
- **BREAKING: legacy L3 histogram's `realUnknownRate` key renamed to
  `legacyL3UnknownRate` (Task T0.4, Tier-0 remediation arc) — one metric, one
  owner.** `aldump --l3-call-graph-stats` (legacy L3 engine) and `aldump
  --program-call-graph-stats` (fresh resolver, `resolve_full_program`) emitted
  DIFFERENT semantics under the identical `realUnknownRate` JSON key — the L3
  histogram excludes `memberNotFound`/`ambiguous` from `unknown`, the fresh
  engine counts `MemberNotFound` as `Unknown` — while CLAUDE.md's "Project
  Direction & The Moat" pointed the north-star measurement at the L3 command.
  A reader comparing the two numbers, or ratcheting the wrong one, got silently
  different answers. Per the roadmap's binding decision, the fresh resolver is
  now the SOLE authoritative metric: `realUnknownRate` is reserved exclusively
  for `--program-call-graph-stats`'s `wholeProgram`/`primaryScoped` output
  (byte-identical, unchanged — no metric-computation change anywhere in this
  task). All 4 L3-family JSON emission sites in `src/bin/aldump.rs`
  (`--l3-call-graph-stats`, `--l3-call-graph-stats-cross-app`,
  `--l3-unknown-breakdown`, `--l3-unknown-breakdown-cross-app` — the brief that
  scoped this task named only 2; scouting found all 4 emit the same
  `engine::l3::resolution_class::Histogram`, so the rename was applied
  consistently across all of them) now emit `legacyL3UnknownRate` plus an
  additive self-describing `"advisory": "legacy L3 engine; authoritative
  metric is --program-call-graph-stats"` field. No `Histogram` sibling field
  implies authority (`total`/`resolved`/`builtin`/`dynamic`/`external`/
  `ambiguous`/`memberNotFound`/`unknown` are all plainly descriptive), so only
  the one key needed renaming. CLAUDE.md's moat section now measures with
  `--program-call-graph-stats` and states the L3 command is legacy/advisory.
  Neither `graphify_export.rs` nor any `engine/l5` detector reads this JSON
  key programmatically — both consume L3 `CallEdge`s directly — so R6 is a
  clean no-op there; nothing to fix. 2 test files pinned the old key and are
  updated (`tests/l3cg_stats_smoke.rs`, `tests/aldump_smoke.rs`'s 3 T0.1
  fail-closed/good-path guards); no committed golden ever pinned
  `realUnknownRate` (grepped every file type), so no golden regen was needed.
  Gates: `cargo test` full workspace green (0 failed); `cargo clippy
  --all-targets --all-features -- -D warnings` clean; CDO's
  `--program-call-graph-stats` SHA-256 confirmed BYTE-IDENTICAL to the frozen
  baseline (`67910e992777b6bdef07b3b0046d1077c96cc03f581743d6404ee93d49913f4f`);
  `rg -n '"realUnknownRate"' src/` afterward shows exactly 2 hits, both inside
  `program_call_graph_stats`'s `wholeProgram`/`primaryScoped` blocks — the only
  emission sites left.
- **Docs + CLAUDE.md doctrine + a legacy-token TRIAGE (Task 3.6, al-sem
  parity retirement arc capstone).** `docs/engine-migration.md` moved to
  `docs/history/` (git mv) with an `ARCHIVED (2026-07-05)` header —
  historical migration narrative, not current guidance. (A same-task review
  fix caught that `docs/engine-gaps.md` was wrongly archived alongside it:
  24 sites across 6 `src/engine/l5` detector files and 17
  `tests/gap_g*.rs` files cite it by path as the live rationale for each
  detector-gap fix — it is moved back to `docs/engine-gaps.md` with the
  ARCHIVED header removed; `engine-migration.md`, which has zero code
  citations, correctly stays archived.) CLAUDE.md's two remaining now-false
  al-sem claims are corrected:
  the grammar-migrations line claiming "the goldens are the al-sem TS
  reference output … source of truth" now states the goldens are Rust-owned
  baselines and the Rust engine is the source of truth; the Testing
  Philosophy line claiming al-sem is merely "frozen, not a live oracle" with
  "LEGACY" tests still pointing at it now states retirement is COMPLETE
  (`al-sem-OBOLETE`, zero tests read it, `REGEN_TEMP_GOLDENS=1` regenerates
  every golden) — the false "some cli-b differentials, r3a1/r4f are LEGACY"
  sentence is deleted outright. A triage (not a purge — legitimate
  provenance/changelog/algorithm-oracle references survive) cleaned the
  remaining soft-pattern hits: `src/bin/alsem.rs`'s `ORDER_REJECTION` message
  (`digest --order`, confirmed a live, reachable, intentionally-hidden clap
  flag — NOT dead like the removed `--dump-model`) drops its "use the TS
  CLI" pointer, now stating the rejection full stop; 10 doc-comments citing
  `U:\Git\al-sem` as a byte-parity oracle or golden source
  (`src/engine/root_classification.rs` + 9 `cli_a_*`/`cli_b_*` differential
  test file headers) are reworded from present-tense ("byte-compares against
  … at `U:\Git\al-sem\…`") to past-tense provenance ("originally sourced
  from al-sem's …, now retired; vendored Rust-owned at `tests/cli-*-goldens/
  …`"); `tests/gate_sarif_differential.rs` and `tests/gate-goldens/
  manifest.json`'s `alsemVersionPin` field (neither byte-asserted by any
  test) are reworded to describe the current `--sarif-version-override` CLI
  flag instead of the dead `AL_SEM_VERSION_OVERRIDE` env var; the one
  surviving stale "used to shell `bun run`" retired-note comment
  (`cli_c_cache_differential.rs`) is trimmed to drop the dangling shell
  invocation while keeping the causal why (al-sem's symbolReader version is
  permanently stuck at 17 vs. the engine's 18). KEPT as legitimate: every
  `Bun`/`localeCompare` reference in `ids.rs`/`gate/cbor.rs`/
  `l5/ordering_facts.rs`/`l5/snapshot_full.rs` (ICU/DUCET collation + CBOR +
  gzip-level algorithm provenance, not al-sem parity); every "no Bun
  required" statement; every `PROVENANCE.md` fixture-origin note; every
  historical `docs/superpowers/{plans,specs,prompts}/*.md` entry from
  completed prior arcs (analogous to CHANGELOG — frozen historical record,
  not live doctrine). The HARD-token audit
  (`AL_SEM_DIR|AL_SEM_VERSION_OVERRIDE|AL_SEM_RELEASE|AL_SEM_DEV_FINGERPRINT|
  DEFAULT_ALSEM_VERSION|KNOWN_DIVERGENCES|dump[-_]model`) returns nothing
  outside `.git/` internals, `CHANGELOG.md`, `PROVENANCE.md` files, and the
  historical prior-arc plan/spec docs noted above.
- **6 differential harnesses gained a Rust-native `REGEN_TEMP_GOLDENS`
  rebaseline path, replacing the al-sem refresh they lost (Task 3.5, al-sem
  parity retirement arc).** `r2_5a_differential.rs`,
  `r2_5b_{cg,cov,eg}_differential.rs`, `r3a4_differential.rs`, and
  `cli_c_events_differential.rs` had no other regen mechanism once their
  al-sem-shelling refresh fns were deleted (see Removed, below). Each now
  mirrors the in-repo pattern already used by `differential.rs` /
  `r2_5b_rt_differential.rs`: at the existing actual-vs-golden comparison
  site, `REGEN_TEMP_GOLDENS=1` writes the ENGINE's own output to the golden
  file instead of asserting against it. `cli_c_events_differential.rs`
  needed the widest change (15 comparison sites across a test-generating
  macro + 6 standalone test fns) — a new `golden_or_regen(name, actual)`
  helper centralizes the write-or-read branch so every golden in a
  multi-golden test still regenerates even after the first write. All 6
  paths are env-gated (inert under a normal `cargo test`) and were each
  verified to reproduce their committed golden byte-for-byte (`git diff`
  empty after a `REGEN_TEMP_GOLDENS=1` run). `cargo test --release
  --workspace` stays green; CDO's `--program-call-graph-stats` SHA-256 is
  unchanged
  (`67910e992777b6bdef07b3b0046d1077c96cc03f581743d6404ee93d49913f4f`).
- **Vendored the 5 live-al-sem-read test files' inputs in-repo (Task 3.3,
  al-sem parity retirement arc) — tests are now self-contained.**
  `tests/aldump_smoke.rs`, `tests/al2dump_smoke.rs`,
  `tests/cli_b_diff_differential.rs`, `tests/cli_c_policy_differential.rs`,
  and `tests/cli_c_cache_differential.rs` no longer read from any al-sem
  checkout or `AL_SEM_DIR`, and no longer skip-gate when one is absent —
  missing inputs are now a hard test failure. 13 fixture trees (`ws-d2`, the
  10 `ws-policy-*` policy workspaces, `ws-diff-rename`,
  `ws-diff-removed-field`) were copied byte-for-byte from the frozen
  `al-sem-OBOLETE` archive into `tests/fixtures/`, each verified against the
  Task-3.0 witness SHA-256 listings before and after commit (a
  `tests/fixtures/** -text` `.gitattributes` rule, committed first, protects
  the bytes from EOL normalization); each vendored area carries a
  `PROVENANCE.md`. The cli-b diff snapshot-pair inputs and rename overlay
  (fixed test data, not engine output) were likewise copied verbatim from the
  witness into `tests/cli-b-goldens/diff/`. Every OUTPUT golden (the L3
  event-graph and L2 projections for ws-d2, all 24 cli-b diff outputs) was
  instead *regenerated from this engine* (Rust-owned baseline, via a new
  `REGEN_TEMP_GOLDENS=1` path in each file) and witness-diffed: all are
  byte-identical to the witness except `al2dump-smoke-goldens/ws-d2.l2.golden.json`,
  which differs only in JSON key order (a benign struct-field-reorder
  artifact — value-equal under order-independent comparison, confirmed by an
  independent canonicalized diff). The cli-c cache golden corpus's al-sem
  fallback (`al_sem_cache_goldens_dir()`) is retired outright — its goldens
  were already permanently stale (frozen at symbolReader version 17 while the
  engine is at 18) and unreproducible from al-sem, so the existing in-repo
  vendored override (`tests/cli-c-goldens/cache/`) is now the sole source.
  `cli_c_policy_differential.rs`'s one remaining al-sem touchpoint — a live
  byte-parity check of the bundled default policy against al-sem's source —
  is retired into a self-contained rule-count sanity check (al-sem is frozen
  forever, so a live drift check against it can never fire again; Task 3.3
  confirmed, one last time, that the two were still identical). All 3
  al-sem-shelling `#[ignore]` refresh tests are deleted (dead code once
  `REGEN_TEMP_GOLDENS` is the regen mechanism). Gate: all 5 files pass with
  `AL_SEM_DIR` pointed at a nonexistent path and no al-sem checkout anywhere
  on disk; CDO's `--program-call-graph-stats` SHA-256 is unchanged
  (`67910e992777b6bdef07b3b0046d1077c96cc03f581743d6404ee93d49913f4f`).
- **Retired the src-side al-sem parity shims (Task 3.2, al-sem parity
  retirement arc).** `alsem_version()` is renamed `driver_version()` and its
  default is now this crate's own `CARGO_PKG_VERSION` instead of the pinned
  al-sem `package.json` version (`0.0.12`) — the engine reports its own
  identity rather than impersonating the retired TS tool. The override env
  var is renamed `AL_SEM_VERSION_OVERRIDE` → `ALCH_DRIVER_VERSION_OVERRIDE`
  (test-harness sentinel values are unchanged, so display goldens are
  byte-identical). The dependency-cache header's `analyzer` stamp is now a
  dedicated `CACHE_ANALYZER_VERSION` const, decoupled from the display
  version (still `"0.0.12"` — cache goldens byte-identical). The cache
  release/dev-fingerprint env vars are renamed `AL_SEM_RELEASE` →
  `ALCH_RELEASE` and `AL_SEM_DEV_FINGERPRINT` → `ALCH_DEV_FINGERPRINT`
  (old names removed outright, not aliased — anyone relying on them for a
  warm cache gets a one-time re-fingerprint, not a correctness hazard).
  `policy check`/`digest`/`prove`/`fingerprint`/`events fanout`/`events
  chains`/`diff` also now report `driver_version()` instead of the retired
  `DEFAULT_ALSEM_VERSION` constant, which is deleted — every CLI path reports
  the same, honest identity; no al-sem version const or literal remains in
  `src/` except `CACHE_ANALYZER_VERSION`.

### Removed
- **The 14 al-sem/Bun-shelling `#[ignore]`d golden-refresh functions +
  `tests/r2_5b_refresh.rs` (Task 3.5, al-sem parity retirement arc) — no test
  anywhere touches `AL_SEM_DIR` or shells `bun run` anymore.** Deleted:
  `differential.rs`'s `refresh_goldens_from_al_sem` (plus its 4 now-orphaned
  helpers — `copy_source_fixture`/`copy_al_tree`/`git_sha`/
  `read_manifest_field`); `r2_5a_differential.rs`'s
  `refresh_r2_5a_goldens_from_al_sem` AND its sibling
  `r2_5a_fixtures_match_al_sem_bytes` byte-parity guard (also
  `AL_SEM_DIR`-gated — caught by grepping for the literal env-var string
  rather than trusting the "refresh" name, since it wasn't one);
  `r3a4_differential.rs`'s `refresh_r3a4_goldens_from_al_sem`;
  `r3a5_differential.rs`'s `refresh_r3a5_goldens_from_al_sem`;
  `r4_differential.rs`'s `refresh_r4_goldens_from_al_sem`; the
  `cli_a_html_differential.rs` / `cli_a_json_differential.rs`
  `refresh_goldens` fns and `cli_a_terminal_differential.rs`'s
  `refresh_terminal_goldens`; `cli_c_events_differential.rs`'s
  `refresh_goldens`; and `tests/r2_5b_refresh.rs` in its entirety
  (`refresh_r2_5b_goldens_from_al_sem`, which refreshed all 4 R2.5b sub-gates
  — rt/cg/eg/cov — in one shot). The 4 `cli_b_{digest,fingerprint,prove,
  snapshot}_differential.rs` files had already lost their refresh fns in an
  earlier task; only stale doc-comment mentions of `AL_SEM_DIR`/`bun run`
  remained and are swept out here. `cli_a_stats_differential.rs`'s
  `refresh()` — which regenerates purely from the ENGINE's own output, never
  `AL_SEM_DIR` or `bun` — was the model for the new regen paths (see
  Changed, above) and is untouched. `grep -rn "AL_SEM_DIR" tests/ src/` is
  now empty repo-wide, and the only surviving `bun run` mention anywhere is a
  pre-existing retired-note comment carrying no runnable instruction.
- **`KNOWN_DIVERGENCES.json` allowlist-tolerance machinery (Task 3.4, al-sem
  parity retirement arc).** The repo-root `KNOWN_DIVERGENCES.json` allowlist
  file is deleted, and the `AllowEntry` struct + `load_allowlist()` loader +
  the two-part gate (fail on any undocumented divergence, fail on any unused
  allowlist entry) are removed from all 13 differential harnesses
  (`differential.rs` — 6 gate sites; `r4_differential.rs`;
  `r2_5a_differential.rs`; `r2_5b_{cg,cov,eg,rt}_differential.rs`;
  `r3a{1,2,2_trace,3,4,5}_differential.rs`). Each gate now asserts directly
  that the computed divergence set is empty — since the allowlist was already
  `[]` (no divergence tolerated today), this is behavior-preserving, just
  stricter code: a future divergence now fails immediately instead of passing
  through a tolerance layer that had become vestigial. `r3a4_differential.rs`
  and `r3a5_differential.rs` also drop their now-vacuous "allowlist must be
  empty" exit-gate assertions (the real byte-match comparison against the
  golden is unchanged). Every harness's actual comparison — structural diff,
  byte-match, forbidden-field scans, anti-degenerate/coverage matrices, oracle
  cross-checks — is untouched. `cargo test --release --workspace` stays fully
  green (159 test-result blocks, 0 failed); CDO's
  `--program-call-graph-stats` SHA-256 is unchanged
  (`67910e992777b6bdef07b3b0046d1077c96cc03f581743d6404ee93d49913f4f`).
- **`analyze --dump-model`.** A hidden flag that only ever rejected itself
  with a CONFIG_ERROR pointing at "the TS CLI" — a tool that no longer
  exists. Removed outright; an invocation now gets clap's own
  unknown-argument rejection instead of the bespoke stub.

### Fixed
- **CDO re-measure + harness rebaseline for the H-1/H-2/H-3 dependency-ingest
  trio (Tier-1 remediation Task T1.2).** `aldump --program-call-graph-stats`
  on the frozen CDO workspace: `primaryScoped` (workspace-only edges) is
  BYTE-IDENTICAL before/after all three fixes (`total` 18108, `unknown` 0,
  `realUnknownRate` 0.0 — the north-star zero holds exactly). The only
  `wholeProgram`-scope delta is `total`/`honestEmpty` 43408 → 43369 (-39),
  attributed ENTIRELY to H-2 by incremental per-commit measurement (H-1 alone:
  byte-identical; H-1+H-2: exactly this delta; H-1+H-2+H-3: no further
  change). Root cause: CDO's workspace root has its OWN `.alpackages`, and
  its ANCESTOR folder (`find_all_alpackages_folders` scans both by design)
  has a SECOND `.alpackages` caching the SAME 12 real dependency apps — 10
  byte-identical duplicates plus 2 genuinely stale extra copies (`Continia
  Document Output` 28.0.0.227530, `Continia Connector App` 28.0.0.225760),
  confirmed real-world instances of exactly the scenario H-2 fixes (pinned in
  a new CDO-gated `cdo_dedup_names_the_real_dropped_duplicates` test).
  Removing the 12 duplicate AppUnits eliminated 39 duplicate call-site
  obligations that were being double-counted in the `wholeProgram` (not
  `primaryScoped`) `honestEmpty` bucket — H-1 (event-subscriber orphan fix)
  and H-3 (NUL-tolerant parse) are CONFIRMED CDO-DORMANT: `abiIntegrity`
  (`abiMapped`/`abiRoutesTotal`/`abiUnmapped`) and every other histogram
  field are byte-identical throughout — no NUL-padded or genuinely-corrupt
  dependency file exists in CDO's real `.alpackages`, and no `local`/
  `internal` publisher in CDO's real deps gained a NEW subscriber binding
  (dormancy here is the documented "fine, honest answer," not a fix that
  didn't work). `genuine_wrong` stayed 0 throughout every measurement.
  One pre-existing test asserted the OLD buggy H-1 behavior as correct:
  `program_resolve_harness.rs`'s `ws_protected_abi_internal_and_local_still_absent`
  expected an `internal`/`local` ABI member to resolve `Unknown(MemberNotFound)`
  (dropped-at-ingestion, indistinguishable from a name that never existed);
  renamed to `..._still_declines_but_with_precise_reason` and rebaselined to
  the correct, more precise `Unknown(InternalNotVisible)`/`Unknown(LocalNotVisible)`
  — a genuine improvement (the member now demonstrably exists and is
  access-declined, not silently absent), not a regression.
- **NUL-padded `SymbolReference.json` silently ingested as an empty ABI, and
  every read/parse failure had zero production reads anywhere (H-3, Tier-1
  remediation Task T1.2).** Some `.app` emitters pad `SymbolReference.json`
  with trailing NUL bytes after the real JSON content. The legacy
  `app_package::parse_symbols` parser (a `serde_json::StreamDeserializer`
  reading only the first JSON value) already tolerated this; the newer engine
  path (`engine::deps::symbol_reference::parse_symbol_reference`) used a
  STRICT `serde_json::from_str`, which fails on the trailing NUL padding
  (not JSON whitespace) even though the leading content is perfectly
  well-formed — a genuinely correct dependency silently ingested with EMPTY
  objects/tables. Worse, `SymbolReferenceAbi.error` (the field meant to
  surface exactly this) had ZERO production reads anywhere in the codebase,
  and a SEPARATE I/O-level failure path (`AbiCache::get_or_load`'s
  `unwrap_or_else(|_| SymbolReferenceAbi::default())`) silently swallowed
  zip-open/file-read failures the same way. Fixed with ONE shared tolerant
  parser (`app_package::parse_first_json_value`, extracted from the legacy
  path's existing technique) now used by BOTH `parse_symbols` and
  `parse_symbol_reference` — `resolve::abi_check`'s integrity harness
  re-parses through the same fixed function, so it was never
  "synchronized-blind" as a separate concern. `AbiCache::get_or_load`'s I/O
  swallow now routes through the SAME `error` field a JSON parse failure
  uses. `abi_ingest::ingest_abi` now returns a named `AbiIngestResult`
  (objects + routines + an optional `error`) instead of a bare tuple, and
  `build_program_graph` collects every non-empty one into a new
  `ProgramGraph.abi_ingest_errors: Vec<AbiIngestError>` (per-app, additive) —
  a broken dependency is now observable instead of silently indistinguishable
  from a genuinely-empty one.
- **Duplicate dependency `.app` versions silently picked the stale one, or
  poisoned an entire app's ABI (H-2, Tier-1 remediation Task T1.2).**
  `dependencies::load_all_apps` scans EVERY `.alpackages` folder reachable by
  walking up the directory tree (own + every ancestor, e.g. for a monorepo's
  shared package cache) and parsed every `.app` found with no GUID-level
  dedup at all; the only dedup was by exact canonical PATH. Two consequences:
  (1) `program::build::build_program_graph`'s dependency-closure GUID match
  (`by_guid.find(...)`) bound to whichever version happened to sort first in
  `load_all_apps`'s final ordering — sorted by raw STRING comparison on the
  version field (the "purely a stable tiebreak" doc comment was false: sorting
  ascending and taking the first match always favors the LOWEST version, e.g.
  a stale ancestor-folder copy silently won over the correct local one); (2)
  the SAME version physically present twice (a file copied into two scanned
  folders) ingested TWICE, producing IDENTICAL `RoutineNodeId`s for every one
  of that app's ABI routines — `dedup_routines_preserving_genuine_overloads`
  then marked EVERY survivor `abi_overload_collapsed`, declining the entire
  app's routines for chain-typing and plain dispatch alike. Fixed with a new
  `dependencies::dedup_by_guid_keep_highest_version`, wired into
  `load_all_apps` itself (protecting every caller uniformly): collapses every
  non-empty-GUID group down to its highest-version survivor (via the existing
  numeric `compare_versions`, never raw-string comparison), naming every
  dropped file in a new `DroppedDuplicateDependency` diagnostic;
  GUID-less entries (malformed/legacy manifests) are never merged against
  each other (fail-closed — no real identity to prove they're the same app).
  `load_all_apps`'s own final sort is also fixed to compare versions
  numerically (`parse_version`) instead of lexicographically, making its
  determinism-tiebreak comment honest again now that at most one entry per
  GUID survives to reach it. `SnapshotBuilder::build` gains a
  `build_with_diagnostics` sibling that surfaces the dropped-duplicate list to
  callers that want it (`build` itself is an unchanged thin wrapper).
  `indexer.rs`'s legacy `index_dependencies` path now logs every drop instead
  of silently absorbing the signature change. `program::build::
  build_program_graph`'s GUID-based dependency-closure matcher is documented
  (not yet behavior-changed) as still not consulting a specific edge's
  MinVersion once dedup leaves at most one candidate per GUID — a narrower,
  separate soundness question left open deliberately rather than risk
  trading a real match for a stricter decline on unconfirmed real-world data.
- **Dependency ABI ingestion silently dropped every `local`/`internal` routine,
  orphaning event-subscriber wiring and making the InternalsVisibleTo friend
  map inert for SymbolOnly deps (H-1, Tier-1 remediation Task T1.2).**
  `abi_ingest::ingest_abi` unconditionally skipped any ABI routine with
  `IsLocal`/`IsInternal` set. AL's `local` on an event PUBLISHER restricts
  RAISING, not SUBSCRIBING — modern BaseApp integration events are `local
  procedure` + `[IntegrationEvent]` (a real dependency probe found 13,581 such
  publisher attributes in BaseApp's `SymbolReference.json`, every one
  discarded); with the publisher routine never becoming a graph node,
  `resolve::index::ResolveIndex::build`'s event-index loop hit `0 => continue`
  with **no record at all** — the subscription (the charter's
  data-is-control-flow wiring) simply vanished. Separately, dropping
  `is_internal` routines meant `ProgramGraph.friends` (wired from a dep's own
  `<InternalsVisibleTo>`) was permanently inert for SymbolOnly deps — there
  was never an `Access::Internal` node to check friendship against. Fixed by
  ingesting ALL routines carrying an access field and mapping `IsLocal`/
  `IsInternal`/`IsProtected` to their matching `Access` variant (`Local`/
  `Internal`/`Protected`, `Public` otherwise) instead of dropping — the
  resolver's EXISTING visibility model (`resolver::object_access_visible_from`
  / `internal_visible_across`) now enforces call-time visibility, so ingestion
  no longer makes that decision by silent deletion; a publisher-kind routine
  stays subscription-eligible regardless of its own `Access` (subscribing is
  not calling — `ResolveIndex`'s candidate filter was already access-blind, so
  no change was needed there once the node exists). `resolve::abi_check::
  RawAbiIndex::build` (the independent re-derivation the integrity harness
  checks `ingest_abi`'s output against) carried the identical stale skip and
  is fixed in lockstep — left alone it would have turned every newly-ingested
  local/internal routine into a false `abi_unmapped` integrity failure. A
  0-candidate subscription (`ResolveIndex`'s event-index loop, the `0` arm of
  the candidate-count dispatch) now records an additive `OrphanSub` diagnostic
  (`ResolveIndex::orphaned_subscriptions()`) instead of a bare `continue`, so
  a genuinely-absent publisher is observable rather than silently invisible.
- **Platform singletons (`Session`, `NavApp`, `CurrPage`, `this`, …) shadowed a
  declared variable of the identical name instead of the other way around
  (Task T1.5, deep-review remediation plan, H-4).**
  `infer_receiver_type`'s singleton match (`receiver.rs`) ran as Step 1,
  BEFORE Step 2's declared-variable/param/global lookup — a `var Session:
  Codeunit "Telemetry Wrapper"` was silently discarded in favor of the
  platform `Session` singleton (a false `builtin` edge; the real
  `Session.LogMessage(...)`-shaped call actually meant `"Telemetry
  Wrapper".LogMessage`, dropped from the graph entirely), and `Session:
  Record Session` (the real virtual table 2000000009) produced a false
  `Unknown`. The L3 sibling engine (`engine/l3/receiver_type.rs:283-318`)
  already got this right — variables-first, with a comment explaining
  exactly this hazard — so the fresh resolver had regressed relative to its
  own sibling. Compiler-probed (`al.exe` 18.0.37.11445 against real
  Microsoft System/Application/Base Application/System Application/Business
  Foundation 28.0.46665.48632 packages): none of the twelve platform
  singleton names are AL reserved words — all compile with zero diagnostics
  as a declared variable name, and a declaration always shadows the
  singleton at that call site (control-verified with `error AL0132: '...'
  does not contain a definition for '...'` on the un-shadowed singleton). A
  same-named implicit-Rec TABLE FIELD wins too, which is why the fix's new
  Step 3c sits after Step 3a's bare-field lookup, not merely after Step 2.
  `this` draws a soft `AL0848: 'this' is a keyword from version '14.0'`
  compiler warning but still compiles and still shadows — so it moved to
  Step 3c along with its twelve sibling keyword strings, with no early-checked exceptions
  left at all (the task brief's tentative `currpage`/`currreport`/`this`
  exception list was falsified by direct compiler probe and corrected,
  matching the L3 sibling's ordering exactly). New TDD fixtures
  (`t1_5_*` in `receiver.rs`) cover: a declared Codeunit-typed `Session` var
  resolving to the declared type; a declared `Record Session` var; the
  undeclared-singleton regression guard; a second singleton name (`NavApp`)
  for generality; the field-vs-singleton precedence case; and the `this`
  shadow case. `cargo test` full suite green (1347+ lib tests, all
  integration suites); CDO impact EXPECTED dormant (real-`unknown` rate
  0.0000% unchanged) per the brief's prediction — the lane's own CDO gate
  never completed; the merge-time combined re-measure is the binding
  confirmation.
- **`Page.RunModal(Page::"X")` / `Report.RunModal(...)` and declared
  Page/Report-typed variables' `.RunModal()` now resolve as real entry-trigger
  `Run` edges into the target's `OnOpenPage`/`OnPreReport`, instead of an
  ordinary `PageInstance::runmodal`/`ReportInstance::runmodal` Catalog builtin
  route (Task T1.3, deep-review-remediation plan).** The whole callee subtree
  behind a `RunModal` call was previously invisible to the north-star metric
  (`builtin` is not counted as a hole). Two independent classifier gaps, both
  closed: (1) `extract::classify_call`'s `ObjectRun` check only recognized the
  keyword-receiver method `"run"`, never `"runmodal"` — now accepts
  `"runmodal"` for `Page`/`Report` keyword receivers (`Codeunit` excluded — it
  has no `RunModal` member); (2) `resolve_member_with_args`'s
  `ReceiverType::Object` arm only special-cased `Codeunit.Run` — now also
  dispatches a declared Page/Report-typed variable's `Run`/`RunModal` call to
  the target's entry trigger. Both populations now share one machinery
  (`resolver::dispatch_entry_trigger`, factored out of `resolve_object_run`'s
  tail) rather than duplicating the entry-trigger-lookup/collapse-marker-guard/
  Opaque-boundary logic a second time. A `RunModal` call whose target cannot
  be proven statically (a runtime variable) keeps its pre-existing honest
  `DynamicOpen`/`HonestDynamic` handling — mirrors `Codeunit.Run(SomeVar)`
  exactly, never `Unknown`. The T0.3 `builtin_dispatch_audit`'s CDO
  regression pin (`CDO_ENTRY_DISPATCH_FLAGGED_PIN`) dropped from 94 to 0 —
  every previously-flagged site now resolves the fix's Run edge. Two
  pre-existing unit tests that had encoded the bug as expected behavior
  (`resolve_member_page_runmodal_emits_catalog_route`,
  `resolve_member_page_declared_proc_shadows_catalog`) were corrected/
  repurposed; a new parity test
  (`resolve_member_page_declared_runmodal_proc_does_not_shadow_entry_trigger`)
  locks in that a declared same-named procedure never shadows the
  entry-trigger dispatch, mirroring the pre-existing `Codeunit.Run` precedent.
- **Arg-dispatch: text-inequality is not incompatibility proof, and text-
  equality-via-erasure is not identity proof either (T1.6, deep-review-
  remediation plan, H-5 — a fleet-confirmed, refuter-walked wrong pick).**
  `arg_dispatch.rs`'s `pick_candidate` compared a `var` parameter's
  compatibility against raw, whitespace-only-normalized type TEXT
  (`sig_fp::normalize_type_text`, which never touches a comma) while its
  by-value canonical identity (`base_keyword`) stripped everything from the
  first whitespace onward — silently erasing an entire generic-argument
  clause. Combined, `P(var d: Dictionary of [Integer, Text])` +
  `P(d: Dictionary of [Integer, Decimal])` called as `P(MyDict)` (`MyDict:
  Dictionary of [Integer,Text]`, spacing differing from the var param's own
  declaration text) made the correct var candidate get wrongly ELIMINATED on
  the spacing difference while the WRONG by-value Decimal candidate
  exact-matched on the erased identity — a confident wrong pick on compiling
  AL (the module's own doc calls this "the cardinal sin"). Fixed by giving
  `CanonicalArgType` a new `Generic { base, args }` variant: `Dictionary of
  [...]`/`List of [...]` type text is now recursively PARSED (base keyword +
  bracket-depth-aware comma-split argument list, each argument itself
  recursively canonicalized) into a structured identity, comparing equal iff
  the parsed shapes agree — whitespace/case inside the clause is normalized
  BY THE PARSE, never by string comparison, and a genuinely different
  instantiation (`[Integer,Text]` vs `[Integer,Decimal]`) is never erased
  into the same identity. A malformed clause (unterminated bracket, a bare
  `of` with no bracket, a trailing empty argument slot) is UNDECIDED
  (`None`) — blocks the pick (degrades to `AmbiguousOverload`), never
  eliminates and never proves. Applied identically to the ABI tier
  (`abi_param_canonical`, sharing the SAME parser) since SOURCE and ABI
  candidates mix within one `pick_candidate` call
  (`resolver::candidate_param_infos_either`) and must never disagree about a
  generic's identity. Separately: member-field arguments (`Foo(Rec.Field)`)
  hardcoded `var_passable: false`, a premise the plan-doc trail showed was
  REVERSED across two earlier rounds without ever running a compiler (one
  round: "record fields ARE generally var-passable"; the next, "correcting"
  it: "AL requires a VARIABLE for a `var` argument", shipped with an
  explicit "No investigation" note). Live-probed this task (`al.exe`
  v18.0.37.11445, CDO's real `.alpackages` cache, platform/application
  `28.0.0.0`): a plain table field (`Cust."No."`), a FlowField (`Cust.
  "Balance (LCY)"`), and a field of a `temporary` record (`TempCust."No."`)
  ALL bind a `var` parameter with ZERO diagnostics; a negative-control
  literal in the same position reports `AL0133` and an undefined field
  reports `AL0132`, confirming the harness genuinely type-checks.
  `var_passable` flipped `false` -> `true`, refuting the "correcting"
  round's premise directly. 12 new tests (`arg_dispatch.rs`), including the
  headline `pick_candidate_dictionary_generic_spacing_variance_resolves_
  var_overload_never_wrong_pick` (old-wrong-pick reproduced, then fixed) and
  2 rewritten member-field-arg tests; `cargo test`/clippy green throughout.
  CDO re-measure: SHA byte-identical to the frozen baseline
  (`67910e992777b6bdef07b3b0046d1077c96cc03f581743d6404ee93d49913f4f`,
  master `5046266`) — `ambiguousResolved`/`unknown` both stayed `0` in
  `primaryScoped` and `wholeProgram`, `genuine_wrong` held throughout; arg
  dispatch fires only on same-object overload sets surviving earlier gates,
  and CDO carries no Dictionary/List-generic-typed overload set for this
  fix to move — dormant on CDO, fixture-proven.
- **`REGEN_TEMP_GOLDENS=0` silently rewrote every golden while reporting green
  (Task T0.6, Tier-0 remediation arc).** Every regen gate checked env-var
  PRESENCE (`std::env::var("REGEN_TEMP_GOLDENS").is_ok()`, or `.is_err()` as
  the negated guard) rather than its VALUE — `is_ok()` is `true` for ANY set
  value, including `"0"`. Fixed by routing every gate through the new
  value-gated `regen_mode()` helper (see Added, above); `REGEN_TEMP_GOLDENS=0
  cargo test` now correctly takes the normal assert path and leaves the
  working tree clean (verified: full-suite `cargo test` and `REGEN_TEMP_
  GOLDENS=0 cargo test` both green with zero golden diffs; `REGEN_TEMP_
  GOLDENS=1 cargo test` green with a no-op diff on every family except one
  pre-existing, unrelated finding, left unresolved and undisturbed: `ws-
  interface-dispatch`'s R0 identity output turned out to be non-deterministic
  depending on unrelated prior test execution in the same process — NOT a
  simple stale golden. Its two `Interface` objects (`IEmpty`, `IProcessor`)
  collide on `stableObjectId` (AL interfaces carry no object number, so
  `engine::snapshot` assigns every interface `0`); the committed golden's
  `IEmpty` entry carries a `signatureFingerprint` that duplicates
  `IProcessor`'s. Running `differential_identity_subset_matches_goldens` in
  isolation reproducibly regenerates the mathematically correct, distinct
  fingerprint for `IEmpty` (`sha256("Interface|0|IEmpty")`); running it as
  part of the full `differential.rs` binary (the normal `cargo test` path)
  reproducibly regenerates `IProcessor`'s hash instead, matching what's
  committed. `object_signature_fingerprint` is a pure function and file
  processing is strictly sequential over an already-sorted list, so the cause
  is upstream — `al_syntax::parse`/`extract_from_ir`'s `Interface` object
  extraction — and unidentified; needs its own dedicated investigation. Full
  reproduction steps in `tests/r0-goldens/README.md`).
- **`tests/common/regen.rs` env-mutating unit test could race a real golden
  gate into silently rewriting a golden (whole-branch review finding on Task
  T0.6, Tier-0 remediation arc).** Because `regen.rs` is `#[path]`-included
  into ~40 golden-asserting test binaries, its own `#[cfg(test)]` integration
  test — `regen_mode_reads_real_env_var_by_value`, which `unsafe { set_var
  ("REGEN_TEMP_GOLDENS", ...) }`'d the process env to exercise all three
  value states, serialized only by a private `ENV_LOCK` that no other test in
  the binary honored — ran concurrently, under `cargo test`'s parallel test
  threads, with every other test in the same binary reading `regen_mode()`
  unlocked. During the window where the var was set to `"1"`, a racing golden
  gate could enter its regen branch and rewrite its committed golden without
  asserting: the exact silent-rewrite hazard T0.6 exists to eliminate,
  reintroduced by the test meant to guard it. Also plain UB (concurrent
  `setenv`/`getenv` is why `set_var` is `unsafe` since edition 2024). Fixed by
  deleting the test outright rather than adding save/restore locking (the
  race window would remain); the five pure `resolve_regen_mode_*` tests fully
  cover `regen_mode()`'s value semantics (a trivial composition over
  `resolve_regen_mode`) without touching the environment, so no coverage was
  lost.
- **Regen-write trailing-newline bug (`program_resolve_harness.rs`
  `fixture_semantic_golden_matches_l3`, Task T0.6).** Its pre-existing regen
  path omitted the trailing newline every other regen path in the repo
  writes, making a byte-identical regen impossible even though the
  assert-mode comparison (structural JSON diff, not byte-compare) never
  surfaced it. One-line fix; the committed golden's own bytes are unchanged.
- **Every CDO-gated test could silently skip forever — including the north-star
  ratchet itself (Task T0.2, Tier-0 remediation arc).** `CDO_WS`-gated tests used
  the bare `let Some(ws) = std::env::var_os("CDO_WS")...else { return; }` idiom,
  which no-ops with zero signal when `CDO_WS` is unset or points at a moved
  tree — including `program_resolve_harness.rs`'s Test 13
  (`cdo_full_program_coverage_and_self_reported_metric`, the coverage +
  `real_unknown_rate` ceiling ratchet). A loud-fail helper
  (`cdo_ws_or_enforce()`) already existed but was wired into only 4 of the
  suite's CDO-gated tests. Routed EVERY bare gate through it: in
  `program_resolve_harness.rs`, lines 866
  (`abi_ingestion_integrity_cdo_gate`), 1321 (Test 13, the ratchet), 3089
  (`route_applicability_zero_violations`), 3150/3236/3290 (the 3
  `#[ignore]`d diagnostic-dump tests), 3393
  (`cdo_unknown_include_sender_plus1_subscribers_preflight_is_zero`), and
  5243 (`fan_out_applicability_zero_violations`); plus the whole-body gates
  in `program_graph.rs:5`
  (`cdo_program_graph_is_app_qualified_and_panic_free`) and
  `snapshot_robustness.rs:5` (`cdo_snapshot_deep_parse_is_panic_free`). Since
  `cargo test` compiles each `tests/*.rs` file as a separate binary/crate, a
  private fn in one can't be `use`d from another — moved the single
  implementation to new `tests/common/cdo.rs` (cargo doesn't treat files
  under a `tests/` subdirectory as their own test targets, only top-level
  `tests/*.rs`), included via `#[path = "common/cdo.rs"] mod cdo;` in all
  three binaries, so the whole suite now shares exactly one implementation
  (the panic message also now names the failing test via
  `std::thread::current().name()`, which libtest sets to the test's path).
  Gate 1 (`CDO_WS` unset): all three binaries green, 179 passed / 0 failed /
  3 ignored + 1 + 1, confirming silent-skip behavior is unchanged. Gate 2
  (`ENFORCE_CDO_WS=1`, `CDO_WS` unset): all 9 non-ignored rewired tests (5
  newly wired + 4 pre-existing) panic loudly naming themselves; the 3
  `#[ignore]`d dumps panic the same way when run with `--ignored`. Gate 3
  (real `CDO_WS`, `ENFORCE_CDO_WS=1`, via the new `scripts/cdo-gate`): PASS,
  and CDO's `--program-call-graph-stats` SHA-256 is unchanged
  (`67910e992777b6bdef07b3b0046d1077c96cc03f581743d6404ee93d49913f4f`),
  confirming R5 — behavior with a valid `CDO_WS` is byte-identical. A broader
  `rg -n 'var_os("CDO_WS")' tests/ src/` sweep also found 6 more bare gates
  embedded as `#[cfg(test)]` unit tests inside `src/{snapshot/snapshot.rs,
  snapshot/parse.rs, program/l3_mint.rs, program/build.rs}` — left unrewired
  as out of scope for this task (the roadmap's T0.2 site enumeration and the
  `scripts/cdo-gate` invocations cover only the `tests/*.rs` ratchet suite;
  these are a different, lower-tier population, not part of the north-star
  ratchet, and would need a second `#[cfg(test)]`-scoped helper since a lib
  unit test can't reach `tests/common/cdo.rs` either) — flagged as a
  candidate follow-up, not fixed here.
- **`aldump`'s stats/projection modes could not fail — a broken/unusable
  workspace silently reported a PERFECT north-star score (Task T0.1, Tier-0
  remediation arc).** `aldump --l3-call-graph-stats <workspace>` is the
  north-star measurement command (the real-`unknown` edge rate); on an
  unusable/fail-closed layout it printed a stderr warning but then emitted
  `Histogram::default()` (`realUnknownRate: 0.0`) on stdout and exited
  `ExitCode::SUCCESS` — any CI/jq ratchet built on it would pass forever
  regardless of whether the tool actually ran. Two other modes
  (`--l3-call-graph-stats-cross-app`, `--l3-unknown-breakdown-cross-app`) had
  the same defect in a different guise: they emitted a JSON body containing
  `"error": "..."` on stdout and STILL exited SUCCESS. Audited all 29
  `aldump` dispatch branches (28 flag-gated modes + the no-flag default) for
  the shape; 23 had it (every mode gated on
  `assemble_and_resolve_workspace_default`/`build_cross_app_l3_from_workspace`
  returning `None`, plus `--r3a4-dep-hooks`/`--r3a5-cross-app-summary`, whose
  underlying library functions are intentionally "engine-never-throws" for
  their differential/oracle test callers and so needed a CLI-boundary
  pre-check instead of a signature change; `--r2.5a-merged-index`, gated on
  path existence since it legitimately accepts a dep-less `.app`/dir). Fixed
  by converting every `None`/`"error"`-body case to `eprintln!` + no stdout +
  `ExitCode::FAILURE` — never a silent default-shaped success. 6 modes were
  already correct (`--l2`, `--program-call-graph-stats`, `--graphify-export{,
  -fragments}`, `--integration-points`, the no-flag default) and were left
  untouched; they follow the same `let Some(x) = ... else { eprintln!(...);
  return ExitCode::FAILURE; }` idiom the fix now applies everywhere else. The
  success path is byte-unchanged (verified via the r2.5a/r3a4/r3a5
  differential/oracle suites + the CDO gate). New `tests/aldump_smoke.rs` CLI
  cases lock a nonexistent-path failure (both source-only and cross-app) and
  a good-path success.
- **`cli_a_{html,json,terminal}_differential.rs` were silently reporting `ok`
  while running ZERO real assertions, ever since al-sem left disk (Task 3.6,
  al-sem parity retirement arc capstone).** Each file's main byte-match test
  and several "anti-degenerate oracle" tests gated on `al_sem_{html,json,
  terminal}_dir().is_dir()` — a hardcoded sibling-checkout path
  (`<repo>/../al-sem/scripts/cli-a-goldens/...`) — and silently `return`ed
  ("SKIPPING") when that path was absent, which it always has been since
  al-sem was archived: `cargo test` showed these as passing while they
  executed no comparison at all. `cli_a_stats_differential.rs` was NOT
  affected (its skip-check already also tested the in-repo vendored dir).
  Worse, once the bogus gate was removed, the corpus itself proved
  incomplete: the vendored `tests/cli-a-goldens/{html,json,terminal}/`
  directories held only the fixtures a prior task had explicitly
  rebaselined (10/22, 17/40, 10/27 respectively) — the rest had always been
  served by the (now-gone) al-sem fallback and were never vendored. Fixed by
  (1) deleting the dead `al_sem_*_dir()` fallback functions and simplifying
  `resolve_golden()` to read the in-repo vendored dir as the sole source
  (mirroring the `cli_c_cache_differential.rs` precedent from Task 3.3); (2)
  removing every skip-gate — a missing local golden is now a hard test
  failure via the existing "golden file missing" divergence path, and the
  three oracle tests that didn't even read a golden lost their spurious
  guard entirely; (3) regenerating the 49 missing goldens (12 html, 20 json,
  17 terminal) via `REGEN_TEMP_GOLDENS=1` — all NEW files, no existing
  golden was overwritten — and spot-checking several for structural sanity
  (e.g. `ws-txn-d46-neg` renders "No findings.", `ws-d34` renders its
  `d34-commit-in-loop` finding). `cli_a_stats_differential.rs`'s `refresh()`
  utility, which had been writing into the (nonexistent) al-sem sibling path
  instead of the in-repo vendored dir, is corrected to target
  `local_stats_dir()`. All 4 `cli_a_*` suites now run 16 real tests (0
  skipped) and pass against a fully self-contained, hard-required corpus.
  (A same-task review fix caught 2 more instances of the identical
  pattern that survived the first pass: `cli_a_html_differential.rs`'s
  `event_graph_fixture_renders_svg` and `cli_a_json_differential.rs`'s
  `envelope_fields_are_correct` each still gated on
  `if !fixture_dir.is_dir() { eprintln!(...SKIPPING...); return; }`
  against a git-tracked `tests/r0-corpus` fixture that always exists —
  converted both to `assert!(fixture_dir.is_dir(), ...)` so a genuinely
  missing fixture fails loudly; confirmed with `--nocapture` that both
  oracles now execute their real body. All 4 `cli_a_*.rs` files are
  re-grepped for any other `is_dir()`-gated skip-return: none remain.)
- **`alsem policy check`'s `policySource` no longer embeds an absolute machine
  path — it is now workspace-relative (Task 3.1, al-sem parity retirement
  arc).** `resolve_policy_check` (`--policy`/auto-detect) and
  `run_policy_explain` (same two branches, an identical bug the task brief's
  scouting missed but which shared the same root cause) each built
  `policySource` as `format!("explicit:{}", abs.display())` /
  `format!("auto:{}", abs.display())` — an absolute, machine- and
  checkout-specific path leaking into committed goldens and any consumer of
  `alsem policy check --format json/human` output, a reproducibility defect.
  New pure helper `pipeline::workspace_relative(workspace, abs)`: the policy
  path becomes relative to the analyzed workspace root, forward slashes on
  every platform (component-wise reconstruction, not a naive
  backslash-replace), no drive letters, no `.`/`..` segments (a
  `normalize_lexical` pass collapses `CurDir`/`ParentDir` components first, so
  a workspace root passed as `.` still strips correctly); a policy file
  **outside** the workspace root falls back to its bare filename. `absolutize`
  broadened to `AsRef<Path>` so it composes with the new helper without an
  extra `&str` round-trip. SARIF (`format_policy_sarif`) does not surface
  `policySource` at all, so it needed no change. 6 new unit tests
  (`pipeline.rs`): inside-workspace, nested-subdir, POSIX- and Windows-style
  outside-workspace fallback, backslash-input normalization, and a `.`-laden
  workspace root. Rebaselined the two affected goldens
  (`tests/cli-c-policy-goldens/ws-policy-custom.custom.{human.txt,json}`):
  `policySource` changes from `auto:U:\Git\al-sem\test\fixtures\
  ws-policy-custom\al-sem.policy.yaml` to `auto:al-sem.policy.yaml` (the
  auto-detect candidate is always `workspace.join("al-sem.policy.yaml")`, so
  its relative form is exactly the bare filename); every other byte in both
  goldens is unchanged. Full CDO harness and the aldump `--program-call-graph-
  stats` JSON SHA-256 stay byte-identical to `.superpowers/sdd/cdo-baseline-
  plan13.md` (`67910e992777b6bdef07b3b0046d1077c96cc03f581743d6404ee93d49913f4f`)
  — the policy gate is disjoint from the call-graph harness.
- **`abi_param_canonical` falls back to `type_text` identity — the
  `already_quoted` shape now participates in a pick (Task 2 review fix,
  roadmap-closure plan).** Task 2's ABI-aware canonicalization reached PAST
  `classify_type_text(&p.type_text)`'s own extracted `table_ref`/`object_ref`
  and required the raw `subtype_raw_name`/`subtype_id` tuple instead — but
  the `already_quoted` reconstruction shape (`type_text = 'Record "Normal
  Table"'`, no `Subtype` at all — per `reconstruct_param_field_type`'s own
  doc "the more common real shape") therefore ALWAYS degraded, defeating the
  feature for the common Record-typed ABI param (fail-safe, but a
  completeness gap). Fix: when the raw tuple is absent, fall back to the
  identity `classify_type_text` already extracted from `type_text` — the
  SAME semantic-identity route `dispatch_canonical_type_text` uses for a
  SOURCE parameter; when BOTH sources are present, cross-validate (resolve
  each independently and require the SAME object) rather than silently
  preferring one — any disagreement degrades the whole param. 3 new unit
  fixtures (`arg_dispatch.rs`): the already_quoted-no-Subtype shape
  canonicalizes and participates in a real 2-overload pick; a
  tuple-vs-text-disagreement fixture degrades; the existing 13 Task 2
  fixtures stay green. Plus a new whole-graph invariant test (`build.rs`):
  no routine surviving `dedup_routines_preserving_genuine_overloads` has
  `abi_overload_collapsed` and `abi_params != CollapsedUntrusted` out of
  lockstep, across a mix of collapsed/non-collapsed ABI and SOURCE routines
  on multiple objects — and a new structural `ApplicabilityReport` counter
  (`abi_overload_collapsed_lockstep_violations`, folded into `is_clean()`)
  wiring the SAME invariant into the CDO-gated `route_applicability_zero_
  violations` harness test over the real graph. Full CDO harness
  BYTE-IDENTICAL to `.superpowers/sdd/cdo-baseline-plan13.md` — CDO carries
  zero `abi_overload_collapsed` routines, so this fix is fixture-proven but
  CDO-dormant, same as Task 2 itself.
- **Step 4b (bare enum-type-name receiver) with-scope symmetry (Task 3,
  roadmap-closure plan).** `infer_receiver_type`'s Step 4b
  (`receiver.rs`, `"CDO Send on Posting".FromInteger(...)`-shaped) had no
  `WithState` gate at all, unlike Step 3a's bare implicit-Rec field arm right
  above it — inside an un-modeled `with` block, a bare enum-type name is
  exactly as syntactically ambiguous as a bare field (the with-target
  record could declare a field of the identical name), so Step 4b could
  silently prefer the enum-static reading over an unproven field reading —
  the same false-`Source`-edge risk class Step 3a's guard exists to close.
  Fix: Step 4b now requires `bare_ctx` present AND
  `WithState::NoWithProven`, the IDENTICAL gate shape Step 3a already uses
  (unconditional rather than object-kind-scoped, since a `with` block can
  wrap a record-typed receiver in any object kind, not only Table/Page). 2
  new fixtures (`step4b_declines_when_with_unproven` — loops
  `InsideWith`/`Unknown`, both decline; `step4b_resolves_when_no_with_proven`
  — `NoWithProven` preserves the existing resolution). The 2 pre-existing
  positive Step-4b fixtures (`bare_quoted_enum_type_name_resolves_...`/
  `bare_unquoted_enum_type_name_resolves_...`) now supply a realistic
  `Some((&body_map, NoWithProven))` `bare_ctx` instead of `None`, matching
  what a real `resolve_full_program` caller actually threads through — the 3
  negative fixtures (collision/routine-shadow/local-var-shadow) are
  unaffected (they either resolve at Step 2, before Step 4b ever runs, or
  already expected `Unknown`, which the new no-`bare_ctx`-is-no-op fallback
  still produces). Full CDO harness BYTE-IDENTICAL to
  `.superpowers/sdd/cdo-baseline-plan13.md` — CDO's Step-4b population sits
  entirely outside any `with` block, so this fix is fixture-proven but
  CDO-dormant.
- **Duplicate-signature rationale comments corrected to the real diagnostic,
  AL0440 (Task 3, roadmap-closure plan; hygiene item flagged by Task 1's own
  review).** `resolver.rs`'s `resolve_in_extendable_scope` doc and 4 Page/
  Report test-fixture comments (the inherited Page base-vs-extension +
  cross-extension cases, and Task 1's new Report cross-extension case)
  cited AL0115 ("base/extension duplicate") and AL0226 ("cross-extension
  duplicate") for why an exact duplicate-signature ambiguity fixture is
  DEFENSIVE-ONLY (uncompilable in real AL) — those citations were never
  independently `al compile`-probed at the time and turned out to be wrong.
  Live-probed this task (`al.exe` v18.0.37.11445, same CDO `.alpackages`
  cache and methodology as Task 1's `al compile` probe): a base Page +
  PageExtension both declaring `SameProc()` reports
  `AL0440: The Page 'ProbePageA' already defines a method called 'SameProc'
  with the same parameter types`; two PageExtensions both declaring
  `DupProc()` reports the identical `AL0440` class against each other. BOTH
  shapes are AL0440, not two distinct codes — comments corrected to cite the
  single real diagnostic and this probe.
- **Roadmap dispositions, probe-grounded (Task 3, roadmap-closure plan).**
  - **QueryExtension: the round-2 addendum's "EXISTS as an AL object type"
    claim was itself FALSE — corrected back to RETIRED (nonexistent
    construct), now on direct compiler evidence rather than either LLM
    review's assertion.** Probed 3 code-bearing shapes (`al.exe`
    v18.0.37.11445, CDO's `.alpackages` cache, platform/application
    `28.0.0.0`): a bare `queryextension` object with only a data-shape
    addition (no code at all), one with an added `procedure`, and one with
    an added `trigger OnBeforeOpen()` — all 3 reject IDENTICALLY with
    `AL0198: Expected one of the application object keywords (table,
    tableextension, page, pageextension, pagecustomization, profile,
    profileextension, codeunit, report, reportextension, xmlport, query,
    controladdin, dotnet, enum, enumextension, interface, permissionset,
    permissionsetextension, entitlement)` — `queryextension` is absent from
    the compiler's own enumerated keyword list. A positive control (the same
    project with only the base `query` object, no extension file) compiles
    clean (exit 0), ruling out an environment/cache setup failure as the
    cause. This directly contradicts the plan's BINDING round-2 closer
    ("`queryextension` EXISTS as an AL object type... the prior 'nonexistent
    construct' wording was false") — that correction was itself ungrounded;
    the ORIGINAL wording was right. Wake condition: a future AL compiler
    version ever adding a `queryextension` object keyword.
  - **Sender param-TYPE mismatch: DEFERRED-WITH-WAKE, not retired.** A
    `Sender` parameter TYPE mismatch between an event's publisher and
    subscriber is impossible to construct in compile-valid AL under a
    CONSISTENT dependency closure (the compiler itself enforces the
    publisher/subscriber signature match at compile time) — but a
    version-drifted closure (a shipped `.app` compiled against an OLDER
    publisher signature, now paired with a newer/changed publisher at
    resolution time) can present a real mismatch this engine does not model.
    Wake condition: a real corpus with a stale/version-drifted symbol
    closure demanding drift analysis.
  - **Protected `Variables[]`: DEFERRED-WITH-DESIGN, not retired.** 3 real
    CDO declarations exist (dependency page/table variables with an explicit
    access modifier), zero consuming extension sites currently reference
    them — population-less on CDO today, so building the machinery now
    would be unverifiable. The 3-layer lift is documented for whenever a
    real consumer appears: (1) `VarDecl`'s access-modifier field (the
    grammar already parses it; `al_syntax::lower` currently drops it), (2)
    `ObjectNode`'s globals exposure (surfacing a protected-marked global
    distinctly from a public one), (3) the scope-merge analog (an extension
    routine's visibility check over a base object's protected globals,
    mirroring the existing protected-member-access rules for routines).
    Wake condition: an extension routine consuming a base object's protected
    global variable in any corpus.
  - **CHANGELOG errata: the stale "unquoted bare field receivers ...
    deliberately deferred" note.** The applicability-param-subtype-recfield
    plan's Task 4 entry (below) listed "unquoted bare field receivers
    (`MyBlob.CreateInStream()`-shaped, deliberately deferred by both T3 and
    T4)" as an open item, and its own body text called the quoted-only scope
    "deliberate documented undercoverage — an unquoted bare field reference
    is deferred to a future task." **That deferral was resolved two arcs
    later:** the receiver-closure-and-arg-increments plan's Task 3 ("Named-
    return-value bindings + implicit-self table fields", below) widened the
    SAME Step 3a machinery (`ResolveIndex::field_in_table` +
    `table_scope_has_routine` + the `WithState::NoWithProven` gate,
    unchanged) to accept an unquoted bare identifier too — landing unquoted
    bare implicit-Rec field receivers for Table/TableExtension, later
    widened again to Page/PageExtension via `SourceTable` (Task 2 of the
    pageext-merge-and-final-residual plan). Not rewriting the original
    entries (append-only errata) — this is the dated correction.
  - **Audit-trail wording convention.** `.superpowers/sdd/` is
    git-ignored (see its own `.gitignore`) — a task report must never claim
    its content is "preserved in git history"; only files actually tracked
    by git qualify for that claim. Recorded here since an earlier report in
    this plan (Task 1) used that wording incorrectly.
- **Grammar repin: spaced `# if`/`# elif`/`# endif` now recognized
  (Task 4, roadmap-closure plan; `tree-sitter-al` v3.1.0 `307dc39` ->
  v3.2.0 `14bd55c`).** Closes the limitation v3.1.0 documented and
  reviewed-and-rejected: a single horizontal space between `#` and the
  directive keyword (`# if`, `# elif`, `# endif`) previously recovered to an
  honest `ERROR` (this engine's `ParseStatus::Recovered` diagnostic) rather
  than parsing. Fixed this time via **scanner-exclusive** ownership (the
  round-2 closer's binding design gate) rather than the reverted
  literal-variant approach that caused v3.1.0's GLR non-determinism: the
  external scanner's `PREPROC_OPEN`/`PREPROC_CLOSE` now consume `#`, optional
  horizontal whitespace, then the keyword, as ONE token — participating in
  the depth counter identically for spaced and unspaced forms — and the
  grammar's `preproc_if`/`preproc_endif` carry ONLY the scanner token (every
  grammar-literal fallback removed), so there is exactly one route to either
  token and nothing for GLR to fork on. `preproc_elif` (no scanner token, no
  depth interaction) separately gained spaced literal variants mirroring the
  pre-existing, safe `preproc_else` pattern. This also retires a latent bug
  the old `'# endif'` literal fallback carried since its introduction: it
  bypassed the scanner's depth counter entirely (zero corpus hits ever fired
  it, but the bug was real). Inert on CDO (zero spaced-preproc source; full
  CDO harness BYTE-IDENTICAL to `.superpowers/sdd/cdo-baseline-plan13.md` on
  every metric, including `aldump --program-call-graph-stats`'s JSON SHA-256).
  `gen-syntax` re-run: zero NAMED-kind vocabulary change (388 named kinds
  unchanged) — only the anonymous token set shifted (9 removed:
  `#if`/`#IF`/`#If`/`#endif`/`#ENDIF`/`#Endif`/`# endif`/`# ENDIF`/`# Endif`;
  3 added: `# elif`/`# ELIF`/`# Elif`) and the embedded
  `GRAMMAR_NODE_TYPES_HASH` anchor updated accordingly — both fully expected,
  neither a RawKind vocabulary move. See the grammar repo's `[3.2.0]`
  CHANGELOG entry and `.superpowers/sdd/task-4-report.md` for the full design
  note, stability-protocol results (5x clean-cache `tree-sitter test` runs +
  5x clean-cache re-parses of the historical GLR-non-determinism repro, all
  identical), and the BC.History (16,898 files) byte-identical manifest
  proof. Local commits only (grammar + this repin) — no push, no tag; rides
  the merge menu with Tasks 1-3.
- **Roadmap-closure arc complete — BUILT 4 / RETIRED 2 / DEFERRED-WITH-WAKE 6, the roadmap's FINAL
  honest state; every zero-metric held byte-identical through all 4 build tasks (Task 5, FINAL,
  roadmap-closure plan).** Full re-measure at HEAD `e994d7b` against the frozen
  `.superpowers/sdd/cdo-baseline-plan13.md` baseline (engine `858e663` / grammar `307dc39`):
  BYTE-IDENTICAL on every tracked row — CDO harness 179 passed / 0 failed / 3 ignored; primary
  `unknown` 0/18108 (0.0000%), whole `unknown` 0/43408; `ambiguousResolved` 0 both scopes;
  `unknownByReason` `{}` both scopes; `recoveredFiles` 0; `genuine_wrong` 0 (54/54 adjudicated); the
  L3 semantic-audit digest and `aldump --program-call-graph-stats` JSON SHA-256 both byte-for-byte
  identical to the baseline; `route_applicability`/`fan_out_applicability` both 0 violations
  (non-vacuous `routes_checked`); `cargo test --workspace` 159 blocks ok, 0 failures; clippy/fmt
  clean.
  - **BUILT (4):** the Report/ReportExtension routine merge via `resolve_in_extendable_scope`
    unification (Task 1); ABI param-type retention behind the structural `AbiParams` enum, lifting
    the SymbolOnly arg-type dispatch gate, plus the `already_quoted` canonicalization fallback
    (Task 2 + its review fix); the Step-4b `WithState` symmetry guard (Task 3); the spaced-`# if`/
    `# elif`/`# endif` scanner-EXCLUSIVE route, `tree-sitter-al` v3.1.0 -> v3.2.0 (Task 4), which
    also root-caused and fixed a real latent depth-counter bug in the old `'# endif'` literal
    fallback. All four are fixture-proven and CDO-population-less by grounding (Tasks 1-3 measure
    zero live CDO population; Task 4 is inert on CDO source) — exactly the outcome the plan
    predicted; "byte-identical CDO" was the acceptance bar throughout, never a metric mover.
  - **RETIRED (2):** QueryExtension — NOT an AL object keyword. This plan's own round-2 addendum
    asserted `queryextension` "EXISTS as an AL object type" and narrowed the retirement to "no
    callable routine members"; Task 3's mandatory `al compile` probe (`al.exe` v18.0.37.11445, 3/3
    code-bearing shapes: bare, `+procedure`, `+trigger`) found that claim FALSE — all 3 reject
    identically with `AL0198: Expected one of the application object keywords (...)`,
    `queryextension` absent from the compiler's own enumerated list, confirmed against a clean
    positive control. Disposition reverts to the ORIGINAL pre-round-2 wording: RETIRED (nonexistent
    construct). Wake: a future AL compiler version ever adding the keyword (re-probe before
    re-asserting either way — see the plan doc's dated correction, added this task). The
    `.dependencies`/CDO same-slug "double-include" framing — already retired 2026-07-05 per the
    permanent law (`.dependencies` folders are ordinary source, confirmed CLEAN by two EARLIER
    plans' Task-0 preflight audits — dataitem-depscope-reason-split and
    sigfp-and-ambiguous-reclassification — before this plan even started); recorded here for the
    capstone's completeness, not new work this arc.
  - **DEFERRED-WITH-WAKE (6), the roadmap's final state — every remaining call-graph item, each
    population-less, each with its own wake condition:** `ProvenAbsent` machinery (wake: a real
    proven-absence population on any corpus); implicit-conversion modeling (wake: a nonzero
    `ambiguousResolved` population); the full `ParseStatus` gate (wake: the first
    absence-claiming consumer); protected `Variables[]` (wake: an extension routine consuming a
    base protected var, in any corpus); preprocessor-symbol fidelity for embedded dependencies
    (wake: a real consumer); `Sender` parameter-TYPE drift analysis (wake: a corpus with a
    version-drifted symbol closure).
  - **Two stale roadmap claims confirmed DONE-VERIFIED, corrected append-only:** unquoted bare
    field receivers (Task 3's errata — landed in the tenth arc, receiver-closure-and-arg-increments
    plan, `e24ad4c`; a CHANGELOG note calling it "deliberately deferred" was stale). Dot-quoted
    field names (e.g. `"No."`) — corrected THIS task: `is_atomic_receiver_token` (`receiver.rs`)
    treats ANY well-formed quoted token as atomic regardless of an embedded dot (proven by
    `infer_receiver_type_step2b_dot_bearing_quoted_dataitem_name_resolves` + the
    `is_atomic_receiver_token_cases` "quoted, embedded dot" case) — the SAME shared primitive every
    quoted-receiver arm (Step 2b, Step 3a, quote-parity) is built on, so this was structural
    immunity all along, never a gap needing its own arm; the applicability-param-subtype-recfield
    plan's stale "dot-quoted field names... not yet covered by any quoted-field arm" claim (above,
    `### Fixed` "Bare implicit-Rec quoted-field receivers..." entry) is corrected here.
  - **Nits:** the stale "15,358" BC.History figure in `tests/program_resolve_harness.rs`'s
    `RESOLVED 2026-07-05` comment corrected to 16,898 (this checkout's actual corpus size, per Task
    4's own measurement); the plan doc
    (`docs/superpowers/plans/2026-07-05-roadmap-closure.md`) gets a dated, append-only correction
    note on its own round-2 QueryExtension addendum, pointing at the Task 3 probe finding above.
  - Product backlog (BC-Brain integration work) stays SEPARATE from this call-graph roadmap, per
    the plan's own binding rule — never folded into the doctrine-deferred list above.
  - Grammar + engine local-only state at close: `tree-sitter-al` v3.2.0 local commit `14bd55c`
    (submodule pin, not pushed); the engine's `feat/roadmap-closure` HEAD carries the submodule
    gitlink update. No push, no tag, no merge to `master` this task — per the plan's explicit
    no-push gate and this task's foreground-only, non-destructive scope.

### Added
- **Scope-resolver unification + the Report/ReportExtension routine merge
  (Task 1, roadmap-closure plan).** `resolve_in_table_scope` and
  `resolve_in_page_scope` (`resolver.rs`) were ~90%-identical hand-copies,
  diverging only in the zero-arity-match branch (Table diagnoses an
  access-exclusion reason; Page forwards to the first name-bearing object so
  `resolve_in_object`'s own `ArityMismatch`/`AccessFilteredOverload`
  diagnostic survives) — confirmed by a pre-refactor dimension-by-dimension
  behavioral inventory (candidate collection, extension filtering, closure
  anchor, access rules, cardinality, ambiguity ordering all identical) before
  any code moved. Unified into `resolve_in_extendable_scope` (the shared
  ~150-line engine) + a `ZeroMatchStrategy` enum (`AccessExcludedReason` |
  `PreserveArityMismatch`) + three ~25-line thin wrappers
  (`resolve_in_table_scope`/`resolve_in_page_scope`/the NEW
  `resolve_in_report_scope`). `Report`-typed receivers
  (`ReceiverType::Object{kind: Report, ..}`) now merge in every
  closure-visible `ReportExtension`'s routines, closing the gap the
  pageext-merge-and-final-residual plan deliberately deferred (that plan's
  Task 1 doc note is superseded by this entry) — a new `report_extensions`
  reverse index (`index.rs`, mirroring `table_extensions`/`page_extensions`;
  `extends_target` was already populated for `ReportExtension` identically
  to `PageExtension`) plus the `:2421` routing site now dispatching
  `Page`/`Report` both through their respective extendable-scope resolver.
  The `PreserveArityMismatch` policy for Report is grounded by an `al
  compile` probe (the tree-sitter-al grammar repo's minimal-probe
  methodology): a same-app `ReportExtension` procedure called through a
  base-Report-typed variable receiver compiles cleanly (the merge itself is
  real, compiler-verified AL semantics), and a wrong-arity call reports
  `AL0135` ("no argument given that corresponds to the required formal
  parameter") — a diagnostic class distinct from the genuine "member not
  found" `AL0132`, confirmed on the same fixture. Zero fixture edits to the
  existing Table/Page test suites (the behavior-preservation postcondition);
  8 new Report-shaped fixtures added (same-app internal resolves; different
  -app internal declines with `InternalNotVisible`; out-of-closure extension
  invisible; two visible extensions ambiguous; visible wrong-arity preserves
  `ArityMismatch`; invisible (out-of-closure) wrong-arity does not leak
  `ArityMismatch`; mixed base+extension wrong-arity is deterministic; base
  -only calls unchanged, including the `ReportInstance` catalog fallback) —
  3 of the 8 independently confirmed as genuine regression-catchers (fail
  against the pre-refactor bare-`resolve_in_object` routing), the remaining
  5 as completeness/non-regression controls mirroring the Page suite's own
  established pattern. Full CDO harness BYTE-IDENTICAL to the frozen
  `.superpowers/sdd/cdo-baseline-plan13.md` baseline (179 passed / 0 failed
  / 3 ignored; primary/whole `unknown`=0; `ambiguousResolved`=0;
  `unknownByReason`={} both scopes; `recoveredFiles`=0; `genuine_wrong`=0; L3
  semantic-audit digest and `aldump --program-call-graph-stats` JSON SHA-256
  both byte-for-byte identical) — Report cross-extension population is
  confirmed ZERO on CDO, so this machinery is fixture-proven but currently
  dormant there, exactly as the grounding predicted.
- **Call-result + boolean argument typing — `ambiguousResolved` 7→0, a FULL
  closure (Task 3, pageext-merge-and-final-residual plan).** Extends
  `arg_dispatch::type_one_arg` with three new arms so an argument that is
  itself a CALL or a comparison/logical expression can now be typed, feeding
  the UNCHANGED `pick_candidate` guard stack from Task 2:
  - **`ExprKind::Call` arm** (`type_call_result_arg_bare`/
    `type_call_result_arg_member`): (a) a bare-identifier call-result
    (`Foo(GetCount())`) mirrors Step 5's guards
    (`receiver::infer_call_result_receiver`) — the local/param/global SHADOW
    guard, then a SINGLE-route `resolve_bare` query (empty `args` — no
    recursion into `pick_candidate`); (b) a Member-function call-result
    (`Foo(X.Method())`) mirrors Step 6's cross-object-chain base typing — the
    base types via the SAME caller-scope-EXACT path the existing `Member`-
    field arm uses (`with`-scope-gated), then a SINGLE-route `resolve_member`
    query. Both read the resolved routine's return type via a new
    `call_result_arg_from_routine_node` — the SAME `abi_overload_collapsed` +
    `return_type_id` ABI structured-cross-validation guards
    `receiver::receiver_from_routine_node` applies to a call-result RECEIVER
    base, but WITHOUT that function's Primitive-decline (an argument WANTS
    exactly the scalar/primitive shapes a receiver dispatch base would
    reject). (c) `RouteTarget::Builtin` additionally consults a new passive,
    per-entry-cited builtin-return catalog (`strsubstno`/`format`/`copystr`/
    `lowercase`/`uppercase`→`text`, `round`→`decimal`, `strlen`→`integer`),
    gated on `resolve_bare` POSITIVELY reporting `Builtin` for the exact
    name — a source procedure shadowing one of these names resolves to
    `RouteTarget::Routine` at Step 1, long before the catalog is ever
    reachable, so a shadowed name is NEVER mistyped by the catalog (proven by
    two mandatory shadowed-name fixtures, `Format`/`CopyStr` declared as
    source procedures with a DIFFERENT return type).
  - **`ExprKind::Binary`/`Parenthesized` arms**: `Eq`/`Ne`/`Lt`/`Le`/`Gt`/`Ge`/
    `And`/`Or`/`Xor`/`In` type UNCONDITIONALLY as `Boolean` (no operand
    inspection — AL defines these operators as Boolean-yielding regardless of
    operand type); every arithmetic operator (`Add`/`Sub`/`Mul`/`Div`/
    `IntDiv`/`Mod`) and the catch-all `Other` decline — including a TEXT `+`
    concatenation (the SAME `Add` variant as numeric addition), proving the
    decline is operator-driven, never "looks numeric"-driven.
    `Parenthesized` unwraps recursively.
  - **A companion `al-syntax` lowerer fix** (`crates/al-syntax/src/lower/
    mod.rs`): `RawKind::InExpression` (`X in [..]`/`X in Y..Z` as an ORDINARY
    expression, not a case pattern) was NOT included in the four-`RawKind`
    union that lowers to `ExprKind::Binary` — it fell into the generic
    catch-all, becoming `ExprKind::Unknown` (a payload-less variant), which
    made any CALL nested inside it (e.g. `Session.CurrentClientType() in
    [ClientType::Web, ..]`) structurally UNREACHABLE to a tree walker
    descending from the statement root, even though the lowerer's own
    "for completeness" recursion had already registered the nested call in
    the arena. A genuine, pre-existing modeling gap (explicitly documented as
    such by the `in_expression_case_pattern_is_a_single_pattern` test, now
    updated) — root-caused and fixed by adding `RawKind::InExpression` to the
    same lowering arm as the other four comparison/logical kinds (identical
    `left`/`operator`/`right` field shape). Required for the `In` operator to
    ever reach the new Boolean-typing arm at all for a plain call argument.
  - **The remaining-ambiguous dump diagnostic**
    (`task3_dump_remaining_ambiguous_resolved_sites_on_cdo`, `#[ignore]`d,
    mirrors the `task2_dump_argtype_dispatch_flips_on_cdo`/
    `task3_dump_untracked_receiver_sites_on_cdo` precedent): dumps every
    `AmbiguousResolved` edge — site, every candidate's target identity, and
    the raw call-site source text — for future mechanical re-grounding.
  - New fixture banks (`tests/r0-corpus/`): `ws-overload-membercall-
    discriminator` (the PrintPDFFile Member-call-result shape, POSITIVE);
    `ws-overload-callresult-guards` (the inner-same-arity-overload-set
    decline NEGATIVE + the two mandatory shadowed-`Format`/`CopyStr`
    POSITIVE proofs); `ws-overload-pageext-callresult` (the addenda-mandatory
    PageExtension-merge-single-route POSITIVE + two-visible-extensions
    NEGATIVE decline — proves Task 3 composes correctly with Task 1's merge
    through the SAME `resolve_member` call). The orphaned
    `ws-overload-callexpr-discriminator` bank is now WIRED to its positive
    outcome (documented rebaseline, not a regression — "rename, don't just
    flip").
  - Full CDO harness (single-threaded): `ambiguousResolved` 7→0 — EVERY
    remaining site flipped, exceeding the plan's own "~4-5" grounding
    estimate. That estimate's "3 sites are SymbolOnly-receiver-blocked"
    premise was FALSIFIED by measurement (CDO's dependencies ship embedded/
    ShowMyCode source, so their receivers are ordinary `RouteTarget::Routine`
    candidates, never `AbiSymbol`) — all 7 sites individually hand-traced
    against real embedded/workspace source (2× `PrintPDFFile`, 1×
    `SendElectronicDocument`, 1× `LogMessage` — a Continia dependency, source
    extracted directly from its `.app` zip package —, 2× `AddUserMessage`
    against Microsoft's real `AOAI Chat Messages` System Application object,
    1× `AddAttachment` against Microsoft's real `Email Message` object); see
    `cdo_full_program_coverage_and_self_reported_metric`'s updated ratchet
    comment for the full per-site adjudication. `real_unknown_rate`/`unknown`
    stay at the floor (0), `genuine_wrong`=0 throughout
    (`cdo_genuine_wrong_is_precedence_adjudicated` +
    `cdo_l3_semantic_audit_no_fresh_wrong`, both re-run and green). Coverage
    (`total`) grows 18104→18108 (primary) / 43404→43408 (whole-program) — an
    honest, additive side effect of the `in_expression` lowerer completeness
    fix surfacing previously-invisible nested call obligations;
    `coverage.holds` stays `true` throughout (no orphaned obligation).

### Fixed
- **`tree-sitter-al` repinned to `v3.1.0` (local, pre-publish) — `recoveredFiles` 8→0, zero-metric strictness held
  (grammar-defects-and-repin plan, Task 1).** The `ParseStatus::Recovered` diagnostic (introduced in the preproc
  foundations plan's Task 3) surfaced two genuine `tree-sitter-al` grammar gaps, both confined to dependency
  (embedded ShowMyCode) source — this task fixes them at the grammar source rather than filing them as a caveat:
  - **`OptionMembers = TableData,...` first-position collision** (bare, unquoted, case-insensitive `tabledata` as
    the FIRST option member lexically collided with the `tabledata` keyword — hit MS `System`'s `Object.Table.al`,
    `NAVAppObjectPrerequisites.Table.al`, `DatabaseLocks.Table.al`) and **`# pragma` whitespace intolerance** (a
    single space between `#` and `pragma` was rejected outright — hit Continia System Application's
    `Http.Codeunit.al`). Grammar fix: a hidden `_tabledata_keyword` rule aliased to `identifier` in `option_member`
    (no new visible node kind); `pragma`/`preproc_region`/`preproc_endregion` tightened to `[ \t]*` (horizontal-only
    — an audit found `preproc_region`/`preproc_endregion` shared the identical `\s*` cross-line hazard, closed in
    the same pass).
  - **Reviewed and reverted:** a preventive fourth fix — `# if`/`# elif` (single space) as literal variants mirroring
    the existing `# else`/`# endif` precedent — was drafted (zero corpus instances) but DROPPED after empirical
    review found it introduced genuine GLR non-determinism (the pre-existing
    `preproc_split_if_then_begin_else_shared` construct, given a spaced open, produced two mutually-exclusive
    stable parses across process states for byte-identical input — `tree-sitter test`'s own pass count flapped
    1453↔1463 with zero source change) plus a silent shape defect under `#if`-nesting (the literal-variant token
    doesn't participate in the scanner's depth counter, so a nested spaced `# if` undercounted depth and lost its
    enclosing `begin_keyword`/`end_keyword` naming). Current, intentional behavior: a spaced `# if`/`# elif` is NOT
    recognized — the file `Recover`s (this diagnostic's designed detection path) instead of parsing silently wrong
    or non-deterministically. Post-revert stability protocol: OS tree-sitter parser cache cleared, `tree-sitter test`
    repeated 5× clean — 1463/1463 every time (byte-stable, non-determinism gone); the reviewer's exact
    `preproc_split_if_then_begin_else_shared`-with-spaced-open repro and a `#if`-nesting-with-inner-spaced-`# if`
    repro both re-parsed 5× with identical tree-hash every time.
  - **Validation, LOCAL only (grammar submodule `f150581`→`6d87aee`, not yet pushed):** `tree-sitter test`
    1463/1463 (1448 pre-existing + 15 new, incl. cross-line LF negatives and the `# if`/`# elif`
    documented-non-support fixtures); `tools/tree-harness.sh` byte-identical before/after on CDO source (551 files)
    and BC.History (this checkout's corpus, 15,358 files) — zero shape change outside the 4 previously-Recovered
    dependency files, and manifest-identical with vs. without the reverted `# if`/`# elif` variants (proving the
    revert changed zero previously-parsed trees); `parse-al-parallel.sh` re-run on the same BC.History corpus:
    15358/15358, 0 errors, 100.0% success; `cargo run -p xtask -- gen-syntax` produced a byte-for-byte identical
    RawKind vocabulary (388 named kinds / 73 fields / 388 typed structs / 13 union enums — after the spaced-if
    revert the generated directory is byte-identical to the pre-plan baseline, `GRAMMAR_NODE_TYPES_HASH` included);
    `cargo test --workspace` zero divergence (159 `test result: ok` blocks); the FULL CDO harness
    (`CDO_WS`/`ENFORCE_CDO_WS=1`, release, single-threaded) confirms `recoveredFiles` 8→0 with **nothing else
    moving**: primary/whole-program `real_unknown_rate`/`unknown` stay at the floor (0/18108, 0/43408),
    `ambiguousResolved` stays 0 (both scopes), `genuine_wrong` stays 0, coverage/determinism gates unchanged, all
    companion CDO gates (`cdo_genuine_wrong_is_precedence_adjudicated`, `cdo_l3_semantic_audit_no_fresh_wrong`,
    applicability) green.
  - Publishing (grammar `origin main` + tag `v3.1.0`, engine pin bump to the public SHA) is a separate follow-up
    task — this entry covers the local-only validation.
- **pageext-merge-and-final-residual arc complete — CDO primary real-`unknown` reaches THE ZERO: 0.0000%
  (0/18108), and `ambiguousResolved` also reaches 0 (Task 4, FINAL — contingent close).** Full re-measure
  (`CDO_WS`/`ENFORCE_CDO_WS=1`, release, single-threaded, 182-test suite) confirms the 3-task arc at its floor,
  byte-identical to Task 3's own measurement:
  - **The zero, both dimensions.** Primary-scoped: `total`=18108, `unknown`=0 (`real_unknown_rate`=0.0000%),
    `unknownByReason`={} (empty — every reason bucket, not just the count, is empty). Whole-program: `total`=43408,
    `unknown`=0. `ambiguousResolved`=0 (both scopes, hard-gated `assert_eq!`) — so the **legacy-inclusive** rate
    (`(unknown + ambiguousResolved) / total`, the pre-sigfp-reclassification-plan metric definition) is ALSO exactly
    0.0000%, not merely the narrower post-reclassification metric: every statically-resolvable call obligation on the
    CDO reference corpus resolves, under either metric definition.
  - **The honest-taxonomy composition, stated in full** (never just "0 unknown" — the residual is real, not vacuous):
    primary `resolved_source`=8279, `resolved_catalog`=5890, `resolved_abi_external`=4, `conditional_resolved`=17,
    `honest_dynamic`=42, `honest_empty`=3876 (sums to 18108); whole-program `resolved_source`=10173,
    `resolved_catalog`=5890, `resolved_abi_external`=4, `conditional_resolved`=319, `honest_dynamic`=42,
    `honest_empty`=26980 (sums to 43408). The `honest_dynamic`/`honest_empty` buckets are the PROVABLY-open residual
    (runtime-typed dispatch and zero-obligation edges respectively) — never conflated with `unknown`.
  - **Companion gates, all re-confirmed:** `genuine_wrong`=0 (`cdo_genuine_wrong_is_precedence_adjudicated`: 54/54
    `known-genuine-divergences.json` overrides independently re-verified — `SameAppSourceProcedure`/
    `CrossAppSourceProcedure` targets re-read directly off disk, `fresh_false_builtin=0 needs_manual_review=0`); L3
    semantic audit `fresh_missing`=0, `fresh_wrong`=149 (ALL 149 adjudicated `fresh_ahead_dispatch`, zero
    `genuine_wrong` among them — `matches`=6120, `fresh_extra`=5108, `fresh_novel`=6693, `golden_missing`=89);
    `route_applicability_zero_violations`/`fan_out_applicability_zero_violations` both 0 violations with non-vacuous
    `routes_checked` (`total_routes`=18590; interface=28, instance_builtin=482, implicit_trigger=1183, event=3404);
    `recoveredFiles`=8 (pinned exact, unchanged — the 2 known `tree-sitter-al` grammar defects, both dependency-only).
  - **The falsified-premise lesson, told for the SECOND time (append-only errata, no rewrite of the original
    claims).** This arc's own "Key facts" preamble already named the first instance (plan-9's "13 workspace
    `MemberNotFound` absences" were actually the `is_metadata_sensitive_instance_method` catalog gap, not absences —
    see the argtype-dispatch-and-page-catalog plan's own capstone entry below). This arc supplies the SECOND: the
    "**Receiver-closure arc complete**" entry immediately below this one, and the argtype-dispatch-and-page-catalog
    Task 1 entry further below, both state the 7 `MemberNotFound` eCandidates sites as "verified-REAL absences" /
    "genuinely absent members, not an engine gap". **That claim was false.** Task 1 of THIS plan found the true cause:
    CDO's own workspace declares all 3 missing members in `Al/Extensions/eCandidates/CDOConnecteCandidates.PageExt.al`
    (`internal`, same-app-visible) — the engine simply never merged a `PageExtension`'s routines into its base
    `Page`'s member-resolution scope (the `Table`/`TableExtension` analog, `resolve_in_table_scope`, existed; no
    `Page` equivalent did). Fixing the merge resolved all 7 sites to `Resolved`/`Source` — they were an ENGINE GAP,
    not a verified absence, exactly the same shape of mistake as plan-9's. **The doctrine, now recorded twice:**
    measure the actual population (read the real source, don't infer "probably absent" from a dependency `.app`
    alone) before building any taxonomy — including `ProvenAbsent` — for it. Neither historical claim above is
    rewritten; this entry is the dated correction.
  - **Deferred, visible (Roadmap — see the plan doc's own "Roadmap — beyond this plan" section for the full
    detail):** `ProvenAbsent` machinery — DEFERRED-WITH-BLUEPRINT (the full 8-obligation proof table, the
    `Route::proven_absent` marker + `ObligationOutcome::ProvenAbsent` design, the `recoveredFiles`-consult invariant,
    and the `app_content_hash`-anchored cache-invalidation requirement are recorded in the plan doc, not implemented —
    `MemberNotFound`==0 on CDO means there is currently no population to validate it against); ABI param-type
    retention (SymbolOnly dispatch — now the ONLY remaining `ambiguousResolved` lever, population-less on CDO today);
    Report/ReportExtension routine-merge (mechanically cheap per Task 1's index inspection, zero measured population
    motivating it); the 2 pinned `tree-sitter-al` grammar defects (`OptionMembers=TableData,...` keyword collision,
    `# pragma` with a stray space); the `.dependencies/CDO` same-slug double-include root cause; implicit-conversion
    modeling; protected `Variables[]`; `Sender` parameter-TYPE validation; Step-4b `WithState` symmetry (opus A).
- **PageExtension routine merge into base-Page member resolution — real-`unknown`
  0.0497%→0.0110%, `MemberNotFound` 7→0 (Task 1, pageext-merge-and-final-residual
  plan).** Closes the engine gap the plan's grounding report identified on
  `9b5f3de`: `resolve_member`'s `ReceiverType::Object` arm dispatched a `Page`-typed
  receiver via a plain `resolve_in_object(target_id, ...)` call on the base page
  alone — a `PageExtension`'s routines are indexed under the EXTENSION's own
  `ObjectNodeId` (`node_extract::extract_nodes`), so they were structurally
  unreachable from a base-Page-typed receiver, exactly mirroring the gap
  `resolve_in_table_scope`/`table_extensions_of` already closed for
  Table/TableExtension. Added the `Page` analog: `ResolveIndex::page_extensions_of`
  (`index.rs`, mirrors `table_extensions_of` exactly) + a new `resolve_in_page_scope`
  (`resolver.rs`) wired into the `Object` arm for `ObjectKind::Page` receivers only,
  BEFORE the instance-builtin catalog fallback. CALLER-closure-anchored visibility
  (never receiver-object-closure-anchored — an extension is a candidate only when
  ITS OWN app is reachable in the CALLING object's dependency closure), the existing
  `internalsVisibleTo`/`Local`/`Protected` access model applied unchanged, and
  aggregate-then-adjudicate (every visible candidate — base ∪ every visible
  extension — collected FIRST, fed to the SAME ambiguity machinery
  `resolve_in_table_scope` uses; no first-wins). Diverges from
  `resolve_in_table_scope` in one deliberate way: preserves the pre-merge
  per-object `ArityMismatch`/access-exclusion diagnostic (a name-bearing-but-
  wrong-arity candidate is forwarded to `resolve_in_object` for its own honest
  reason rather than collapsing into a bare `MemberNotFound`) — `resolve_in_table_scope`'s
  own cardinality check folds arity-exact matching into existence, making its
  `ArityMismatch` branch provably unreachable; that pre-existing Table-arm
  behavior is untouched (out of scope). 8 new TDD fixtures in `resolver.rs`
  (same-app internal resolves; different-app internal declines
  `InternalNotVisible`, not a bare `MemberNotFound`; out-of-closure extension is
  structurally invisible `MemberNotFound`; two caller-visible extensions
  declaring the same member → `OverloadAmbiguous`, no first-wins; base-vs-
  extension exact duplicate → `OverloadAmbiguous`, defensive-only — AL0115/AL0226
  make both uncompilable in real source; base-only calls + the instance-builtin
  catalog fallback unchanged; arity-mismatch on a base-only candidate preserves
  `ArityMismatch`; a `public` extension procedure from a transitively-depended-on
  app resolves — the cross-app-legal case) + 3 `page_extensions_of` index unit
  tests. Full CDO harness (`CDO_WS`/`ENFORCE_CDO_WS=1`, 173 tests,
  single-threaded): primary/whole `unknown` 9→2 (`real_unknown_rate`
  0.0497%→0.0110%, exact 2/18104), `unknownByReason`={UntrackedReceiver: 1,
  BuiltinPrecedenceCollision: 1} — `MemberNotFound` fully closed (all 7 sites,
  `CDOeCandidatesEventHandler.Codeunit.al` calling `GetOutputProfile`/
  `OnlyVendorsAreHandled`/`OnlyCustomersAreHandled` on `Page "CTS-CDN Connect
  eCandidates"`, declared `internal` in `Al/Extensions/eCandidates/
  CDOConnecteCandidates.PageExt.al`, same app as every caller — resolved). L3
  preflight site ledger (a before/after toggle-diff re-run, not a guess):
  `matches` 6127→6120 (−7), `fresh_extra` 5100→5107 (+7), `fresh_wrong`=149,
  `fresh_missing`=1, `genuine_wrong`=0 — all BYTE-IDENTICAL before/after except
  the expected ±7 `matches`/`fresh_extra` swap, confirming the 7 sites move
  cleanly from "L3 golden also empty there" to "fresh now ahead of L3" — never
  `fresh_wrong`, never unexplained (the "L3 likely also missed these" prediction,
  confirmed). `ambiguousResolved`=7 unchanged (exact, both scopes).
  `genuine_wrong`=0 throughout. **Report/ReportExtension merge: DEFERRED**
  (dated 2026-07-04) — the index-level `report_extensions_of` analog would be
  mechanically cheap (`extends_target` is already populated identically for
  `ReportExtension`), but the `ArityMismatch`-preserving resolver logic is
  bespoke, not a mechanical index swap, and needs its own dedicated fixtures +
  a fresh CDO measurement; the population motivating this task is 100% Page,
  zero measured Report-typed cross-extension receiver calls on CDO.
- **The Page implicit-Rec field arm + global suppression of instance-only
  names from bare-builtin probing — real-`unknown` 0.011%→0.0000%, the FLOOR
  (Task 2, pageext-merge-and-final-residual plan).** Two independent fixes.
  (a) Widened `receiver.rs` Step 3a (the bare implicit-Rec quoted/unquoted
  FIELD arm) from `Table | TableExtension` to also cover `Page |
  PageExtension`, via the existing `resolver::implicit_rec_table_id` (now
  `pub(crate)`) — the same per-kind lookup `resolve_bare`'s Step 3 already
  used for the analogous bare-CALL case, so the two paths can't drift apart.
  Closed a soundness gap the widening exposed: for Page/PageExtension the
  implicit-Rec table is a DIFFERENT object than the caller, so the
  pre-existing `table_scope_has_routine` shadow guard alone can't see a
  same-named routine the PAGE ITSELF declares; added
  `ResolveIndex::routines_in_object` as an additional OR'd guard (a no-op for
  Table/TableExtension, the missing half for Page/PageExtension). Fixes the
  real site `"View (Blob)".CreateInStream(ReadStream)` in Page 6175411
  (`.dependencies/CDO/Page/CDOPageDefaultFilters.Page.al:88`, `SourceTable`
  field(28)). (b) A compiler-grounded, GLOBAL suppression of 19
  `member_catalog::PAGE_INSTANCE` names (`Run`/`RunModal`/`Close`/`Update`/
  `Activate`/`CancelBackgroundTask`/`Caption`/`Editable`/
  `EnqueueBackgroundTask`/`GetBackgroundParameters`/`GetRecord`/`LookupMode`/
  `ObjectId`/`PromptMode`/`SaveRecord`/`SetBackgroundTaskResult`/`SetRecord`/
  `SetSelectionFilter`/`SetTableView`) from the BARE-call builtin candidate
  set (`resolver::INSTANCE_ONLY_NEVER_BARE`, MS-Learn-cited per name):
  `GLOBAL_BUILTIN_METHODS` (785 names) is a straight union of ALL 97
  AL-compiler-documented types' methods with zero regard for receiver
  requirements — cross-referencing the generator's own per-type dump
  (`tools/gen-al-builtins/out/member_builtins.json`) proves every one of
  these 19 names is owned EXCLUSIVELY by receiver-qualified instance types
  (Page/Codeunit/Report/Xmlport/Dialog/RecordRef/…), never the "System"
  pseudo-bucket the same dump shows houses genuinely bare-global names
  (`Format`/`Today`/`GuiAllowed`) — confirmed against MS Learn per name.
  `Message`/`Error`/`Confirm` are the deliberate near-miss (also filed under
  a receiver-shaped `Dialog` bucket, but MS Learn is explicit they're
  callable "without specifying the data type name") and are correctly NOT
  suppressed. The suppression gates BOTH of `resolve_bare`'s consuming call
  sites — the Step 3 PROBE-THEN-DECIDE collision guard
  (`is_bare_builtin_or_page_intrinsic`) and the Step 4 plain catalog
  fallback — never `resolve_member` (the qualified path is structurally
  untouched: only 2 `global_builtin_id` call sites exist in `resolver.rs`,
  both inside `resolve_bare`). Fixes the real site
  `CDOEMailJobs.Page.al:125`'s bare `Run()` (previously an unproven
  `Unknown(BuiltinPrecedenceCollision)` against `CDOEMailJob.Table.al:192`'s
  `procedure Run()`) to resolve correctly to the table's own procedure; a
  bare call to one of the 19 names with NO source candidate anywhere now
  correctly declines to `Unknown` instead of a false `Catalog`/`Builtin`
  route (verified in both a Page and a Codeunit context — the Codeunit case
  exercises Step 4's guard in isolation, since Step 3 is structurally
  skipped for Codeunit). An ungrounded name (e.g. `Rename`, a real
  `Record.Rename` also present in the 785-name union but never individually
  grounded) keeps the pre-existing fail-closed collision behavior unchanged
  — scope discipline, not a blanket "any collision wins" change. 11 new TDD
  fixtures across `receiver.rs`/`resolver.rs`. Discovered and corrected a
  STALE adjudication in the process: the CDO L3-audit manifest already
  carried an entry for this exact site, adjudicated long ago as a
  `builtin-catalog-fp-collision` (fresh's OWN disposition there WAS
  `Catalog`/`Builtin(run)` at the time, because that adjudication checked
  only the SAME UNIT for a competing procedure, never the page's own
  SourceTable) — corrected IN PLACE (same site key, entry count unchanged at
  54) to a new `SameAppSourceProcedure` adjudication shape (the same-app
  analog of the existing `CrossAppSourceProcedure` shape, verified against
  the live workspace source tree directly since a workspace never carries
  its own compiled `.app` in `.alpackages`), never duplicated. Full CDO
  harness (173 tests, single-threaded): primary/whole `unknown` 2→0
  (`real_unknown_rate` 0.011%→0.0000%, the floor), `unknownByReason`
  becomes `{}` (empty — first time ANY reason bucket is empty), every OTHER
  bucket (`ambiguousResolved`=7, `resolved_catalog`, `resolved_source`)
  byte-identical, `genuine_wrong`=0 throughout (the one stale entry
  corrected, not newly failing). A dedicated blast-radius sweep of the
  entire CDO workspace (both `Al/` and `.dependencies/` folders) for every
  bare occurrence of the 19 suppressed names confirmed these are the only 2
  real behavior-changing sites in the whole corpus. Both `unknown`
  count-ceiling ratchets and the rate ceiling re-derived and tightened to
  the new measured floor. **Task 2 review fix (dated 2026-07-04):** the L3
  semantic-audit ledger above was independently re-derived and found to
  contain an arithmetic error — `matches`/`fresh_wrong` were reported
  unchanged (6120/149) when the TRUE movement was `fresh_missing` 1→0 (Site
  B, the bare `Run()` above, was the sole `fresh_missing` occupant and moved
  into `matches` once it resolved with source evidence identical to L3's own
  frozen golden — an HMAC re-verification confirmed L3 was never wrong here,
  only fresh's pre-Task-2 answer was) and `fresh_extra` 5107→5108 (Site A,
  the Page implicit-Rec field arm, independently moved `matches`→`fresh_extra`
  for an unrelated reason). `FRESH_MISSING_CEILING` re-derived and tightened
  `5→2` from the corrected measured value (0, not 1); `genuine_wrong`=0 and
  `ambiguousResolved`=7 unaffected throughout. See
  `.superpowers/sdd/task-2-report.md` §9 for the full correction.
- **Receiver-closure arc complete — real-`unknown` 0.43%→0.05%, `ambiguousResolved`
  13→7 (Task 5, FINAL, receiver-closure-and-arg-increments plan).** Full CDO
  re-measure confirms the 4-task arc at its floors, byte-identical to Task 4's
  own measurement: primary `unknown`=9/18104 (`real_unknown_rate`=0.0497%,
  ceiling 0.000498); whole-program `unknown`=9/43404 (0.02%); the
  **legacy-inclusive** rate (`unknown + ambiguousResolved`, the
  pre-sigfp-reclassification-plan metric definition) is (9+7)/18104≈0.088%,
  both scopes. `unknownByReason`={UntrackedReceiver: 1,
  BuiltinPrecedenceCollision: 1, MemberNotFound: 7}, sum==9 —
  `CompoundReceiver` no longer appears (0 sites). `ambiguousResolved`=7
  (primary/whole, exact). `genuine_wrong`=0 throughout every task.
  `route_applicability`/`fan_out_applicability` violations=0 with
  non-vacuous `routes_checked`; `recoveredFiles`=8 (pinned, unchanged); the
  sig_fp collision-guard group count stays 0/0 (inherited from the
  sigfp-and-ambiguous-reclassification plan — untouched by this arc, which
  never modifies ABI/source fingerprint fold logic; all sig_fp-related
  fixture tests re-ran green). All of the above confirmed by an independent
  single-threaded 173-test CDO re-run under `ENFORCE_CDO_WS=1`, byte-identical
  before/after this task's own doc-only nit sweep.
  - **The 100%-mechanical-population story, closed.** Two grounding reports
    (session start) enumerated ALL 69 `CompoundReceiver`+`UntrackedReceiver`
    sites against real CDO source and found them 100% mechanical — zero
    genuinely-dynamic residue. Across Tasks 1-4: **68 of the 69 closed**
    (Task 1: 37 `CurrPage.<usercontrol>` ControlAddIn sites, `CompoundReceiver`
    51→14, via a closed-if-known tri-state gate + an `interface_procedure`
    lowering foundation fix that also made Interface/ControlAddIn procedure
    signatures visible to the LSP front-end for the first time; Task 2: 9
    parens-less zero-arg framework members + 4 `ErrorInfo.CustomDimensions`
    sites, `CompoundReceiver` 14→1, via a context-sensitive zero-arg lookup
    that FALSIFIES this codebase's THIRD recurrence of the "AL procedures
    ALWAYS require parens" premise (the module doc claim and its enforcing
    negative test both corrected — see the al-parens-optional-procedure-calls
    memory); Task 3: 11 named-return-binding sites + 3 implicit-self
    table-field sites, `UntrackedReceiver` 18→4, via a cross-crate al-syntax
    lowerer fix (`RoutineDecl.return_name`) plus a proven bare-identifier
    precedence order; Task 4: the 4-site (D)/(F)/(G) enum-shape population,
    `UntrackedReceiver` 4→1, via split enum TYPE-static/VALUE-instance
    catalogs). **The 1 residual is an HONEST, explicitly out-of-scope gap**
    (`"View (Blob)".CreateInStream(...)` on a Page's implicit-Rec
    SourceTable-field shorthand — Step 3a is Table/TableExtension-only by
    design, not a Page arm; see Roadmap below), not a failure to close.
  - **The residual 9, stated plainly:** 0 `CompoundReceiver`; 1
    `UntrackedReceiver` (the honest Page-gap above); 1
    `BuiltinPrecedenceCollision` (a pre-existing, independently-adjudicated
    collision, untouched by this arc); 7 `MemberNotFound` (verified-REAL
    eCandidates absences — genuinely absent members, not an engine gap;
    ProvenAbsent machinery to formalize that proof is deferred, see
    Roadmap).
  - **`ambiguousResolved` 13→7**, alongside: Task 3 flipped 2 (the
    `GetJsonAttribute` family's named-return-typed `var` args); Task 4
    flipped 4 (3 member-field-arg discriminators + the `with`-scan
    comment-blindness restoration for `UseContiniaAuthorization`). Every
    flip individually adjudicated compiler-correct against real CDO field
    declarations (see `.superpowers/sdd/task-3-report.md` /
    `task-4-report.md`).
  - **Nit sweep (Task 5):** corrected 4 stale doc claims found while closing
    out — (1) the CDO L3 semantic audit's `FRESH_MISSING_CEILING` doc-comment
    still said "measured 3" from its 2026-07-03 pin, though the LIVE value was
    already 1 by Task 2 of this plan (`task-2-report.md` independently
    recorded it, byte-identical before/after that task); (2)
    `zero_arg_aware_lookup`'s bare-`Member` branch gained a
    `debug_assert_eq!(arity, 0)` documenting/enforcing the caller invariant
    that a bare `Member` is always zero-arg by construction; (3) the Task 4
    CHANGELOG entry's own receiver-arms bullet still described the
    `QualifiedEnum` else-branch as "grammar-guaranteed enum-value-literal" —
    the exact claim that same Task's review fix (above) proved FALSE
    (an Option-typed field base parses identically); reworded to match the
    corrected, final (recursive-verification) design; (4) two `resolver.rs`
    test comments dating from before the sigfp-and-ambiguous-reclassification
    plan's Task 2 still asserted "source `sig_fp` is always 0" as present
    fact — verified false by direct measurement (`Foo(Integer)`/`Foo(Text)`
    now fingerprint to genuinely distinct `sig_fp`s; the observed decline
    reason is deterministically `AccessFilteredOverload`, not the
    historically-described `InternalNotVisible`), corrected in both sites.
    None of these were behavior changes — pure doc/comment corrections plus
    one debug-only assertion; the CDO gate stayed provably byte-identical
    before/after.
  - **Deferred, visible (Roadmap below):** `ProvenAbsent` machinery for the
    7 `MemberNotFound` sites (consult `recoveredFiles` per the completeness
    invariant before any future absence claim); the Page-owned implicit-Rec
    field arm (the 1-site residual); builtin/member call-result argument
    typing; ABI parameter retention (SymbolOnly dispatch); the 2 pinned
    tree-sitter-al grammar defects (`OptionMembers=TableData,...` keyword
    collision, `# pragma` with a stray space); the `.dependencies/CDO`
    same-slug double-include root cause; implicit-conversion modeling;
    protected `Variables[]`; `Sender` parameter-TYPE validation.
- **Qualified-value bases are now verified Enum-typed before VALUE-instance
  dispatch — Option-qualified receivers decline (Task 4 review fix).** The
  `QualifiedEnum` receiver arm's "else" branch (the (D) field::value chain
  below) classified ANY non-`Enum::"Type"` qualified-value base as
  `ReceiverType::EnumType` on the strength of a doc-comment claim that the
  grammar guarantees every such shape is enum-VALUE-typed. That claim is
  FALSE: the grammar only guarantees the `X::Y` SHAPE — an **Option**-typed
  field base (`Rec."OptField"::Val`, common legacy AL) parses to the
  identical `QualifiedEnum` node and reached the same unconditional accept.
  Harmless today (Option values have zero auto-methods per MS Learn, so no
  member call on one can resolve; CDO `genuine_wrong=0` throughout), but a
  violation of the never-guess cardinal rule. The arm now recurses the same
  base-typing every other compound-receiver arm uses
  (`infer_receiver_type_for_expr` on the `enum_type` base) and accepts
  VALUE-instance dispatch ONLY when the base actually types Enum-shaped
  (`EnumType` — a declared `Enum "X"` field/var — or `EnumTypeStatic` for
  nested `Enum::"Type"::"Value"`); anything else (an Option field's
  `Primitive`, an unresolvable field's `Unknown`, `Record`, …) declines,
  fail-closed. Doc comments corrected to the TRUE invariant. Two new
  negative fixtures (Option-field base, unresolvable-field base) + the
  existing (D)-shaped enum-field chain as the regression pin. CDO
  byte-identical: real-`unknown` 9/18104 [0.0497%], `ambiguousResolved` 7,
  `genuine_wrong` 0.
- **Enum-shape receivers, member-field arg dispatch, comment-aware with
  scan: real-`unknown` 0.072%→0.05%, `ambiguousResolved` 11→7
  (receiver-closure-and-arg-increments plan, Task 4).** Closes the 4-site
  (D)/(F)/(G) enum-shape receiver population, adds member-field argument
  typing, and fixes a comment-blindness bug in the `with`-scope raw-text
  scan:
  - **Split enum catalogs (round-2 closer, BINDING):** a new
    `FrameworkKind::EnumTypeStatic` + `ReceiverType::EnumTypeStatic{name_lc}`
    represent the enum TYPE reference itself (`Enum::"Type"` / a bare
    enum-type-name receiver), distinct from the existing `EnumType`
    (a declared `Enum "X"`-typed VALUE, or an enum-value-literal chain).
    Two closed catalogs, per MS Learn `enum-data-type` (fetched 2026-07-04):
    VALUE-instance = `AsInteger`/`Names`/`Ordinals` (`FromInteger` removed —
    it was WRONGLY reachable from a value receiver pre-fix, a real
    correctness bug); TYPE-static = `FromInteger`/`Names`/`Ordinals`
    (`AsInteger` excluded — no value to convert via a bare type reference).
  - **Receiver arms:** `infer_receiver_type_for_expr` gains an
    `ExprKind::QualifiedEnum` arm — `Enum::"Type"` (fail-closed existence
    check against a real `Enum` object) → `EnumTypeStatic`; any other
    `X::Value` shape (e.g. `Rec."Field"::Value`) recurses the SAME
    base-typing every other compound-receiver arm uses and accepts
    `EnumType` ONLY when that base actually verifies Enum-shaped (a doc
    claim that the grammar alone GUARANTEES this shape is enum-VALUE-typed
    turned out to be false — an Option-typed field base parses identically;
    see this same Task's own "Fixed" review-fix entry above, landed in the
    same commit range, for the full correction). `infer_receiver_type` gains a new
    Step 4b: a bare (quoted or not) enum-type-name receiver resolves to
    `EnumTypeStatic` ONLY when the programmatic collision rule passes
    (`same_normalized_name && kind != Enum`, checked over the WHOLE object
    index, never closure-scoped) AND no routine shadow exists
    (`object_scope_has_bare_routine_shadow`, generalizing the existing
    `table_scope_has_routine` precedent to every object kind).
  - **Member-field arg typing:** `arg_dispatch::type_one_arg` gains an
    `ExprKind::Member` arm — `Foo(Rec.Field)` / `Foo(Rec."Quoted Field")`
    types via the SAME `field_in_table` + `table_scope_has_routine`
    machinery `receiver.rs`'s record-field arm uses, gated on
    `WithState::NoWithProven`, a bare (not multi-hop) declared-var
    (not implicit-Rec) Record base. `var_passable: false` HARDCODED (AL
    requires a variable for a `var` argument — a field expression never
    qualifies) — a fixture proves this ELIMINATES a sibling `var`-mode
    overload candidate that would otherwise degrade the pick.
  - **Comment-aware with-scan:** `extract::routine_has_with_token` rewritten
    from a raw substring search into a lexer-lite scanner that excludes
    `// ...` line comments, non-nested `/* ... */` block comments, `'...'`
    string literals (AL `''` escaping), and `"..."` quoted identifiers
    (`""` escaping) before checking for a standalone `with` token;
    unterminated comments/strings/quoted-identifiers conservatively count as
    a hit (uncertain → conservative, never a false negative). The two-signal
    AST-depth + raw-text-scan design is unchanged — only the text signal's
    precision improved.
  - **CDO gate:** primary real-`unknown` 13→9 (0.072%→0.0497%
    [9/18104=0.0004971]); `CompoundReceiver` 1→0 (site D), `UntrackedReceiver`
    4→1 (sites F×2 + G — the residual 1 is the honest Page-SourceTable
    implicit-field gap, out of scope by design); `BuiltinPrecedenceCollision`
    stays 1; `MemberNotFound` stays 7 (honest eCandidates absences).
    `ambiguousResolved` 11→7: 3 member-field-arg flips
    (`CreateReportUsingTemplateLineReports`'s `."Output Format"` enum-typed
    field discriminator + `."Report Layout Name"`/`"AppID"` non-discriminating
    fields; `CreateReportUsingReportSelection`'s equivalent quoted-field
    args; `PrintPDFFile`'s own `DOPrintDocument.Printername`) plus the
    `UseContiniaAuthorization`/`Authorize` restoration (the with-scan
    comment-blindness fix) — every flip individually adjudicated against
    real CDO field declarations (source-verified, not merely plausible; see
    the harness ratchet comment and `.superpowers/sdd/task-4-report.md`).
    `genuine_wrong`=0, L3 semantic audit unchanged. Ratchets re-derived
    (rate ceiling 0.000719→0.000498, unknown-count ceilings 27→9,
    ambiguousResolved 11→7), dated 2026-07-04.
- **Named-return-value bindings + implicit-self table fields — receiver and
  arg typing: real-`unknown` 0.15%→0.072%, `UntrackedReceiver` 18→4,
  `ambiguousResolved` 13→11 (receiver-closure-and-arg-increments plan,
  Task 3).** Closes the 11-site (E) named-return-binding population plus the
  3-site (H) implicit-self Table/TableExtension field population, and flips
  2 (#9/#10) previously-ambiguous overload picks:
  - **Root cause.** `procedure X() Ret: Record Y`'s NAMED-RETURN binding
    name was discarded entirely at lowering (`al_syntax::lower::lower_routine`
    only ever read the grammar's `return_type` field, never `return_value`),
    so a mid-body bare reference to `Ret` (`Ret.Get(...)`) had no scoped
    symbol to type against — `UntrackedReceiver`. Separately, Step 2's
    variable lookup had no arm at all for an implicit-self TABLE field
    referenced by a BARE UNQUOTED name (`Attachment.CreateInStream(X)`
    inside the table's own procedure, no `Rec.` prefix) — only the QUOTED
    form (`"File Blob".CreateInStream(...)`) was wired, from an earlier
    task.
  - **Lowerer fix (cross-crate, al-syntax):** `RoutineDecl` gains
    `return_name: Option<String>`, captured from the grammar's
    `_procedure_named_return`'s `return_value` field (unquoted, mirroring
    `Param`/`VarDecl` name storage); `None` for an anonymous `: Type` return
    or no return spec at all. Full workspace suite re-run; zero golden
    movement (the field is additive and was previously read by nothing).
  - **The proven precedence** (round-2 closer, BINDING — the task report has
    the full compiler-fixture citation): param/local (same scope, mutually
    exclusive with each other AND the named-return binding — any collision
    is a compile error) → the routine's own named-return binding → object
    globals → [routine-shadow check, parens-optional] → implicit-self table
    fields LAST among value symbols. A new SHARED helper,
    `receiver::caller_scope_symbol` (+ `CallerScopeSymbol` tri-state:
    `Found`/`NotFound`/`MalformedDuplicate`), encodes exactly this
    param→local→named-return→global order and is used by BOTH
    `receiver.rs`'s Step 2 and `arg_dispatch.rs`'s `type_one_arg` caller-
    scope-exact arg lookup — the two lookups can no longer drift.
  - **SAME-SCOPE-ONLY malformed-duplicate rule:** a named-return binding
    colliding with a param/local of the identical name (never legal AL — a
    compile error) declines outright (`Unknown`/untyped) for that
    identifier, rather than picking a winner; a binding SHADOWING a
    same-named GLOBAL is ordinary, valid AL precedence — the binding wins.
    Both directions fixture-proven in both `receiver.rs` and
    `arg_dispatch.rs`.
  - **Implicit-self field arm widened:** Step 3a's existing quoted-field
    machinery (`ResolveIndex::field_in_table` + the `table_scope_has_routine`
    routine-shadow guard, `WithState::NoWithProven` gate) now ALSO accepts
    an unquoted bare identifier — the SAME code path, just without the
    `starts_with('"')` restriction (defensively excludes literal unquoted
    `rec`/`xrec`, which fall through to the Step 3b identity fallback
    unchanged). Table/TableExtension only, exactly as before; non-Table
    objects (including a Page's own SourceTable-implicit-field shorthand)
    are explicitly OUT of scope and unaffected.
  - **The #9/#10 arg-typing flip:** `arg_dispatch::type_one_arg`'s
    caller-scope-exact lookup (now via the shared helper) types a
    bare-identifier ARG that is the caller's own named-return binding
    exactly like a local — enough evidence for `pick_candidate` to
    disambiguate a `var`-typed overload position that was previously always
    untyped (no way to find the binding in caller scope at all).
  - **CDO gate:** primary real-`unknown` 27→13 (0.15%→0.0718%
    [13/18104=0.0007181]); `UntrackedReceiver` 18→4 (-14: 11 named-return
    sites + 3 implicit-self-table sites, ALL individually adjudicated
    against real CDO source — see `.superpowers/sdd/task-3-report.md` for
    the full per-site ledger); every other bucket byte-identical
    (`CompoundReceiver`=1, `BuiltinPrecedenceCollision`=1,
    `MemberNotFound`=7). The residual 4 `UntrackedReceiver` sites are ALL
    confirmed out-of-this-task's-scope: 2 `Enum::"Type".Ordinals()` +
    1 bare enum-type-name `"Type".FromInteger(...)` (categories F/G,
    deferred to Task 4), + 1 Page-SourceTable implicit-field shorthand
    (Table/TableExtension-only by design, a separately-tracked gap, not a
    regression). `ambiguousResolved` 13→11 (2 flips, BOTH in `Page 6175389
    "CDO Local Print Service Part"`'s 3-overload `GetJsonAttribute` family —
    `GetErrorMessageFromResponse`/`GetStatusCodeFromResponse`'s own
    named-return bindings now type the `var`-parameter argument, eliminating
    the sibling-typed overload; both adjudicated compiler-correct).
    `genuine_wrong`=0, L3 semantic audit unchanged (all newly-resolved
    sites' frozen L3 golden was already empty for them). Ratchets
    re-derived (rate ceiling 0.001492→0.000719, unknown-count ceilings
    27→13, ambiguousResolved 13→11), dated 2026-07-04.
- **Zero-arg framework members resolve parens-less — parens are OPTIONAL in
  AL: real-`unknown` 0.22%→0.15%, `CompoundReceiver` 14→1 (receiver-closure-
  and-arg-increments plan, Task 2).** Closes the 9-site (A) population
  (`Response.Content.ReadAs(X)`-style — the zero-arg framework getter written
  WITHOUT parens, idiomatic AL) plus the 4-site (B) population
  (`ErrorInfo.CustomDimensions.{ContainsKey,Get}`):
  - **Root cause (the (A) 9):** a parens-less zero-arg call parses as
    `ExprKind::Member` (`is_method: false`) — structurally IDENTICAL to a
    property/field read — but the framework/RecordRef-family/enum-chain
    return-type tables were keyed strictly on `(.., is_method, arity)`, so a
    bare `Member` hop missed every `is_method: true, arity: 0` row and the
    chain declined. The module doc (`framework_returns.rs`) claimed "AL
    procedures ALWAYS require parens" — FALSE (the standing
    parens-are-optional correction, third recurrence; `Response.Content;`
    compiles, Code Cop AA0008 is style-only) — and the negative tests
    `framework_chain_wrong_form_property_instead_of_method_declines` /
    `ws_compound_framework_wrong_form_property_instead_of_method_stays_unknown` /
    `ws_chain_tables_xml_wrong_form_property_instead_of_method_stays_unknown`
    enforced the wrong behavior.
  - **Fix — context-sensitive lookup at the inference boundary, keeping the
    `is_method` schema** (round-1 addendum C3 design, BINDING): a new
    `zero_arg_aware_lookup` wrapper in `receiver.rs` wraps all three table
    probes in `infer_compound_member_receiver`. A genuine zero-arg `Call`
    (`X.Content()`) looks up the method row directly, unchanged. A bare READ
    `Member` (`X.Content`) tries the exact property row (`is_method: false`)
    FIRST, then the `is_method: true, arity: 0` method row as the
    parens-less-call fallback. Both-rows-exist-with-conflicting-return-kinds
    → `None` (fail-closed decline, never a guess) — currently unreachable
    (the pre-flip audit confirmed ZERO `is_method: false` rows exist in any
    of the three tables, so the fallback is unambiguous today), branch-proven
    by direct unit tests for when a future property row lands.
  - **Assignment-LHS can never become a call edge** (round-1 addendum,
    verified + pinned): the normalization lives entirely inside receiver
    TYPING of already-extracted call obligations; `collect_calls_v2`'s
    `Member` arm never emits a call site for a bare `Member`, so
    `X.Content := Y` is structurally invisible to extraction. Two new
    extraction tests pin this invariant.
  - **ErrorInfo rows (the (B) 4):** `(ErrorInfo, "customdimensions", true, 0)
    → Dictionary` added to `framework_return_kind` with MS Learn provenance
    (methods-auto/errorinfo, fetched 2026-07-04: `CustomDimensions([Dictionary
    of [Text, Text]])`, runtime 3.0) — the arity-1 SETTER form deliberately
    not tabled (no chainable return, same shape as `HttpRequestMessage.
    Content`). **Representability: YES** — `FrameworkKind::Dictionary`
    already exists and `member_catalog`'s DICTIONARY set already lists
    `containskey`/`get`; the Dictionary's generic VALUE type is untracked but
    irrelevant here because the leaf `ContainsKey`/`Get` calls ARE the edges
    (builtin Catalog) — no chaining past `Get`'s result is needed at any real
    site; if it were, that deeper hop would decline fail-closed.
  - **Documented rebaselines (correctness over compatibility):** the three
    wrong-form negative tests flipped + renamed (`..parens_less_property_form
    _resolves..`), each citing the parens memory; the false module doc
    corrected.
  - **CDO gate:** primary real-`unknown` 40→27 (0.22%→0.15%
    [27/18104=0.0014914]), `CompoundReceiver` 14→1 (the residual 1 is the (D)
    enum-chain site deferred to Task 4), every other bucket byte-identical
    (`UntrackedReceiver`=18, `MemberNotFound`=7,
    `BuiltinPrecedenceCollision`=1). All 13 sites individually adjudicated
    against real CDO source (9 in `Codeunit 6175322 "CDO Http Management"`,
    4 in Codeunits 6175309/6175376). L3 semantic audit: matches 6158→6145 /
    fresh_extra 5069→5082 (the frozen L3 golden held these 13 leaf sites
    unresolved too — fresh is now AHEAD of the retired reference;
    `fresh_extra_verified` per-site), `fresh_missing` stays 1, `fresh_wrong`
    stays 149/149 adjudicated, `genuine_wrong`=0, `ambiguousResolved`=13
    unchanged. Ratchets re-derived (rate ceiling 0.002210→0.001492,
    unknown-count ceilings 40→27), dated 2026-07-04.
- **`CurrPage.<usercontrol>` ControlAddIn receivers — closed-if-known gating:
  real-`unknown` 0.43%→0.22%, `CompoundReceiver` 51→14 (receiver-closure-and-
  arg-increments plan, Task 1).** Closes the 37-site mechanical population
  the prior arc's grounding report identified: `CurrPage.<usercontrol>.
  Method(...)` (30 sites, on source-declared `CDO.Editor`/`CDO.PrintService`)
  and `CurrPage.<usercontrol>.SetContent(...)` (7 sites, on platform
  `WebPageViewer`) previously fell through the `CurrPage.<part>.Page`
  subpage-instance Step 0 (which requires `PageControlKind::Part`) to a
  generic 2-hop decline — `CompoundReceiver`. A new Step 0b matches the bare
  `CurrPage.<usercontrol>` shape and dispatches through a **tri-state,
  closed-if-known `ControlAddIn` gate** (round-2-closer BINDING design):
  - **Resolved** (the addin type resolves to exactly one cleanly-parsed
    object) → the called member must match ONE of the addin's declared
    procedures (name + **arity**; events structurally excluded — see below)
    UNIONED with the platform base-member surface. That union is
    **researched and found EMPTY**: MS Learn's `Visible` property page
    confirms control properties like `Visible`/`Editable`/`Enabled` are
    page-LAYOUT DESIGN-TIME properties, never `CurrPage.<control>.<member>`
    RUNTIME member calls, and no generic AL-callable base method is
    documented for any control add-in beyond its own declared procedures —
    an executable test (`resolve_member_controladdin_declared_no_platform_
    base_members_silently_resolve`) proves the empty union is enforced, not
    assumed. A name/arity miss is an honest `Unknown(MemberNotFound)`, never
    a guessed `Catalog`.
  - **TruePlatform** (zero reachable declaration, but the name is on a small,
    MS-Learn-grounded allowlist — currently just `webpageviewer`, matching
    the real CDO corpus's bare, unqualified reference to
    `"Microsoft.Dynamics.Nav.Client.WebPageViewer"`) → open-accept `Catalog`
    unconditionally, same `BuiltinId` text as before
    (`"ControlAddIn::<method>"` — the real CDO golden
    `cdo-deanon-map.json` already carries this shape).
  - **Ambiguous** (≥2 reachable declarations) / **Degraded**
    (`parse_incomplete`) / genuinely-unresolved-and-non-platform → `Unknown`,
    never open-accepted.
  - The pre-existing **direct-var `var X: ControlAddIn "Foo"` open-accept
    path is retrofitted to the SAME gate** (it was itself a latent
    false-`Catalog` vector — `FrameworkKind::ControlAddIn`'s blanket
    "every method is builtin" policy is REMOVED; `ControlAddIn` moved to a
    dedicated `ReceiverType::ControlAddIn { name_lc, surface }` variant so
    the specific addin's identity survives into Phase B). Zero CDO impact
    (no direct-var `ControlAddIn` declaration exists anywhere in the real
    corpus) — a pure soundness fix, unit-tested only.
  - `SystemPart` controls are explicitly OUT of the arm (native platform
    components, not JS add-ins — default-decline, dated note; a closed
    SystemPart catalog is future work if real sites appear).
  - **Root-cause al-syntax fix required first**: `al_syntax::lower::
    collect_routines` never lowered `interface_procedure` nodes (the
    grammar rule BOTH `controladdin_body` and `interface_body` use for
    signature-only procedure declarations — no body, sometimes no trailing
    `;`) — controladdin/interface procedures were completely invisible to
    `RoutineDecl`/`RoutineNode` extraction, with zero name/arity to gate on.
    Fixed at the source: `interface_procedure` now lowers identically to
    `procedure` (name + arity via the same inlined `_procedure_name_and_
    params`; a bonus fidelity fix recovers the return-type spec from the
    nested `interface_procedure_suffix` child too). `RoutineNodeId.
    params_count` (already a first-class field) gives arity gating for
    free. Since `event_declaration` is a structurally DISTINCT grammar node
    `collect_routines` still never matches, events are excluded from the
    declared-procedure surface by construction, not by a filter. This is a
    SHARED, correctness-improving fix (al-syntax is a foundation crate) —
    it also, as a genuine bonus, makes `Interface` object procedure
    signatures visible to the LSP call-hierarchy front-end
    (`parser::parse_file_ir`, Rust-owned golden rebaselined) for the first
    time. The frozen, differential-gated LEGACY `engine::l2`/`engine::l3`/
    `engine::l4` port-parity pipeline (a SEPARATE consumer of the same
    shared IR, still byte-compared against committed al-sem-derived
    goldens/vectors) explicitly excludes `Interface`/`ControlAddIn` object
    routines at its own ingestion points (`l3_workspace.rs`,
    `l2_workspace.rs`, `engine::snapshot.rs`) to preserve that historical
    parity contract untouched — zero legacy golden/vector edits needed.
  - New `ObjectNode::parse_incomplete` field (file-level
    `ParseStatus::Recovered`, additive) backs the Degraded tri-state arm.
  - **Measured on the real CDO workspace**: `CompoundReceiver` 51→14 (-37,
    every one of the 37 sites individually source-adjudicated: all 30
    `CDO.Editor`/`CDO.PrintService` calls match a real declared procedure at
    the real declared arity — zero typos, zero arity mismatches, zero events
    called — and all 7 `WebPageViewer` calls hit the open `TruePlatform`
    path), primary `unknown` 77→40 (`real_unknown_rate` 0.43%→0.22%),
    `unknownByReason`={CompoundReceiver: 14, UntrackedReceiver: 18,
    BuiltinPrecedenceCollision: 1, MemberNotFound: 7} — every other bucket
    byte-identical. `ambiguousResolved` stays 13 (untouched). `genuine_wrong`
    stays 0 (`cdo_l3_semantic_audit_no_fresh_wrong`); all 37 sites classify
    `matches_l3` (L3's own legacy `PageControlKind::UserControl` inference
    already open-accepted these as `ControlAddIn`/builtin, so fresh's
    NEWLY-gated-but-still-Catalog result agrees) — no L3 disagreement, no
    ratchet regression. Ratchets re-derived and dated in
    `tests/program_resolve_harness.rs` (`primary_rate` ceiling
    0.004254→0.002210, `ph.unknown`/`h.unknown` ceilings 77→40).
- **Arc capstone — catalog completion + arg-type dispatch + preproc
  foundations: real-`unknown` 0.52%→0.43%, `ambiguousResolved` 56→13 (Task 4,
  FINAL, argtype-dispatch-and-page-catalog plan).** Full CDO re-measure
  confirms the 3-task arc at its floors: primary `unknown`=77/18104
  (`real_unknown_rate`=0.43%; whole-program `unknown`=77/43404=0.18%),
  `unknownByReason`={CompoundReceiver: 51, UntrackedReceiver: 18,
  BuiltinPrecedenceCollision: 1, MemberNotFound: 7}, `ambiguousResolved`=13
  (primary/whole), `genuine_wrong`=0, `route_applicability`/
  `fan_out_applicability` violations=0 with non-vacuous `routes_checked`
  (interface=28, instance_builtin=481, implicit_trigger=1183, event=3404),
  `recoveredFiles`=8 (pinned), the sig_fp collision-guard group count 0/0 —
  all byte-identical across two full single-threaded 174-test CDO runs
  (before and after this task's own nit fixes below). The
  **legacy-comparable rate** (the pre-sigfp-reclassification-plan metric
  definition, `unknown + ambiguousResolved` treated as one undecided
  population) moves 0.83%→0.50% ((77+13)/18104) — the dispatch increment's
  real, if modest, contribution once folded back into the older
  denominator.
  - **The falsified-`ProvenAbsent` premise, told honestly.** This plan
    originally set out to build `ProvenAbsent` machinery for the 13
    workspace-tier `MemberNotFound` sites inherited from the prior arc's
    grounding report. Task 1's own preflight investigation falsified that
    premise BEFORE any such machinery was built: all 13 (+5 embedded
    siblings) were never absences — they were ONE deliberate engine catalog
    gap (`is_metadata_sensitive_instance_method` excluding real, always-present
    Page/Report instance intrinsics). Building a "proven absent" proof for a
    member that actually exists would have codified a false claim. The
    lesson, worth generalizing: **measure the population before building
    taxonomy for it.** The real fix was catalog completion (18 sites →
    `Resolved`/`Catalog`, `unknown` 95→77), an ordinary correctness fix, not a
    new ObligationOutcome. Only the 7 CDOeCandidates sites are genuine,
    independently-verified absences (the target members provably don't exist
    in the installed dependency at any visibility) — they remain honest
    `Unknown(MemberNotFound)` and are now the documented, and only, real
    `ProvenAbsent` prototype population for a future plan.
  - **The dispatch increment.** Task 2's fail-closed arg-type overload
    dispatch (source tier: literals + declared vars, exact semantic-identity
    matching only — Text/Code length brackets non-discriminating,
    object-bearing types via the existing fail-closed `resolve_object_ref`,
    `Variant` candidates always degrade, `var`-mode requires exact
    by-ref-compatible typing, literal typing is candidate-set-aware per
    fixture-proven family) picked 44 of the 56 CDO `ambiguousResolved` sites
    to `Resolved` (56→12). A same-task review fix then closed a dormant
    wrong-pick vector (a `with`-scope gate mirroring `resolve_bare`'s
    existing Step 3 guard, since arg typing had no visibility into
    `with`-block identifier rebinding) and, in doing so, honestly reverted
    ONE of the 44 picks (`UseContiniaAuthorization`, a `WithState::Unknown`
    routine whose comment-vs-AST with-detection signals disagree) back to
    `AmbiguousResolved` — 12→13, **43 net picks**, every one fail-closed-
    guarded and individually adjudicated against the real CDO source (see
    `.superpowers/sdd/task-2-report.md`). `genuine_wrong` stayed 0 throughout.
  - **The preprocessor foundations** (Task 3): the `#if` union-read verified
    TRUE for objects/routines/globals and PINNED; two flat-loop gaps found
    and dispositioned (a real, previously-silent property-drop bug fixed via
    a new `collect_properties` descend helper; a defensive `implements`
    descend where grounding proved no live gap exists); a program-layer
    `singular_property_value` conflict degrade (fail-closed `None` on a
    genuine cross-`#if`-branch disagreement, never first/last-wins); a new
    `ParseStatus::Recovered` diagnostic that immediately proved its worth by
    surfacing 2 real, previously-invisible `tree-sitter-al` grammar defects
    confined to dependency (embedded) source — an `OptionMembers =
    TableData,...` first-position keyword collision (Microsoft `System`'s
    `Object`/`NAVAppObjectPrerequisites`/`DatabaseLocks` tables) and a
    `# pragma` (space after `#`) not recognized (Continia System
    Application's `Http.Codeunit.al`) — both out of this plan's scope, filed
    for a future dedicated grammar task. All CDO-inert (zero live conditional
    `SourceTable`/`TableNo`/`implements` on this corpus) except the new
    diagnostic itself.
  - **This task's own review-nit fixes** (cheap, mechanical, no resolution
    behavior change on CDO): (1) `node_extract::singular_property_value`'s
    conflict check now compares `ObjectRef` on **semantic identity**
    (`normalized_lc` for a name reference, the numeric id for an id
    reference) via a new `object_ref_pair_conflicts` helper, instead of the
    derived `PartialEq` on `(ObjectRef, bool)` — the old comparison also
    matched `ObjectRef::Name`'s display-only `raw` text, so two `#if`
    branches naming the SAME table with different casing (`Customer` vs
    `CUSTOMER` — AL object-name references are case-insensitive) would have
    been misclassified as a conflict and spuriously degraded to `None`. 2 new
    unit tests: `preproc_same_table_different_case_branches_are_not_a_conflict`
    (must resolve) and `preproc_differing_temporary_marker_is_still_a_conflict`
    (a differing `temporary` marker is still a real conflict, control). (2) A
    CHANGELOG test-count correction for the Task 3 entry: `lower/mod.rs`
    contributed 6 new tests, not 7 (git-diff-verified against commit
    `dbf2c56`); the itemized total corrected 21→16. (3) The **new clippy
    bar**: `cargo clippy --release --all-features --all-targets -- -D
    warnings` is now CLEAN (previously only the narrower `--all-features`
    gate, without `--tests`, was enforced) — fixed 4 pre-existing findings:
    a dead `confidence_complete` test helper in `format_pr_summary.rs`
    (zero call sites, removed); a `useless_vec` in `edge.rs`'s
    `edge_constructs_and_is_orderable` test (`vec![e.clone(), e]` → an array
    literal); a `manual_checked_ops` division in
    `program_resolve_harness.rs`'s L3-audit percentage print (now
    `.saturating_mul(100).checked_div(audit.paired).unwrap_or(0)`); a
    `ptr_arg` on `cli_a_json_differential.rs`'s `run_json_path(ws: &PathBuf,
    ..)` (→ `&Path`, newly surfaced by the wider `--all-targets` scope, not
    in the plan's original 3-finding list). `cargo fmt --check` and
    `cargo test --workspace` (159 green `test result: ok` blocks, including
    the 2 new node_extract tests) both stayed green throughout.
  - **Deferred roadmap** (unclaimed, in rough priority order):
    `ProvenAbsent` machinery prototyped against the 7 CDOeCandidates sites
    (the now-confirmed real absence population); a comment-aware
    `with`-token scan (would restore the `UseContiniaAuthorization` pick and
    close the same dormant gap in `resolve_bare`'s pre-existing Step 3
    implicit-Rec decline); the 2 newly-found `tree-sitter-al` grammar defects
    (a `grammar.js` fix + a full BC.History 15k-file revalidation pass — a
    genuinely separate undertaking); the deferred arg-typing increments
    (`Enum::Value`, call-result, `Rec.Field` argument typing — would
    disambiguate more of the residual 13 `ambiguousResolved`); ABI
    param-type retention (unlocks `SymbolOnly`-tier arg dispatch, currently
    tier-gated out entirely); implicit-conversion modeling
    (compiler-backed); the full `ParseStatus::Clean` per-file gate (today
    only a surfaced diagnostic); and `CompoundReceiver`=51 — now the single
    largest `unknown` bucket and the natural next lever.
  - See `.superpowers/sdd/task-4-report-close.md` for the full re-measure
    writeup.

- **Preprocessor foundations: `#if`-wrapped object properties + a defensive
  `implements` descend; program-layer conflict degradation; a
  `ParseStatus::Recovered` diagnostic (Task 3, argtype-dispatch-and-page-
  catalog plan).** `al_syntax::lower`'s `#if` union-read was verified TRUE
  for objects/routines/globals but had two flat-loop gaps:
  - **Properties (real, verified gap).** `lower_object`'s property collection
    was a flat loop over `body.named_children()` checking `member.kind() ==
    RawKind::Property` directly — a `#if`-wrapped property (e.g.
    `SourceTable`) is a child of a `preproc_conditional` wrapper, never a
    direct `Property` node, so it was silently DROPPED ENTIRELY (verified
    failing-first: zero properties captured, not even a first-wins pick).
    Fixed with a new `collect_properties` helper mirroring `collect_globals`'s
    established descend pattern.
  - **`implements` (defensive fix, no live gap found).** Ground-truthed via
    `tree-sitter parse` dumps of `tree-sitter-al/grammar.js`: the only
    grammar-reachable `#if`-conditional `implements` shape
    (`preproc_split_declaration`, a whole-object header split) is already
    flattened by the grammar itself — no wrapper node exists between the
    object and either branch's `implements_clause`, so the original flat
    loop already found both unaided. Refactored into a recursive
    `extract_implements_walk` that also descends `is_preproc_wrapper` anyway
    (the same pattern used everywhere else) — documented honestly as
    defensive/future-proofing, not a bug fix.
  - **Program-layer conflict degradation (`node_extract.rs`).** After the
    properties fix, `#if A SourceTable=X #else SourceTable=Y #endif` now
    surfaces BOTH values — the old `.iter().find(...)` read would have
    silently first-wins-picked `X` (verified failing-first). New
    `singular_property_value` collects every matching property and returns
    the value iff ALL occurrences agree, else fail-closed `None` — applied to
    both `SourceTable` and `TableNo`. `ObjectDecl.implements` (a list-valued,
    additive-fan-out-only property) is INTENTIONALLY never degraded — every
    consumer (`interface_route_applicable`, `ResolveIndex`'s implementer
    index) only ever asks "might this object implement `iface`?", so a wider
    union is sound; a singular property feeds a SINGLE implicit-Rec
    decision, so silently picking a conflicting branch would fabricate a
    false single-target confidence.
  - **`ParseStatus::Recovered` diagnostic.** New `snapshot::parse::
    recovered_file_paths` (count + file paths, additive, non-gating) wired
    into `ProgramReport.recovered_files` and aldump's
    `--program-call-graph-stats` JSON (`recoveredFiles: {count, paths}`).
    Doc-pinned invariant: any future absence/`ProvenAbsent`-shaped claim must
    consult this before treating a file's content as complete; the full
    per-file resolution gate is deferred (no absence claim exists yet to
    gate). **Immediately proved its worth**: the new CDO-gated assertion
    surfaced TWO real, previously-invisible `tree-sitter-al` grammar defects
    — (1) `OptionMembers = TableData,...` (the bare identifier `TableData`
    as the FIRST option member collides with the `tabledata` keyword that
    also starts `tabledata_permission_list`, a sibling `_property_value`
    alternative — reproduced minimally, confirmed first-position-only), on
    Microsoft `System`'s `Object`/`NAVAppObjectPrerequisites`/
    `DatabaseLocks` tables; (2) `# pragma warning disable LC0088` (a space
    between `#` and `pragma`) is not recognized, only `#pragma` (no space)
    is, on Continia System Application's `Http.Codeunit.al`. Both are
    confined to DEPENDENCY (embedded) source, never CDO's own primary
    workspace code, and are filed as a dated note for a future dedicated
    `tree-sitter-al` grammar task — fixing them is out of this plan's scope.
    Pinned exact in `tests/program_resolve_harness.rs`
    (`cdo_full_program_coverage_and_self_reported_metric`): 8 entries (4
    distinct files, each doubled by a pre-existing, unrelated
    app-duplication artifact in `parse_snapshot`'s per-`AppUnit` parsing).
  - **CDO harness** (`CDO_WS`, `ENFORCE_CDO_WS=1`, release,
    `--test-threads=1`, full 174-test suite, twice): BYTE-IDENTICAL
    resolution — `unknown`=77 (primary/whole), `real_unknown_rate`=0.43%,
    `unknownByReason`={CompoundReceiver: 51, UntrackedReceiver: 18,
    BuiltinPrecedenceCollision: 1, MemberNotFound: 7}, `genuine_wrong`=0 —
    CDO's dependency closure contains zero live `#if`-conditional
    `SourceTable`/`TableNo`/`implements` declarations, so the fixes are
    correctness-complete but inert on this corpus; the only CDO-visible
    change is the new `recoveredFiles` diagnostic itself (0→8, both grammar
    defects above, confirmed as recorded, not silently masked). Full
    workspace suite (`cargo test --workspace`, no CDO_WS): 159 green
    `test result: ok` blocks, zero failures, zero golden movement anywhere.
  - 16 new unit/integration tests across `crates/al-syntax/src/lower/mod.rs`
    (6, not 7 — corrected count, Task 4 review nit), `src/program/
    node_extract.rs` (3), `src/program/build.rs` (2),
    `src/program/resolve/applicability.rs` (1), `src/snapshot/parse.rs` (2),
    `src/program/resolve/full.rs` (2), plus the CDO-gated ratchet in
    `tests/program_resolve_harness.rs`. See `.superpowers/sdd/task-3-report-
    preproc.md` for the full writeup.

- **`with`-scope gate for bare-identifier arg typing — closes a dormant
  wrong-pick vector in fail-closed arg-type dispatch (Task 2 review fix,
  argtype-dispatch-and-page-catalog plan v2.1).** `arg_dispatch::
  type_call_args`/`type_one_arg` never consulted the call site's
  `WithState`, even though `resolve_call_site_obligation` (`full.rs`)
  already threads it to callee resolution. Inside a `with X do` block, AL
  rebinds a bare identifier to the WITH-receiver's member — the arg-typing
  module's caller-scope-EXACT lookup (params → locals → globals)
  structurally cannot see that rebinding, so a bare-identifier argument
  could be typed against the WRONG (caller-scope) declaration and
  fail-closed-PICK the wrong overload (e.g. `with Rec do
  Target.Foo(SomeField)` where a table field's Decimal shadows a same-named
  global Text across a `(Decimal)`/`(Text)` overload pair). Dormant on CDO
  (zero `with` blocks in the corpus — no flip tainted). Fixed by mirroring
  `resolve_bare`'s existing Step 3 with-guard exactly: a bare-identifier arg
  now yields `ArgDispatchInfo::untyped()` (degrading the whole call — no
  pick, stays `AmbiguousResolved`) unless `with_state ==
  WithState::NoWithProven`; a literal argument is unaffected (it cannot be
  rebound by `with`). New fixture `tests/r0-corpus/ws-overload-with-scope/`
  (`CallInsideWith` proves the degrade; `CallOutsideWith` is the control
  proving the same call still confidently picks outside any `with`) plus 3
  new unit tests in `arg_dispatch.rs`.
  - **Also (Finding 3, same review):** `candidate_param_infos` now degrades
    to `None` (missing candidate metadata) when the candidate declaration is
    `parse_incomplete` — a param TYPE is the first place candidate metadata
    adjudicates a pick, so a parser-recovery artifact there is never trusted,
    consistent with every other `parse_incomplete` consumer in the codebase
    (`engine::l5`'s detectors, `l3_workspace` coverage, etc.). 2 new unit
    tests (`candidate_param_infos_degrades_on_parse_incomplete` +
    parse-complete control).
  - **CDO harness re-run** (`CDO_WS`, `ENFORCE_CDO_WS=1`, release,
    `--test-threads=1`): `unknown`/`real_unknown_rate` stay byte-identical at
    77/0.43% (a disjoint histogram bucket) and `genuine_wrong=0` holds, but
    `ambiguousResolved` moves **12 → 13** — INVESTIGATED (a rise is this
    ratchet's own documented "verify before updating" trigger), root-caused
    to exactly ONE site via a before/after diff of
    `task2_dump_argtype_dispatch_flips_on_cdo` (`git stash`-isolated against
    the identical snapshot): `UseContiniaAuthorization`
    (`Codeunit 6175322 "CDO Http Management"`) reverts to
    `AmbiguousResolved`. Its own routine body has no real `with` block (AST
    depth 0), but a LEADING COMMENT contains the standalone word "with" —
    `extract::routine_has_with_token`'s raw-text scan is deliberately
    comment-blind by design, so the two with-detection signals disagree and
    resolve to `WithState::Unknown` for the whole routine. This is not a new
    gap: `resolve_bare`'s pre-existing Step 3 has ALWAYS skipped on this same
    `Unknown` signal for this routine; the review fix's with-scope gate
    faithfully mirrors that same established precedent into arg typing, as
    the finding required, rather than adding an inconsistent narrower gate.
    Ratchet re-pinned 12→13 in `tests/program_resolve_harness.rs` with the
    full root-cause writeup inline.
  - See also `.superpowers/sdd/argtype-dispatch-task-2-report.md` §8 and the
    `### Added` section below (Task 2, "Fail-closed argument-type overload
    dispatch") for a report-accuracy correction to that entry's flip-category
    breakdown (Finding 2, same review).

- **Page/Report instance-catalog completion — `SetTableView`/`SetRecord`/
  `GetRecord`/`SetSelectionFilter` exist unconditionally (Task 1,
  argtype-dispatch-and-page-catalog plan).** `resolver::
  is_metadata_sensitive_instance_method` previously excluded ALL of
  `Page.{SetTableView,SetRecord,GetRecord,SetSelectionFilter,SaveRecord}` and
  `Report.SetTableView` from the instance-builtin catalog fallback, reasoning
  that "argument/return types depend on the object's source table, so we
  can't validate the call" — a conflation of argument-type validation with
  member EXISTENCE (the resolver has never validated any catalog method's
  arguments). These are real, unconditional platform intrinsics present on
  every Page/Report object regardless of source table (MS Learn-documented;
  L3's own `PAGE_INSTANCE`/`REPORT_INSTANCE` catalogs already listed them).
  The exclusion now narrows to `SaveRecord` only, which genuinely IS
  CurrPage-only (a compiler error on a declared Page-typed variable) — kept
  excluded from the general `Object{Page}` catalog fallback; `CurrPage.
  SaveRecord()` was already unconditionally resolved via the separate
  `Framework(PageInstance)` receiver arm (the CurrPage-origin distinction is
  structural — a different `ReceiverType` variant entirely — never inferred
  from a resolved page id/type, per round-2 review). Primary real-`unknown`
  moves 0.52%→0.43% (`unknown` 95→77, `MemberNotFound` 25→7): 18 CDO sites
  (10 `Page.SetTableView`, 4 `Report.SetTableView`, 1 `Page.SetRecord`, 3
  `Page.GetRecord` including the `CurrPage.<part>.Page` subpage-instance
  chain) move from `Unknown(MemberNotFound)` to `Resolved`/`Catalog`; the
  remaining 7 `MemberNotFound` sites (the CDOeCandidatesEventHandler
  `OnlyVendorsAreHandled`/`OnlyCustomersAreHandled`/`GetOutputProfile` calls)
  stay honest `Unknown` — independently verified ABSENT from the installed
  dependency's page source at any visibility, the future `ProvenAbsent`
  prototype population.
  - **Adjudication:** all 18 flips' targets were cross-checked against the
    frozen L3-validated semantic golden. 16/18 land cleanly in `matches`. 2
    sites (`FieldsWithRelationPage.SetTableView(Field)` on Page 6175276 and
    Page 6175425) surfaced a PRE-EXISTING L3 golden site-collision defect —
    both pages declare TWO field controls with their own same-named `trigger
    OnAssistEdit()`, and L3's frozen `(unit, line, callee_fp)` golden key
    carries no per-control qualifier, so it mis-pairs the site with the
    OTHER same-named trigger's unrelated call. Fresh's `RoutineNodeId`
    (which carries `enclosing_member_lc`) disambiguates correctly; this is
    the same `builtin-catalog-fp-collision` class already documented for 43
    prior sites, now with a duplicate-trigger-name variant. Added 2 entries
    to `known-genuine-divergences.json`/`adjudicated-overrides.json` (42→54
    total; a new `receiver_kind: "PageInstanceVar"` shape, extending
    `tests/program_resolve_harness.rs`'s independent source-adjudication
    harness to verify a declared-Page-variable receiver via a `<name>: Page
    ...` source-text scan, as opposed to the fixed `CurrPage`/`Page`
    singleton token the pre-existing `"PageInstance"` shape checks).
    `genuine_wrong=0` held throughout.

### Added
- **Fail-closed argument-type overload dispatch (Task 2, argtype-dispatch-
  and-page-catalog plan v2.1).** `resolve_in_object`'s `_` arm (the
  prevalidated, same-name/same-arity `AmbiguousOverload` candidate set) now
  attempts one additional fail-closed step before falling back to
  `AmbiguousResolved`: type the call's arguments (SOURCE tier only — literals
  of a fixture-proven family, and bare identifiers resolved via the SAME
  caller-scope-EXACT `params → locals → globals` lookup `receiver.rs`'s Step
  2 uses) and, iff EXACTLY ONE candidate's full parameter list is
  dispatch-compatible and every OTHER candidate is PROVEN incompatible at
  some position, pick it. New module `program::resolve::arg_dispatch`
  (`ArgDispatchInfo`/`ParamDispatchInfo`/`CanonicalArgType`, 17 unit tests):
  - **Dispatch-canonical identity, not text identity:** object-bearing types
    (Record/Page/Report/Codeunit/Query/XmlPort/Interface/Enum) canonicalize
    via the EXISTING fail-closed `ResolveIndex::resolve_object_ref` semantic
    identity; Text/Code length brackets are stripped (non-discriminating for
    by-value compatibility); scalar families compare by exact base keyword
    (`integer` != `decimal` != `biginteger`, `text` != `code`) — an
    UNRESOLVABLE object-bearing type leaves that position untyped rather
    than guessing.
  - **`var` parameters are ByRef-EXACT** (length INCLUDED — `var Text[30]`
    never matches `var Text[50]`); a literal/call-result argument is never
    var-passable (a `var` parameter is a sound elimination against it, not a
    degrade).
  - **A `Variant`/`Any` candidate at a discriminating position degrades the
    whole call**, computed from the FULL candidate set before any
    compatibility filtering — a naive "exclusion" matcher would otherwise
    leave Variant as an unproven "sole survivor."
  - **Call-level degradation:** ANY untyped argument position (every
    expression shape beyond a bare identifier/literal is deferred — call-
    result, `Rec.Field`, `Enum::Value` — to a documented future increment)
    or ANY candidate with unresolvable/missing parameter metadata degrades
    the WHOLE call; an unknown-metadata candidate is never filtered OUT of
    the competition to let the rest resolve.
  - **A same-"soft-family" mismatch (Text/Code/Char/Label, or
    Integer/Decimal/BigInteger) is UNDECIDED, not eliminated** — AL's own
    conversions between these mean a mismatch there cannot be proven
    incompatible; an undecided candidate blocks a confident pick exactly
    like a second exact match would. The plan's C6 literal-typing rule
    (a STRING literal degrades whenever a Code/Char candidate is present; an
    INTEGER literal degrades whenever a Decimal/BigInteger candidate is
    present — except the compiler-proven Integer-literal-vs-`Code[N]` pair,
    where ordinary exact-mismatch elimination already applies) is
    additionally encoded verbatim for direct traceability.
  - **Tier-gated to SOURCE** (`obj_tier != SymbolOnly`): a SymbolOnly
    candidate carries no `BodyMap` entry, so it can never supply parameter
    metadata — the gate is explicit (clean skip), not incidental.
  - Plumbing: `RawSiteV2.args: Vec<ExprId>` (`extract.rs`); `arg_dispatch::
    type_call_args` built ONCE per call-site obligation in
    `resolve_call_site_obligation` (`full.rs`) and threaded through new
    `resolve_bare_with_args`/`resolve_member_with_args`/`resolve_in_
    table_scope` variants (`resolver.rs`) — the pre-existing `resolve_bare`/
    `resolve_member` stay thin `args = &[]` wrappers, so none of this
    module's ~90 existing unit tests needed touching. `sig_fp::
    normalize_type_text` is now `pub(crate)`.
  - Wired the pre-authored ORPHANED fixture banks `ws-overload-arg-type`/
    `-arg-pos2`/`-negatives` (commit `b4ff081`) plus the deferred-increment
    guard banks `-enum-discriminator`/`-field-discriminator`/
    `-callexpr-discriminator` (assert NOT-yet-flipped — those argument
    shapes stay untyped in this increment). Rebaselined `ws-overload-
    collision`'s `Resolve(5)` call: the Integer literal now confidently
    picks the `Resolve(X: Integer)` overload (an Integer literal structurally
    cannot bind `Code[20]` — the compiler-proven exemplar named in the plan's
    C6 addendum); added a new `CallAmbiguousUntyped` control (a call-result
    argument) proving the pick does not over-fire when there is genuinely no
    evidence.
  - **CDO measurement** (`CDO_WS`, `ENFORCE_CDO_WS=1`, release,
    `--test-threads=1`): `ambiguousResolved` 56→**12** (44 sites flip to
    `Resolved`) — a MAJORITY, not the "minority" the plan anticipated;
    `unknown`/`real_unknown_rate` byte-identical at 77/0.43% (a DIFFERENT
    histogram bucket; arg-type dispatch never touches `Unknown`).
    **Adjudication** (`.superpowers/sdd/argtype-dispatch-task-2-report.md`
    has the full table + a review-fix correction section (§8);
    `task2_dump_argtype_dispatch_flips_on_cdo`, a new `#[ignore]`d
    diagnostic, reproduces the raw dump): reviewer-reproduced breakdown of
    the 44 (corrects an earlier draft of this entry's "~34 object-record /
    ~7 cross-family / ~3 unreadable" estimate, which had the unreadable
    share off by an order of magnitude) — **25/44 (57%) have
    `picked_decl=[<unreadable>]`** (the diagnostic's naive path-join can't
    resolve dependency/base-app source paths; not independently re-verified
    this task); of the 19/44 with readable decl text, the majority are
    Object/Record EXACT-IDENTITY eliminations — CDO's real code very
    commonly overloads a procedure BY RECORD TYPE (`CheckAndSetHandled`,
    `PrintPDFFile`, `RunPrePostValidation`, the obsoleted
    `SendElectronicDocument` shim family that funnels a `Code[20]` into a
    local `Record "CDO Send Code"` before dispatching) — the SOUNDEST
    elimination category in the whole design (two different AL record types
    are never assignment-compatible without an explicit `RecordRef`/
    `Variant` detour); `GetJsonAttribute`'s 3-overload family is the ONLY
    hand-traced cross-family Base elimination (a `var returnValue: Text`
    argument eliminates both a `Text`-first-param sibling and a `var
    Integer`-typed sibling by two INDEPENDENT discriminating positions) — no
    other cross-family pick was independently traced. The pick
    PRECONDITIONS held identically for all 44 regardless of decl
    readability (the ABI/BodyMap carries full candidate parameter
    TYPE+MODE metadata even when the diagnostic can't render the winning
    decl's source text), and the frozen-L3 semantic audit gate
    (`cdo_l3_semantic_audit_no_fresh_wrong`, green throughout) corroborates
    the WHOLE population, not just the hand-traced subset. NONE of the 44
    touch the "undecided" soft-family gate (no Text-vs-Code or
    Integer-vs-Decimal pick fired on real CDO code). `genuine_wrong=0` and
    the L3 semantic audit both HARD gates, both re-run green on the
    identical snapshot. `ambiguous_resolved` ratchet re-pinned 56→12 (both
    scopes); `unknown`/`real_unknown_rate` ceilings unchanged.
  - **Out of scope (deferred, documented in the plan's roadmap):**
    Enum::Value / call-result / `Rec.Field` argument typing; ABI-tier
    (SymbolOnly) parameter-type retention; implicit-conversion modeling.

### Added
- **Source `sig_fp` identity + `AmbiguousResolved` reclassification — arc
  complete (Task 5 FINAL, sigfp-and-ambiguous-reclassification plan). Primary
  real-`unknown` moves 0.83%→0.52% (`unknown` 151→95, `realUnknownRate`
  0.008340698188245692→0.0052474591250552365) by a DOCUMENTED
  metric-definition change, not a resolution improvement: 56 genuine
  same-object overload-ambiguity call sites, which the engine already proved
  it could enumerate exhaustively and completely (a closed candidate set,
  not an open-world guess), move from `Unknown` to a new
  `ObligationOutcome::AmbiguousResolved` — "exactly one candidate fires at
  runtime, chosen by argument-type dispatch this engine does not perform;
  not a resolution gap." These edges remain PRACTICALLY unresolved at
  runtime from the tooling's perspective (nothing picks a winner) — the
  both-ways `Histogram::legacy_unknown_rate_including_ambiguous()` /
  `realUnknownRateLegacyIncludingAmbiguous` reads BYTE-IDENTICAL to the
  pre-change `realUnknownRate` at both scopes (0.008340698188245692 primary
  / 0.003478942032992351 whole), proving the move is a pure relabeling, not
  a stat-juke.
  - **The sequencing (T1→T4) that makes the relabeling honest.** A candidate
    set deduped on an ALIASED id would silently collapse a genuine 2-overload
    ambiguity into a false-appears-resolved single route — the exact footgun
    the pre-existing `index.rs:157-168` comment warned about. So identity had
    to be fixed BEFORE candidates could be safely carried: **Task 1** added a
    `source_overload_aliased` fail-closed marker (mirroring the pre-existing
    ABI `abi_overload_collapsed` pattern) plus a dual-publisher
    `emit_event_flow_edges` SKIP guard (never a synthetic zero-span) for the
    one case an aliased id's corrupted last-write-wins span could otherwise
    leak — measured `eventFlowDualPublisherAliasSkips=0` on CDO throughout
    (the guard never had to fire; 0 of the 6 primary / 313 whole-program
    aliased groups had ≥2 publisher siblings). **Task 2** then gave source
    overloads REAL identity: one shared `source_routine_node_id` constructor
    (`src/program/sig_fp.rs`) fingerprints every parameter's normalized type
    text + `var`/by-ref flag (fnv1a, length-delimited, reusing the
    `abi_ingest::param_type_fp` primitive) instead of the old universal
    `sig_fp: 0`. A Task 2 review fix then caught a 6th LIVE construction site
    the original 5-site audit had missed
    (`semantic_golden.rs::build_fan_out_site_context`, which independently
    re-walks call sites for `route_applicability`'s fan-out soundness teeth)
    — post-fix, all 5 live sites (one of the original 5 was dead code,
    deleted rather than migrated) are unified on the one constructor, closing
    a real applicability-gate regression the narrower fixture set couldn't
    catch (measured pre-fix: `interface_applicability_violations=24`,
    `implicit_trigger_violations=324`, both silently green). The
    `source_overload_aliased` marker's role flips post-Task-2 from "fires for
    every genuine overload pair" to a pure **residual-collision guard**:
    `source_overload_alias_collision_guard_group_count_pinned_on_cdo` pins
    the post-fix marked-GROUP count at **0 primary / 0 whole-program** on CDO
    (down from the pre-Task-2 baseline of 6/313 — every real overload now
    gets its own distinct id and never reaches the guard at all; a nonzero
    reading in future would mean a genuine `sig_fp` normalization collision,
    a threshold alert to investigate, never to silently mask). **Task 3**
    landed the `Condition::AmbiguousDispatch` /
    `DispatchShape::AmbiguousOverload` / `ObligationOutcome::AmbiguousResolved`
    taxonomy as INERT mechanics (CDO byte-identical before any producer used
    it), including the structural anti-laundering backstop that a
    mixed/degraded candidate set (any collapse-marked or `Evidence::Unknown`
    candidate) can never construct `AmbiguousOverload` at all — it stays the
    honest single `Unresolved(OverloadAmbiguous)` route. **Task 4** wired it:
    `resolve_in_object`'s genuine `>1`-candidate arm now returns one
    concrete `Route` per candidate (each `Condition::AmbiguousDispatch`,
    `fires_by_default()==false`, excluded from `default_reachable_routes()`
    but included in the new `may_reachable_routes()` may-traversal set) —
    see the Task 4 entry below for the full wiring, fixture, and
    per-emission-site partition detail.
  - **The `.dependencies` audit (Task 0, preflight — user-requested roadmap
    item, now CLOSED).** Swept every source walker
    (`snapshot/provider.rs`, `engine/snapshot.rs`, `engine/l2/l2_workspace.rs`,
    `indexer.rs`, `main.rs`) plus every other `dependencies`-adjacent hit
    (the `app.json` manifest field, the unrelated `.alpackages`
    external-dependency machinery, doc mentions, frozen goldens): **CLEAN —
    no walker anywhere special-cases a `.dependencies` folder**; it is a
    normal old CAL→AL decompiled-source naming convention, already parsed,
    resolved, and represented in the committed goldens (confirmed by real
    resolved edges under `.dependencies/CDO/**` in
    `tests/goldens/semantic-edges/*.json`). No fix was needed; the CDO
    baselines this whole arc measured against required no re-derivation.
  - **Full re-measurement (Task 5, this entry, `--test-threads=1`, full
    160-test `program_resolve_harness` + the separate CDO-gated
    `source_overload_alias_collision_guard_group_count_pinned_on_cdo` lib
    test): every number reproduced BYTE-IDENTICAL to Task 4's post-change
    baseline** — `unknown`=95 (primary=whole), `realUnknownRate`=0.5247%
    primary / 0.2189% whole, `ambiguousResolved`=56 both scopes (ratchet
    pinned, `assert_eq!`), `unknownByReason`={CompoundReceiver: 51,
    UntrackedReceiver: 18, BuiltinPrecedenceCollision: 1, MemberNotFound: 25}
    (sums to 95, both scopes), `genuine_wrong`=0 (HARD GATE), `fresh_missing`=3,
    `fresh_wrong`=149 (all 149 adjudicated `fresh_ahead_dispatch`, never
    `genuine_wrong`), `fresh_extra`=5024, `matches`=6201, audit digest
    `b7b7407c71c19191feed4ca255614615154921427c0291b630cac88e6c6b08ac`; both
    applicability gates green and NON-VACUOUS
    (`route_applicability`: `total_routes=18663 violations=0 abi_unmapped=0`;
    `fan_out_applicability`: all 4 violation kinds 0,
    `routes_checked[interface=28 instance_builtin=463 implicit_trigger=1183
    event=3404]`, all > 0); frozen `cdo_event_audit_frozen_load` /
    `cdo_trigger_audit_frozen_load` digests unmoved
    (`728d9bb6a5c...8281ac` / `a250f70896...39c28`); the T1 dual-publisher
    guard fired 0 times (`eventFlowDualPublisherAliasSkips=0`); the T2
    residual-collision guard pinned 0/0 (primary/whole-program marked
    groups). All 160/160 harness tests + the collision-guard lib test pass,
    single-threaded, foreground, `CDO_WS`+`ENFORCE_CDO_WS=1`.
  - **Candidate-distribution correction (a review-caught undercount in the
    Task 4 entry below, fixed here with the real `--graphify-export`
    breakdown):** the 56 reclassified sites are **52 unique
    (caller, target-method) pairs** — 39 with 2 candidate overloads, 12 with
    3, and exactly 1 with 9 (`Codeunit "Http Content"`, System Application id
    2354, `.Create` — a genuinely 9-way-overloaded platform method) — summing
    to **123 unique (caller, candidate-target) routes**; 10 of those 123
    pairs have a second call site inside the SAME caller reaching the SAME
    candidate set, contributing a second `GEdge` each, which is where the
    previously-reported **133 total `GEdge`s** (123 + 10) actually comes
    from — not a uniform "2-3 candidates each" as originally stated.
  - **Doc fixes (review nits):** `full.rs`'s `CalleeShape::Commit` arm comment
    corrected from "the vanishingly rare case" to "structurally impossible in
    valid AL" — `Commit` is a reserved statement keyword, so no compiling AL
    source can ever declare a procedure that collides with it; the arm stays
    defensive-only, not a reachable live path. `sig_fp.rs`'s
    `source_routine_node_id` doc corrected from "the 5-site audit" naming only
    4 call sites to the full 6-site reality (5 originally audited + the
    Task-2-review-caught 6th, `semantic_golden.rs::build_fan_out_site_context`;
    today's live call-site count is 5, since one of the original 5 was dead
    code deleted rather than migrated — the 6 names the audit's total reach).
  - **DEFERRED** (recorded, not started this arc — the plan's own
    out-of-scope list plus the roadmap beyond it): the 13 workspace-tier
    `MemberNotFound` sites (need genuinely new proven-absent machinery; the
    preprocessor union-read favors absence proofs but needs its own
    confirming fixture first); **arg-type dispatch** — now the natural NEXT
    lever, since the 56 `AmbiguousResolved` edges already CARRY their full
    candidate set and only need a picker; the cross-object table-scope +
    interface per-implementer ambiguity populations (measured 0/56 on CDO
    this arc, so out of scope by measurement, not by fiat); unquoted bare
    implicit-`Rec` fields; protected `Variables[]`; `Sender` param-TYPE.
- **Candidate-carrying `AmbiguousResolved` for same-object overload ambiguity —
  metric-definition change (Task 4, sigfp-and-ambiguous-reclassification
  plan).** `resolve_in_object`'s genuine `>1`-visible arm now PREVALIDATES
  every candidate (concrete: not collapse-marked, and its constructed route
  carries non-`Unknown` evidence) BEFORE ever constructing
  `DispatchShape::AmbiguousOverload` — a mixed/degraded set (any candidate
  fails prevalidation) stays the pre-Task-4 single `Unresolved(OverloadAmbiguous)`
  route, `Exact` shape, unchanged. When every candidate is concrete, the
  function now returns ONE `Route` per candidate (Source for source tiers,
  Opaque+`AbiSymbol` for SymbolOnly, via the existing `make_routine_route`),
  each tagged `Condition::AmbiguousDispatch` — the taxonomy Task 3 built as
  inert mechanics is now WIRED. Step 0 measurement (CDO, throwaway
  instrumentation, deleted before commit) partitioned the 56 `OverloadAmbiguous`
  sites by emission call site: **100% (56/56) emit from `resolve_in_object`'s
  own arm via 3 non-nested, same-object call sites** (`resolve_member`'s
  `Object` receiver 41, `resolve_bare`'s Step 1 own-object 13,
  `resolve_bare`'s Step 3 implicit-Rec single-winning-table-scope-object 2) —
  ZERO from the cross-object table-scope `Ambiguous` outcome, ZERO from the
  interface per-implementer `matching!=1` arm, ZERO nested under an
  interface's SymbolOnly/source-tier delegate. The reclassification scope is
  therefore the FULL 56-site population, with no site scoped out.
  `resolve_in_object`'s signature is now `Option<(DispatchShape, Vec<Route>)>`
  (the file's own tuple convention) — all 7 call sites updated. `resolve_bare`
  (public API) is now `(DispatchShape, Vec<Route>)` too, so
  `resolve_call_site_obligation`'s `Bare`/`Commit` arms thread the REAL shape
  through instead of hardcoding `Exact` (behavior-preserving for every other
  case: `completeness_for_shape` maps both `Exact` and `AmbiguousOverload` to
  `Complete`). **Interface nesting OUT OF SCOPE (round-1 addendum, honored
  even though CDO measured zero live nested cases):** a new
  `interface_delegate_route` helper collapses a per-implementer
  `AmbiguousOverload` result back to the single pre-Task-4
  `Unresolved(OverloadAmbiguous)` route rather than extending the
  already-`Polymorphic` edge — pinned by a dedicated nested-interface fixture
  asserting the edge stays `Polymorphic`/2-routes (not 3) and never
  `AmbiguousResolved`. **Both-ways metric reporting (round-1 addendum,
  BINDING):** `Histogram::legacy_unknown_rate_including_ambiguous()` (and the
  additive `aldump` `--program-call-graph-stats` key
  `realUnknownRateLegacyIncludingAmbiguous`) reports the rate under the OLD
  (pre-Task-4) definition side-by-side with the new `realUnknownRate`, so the
  metric-definition change is never stat-juked — these edges remain
  PRACTICALLY unresolved at runtime from the tooling's perspective (a closed
  candidate set, not a pick). Charter §5/§8 (`docs/superpowers/specs/2026-06-28-bc-semantic-intelligence-charter.md`)
  get the explicit metric-definition addendum. Fixtures: a genuine 2-overload
  and a genuine 3-overload same-object call → `AmbiguousResolved`/N candidate
  routes, each `fires_by_default()==false` + in `may_reachable_routes()` +
  excluded from `default_reachable_routes()`, `Histogram.ambiguous_resolved`
  incremented and `Histogram.unknown` NOT incremented; a collapse-marked
  candidate mixed into an otherwise-ambiguous set → shape stays `Exact`,
  `Unknown(OverloadAmbiguous)`, never `AmbiguousDispatch`-tagged; the nested
  interface case (above); `ArityMismatch`/`AccessFilteredOverload`/
  `AbiCollapsedOverload`'s existing T2-split reasons regression-verified
  unchanged. Three PRE-EXISTING tests encoded the old single-route behavior
  and were REBASELINED (correctness over backwards compatibility — the new
  behavior is verifiably right): `ws_overload_collision_ambiguous_call_is_
  honest_unknown` → `..._becomes_ambiguous_resolved_with_two_candidates`
  (the canonical real-fixture ambiguity site, now 2 `Source` routes);
  `ws_cross_object_chain_abi_overload_uncollapsed_plain_dispatch_declines_
  ambiguous` → `..._becomes_ambiguous_resolved` (proves the SymbolOnly/ABI
  path too — 2 `Opaque`+`AbiSymbol` routes, since ABI candidates ARE
  "concrete exact" per the strict precondition); `unknown_reason_breakdown_
  over_real_fixtures_sums_and_spans_reasons` dropped `OverloadAmbiguous` from
  its expected-reasons list (its sole source in that corpus reclassified).
  CDO (measured, `--test-threads=1`, full 160-test harness, ALL GREEN):
  primary+whole `unknown` 151→95 (the full 56-site same-object population
  reclassified, `unknownByReason.overloadAmbiguous` 56→0, every other reason
  byte-identical), `ambiguousResolved` 0→56 in both scopes, `realUnknownRate`
  0.8341%→0.5247% (primary, 95/18104), 0.3479%→0.2189% (whole, 95/43404);
  `realUnknownRateLegacyIncludingAmbiguous` byte-identical to the pre-Task-4
  `realUnknownRate` at both scopes (0.008340698188245692 primary /
  0.003478942032992351 whole — the both-ways proof the reclassification is a
  pure relabeling, never a stat-juke). `--graphify-export` on CDO: 133
  `GEdge`s with `obligation:"ambiguous_resolved"` + `dispatch_shape:
  "ambiguous_overload"` + `may_fire:true` — 52 unique caller-target pairs (39
  with 2 candidates, 12 with 3, and 1 with 9 — `Codeunit "Http Content"
  .Create`, System Application id 2354) summing to 123 unique
  (caller, candidate-target) routes, with 10 of those pairs contributing a
  second `GEdge` for a repeat call site in the same caller (123 + 10 = 133
  total `GEdge`s) — real end-to-end confirmation of the Task 3 DTO mapping,
  not just the unit fixture. `genuine_wrong=0` (HARD GATE, unchanged); every
  one of the 56 reclassified sites landed `fresh_extra` (L3's frozen golden
  was EMPTY for all of them — acceptance-matrix rule 1, ungated): `fresh_extra`
  4968→5024 (+56), `matches` 6257→6201 (-56, the mirror movement),
  `fresh_wrong`/`fresh_missing`/`fresh_ahead_dispatch` counts BYTE-IDENTICAL
  (149/3/149) to the pre-Task-4 baseline (`genuine_wrong` stays 0 in that
  count too) — `FRESH_WRONG_CEILING`/`FRESH_MISSING_CEILING` need no motion;
  the audit digest moved (expected — fresh's projected targets for these 56
  sites are new non-empty content). Ratchets re-derived (dated 2026-07-03) to
  the new floor (`unknown<=95`, `real_unknown_rate<=0.005248`,
  `ambiguous_resolved==56` new ratchet); `.superpowers/sdd/task-4-report.md`
  has the full partition + exhaustive adjudication.
- **`AmbiguousDispatch`/`AmbiguousOverload`/`AmbiguousResolved` taxonomy trio —
  inert mechanics (Task 3, sigfp-and-ambiguous-reclassification plan).** Lays
  the honest vocabulary for reclassifying genuine same-object overload
  ambiguity (Task 4) OUT of `unknown` without ever calling it "resolved" in
  the misleading sense: `Condition::AmbiguousDispatch` ("exactly one of these
  routes fires at runtime, chosen by argument-type dispatch this engine
  cannot perform; not user-conditional") makes `Route::fires_by_default`
  return `false`, same as `ManualBinding`, and is included in the new
  `Edge::may_reachable_routes` may-traversal set (`default_reachable_routes`
  unchanged — a must-traversal set that correctly excludes both). Every
  `default_reachable_routes()` consumer was audited: none exist outside
  `edge.rs`'s/`resolver.rs`'s own reachability-contract tests, so no
  must-vs-may switch was needed. `DispatchShape::AmbiguousOverload` maps to
  `SetCompleteness::Complete` in `completeness_for_shape` — the candidate set
  is snapshot-enumerated and CLOSED, unlike `Polymorphic`'s open-world
  `Partial`. `ObligationOutcome::AmbiguousResolved` is a new `classify_obligation`
  branch with a STRICT precondition, checked before the pre-existing
  has-real/all-manual logic and never trusting a producer's shape choice
  alone: shape is `AmbiguousOverload`, the route set is non-empty, EVERY
  route carries `AmbiguousDispatch`, no route has `Evidence::Unknown`, and no
  route's target is `Unresolved` (i.e. every candidate is a concrete exact
  route — this alone excludes any collapse-marked candidate too, since a
  collapse-marked candidate manifests as an `Evidence::Unknown` route). A
  mixed/degraded candidate set fails this precondition and falls through
  UNCHANGED to the existing classification (e.g. a mix of one
  `AmbiguousDispatch` route + one Unknown-evidence route lands
  `ConditionalResolved` via the same not-fires-by-default fallback path
  `ManualBinding`-only sets already used — never misclassified as
  `AmbiguousResolved`, never silently dropped). `Histogram` gains an
  `ambiguous_resolved` counter (both `edge.rs`'s `Histogram::of_edges` AND
  `full.rs`'s documented-duplicate `count_into_histogram` — pinned
  byte-identical via a cross-check test), `graphify_export` gains a
  `project_edge` arm (`obligation:"ambiguous_resolved"`,
  `dispatch_shape:"ambiguous_overload"`, `condition:"ambiguous_dispatch"`,
  `confidence:"INFERRED"` — never `"AMBIGUOUS"`, which is reserved for
  `Unknown`'s true failure) plus an ADDITIVE, additive-only `GEdge.may_fire`
  field: `Some(true)` on every `AmbiguousDispatch` route so BC-Brain can
  never read the `fires_by_default:false` shape as dead code (exactly one
  candidate IS guaranteed to run) — pinned with an export fixture NOW even
  though no producer constructs these shapes yet (Task 4). `aldump`'s
  hand-built `--program-call-graph-stats` JSON (the one NON-compiler-forced
  surface) gains `ambiguousResolved` in both `wholeProgram` and
  `primaryScoped`. `integration_report.rs`'s `conditions()` mapping and
  `semantic_golden.rs`'s `route_applicability` gate both audited: the latter
  falls through to its `_ => {}` arm for the new shape (unit-tested, not
  assumed — an `AmbiguousOverload` `Call` edge matches neither the
  `Polymorphic` nor `Multicast` fan-out arms). **Inert by construction**:
  nothing in the resolver constructs `AmbiguousOverload`/`AmbiguousDispatch`
  yet, so this is mechanics only — CDO-confirmed byte-identical (primary
  `unknown=151`, `realUnknownRate=0.83%`, `genuine_wrong=0`,
  `ambiguousResolved=0` in both scopes, the `--graphify-export` output
  contains zero occurrences of `may_fire`/`ambiguous_overload`/
  `ambiguous_dispatch`/`ambiguous_resolved`) and the full 160-test CDO-gated
  harness green.
- **Real source `sig_fp` via ONE shared `RoutineNodeId` constructor — distinct
  overload identity (Task 2, sigfp-and-ambiguous-reclassification plan).**
  Source-tier `sig_fp` was hardcoded `0` at 5 independent reconstruction
  sites, so two genuine same-name/same-arity SOURCE overloads (differing only
  by parameter TYPE) aliased onto ONE `RoutineNodeId` (6 primary / 313
  whole-program aliased groups measured on CDO pre-fix), corrupting publisher
  spans (`BodyMap` last-write-wins) and merging the two overloads' caller
  identity on outgoing edges. New module `src/program/sig_fp.rs`: the shared
  `fnv1a` + `write_len_prefixed` primitives (moved from `abi_ingest`, now
  reused by BOTH tiers) and `source_routine_node_id(object, decl)` — the ONE
  constructor now used by ALL live source-tier reconstruction sites
  (`node_extract::extract_nodes`, `resolve::body_map::BodyMap::build`,
  `resolve::full::resolve_full_program_from_parts`,
  `resolve::stub::resolve_program`), so a declaration's identity can never
  silently diverge between sites. `sig_fp` = FNV-1a over the length-delimited
  fold of each parameter's `(conservatively normalized type text, by_ref)`
  tuple: normalization is LEXER-INSENSITIVE ONLY (trim + ASCII-lowercase +
  whitespace-run collapse — never quote-stripping/ID-vs-Name resolution,
  which would need compiler backing; under-normalization only splits, never
  aliases); `var` is folded as its own component (a separate grammar field,
  not part of the type text — array rank/subtype qualifiers ARE already in
  the verbatim `Param.ty` text); `params.is_empty() → 0` (ABI
  `param_type_fp` convention parity). The 5th audited site,
  `resolve::full::obligation_inventory` (+ its `Obligation`/`ObligationKind`
  carriers), was reviewer-confirmed DEAD CODE with zero callers (coverage is
  tracked inline in `resolve_full_program_from_parts`, never via that
  pre-pass) and is DELETED, with a historical note in `full.rs`'s module doc.
  **Marker reframe (T2 Step-1(d)):** `RoutineNode::source_overload_aliased`
  is now a same-id/different-`param_sig_key` COLLISION GUARD — normal
  overloads get distinct ids and survive UNMARKED; true re-parse duplicates
  still collapse unmarked; only a residual same-id/different-key survivor (a
  `sig_fp` normalization collision) is marked/fail-closed (the Task 1
  dual-publisher event-flow skip guard stays as the permanent net). Fixtures:
  `sig_fp.rs` unit tests (distinct types→distinct fp; case/whitespace
  variants→same fp; quoted-name-vs-numeric-ID never unified; `var`
  distinguishes; empty→0), `build.rs`
  `source_distinct_sig_fp_overloads_survive_unmarked` +
  `source_normalization_collision_marks_both_survivors_collision_guard`, the
  new end-to-end 4-site parity + per-overload-attribution fixture
  `tests/fixtures/sigfp_overload_identity` +
  `sigfp_identity_agrees_across_all_four_live_sites`, and the reframed Tests
  23f/23h (`distinct_sig_fp_overloads_survive_unmarked`,
  `distinct_sig_fp_publishers_both_emit_correct_spans` — each publisher
  overload now emits its OWN EventFlow edge with its OWN `name_origin` span,
  the exact fidelity fix this plan targeted; the Task 1 skip guard no longer
  fires for them). **Pinned:** the post-Task-2 collision-guard-marked group
  count on CDO is asserted at 0/0 (primary/whole-program) by the new
  CDO-gated `source_overload_alias_collision_guard_group_count_pinned_on_cdo`
  — any future nonzero = a normalization collision to investigate, never
  mask. CDO re-measure (CDO_WS, single-threaded): dispatch outcomes
  UNCHANGED — primary `unknown`=151, `real_unknown_rate`=0.8341%,
  `unknownByReason` byte-identical, `coverage.holds`=true, `genuine_wrong`=0;
  semantic goldens unmoved (site keys never encode `sig_fp`); frozen
  event/trigger digests byte-identical (CDO's aliased pairs carry zero
  dual-publishers, so no publisher span actually corrected on CDO — the
  span fix is proven by the in-repo fixture instead);
  `eventFlowDualPublisherAliasSkips`=0. `cargo test --workspace` green.
- **`.dependencies` folder special-casing preflight audit — CLEAN (Task 0,
  sigfp-and-ambiguous-reclassification plan).** Read-only sweep of every
  source walker for `.dependencies` folder-name special-casing, requested by
  the user as a PREFLIGHT before Task 1's CDO baselines: `src/snapshot/
  provider.rs::walk_al_source`, `src/engine/snapshot.rs::discover_al_files`/
  `count_app_json`, `src/engine/l2/l2_workspace.rs::discover_al_files`/
  `discover_al_files_app_scoped`/`count_app_json_paths` (which `src/engine/
  l3/l3_workspace.rs` reuses), `src/indexer.rs::index_directory`, and `src/
  main.rs::run_analysis` all skip only `.alpackages`/`.snapshots`/
  `node_modules`/`.git` — never `.dependencies`. Every other `dependencies`
  hit in the codebase is either the app.json manifest FIELD (`dependencies[]`
  / `declared_dependencies` / `primaryDependencies`) or the unrelated
  `.alpackages` external-dependency-resolution machinery (`src/
  dependencies.rs`, `indexer.rs::index_dependencies`). Confirmed positively:
  the frozen semantic goldens (`tests/goldens/semantic-edges/*.json`) already
  carry real resolved call-graph edges for CDO's own `.dependencies/CDO/**`
  source files, proving they are ingested and resolved as normal AL source,
  not excluded. No script/doc claims otherwise (a sibling plan's now-VOID T1
  proposal to skip `.dependencies` was deleted before implementation — see
  `docs/superpowers/plans/2026-07-03-dataitem-depscope-reason-split.md`'s
  header and this repo's prior CHANGELOG entry). No code changes required;
  Task 1 proceeded on unmodified CDO baselines.
- **Source-overload collision guard — `RoutineNode::source_overload_aliased`
  + `emit_event_flow_edges` dual-publisher SKIP guard (Task 1,
  sigfp-and-ambiguous-reclassification plan).** Source-tier `sig_fp` is
  always `0`, so two genuine same-name/same-arity SOURCE overloads (differing
  only by parameter TYPE) alias onto ONE `RoutineNodeId`;
  `dedup_routines_preserving_genuine_overloads` already kept both survivors
  (the prior Task 2 review fix), but neither was flagged as aliased, so a
  role-lookup consumer (rather than arity-filtered dispatch) had no way to
  know a `BodyMap` last-write-wins span lookup for the shared id might
  answer for the WRONG sibling. `RoutineNode` gains a new non-serialized
  `source_overload_aliased: bool` field (mirrors `abi_overload_collapsed`'s
  shape): `dedup_routines_preserving_genuine_overloads` (`build.rs`) marks
  EVERY survivor of a same-id run with ≥2 DISTINCT `param_sig_key`s, while a
  TRUE re-parse duplicate (one distinct key) still collapses to a single
  unmarked survivor. `resolver::emit_event_flow_edges` gains a new
  `dual_publisher_alias_ids` collision guard: a publisher id is SKIPPED
  entirely (never a synthetic zero-span) only when ≥2
  `source_overload_aliased` siblings sharing that id are BOTH publishers — a
  TRUE dual-publisher collision; a single-publisher-sibling pair (one
  overload is a publisher, its sibling is not) is unaffected and keeps
  emitting its one edge unchanged. Each skip is counted by the new
  `resolver::dual_publisher_alias_skip_count`, surfaced as `ProgramReport::
  event_flow_dual_publisher_alias_skips` and in aldump's
  `--program-call-graph-stats` JSON (`eventFlowDualPublisherAliasSkips`) for
  the report path. Four new fixtures in `tests/program_resolve_harness.rs`
  (`source_overload_alias_marks_both_survivors`,
  `true_duplicate_collapses_unmarked`,
  `dual_publisher_alias_skips_event_flow_edges`, plus the pre-existing
  `compound_obj_dup_and_overload_*` single-publisher-sibling pair confirmed
  unaffected); a mutation check (temporarily disabling the marking condition
  and the skip guard) confirmed the new assertions genuinely catch the
  regression before being restored green. CDO re-measure (`CDO_WS`,
  `ENFORCE_CDO_WS=1`, single-threaded, `--release`): resolution stats
  BYTE-IDENTICAL (primary `unknown`=151, `real_unknown_rate`=0.8341%,
  `unknownByReason` unchanged, `coverage.holds`=true, `genuine_wrong`=0) and
  `eventFlowDualPublisherAliasSkips`=0 — CDO's 6 aliased id-groups in the
  primary workspace app (18 marked routines total; hundreds more across
  embedded Base Application/CTS-SYS dependency source) carry ZERO publishers
  among them, so the dual-publisher guard never fires on CDO today and the
  frozen event/trigger digests are unmoved (confirmed via
  `cdo_event_audit_frozen_load`/`cdo_trigger_audit_frozen_load`, both
  byte-identical). `cargo test --workspace`: 159/159 in the touched harness,
  full workspace suite green; `cargo fmt --check` and `cargo clippy --release
  --all-features -- -D warnings` both clean.
- **Report-dataitem receivers + Unknown reason-split complete — real-`unknown`
  0.99%→0.83% (dataitem-depscope-reason-split plan, Task 3, FINAL — arc capstone).**
  Full re-measure on CDO (`CDO_WS`, `ENFORCE_CDO_WS=1`, single-threaded, `--release`,
  combined 156/156-test `program_resolve_harness` run): primary `real_unknown_rate`
  **0.83%** (raw 151/18104=0.008341), whole-program rate 0.35% (151/43404),
  primary/whole `unknown`=151/151, `unknownByReason`={`compoundReceiver`: 51,
  `untrackedReceiver`: 18, `overloadAmbiguous`: 56, `builtinPrecedenceCollision`: 1,
  `memberNotFound`: 25} (sum=151, verified both scopes via `aldump
  --program-call-graph-stats` directly against `CDO_WS`), `unknownReceiverTier`
  splits the 25 `memberNotFound` sites `embedded_source: 12` / `workspace: 13`.
  `genuine_wrong`=0, `fresh_missing`=3, `fresh_wrong`=149 (all `fresh_ahead_dispatch`).
  All 9 CDO gates green (metric, audit, ABI integrity, both applicability teeth
  non-vacuous — interface=28/instance_builtin=463/implicit_trigger=1183/event=3404 —
  the Sender+1 preflight, both frozen trigger/event audits byte-identical digests, the
  precedence-adjudicated `genuine_wrong` breakdown `l3_error_intrinsic`=52/
  `fresh_false_builtin`=0/`needs_manual_review`=0). `cargo test --workspace`: 2031
  passed, 0 failed; `cargo fmt --check` and `cargo clippy --release --all-features -- -D
  warnings` both clean. **Net across the T1-T2 arc (this plan): 0.99% (180) → 0.83%
  (151), −29 count / −0.16pp, `genuine_wrong` stays 0 through both tasks.** Trajectory:
  **T1** (report-dataitem receivers) modeled `ObjectDecl.report_dataitems`/
  `RoutineDecl.dataitem_source_table` as first-class receiver-typing inputs — a new
  Step 2b dataitem-name lookup in `infer_receiver_type`, a routine-contextual
  Report/ReportExtension arm of `infer_implicit_rec`, a centralized quote-aware
  `is_atomic_receiver_token` guard (fixing the naive dot-substring check that
  mislabeled quoted dataitem names with embedded periods `CompoundReceiver`), and an
  additive `modify()` lowerer fix (`RawKind::ModifyModification` carries `Target`, not
  `Name` — `collect_routines`'s Name-based gate never saw it). Landed in TWO commits:
  the initial implementation (`78ff3e4`, 180→159) then a review-fix (`5b1bb94`,
  159→151) that caught and corrected its OWN regression — the centralized guard's
  unquoted-branch `(`-exclusion ran before its quote-parity check, so a QUOTED field
  name containing an interior paren (`"View (Blob)"`, `"Request Page (XML)"`, real BC
  shapes) wrongly fell to `Unknown(CompoundReceiver)`. The corrected accounting: the
  dataitem mechanism's real, unmasked yield is 19 distinct dataitem-name receivers
  resolving across 29 total call-site edges (spanning both the `UntrackedReceiver` and
  the quote-fix-enabled `CompoundReceiver` paths), netted against the review-fix's own
  8 site restorations (`Unknown(CompoundReceiver)`→`Catalog`, `Blob::createinstream`/
  `createoutstream`) + 1 relabel (`CompoundReceiver`→`UntrackedReceiver`, genuinely
  `Unknown` before and after) — reconciling exactly to the measured bucket movement
  `CompoundReceiver` 61→51 (−10) / `UntrackedReceiver` 37→18 (−19) = −29 = 180−151.
  Exhaustive pre/post edge-dump diffs (all 18,586 CDO routes, not a sample) back both
  the initial implementation and the review-fix; `genuine_wrong`=0 held throughout.
  **T2** (Unknown reason-split, diagnostic-only) split `OverloadAmbiguous` into its 4
  conflated emission shapes (`ArityMismatch`, `AbiCollapsedOverload`,
  `AccessFilteredOverload`, and the residual genuine `>1`-visible-candidate case) and
  `MemberNotFound` into `ObjectNotInGraph` (receiver object itself absent) vs.
  `MemberNotFound` (member absent on a resolved surface, now tagged with an additive
  `Route::receiver_tier`) — count-preserving by construction, verified
  **zero-movement**: every one of CDO's 151 residual sites landed in the SAME reason
  bucket before and after (0 `ArityMismatch`, 0 `AbiCollapsedOverload`, 0
  `AccessFilteredOverload`, 0 `ObjectNotInGraph`). **What the zero-movement result
  MEANS** (the actual deliverable, not a null result): CDO's residual `OverloadAmbiguous`
  population (56 sites) is uniformly the textbook case — genuine multi-candidate,
  same-arity, visible-to-the-caller ambiguity (e.g. `HttpMgt.DownloadFile(ReadStream,
  Url)` vs. two real 2-arg source overloads) — which VALIDATES the deferred
  outcome-reclassification plan's `OverloadAmbiguous`-targeting design (a
  candidate-carrying, non-default-reachable `ObligationOutcome`, the
  `ConditionalResolved`/`fires_by_default` precedent) as aimed at the right population;
  it is not chasing a phantom. And the new `receiver_tier` diagnostic's `memberNotFound`
  split (`embedded_source: 12` / `workspace: 13`) tier-PROVES the 13 `workspace`-tier
  sites are honest-empty candidates (only a source-complete tier can ever prove member
  absence — `SymbolOnly` never can), a data-backed target for that same future plan.
  **The plan's original `.dependencies/` ingestion-scope task was DELETED before this
  arc started** (binding user correction, recorded in the plan header 2026-07-03):
  `.dependencies/` folders in the CDO workspace are normal AL source (an old CAL→AL
  conversion naming convention), not a stray decompiled cache — excluding them would
  have dropped real source from the graph. No code in this arc touches the ingestion
  walker; the 9/25 `.dependencies/`-resident `MemberNotFound` sites documented in the
  plan's grounding report are honest workspace reality, not a bug. Ratchets confirmed
  AT the measured floor (rate ceiling `0.008341`, primary/whole `unknown` ceiling `151`
  — tightened in `bd5d900`, re-confirmed byte-identical this task, no further
  tightening needed); `fresh_missing`/`fresh_wrong` ceilings (3/149) unchanged.
  **DEFERRED (next plan, now data-backed):** the outcome-reclassification proper (a new
  `ObligationOutcome` for genuine `OverloadAmbiguous`, candidate-carrying;
  tier-proven-empty treatment for the 13 `workspace`-tier `MemberNotFound` sites) — its
  own plan + review; report-dataitem leftovers (none — all 29 real CDO dataitem uses
  now resolve); unquoted bare implicit-Rec fields (still deferred, unrelated to
  dataitems); the source-tier `sig_fp=0` overload-identity degeneracy (two
  same-arity, different-parameter-TYPE source overloads alias one `RoutineNodeId` —
  root-caused this arc, fixed nowhere, flagged as pre-existing and out of scope);
  the `.dependencies/`-special-casing audit (user-requested follow-up: a quick grep
  found no other special-casing of that directory name in the ingestion path, but a
  thorough sweep of the full walker/dependency-resolution surface is still owed);
  protected `Variables[]`; Sender param-TYPE validation (only arity is currently
  checked).
- **`UnknownReason` reason-split: `ArityMismatch`/`AbiCollapsedOverload`/
  `AccessFilteredOverload` (out of `OverloadAmbiguous`) + `ObjectNotInGraph` (out of
  `MemberNotFound`) + the additive `Route::receiver_tier` diagnostic
  (dataitem-depscope-reason-split plan, Task 2 — DIAGNOSTIC-ONLY, count-preserving).**
  `resolve_in_object`'s `OverloadAmbiguous` conflated four structurally distinct decline
  shapes (`src/program/resolve/resolver.rs`): zero arity-matched candidates now emits
  `ArityMismatch` (nothing to be ambiguous BETWEEN); the sole visible candidate being
  `RoutineNode::abi_overload_collapsed`-marked now emits `AbiCollapsedOverload` (an ABI
  ingestion-fidelity admission, not a live candidate set); access narrowing an
  originally-ambiguous (`pre_filter_count > 1`) set down to exactly one visible survivor,
  then declining rather than selecting it, now emits `AccessFilteredOverload`; a genuine
  `>1`-visible same-arity ambiguity is UNCHANGED, still `OverloadAmbiguous`. Scoped
  strictly to `resolve_in_object`'s own three emission sites — the other
  `routine_is_collapse_marked` call sites (`resolve_object_run`'s entry-trigger lookup,
  `resolve_implicit_trigger`'s fan-out, `resolve_member`'s inline `Codeunit.Run`
  special-case) are unchanged, still `OverloadAmbiguous`, per the plan's explicit
  grounding. Similarly, `MemberNotFound` conflated "the receiver OBJECT itself is absent
  from the graph" with "the receiver resolved but the member is absent" —
  `resolve_object_run`'s and `resolve_member`'s `Object`-arm absent-target shapes now emit
  `ObjectNotInGraph` (no externality claim — an `UndeclaredExternalTarget`-style label was
  considered and dropped as unprovable from mere absence, per the charter's open-world
  discipline); every other `MemberNotFound` site (bare-call Step 5's untouched default,
  `resolve_member`'s `SelfObject`/`Interface` arms, the post-`resolve_in_object`-None
  Object-arm fallback) stays `MemberNotFound`, now additionally tagged with the resolved
  receiver's `TrustTier` via a new `Route::receiver_tier: Option<TrustTier>` field — a
  SEPARATE additive/nullable diagnostic, not a reason-string split (`MemberNotFound`
  stays one stable `as_str()` key; `ObjectNotInGraph` always carries `receiver_tier:
  None`, since there is no resolved receiver to tag). `TrustTier` gained `Hash`/
  `PartialOrd`/`Ord` derives (needed for `Route`'s existing derive stack) and a canonical
  `as_str()` method (`graphify_export::tier_str` now delegates to it, byte-identical
  output). New `unknown_receiver_tier_breakdown` function
  (`src/program/resolve/edge.rs`) stratifies by `(UnknownReason, Option<TrustTier>)`,
  wired additively into `aldump --program-call-graph-stats`'s new `unknownReceiverTier`
  JSON key (sibling of `unknownByReason`, both `wholeProgram`/`primaryScoped` scopes) and
  `graphify_export`'s `GEdge.unknown_receiver_tier` field (appended last, never reorders
  existing keys — BC-Brain consumes this export). Diagnostic-only by construction: no
  `ObligationOutcome`/`classify_obligation` change, `Evidence::kind()`'s projection
  untouched, committed semantic goldens byte-identical (no regen needed), per-site
  bijection holds (every pre-Task-2 `Unknown` site maps 1:1 to a post-Task-2 `Unknown`
  site with only the reason/`receiver_tier` diagnostic fields changed). **3** new
  collision-free unit fixtures in `resolver.rs` (corrected 2026-07-03, Task 3 doc-count
  fix — the genuine `>1`-visible-ambiguity control, and a manually
  constructed distinct-`sig_fp` fixture for `AccessFilteredOverload` — two SOURCE-tier
  same-arity, different-PARAMETER-TYPE overloads share one `RoutineNodeId` since source
  `sig_fp` is always 0, so an AL-source-text fixture for that shape is unreliable; see
  `resolve_member_object_two_distinct_sig_fp_overloads_access_narrowed_to_one_declines`'s
  doc, and the Step-5-default `MemberNotFound`+tier fixture; the `ArityMismatch`/
  `AbiCollapsedOverload`/`ObjectNotInGraph`-×2 shapes were exercised by TIGHTENING 4
  pre-existing tests instead of adding new ones) plus 2 new `edge.rs` unit tests (`as_str()` key uniqueness,
  `unknown_receiver_tier_breakdown`'s sum/stratification invariants). Measured on CDO
  (`CDO_WS`, single-threaded, `--release`): `real_unknown_rate`/`unknown` count BYTE-
  IDENTICAL at 0.83% / 151 (both primary and whole-program) — a genuine, measured
  zero-movement result: CDO's current 151-site residual happens to be homogeneous per
  shape family (every `OverloadAmbiguous` site is a genuine >1-visible ambiguity, every
  `MemberNotFound` site is a resolved-surface member-miss; the collapse-marker guard is
  dormant on CDO by construction — 0 `abi_overload_collapsed` routines). The NEW
  `unknownReceiverTier` diagnostic DOES surface new information: the 25
  `memberNotFound` sites split `embedded_source: 12` / `workspace: 13` (verified via
  `aldump --program-call-graph-stats` directly against `CDO_WS`). `genuine_wrong`=0 and
  every applicability/preflight/audit gate green (156/156 harness tests, full CDO run).
- **Report-dataitem receivers + trigger implicit-Rec + quote-aware token guard + additive
  `modify()` lowering — real-`unknown` 0.99%→0.88% (dataitem-receivers plan, Task 1).**
  Models `al_syntax::ir::ObjectDecl.report_dataitems`/`RoutineDecl.dataitem_source_table`
  as first-class receiver-typing inputs in the fresh engine (previously consumed only by
  the legacy L2 engine): `node_extract::DataitemNode` on `ObjectNode` (Report/
  ReportExtension only, mirrors `page_controls`); a new Step 2b in `infer_receiver_type`
  (`src/program/resolve/receiver.rs`) — a unique dataitem-NAME receiver match (case-
  insensitive, unquoted), strictly after Step 2's var/param/global miss (vars always
  shadow a dataitem), fail-closed on a same-named report procedure collision or a
  duplicate name across the own+extended-base dataitem maps; the Report/ReportExtension
  arm of `infer_implicit_rec` is now ROUTINE-CONTEXTUAL (binds from the enclosing
  `RoutineDecl.dataitem_source_table`, or the new `modify()` resolve-time fallback below —
  never object-level, never for a `requestpage` trigger). Two pre-existing defects fixed
  alongside: (1) **the naive dot-substring quote guard** — `receiver_lc.contains('.')`
  mislabeled a QUOTED dataitem name with an embedded period (`"Sales Cr.Memo Header
  Filter"`, a real CDO name) `CompoundReceiver`; replaced by one centralized
  `is_atomic_receiver_token` helper (quote-aware: atomic iff no unquoted `.`) shared by
  `receiver.rs`'s Step 2/3a/4 guards and `full.rs`'s `CompoundReceiver` relabeling; (2)
  **the ReportExtension `modify()` lowerer gap** — `RawKind::ModifyModification` carries
  its target in the grammar's `target` field, not `name`, so `collect_routines`'s
  Name-based member-wrapper gate never recognized it, losing `enclosing_member` for every
  trigger nested in `modify(X) { .. }`. Fixed additively: `collect_routines`
  (`crates/al-syntax/src/lower/mod.rs`) gets a dedicated `ModifyModification` arm (reads
  `Target`) plus a new `RoutineDecl.in_dataset_modify_context: bool` field — `true` only
  for a CONFIRMED report/report-extension `dataset { modify(X) { .. } }` member (forced
  `false` descending into `requestpage`, REQUESTPAGE ISOLATION, and for every other
  `modify()` context — fields/layout/views); the resolver's dataitem-map fallback
  (`resolve_dataitem_source_table`, reused by both Step 2b and the `modify()` case) fires
  only when that flag is set. New fixtures `tests/r0-corpus/ws-report-dataitem/` (5
  positive scenarios + 5 negatives: var-shadow, procedure-name collision, duplicate-
  across-own-and-base, requestpage isolation, genuinely-compound-stays-compound) + 12 new
  `receiver.rs` unit tests + 3 focused `al-syntax` lowerer unit tests for
  `ModifyModification.Target`. Existing `ws-page-rec/src/ReportWithDataitem.Report.al`
  fixture's `Rec.GetDisplayName()` (previously an intentional NEGATIVE, per the old
  per-dataitem-scoping gap) now correctly resolves `Evidence::Source` — updated, not a
  regression. CDO (`CDO_WS`, single-threaded, `--release`): primary `real_unknown_rate`
  0.99%→0.88% (raw 159/18104=0.008782), primary/whole `unknown` 180→159,
  `unknownByReason` delta `UntrackedReceiver` 37→17 (−20) + `CompoundReceiver` 61→60
  (−1), every other bucket byte-identical (`OverloadAmbiguous`=56,
  `BuiltinPrecedenceCollision`=1, `MemberNotFound`=25); `genuine_wrong`=0 and
  `fresh_wrong`=149 both UNCHANGED (companion audit gates); `fresh_missing`=3 unchanged.
  All 9 CDO gates green: metric, audit, ABI integrity, both applicability teeth
  (interface/instance_builtin/implicit_trigger/event all 0 violations, non-vacuous route
  counts), the include-Sender preflight, the trigger/event frozen audits, and the
  genuine-wrong precedence adjudication. Real-CDO-source-grounded: the dot-bearing
  `"Sales Cr.Memo Header Filter".GetView()`/`.GetFilters()` pattern (Report 6175283 "CDO
  Update Output Profile", lines 435/510) spot-verified — both are platform `Record`
  catalog methods, so once Step 2b types the receiver, the pre-existing builtin-catalog
  dispatch (untouched by this task) resolves them safely regardless of table identity.
  Metric/count ratchets tightened (0.00995→0.00879 / 180→159, dated 2026-07-03); the
  `FRESH_MISSING_CEILING`/`FRESH_WRONG_CEILING` audit ratchets are unchanged (measured
  values didn't move). Out of scope (deferred, per the plan): unquoted bare
  implicit-Rec dataitem-name fields; XmlPort/Query dataitem modeling (zero on CDO).
  **Correction (Task 1 review fix, below):** the `CompoundReceiver` 61→60 (−1) delta
  reported above was NOT a clean, isolated movement — it silently netted a genuine
  −10 dataitem win against a +9 regression this same task introduced in
  `is_atomic_receiver_token` (8 sites false-demoted to `Unknown`, +1 relabel). See the
  Fixed entry for the corrected accounting and final post-fix numbers.

### Fixed
- **`resolve_in_object`'s `_` arm prevalidated only ABI collapse-marking, not
  source-alias — the last laundering path out of `unknown` for a residual
  `sig_fp` collision (whole-branch review fix, F1, HIGH).** The plan's own
  binding addendum requires the `DispatchShape::AmbiguousOverload` prevalidation
  to decline when "NO candidate is collapse-marked (ABI **or source-alias**)",
  but the `_` arm's `degraded` predicate consulted only `RoutineNode::
  abi_overload_collapsed` via `routine_is_collapse_marked` — never
  `RoutineNode::source_overload_aliased`. Two `source_overload_aliased`
  survivors (a residual same-id `sig_fp` collision — two GENUINELY DISTINCT
  source overloads sharing one `RoutineNodeId`) would both resolve through the
  SAME `BodyMap` entry (`BodyMap` is keyed by `RoutineNodeId`), producing two
  IDENTICAL-target concrete routes that slipped past both the `_` arm's
  prevalidation AND `edge::classify_obligation`'s `is_ambiguous_resolved`
  classifier backstop, constructing a confident-looking `AmbiguousOverload`/
  `AmbiguousResolved` edge out of a genuine unresolved collision. Fixed: the
  `_` arm's `degraded` predicate now ALSO treats a `source_overload_aliased`
  candidate as degraded (new `routine_is_source_aliased` helper, mirroring
  `routine_is_collapse_marked`), plus a cheap belt-and-braces dedup-shrink
  check — deduping `visible`'s `RoutineNodeId`s down to fewer entries than
  routes is never a valid `AmbiguousOverload` input, regardless of either
  marker. Both degrade to the existing single `Unresolved(OverloadAmbiguous)`
  route, `DispatchShape::Exact`. Unit test added
  (`resolve_member_object_ambiguous_set_with_source_alias_candidates_stays_unknown`):
  a synthetic same-id source-aliased pair, with a REAL `BodyMap` entry (via
  `sig_fp::source_routine_node_id` on real parsed source) so both candidates
  resolve non-`Unknown` — proven failing before the fix (constructed
  `AmbiguousOverload` with two identical `Routine(...)` routes) and passing
  after. **Inert on CDO**: `source_overload_alias_collision_guard_group_count_
  pinned_on_cdo` measures 0/0 marked groups on the real workspace, so this
  fix cannot move any CDO number — independently re-confirmed: full 160-test
  `program_resolve_harness` byte-identical (`unknown` 95/95,
  `realUnknownRate` 0.52%, `ambiguousResolved` 56/56 exact-pinned,
  `genuine_wrong=0`). Also rewrote the now-stale "source `sig_fp` is always
  `0`, so two distinct SOURCE declarations never collide" doc comments (both
  the module-level doc and `resolve_in_object`'s own arity-match comment,
  F2, MEDIUM) — the exact false reasoning that masked F1 — to describe the
  post-Task-2 reality: `sig_fp` is a real fingerprint, a genuine overload
  pair almost never collides, and a residual collision is caught by the
  degraded-set guard above, never trusted as distinct. Additionally hardened
  `graphify_export.rs`'s `AmbiguousResolved` arm (F3, observation) with the
  same Unknown/Unresolved skip the sibling `Resolved` arm already has, plus a
  `debug_assert` — safe by `classify_obligation`'s `is_ambiguous_resolved`
  invariant today, defense-in-depth against a future producer bug.
- **`build_fan_out_site_context` missed the Task 2 `source_routine_node_id`
  unification — the 6th live `RoutineNodeId` construction site, still
  hardcoding `sig_fp: 0` (Task 2 review fix).** Task 2 (above) migrated 4 live
  reconstruction sites onto the shared `source_routine_node_id` constructor
  but missed `semantic_golden.rs::build_fan_out_site_context` — production
  code that re-walks the same call sites `resolve_full_program` resolves to
  recover `FanOutSiteContext` for `route_applicability`'s fan-out soundness
  teeth. Because `SiteId.caller: RoutineNodeId` participates in `Eq`/`Hash`,
  the map this function built could never be looked up by
  `route_applicability` for any caller with ≥1 parameter (real `sig_fp` on
  one side, hardcoded `0` on the other), silently falling into the
  fail-closed `None` branch and flagging every such route a VIOLATION on
  CDO: `interface_applicability_violations=24`,
  `implicit_trigger_violations=324` — both gates (`route_applicability_
  zero_violations`, `fan_out_applicability_zero_violations`) were broken
  while still reporting green, because the in-repo `fanout-applicability`
  fixture's only caller (`Go()`) happened to be zero-param, where a
  hardcoded `0` and a real `sig_fp` are indistinguishable. Fixed:
  `build_fan_out_site_context` now calls `source_routine_node_id` like every
  other live site (true 6-site unification). The fixture was hardened so
  this class of bug can never pass silently again:
  `tests/fixtures/fanout-applicability/Interface.al` and `Trigger.al`'s
  callers are now PARAMETERIZED (`Go(Dummy: Integer)`), forcing the map
  lookup to depend on a genuinely non-zero, correctly-agreeing `sig_fp` —
  this fixture change reproduces the bug (proven failing before the fix,
  passing after). CDO re-verified post-fix: both applicability gates at 0
  violations with NON-VACUOUS route counts
  (`interface_routes_checked=28`, `implicit_trigger_routes_checked=1183`,
  `instance_builtin_routes_checked=463`, `event_routes_checked=3404`);
  `cdo_full_program_coverage_and_self_reported_metric` unchanged
  (`unknown=151` / `real_unknown_rate=0.0083`); `genuine_wrong=0`; frozen
  event/trigger digests byte-identical; the pinned source-overload
  collision-guard group count unchanged at 0/0. Also updated `build.rs`'s
  now-stale "source `sig_fp` is always `0`" doc comments (present-tense,
  written before Task 2 landed) to describe the post-Task-2 reality: `sig_fp`
  is now a real parameter-type fingerprint, and a same-id survivor run means
  either a true re-parse duplicate or a rare residual fnv1a collision, not
  the general case. `sig_fp.rs` gained an explicit doc note on the `by_ref`
  fold's over-split-never-alias asymmetry rationale, honestly flagging as an
  open question whether AL itself treats a var-only-differing parameter list
  as a legal distinct overload.
- **`is_atomic_receiver_token` judged a well-formed QUOTED receiver token on its
  UNQUOTED-branch `(` call-shape exclusion before its own quote-parity check — 8
  real-field CDO sites false-demoted to `Unknown` (dataitem-receivers plan, Task 1
  review fix).** Task 1's centralization of the atomic-receiver-token guard (above)
  applied the unquoted branch's `if s.contains('(') { return false; }` BEFORE checking
  whether `s` was a well-formed quoted span — so a QUOTED identifier containing an
  interior paren (a real BC field-name shape: `"View (Blob)"`, `"Request Page (XML)"`)
  wrongly classified COMPOUND instead of ATOMIC, and Step 3a's bare implicit-Rec
  quoted-field lookup never engaged for it. Confirmed by exhaustive pre/post edge-dump
  diff over all 18,586 CDO routes (only 9 differ, zero collateral elsewhere): **8
  regressed sites** — `Table 6175282 "CDO Queue Entry".al:172/:179`, `Table 6175284
  "CDO E-Mail Template Line".al:900/:911`, `Table 6175307 "CDO E-Mail Templ. Line
  Report".al:287/:298`, `.dependencies/CDO/Table/CDOPageDefaultfilter.Table.al:184/:193`
  — restored from `Unknown(CompoundReceiver)` back to `Catalog`
  (`Blob::createoutstream`/`Blob::createinstream`, matching the SAME field shapes Task
  4 (applicability-param-subtype-recfield plan v2.1) had already independently
  confirmed resolved via this exact Blob-catalog path before Task 1 ever ran); **1
  site relabeled** (`.dependencies/CDO/Page/CDOPageDefaultFilters.Page.al:87`,
  `CalcFields("View (Blob)")`) from `Unknown(CompoundReceiver)` to
  `Unknown(UntrackedReceiver)` — genuinely `Unknown` before AND after, a diagnostic
  reason-bucket correction only, not a resolution change. Fixed: the quoted branch of
  `is_atomic_receiver_token` is now judged PURELY on quote-parity (`len() > 2`, starts
  AND ends with `"`, exactly 2 quote characters) — an interior paren inside a
  well-formed quoted span is just a character of the identifier, never a call-shape
  signal (a quoted span can never itself be a call target); the `(` call-shape
  exclusion now applies ONLY to the unquoted branch. New unit tests
  (`is_atomic_receiver_token_quoted_paren_is_atomic`,
  `is_atomic_receiver_token_paren_fix_negatives`,
  `step3a_bare_quoted_field_with_interior_paren_resolves_blob` in `receiver.rs`) pin
  the fix; Step 3a's now-redundant `len()`/`ends_with('"')` re-check (subsumed by the
  helper once gated on `starts_with('"')`) removed. **The `modify()` lowerer fix
  (Task 1, above) is GLOBAL** — `collect_routines`'s `RawKind::ModifyModification` arm
  fires for any `modify()` block regardless of enclosing object kind (Table/Page/
  PageExtension/TableExtension too, not only report `dataset`/`requestpage`); this was
  correct but undescribed/untested — pinned by a new
  `modify_modification_in_tableextension_fields_populates_member_not_dataset_context`
  lowerer test (confirms `enclosing_member` populates for a TableExtension field
  `modify()` trigger while `in_dataset_modify_context` correctly stays `false`, since
  `dataset_ctx` is only ever forced `true` descending into a report `DatasetSection`/
  `ReportDataitem`) — inert on CDO (verified: zero TableExtension `modify()` sites
  exercise the resolver's dataset fallback). CDO re-measure (`CDO_WS`,
  single-threaded, `--release`): primary/whole `unknown` **159→151**, primary
  `real_unknown_rate` **0.88%→0.83%** (raw 151/18104=0.008340); `unknownByReason`
  `CompoundReceiver` 60→**51** (−9 = the 8 restorations + the 1 relabel-away),
  `UntrackedReceiver` 17→**18** (+1 = the relabel-in), `OverloadAmbiguous`=56,
  `BuiltinPrecedenceCollision`=1, `MemberNotFound`=25 all byte-identical;
  `genuine_wrong`=0 and `fresh_wrong`=149 both UNCHANGED; `fresh_missing`=3 unchanged;
  trigger/event frozen-audit digests UNCHANGED; fan-out non-vacuity counts
  (interface=28, instance_builtin=463, implicit_trigger=1183, event=3404) UNCHANGED.
  All 9 CDO gates green. Metric/count ratchets tightened (0.00879→0.00834 /
  159→151, dated 2026-07-03).

- **Applicability-checker fix + ABI param-Subtype fidelity + record-field chains complete
  — real-`unknown` 1.75%→0.99%, SUB-1% for the first time (applicability-param-subtype-
  recfield plan v2.1, Task 5, FINAL — arc capstone).** Closes the plan Task 1 opened.
  Full re-measure on CDO (`CDO_WS`, `ENFORCE_CDO_WS=1`, single-threaded, `--release`),
  byte-identical to Task 4's own measurement (Task 5 makes no resolver changes): primary
  `real_unknown_rate`=0.99% (raw 180/18104=0.009943), whole-program rate=0.41%
  (180/43404), primary/whole `unknown`=180/180, `genuine_wrong`=0, `fresh_missing`=3,
  `fresh_wrong`=149 (all `fresh_ahead_dispatch`), `unknownByReason`={CompoundReceiver: 61,
  UntrackedReceiver: 37, OverloadAmbiguous: 56, BuiltinPrecedenceCollision: 1,
  MemberNotFound: 25} (sum=180=`unknown`, verified both scopes). Full 7-gate CDO harness
  green in one combined process: `cdo_full_program_coverage_and_self_reported_metric`,
  `cdo_l3_semantic_audit_no_fresh_wrong`, `fan_out_applicability_zero_violations`
  (event_violations=0, non-vacuity interface=28/instance_builtin=463/
  implicit_trigger=1183/event=3404), `route_applicability_zero_violations`
  (violations=0, abi_unmapped=0), `cdo_unknown_include_sender_plus1_subscribers_
  preflight_is_zero` (count=0), `cdo_genuine_wrong_is_precedence_adjudicated`
  (`l3_error_intrinsic`=52, `fresh_false_builtin`=0, `needs_manual_review`=0),
  `committed_goldens_metadata_is_valid` (52/52). **Net across the whole T1-T4 arc:
  1.75% (317) → 0.99% (180), −137 count / −0.76pp, `genuine_wrong` stays 0 through every
  task.** Trajectory: **T1** — the pre-existing broken `event_violations=200` applicability
  gate root-caused to `ae35e90`'s Sender-tolerant `+1` wiring predating the checker's
  still-strict arity invariant (a synchronized-wrongness risk closed by making the
  tolerance CONDITIONAL on the publisher's actual `IncludeSender` attribute value, never a
  blanket `+1`, via one shared `event::subscriber_arity_bound` helper consumed by both
  wiring and checker) — `event_violations` 200→0, CDO byte-identical (both gates were
  dormant on real over-wired routes; the 200 were exactly the legitimately-wired
  `IncludeSender=true` population), full CDO harness 126/128→128/128. **T2** — ABI
  param/field Subtype fidelity (`parse_method`/`parse_field` carrying the full
  `Codeunit "Dep A"`-shaped text instead of the bare outer keyword, plus a
  discriminator-bearing `param_type_fp` closing the Id-only-subtype collapse sliver, plus
  a plain-dispatch collapse-marker guard) — **CDO-DORMANT plumbing, not a metric mover**:
  every CDO dependency is `EmbeddedSource`, never `SymbolOnly`, so zero routines are ever
  collapse-marked on this corpus; proven exclusively by fixtures against a real
  no-embedded-source probe `.app`, exactly like the prior plan's Task 1 protected-ABI fix.
  A same-task review fix extended the marker guard to all five route-construction sites
  (plain dispatch + Run/trigger/event paths), also CDO-dormant. **T3** — the table-field
  type index (`FieldNode` on `ObjectNode`, populated from source `FieldDecl` and ABI
  `AbiField`) + the non-method `Member{object, member}` record-field arm in
  `infer_compound_member_receiver` (`Rec."Field".X()` and any `Record`-typed base, not
  only literal `Rec`) + EnumType-as-chain-base (`Ordinals()`/`Names()` → `Framework(List)`)
  — the largest single-task drop of the arc: `CompoundReceiver` 144→61 (−83), rate
  1.75%→1.29%. **T4** — bare implicit-Rec QUOTED-field receivers (`"Field".X()` with no
  `Rec.` prefix inside a Table/TableExtension's own procedure) + a Step-2 quote-parity
  fix (a quoted identifier naming a real local var previously never matched the
  already-unquoted `VarDecl` name and silently fell through) — `UntrackedReceiver`
  91→37 (−54), rate 1.29%→0.99%. **The round-2 proc-shadow guard correction**
  (`ResolveIndex::table_scope_has_routine`, applied to both T3's and T4's field arms):
  AL's parens are optional on a zero-argument procedure call (`Rec.Insert;` compiles —
  Code Cop AA0008 flags the missing parens as a STYLE issue, not a compile error), so a
  bare `Member` AST node — and a bare quoted receiver used as the base of a further call
  — is structurally AMBIGUOUS between a field/property access and a parens-less
  procedure-call chain; a same-named routine anywhere in the visibility-scoped table
  surface now declines field-typing rather than guessing. Measured CDO delta from the
  guard alone: zero (the exhaustive edge-diffs for both T3 and T4 showed no site
  regressed) — a soundness correction that happened to cost nothing on this corpus, not a
  metric-neutral no-op by construction. **Exhaustive adjudication sign-off (re-confirmed,
  not re-sampled):** T3's 83 newly-`Catalog` edges and T4's 54 newly-`Catalog` edges were
  each hand-adjudicated against real CDO source during their own task (full before/after
  edge-dump diffs, zero site additions/removals/collateral changes — see
  `.superpowers/sdd/task-3-report.md` and `task-4-report.md`); 83+54=137 equals the exact
  net `unknown` count drop (317→180) and the exact sum of the two bucket drops
  (`CompoundReceiver` −83, `UntrackedReceiver` −54); no dataitem/var was mis-typed as a
  field anywhere (the var/param/global lookup and the routine-shadow guard both run and
  win BEFORE any field lookup, per fixture). **Ratchets:** already at the measured floor
  from Task 4 (rate ceiling `0.00995` vs. measured `0.009943`; count ceilings `180` vs.
  measured `180` exactly) — re-confirmed byte-identical this task, no further tightening
  needed; `fresh_missing`/`fresh_wrong` ceilings (5/149) likewise unchanged and
  re-confirmed. **Two review-doc fixes folded in:** (1) `tests/r0-corpus/
  ws-bare-implicit-rec-field/PROOF.md` and the `quote_parity_quoted_var_receiver_resolves_
  as_var` test doc comment previously claimed `"Sales Header Filter"` was merely a naming
  convention echoing a Report dataitem, not an actual one — CORRECTED: it IS a real
  `dataitem("Sales Header Filter"; "Sales Header")` construct (`Report 6175283 "CDO
  Update Output Profile"`, line 15, verified against `CDO_WS`); the fixture only reuses
  the name to exercise the name-agnostic quote-parity mechanism, and real sites like it
  sit honestly unresolved in the 37-site `UntrackedReceiver` residual because Report
  objects are excluded from Step 3a's `Table | TableExtension` gate (sound, not a gap);
  report-dataitem receiver modeling is now documented as a real roadmap lever. (2) Added
  a `sig_fp` stability doc note on `RoutineNodeId` (`src/program/node.rs`): ABI node
  identity is not stable across a fidelity change to the Subtype-reconstruction logic
  (T2's own persistence-audit conclusion) — a future consumer that persists a
  `RoutineNodeId` must version its own cache rather than assume forward/backward
  stability. **DEFERRED (next plan, unchanged from the prior arc's roadmap plus new
  findings this arc):** report-dataitem receivers (`ObjectDecl.report_dataitems` unmodeled
  in `src/program`, ~27+ real CDO sites); dot-quoted field names (e.g. `"No."`, not yet
  covered by any quoted-field arm); unquoted bare field receivers (`MyBlob.
  CreateInStream()`-shaped, deliberately deferred by both T3 and T4); the remaining
  `UntrackedReceiver` non-field residual; honest-taxonomy reclassification of
  `OverloadAmbiguous`=56/`MemberNotFound`=25 into charter §5 sub-states; protected
  `Variables[]` (dependency page/table variables, once var-access modelling exists);
  deeper cross-object chains; risk-weighted centrality reporting (charter §8).
- **Bare implicit-Rec quoted-field receivers + var-lookup quote parity, fail-closed
  (applicability-param-subtype-recfield plan v2.1, Task 4).** CDO primary real-`unknown`
  **1.29% (234) → 0.99% (180)**, `UntrackedReceiver` **91→37 (−54)**, every other
  `unknownByReason` bucket BYTE-IDENTICAL (`CompoundReceiver`=61, `OverloadAmbiguous`=56,
  `BuiltinPrecedenceCollision`=1, `MemberNotFound`=25), `genuine_wrong` stays **0**. Three
  pieces: (1) **Step 2 quote-parity fix** (`infer_receiver_type`,
  `src/program/resolve/receiver.rs`) — the pre-existing var/param/global lookup compared
  the RAW quote-retaining receiver text against `VarDecl`/`Param` names, which are stored
  ALREADY UNQUOTED (the lowerer's `ident_text` strips AL quote characters); a quoted
  identifier naming a real local var could therefore never match and silently fell
  through. Now unquotes (via the existing `unquote_identifier` helper) before comparing,
  gated on the same bare-identifier shape the static-framework-name step already uses.
  MEASURED CDO YIELD ZERO on this corpus (no site in the exhaustive edge-diff resolved via
  this path alone — every flip is the new Step 3a arm below) — framed honestly as
  necessary soundness/precedence plumbing, like the earlier ABI param-Subtype fix,
  verified correct by dedicated unit + r0-corpus fixtures instead. (2) **Step 3a — bare
  implicit-Rec QUOTED-field receiver**: `"Field".X()` with NO `Rec.` prefix, written
  inside a Table/TableExtension's own procedure, means exactly `Rec."Field".X()`.
  Mirrors `resolve_bare`'s Step-3 implicit-Rec precedent for bare CALLS (same strict
  `ObjectKind::Table | TableExtension` guard, same `WithState::NoWithProven` with-guard),
  looking the field up via the SAME visibility-scoped `ResolveIndex::field_in_table`
  surface Task 3's explicit `Rec."Field"` arm consults. Runs only on a Step 2 miss (AL
  scoping: a var/param/global always shadows a field). Quoted-only is deliberate
  documented undercoverage — an unquoted bare field reference is deferred to a future
  task. (3) **Round-2 soundness correction — the routine-shadow guard**
  (`ResolveIndex::table_scope_has_routine`, `src/program/resolve/index.rs`), applied to
  BOTH the new Step 3a arm AND Task 3's existing `Rec."Field".X()` compound arm: AL's
  parens are optional on a zero-argument procedure call (`Rec.Insert;` compiles — the
  Code Cop AA0008 flags the missing parens as a style issue, not a compile error), so a
  bare `Member` AST node (and a bare quoted receiver used as the base of a further call)
  is structurally ambiguous between a field/property access and a parens-less
  procedure-call chain. A same-named routine anywhere in the same visibility-scoped table
  surface now declines field-typing rather than guessing. Measured CDO delta from the
  guard alone: **zero** (confirmed by the exhaustive edge-diff — no Task-3 site regressed).
  **Exhaustive adjudication (not a sample):** a full before/after CDO edge-dump diff
  showed exactly 54 changed route-lines — the SAME 54 sites flipping
  `Unknown(UntrackedReceiver)`→`Catalog`, IDENTICAL `(from, span)` key sets (zero site
  additions/removals/collateral changes): 53 Blob-catalog edges (`Blob::createinstream`/
  `createoutstream`/`hasvalue`, fields spot-verified `Blob` across 11 distinct tables) and
  1 `Text::trim` (Table 6175281 "CDO Setup", a Text[250] field's own `OnValidate`
  trigger). The `Text::trim` site was ALSO `genuine_wrong` against the frozen L3 golden
  until adjudicated: L3's golden misattributes this callee_fp to an unrelated procedure
  (`CheckAzureContainerPerCompany`, called from a DIFFERENT field's `OnValidate` trigger
  8-31 lines away) — the SAME L3 line/routine-key misattribution bug already documented
  for the sibling `CopyStr`/`MaxStrLen` calls on this exact line
  (`known-genuine-divergences.json` entries 39-40); independently re-verified `Text::trim`
  a genuine catalog member and the field genuinely `Text[250]`, added as entry 52
  (`l3_error_intrinsic`) — the independent-verification harness
  (`cdo_genuine_wrong_is_precedence_adjudicated`) gained a new `receiver_kind: "Framework"`
  case (reuses `classify_type_text` — the SAME classifier the resolver itself uses — to
  resolve `catalog_key`'s type prefix, never a bespoke re-implementation).
  **Static var-extraction audit** (round-2 addendum, required before landing): confirmed
  via the tree-sitter-al grammar that AL has NO block-scoped variable declarations (a
  `var_section` only ever appears in a procedure/trigger's own preamble, never nested
  inside `if`/`while`/`repeat`/`case`/`for` — grammar-verified, not merely assumed) — the
  brief's named concern ("locals in repeat/while/if/case/for blocks") is structurally a
  non-issue. Found (and documented as orthogonal, not a blocker): whole-body preprocessor-
  split routines (`preproc_split_procedure`/`preproc_split_procedure_preamble`/procedures
  using `preproc_split_procedure_body`/`preproc_split_complete_body`) are either entirely
  unindexed as routines or indexed with `body: None` — a PRE-EXISTING, symmetric coverage
  gap (zero call-site obligations extracted either way) with no false-`Source` risk, since
  a routine with no obligations can never have a receiver mis-typed. Fixtures:
  `tests/r0-corpus/ws-bare-implicit-rec-field/` (2 positive Blob/Text bare-field
  procedures + TableExtension own/base-field folding + var-shadows-field quote-parity +
  routine-shadow-declines + non-Table-scope negative + unknown-field negative) + unit
  fixtures in `receiver.rs` (Step 2 quote parity, Step 3a positive/negative/with-guard/
  bare_ctx-optionality, routine-shadow for both arms) + `ResolveIndex::table_scope_has_
  routine` unit fixtures (base/extension/out-of-closure/absent) in `index.rs`. Ratchets
  tightened (dated 2026-07-03): primary rate ceiling 0.01293→0.00995, primary/whole
  `unknown` count ceilings 234→180, `fresh_missing` ceiling 10→5 (measured 3); `fresh_wrong`
  ceiling unchanged at 149 (re-confirmed byte-identical — the new divergence is overlaid
  before the diff runs).
- **`--graphify-export` edges carry `unknown_reason`.** For an `unknown`-obligation
  edge, the export now emits its first unresolved route's diagnostic reason
  (`compoundReceiver`, `catalogMiss`, `memberNotFound`, …) via `UnknownReason::as_str`,
  so the BC-Brain consumption layer can surface the "why" behind each unresolved edge,
  not merely that it is unknown. Additive and `skip_serializing_if` None on every
  non-unknown edge — existing goldens unaffected.
- **Table-field type index + `Rec."Field".X()` record-field chains + EnumType chain
  base, fail-closed (applicability-param-subtype-recfield plan v2.1, Task 3).** The
  largest single-task real-`unknown` drop since the arc began: CDO primary
  real-`unknown` **1.75% (317) → 1.29% (234)**, `CompoundReceiver` **144→61 (−83)**,
  every other `unknownByReason` bucket BYTE-IDENTICAL (`UntrackedReceiver`=91,
  `OverloadAmbiguous`=56, `BuiltinPrecedenceCollision`=1, `MemberNotFound`=25),
  `genuine_wrong` stays **0**. Four pieces: (1) **`FieldNode{name_lc, type_text}` on
  `ObjectNode`** (`src/program/node_extract.rs`) — the table-field surface, populated
  from source `FieldDecl` (`extract_nodes`; `FieldDecl` previously had zero consumers
  under `src/`) AND from ABI `AbiTable`/`AbiField` (`abi_ingest::ingest_abi` via the
  new `abi_table_fields` — consumes Task 2's Subtype-qualified `parse_field`, so an
  ABI Enum field carries `Enum "X"`). The type is carried as RAW DECLARED TEXT and
  classified ONLY at the consumer via the same `classify_type_text` every declared
  type goes through — never `FieldDecl::is_blob_like` (which also flags
  Media/MediaSet and would falsely broaden a Media field into the Blob catalog).
  (2) **`ResolveIndex::field_in_table`** (`src/program/resolve/index.rs`) —
  VISIBILITY-SCOPED field lookup: base-table fields + only `TableExtension` fields
  inside the referencing object's dependency closure (the same closure discipline
  `resolve_in_table_scope` applies to routines; an out-of-closure extension field
  never resolves), UNIQUE match or `None`, with identical `(object, name, type)`
  declarations deduped by provenance BEFORE the duplicate check (a `#if`/`#else`
  re-parse duplicate never manufactures artificial ambiguity) and every real
  duplicate-decline logged (`log::debug!`, object + field name). (3) **The
  non-method `Member{object, member}` record-field arm** in
  `infer_compound_member_receiver` (`src/program/resolve/receiver.rs`): `!is_method`
  + base types `Record{table: Some}` → `field_in_table` → `classify_type_text` →
  `parsed_type_to_receiver` — handles BOTH quoted (`Rec."Error Message"`) and
  unquoted (`Rec.BlobField`) member names, and ANY `Record`-typed base variable
  (`DOFile."File Blob".X()`), not only literal `Rec`; all declines fall through to
  honest `Unknown`. (4) **EnumType-as-chain-base** (`enum_chain_return_kind`,
  `src/program/resolve/framework_returns.rs`): `Ordinals()`/`Names()` on an Enum
  VALUE receiver → `Framework(List)` (MS Learn methods-auto/enum, fetched
  2026-07-02: both return `List of [...]`; `AsInteger`/`FromInteger` deliberately
  excluded — primitive/Enum returns, nothing to chain), enabling the multi-level
  `Rec."eSeal Service".Ordinals().Count()`. **Exhaustive adjudication (not a
  sample):** a full before/after CDO edge-dump diff showed exactly 83 changed
  route-lines — the SAME 83 sites flipping `Unknown(CompoundReceiver)`→`Catalog`,
  zero site additions/removals, zero collateral changes: 68 Blob-catalog edges
  (every field verified `Blob` in its declaring table's real source), 7
  `Enum::asinteger` (5 distinct verified Enum fields), 1 `Enum::ordinals` + 1
  `List::count` (the multi-level chain, field verified `Enum CDOESealService`), 5
  `Media::hasvalue` (`"Media Reference"; Media` on the PLATFORM ABI table "Media
  Resources" — verified from the Microsoft System .app's SymbolReference.json,
  proving the ABI-tier field path live AND classify-strict: Media routes to the
  MEDIA catalog, never falsely Blob), 1 `Text::contains` (`"Additional
  Information"; Text[250]`, verified from Base App embedded source). Fixtures:
  `tests/r0-corpus/ws-record-field-chain/` (3 positives incl. the multi-level Enum
  chain + TableExtension folding; 5 fail-closed negatives: unknown field,
  scalar-typed field, duplicate field across base+extension, Page receiver with a
  quoted member, local-var-shadows-field-name) + `field_in_table` unit fixtures
  (visibility/out-of-closure/duplicate/provenance-dedupe) + ABI ingestion unit
  fixtures + `enum_chain_return_kind` table tests. The prior
  `ws-compound-framework` fixture (j) (`Rec.BlobField.CreateOutStream()`,
  previously a DEFERRED-shape negative) now correctly resolves
  `Blob::createoutstream` — rebaselined as a positive with its history documented.
  Ratchets tightened (dated 2026-07-03): primary rate ceiling 0.01751→0.01293,
  primary/whole `unknown` count ceilings 317→234. Found-and-documented (out of
  scope, `ws-record-field-chain/PROOF.md`): a pre-existing, Task-3-independent gap
  where a QUOTED bare identifier referencing a local variable never matches Step
  2's variable lookup (quote-parity asymmetry) — noted for a future task.
- **cross-object chains + protected-ABI plan v2.1, Task 5 (FINAL): re-measure,
  exhaustive-adjudication sign-off, ratchet finalization — arc capstone**
  (`tests/program_resolve_harness.rs`). Closes the plan Task 1 opened. Full re-measure
  on CDO (`CDO_WS`, `ENFORCE_CDO_WS=1`, single tests, `--release`): primary/whole
  `unknown`=317, `real_unknown_rate`=1.75% (raw 317/18104=0.017510), `genuine_wrong`=0,
  `fresh_missing`=4, `fresh_wrong`=149 (all `fresh_ahead_dispatch`),
  `unknownByReason`={CompoundReceiver: 144, UntrackedReceiver: 91, OverloadAmbiguous: 56,
  BuiltinPrecedenceCollision: 1, MemberNotFound: 25} (sum=317=`unknown`, verified both
  primary and whole scopes). **Net across the whole plan: 1.82%(329)→1.75%(317), −12
  count / −0.07pp, `genuine_wrong` stays 0 through every task.** Trajectory: Task 1
  protected-ABI soundness fix — CDO-DORMANT (its only true SymbolOnly unit exposes zero
  public routines; both metric gates byte-identical 1.82%/329), proven exclusively by 9
  new in-repo fixtures against a real no-embedded-source probe `.app`; Task 2 structured
  ABI return-type plumbing — resolution-NEUTRAL by construction (nothing consumed
  `RoutineNode.return_type` yet), byte-identical 1.82%/329; Task 3 cross-object
  call-result chains (`Var.Method().X()` via a pure `resolve_member` type-query) —
  329→327 (`CompoundReceiver` 156→154, −2), 1.82%→1.81%, plus a same-task review fix
  (collapsed-ABI-overload-survivor decline) that stayed byte-identical (327/1.81%,
  0 collapse-marked routines in the whole CDO graph); Task 4 Xml + `RecordRef`-family
  typed-return tables plus the HTTPCONTENT-catalog-was-never-stale course-correction and
  a genuine pre-existing Step-4 fail-open bugfix — 327→317 (`CompoundReceiver` 154→144,
  −10), 1.81%→1.75%. **Exhaustive-adjudication sign-off (re-confirmed, not re-sampled):**
  Task 3's 2 newly-resolved edges (`Codeunit 6175364 "CDO Universign E-Seal Service"`'s
  `ProcessSealResponse`, `Response.GetContent().AsText()`/`.AsBlob()`) and Task 4's 10
  newly-`Catalog` edges (4 `RecordRef.Field(n).<Leaf>()`, 1
  `RecordRef.KeyIndex(1).FieldIndex(1)`, 5 `Node.AsXmlElement().<Add|GetChildNodes>()`)
  were each hand-adjudicated against real CDO/System-Application source during their own
  task (see `.superpowers/sdd/task-3-report.md` §6 and `task-4-report.md`'s edge table) —
  2+10=12 equals the exact `CompoundReceiver` bucket drop (156→144) and the exact
  `unknown` count drop (329→317); no edge unaccounted for. Both tasks' methodology dumped
  and diffed the FULL (Task 3) or provably-exhaustive-for-the-touched-code (Task 4, the
  4 new-table BuiltinId prefixes — no other code path could possibly have changed)
  before/after edge set, so a changed TARGET/EVIDENCE on a pre-existing edge would have
  surfaced as a removed+added pair, not just a net-new addition; both diffs showed only
  additions, zero removals. **Protected-ABI dependency check:** none of the 12
  adjudicated edges depends on a mislabeled-protected ABI member — impossible by
  construction, not merely by inspection: Task 3's 2 edges resolve through the System
  Application's `EmbeddedSource` tier (not SymbolOnly at all) via `resolve_in_object`'s
  uniform per-candidate-visibility discipline (identical for every tier since Task 1);
  Task 4's 10 edges resolve entirely through the compiled-in `Catalog`/builtin dispatch
  tables (`framework_returns.rs`/`recordref_returns.rs` → `member_catalog.rs`), which
  never reads `AbiRoutine`/`Access` data at all. **Ratchet finalization:**
  `real_unknown_rate` ceiling tightened `0.0176`→`0.01751` (a 5-decimal margin above the
  exact raw measured value 0.017510, since the 4-decimal `0.0175` display value alone
  sits BELOW the true raw rate and would spuriously trip); primary/whole `unknown` COUNT
  ceilings tightened `320`→`317` (exact measured floor, no margin needed for an integer
  count). `fresh_wrong`/`fresh_missing` ceilings (149 exact / 10 with margin over
  measured 4) are UNCHANGED — neither moved across T3/T4, kept per the plan's own
  "keep, don't re-tighten what this plan didn't touch" scope. **Pre-existing-failure
  investigation:** `fan_out_applicability_zero_violations` and
  `route_applicability_zero_violations` (both `EventFlow soundness violated on CDO_WS`,
  `event=200` vs expected `0`) were flagged during Task 3/4 as failing before this plan
  started. Probed via a clean `git checkout` of master's base commit `8a484d4` (working
  tree was fully clean of tracked changes) + a full release rebuild + an
  `ENFORCE_CDO_WS=1` re-run: **both tests fail identically on master** (same
  `event=200`/`0` assertion, same panic site) — confirmed PRE-EXISTING, unrelated to this
  plan (no event-flow/fan-out code touched by any of Tasks 1-5), likely graphify-era per
  the Task-3 report's own hypothesis. Documented here as known-broken-on-master, left
  open for a future plan to root-cause; NOT a regression introduced by this work.
  **DEFERRED (next plan, see the plan doc's "Roadmap — beyond this plan" section):**
  record-field chains (`Rec."Field".X()` — needs a table-field type index on
  `ObjectNode`, `FieldDecl` already parsed with zero consumers); `UntrackedReceiver`=91;
  honest-taxonomy reclassification of `OverloadAmbiguous`=56/`MemberNotFound`=25 into
  charter §5 sub-states; the ABI param-fingerprint `Subtype` degradation
  (`param_type_fp`/`AbiParameter`, incl. recovering the collapse-marked safe subset once
  fixed); protected `Variables[]` (dependency page/table variables, relevant once
  var-access modelling exists); the two pre-existing `fan_out`/`route_applicability` CDO
  failures documented above.
- **Xml framework chains + a NEW `RecordRef`/`FieldRef`/`KeyRef` typed-return table
  (chain-tables plan, Task 4).** `src/program/resolve/framework_returns.rs`: `Xml`
  entries added to `framework_return_kind` — `XmlElement.Create(...)` (arities 1-4),
  the full symmetric `AsXmlXxx()` zero-arg conversion family (`AsXmlNode`/
  `AsXmlElement`/`AsXmlText`/…), and `GetChildNodes()` — every entry keyed
  `(kind, member_lc, is_method, arity)` with per-entry MS-Learn provenance, uncertain
  arities/members OMITTED. New module `src/program/resolve/recordref_returns.rs`
  adds `recordref_family_return_kind`, a DISTINCT `(RecordRefFamilyKind, member_lc,
  is_method, arity) -> Option<RecordRefFamilyKind>` table for the `RecordRef`/
  `FieldRef`/`KeyRef` unit-variant family (`Field`/`FieldIndex` -> `FieldRef`,
  `KeyIndex` -> `KeyRef`, `KeyRef.FieldIndex` -> `FieldRef`) — same fail-closed,
  table-miss-declines contract as `framework_return_kind`. Deliberately excludes
  scalar accessors (`FieldCount`/`KeyCount`, which return `Integer`) and
  `FieldRef.Value` (variant-like LEAF data, never chainable — a chained `.X()` off it
  stays `Unknown`), plus the validated-but-out-of-scope `FieldRef.Record()`/
  `KeyRef.Record()`. `src/program/resolve/receiver.rs`: `infer_compound_member_receiver`
  gains the matching `ReceiverType::{RecordRef,FieldRef,KeyRef}` arm (same
  immediate-decline-on-table-miss mechanism as the `Framework` arm). Also fixes a
  genuine PRE-EXISTING fail-open bug found while grounding this task's fixtures
  against real CDO source: `infer_receiver_type`'s Step 4 called `classify_type_text`
  on the RAW receiver text unconditionally, and its `Xml` arm is the only
  prefix-wildcard match (`s.starts_with("xml")`) in an otherwise all-exact-match
  function — a COMPOUND receiver whose full text happened to start with `"xml"`
  (e.g. the outer `.AsXmlNode()` call's receiver in `XmlElement.Create('root').
  AsXmlNode()`) would short-circuit to `Framework(Xml)` at Step 4, bypassing the real
  per-hop chain-typing entirely. Fixed by gating Step 4 to genuine bare identifiers
  (`!receiver_lc.contains('.') && !receiver_lc.contains('(')`), matching the step's
  own documented "bare identifier" intent. 22 new fixtures (14 fixture-based + 8
  table-level unit tests) over `tests/r0-corpus/ws-chain-tables/` cover 6 positives
  and 8 negatives (un-tabled member, wrong form, wrong arity, same-named member on a
  non-family receiver, the `FieldRef.Value` chain-decline, an unvalidated/omitted
  entry, and an HTTPCONTENT regression pin — see below). CDO gate: `CompoundReceiver`
  154→144 (-10), primary/whole `unknown` 327→317, `real_unknown_rate` 1.81%→1.75%;
  all 10 newly-resolved edges EXHAUSTIVELY hand-adjudicated correct via a full
  before/after edge-dump diff (not a sample); `genuine_wrong` stays 0.
- **Investigation finding, NOT implemented (course correction on this task's original
  brief): the `HttpContent` framework catalog was never stale.** The brief called for
  adding `AsText`/`AsBlob`/`AsInStream`/`AsJson*` to `member_catalog.rs`'s `HTTPCONTENT`
  set. Verified against BOTH live `methods-auto/httpcontent` (Microsoft Learn) and this
  project's own SymbolReference-generated `member_builtins.json`
  (`ms-dynamics-smb.al-18.0.2293710`): the platform `HttpContent` VALUE TYPE has
  exactly `Clear`/`GetHeaders`/`IsSecretContent`/`ReadAs`/`WriteFrom` — a byte-for-byte
  match with the existing catalog. The methods named `AsText`/`AsBlob`/`AsInStream` are
  real, but belong to the UNRELATED System Application `Codeunit "Http Content"`
  (`System.RestClient`), resolved via ordinary object/procedure resolution, not the
  framework catalog; its one real CDO call site was already resolved by the prior plan
  v2.1 Task 3 cross-object-chain fix. Adding those members to the framework catalog
  would have been a fabricated entry that could never fire correctly — not implemented.
  `tests/r0-corpus/ws-chain-tables/src/CTCaller.Codeunit.al`'s
  `TestHttpContentAsTextStaysUnknown` regression-pins the correct (declined) behavior;
  full writeup in `tests/r0-corpus/ws-chain-tables/PROOF.md`.

### Fixed
- **Collapse-marker guard now covers every route-construction site, not just plain
  dispatch — Run/trigger/event paths decline marked ABI survivors
  (applicability-param-subtype-recfield plan v2.1, Task 2 review fix).** Task 2's own
  plain-dispatch marker guard (`resolve_in_object`'s single-visible-candidate arm)
  documented itself as "the SINGLE choke point every plain-call AND qualified-member
  dispatch path funnels through" — a factually wrong claim (corrected in this fix): FOUR
  other `make_routine_route` call sites in `src/program/resolve/resolver.rs` look up a
  routine directly by ROLE (entry trigger / trigger name / subscriber match) rather than
  through that name+arity SELECTION boundary, so a collapse-marked survivor (a dedup
  collapse of ≥2 raw ABI entries that fingerprint-collided — see
  `build::dedup_routines_preserving_genuine_overloads`) could still reach a confident
  `Opaque` route through any of them, unguarded: (1) `resolve_object_run`
  (`Codeunit.Run`/`Page.RunModal`/`Report.Run`'s entry-trigger dispatch); (2)
  `resolve_member`'s own inline `Codeunit.Run(arity<=1)` special case; (3)
  `resolve_implicit_trigger`'s base-table + `TableExtension` trigger fan-out; (4)
  `emit_event_flow_edges`'s subscriber fan-out. Added a single shared helper,
  `routine_is_collapse_marked`, and applied it at all four sites (replacing the
  duplicated inline lookup inside `resolve_in_object` too, now a thin caller of the same
  helper): sites (1)-(3) decline to `Unresolved`/`Unknown(OverloadAmbiguous)` in place of
  the marked candidate's route (site (3)'s Multicast fan-out keeps the route SLOT as an
  honest Unknown rather than silently shrinking the set's cardinality); site (4) instead
  SKIPS the marked subscriber's route entirely — its `SetCompleteness::
  Partial{ReverseDependentSubscribers}` is already open-world, so omitting one
  untrustworthy candidate doesn't understate an otherwise-closed count the way (3)'s
  fan-out would. Corrected the "SINGLE choke point" doc claim to honestly enumerate the
  five guarded sites instead. One new end-to-end fixture (`tests/r0-corpus/
  ws-cross-object-chain`): extended the existing N11 probe `.app` (`Dep Run Collapse`,
  object 60105) with a LITERALLY DUPLICATED raw `OnRun` JSON entry (0-arg —
  `param_type_fp` folds to the fixed `0` for an empty `Parameters[]`), reachable via
  `Codeunit.Run(Codeunit::"Dep Run Collapse")` — proves site (1) declines rather than
  resolving the arbitrary duplicate survivor confidently (written failing first, verified
  red against the pre-fix code, then green). 8 new resolver-level unit tests (marked +
  unmarked control pairs for all four sites) round out the coverage the review specifically
  asked for. CDO: both gates byte-identical to the pre-existing baseline
  (`real_unknown_rate`=1.75%/317, `genuine_wrong`=0, `fresh_missing`=4, `fresh_wrong`=149
  all `fresh_ahead_dispatch`) — every CDO dependency is `EmbeddedSource`, structurally
  never `SymbolOnly`, so `abi_overload_collapsed` is never set there and all four newly
  guarded sites are dormant on CDO by construction, exactly like the original Task 2
  guard.
- **Event-applicability checker fix — the pre-existing `event_violations=200` broken gate
  root-caused and closed (applicability-param-subtype-recfield plan v2.1, Task 1).**
  `verify_event_subscriber_route`'s strict arity invariant (`differential.rs`) predated
  `ae35e90`'s Sender-tolerant `+1` wiring (`index.rs`) — the checker still flagged every
  route the wiring had just correctly started admitting (the 200 `event_violations` on
  master were EXACTLY the +200 `IncludeSender` subscribers `ae35e90` wired). Root cause:
  `ae35e90`'s wiring applied a BLANKET `+1` to every `[IntegrationEvent]`/
  `[BusinessEvent]`/`[InternalEvent]` publisher regardless of whether the publisher
  actually declared `IncludeSender: true` — a synchronized-wrongness risk (the `+1` is
  only legal AL when the attribute says so). Fix: ground-truthed (Microsoft Learn,
  2026-07-02) that ALL THREE publisher attributes carry `IncludeSender` as their FIRST
  positional arg (`[IntegrationEvent(IncludeSender, GlobalVarAccess[, Isolated])]`,
  `[BusinessEvent(IncludeSender[, Isolated])]`, `[InternalEvent(IncludeSender[,
  Isolated])]`) — previously unparsed anywhere in the codebase (only `Isolated` was read).
  Added `RoutineNode::include_sender: Option<bool>` (tri-state; single source of truth),
  populated at ingestion: source via `event::publisher_include_sender` (reads the raw IR
  attribute arg); ABI via `abi_ingest::abi_publisher_include_sender` (reads the
  `SymbolReference.json` structured attribute arg) — a 13,581-entry probe of a real
  Microsoft Base Application `SymbolReference.json` (`Codeunits` + every nested
  `Namespaces[]` level) found 100% coverage, zero unparseable entries, so ABI-tier is
  `Some` in practice exactly like source. Added ONE shared helper,
  `event::subscriber_arity_bound(publisher_params_count, include_sender)` — `+1` ONLY
  when `include_sender == Some(true)`, `None`/`Some(false)` both mean no tolerance
  (fail-closed) — consumed by BOTH `index.rs`'s wiring and
  `differential::verify_event_subscriber_route`'s independent checker, so the two can
  never drift again. `route_applicability_zero_violations` (Test 15)'s panic message now
  prints all six `ApplicabilityReport::is_clean()` counters (previously only
  `witness_contract_violations`/`abi_unmapped` — a genuine observability gap that hid
  which family actually failed). Residual (documented, not closed): Sender param-TYPE
  compatibility is not validated, arity-only. CDO: `event_violations` 200→0 on both
  gates; `cdo_full_program_coverage_and_self_reported_metric` +
  `cdo_l3_semantic_audit_no_fresh_wrong` byte-identical to the pre-existing baseline
  (`real_unknown_rate`=1.75%/317, `genuine_wrong`=0, `fresh_missing`=4, `fresh_wrong`=149)
  — confirms the 200 were exactly the `ae35e90` IncludeSender-true population, zero
  non-IncludeSender over-wired routes existed to correct. Full CDO harness 128/128 (was
  126/128 on master). 6 new regression units (2 wiring-level in `index.rs`, 2
  checker-level in `semantic_golden.rs`, plus `event.rs`'s ingestion-level parsing +
  bound-arithmetic units) prove BOTH directions: IncludeSender=true admits the `+1`;
  IncludeSender=false/unknown rejects it.
- Stale comment in `src/program/abi_ingest.rs` (`param_sig_key`'s "no content key
  needed" rationale) corrected — it contradicted `build::
  dedup_routines_preserving_genuine_overloads`'s `abi_overload_collapsed` marking
  logic in the same codebase, which exists precisely because a same-`RoutineNodeId`
  ABI run is NOT always a true duplicate (`param_type_fp` degrades a parameter's type
  to its outer keyword only, so two distinct overloads differing by Subtype can share
  both the id and the empty `param_sig_key`).
- **ABI param/field Subtype fidelity — genuine overloads un-collapse and decline
  honestly; plain-dispatch collapse-marker guard closes a latent false-`Opaque` class
  (applicability-param-subtype-recfield plan v2.1, Task 2).** `parse_method`'s param
  mapping (`src/engine/deps/symbol_reference.rs`) took only `RawTypeDef.name`,
  degrading every object-typed parameter to its bare outer keyword (`"Codeunit"`) and
  silently dropping its `Subtype` — the same root cause as the already-fixed
  return-type gap, but for params. Added `reconstruct_param_field_type` — a NEW
  generalized helper (deliberately NOT `reconstruct_return_type_text`, which has a
  DIFFERENT fail-closed contract: decline-to-`None`) reused by both `parse_method`
  (params) and `parse_field` (fields): reconstructs FULL source-shaped text
  (`Codeunit "Dep A"`) when `Subtype.Name` is quote-free; on the DECLINE shapes
  (Id-only Subtype; a Subtype Name containing `"`) falls back to the BARE OUTER NAME
  for TEXT (never empty — `param_type_fp`/dedup have no "empty = untrustworthy"
  contract, unlike returns), while additionally carrying the RAW discriminator
  (`AbiParameter::subtype_id`/`subtype_raw_name`/`subtype_tag`) so the TEXT fallback
  never loses distinguishing power. `abi_ingest::param_type_fp` now folds a
  length-delimited canonical tuple (outer kind + subtype id + raw subtype name + a
  degradation tag) per parameter via the project's stable FNV-1a primitive (never
  `DefaultHasher`) — closing the round-1 critical sliver: two DIFFERENT Id-only
  Subtypes (`DoIt(Codeunit 10)` vs `DoIt(Codeunit 20)`) sharing an identical
  bare-fallback TEXT now fingerprint DIFFERENTLY and never silently collapse onto one
  ABI overload survivor; they instead correctly decline `OverloadAmbiguous` at
  dispatch as two live, un-collapsed candidates. An ABI Enum FIELD now correctly
  carries `Enum "X"` instead of the bare `"Enum"` this dropped before (`parse_field`,
  same helper). **Plain-dispatch marker guard (round-1 critical, defense in depth):**
  `resolve_in_object`'s single-visible-candidate arm — the FINAL candidate-selection
  boundary every bare-call AND qualified-member dispatch path in the module funnels
  through — now declines `OverloadAmbiguous` when the sole surviving candidate is
  `RoutineNode::abi_overload_collapsed`. Previously the marker gated ONLY the
  cross-object chain type-query boundary (`routine_node_for_type_query`); a marked
  survivor could still resolve CONFIDENTLY via ordinary PLAIN dispatch (e.g.
  `DepCollapse.Get(X)` called directly, never chained onward) — an unguarded
  false-`Source`/`Opaque` vector this closes. `RoutineNode::param_sig_key` stays
  hardcoded empty for ABI routines (unaffected; safe by construction post-fix — see
  the updated doc on `dedup_routines_preserving_genuine_overloads`). **sig_fp
  persistence audit:** grepped for `RoutineNodeId`/`AbiRoutineKey`/`sig_fp`/
  `param_type_fp` serialization across caches/incremental artifacts/CI baselines —
  none found; documented that ABI node identity is not stable across fidelity
  changes (expected, no version-bump needed). **Fold-in (T1 review):** added the
  preflight diagnostic T1 spec'd but never landed —
  `index::count_unknown_include_sender_plus1_subscribers` counts event-subscriber
  routines sitting at exactly `publisher_arity + 1` whose resolved publisher's
  `IncludeSender` is UNKNOWN (the population the fail-closed no-`+1`-without-evidence
  policy silently orphans); a new CDO gate
  (`cdo_unknown_include_sender_plus1_subscribers_preflight_is_zero`) asserts `0`.
  **CDO: byte-identical (1.75%/317, `genuine_wrong`=0, 0 `abi_overload_collapsed`
  before AND after — all CDO deps are `EmbeddedSource`, so this fix is dormant on CDO
  by construction; the fixed N11 probe-`.app` pair (`tests/r0-corpus/
  ws-cross-object-chain`) now ingests as two DISTINCT, un-collapsed candidates that
  decline `OverloadAmbiguous` at PLAIN dispatch on the INNER `Get(Helper)` call
  itself — pre-fix that call silently resolved `Opaque` to an arbitrary survivor and
  only the OUTER `.ReadAs()` chain call declined, via the separate chain guard).**

- **Cross-object call-result chains: `Var.Method().X()` now resolves via a PURE
  `resolve_member` type-query on the base's static type, fail-closed (cross-object chains +
  protected-ABI plan v2.1, Task 3).** `src/program/resolve/receiver.rs`:
  `infer_compound_member_receiver` gains a new arm — strictly the procedure-CALL form
  (`ExprKind::Call{function: Member{base, member}, ..}`; a bare `Member` property/field
  access is never this arm). When `base` types (via the existing AST-native recursive
  `infer_receiver_type_for_expr`) to `Object`/`Record`/`SelfObject`/`Interface`, the base
  call's return type is typed by calling `resolve_member(base_ty, member_lc, arity, ..)` as
  a TYPE-QUERY — the SAME dispatch arity the caller uses (never a second `args.len()`
  model). Guard: EXACTLY ONE route (an `Interface` base fans out to every implementer —
  more than one is a genuinely polymorphic prefix, conservative decline, never a guessed
  pick); a route with no routine identity (`Unresolved`/`Builtin`) also declines. The
  resolved routine's declared `return_type` (Task 2's plumbing, now consumed for the first
  time) is parsed via `classify_type_text` → `parsed_type_to_receiver`, WITH Task 2's
  Name+Id cross-validation applied whenever the return type carries a structured ABI
  `Subtype` pair — the object the Name resolves to must ALSO carry that declared Id, or the
  whole chain declines rather than trust a name-only match. `src/program/resolve/
  resolver.rs`: new `pub(crate) routine_node_for_type_query` reads the `RoutineNode` behind
  a route's target regardless of shape — `RouteTarget::Routine` direct via
  `binary_search_by`; `RouteTarget::AbiSymbol` via the ABI-PREFIX UNIQUENESS GUARD
  (`resolve_abi_prefix_routine`): reconstructs the declaring `ObjectNodeId` from the
  `AbiRoutineKey`, then requires the SAME arity matcher + per-candidate visibility
  (`routine_candidate_is_visible`) `resolve_in_object` uses to find EXACTLY ONE surviving
  candidate — same-name/same-arity siblings decline (ABI parameter types are degraded, no
  `Subtype` carried on parameters, so two genuinely different overloads can share an arity
  without the engine being able to disambiguate). Single-implementer interface prefixes
  prefer the interface's OWN declared method signature when the graph models one
  (`interface_own_routine_node`) over the resolved implementer's, since AL guarantees they
  match exactly. 15 new end-to-end fixtures over a real `.al` + two real SymbolOnly probe
  `.app`s (`tests/r0-corpus/ws-cross-object-chain/`) cover: a SOURCE prefix, an ABI prefix
  carrying a nested `Subtype` (leaf resolves + a NEGATIVE internal-leaf-not-visible
  sibling), a single-implementer interface prefix positive, and 11 fail-closed negatives
  (polymorphic interface prefix, builtin-only prefix, wrong-arity source/ABI prefix, ABI
  same-name overloads with different returns, scalar/no return type, cross-app-ambiguous
  return, Name+Id mismatch, the deferred record-field/property form, and a 3-level chain
  whose middle hop fails to type). CDO: primary/whole `unknown` 329→327 (`CompoundReceiver`
  156→154, every other bucket byte-identical), `real_unknown_rate` 1.82%→1.81%,
  `genuine_wrong` stays 0 — both newly-resolved sites exhaustively hand-adjudicated correct
  against the Microsoft System Application's real embedded source
  (`Codeunit 6175364 "CDO Universign E-Seal Service"`'s `ProcessSealResponse`:
  `Response.GetContent().AsText()`/`.AsBlob()` where `Response: Codeunit "Http Response
  Message"` declares `GetContent(): Codeunit "Http Content"`, which declares
  `AsText(): Text`/`AsBlob(): Codeunit "Temp Blob"`) — the exact real-world idiom this task
  targets.
- **Structured ABI return types: `Subtype` is now parsed from `SymbolReference.json` and
  reconstructed into source-shaped `RoutineNode.return_type` text — resolution-neutral
  enabling plumbing for Task 3's cross-object call-result chains (cross-object chains +
  protected-ABI plan v2.1, Task 2).** `src/engine/deps/symbol_reference.rs`: `RawTypeDef`
  gains a nested `subtype: Option<RawSubtype { name, id }>` (serde-renamed to the JSON keys
  `Subtype`/`Name`/`Id`); a new `reconstruct_return_type_text` fail-closed rule set turns
  `{"Name":"Codeunit","Subtype":{"Name":"Http Content","Id":2354}}` into the quoted
  source-shaped `Codeunit "Http Content"` (Name-preferred), a bare `{"Name":"HttpHeaders"}`
  (no `Subtype`) passes through unchanged, and — critically — an **Id-only Subtype (no
  Name) declines to `None`**: AL object ids are not cross-app unique, so a bare numeric
  reconstruction could resolve to the wrong app's object. A `Subtype.Name` containing a `"`
  also declines to `None` (never escaped/synthesized — a downstream text classifier must
  never see manufactured escaping), and a namespace/dot-qualified name or a generic/
  container return (`List of [...]`) is always carried verbatim or declined, never
  truncated or approximated. `AbiRoutine`/`RoutineNode` additionally carry the raw
  `(name, id)` pair (`return_type_id`) whenever BOTH are present in the JSON — independent
  of the text landmines above (it is a structured identity comparison, never text
  synthesis) — so Task 3 can cross-validate: when a return type's Subtype declares both a
  Name and an Id, the object the Name resolves to must ALSO carry that Id before a
  cross-object chain hop trusts it. `src/program/abi_ingest.rs`: `RoutineNode.return_type`/
  `return_type_id` are now populated from this reconstruction (replacing the prior
  deliberate `None` hard-set); `RoutineNode.return_type` stays non-serialized. **Nothing
  consumes `RoutineNode.return_type` for an ABI-tier routine yet (Task 3 does)** — CDO's
  self-reported metric stays BYTE-IDENTICAL (1.82% / `unknown=329`, `genuine_wrong=0`).
  Fold-in from Task 1's review: `routine_candidate_is_visible` now DELEGATES to
  `object_access_visible_from` instead of duplicating its per-`Access` rule (one predicate,
  no drift vector), and a new fixture
  (`bare_extension_base_symbolonly_wrong_arity_existence_never_leaks_into_emission`) proves
  the SymbolOnly existence boolean's arity-deferred `true` never leaks into a false emission
  when the ONLY caller of that boolean (`resolve_bare` Step 2 / `resolve_in_table_scope`)
  hands off to `resolve_in_object`'s own arity-exact selection — the genuine boundary case
  Task 1's fixture (g) exercised via a different (Object-receiver) dispatch path and missed.
  **Known follow-up (out of scope for this task):** `abi_ingest::param_type_fp` (parameter
  signature fingerprinting) still hashes only the bare `TypeDefinition.Name`, not a
  `Subtype`-reconstructed shape — a sibling gap to the return-type reconstruction here,
  left for whenever parameter-type ABI fidelity is prioritized.

### Fixed
- **Chain-typing now declines on collapsed ABI overload survivors — a dedup-collapse marker
  makes the silent same-`RoutineNodeId` ABI fold visible, fail-closed (cross-object chains +
  protected-ABI plan v2.1, Task 3 review fix).** The blocking review finding:
  `abi_ingest.rs` hardcodes ABI `RoutineNode.param_sig_key = String::new()`, so
  `build::dedup_routines_preserving_genuine_overloads` (which de-dupes a same-id run by
  `param_sig_key`) SILENTLY collapsed any second same-name/same-arity/same-outer-param-kind
  ABI overload to an arbitrary first survivor — and `param_type_fp` fingerprints only a
  parameter's OUTER type keyword (never its `Subtype`), so two genuinely different overloads
  (`Get(X: Codeunit A)` vs `Get(X: Codeunit B)`) hash-collide onto ONE `RoutineNodeId`.
  Task 3's chain arm reads the survivor's `return_type` — if a collapsed sibling had a
  different object-typed return, that mis-types the chain receiver → potential false
  `Source` (the cardinal sin). 77 such collapsed pairs exist in CDO's real dependency ABIs
  (3 in Microsoft Base App also differing in RETURN type); previously unmanifested only
  because the observed differing returns were scalar/None (the scalar-decline saved it
  incidentally). Fix, narrowly scoped and fail-closed (no param-`Subtype` modeling — that is
  a scheduled follow-up): (1) new non-serialized `RoutineNode.abi_overload_collapsed` marker,
  set by `dedup_routines_preserving_genuine_overloads` EXACTLY when ≥2 raw
  `TrustTier::SymbolOnly` entries shared one node id (SOURCE routines are never marked —
  their `param_sig_key` is real parsed content, so a same-key collapse there is always a
  true re-parse duplicate); (2) `resolver::routine_node_for_type_query` (the single choke
  point both `RouteTarget::Routine` and `AbiSymbol` type-query arms funnel through) and
  `receiver::receiver_from_routine_node` (also covering the `interface_own_routine_node`
  path) DECLINE when the resolved prefix routine is collapse-marked — the return type is
  untrustworthy by construction; (3) corrected the stale `resolve_in_object` comment claiming
  dedup "preserves every raw entry in that genuine collision" (now known false for ABI
  routines); (4) extended the `ws-cross-object-chain` probe `.app` with a new `Dep Collapse`
  codeunit declaring two `Get` overloads differing ONLY in param `Subtype` with DIFFERENT
  object-typed returns + fixture N11/test 32p proving the chain declines (test-first:
  pre-fix it emitted an `Opaque` route to the arbitrary survivor's return object) + 4 new
  `build.rs` unit tests pinning the marker semantics (ABI sig-fp collision marks; lone ABI
  routine never marks; distinct-sig_fp ABI pair survives unmarked; SOURCE duplicate collapses
  unmarked). Also folded in review finding 2: `receiver_from_routine_node`'s Name+Id
  cross-validation object lookup now uses `binary_search_by` over the id-sorted
  `graph.objects` (new `object_by_id` helper) instead of an O(n) linear scan. CDO:
  byte-identical — primary/whole `unknown`=327, 1.81%, `CompoundReceiver`=154,
  `genuine_wrong`=0; direct probe confirmed ZERO collapse-marked routines in the whole CDO
  graph (all real deps ship embedded source) and all 5 `GetContent` nodes un-marked — the 2
  real resolved chain edges survive.
- **Protected-ABI soundness: `IsProtected` is now parsed from `SymbolReference.json`,
  carried as `Access::Protected` (not dropped, not hardcoded `Public`), and the three
  SymbolOnly visibility short-circuits in `resolver.rs` are closed — the ABI/SymbolOnly
  (cross-app `.app` dependency) tier previously mislabeled every dependency `protected`
  member as `Public` and let `resolve_in_object`'s SymbolOnly branch pick
  `candidates.first()` with NO visibility check, an order-dependent false-`Source`/`Opaque`
  vector for any workspace with a real SymbolOnly (no-embedded-source) dependency (cross-object
  chains + protected-ABI plan v2.1, Task 1).** `src/engine/deps/symbol_reference.rs`:
  `RawMethod`/`AbiRoutine` gain `is_protected` (`#[serde(rename="IsProtected")]`, default
  false — verified against real Microsoft System App data: 10 `"IsProtected":true` entries,
  1:1 with its embedded source's 10 `protected procedure`s) and a tri-state
  `parameters_known` flag (an explicit empty `Parameters:[]` is a KNOWN 0-arity; an
  absent/unparseable `Parameters` field is UNKNOWN arity, never zero).
  `src/program/abi_ingest.rs`: survivors of the pre-existing `is_local||is_internal` drop now
  carry `Access::Protected` (not `Access::Public`) when `IsProtected`; a new
  `UNKNOWN_ARITY` (`usize::MAX`) sentinel `params_count` for unknown-arity routines can never
  arity-match a real call site, so it structurally never emits — no special-casing needed
  downstream. `src/program/resolve/resolver.rs`: (1) `resolve_in_object`'s SymbolOnly branch
  no longer special-cases the tier at all — it now flows through the SAME arity-exact +
  per-candidate-visibility selection (incl. the overload-narrowing guard) the source tier
  already used, emitting only on an unambiguous, visible, arity-matched candidate; (2)
  `object_has_visible_member_candidate`'s SymbolOnly short-circuit is now a NAME-ONLY `.any()`
  scan over every same-name candidate (factored into a new shared `object_access_visible_from`
  predicate so the arity-filtered source scan and the name-only SymbolOnly scan can never
  drift) — a protected first sibling can no longer hide a visible public one, and this boolean
  stays existence/diagnostics-only, never edge evidence; (3) `access_exclusion_reason` is now
  tier-agnostic (dropped its `obj_tier` parameter entirely) and computes the real
  `ProtectedNotVisible`/`InternalNotVisible`/`LocalNotVisible` reason for SymbolOnly instead of
  a hardcoded `None`. New fixture `tests/r0-corpus/ws-protected-abi/` (a real probe `.app`, no
  embedded source) end-to-end proves: a non-extending caller sees honest
  `Unknown(ProtectedNotVisible)` on a `protected` ABI member; a genuine `PageExtension` of the
  ABI base DOES see it (carry-Protected, not drop); `local`/`internal` stay dropped; a mixed-
  arity mixed-access overload pair (`protected GetWorker()` / `public GetWorker(ID)`) never
  lets the arity-0 call silently select the visible arity-1 sibling; a single visible public
  `Get(ID)` never emits on a wrong-arity `Get()` call; an interface fan-out to a SymbolOnly
  implementer applies the SAME per-candidate visibility as its source-tier sibling; a
  same-id/name but wrong-KIND workspace object (`Table 60000 "Dep Page"` vs the ABI's `Page
  60000 "Dep Page"`) never bleeds identity into the real ABI base. Verified ABI `Variables[]`
  is not parsed or ingested anywhere in the codebase (grep-confirmed zero occurrences) — the
  deferred `protected` table/page VARIABLE modifier is a genuine no-op today, not a silent
  soundness gap. **Empirically CDO-neutral**: CDO's only true SymbolOnly unit ships zero
  public routines (all real deps ship EmbeddedSource/ShowMyCode), confirmed by a new
  diagnostic (`abi_ingestion_integrity_cdo_gate`) enumerating non-empty SymbolOnly objects —
  0 found — so both CDO metric gates stay BYTE-IDENTICAL (1.82% / `unknown=329`,
  `genuine_wrong=0`); the fix is proven exclusively by the new in-repo fixtures.
- **`internalsVisibleTo` friend-app parsing (`parse_manifest_xml`) is now scoped to
  `<InternalsVisibleTo>`, not a whole-document `<Module>` scan** (`src/app_package.rs`,
  whole-branch review M1). The friend-app scan previously used
  `doc.descendants().filter(|n| n.has_tag_name("Module"))` — a whole-document scan not
  restricted to the `<InternalsVisibleTo>` section. AL's `NavxManifest.xml` places
  `<Module Id Name Publisher/>` elements only under `<InternalsVisibleTo>`, but the loose
  scan meant a stray `<Module>` element elsewhere in the manifest would have been ingested
  as a spurious friend app; if its GUID happened to resolve to a real app in the snapshot,
  that app's `internal` calls into the exposing app would false-resolve to `Source` — a
  latent false-`Source` vector. Fixed by finding the `<InternalsVisibleTo>` element first
  and iterating only its `<Module>` children (empty friend list if the section is absent).
  Behavior-preserving on real manifests (CTS-CDN's `<Module>` entries are already under
  `<InternalsVisibleTo>`) — CDO's self-reported metric is unchanged at 1.82% (329
  `unknown`/18104), `genuine_wrong=0`. New unit test
  `parse_manifest_xml_ignores_stray_module_outside_internals_visible_to` asserts a stray
  `<Module>` outside the section is excluded from the friend list.
- **uniform-access-and-compound-receiver plan Task 1: `resolve_in_object` is now PER-CANDIDATE
  access-aware — closes the last two false-`Source` gaps in `resolve_member`, the `Object`-arm
  and both `Interface`-impl fan-out delegates** (`src/program/resolve/resolver.rs`).
  `resolve_in_object` (the shared arity-matching routine lookup 7 callers share) previously did
  ZERO access filtering of its own; callers A (`resolve_in_table_scope`)/B (`resolve_bare` Step 1
  self)/C (`resolve_bare` Step 2 extension-base, Task 1.5)/E (`SelfObject`) were pre-gated or
  self-referential no-ops, but D (`resolve_member`'s `ReceiverType::Object` general dispatch) and
  F/G (the `Interface` SymbolOnly/Source-impl fan-out delegates) had no such gate, so a cross-app
  `internal` or same-app-but-different-object `local` member reached through an Object receiver
  or an interface implementer could false-resolve to `Source`. Added a new
  `routine_candidate_is_visible` predicate (the per-`Access` rule — Public always visible;
  Local only to the declaring object itself; Internal only same-app; Protected only to self or a
  direct kind-compatible extension via `ResolveIndex::object_extends`; an access-lookup miss
  fails closed) applied PER CANDIDATE rather than existentially, and threaded `from_object:
  &ObjectNodeId` through all 7 callers. **The overload-narrowing guard:** selection now computes
  `pre_filter_count` (arity-matched candidates BEFORE visibility) and only picks the lone visible
  survivor when it was ALSO the lone candidate pre-filter; if access narrowed an
  originally-ambiguous (`pre_filter_count > 1`) same-arity set down to one visible candidate, that
  is NOT a safe selection (arg types are unproven, so access alone can't prove which overload the
  call meant) — it stays an honest `Unknown(OverloadAmbiguous)` rather than manufacturing a false
  `Source`. `Codeunit.Run`/`resolve_object_run` (entry-trigger dispatch) and event-subscriber
  edge emission both bypass `resolve_in_object` entirely and are untouched. 15 new unit tests in
  `src/program/resolve/resolver.rs` cover the full matrix: positive controls (cross-app `public`,
  same-app `internal`, direct-extension `protected`, `this.LocalProc()`, bare own `local`,
  same-app internal interface-impl, `Codeunit.Run`-with-no-`OnRun` opaque control) and negatives
  (Object-arm cross-app `internal`/same-app `local`-cross-object/non-extension `protected`
  same-app+cross-app/wrong-kind-extension `protected`, the mixed-access same-arity overload guard,
  cross-app `internal` interface implementer excluded while a sibling `public` implementer still
  resolves, and a user-defined member literally named `Run` with arity 2 proving the
  `Codeunit.Run` exemption is scoped to arity≤1, not name-based) — TDD-verified against the
  pre-fix code (temporarily neutralized `routine_candidate_is_visible`, confirmed the exact wrong
  `Source` routes the fix corrects, then restored). CDO (`CDO_WS`): `genuine_wrong` stays 0;
  primary/whole `unknown` count rose 356→407 (+51, ALL in the `InternalNotVisible` bucket — every
  other `unknownByReason` bucket byte-identical), `real_unknown_rate` 1.97%→2.25%. Spot-checked
  against real CDO source (e.g. `Interface "CTS-CDN IPrePostValidator"` fan-out calls and a `Page
  "CTS-CDN Connect eCandidates"` Object-receiver call, both targeting the same
  "Continia Delivery Network" dependency app). The companion `cdo_l3_semantic_audit_no_fresh_wrong`
  gate IMPROVED alongside this fix: `matches` against the L3 golden rose 6460→6510 and `fresh_wrong`
  fell 149→148 (ceiling tightened to match), `fresh_missing` unchanged at 4. **Narrative correction
  (Task 1.5, below): this +51/2.25% was reported here as an unqualified "soundness correction" —**
  **that was INCOMPLETE.** It correctly failed closed on cross-app `internal` (no friend exception
  modeled yet), but every one of the resulting 60 `InternalNotVisible` sites (the +51 here plus 9
  pre-existing) turned out to be AL-LEGAL calls the declaring app's own manifest explicitly
  authorizes via `<InternalsVisibleTo>`. See Task 1.5 immediately below for the restoration and the
  corrected combined story.
- **uniform-access-and-compound-receiver plan Task 1.5 (inserted after Task 1): model
  `internalsVisibleTo` friend apps — cross-app `internal` visible to declared friends**
  (`src/app_package.rs`, `src/snapshot/snapshot.rs`, `src/program/build.rs`, `src/program/graph.rs`,
  `src/program/resolve/resolver.rs`). AL: an `internal` member is visible within its declaring app
  AND to any app the declaring app's manifest lists in `<InternalsVisibleTo><Module Id Name
  Publisher/></...>` (a "friend" app) — a field that was already sitting right next to
  `<Dependencies>` in every manifest, unread. Measuring CDO's `InternalNotVisible` bucket after Task
  1 proved 100% of it (60 sites) is CDO calling `internal` members of its CTS-CDN dependency, whose
  manifest explicitly names CDO a friend — Task 1's strict same-app-only rule was an OVER-DECLINE,
  not a soundness floor. Four layers, all new: (1) `app_package.rs::parse_manifest_xml` (factored
  out of `parse_manifest` for unit-testability) now also parses `<InternalsVisibleTo>` into a new
  `AppMetadata::internals_visible_to: Vec<FriendApp>` (`FriendApp` has no `version` — `<Module>`
  entries don't carry one). (2) `snapshot.rs` carries it onto a new `AppUnit::internals_visible_to`
  (dependency units only; the workspace unit is never itself treated as a dependency in this
  closed-world model, so its own friend list is out of scope). (3) `build_program_graph` gained
  Step 3b: resolve each friend GUID to an `AppRef` the same guid-first/name+publisher-fallback way
  Step 3 resolves dependencies, populating a new `ProgramGraph::friends: HashMap<AppRef,
  BTreeSet<AppRef>>` (key = app EXPOSING internals → its trusted callers; one-directional, per the
  DECLARING app, never inferred from the reverse). (4) a new `internal_visible_across` helper
  (`exposing_app == caller_app || friends[exposing_app].contains(caller_app)`) replaces the bare
  `==` in BOTH `routine_candidate_is_visible` and `object_has_visible_member_candidate`'s
  `Access::Internal` arms (plus `access_exclusion_reason`'s matching arm, so the diagnostic stays
  consistent with the visibility predicate). A welcome unplanned side effect: because
  `object_has_visible_member_candidate` also gates `resolve_bare`'s Step 2 (extension-base), 7
  further sites that a documented `resolve_bare` reason-overwrite gap had mislabeled
  `ReceiverOutOfClosure` instead of `InternalNotVisible` now resolve directly too (Step 2 succeeds
  outright and never reaches the overwrite path). 4 new unit tests in
  `src/program/resolve/resolver.rs` (friend-authorized resolves; a true-stranger CONTROL still
  declines; DIRECTIONALITY — A trusting B doesn't imply B trusts A back; same-app unaffected), TDD
  RED-verified by temporarily hardcoding the same-app-only rule and confirming the exact 2
  friend-dependent tests fail while the control/same-app tests stay green. New fixtures under
  `tests/r0-corpus/ws-friend-app-internal/`. CDO (`CDO_WS`): `genuine_wrong` stays 0;
  `InternalNotVisible` bucket dropped to exactly 0; primary/whole `unknown` 407→340 (a drop of 67 —
  the 60 originally measured plus the 7 `ReceiverOutOfClosure` side-effect sites above; CORRECTED
  2026-07-02, Task 5 — this entry and the mirroring ratchet comment in
  `tests/program_resolve_harness.rs` previously said "10", which doesn't sum to the measured 67),
  `real_unknown_rate` 2.25%→1.88% — BELOW every prior recorded floor, including the pre-Task-1
  1.91%, confirming the Task-1-alone number was never the true honest floor. Adjudicated a sample
  of restored edges against real CDO/CTS-CDN source (both `.app`s extracted directly): the base
  Page's 3 `internal` procedures, both implementers of `IPrePostValidator.Validate` declaring
  `internal procedure Validate`, and CTS-CDN's manifest literally listing CDO's real `AppId` as a
  friend — every sampled edge targets the correct member. `cdo_l3_semantic_audit_no_fresh_wrong`:
  `fresh_wrong` rose 148→149 (ratchet retightened to the exact measured value) because the retired
  L3/al-sem TS reference never modeled `InternalsVisibleTo` either, so one of the 67 restored sites
  now diverges from the (equally naive, frozen) golden rather than matching it — adjudicated
  `fresh_ahead_dispatch`, not `genuine_wrong`, per this project's "no byte-parity with al-sem, fresh
  is Rust-owned" charter. `fresh_missing` unchanged at 4.
- **soundness-completion plan Task 2: shape-preserving object-typed declared-var resolution
  (`ParsedType::Object` → `ObjectRef`) — mirrors I1's `Record` fix for the `Object` sibling**
  (`src/program/resolve/receiver.rs`). `ParsedType::Object { kind, name: String }` collapsed a
  numeric AL object id (`Codeunit 80`) and a QUOTED digit-string name (`Codeunit "80"` — a
  codeunit literally NAMED `80`) into the identical string `"80"` in `parse_object_kind_type`
  BEFORE `resolve_object_name_lc` ever ran; that function then re-parsed the already-unquoted
  string with `.parse::<i64>()`, so both shapes silently resolved by numeric id 80 — the exact
  I1 `ParsedType::Record` shape-loss bug, still open for `Object`. `ParsedType::Object` now
  carries a losslessly-shaped `object_ref: ObjectRef` (`Id`/`Name`, exactly like `Record`'s
  `table_ref`), classified in `parse_object_kind_type` before any unquoting happens (a bare
  numeric string is `Id`; a QUOTED numeric string fails the `i64` parse on the quote characters
  and becomes `Name`, matching `classify_type_text`'s `Record` arm precedent). A new
  `resolve_object_ref_lc` replaces `resolve_object_name_lc`, calling the same fail-closed,
  dependency-closure-scoped `ResolveIndex::resolve_object_ref` Tasks 5/6 already use for
  `SourceTable`/`TableNo` — no `.parse::<i64>()` call remains anywhere in this path. A `Unique`
  resolution now carries the resolved id up front in `ReceiverType::Object` (mirrors Task 7's
  `CurrPage.<part>.Page` carried-id short-circuit), so `resolve_member`'s `Object` arm no longer
  needs a redundant second by-name lookup for the (common) resolved case. New unit tests cover
  the numeric-vs-quoted-name distinction for all 5 kinds `resolve_object_ref_lc` serves
  (Codeunit/Page/Report/Query/XmlPort), plus a new end-to-end call-graph fixture
  (`tests/r0-corpus/ws-object-name-shape/`, loaded via `resolve_full_program`): `codeunit 80
  RealById` (no `P()`) + `codeunit 50100 "80"` (declares `P()`) + a caller declaring `C:
  Codeunit "80"; C.P()` — the fresh edge now correctly targets the NAMED codeunit
  (`Evidence::Source`, id 50100), where pre-fix it collapsed to id 80 and produced a false
  `Unknown` (id-80's `RealById` has no `P()`). TDD-verified: the end-to-end fixture failed
  against the unmodified code with the exact predicted `Unknown` route before the fix landed.
  CDO (`CDO_WS`): `genuine_wrong` stays 0; `real_unknown_rate` and every other CDO metric
  UNCHANGED (dormant, like I1 — digit-named AL objects are ~never seen in real Business
  Central).
- **soundness-completion plan Task 1: caller-identity-aware member visibility — closes two
  latent false-`Source` gaps in `object_has_visible_member_candidate`** (its sole caller,
  `resolve_in_table_scope`, and `ResolveIndex`) — same-app `local` was treated as app-scoped
  (AL `local` is OBJECT-scoped: visible only to the DECLARING object) and cross-app
  `Access::Protected` was completely unfiltered. Both are now gated by the CALLER's resolved
  object identity (`ObjectNodeId`, never a lowercased-name comparison), per access level:
  `Public` always visible; `Local` only to the declaring object itself (self); `Internal` only
  same-app (friend-app `InternalsVisibleTo` is out of scope, documented, fails closed to
  `Unknown`); `Protected` only to self OR a DIRECT, kind-compatible extension of the declaring
  object, via a new `ResolveIndex::object_extends` (identity-resolved through
  `resolve_object_ref`, generalized across every AL extension kind — TableExtension→Table,
  PageExtension→Page, ReportExtension→Report, EnumExtension→Enum — never transitive, never
  reverse, never peer). The biggest latent bug closed: a `TableExtension`'s `protected`
  procedure was visible to a SIBLING extension of the same base table (peer-bleed) — now
  correctly declines to honest `Unknown`. New `ObjectKind::is_extension_kind`/
  `extension_base_kind` methods (`crates/al-syntax`). 15 new + 3 reused unit tests in
  `src/program/resolve/resolver.rs` cover the full access matrix (self/same-app-cross-object/
  peer/cross-app × local/protected/internal); TDD-verified against the pre-fix code (temporarily
  reverted, confirmed the exact wrong routes the fix corrects, then restored). New fixture
  matrix + `COMPILER_PROOF.md` under `tests/r0-corpus/ws-visibility-local-protected/`. Also adds
  the Item-4 explanatory comment (`is_bare_builtin_or_page_intrinsic` + `resolve_member`'s
  `Record`/`RecordRef` arms): the Record-receiver source-shadows-catalog precedent is
  deliberately NOT collision-guarded — corpus-validated correct AL precedence, not a bug. CDO
  (`CDO_WS`): `genuine_wrong` stays 0; `real_unknown_rate` unchanged at 1.91% (346 unknown) —
  this soundness fix has zero measurable footprint on the CDO corpus (the affected pattern is
  rare/absent there), consistent with the task brief's prediction.
- **soundness-completion plan Task 1.5: access-filter `resolve_bare`'s Step 2
  ("extension base") — closes a false-`Source` `resolve_in_object` left unfiltered**
  (`src/program/resolve/resolver.rs`). Task 1 made `resolve_in_table_scope` (the Rec-member +
  bare-Rec paths) caller-identity-aware, but `resolve_bare`'s Step 2 — resolving a bare call
  against a `*Extension`'s BASE object — is a separate path through `resolve_in_object`, which
  does zero access filtering. A bare call from a `TableExtension`/`PageExtension`/… to a base
  object's `local` procedure, or to a CROSS-APP `internal` procedure, previously false-resolved
  to `Source`. Step 2 now gates on the SAME Task-1 rule via `object_has_visible_member_candidate`
  (the extension is the caller, the base is the candidate): base `Local` is NEVER visible to a
  bare call from an extension (base-self only); cross-app `Internal` requires the same app;
  `Protected` stays visible (Step 2's caller is by construction a direct extension of the base,
  so self-or-extends trivially holds — confirmed still correct, not accidentally permissive);
  `Public` stays visible. Not-visible declines Step 2 entirely (no `resolve_in_object` call),
  falling through to Step 3/4/5 exactly like the pre-existing "no candidate" shape. Minor
  cleanup: `ResolveIndex::object_extends`'s object lookup switched from an O(n)
  `graph.objects.iter().find` to a `binary_search_by`, mirroring `lookup_routine_access`
  (`graph.objects` is sorted by `ObjectNodeId` at construction). 6 new unit tests in
  `resolver.rs` (TableExtension `local`-excluded + `Public`/`Protected` controls, cross-app
  `internal`-excluded, PageExtension `local`-excluded + `Public` control), TDD-verified against
  the pre-fix code (temporarily reverted, confirmed the exact wrong routes — false `Source` to
  the base's `local`/cross-app-`internal` member — then restored). CDO (`CDO_WS`):
  `genuine_wrong` stays 0; primary/whole-program `unknown` rose 346→356 (+10, rate 1.91%→1.97%,
  still under the 0.021 ceiling) — spot-check VERIFIED as a genuine correction, not an
  over-decline: every +10 edge is a bare call in `CDOConnecteCandidates.PageExt.al`
  (PageExtension in app "Continia Document Output") to an `internal procedure`
  (`GetIsSingleConnect`/`GeteCandidatesFiltered`/`GetIsVendor`) declared on the base Page in app
  "Continia Delivery Network" — a genuinely different app (confirmed via `app.json` dependency
  GUIDs and by extracting that dependency's embedded ShowMyCode source directly). The `unknown`
  COUNT ceilings raised 355→365 (soundness beats the metric; the `<= 0.021` rate ceiling was not
  tripped, left unchanged).

### Added
- **uniform-access-and-compound-receiver plan Task 2: thread the parsed receiver `ExprId` to
  `infer_receiver_type` + add `return_type` to source `RoutineNode` — enabling infra for Tasks
  3-4, RESOLUTION-NEUTRAL** (`src/program/resolve/extract.rs`, `full.rs`, `receiver.rs`,
  `src/program/node_extract.rs`, `abi_ingest.rs`). Two primitives Tasks 3-4's compound-receiver
  resolvers need were missing: (1) the resolver only ever saw a call site's receiver as a raw
  `receiver_text: String` (`CalleeShape::Member`) — the STRUCTURED `Expr` tree-sitter/al-syntax
  had already built for it (`ExprKind::Call{function,args}` / `Member{base,member}` / …) was
  discarded at extraction; (2) `RoutineNode` (the program-graph node) had no `return_type`, even
  though `RoutineDecl.return_type: Option<String>` was already parsed and available. Added
  `CalleeShape::Member.receiver: Option<ExprId>`, populated at its sole construction site
  (`extract.rs::classify_call`) from the `object` `ExprId` classification already derives
  `receiver_text` from; threaded it through `ObligationKind::CallSite` (implicitly, via `shape`)
  into `resolve_call_site_obligation` (which now also takes `file: &AlFile` so the id can be
  dereferenced) and on into `infer_receiver_type`'s new `receiver_expr: Option<(&AlFile, ExprId)>`
  parameter — a resolver can now call `file.ir.expr(id)` to inspect the receiver's real shape
  instead of re-parsing `receiver_text` (which cannot recover argument count/shape and would
  corrupt on a `.` inside a string-literal argument). `infer_receiver_type`'s existing Steps 0-4
  are UNCHANGED and still dispatch purely on `receiver_lc`; the new parameter is accepted but not
  yet consumed (Tasks 3-4 give it behavior). Added `RoutineNode.return_type: Option<String>`,
  copied verbatim from `RoutineDecl.return_type` in source extraction (`node_extract.rs`); the ABI
  ingestion path (`abi_ingest.rs`) sets `None` (`AbiRoutine.return_type_text` stays unprojected —
  a documented, deferred scope gap, not an oversight). **Golden-neutrality mechanics (mandatory,
  not incidental):** `CalleeShape` switched from `#[derive(PartialEq, Eq)]` to a MANUAL impl that
  compares every variant's payload EXCEPT `Member.receiver` — an `ExprId` is only stable within
  the single `AlFile` it was produced from and carries no resolution-affecting information on its
  own, so it must never influence obligation identity, dedup keys, ordering, or output; neither
  `CalleeShape`/`RawSiteV2`/`ObligationKind`/`RoutineNode` derive `Hash`/`Ord`/`Serialize`, so no
  further exclusion sites existed. Verified: 4 new invariant unit tests (`extract.rs`,
  `receiver.rs` x1, `node_extract.rs`, `abi_ingest.rs`) proving the `Func(1,2,3).M()` receiver
  dereferences to a real `ExprKind::Call{args.len()==3}` node AND that feeding it into
  `infer_receiver_type` still returns the pre-existing `Unknown` (neutral); full `cargo test
  --workspace` green (no golden moved — `git status` on `tests/goldens/` clean); CDO
  (`CDO_WS`) `cdo_l3_semantic_audit_no_fresh_wrong` + `cdo_full_program_coverage_and_self_reported_metric`
  UNCHANGED at real-`unknown` 1.88% / 340 (the post-Task-1.5 baseline; this task adds zero resolution
  behavior — pure carry + field populate).
- **uniform-access-and-compound-receiver plan Task 3: resolve `Func().Method()` compound
  receivers via `resolve_bare`-typed prefix return type, fail-closed** (`src/program/resolve/
  receiver.rs`, `resolver.rs`, `full.rs`, `semantic_golden.rs`). New Phase-A Step 5 in
  `infer_receiver_type` (`infer_call_result_receiver`): when the receiver's structured `Expr`
  (Task 2's `receiver_expr`) is `ExprKind::Call{function, args}` with a BARE-identifier
  `function` (a dotted/member function — the `Obj.Method().X()` cross-object chain — declines,
  DEFERRED to Task 4), types the receiver by the return type of that bare same-object function:
  (1) **local-shadowing guard FIRST** (round-2 gemini critical) — `resolve_bare` cannot see
  locals/params/globals, but a same-named variable SHADOWS a same-named procedure in AL, so a
  `function_lc` match against `routine.params`/`routine.locals`/`object_globals` declines
  immediately, never typing a variable-index access as a call; (2) otherwise calls
  `resolve_bare(from_object, function_lc, args.len(), ...)` as a TYPE QUERY, requiring the
  single returned `Route` (always exactly one, by `resolve_bare`'s own contract) to target
  `RouteTarget::Routine` — reusing `resolve_bare`'s own-object/extension-base/implicit-Rec/
  builtin precedence, same-arity-overload-ambiguity decline, and builtin/Rec-shadow
  PROBE-THEN-DECIDE collision guard for free; (3) requires `RoutineNode.return_type` (Task 2)
  to be `Some` and parse (via `classify_type_text`) to a non-`Primitive` shape; (4) converts the
  parsed type to a `ReceiverType` via the EXISTING `parsed_type_to_receiver` — the same
  fail-closed conversion Step 2's declared-variable path already uses, so a cross-app-ambiguous
  `Record`/`Object` return inherits its decline-to-`None` (never guess) and an `Interface`
  return becomes `ReceiverType::Interface` (polymorphic fan-out). `infer_receiver_type` gained a
  new `bare_ctx: Option<(&BodyMap<'_>, WithState)>` parameter (mirrors Task 2's `receiver_expr`
  pattern: `Some` only at `resolve_full_program`'s real `CalleeShape::Member` call site;
  `None` — Step 5 a no-op — everywhere else: unit tests, `semantic_golden.rs`, the `RecordOp`
  shape), avoiding any signature churn for the ~50 pre-existing test call sites' RESOLUTION
  behavior (mechanically threaded through). New fixture `tests/r0-corpus/ws-compound-call-
  result/` + 12 tests in `tests/program_resolve_harness.rs`: POSITIVE `GetCustomer().Name()`
  (Record-return), `GetHelper().DoWork()` (Codeunit-return shape), `GetIFoo().Bar()`
  (Interface-return, fans out Polymorphic to the sole implementer — Task-2-finding-3 return-type-
  SHAPE coverage for all three consumed shapes); NEGATIVE: overloaded prefix with an arg count
  matching neither declared arity (wrong-overload guard), scalar (`Integer`) return, absent
  prefix, arity-mismatch against a single overload, Rec/builtin-shadow collision (`Update()`
  colliding with the `PageInstance` intrinsic from inside a Page's implicit-Rec), a local variable
  shadowing an own procedure of the same name (proven load-bearing — the shadowed procedure
  would otherwise resolve cleanly), the DEFERRED cross-object-chain shape (`Obj.DoWork().Bar()`),
  and a string-literal-dot-arg prefix (`Foo('a.b').Bar()`, proving the AST-based inspection is
  never confused by a dot inside a string literal, unlike a hypothetical text-based approach).
  Each fixture routine surfaces TWO call obligations (the inner `Func()` bare call, resolved
  independently and unrelated to this task, plus the outer `.Method()` member call Step 5
  actually types) — the test helper selects the outer (widest-span) edge. CDO (`CDO_WS`):
  `genuine_wrong` stays 0 (companion gate unchanged: `fresh_wrong=149`/`fresh_missing=4`); primary
  and whole-program `unknown` BYTE-IDENTICAL to the pre-Task-3 baseline — 340/340,
  `unknownByReason={CompoundReceiver: 167, UntrackedReceiver: 91, OverloadAmbiguous: 56,
  BuiltinPrecedenceCollision: 1, MemberNotFound: 25}` on both sides, ZERO newly-`Resolved`
  call-result edges to adjudicate. Root cause (exhaustively grepped, not sampled — see
  `tests/program_resolve_harness.rs`'s `cdo_full_program_coverage_and_self_reported_metric` for
  the exact command): CDO's source tree contains ZERO occurrences of a BARE (non-member-qualified)
  `Func().Method()` chain; every real chained-call-result idiom present (`JsonToken.AsValue()
  .AsText()`, `XmlElement.Create(Name).AsXmlNode()`, `Response.GetContent().AsText()`, …) is
  `Var.Method().Method()` — the DEFERRED cross-object-chain shape (Task 4's scope), not this
  task's bare-function shape. Not a soundness or implementation gap — the `ws-compound-call-
  result` fixtures independently prove Step 5 fires and resolves correctly when the bare shape
  IS present; this real corpus simply doesn't write AL that way. Ceiling NOT re-tightened
  (nothing moved to tighten it against); left at 348/0.020 pending Task 4.
- **uniform-access-and-compound-receiver plan Task 4: resolve `<Framework>.<Prop|Method()>`
  compound receivers via a versioned return-type table, plus `this.<rest>` self-scoped
  stripping, fail-closed** (new `src/program/resolve/framework_returns.rs`; modified
  `src/program/resolve/receiver.rs`). New Phase-A Step 6 in `infer_receiver_type`, split into an
  AST-native recursive entry point (`infer_receiver_type_for_expr`, dispatching on
  `ExprKind::Identifier`/`QuotedIdentifier`/`Member`/`Call{function: Member}`) plus a shared
  dispatcher (`infer_compound_member_receiver`) for two sub-cases: (a) **framework chain** — when
  the receiver is `<base>.<member>` or `<base>.<member>(args)`, `base` is recursively typed, and
  if it resolves `Framework(kind)`, `(kind, member_lc, is_method, arity)` is looked up in the new
  `framework_return_kind` table (10 entries — 6 JSON conversions `JsonToken.AsObject/AsArray/
  AsValue()` and `JsonObject/JsonArray/JsonValue.AsToken()`, 4 HTTP-chain `HttpResponseMessage
  .Content/Headers()`, `HttpRequestMessage.Content()`, `HttpClient.DefaultRequestHeaders()` — a
  table miss, wrong form (property vs. method-with-parens), or wrong arity all decline); (b)
  **`this.<rest>`** — when `base` is literally the `this` identifier, `member` resolves
  (`infer_this_member`) against a SELF-ONLY scope of `object_globals` ONLY (never
  `routine.params`/`routine.locals`), per AL's documented `this.` semantics ("Use the `this`
  keyword for codeunit self-reference": referencing "methods and globals within the same
  object") — a `this.Method(...)` CALL form is deliberately DEFERRED (declines) since typing it
  needs `resolve_bare`-style routine lookup, out of this step's scope. Every table entry is
  individually provenanced against Microsoft's `methods-auto` reference (`"Available or changed
  with runtime version 1.0"`, fetched 2026-07-02) AND cross-checked for membership against the
  independently-generated `member_catalog.rs` phf sets; entries L3's table claims but neither
  source confirms (`JsonObject`/`JsonArray`/`JsonValue` allegedly also having `AsValue`/
  `AsObject`/`AsArray`) are correctly OMITTED — Rust-owned more accurate than al-sem, not ported.
  A module-level `MIN_SUPPORTED_RUNTIME` pin documents the policy (every entry floors at runtime
  1.0, satisfied by every real BC workspace, so no per-workspace dynamic gate is wired — a future
  higher-floor entry must add one). Folded in the Task-3 review finding: `infer_call_result_
  receiver`'s return-type lookup switched from an O(n) linear `.find` to `graph.routines
  .binary_search_by`, mirroring `lookup_routine_access`/`make_routine_route`'s existing idiom.
  **Round-2 self-found regression, fixed before landing:** the AST-native base recursion
  originally fed a `QuotedIdentifier`'s ALREADY-UNQUOTED IR text back into `infer_receiver_type`,
  which could then spuriously match Step 4's naive first-whitespace-token static-framework-name
  check for a quoted field/var name that merely STARTS WITH a framework keyword word (e.g. a
  `Blob` field literally named `"File Blob"` unquotes to `"file blob"`, colliding with the `File`
  framework type) — caught during this task's own CDO exhaustive adjudication (real site: Table
  "CDO File"'s own `"File Blob"` field). Fixed by RE-QUOTING a `QuotedIdentifier` before
  recursing, restoring byte-for-byte parity with the top-level `receiver_lc` (always quoted, when
  sliced from raw source text, for a quoted name) Steps 0-4 already see; pinned by a regression
  test (`quoted_identifier_never_collides_with_framework_keyword_via_recursion`). New fixture
  `tests/r0-corpus/ws-compound-framework/` + 10 tests in `tests/program_resolve_harness.rs`, plus
  12 direct unit tests in `receiver.rs`: POSITIVE `Response.Content().ReadAs(...)`,
  `JToken.AsObject().Get(...)`, `this.DialogWindow.Open()`; NEGATIVE base-not-framework,
  table-miss, wrong-form (property vs. method), wrong-arity, a mis-typed recursive intermediate
  hop, a same-named member on a non-`Framework` base never hitting the table, the DEFERRED
  record-field shape (`Rec.BlobField.CreateOutStream()`), `this.` ignoring locals/params, and
  `this.Method()` call-form deferral. CDO (`CDO_WS`): `genuine_wrong` stays 0 (companion gate
  unchanged: `fresh_wrong=149`/`fresh_missing=4`); primary/whole `unknown` 340→329 (rate
  1.88%→1.82%), `unknownByReason` `CompoundReceiver` 167→156 (every other bucket
  byte-identical). All 11 newly-`Catalog` sites EXHAUSTIVELY hand-adjudicated against real CDO
  source (not a sample — diffed the full before/after edge set via a throwaway per-site dump,
  deleted before commit): 2 `this.DialogWindow.Open`/`.Close()` sites in `Page 6175313 "CDO
  eDocuments Setup Wizard"` (confirmed `DialogWindow: Dialog;` is a genuine object-level global,
  not local) resolving to the `Dialog` catalog, and 9 `<JsonToken var>.AsValue().AsText()`/
  `.AsInteger()` chains across `Codeunit 6175274`/`6175322`/`6175347`, `Page 6175389` (×3), and
  `Table 6175273` (×3) resolving to the `JsonValue` catalog — every base variable's declared type
  and every leaf member independently confirmed against the real source. The HTTP-chain table
  entries and the `HttpResponseMessage.Content()`/`GetContent()` shape from the task brief's
  illustrative example have ZERO occurrences in CDO's source (CDO uses a custom `GetContent()`
  wrapper method, not the real platform `Content()`); ratchets tightened to the measured floor
  (348→337 count, 0.020→0.019 rate) with a small deterministic margin, not loosened.
- **uniform-access-and-compound-receiver plan Task 5 (FINAL): re-measure, exhaustive-adjudication
  sign-off, ratchet finalization — arc capstone** (`tests/program_resolve_harness.rs`,
  `src/program/resolve/resolver.rs`). Closes the plan Task 1 opened. Full re-measure on CDO
  (`CDO_WS`, `ENFORCE_CDO_WS=1`, single tests, `--release`): primary/whole `unknown`=329,
  `real_unknown_rate`=1.82%, `genuine_wrong`=0, `fresh_missing`=4, `fresh_wrong`=149,
  `unknownByReason`={CompoundReceiver: 156, UntrackedReceiver: 91, OverloadAmbiguous: 56,
  BuiltinPrecedenceCollision: 1, MemberNotFound: 25} (sum=329=`unknown`; `InternalNotVisible`/
  `ReceiverOutOfClosure` both exactly 0, absent from the map). **Net across the whole plan:
  1.97%(356)→1.82%(329), −27 count / −0.15pp, `genuine_wrong` stays 0 through every single task.**
  Trajectory: Task 1 356→407 (a TRANSIENT over-decline, corrected below, not a durable floor);
  Task 1.5 407→340 (the friend-app model, −67, BELOW every prior floor); Task 2 340→340
  (golden-neutral primitives, by construction); Task 3 340→340 (0 change — `Func().Method()`
  resolution is CORRECT and structurally DORMANT on CDO: this real corpus contains ZERO bare
  chained-call-result sites; every real chained-call idiom found is member-qualified
  (`Var.Method().Method()`), the DEFERRED cross-object-chain shape, not the bare-function shape
  this task built); Task 4 340→329 (−11, the framework-table + `this.` resolver). **Exhaustive-
  adjudication sign-off:** Task 3 contributed 0 newly-resolved edges (vacuously satisfied —
  nothing to adjudicate, confirmed by an exhaustive grep of CDO's source tree, not a sample);
  Task 4's 11 newly-`Catalog` edges (2 `this.DialogWindow.Open`/`.Close()` sites + 9
  `<JsonToken var>.AsValue()...` chain sites) were EACH hand-adjudicated against real CDO source
  during Task 4 itself (see `.superpowers/sdd/task-4-report.md`), and the count equals the
  `CompoundReceiver` bucket drop (167→156) exactly — no edge unaccounted for. **Protected-ABI
  guard:** none of the 11 adjudicated edges depend on any dependency's ABI-ingested member — all
  11 resolve through the structural, compiled-in Framework builtin catalog (`Dialog`/`JsonValue`,
  via `framework_returns.rs` → `member_catalog.rs`), never through `AbiRoutine`/`RawMethod`
  access-level data, so the ABI `protected`-schema gap (documented, still open — see the roadmap
  below) cannot mislabel any of them; the guard is satisfied by construction, not merely by
  inspection. **Ratchet finalization:** the per-task tightening already landed the ceilings at
  the measured floor (`primary_rate <= 0.019` vs measured 0.0182; `unknown <= 337` vs measured
  329 — both an 8-count/~0.0008 deterministic margin, matching this file's own established
  convention) — confirmed tight on re-measurement, no further tightening or loosening needed.
  **Historical-comment correction (Task 1.5 review minor (b)):** the `ReceiverOutOfClosure`
  "dropped from 10 to 0" claim in the ratchet comment (and this file's own Task 1.5 entry, which
  separately said "10 further sites" vs "7-ish... side effect" two paragraphs apart — an internal
  self-contradiction) did not sum consistently against the measured 67-site Task 1.5 drop: two
  independently-recorded sources disagree on the pre-friend-model split.
  `.superpowers/sdd/task-1-report.md`'s own pre/post-Task-1 table reads `InternalNotVisible`
  6→57, `ReceiverOutOfClosure` unchanged at 10 (57+10=67); `.superpowers/sdd/progress.md`'s
  contemporaneous code-review note (written during Task 1's own review, citing a hands-on CDO
  re-measurement) instead states `InternalNotVisible`=60 pre-Task-1.5, implying
  `ReceiverOutOfClosure`=7 (60+7=67). Both splits sum to the correct 67; `.superpowers/sdd/task-3-report.md`'s
  explicit post-Task-1.5 histogram independently confirms the END state (both buckets OMITTED —
  i.e. exactly 0). Sided with the reviewer's hands-on figure (60/7) — it matches this file's own
  pre-existing "7-ish" hedge and the reviewer's citation is a fresh re-measurement, not a report
  transcription — over the summary-table 57/10; either way the ambiguity is cosmetic/historical
  only (today's CDO-measured values are unambiguous, see Step 1 above: both 0). Corrected both
  this file (above) and the harness ratchet comment to "7", not "10", with the reasoning inline.
  **Directionality test strengthened (Task 1.5 review minor (a)):**
  `resolve_member_object_cross_app_internal_friendship_not_bidirectional` previously asserted
  only the GRANTED direction (B → A resolves `Source`) plus a same-app B → B sanity check — the
  actual REVERSE call (A → B, where B declares no friends of its own) was never exercised, so a
  bidirectionality regression in `internal_visible_across` (e.g. an accidental
  `friends.get(a).contains(b) || friends.get(b).contains(a)` symmetric check) could have slipped
  through untested. Added a third caller object (`DirACaller`, app A, with App A now also
  depending on App B so the call is topologically reachable) and a real A → B `resolve_member`
  call against `DirBTarget.SecretB()`, asserting `RouteTarget::Unresolved` /
  `Evidence::Unknown(UnknownReason::InternalNotVisible)` — proving friendship is one-directional
  by actually calling in the un-authorized direction, not merely by construction/inspection.
  97/97 resolver unit tests pass (0 regressions from either fold-in fix). CDO (`CDO_WS`): both
  gates re-run and confirmed green and deterministic at the numbers above;
  `sum(unknownByReason)==unknown` holds (asserted in-test); the
  `resolve_module_has_no_stray_engine_l3_l2_imports` grep-guard holds (no `engine::l3`/
  `engine::l2` import added anywhere in this plan). **What stays honestly DEFERRED (next plan,
  see the plan doc's own "Roadmap — beyond this plan" section):** cross-object return-type chains
  (`Var.Method().X()` — the BULK of the remaining `CompoundReceiver`=156, needs ABI
  `return_type_text` un-discarded from `symbol_reference.rs`/`abi_ingest.rs` plus base-var
  chain-typing on the node model, not just receiver-AST typing); record-field member-of-member
  (`Rec.BlobField.X()` — a Table field-type index on `ObjectNode`); `UntrackedReceiver`=91; the
  honest-taxonomy reclassification of `OverloadAmbiguous`=56/`MemberNotFound`=25 into charter §5
  sub-states (gated, proven per-route, needs a fresh external review per its own established
  precedent); the `protected`-ABI-schema gap (`IsProtected` ingestion); the `resolve_bare`
  reason-overwrite precision fix; the `full.rs` histogram dedup.
- **soundness-completion plan Task 3: fresh-native `UnknownReason` diagnostic +
  stratified `aldump` unknown breakdown (charter §8 stratified reporting) — DIAGNOSTIC
  ONLY, the real-`unknown` COUNT and `ObligationOutcome` classification are UNCHANGED**
  (`src/program/resolve/edge.rs`, `resolver.rs`, `full.rs`, `stub.rs`, `differential.rs`,
  `src/bin/aldump.rs`). `Evidence::Unknown` is now `Evidence::Unknown(UnknownReason)` — a new
  15-variant, fresh-native enum (`CompoundReceiver`, `UntrackedReceiver`, `UnclassifiedCallee`,
  `OverloadAmbiguous`, `BuiltinPrecedenceCollision`, `WithScopeGuard`,
  `CodeunitTableNoExcluded`, `ReportRecExcluded`, `ProtectedNotVisible`, `LocalNotVisible`,
  `InternalNotVisible`, `CatalogMiss`, `ReceiverOutOfClosure`, `MemberNotFound`,
  `IndexIntegrationGap`) tagging EVERY structurally-distinct
  `Evidence::Unknown` construction site across `resolver.rs`/`full.rs`/`stub.rs` with the site-specific
  decline cause (the payload is REQUIRED at construction — the compiler enumerated every site;
  no catch-all "forgot to tag" bucket). Two new resolver helpers thread the reason through
  multi-step precedence chains that previously converged on one shared fallback route:
  `resolve_bare`'s Step 5 now tracks a running `reason` set by whichever precedence step
  declined (`WithScopeGuard`/`ReceiverOutOfClosure`/`CodeunitTableNoExcluded`/
  `ReportRecExcluded`/access-exclusion), and `resolve_in_table_scope` returns a new
  `TableScopeOutcome` (`Resolved`/`Ambiguous`/`NotVisible{access_excluded}`) instead of a bare
  `Option`, letting both its callers (`resolve_bare` Step 3, `resolve_member`'s `Record` arm)
  distinguish "no candidate at all" from "a candidate existed but was
  `Local`/`Internal`/`Protected`-excluded" via a new `access_exclusion_reason` helper. A dotted
  `receiver_text` (`A.B.C`) on an otherwise-`UntrackedReceiver` member call is relabeled
  `CompoundReceiver` in `full.rs` (AL variable/singleton/framework names never contain a dot).
  **Serialization boundary:** a new `Evidence::kind() -> EvidenceKind` projection collapses
  `Unknown(_)` back to a single reason-agnostic `Unknown` kind; `differential.rs`'s
  `witness_contract_holds` and `Histogram`'s evidence-scoring both switched to comparing on
  `.kind()`, never the raw payload — the committed anonymized semantic goldens
  (`tests/goldens/semantic-edges/*.json`) never serialized `Evidence` in the first place
  (`CanonicalTarget`/`GoldenTarget` only carry `RouteTarget`-derived identity), so they stay
  byte-identical with **no regen** (verified: `git status` clean on the goldens dir after the
  full CDO audit run). New `unknown_reason_breakdown(&[Edge]) -> BTreeMap<UnknownReason, usize>`
  (`edge.rs`) surfaces the stratification, counted per-edge (not per-route) so
  `sum(values()) == ` the `Unknown` obligation count by construction — pinned by a synthetic
  unit test (5 edges, 4 distinct reasons incl. a duplicate) and an end-to-end integration test
  over 6 real `ws-*`/fixture workspaces via `resolve_full_program` (spans 5 distinct reasons).
  `aldump --program-call-graph-stats` gained an `unknownByReason` object (camelCase keys via
  `UnknownReason::as_str()`, never `Debug`) on both the `wholeProgram` and `primaryScoped`
  histograms. CDO (`CDO_WS`): `real_unknown_rate`/`unknown` COUNT UNCHANGED (primary 1.97%,
  356 both whole-program and primary-scoped — byte-identical to the pre-Task-3 measurement);
  `cdo_l3_semantic_audit_no_fresh_wrong` still `genuine_wrong=0` with the goldens untouched.
- **soundness-completion plan Task 4 (FINAL, CAPSTONE): re-measured, the residual
  stratified breakdown pinned as the next plan's roadmap, the stratification invariant
  now gated on CDO — the plan is closed** (`tests/program_resolve_harness.rs`; no
  resolver source changes — verification + gate + docs, by design). Closes the
  soundness-completion arc (Tasks 1, 1.5, 2, 3, all already individually logged above
  in this same `[Unreleased]` section). **Stated plainly: this plan HARDENED soundness
  and STRATIFIED the residual; it did NOT reduce the real-`unknown` count** — the count
  ROSE 346→356 as a direct, verified consequence of Task 1.5's soundness correction (a
  false-`Source`→honest-`Unknown` fix); burning the residual down is the NEXT,
  data-driven plan this task's breakdown table exists to prioritize.
  - **Re-measured 2026-07-01, byte-identical to Task 3's own CDO run** (independent
    single-threaded release re-run against the live `CDO_WS` workspace): primary
    real-`unknown` rate **1.97%** (`unknown=356/18104`, exact `realUnknownRate=
    0.019664162615996465`); whole-program **0.83%** (`unknown=356/42843`, exact
    `0.008309408771561283`). Coverage holds (`parsed_obligations==classified_edges==
    42843`), `abi_unmapped=0`. `cdo_l3_semantic_audit_no_fresh_wrong`: `genuine_wrong=0`,
    `fresh_missing=4`, `fresh_wrong=149` (all 149 adjudicated `fresh_ahead_dispatch`,
    51/51 `l3_error_intrinsic` overlay entries applied) — all EXACTLY matching Task
    1.5/Task 3's own recorded numbers, no drift.
  - **The 356 residual by `UnknownReason` (the measured deliverable this task exists to
    record):**

    | Reason | Count | % of 356 |
    |---|---|---|
    | `compoundReceiver` | 167 | 46.9% |
    | `untrackedReceiver` | 91 | 25.6% |
    | `overloadAmbiguous` | 56 | 15.7% |
    | `memberNotFound` | 25 | 7.0% |
    | `receiverOutOfClosure` | 10 | 2.8% |
    | `internalNotVisible` | 6 | 1.7% |
    | `builtinPrecedenceCollision` | 1 | 0.3% |
    | (all other 8 `UnknownReason` variants) | 0 | — |

    **Next-plan levers, ranked:** `compoundReceiver` + `untrackedReceiver` = 258/356
    (73%) — genuine RESOLUTION gaps (chained/subpage receivers, untracked
    variable/singleton receivers), the biggest burndown opportunity.
    `overloadAmbiguous` + `memberNotFound` + `receiverOutOfClosure` = 91/356 (26%) —
    charter §5 candidates for honest-sub-state RECLASSIFICATION (routing genuinely-honest
    outcomes like overload-ambiguity or a genuinely-absent member out of real-`unknown`
    into a distinct `ObligationOutcome`, pending a fresh external review per the plan's
    own roadmap — proven per-route genuine, never laundered). `internalNotVisible` (6) is
    Task 1.5's own correction, already root-caused. `builtinPrecedenceCollision` (1) is a
    single residual site.
  - **Stratification invariant now GATED on CDO, not just fixtures**: `sum(unknownByReason)
    == unknown` — and, structurally (by `Evidence::Unknown(UnknownReason)`'s payload being
    REQUIRED at construction, never `Option`), "every `Unknown` obligation carries a
    reason" — was previously pinned only over 6 curated fixture workspaces
    (`unknown_reason_breakdown_over_real_fixtures_sums_and_spans_reasons`, always-run, no
    `CDO_WS`). `cdo_full_program_coverage_and_self_reported_metric` now asserts the SAME
    invariant over the REAL CDO corpus (both whole-program and primary-scoped
    `unknown_reason_breakdown`), closing the gap where a future decline site reaching
    `ObligationOutcome::Unknown` without tagging a reason (e.g. an empty-routes
    non-fanout edge, or an `Unresolved`-target route carrying non-`Unknown` evidence)
    could silently understate `aldump`'s `unknownByReason` while `unknown` itself climbed
    undetected — CDO is the only corpus large/diverse enough to have caught this class of
    gap historically (it is exactly how Task 1.5's own +10 was found).
  - **Ratchets: unchanged, all still hold with margin** (never loosened; Task 1.5 already
    raised these with a soundness justification, this task only re-confirms):
    `primary_rate <= 0.021` (measured 0.0197); primary + whole-program `unknown` COUNT
    `<= 365` (measured 356, the same ~9-count margin Task 1.5 left); `FRESH_MISSING_CEILING
    = 10` (measured 4); `FRESH_WRONG_CEILING = 149` (measured 149, exact, zero margin);
    `genuine_wrong == 0` hard gate (measured 0).
  - **Gates**: `cargo clippy --release --all-features -- -D warnings` clean (no `--tests`);
    `cargo fmt --check` clean; `cargo test --workspace` green (no `CDO_WS`, 159 test
    binaries incl. doctests, including the fixture-scoped stratification invariant and
    `resolve_module_has_no_stray_engine_l3_l2_imports` — no `engine::l3`/`engine::l2`
    import exists anywhere under `src/program/resolve` beyond the one sanctioned
    `builtins.rs::global_builtins` exception); the full `program_resolve_harness.rs` CDO
    suite (6 tests) + `program_graph.rs` + `snapshot_robustness.rs` CDO tests green under
    `CDO_WS` + `ENFORCE_CDO_WS=1`, single-threaded, release, deterministic (two consecutive
    `resolve_full_program` runs produce byte-identical histograms); `git status` clean on
    `tests/goldens/semantic-edges/` after the full CDO run (goldens byte-identical —
    structurally guaranteed since Task 3, as `Evidence`/`Route` carry no
    `Serialize`/`Deserialize` derive).
  - **Roadmap follow-ups carried forward** (non-blocking, tracked for the next plan): (1)
    `resolve_bare` Step 2→3's `reason` overwrite is unconditional, under-reporting
    access-exclusion vs. with-guard/out-of-closure on overlap when both could apply (fix:
    first-non-default-wins priority; the dominant reason buckets above are unaffected);
    (2) `full.rs`'s `count_into_histogram` duplicates `edge.rs`'s evidence-scoring logic
    (dedup candidate); (3) `ObsoleteState` (an obsolete-`Removed` member cannot link in
    AL — a latent false-`Source`, needs ingest-tier support before it can be checked);
    (4) `ReceiverType::Object`/`SelfObject` arms' `resolve_in_object` calls remain
    access-UNFILTERED — the 3rd instance of the pattern Task 1.5 fixed for `resolve_bare`
    Step 2 (`SelfObject` is incidentally safe; `Object` cross-app member calls are a
    residual false-`Source` exposure, same shape as the bug this plan's Task 1.5 closed);
    (5) `resolve_object_name_lc`'s `id: None` by-name-reparse fallback is the INVERSE of
    the bug Task 2 fixed (a numeric string that fails the `Id` parse could coincidentally
    match an object NAMED that digit-string) — pre-existing, dormant on real AL (digit-named
    objects are ~never seen in practice), tracked here rather than fixed speculatively.
- **follow-up plan v2.1 Task 4 (FINAL, CAPSTONE): the fail-closed object-resolution +
  bare implicit-`Rec` follow-up arc is closed — re-measured, ratchets tightened to the
  new floor** (`tests/program_resolve_harness.rs`; no resolver source changes — this
  task is verification + ratchet + docs, by design) — closes the follow-up plan v2.1 arc
  (Tasks 1-3, all already individually logged below in this same `[Unreleased]` section).
  Summary of the whole arc, before/after:
  - **Task 1 (I1 root fix)**: `resolve_object`/`object_by_number` made ambiguity-aware —
    an own-app declaration still shadows (wins), but more than one VISIBLE-in-closure
    dependency match now fails closed to `None` instead of the old lowest-`ObjectNodeId`
    pick-first tiebreak, which could silently route a cross-app same-name/id table
    collision to the WRONG dependency as a confident `Source` edge. `resolve_object_ref`'s
    `Id` arm gained the same own-app-shadow the `Name` arm already had.
    `parsed_type_to_receiver`'s declared-var `Record` arm made shape-preserving
    (`ParsedType::Record` now carries a structured `ObjectRef`, not a lossy lowercased
    string, so `Record 18` and `Record "18"` are no longer conflated). The pick-first
    `resolve_table_id` helper was deleted outright; every semantic caller inherits the
    fail-closed behavior automatically from the shared base functions. CDO-dormant (a
    real compile closure is AL-illegal for same-name cross-app tables), validated by new
    synthetic multi-app unit/e2e tests instead.
  - **Task 2 (visibility-scoped extraction)**: `resolve_member`'s Record-receiver scope
    search extracted into a new shared helper, `resolve_in_table_scope`, now
    closure-scoped (a `TableExtension` outside `from_object`'s dependency closure is
    excluded — was previously whole-snapshot, `WorldMode::AnalyzedSnapshot`) and
    `Access`-filtered (a cross-app `Internal`/`Local` candidate is excluded — was
    previously counted despite being AL-invisible). A false `Source` route is the
    cardinal sin this closes. CDO: 6 sites moved `fresh_extra`→`matches` (both now
    correctly decline); `genuine_wrong` stayed 0; zero collateral movement on
    `fresh_missing`/`fresh_wrong`.
  - **Task 3 (`resolve_bare` Step 3 — bare implicit-`Rec`)**: implemented the
    previously-empty `// 3. Implicit-Rec (deferred)` TODO — a bare (unqualified) call
    inside a `Table`/`Page`/`TableExtension`/`PageExtension` object that falls through
    Step 1 (own object) and Step 2 (extension base) now implicitly dispatches to `Rec`,
    matching real AL semantics. Every guard independently fail-closed: strict
    `ObjectKind` guard (Codeunit — even with a matching `TableNo` — Report, XmlPort,
    Query never enter Step 3); tri-state `with`-guard (AST `with`-depth AND a redundant
    raw-text scan must BOTH agree there's no enclosing `with` before Step 3 runs);
    per-kind implicit-table lookup reusing the same helpers Tasks 5-7 built for the
    EXPLICIT `Rec.Foo()` case; Task 2's `resolve_in_table_scope` reused unchanged for the
    visibility-scoped search; builtin/`PageInstance`-intrinsic collision PROBE-THEN-DECIDE
    (a table-scope hit whose name ALSO matches a global builtin or page intrinsic fails
    closed to `Unknown`, never assumes precedence). Surfaced and root-caused 7 NEW
    `genuine_wrong` sites on the first CDO run — all one shape, a `Navigate` action's bare
    `Navigate();` call newly resolving via Step 3 to 7 distinct real Microsoft Base
    Application posted-document-header tables' own genuine `procedure Navigate()`,
    independently re-verified against Base App's embedded ShowMyCode source table by
    table — adjudicated via the established `CrossAppSourceProcedure` overlay mechanism
    (extended to accept a BARE, not just qualified-member, callee shape), never
    whitelisted; `known-genuine-divergences.json`/`adjudicated-overrides.json` grew
    44→51, re-confirmed by the independent `cdo_genuine_wrong_is_precedence_adjudicated`
    re-derivation test. 13 new fixtures in `tests/r0-corpus/ws-bare-implicit-rec/`
    (11 original + 2 from a review fix pass closing a `TableExtension`/`PageExtension`
    caller coverage gap), exercised via `resolve_full_program` end-to-end.
  - **Net result** (CDO `CDO_WS`, RE-MEASURED and CONFIRMED byte-identical 2026-07-01 for
    this Task 4 closing report — reproduced independently against the live workspace,
    once by Task 3 and once again here, both single-threaded release runs matching to
    the exact float): primary real-`unknown` rate **2.81%→1.91%** (`unknown` 346/18104,
    exact `realUnknownRate=0.0191118` per `aldump --program-call-graph-stats`);
    whole-program **1.19%→0.81%** (`unknown` 346/42843, exact `0.0080760`). L3 semantic
    audit `fresh_missing` **102→4** — closes the dominant bare-call-implicit-
    SourceTable-dispatch bucket the beyond-1B.3b Task 8 characterization identified
    (82/102), plus almost all of the residual 12+8 (not individually re-characterized
    site-by-site this arc — an honest open item, not a specific root cause claim).
    `fresh_wrong` 139→149 (all 149 adjudicated `fresh_ahead_dispatch` — fresh REFINES L3,
    expected collateral movement from closing a real completeness gap, not a
    divergence). `genuine_wrong` stays exactly **0** throughout the whole arc (the 7
    newly-surfaced Task-3 sites were root-caused and adjudicated, never whitelisted).
    Coverage holds (`parsed_obligations==classified_edges==42843`), ABI integrity clean
    (`abi_unmapped=0`, `abi_routes_total=4`), deterministic across repeated runs (two
    independent full single-threaded CDO runs this task, in addition to Task 3's own
    determinism checks, produced identical histograms/digests).
  - **What stays honestly `Unknown`** (unchanged by this arc; the residual is
    CHARACTERIZED, not fixed — fixing it is future work; see
    `docs/superpowers/plans/2026-07-01-resolve-followup-fail-closed-bare-rec.md`'s
    "Roadmap — beyond this plan"): the 4-site `fresh_missing` residual (not individually
    re-diagnosed this arc); `with`-scope RESOLUTION (Step 3's guard only SKIPS inside a
    `with` block today — it does not yet BIND a bare call to the `with` record variable,
    so a genuinely-resolvable call inside `with` still honestly declines rather than
    resolving); Codeunit `TableNo`/`OnRun` implicit-`Rec` for a BARE call (Step 3's
    `ObjectKind` guard structurally excludes Codeunit — AL's bare-implicit-dispatch
    fallback is a Page/Table source-record mechanism, distinct from the Codeunit
    `TableNo` one Task 6 already closed for the EXPLICIT `Rec.Foo()` shape); a
    compiler-verified table-proc↔builtin PRECEDENCE proof (the probe-then-decide
    collision guard fails closed to `Unknown` rather than assuming a direction —
    relaxing it needs independent proof of real AL compiler precedence, not assumption);
    `Access::Protected` visibility (Task 2 intentionally left it unfiltered, a documented
    gap) and same-app `local`-object-scope visibility nuances; same-arity-TYPE overload
    DISPATCH (the genuinely-ambiguous `Variant`-typed-arg case, out of scope for the
    whole arc); Report/ReportExtension implicit `Rec` (dataitem block-scope, not
    object-level — excluded since beyond-1B.3b Task 5).
  - **Ratchets tightened** (`tests/program_resolve_harness.rs`,
    `cdo_full_program_coverage_and_self_reported_metric` +
    `cdo_l3_semantic_audit_no_fresh_wrong`; a ratchet never loosens): `primary_rate <=`
    **0.030 → 0.022 (Task 3) → 0.021** (measured 0.0191, dated 2026-07-01); primary
    `unknown` COUNT ceiling **520 → 360 (Task 3) → 355** (measured 346); companion
    whole-program `unknown` COUNT ceiling, same trajectory and same measured value;
    `FRESH_MISSING_CEILING` **110 → 15 (Task 3) → 10** (measured 4, breakdown comment
    updated to note this task's byte-identical re-confirmation); `genuine_wrong == 0`
    stays the pre-existing HARD gate (unchanged, still exact-zero); `FRESH_WRONG_CEILING`
    **150 → 152 (Task 3) → 149** (measured 149, now EXACT — zero margin, matching
    `genuine_wrong`'s own zero-tolerance philosophy, so even ONE new `fresh_wrong` site
    trips a manual review rather than passing inside slack).
  - No `engine::l3`/`engine::l2` import exists anywhere under `src/program/resolve`
    beyond the one sanctioned `builtins.rs::global_builtins` exception —
    `resolve_module_has_no_stray_engine_l3_l2_imports` (unmodified this task) still
    passes.
  - Gates: `cargo clippy --release --all-features -- -D warnings` clean; `cargo fmt
    --check` clean; `cargo test --workspace` green (no `CDO_WS`, all 65
    `program_resolve_harness.rs` tests plus the full workspace suite); full
    `program_resolve_harness.rs` suite (65 tests) green under `CDO_WS` +
    `ENFORCE_CDO_WS=1`, single-threaded, release, against the tightened ratchets above,
    including the non-vacuity route-count checks (`fan_out_applicability_zero_violations`
    routes_checked interface=28/instance_builtin=455/implicit_trigger=1183/event=2464,
    `route_applicability_zero_violations` total_routes=17646).

### Added
- **(export) incremental graphify fragments + content-hash manifest — `aldump --graphify-export-fragments` (P3)**
  (`src/program/graphify_export.rs`, `src/bin/aldump.rs`) — partitions the graphify
  document into one fragment per AL object (`{nodes, edges, hyperedges}`: the
  object node + its routines + `contains` + the edges/hyperedges ORIGINATING from
  it) plus a `shared` fragment for cross-fragment target nodes (builtin / external
  / dynamic / unresolved) so nothing dangles when graphify `build_merge`s them.
  `manifest[objectId]` is a stable FNV-1a content hash of the fragment; a
  downstream consumer (Obsidian vault, embeddings) diffs the manifest across runs
  to re-process ONLY the objects whose output changed — the incremental primitive
  that matters for AL (whole-program resolution is already cheap, so the win is
  skipping downstream vault/vector work, not extraction). Verified: manifest is
  run-stable (unit test); editing a fixture leaves unchanged objects hash-identical
  and surfaces the new object as ADDED; scales to the real workspace (11,718
  fragments + manifest, partition totals reconcile with the flat document). New
  test `fragments_partition_by_object_with_stable_manifest`. 812 lib tests.
- **(export) integration-points report — `aldump --integration-points` + `program::integration_report`**
  (`src/program/integration_report.rs`, `src/bin/aldump.rs`) — a dedicated
  "who-reacts-to-what" projection of the resolved event wiring, scoped to the
  workspace app's **integration surface**: **inbound** (workspace subscribes to an
  external/platform event — "what external changes my app hooks into"),
  **outbound** (an external app subscribes to a workspace event — "what extension
  points my app exposes, and who uses them"), and **internal**. Each event lists
  its publisher (app / object / event / kind) and every bound subscriber (app /
  object / procedure / conditions / cross-app), with whole-program totals in the
  summary. Measured on DocumentOutput/Cloud: 25,440 events / 3,404 subscriptions /
  395 cross-app whole-program; **68-event workspace surface** (53 inbound, 20
  outbound, 2 internal) — e.g. the app hooks Base App `Customer.OnAfterDeleteEvent`
  / `Purch.-Post.OnAfterProcessPurchLines`, and exposes `"CDO Events".
  OnAfterCreateDocument` consumed by 2 apps. Completes P2 (event hyperedges +
  integration-points view). New test `inbound_workspace_subscription_reported`.
- **(export) graphify hyperedges — event neighbourhoods + interface families (P2)**
  (`src/program/graphify_export.rs`) — the graphify adapter now populates
  `hyperedges` (previously always empty) with the non-pairwise integration
  structure: (1) **event groups** — one publisher event + all its ≥2 subscribers
  (`{id, label, kind:"event_group", nodes:[pub, sub1, …]}`), and (2) **interface
  families** — one interface + its ≥2 implementers (`kind:"interface_group"`).
  graphify renders each as a shaded region and preserves them in `graph.json`.
  Measured on DocumentOutput/Cloud: **529 hyperedges** (453 event groups, sizes
  3–27, mean 4.6; 76 interface families), zero dangling node refs, all 529
  round-trip through graphify `attach_hyperedges`. New test
  `event_with_multiple_subscribers_emits_hyperedge`.
- **(resolve) platform PAGE-event subscriber wiring (extends the table-event synthesis)**
  (`src/program/resolve/event.rs`, `src/program/build.rs`) — extends synthetic
  `PublisherKind::Platform` publishers to PAGE platform events (`OnOpenPageEvent`,
  `OnClosePageEvent`, `OnQueryClosePageEvent`, `OnAfterGetRecordEvent`,
  `OnAfterGetCurrRecordEvent`, `OnNewRecordEvent`, `On{Insert,Modify,Delete}
  RecordEvent`, `On{Before,After}ValidateEvent`, `On{Before,After}ActionEvent`),
  routed by the subscriber's `ObjectType::Page`. Page record/lifecycle/action
  subscriptions were the dominant residual after the table-event + `Sender` fixes.
  Measured on DocumentOutput/Cloud: orphaned subscribers **142 → 6** (99.8% of all
  3410 subscribers now wired); the residual 6 are individual Base App / test-lib
  edge cases. Coverage holds; real-unknown unchanged. 809 lib tests (new
  `platform_page_event_subscriber_wires_via_synthetic_publisher`).

### Fixed
- **(resolve) event subscriber–publisher arity match ignored the implicit `Sender` param**
  (`src/program/resolve/index.rs`) — `ResolveIndex`'s candidate filter used
  `publisher.params_count >= sub_params`, but an `[IntegrationEvent(IncludeSender=
  true, …)]` (also Business/Internal) prepends an implicit `Sender` parameter that
  a subscriber captures, so a subscriber to a 0-explicit-param publisher legally
  declares arity 1 (`procedure OnRegisterManualSetup(var Sender: Codeunit …)`).
  `0 >= 1` is false, so every `IncludeSender` subscriber was dropped and its
  integration edge lost. The bound is now the AL-correct Sender-tolerant
  `sub_params <= params_count + 1` (never rejects a valid subscriber); overload
  disambiguation prefers an exact-arity match and only falls back to the `+1`
  (Sender) match, so genuine ambiguity is still recorded. Measured on
  DocumentOutput/Cloud: orphaned subscribers **342 → 142** (+200 wired), **all
  workspace-app subscribers now bound (0 orphans)**; residual 142 are
  base-application-internal. Coverage holds; real-unknown unchanged. 808 lib tests
  (new `subscribers_of_include_sender_publisher_binds_arity_one_subscriber`).

### Added
- **(resolve) platform table-event subscriber wiring — synthetic `PublisherKind::Platform` publishers**
  (`src/program/resolve/event.rs`, `src/program/build.rs`,
  `src/program/graphify_export.rs`) — `[EventSubscriber(ObjectType::Table,
  Database::X, 'OnAfterDeleteEvent'/'OnAfterValidateEvent'/…)]` targets a
  platform-generated table event (implicit DB-trigger / field-validate) that has
  **no publisher routine in source**, so the resolve index (which binds a
  subscriber to a `publisher_kind`-bearing routine) found no candidate and the
  subscriber **orphaned** — its integration edge ("this fires when X is deleted",
  the charter's data-is-control-flow wiring) silently lost. On a real BC app this
  orphaned ~27% of all subscribers (946/3410). `build_program_graph` now injects a
  synthetic `PublisherKind::Platform` publisher routine on the table for each
  subscribed `(table, event)` (the 8 CRUD `OnBefore/After{Insert,Modify,Delete,
  Rename}Event` + `OnBefore/AfterValidateEvent`), collapsing per-field granularity
  so the index's `(object, name)` candidate model binds each to exactly one
  publisher; never shadows a real source publisher. Everything downstream — index
  match, `emit_event_flow_edges`, obligation coverage, graphify export (new
  `platform_event` routine kind) — flows through the existing publisher machinery
  unchanged. Measured on DocumentOutput/Cloud (+ Continia/MS deps): orphaned
  subscribers **946 → 342**, real publisher→subscriber wiring **2,464 → 3,068**
  (+604), 436 platform publishers injected, obligation coverage still holds,
  real-unknown unchanged (0.81%). Residual 342 are a distinct category (Codeunit
  integration-event matching misses), not table events. 807 lib + 65 harness tests
  green.
- **(export) graphify adapter — `aldump --graphify-export <workspace>` + `program::graphify_export`**
  (`src/program/graphify_export.rs`, `src/program/resolve/full.rs`,
  `src/bin/aldump.rs`) — projects the whole-program **resolved** call graph into a
  [graphify](https://github.com/safishamsi/graphify) node-link extraction document
  (`{ nodes, edges, hyperedges }`) consumed by graphify's `build_from_json`, so
  graphify's clustering / Obsidian-vault / HTML / Neo4j / MCP-query stack runs on
  engine-resolved AL edges instead of graphify's generic name-matching AST resolver
  (which has no AL parser and cannot resolve AL dispatch). One node per AL object +
  routine (+ synthetic builtin/external/dynamic/unresolved targets so no edge
  dangles); one edge per resolved route. The honest obligation taxonomy is bridged
  to graphify's `EXTRACTED`/`INFERRED`/`AMBIGUOUS` confidence tiers **without
  laundering** — `Source`/`Catalog`/`Abi` → `EXTRACTED`, `HonestDynamic`/
  `HonestEmpty` → `INFERRED`, `Unknown` (the one true failure) → `AMBIGUOUS` — with
  the full classification preserved verbatim in `obligation`/`evidence`/
  `dispatch_shape` edge attributes. `EdgeKind` maps to `calls`/`calls_builtin`/
  `calls_external`/`runs`/`triggers`/`raises_event`. Node ids are keyed on the
  resolved app **name** (never the run-order-dependent interned `AppRef`). Verified
  end-to-end: the emitted document round-trips through graphify's real
  `build_from_json` with zero dangling edges, and the graphify confidence histogram
  reproduces the engine's `--program-call-graph-stats` obligation histogram
  (anti-laundering). `resolve_full_program` refactored to share a `build_context`
  helper with the new `resolve_full_program_for_export` (behaviour-preserving; the
  65-test program-resolve harness is unchanged). Mapping spec: `U:\Git\graphify\adapter.md`.
- **(resolve) `resolve_bare` Step 3 — bare implicit-`Rec` dispatch, `with`-guarded + builtin-collision-fail-closed, visibility-scoped (follow-up plan v2.1 Task 3)**
  (`src/program/resolve/resolver.rs`, `src/program/resolve/extract.rs`,
  `src/program/resolve/receiver.rs`) — implements `resolve_bare`'s Step 3,
  previously an empty `// TODO`: a BARE (unqualified) call inside a
  `Table`/`Page`/`TableExtension`/`PageExtension` object that falls through
  Step 1 (own object) and Step 2 (extension base) now implicitly dispatches
  to `Rec` — AL semantics: `SomeProc()` in Page/Table code means
  `Rec.SomeProc()` as a LAST-RESORT fallback. Every guard is independently
  fail-closed:
  - **Strict `ObjectKind` guard**: structurally limited to `{Table, Page,
    TableExtension, PageExtension}`; every other kind (Codeunit — even one
    with a matching `TableNo`, Report, XmlPort, Query, …) skips Step 3
    entirely, no accidental leakage.
  - **`with`-guard, tri-state (`WithState`, new in `extract.rs`)**: Step 3
    runs ONLY on `NoWithProven`. Investigated whether the IR tracks
    enclosing `with X do` scope: it does — `walk_stmt_v2`'s `with`-depth
    tracking is an EXHAUSTIVE match over every `StmtKind` variant (no
    wildcard arm), so it is structurally sound for every site it visits.
    Given this project's history of grammar/lowering surprises (see
    `CLAUDE.md`), the AST signal is combined CONJUNCTIVELY with a redundant,
    cheap whole-routine raw-text scan for a standalone `with` token
    (`routine_has_with_token`) — `NoWithProven` only when BOTH agree; a
    scan-hit with AST depth 0 (the two signals disagreeing) is `Unknown`
    (skip), never trusted as with-free. False positives (over-skip) are
    safe; a false negative (running Step 3 inside an unrepresented `with`)
    would be a false `Source` edge, so the raw scan is fail-closed insurance
    at negligible cost — `with` is rare in practice (Base App 24 removed it,
    AppSourceCop forbids it).
  - **Per-kind implicit table lookup**: `Table`→self; `Page`→
    `resolve_source_table_ref(source_table)`; `TableExtension`→
    `resolve_tableext_base_table` (`resolve_object_ref(Table,
    extends_target)`, Task-1 fail-closed); `PageExtension`→
    `resolve_pageext_base_source_table`. All three helpers already existed
    in `receiver.rs` for the EXPLICIT `Rec.Foo()` case (Tasks 5-7) and are
    now `pub(crate)`, reused as-is rather than re-derived — one correct
    answer per kind, no duplicated logic.
  - **Visibility-scoped search**: reuses Task 2's `resolve_in_table_scope`
    (base table ∪ its visible `TableExtension`s, closure- and
    access-filtered) unchanged.
  - **Builtin/intrinsic PROBE-THEN-DECIDE**: after a table-scope search
    finds a same-name+arity candidate, if the name ALSO matches a global
    builtin or a bare-callable `PageInstance` intrinsic (`Update`/`Close`/…),
    the collision is an UNPROVEN precedence — fail closed to `Unknown`
    (never `Catalog`) rather than assume the table wins. A builtin/intrinsic
    name with NO table candidate still falls through to Step 4 (`Catalog`)
    unchanged.
  11 new fixtures in `tests/r0-corpus/ws-bare-implicit-rec/` (positive:
  Page→table dispatch, visible-TableExtension dispatch; negatives: own-object
  shadow, sibling-extension ambiguity, builtin collision, page-intrinsic
  collision, `with`-block suppression, no-implicit-table Codeunit,
  same-table own-trigger shadow-guard, PageExtension base-vs-SourceTable
  precedence, strict-kind Report/Codeunit+TableNo exclusion) exercised via
  `resolve_full_program` end-to-end, asserting the EXACT route at the EXACT
  site for every case. One fixture bug caught by the guards themselves during
  authoring: an initial `GetName` procedure name collided with the REAL AL
  global builtin `GetName` (an XmlNode/Media intrinsic), correctly forcing
  the collision path to `Unknown` — renamed to `GetDisplayText` for a clean
  positive case.
  **CDO gate (measured 2026-07-01, `CDO_WS`)**: primary real-`unknown` rate
  2.81%→**1.91%** (unknown 508→346/18104), whole-program 1.19%→**0.81%**
  (unknown 508→346/42843) — `cdo_full_program_coverage_and_self_reported_metric`
  ceilings tightened accordingly (0.030→0.022, 520→360). `fresh_missing`
  102→**4** (FRESH_MISSING_CEILING 110→15) — closes the dominant
  bare-call-implicit-SourceTable-dispatch bucket (82/102) the beyond-1B.3b
  Task 8 characterization identified, plus most of the residual. 7 NEW
  `genuine_wrong` sites surfaced (all 7 the SAME shape: a `Navigate` action's
  bare `Navigate();` call, newly resolving via Step 3 to each Page's
  `SourceTable`'s own `procedure Navigate()` — a REAL, ordinary Base
  Application procedure on 7 distinct posted-document-header tables,
  independently re-verified against Base App's own embedded ShowMyCode
  source: `Return Receipt Header`/`Issued Fin. Charge Memo
  Header`/`Service Cr.Memo Header`/`Service Invoice Header`/`Sales
  Cr.Memo Header`/`Sales Shipment Header`/`Service Shipment Header`).
  Root-caused (not whitelisted): fresh's new target is objectively correct
  per real BC semantics; L3's frozen golden simply predates bare-implicit-Rec
  dispatch and never modeled the shape. Extended
  `verify_cross_app_source_procedure_override`
  (`tests/program_resolve_harness.rs`) to accept a BARE `callee_text` (in
  addition to the existing qualified-member-call shape) for the
  `CrossAppSourceProcedure` adjudication path, then added all 7 as
  independently-source-verified `l3_error_intrinsic` entries to
  `adjudicated-overrides.json` + `known-genuine-divergences.json` (both
  42+2→**51** now), re-confirmed by the independent
  `cdo_genuine_wrong_is_precedence_adjudicated` re-derivation test.
  `genuine_wrong` stays exactly **0**. `fresh_wrong` 139→149 (all
  `fresh_ahead_dispatch`, expected collateral movement from closing a real
  completeness gap; FRESH_WRONG_CEILING tightened 150→152). Hand-adjudicated
  a sample across object kinds including the report's own worked example
  (`Page 6175272 "CDO E-Mail Templates"`'s bare
  `GetReportSelection()`/`GetReportName()` → table 6175283) — all confirmed
  correct.

### Fixed
- **(resolve) Visibility-scoped `resolve_in_table_scope` — closure-filter
  `TableExtension`s and exclude cross-app `Internal`/`Local` members from the
  Record-receiver source-shadows-catalog scope (fail-closed)**
  (`src/program/resolve/resolver.rs`) — `resolve_member`'s `ReceiverType::
  Record` arm previously built its candidate scope (base table ∪
  `TableExtension`s) via `ResolveIndex::table_extensions_of`, which is
  whole-snapshot (`WorldMode::AnalyzedSnapshot`, no app scoping). A
  `TableExtension` declared in an app `from_object` does NOT depend on could
  therefore be added to the scope and mint a confident `Source` route to a
  symbol the real AL compiler could never have resolved (from_object's app
  never imported it) — a false `Source` is the cardinal sin. Separately, a
  cross-app SOURCE-tier candidate procedure marked `Access::Internal`/
  `Access::Local` (visible only within its own declaring app) was never
  checked against the caller's app, so it could also be counted as a
  candidate despite being AL-invisible to `from_object`. Extracted the
  scope+cardinality algorithm into a new shared helper,
  `resolve_in_table_scope` (`from_object`, `table_id`, `name_lc`, `arity`,
  `graph`, `index`, `body_map` → `Option<(DispatchShape, Vec<Route>)>`), which
  now gates BOTH the base table and every extension on
  `graph.topology.closure(from_object.id.app)` membership before counting
  candidates, and additionally excludes (via new helpers
  `object_has_visible_member_candidate`/`lookup_routine_access`) any
  cross-app candidate whose `Access` is `Local`/`Internal` — a lookup miss
  fails closed (excluded), never assumed visible. SymbolOnly (ABI-ingested
  `.app` dependency) routines are unaffected: `abi_ingest.rs` already drops
  `is_local`/`is_internal` ABI routines at ingestion, so the access filter is
  additive only for SOURCE-tier cross-app objects (e.g. a multi-app
  workspace with an embedded dependency's own AL source).
  `Access::Protected` is intentionally left unfiltered (out of scope; a
  documented gap). `resolve_member`'s `Record` arm now simply calls the
  extracted helper. Behavior is otherwise IDENTICAL — the change is
  additive-decline only (a case that previously resolved to a false `Source`
  or a false ambiguous `Unknown` now correctly declines/resolves per real AL
  visibility rules); every pre-existing passing test is unaffected. 6 new
  characterization tests verify: base+extension same-name collision →
  `Unknown`; a `TableExtension` in an app outside `from_object`'s dependency
  closure → declines (does not resolve); a cross-app `Internal`/`Local`
  method (on the base table OR an extension) → excluded; a cross-app
  `Public` method → still resolves (regression guard, proves the filter
  doesn't over-exclude). Confirmed the bug pre-fix by re-running the new
  tests against the unmodified code: 4 of 6 failed exactly as predicted, each
  resolving a false `Source` route. CDO gate: `genuine_wrong` stays `0`;
  on the real CDO semantic audit, 6 sites move from `fresh_extra` (fresh
  falsely "ahead" of the L3 reference) into `matches` (both now correctly
  decline) — a quantified, isolated soundness correction with zero
  collateral movement on `fresh_missing`/`fresh_wrong`/`genuine_wrong`.
- **(resolve) Fail-closed object resolution at the root — `resolve_object`/
  `object_by_number` are now ambiguity-aware; `resolve_object_ref`'s `Id` arm
  gained own-app-shadow; `parsed_type_to_receiver`'s declared-var `Record` arm
  is now shape-preserving; `resolve_table_id` deleted (I1)** (`src/program/graph.rs`,
  `src/program/resolve/index.rs`, `src/program/resolve/receiver.rs`,
  `tests/program_resolve_harness.rs`) — a cross-app same-name/id TABLE
  collision (two dependency apps both declaring the same table) could make
  `ProgramGraph::resolve_object`/`ResolveIndex::object_by_number` silently
  pick the lowest `ObjectNodeId` as a confident `Source` edge, potentially
  routing to the WRONG dependency's table — a false `Source` route is the
  cardinal sin (I1). Root fix (not a wrapper): both base functions now
  preserve the own-app shadow (a `from`-app declaration always wins) but
  return `None` on more than one VISIBLE-in-closure dependency match, never
  the old lowest-id tiebreak; every semantic caller (extension-base lookup,
  `ObjectRun` target resolution, typed `Object` receiver dispatch, and
  event-subscriber publisher resolution in `resolver.rs`/`index.rs`) inherits
  the fail-closed behavior automatically since the base functions themselves
  changed. A full caller audit (`rg "resolve_object\("` / `"object_by_number\("`
  across `src`/`tests`) found every call site is a genuine semantic
  AL-object-reference resolution — no non-semantic (indexing/diagnostic)
  caller existed, so no pick-first escape hatch was needed. `resolve_object_ref`'s
  `Id` arm gained the same own-app-shadow the `Name` arm already had (mirrors
  `object_by_number`'s existing self-shortcut — behavior-preserving for the
  self-declared case, newly correct for the cross-app-collision case).
  `parsed_type_to_receiver`'s `Record` arm (`var R: Record <X>` declared-type
  resolution, Caller A) now threads `from_object`'s `ObjectNodeId` and resolves
  via the shared fail-closed `resolve_object_ref`/`resolve_source_table_ref`
  helper instead of the deleted `resolve_table_id`; `ParsedType::Record` now
  carries an `ObjectRef` (extended in `classify_type_text`) instead of a bare
  lowercased string, so `Record 18` (numeric id) and `Record "18"` (a table
  literally NAMED "18") are losslessly distinguished all the way through —
  previously both collapsed to the same string `"18"` after quote-stripping,
  so a quoted digit-string table NAME was silently coerced into a guessed
  numeric id. `infer_implicit_rec`'s `TableExtension` arm (Caller B) was
  rewritten on the `resolve_pageext_base_page`/`resolve_source_table_ref`
  template (`resolve_tableext_base_table`, new). A new grep-guard test
  (`resolve_module_pick_first_base_function_callers_are_a_known_allowlist`,
  sibling to `resolve_module_has_no_stray_engine_l3_l2_imports`) locks the
  audited PRODUCTION caller set of `resolve_object`/`object_by_number` in
  `src/program/resolve/*.rs` so a future new call site must be deliberately
  classified rather than silently inheriting pick-first-shaped assumptions.
  CDO gate: `genuine_wrong` stays `0` and `real_unknown_rate` stays `2.81%`
  (unchanged) — I1 is dormant on CDO since same-name tables across a real
  compile closure are AL-illegal, so no CDO app exercises the cross-app
  collision path; the fix is validated by new unit/e2e tests instead
  (synthetic multi-app graphs, since a real buildable `.app` fixture cannot
  express an illegal same-name collision).
- **(resolve) Wire implicit Base App/System App dependency into the `src/program`
  closure — THE dominant lever for the real-`unknown` burndown (beyond-1B.3b
  Task 5.5)** (`src/dependencies.rs`, `src/snapshot/snapshot.rs`,
  `src/engine/deps/cross_app_l3.rs`, `src/program/resolve/abi_check.rs`,
  `src/program/resolve/semantic_golden.rs`, `tests/r0-corpus/ws-baseapp-closure(-control)/`
  NEW, `tests/goldens/semantic-edges/*.json`) — the `src/program` dependency-closure
  builder read ONLY the explicit app.json `dependencies[]` array and never
  converted the top-level `application`/`platform` fields (Base App / System
  App) into topology edges. Real BC apps declare Base App via `application`,
  NOT `dependencies[]`, so Base App was systematically ABSENT from every app's
  closure — every cross-Microsoft-layer call (PageExtensions, `Record`/`Codeunit`
  vars typed at a Base App object, …) resolved `OutOfClosure` → an honest but
  wrong `Unknown`. `crate::dependencies::append_implicit_ms_tier_deps` now
  appends implicit `AppDependency` rows for `MS_APPLICATION_TIER`
  (Base App `437dbf0e-84ff-417a-965d-ed2bb9650972` + Foundation/System-tier)
  when `application` is non-empty, and `MS_PLATFORM_TIER` (System App
  `63ca2fa4-4f03-4f2b-a480-172fef340d3f`) when `platform` is non-empty — reusing
  the GUID/name tier DATA already defined in `engine::deps::cross_app_l3`
  (now `pub(crate)`, mirroring the existing `engine::l3::global_builtins` reuse
  precedent) rather than duplicating it. Wired at BOTH `declared_deps`
  construction sites in `SnapshotBuilder::build` (the workspace unit AND every
  dependency unit — a dep can itself implicitly depend on Base App/System App
  via its own manifest), with a self-referential guard (an app never gains
  itself as an implicit dependency) and NO injection when `application`/
  `platform` is absent or empty (fixtures with a minimal app.json are
  unaffected). Two related pre-existing latent bugs, dormant only because
  Base App was never reachable before, were surfaced and fixed as part of this
  change: (1) `abi_check.rs`'s ABI-ingestion-integrity check flagged
  `resolve_object_run`'s implicit-entry-trigger Opaque-fallback keys
  (`onrun`/`onopenpage`/`onprereport`) as "unmapped" — entry triggers
  structurally never appear in a `.app`'s `SymbolReference.json` `Methods`
  array (verified against real Base Application source: two Warehouse pages
  carry zero `Methods` entries) for ANY BC app, so this was never a real
  ingestion bug; `is_entry_trigger_boundary_key` now exempts this exact key
  shape. (2) Base App ships full embedded (ShowMyCode) AL source, so two
  newly-reachable calls (`SalesInvHeader.SendRecords()`,
  `CustomerConsentMgt.ConfirmUserConsent()`) resolved correctly with
  `Evidence::Source` to real Base App procedures — independently verified by
  extracting and reading Base App's actual embedded source — while L3's frozen
  golden paired the same two sites with unrelated targets (a collapsed nested-
  event-subscriber set for the first; a different call's target entirely for
  the second, an L3 site/line-bookkeeping defect). Both are genuine L3-golden
  defects, not fresh bugs, and are adjudicated via the SAME `adjudicated-
  overrides.json` mechanism beyond-1B.3b Task 3 established (a new
  `CrossAppSourceProcedure` target shape alongside the existing
  builtin-catalog shape; `known-genuine-divergences.json` grows from 42→44
  entries, independently re-verified at test time against Base App's real
  source, never against a fresh-computed edge). CDO (`CDO_WS`): primary
  real-`unknown` rate 6.50%→3.30% (whole-program 2.75%→1.39%) — a LARGE drop,
  as expected; `fresh_missing` 176→174 (workspace-internal buckets Tasks
  4-7 own; Base App closure's effect is almost entirely on the rate, not this
  narrower L3-paired-completeness metric); `genuine_wrong` stays 0 (adjudicated,
  not whitelisted — every new divergence was independently source-verified
  fresh-correct before adjudication). 3 new `dependencies.rs` unit tests + 3
  new `snapshot.rs` `AppUnit`-level tests + 1 new `abi_check.rs` exemption
  unit test (+ a negative control) + 2 new end-to-end fixtures
  (`ws-baseapp-closure`/`ws-baseapp-closure-control`, a hand-built synthetic
  Base App `.app`) proving the positive (application field present → resolves)
  and negative (absent → stays honest `Unknown`) cases.

### Added
- **beyond-1B.3b Task 8 (CAPSTONE): the real-`unknown` burndown arc is
  closed — re-measured, ratchets tightened to the new floor,
  `engine::l3`/`engine::l2` grep-guard added** (`tests/program_resolve_harness.rs`;
  no resolver source changes — this task is verification + ratchet + docs, by
  design) — closes the beyond-1B.3b real-`unknown` burndown arc (Tasks 1–7 +
  inserted 5.5, all already individually logged above). Summary of the whole
  arc, before/after:
  - **Task 1**: lookup PRECEDENCE fix — a workspace/dependency SOURCE
    definition now shadows the global builtin catalog (was: builtin catalog
    checked first, silently hiding a same-named user procedure) — plus a
    structural (name+arity-shaped, not string-matched) builtin-catalog match.
  - **Task 2**: fail-closed SAME-ARITY OVERLOAD guard — `resolve_in_object`
    no longer picks the first candidate when N>1 source overloads share
    `(object, name_lc, params_count)`; collision-aware index preserves every
    raw entry instead of dropping one, and >1 arity-matched candidates fails
    closed to `Unresolved` (mirrors the interface fan-out `>1 → Unresolved`
    rule) rather than guessing.
  - **Task 3**: PRECEDENCE-ADJUDICATED `genuine_wrong=42` via a source-identity
    overlay (`adjudicated-overrides.json`) — the frozen L3 golden stays
    UNTOUCHED (never edited/rebaselined); at audit time the adjudicated
    target is substituted in-memory before diffing, so fresh matches by
    construction, backed by an INDEPENDENT re-derivation test
    (`cdo_genuine_wrong_is_precedence_adjudicated`) that re-hashes the source,
    re-confirms the call shape/receiver-kind/arity, and re-derives the
    verdict from the structural catalog — never trusting the committed
    override's own fields.
  - **Task 4**: `ObjectNode` FIDELITY groundwork (`SourceTable`/`TableNo`/
    `page_controls`/`is_temporary`, structured `ObjectRef`) + `objects_by_id`
    index + the ONE shared fail-closed `resolve_object_ref` helper
    (`Unique`/`Ambiguous`/`OutOfClosure`/`Unresolved`) that Tasks 5–7 all
    build on — pure additive groundwork, zero resolution behavior change on
    its own.
  - **Task 5**: Page/PageExtension implicit `Rec` via `ObjectNode.source_table`
    (the `Rec.Method()` MEMBER-call shape), topology-aware fail-closed;
    Report/ReportExtension deliberately excluded (dataitem-scoped, not a
    single object-level table — still open, see below).
  - **Task 5.5 — THE DOMINANT LEVER**: wired the IMPLICIT Base App/System App
    dependency into the `src/program` closure. Real BC apps declare Base App
    via the top-level `application` manifest field, NOT `dependencies[]` —
    the closure builder read only the latter, so Base App (and every
    cross-Microsoft-layer call into it: PageExtensions, `Record`/`Codeunit`
    vars typed at a Base App object, …) was systematically unreachable,
    resolving an honest but wrong `Unknown`. This ONE fix moved the primary
    real-`unknown` rate **6.50%→3.30%** (whole-program 2.75%→1.39%) — by far
    the largest single jump in the arc, confirming the north-star hypothesis
    that most residual `unknown` was a missing-dependency-edge problem, not a
    resolution-logic problem.
  - **Tasks 6/7**: closed the remaining charter-§3-node-fidelity receiver
    gaps — Codeunit implicit `Rec` via `ObjectNode.table_no` (Task 6,
    `TestRunner` subtype honest-declined) and `CurrPage.<part>.Page`
    subpage-instance compound receivers (Task 7, control-vs-subpage-instance
    distinction preserved, `SystemPart`/`UserControl` deliberately excluded).
  - **Net result** (CDO `CDO_WS`, RE-MEASURED and CONFIRMED 2026-07-01 for
    this Task 8 closing report — every number below independently reproduced
    against the live workspace, not merely carried forward):
    primary real-`unknown` rate **6.46%→2.81%** (`unknown` 508/18104);
    whole-program **1.19%** (`unknown` 508/42843) — the whole-program arc
    trajectory, chained from each task's own logged before/after (Task 2's
    soundness correction 2.73%→2.80%, Task 3/4 unchanged at 2.80%, Task 5.5
    2.75%→1.39% — the small 2.80%→2.75% step is Task 5's own contribution,
    Task 6 1.39%→1.34%, Task 7 1.34%→1.19%); no isolated whole-program figure
    was separately logged for Task 1 alone, so "whole-program pre-arc" is not
    cited as a single round number here — only primary's 6.46% carries that
    role (the number the original `<= 0.07` ceiling was set against). L3
    semantic audit `fresh_missing`
    **191→102**; the `genuine_wrong` CANDIDATE set stayed exactly constant
    across Tasks 1–2 (42→42, "no new divergence", per Task 2's own
    before/after) — no task ever introduced a NEW disjoint divergence beyond
    the 1B.3a-era 42 — and from Task 3's precedence-adjudication overlay
    onward the AUDIT's reported `genuine_wrong_count` is exactly **0** (the
    42 sites are adjudicated `l3_error_intrinsic` L3-golden defects, matched
    by construction against the overlaid target) through every subsequent
    task including this one; Task 5.5 grew the manifest 42→44 (2 NEW
    `CrossAppSourceProcedure` L3-golden defects it surfaced), independently
    source-verified against real Base App source, never whitelisted —
    `genuine_wrong_count` stayed 0 after that growth too. `fresh_wrong=139`
    (all 139 adjudicated `fresh_ahead_dispatch` — fresh REFINES L3, not a
    divergence).
  - **What stays honestly `Unknown`** (unchanged by this task; the residual
    is CHARACTERIZED here, not fixed — fixing it is the next plan): Task 8
    live-minted the L3-validated golden and diffed the 102-site
    `fresh_missing` residual site-by-site (throwaway diagnostic, not
    committed — see `.superpowers/sdd/task-8-report.md`) and source-verified
    the dominant pattern directly against real CDO source: **82/102 sites**
    are a BARE (unqualified) call inside a Page/Report trigger that should
    implicitly dispatch to the object's own `SourceTable`'s global
    procedures when no local/extension-base/builtin match exists — e.g.
    `Page 6175272 "CDO E-Mail Templates"`'s `OnAfterGetRecord` calls bare
    `GetReportSelection()`/`GetReportName()`, both defined on
    `SourceTable = "CDO E-Mail Template Header"` (table 6175283). This is
    `resolve_bare`'s own documented `// 3. Implicit-Rec (deferred)` TODO
    (`src/program/resolve/resolver.rs`) — a DIFFERENT, never-built gap from
    the `Rec.Method()` explicit MEMBER-call implicit-Rec Tasks 5/6 already
    closed. **12/102 sites** are a bare call to a procedure on the caller's
    OWN object from a NESTED field-level trigger (e.g. `Table 6175281
    "CDO Setup"`'s `"Azure Blob Container Name"` field's `OnValidate` calls
    bare `CheckAzureContainerPerCompany()`, an `internal procedure` on the
    SAME table's top level) — root cause not yet isolated, a candidate being
    the TableExtension-arm fail-closed consistency pass (next plan, below).
    The remaining 8 are overload sets mixing a same-object and a cross-object
    candidate. Also still honestly `Unknown`/deferred, unchanged from prior
    tasks: Report/ReportExtension implicit `Rec` (dataitem block-scope, not
    object-level — Task 5 explicitly excluded it); `TestRunner` Codeunit
    subtype (Task 6 explicitly declined it); deep compound-receiver chains
    beyond one `.Page` hop and `SystemPart`/`UserControl` controls (Task 7
    explicitly declined them); cross-app-ambiguous tables (`Ambiguous` by
    `resolve_object_ref`'s design, Task 4); the pre-existing L3-golden
    span/line-offset (`known-genuine-divergences.json`'s adjudication
    already accounts for this independently, unrelated to Task 8's ratchets).
    Full same-arity-TYPE overload DISPATCH remains the genuinely-ambiguous
    `Variant`-typed-arg case (out of scope for the whole arc, tracked
    separately, fixture at `tests/r0-corpus/ws-overload-arg-type/`).
  - **Ratchets tightened** (`tests/program_resolve_harness.rs`,
    `cdo_full_program_coverage_and_self_reported_metric` +
    `cdo_l3_semantic_audit_no_fresh_wrong`; a ratchet never loosens):
    `primary_rate <=` **0.07 → 0.030** (measured 0.0281, dated comment); NEW
    primary `unknown` COUNT ceiling **`ph.unknown <= 520`** (measured 508) —
    a ratio ceiling alone can hide a regression if `total` also shifts, a
    count catches it; NEW companion whole-program `unknown` COUNT ceiling
    **`h.unknown <= 520`** (measured 508, defense-in-depth for a future
    dependency-internal regression the primary-scoped count alone would
    miss); `FRESH_MISSING_CEILING` **191 → 110** (measured 102, breakdown
    comment rewritten with the 82/12/8 source-verified characterization
    above, superseding the stale 1B.3a-era `page_rec=115+
    codeunit_implicit_rec=24+trigger=38+other=14` breakdown that Tasks 5–7
    had already substantially drained); NEW divergence ratchet:
    `genuine_wrong == 0` stays the pre-existing HARD gate (unchanged, still
    exact-zero, never "still-acceptable known wrongness"), plus a NEW
    `fresh_wrong` COUNT ceiling **`<= 150`** (measured 139) — `genuine_wrong`
    alone cannot see a new confidently-wrong edge that happens to also pass
    the (heuristic) `fresh_ahead_dispatch` refinement test; pinning the
    `fresh_wrong` total is defense-in-depth so such a site still trips a
    review.
  - **NEW grep-guard test** — `resolve_module_has_no_stray_engine_l3_l2_imports`
    (no `CDO_WS` needed, always runs) closes the "convention-only, no CI
    enforcement" gap two reviewers flagged against 1B.3b Task 3's invariant.
    It scans every `.rs` file directly under `src/program/resolve` (flat
    directory, verified no subdirectories) except the ONE sanctioned
    `builtins.rs::global_builtins` exception, strips `//`/`///`/`//!`
    comments per line (so the several files' module docs that legitimately
    NAME `engine::l3`/`engine::l2` in prose — `differential.rs`,
    `semantic_golden.rs`, `member_catalog.rs` — do not false-positive), and
    fails on any remaining `engine::l3`/`engine::l2` substring in actual
    code. Verified zero offending imports today (matches the existing
    `builtins.rs`-only baseline this task independently re-confirmed via
    manual `grep`); a `scanned_files > 5` sanity assertion guards against the
    test passing vacuously if directory listing ever silently breaks.
  - **No `engine::l3`/`engine::l2` import added by this task** (grep-guarded,
    self-verified — the new test itself asserts this).
  - **Gates** (all FOREGROUND, this task): `cargo test --workspace` (no
    `CDO_WS`) — 51/51 `program_resolve_harness` tests pass (50 pre-existing +
    1 new grep-guard), full workspace green; `cargo clippy --release
    --all-features -- -D warnings` — clean; `cargo fmt --check` — clean;
    (`CDO_WS` + `ENFORCE_CDO_WS=1`, single-test runs, release profile — CDO
    tests cannot run concurrently, unrelated pre-existing constraint) all six
    CDO gates green + deterministic under the tightened ratchets:
    `cdo_full_program_coverage_and_self_reported_metric`,
    `cdo_l3_semantic_audit_no_fresh_wrong`, `cdo_trigger_audit_frozen_load`
    (`matches=185`, `fresh_wrong=0`, unchanged), `cdo_event_audit_frozen_load`
    (`matched_pairs=2`, unchanged), `route_applicability_zero_violations`
    (`total_routes=17646`, `violations=0`), `fan_out_applicability_zero_violations`
    (all four violation counters `0`, non-vacuous
    `routes_checked[interface=28 instance_builtin=455 implicit_trigger=1183
    event=2464]`), `cdo_genuine_wrong_is_precedence_adjudicated`
    (`l3_error_intrinsic=44`, `fresh_false_builtin=0`, `needs_manual_review=0`).
  - **Next plan** (unchanged scope from the roadmap, now with the Task 8
    residual characterization sharpening it): the BARE-call implicit-Rec
    dispatch (`resolve_bare` Step 3 — the now-dominant 82/102 residual
    bucket), full same-arity-TYPE overload DISPATCH, Report implicit-`Rec`
    with dataitem block-scope context, and a TableExtension-arm fail-closed
    consistency pass (candidate root cause for the 12/102 same-object nested-
    trigger residual).

- **(resolve) `CurrPage.<part>.Page` subpage-instance receivers, control-aware
  fail-closed (`regression_compound_receiver`, beyond-1B.3b Task 7)**
  (`src/program/resolve/receiver.rs`, `src/program/resolve/resolver.rs`,
  `tests/r0-corpus/ws-compound-receiver/` NEW, `tests/program_resolve_harness.rs`)
  — `infer_receiver_type` matched the WHOLE lowercased receiver text against its
  arms, so a compound receiver like `"currpage.lines.page"` never matched
  anything and fell through to `Unknown` (the `compound_receiver` bucket, ≈47
  CDO sites). A new Step 0 recognizes EXACTLY the `<part>.Page` shape (one
  control segment + one trailing `.Page` accessor, quoted or unquoted, via a
  new `parse_currpage_dot_page_segment` parser): a `Part` control's `target`
  resolves through the fail-closed `ResolveIndex::resolve_object_ref` (Task 4)
  to the subpage Page object — the CONTROL-vs-SUBPAGE-INSTANCE distinction a
  prior reviewer flagged is load-bearing: `CurrPage.<part>` alone (no `.Page`)
  addresses the CONTROL (`.Update`/`.Visible`, structural methods) and is
  deliberately NOT modeled here; a `SystemPart`/`UserControl` control, an
  unknown part name, a chain deeper than one `.Page` accessor, or a
  non-`Unique` target resolution all fall through to honest `Unknown` rather
  than fabricate a route (a wrong subpage is a false `Source` edge, the
  cardinal sin). A PageExtension with no matching control of its own also
  consults its extended BASE page's controls (`find_page_control`, mirroring
  L3's `symbol_table::page_controls_for` merge) via a new shared
  `resolve_pageext_base_page` helper, factored out of (and now reused by) the
  existing Task 5 `resolve_pageext_base_source_table`. `ReceiverType::Object`
  gains a third field, `id: Option<ObjectNodeId>`, so Step 0 carries the
  resolved id MECHANICALLY rather than re-deriving it by name; `resolve_member`'s
  `Object` arm short-circuits on a present `id` (bypassing `graph.resolve_object`
  by-name entirely) — proven by a new unit test that supplies a deliberately
  WRONG `name_lc` alongside a valid `id` and confirms resolution still follows
  the id. 20 new `receiver.rs` unit tests (positive incl. quoted control name,
  all 5 negative shapes, PageExtension base-control inheritance, low-level
  parser edge cases) + 1 new `resolver.rs` id-short-circuit unit test + 1
  end-to-end `tests/r0-corpus/ws-compound-receiver/` fixture (9 call
  obligations in one routine: 1 positive + 8 negatives covering every
  declined shape) asserting the exact positive route and that all 8 negatives
  stay `Unknown`. CDO (`CDO_WS`): primary real-`unknown` rate 3.17%→2.81%
  (whole-program 1.34%→1.19%, `unknown` 573→508, a 65-site drop); the L3
  semantic audit's `fresh_missing` drops 150→102 with `genuine_wrong` staying
  `0` before and after (soundness backstop unaffected, `matches` 6324→6360).
  Sample-adjudicated 39+16 real CDO sites (`CDOActions.Page.HideActions` across
  16 PageExtensions incl. a PageExtension-owned Part control;
  `EMailTemplateLines.Page.SetVariantCaption`; `UserSetupSubPage.Page.
  CreateUpdateTempRecs`/`.Changed`; `ConfigLines.Page.LoadConfigFromOnline`/
  `LoadConfigFromFile`/`CreateTempTable`/`Import` resolving CROSS-APP into a
  dependency's Page object; `TemplateMergeFields.Page.SetMergeFields`;
  `ConflictSubform.Page.UpdateProgress`) hand-verified line-for-line against
  the real CDO source and the target pages' declared procedures, plus a
  qualitative check that CDO's abundant bare-control/`UserControl` sites
  (`CurrPage.HTMLEditor.SetHTML(...)`, `CurrPage.PrintService.Configure(...)`,
  `CurrPage.WebPageViewer.SetContent(...)`, etc. — no `.Page`) do not appear
  among the newly-resolved routes. Deterministic across two runs (`cargo test
  --workspace`, no `CDO_WS`, stays fully green) — the `ReceiverType::Object`
  field addition rippled to ~15 existing test constructions (all updated to
  `id: None`), zero other existing assertions changed.
- **(resolve) Codeunit implicit `Rec` via `ObjectNode.table_no`, fail-closed;
  `TestRunner` honest-declined (beyond-1B.3b Task 6)**
  (`src/program/resolve/receiver.rs`, `tests/r0-corpus/ws-codeunit-rec/` NEW,
  `tests/program_resolve_harness.rs`) — the direct analog of Task 5:
  `infer_implicit_rec`'s Codeunit arm used to unconditionally return `Unknown`
  (Codeunit had no arm at all). It now resolves the object's own `table_no`
  through the fail-closed `ResolveIndex::resolve_object_ref` (Task 4): a
  single unambiguous in-closure match yields `Record{table: Some(id)}`; a
  declared-but-unresolved `TableNo` (cross-app name ambiguity, out-of-closure)
  stays `Record{table: None}` — mirroring Page's non-`Unique` treatment,
  since a Record entity DOES exist there and builtins still resolve
  table-independently. This differs from Page in one deliberate way: a
  Codeunit only gets an implicit-Rec entity AT ALL when `TableNo` is declared
  — no `TableNo` (including `Subtype = Test`/`TestRunner` codeunits, which
  never declare one; no statically-typed implicit Rec for them, unhandled
  even in the legacy L3 engine) stays the honest `Unknown`, never
  `Record{table: None}` (there is no Record entity to type in the first
  place). `ObjectNode` does not track `Subtype` at all — the `TableNo`
  presence check alone already produces the correct decline for
  Test/TestRunner codeunits, nothing fabricated. 4 new `receiver.rs` unit
  tests (own-table unique/no-`TableNo`/ambiguous/out-of-closure, reusing Task
  5's page-rec fixture topology) + 5 new end-to-end
  `tests/r0-corpus/ws-codeunit-rec/` fixtures covering the positive case,
  three negatives (no `TableNo`; `Subtype = TestRunner`; cross-app ambiguous
  `TableNo` across two dependency apps sharing a table name), and a
  local-`var`-shadow case. CDO (`CDO_WS`): primary real-`unknown` rate
  3.30%→3.17% (whole-program 1.39%→1.34%); the L3 semantic audit's
  `fresh_missing` drops 174→150 (a 24-site drop, matching the
  `codeunit_implicit_rec` bucket size exactly) with `genuine_wrong` staying
  `0` before and after (soundness backstop unaffected) — 5 sample sites
  across 2 distinct Codeunit/Table pairs hand-verified against the real CDO
  source (`CDO Queue Management`→`CDO Queue Entry.HandleEntry`, `CDO Merge
  Field Value Finder`→`CDO E-Mail Codeunit Parameter.SetReturnValue` ×4), all
  confirmed correct. Deterministic across two runs (`cargo test --workspace`,
  no `CDO_WS`, stays fully green); incidentally refreshed one pre-existing
  golden entry (`ws-baseapp-closure/src/WsBaseCaller.Codeunit.al::0::Run`)
  that had drifted from unrelated `tree-sitter-al` grammar movement, verified
  present on clean HEAD before this task's changes.
- **(resolve) Page/PageExtension implicit `Rec` via `ObjectNode.source_table`,
  topology-aware fail-closed (beyond-1B.3b Task 5)**
  (`src/program/resolve/receiver.rs`, `tests/r0-corpus/ws-page-rec/` NEW,
  `tests/program_resolve_harness.rs`) — `infer_implicit_rec`'s Page arm now
  resolves the object's own `source_table` through the fail-closed
  `ResolveIndex::resolve_object_ref` (Task 4): a single unambiguous in-closure
  match yields `Record{table: Some(id)}`; anything else (no `SourceTable`
  property, cross-app name ambiguity, out-of-closure) stays `Record{table:
  None}` — a guessed table would be a false `Source` edge, so this fails
  closed, never guesses. A PageExtension with no own `SourceTable` inherits by
  resolving its `extends` target to exactly one in-closure base Page (same
  fail-closed rule) and reading THAT page's `source_table`; an own
  `SourceTable` that fails to resolve does NOT fall through to the base page
  (an explicit override that declines stays declined). Report/ReportExtension
  are deliberately EXCLUDED — a report's implicit Rec is scoped PER-DATAITEM,
  not a single object-level `SourceTable`, so they keep returning
  `Record{table: None}` unconditionally (a future task). Builtins
  (`FieldCaption`/etc., table-independent per the `ReceiverType::Record` doc)
  and `record_op_names` calls (`SetRange`/`FindSet`/etc., a separate
  implicit-trigger dispatch path) are unaffected either way; only a
  NON-builtin method call on a now-resolved table flips from honest `Unknown`
  to `Source`. 8 new `receiver.rs` unit tests (own-table unique/ambiguous/
  out-of-closure, PageExtension inherit/override/dangling-extends, Report
  exclusion even when a `source_table` is defensively present) + 5 new
  end-to-end `tests/r0-corpus/ws-page-rec/` fixtures covering the positive
  case, both negatives (no `SourceTable`; cross-app ambiguous `SourceTable`
  across two dependency apps sharing a table name), a local-`var`-shadow case,
  and the Report exclusion. CDO (`CDO_WS`): primary real-`unknown` rate
  6.62%→6.50% (22 sites flip `Unknown`→`Source`, all hand-verified against the
  real CDO source incl. one genuine cross-app resolution); the L3 semantic
  audit's `fresh_missing` drops 191→176 with `genuine_wrong` staying `0` both
  before and after (soundness backstop unaffected) — deterministic across two
  runs (`cargo test --workspace`, no `CDO_WS`, stays fully green).
- **(resolve) Object node fidelity (`SourceTable`/`TableNo`/page-controls/
  `is_temporary`) + `objects_by_id` index + fail-closed `resolve_object_ref`
  (beyond-1B.3b Task 4)** (`src/program/node_extract.rs`,
  `src/program/resolve/index.rs`) — pure additive groundwork for Tasks 5–7
  (Page/Codeunit implicit-`Rec`, `CurrPage.<part>`); no consumer yet, zero
  resolution behavior change. `ObjectNode` gains `source_table`/`table_no`:
  `Option<ObjectRef>` where `ObjectRef` losslessly distinguishes a numeric AL
  id (`SourceTable = 36` → `Id(36)`) from a name (`SourceTable = "Sales
  Header"` → `Name{raw, normalized_lc}`), `source_table_temporary: bool` (a
  trailing `, Temporary` / ` temporary` marker on the `SourceTable` value,
  stripped losslessly — requires an explicit separator so a table literally
  named `MyTemporary` is never truncated), and `page_controls:
  Vec<PageControlNode>` (`part`/`systempart`/`usercontrol` sections, document
  order, `PageControlKind` + `ObjectRef` target). Populated in `extract_nodes`
  from the IR's `ObjectDecl.properties`/`page_controls`, scoped per object kind
  (`SourceTable` for Page/PageExtension/Report/ReportExtension, `TableNo` for
  Codeunit, `page_controls` for Page/PageExtension only). `ResolveIndex` gains
  two new GLOBAL (whole-snapshot) grouped indexes, `objects_by_id: HashMap<
  (ObjectKind, i64), Vec<ObjectNodeId>>` and `objects_by_name: HashMap<
  (ObjectKind, String), Vec<ObjectNodeId>>`, built in the same pass as the
  existing `objs_by_number` (which is left unchanged for its existing
  self-preferred/best-tiebreak callers) — these feed the new
  `resolve_object_ref(graph, from, kind, &ObjectRef) -> ObjectRefResolution`,
  the ONE shared helper Tasks 5–7 will call, returning `Unique(ObjectNodeId)` /
  `Ambiguous` / `OutOfClosure` / `Unresolved`. Fail-closed by construction:
  only `Unique` ever carries an id. An `Id` ref matches the same `ObjectKind`
  only, closure-filtered, with NO shadow priority (two in-closure declarations
  of the same numeric id — an anomaly a merged whole-program snapshot can
  surface even though a real AL compile never would — is `Ambiguous`, not
  guessed). A `Name` ref matches by kind + lowercased name; an object declared
  in `from`'s own app always shadows a same-named dependency object (mirrors
  the existing self-preference in `object_by_number`/`resolve_object`), so two
  DEPENDENCY apps sharing a name (neither is `from` itself) is `Ambiguous`.
  `OutOfClosure` (declared somewhere in the snapshot, just unreachable from
  `from`) is kept distinct from `Unresolved` (never declared with that
  kind+id/name at all) — a more informative decline for Tasks 5–7 to reason
  about. 15 new unit tests (7 node-lowering + 8 `resolve_object_ref`,
  including a cross-app id/name collision and a two-independent-builds
  determinism check); `cargo test --workspace` (no `CDO_WS`) stays fully green
  — no existing test's assertions changed.

### Fixed
- **(resolve) Precedence-adjudicate `genuine_wrong=42` via a source-identity
  overlay — L3 golden UNTOUCHED (beyond-1B.3b Task 3)**
  (`tests/goldens/semantic-edges/adjudicated-overrides.json` NEW,
  `tests/goldens/semantic-edges/known-genuine-divergences.json`,
  `src/program/resolve/semantic_golden.rs`, `tests/program_resolve_harness.rs`) —
  the 42 CDO `genuine_wrong` sites (fresh classifies the call a platform
  `builtin`; the frozen L3 golden `cdo-anon.json` emits a source-procedure
  target for the same callee) were adjudicated by DIRECTIONALITY, INDEPENDENTLY
  of fresh's output: for each site, open the CDO source at `(unit, line)`, read
  the actual call syntax + receiver, confirm the claimed name+receiver-kind is a
  real member of the STRUCTURAL builtin catalog (`builtins::is_global_builtin` /
  `member_catalog::member_builtin`), and grep the SAME unit for a competing local
  `procedure <name>(` declaration (the Task-1 lookup-precedence shadow check).
  Result: all 42 are `l3_error_intrinsic` (fresh is CORRECT — a genuine
  intrinsic that L3 mis-resolved to a coincidentally-named source routine); ZERO
  `fresh_false_builtin`, ZERO `needs_manual_review`. Corrections live in a NEW
  SEPARATE overlay `adjudicated-overrides.json` (canonical catalog keys
  name+arity+receiver-kind + a `source_sha256` per unit + a human note — NEVER a
  serialized fresh edge/route/graph-node id); `cdo-anon.json` is left byte-for-byte
  UNTOUCHED. `run_cdo_semantic_audit` now loads `cdo-anon.json`, applies the
  overlay IN-MEMORY (`apply_adjudicated_overrides` — replaces the L3 target of
  each `l3_error_intrinsic` site with the adjudicated `Builtin` catalog target),
  then diffs fresh against the OVERLAID oracle: `genuine_wrong` drops 42→0
  (`fresh_wrong=132`, all `fresh_ahead_dispatch`), with the resolver's own output
  UNCHANGED (whole-program `real_unknown_rate=2.80%`, primary `6.62%`,
  `resolved_source=8607`, `unknown=1199` — identical to the Task-2 baseline;
  `fresh_missing=191` ceiling holds; audit deterministic, `paired=11377>0`). New
  CDO-gated test `cdo_genuine_wrong_is_precedence_adjudicated` RE-DERIVES every
  verdict from LIVE source + the catalog at test time (never from fresh, never
  from the overlay's own committed fields), FAILS LOUDLY on any `source_sha256`
  drift (CDO_WS is a dirty live workspace), and asserts 0 `fresh_false_builtin` /
  0 `needs_manual_review` (fail-closed — an unresolved dimension is never
  auto-passed). The bare `assert_eq!(manifest_len, 42)` was replaced with full
  manifest+overlay invariants (per-entry `verdict`/`callee_text`/`source_sha256`,
  no dup site keys, every `l3_error_intrinsic` has a matching overlay entry and
  vice-versa, and a testable non-circularity guard: overlay entries carry NO
  fresh-edge-id-shaped field). All invariant/metadata checks are UNCONDITIONAL
  (pass without `CDO_WS`, public CI).
- **(resolve) Fail-closed same-arity SOURCE-overload guard — node soundness
  prerequisite (beyond-1B.3b Task 2, incl. review-fix pass)**
  (`src/program/build.rs`, `src/program/node_extract.rs`,
  `src/program/abi_ingest.rs`, `src/program/resolve/resolver.rs`,
  `src/program/resolve/index.rs`, `src/program/resolve/applicability.rs`,
  `src/program/resolve/semantic_golden.rs`,
  `tests/r0-corpus/ws-overload-collision/` NEW, `tests/program_resolve_harness.rs`,
  `tests/ir-l2-goldens/l2_features.snapshot`, `tests/parser-ir-goldens/projection.snapshot`) —
  `RoutineNodeId.sig_fp` is always `0` for source-bearing routines, so two
  DISTINCT source overloads sharing `(object, name_lc, params_count)` (same
  name+arity, differing only by param TYPE) collide onto one `RoutineNodeId`.
  `build_program_graph`'s post-sort `dedup_by` then silently dropped one of
  them with no record, and `resolve_in_object` picked the FIRST arity-matched
  candidate with no ambiguity check — a confident `Source` route to a
  collapsed/pick-first node. Fixed in two parts: (1) `build.rs` now computes
  each object's raw duplication factor BEFORE any dedup runs (the yardstick
  that separates a legitimate whole-file re-parse — e.g. a sibling app
  embedded as both workspace source and compiled dep — from a genuine
  same-arity overload collision) and `dedup_routines_preserving_genuine_overloads`
  preserves EVERY raw entry in a collision run instead of collapsing it, so
  `ResolveIndex`'s existing `routines_by_obj_name` collection sees the true
  candidate count with no signature/API changes needed anywhere downstream;
  (2) `resolve_in_object` now collects ALL arity-matched candidates and
  branches on count — exactly one resolves as before, zero or **more than
  one** returns honest `Unresolved`/`Evidence::Unknown` (mirroring the
  interface-implementer fan-out's pre-existing `>1 → Unresolved` rule) —
  applied uniformly to every caller (`resolve_bare`'s own-object/extension-base,
  member `Object`/`SelfObject` dispatch) since they all delegate through the
  one function. Full arg-type DISPATCH to disambiguate remains explicitly out
  of scope (no arg types are captured yet) — this only prevents a
  confident-WRONG `Source` edge to a collapsed/guessed node, never fabricates
  a resolution. New fixture `tests/r0-corpus/ws-overload-collision/` (two
  `Resolve(Integer)`/`Resolve(Code[20])` overloads + a single-overload control
  target) pins: the ambiguous call resolves honest `Unknown` (not a guessed
  `Source`), both raw overloads survive the graph build (`graph.routines`
  contains 2 `resolve` entries, not 1), and the control case still resolves
  cleanly. CDO re-measurement (`CDO_WS`, isolated single-test runs, before/after
  diffed via a temporary revert): a clean, isolated correction of exactly 30
  previously-confident pick-first `Source` edges → honest `Unknown`
  (`resolved_source` whole-program 8637→8607, `unknown` 1169→1199; primary
  `real_unknown_rate` 6.46%→6.62%, still inside the existing 0.07 regression
  ceiling) with **zero** change to every other histogram bucket, to
  `genuine_wrong` (42→42, exact manifest match, no new divergence), or to the
  `fresh_missing` completeness ceiling (191→191) — a pure soundness
  correction, not a regression.
  **Review-fix pass (compound object-duplication × overload dedup):** review
  found `dedup_routines_preserving_genuine_overloads` was binary per
  duplicate-id run (collapse the whole run to 1, or keep every entry) — in
  the COMPOUND case where an object is embedded BOTH as workspace source AND
  an embedded dep (`obj_dup=2`) AND declares a genuine same-arity overload
  pair, a run of 4 raw entries (2 overloads × 2 copies) was kept in full
  instead of collapsing to the canonical 2, and `ResolveIndex::build`'s
  `routine_by_id: HashMap<RoutineNodeId, usize>` silently lost one
  physical routine's `publisher_kind` on the second `insert` whenever two
  canonical routines shared an id — together these could inflate
  `graph.routines`/event-flow obligations for a duplicated publisher, or
  push a LEGITIMATE single-target event subscription into
  `ambiguous_subscriptions` and drop it. Root-caused by making the dedup
  CONTENT-AWARE instead of a duplication-factor heuristic: `RoutineNode`
  gained `param_sig_key` (the lowercased, `|`-joined parameter-type-text
  sequence, computed at extraction time, mirroring
  `abi_ingest::param_type_fp`'s normalization for source params), and
  `dedup_routines_preserving_genuine_overloads` now collapses a run to one
  canonical entry PER DISTINCT signature — correct regardless of how many
  times the object itself was duplicated (no `obj_dup` counting needed
  anymore; the pre-pass `HashMap<ObjectNodeId, usize>` computation was
  removed). `ResolveIndex::build`'s event-subscriber index now groups
  `graph.routines` INDICES (not lossy `RoutineNodeId` keys) per
  `(object, name_lc)`, so a `publisher_kind` lookup can never collapse two
  physical routines sharing an id into one. New fixture (hand-built
  `AppSetSnapshot` with the same app identity present twice — one workspace
  unit, one synthetic embedded-dep unit — both embedding an object with a
  genuine `Resolve(Integer)`/`[IntegrationEvent] Resolve(Text)` overload
  pair) proves the compound case: `graph.routines` holds exactly 2 canonical
  `Resolve` entries (not 4), and a legitimate single-target `OnResolve`
  subscription resolves cleanly (`ambiguous_subscriptions` stays empty)
  where it was previously mis-flagged ambiguous with `candidate_count=4`;
  both assertions confirmed failing against the pre-fix code before the fix
  landed. CDO re-run (`CDO_WS`, single isolated test) shows the original
  Task 2 correction preserved exactly, byte-for-byte: `resolved_source=8607`,
  `unknown=1199`, primary `real_unknown_rate=6.62%` — no new drift.
- **(resolve) Source shadows builtin — lookup-precedence soundness fix +
  structural builtin-catalog match (beyond-1B.3b Task 1, incl. review-fix
  pass)**
  (`src/program/resolve/resolver.rs`, `src/program/resolve/builtins.rs`,
  `src/program/resolve/member_catalog.rs`, `tests/r0-corpus/ws-builtin-shadow/`
  NEW, `tests/r0-corpus/ws-builtin-shadow-arity/` NEW,
  `tests/program_resolve_harness.rs`) — `resolve_member`'s `Record`
  receiver arm was **catalog-FIRST**: a user/source table procedure whose
  name+arity coincided with a genuine platform-intrinsic Record method (e.g.
  `FieldNo`, `SetRecFilter`) was mis-classified `Evidence::Catalog` instead of
  the correct `Evidence::Source` — AL semantics say a visible source/ABI
  routine SHADOWS a same-named intrinsic. This was the root cause behind the
  42 `builtin-catalog-fp-collision` semantic-audit divergences. Fixed by
  gathering every visible source/ABI candidate across the base table AND its
  TableExtensions FIRST, with explicit cardinality semantics: exactly one
  candidate → `Source`/`Abi`/`Opaque`; **more than one → honest ambiguous
  `Unknown`** (source ambiguity still shadows the catalog — never pick-first,
  never fall through to a false intrinsic); zero candidates (or an
  unresolved table) → consult the Record builtin catalog, preserving the
  existing table-independent-builtin behavior. `resolve_bare`'s own-object
  precedence was already source-before-catalog (investigated and confirmed
  correct pre-fix; kept as a regression-locking fixture, not a second bug).
  **Secondary, previously-undisclosed behavior change caught in review:** a
  base-table name match with the WRONG arity no longer short-circuits the
  scope walk to a false `Unknown` — it now correctly falls through to a
  sibling `TableExtension` that declares the matching arity (pinned by the
  new `tests/r0-corpus/ws-builtin-shadow-arity/` fixture + the
  `ws_builtin_shadow_arity_base_wrong_arity_falls_through_to_extension`
  harness test; empirically verified to fail against the pre-fix
  short-circuit by a temporary revert-and-rerun). **Investigation note:** the
  catalog membership check is an exact-lowercase-string `phf::Set` lookup (no
  fingerprint/hash digest is stored or compared anywhere in this path —
  confirmed by reading `builtins.rs`, `member_catalog.rs`, and
  `abi_ingest.rs`'s `param_type_fp`/`fnv1a`, which fingerprints ABI routine
  *signatures* for `RoutineNodeId` identity, an unrelated concern), so a true
  hash collision cannot occur today; `BuiltinId` is built directly from the
  query string, so the catalog is name-exact and fail-closed BY
  CONSTRUCTION (a non-catalog name always returns `None`) — this is asserted
  directly by `global_builtin_id_is_name_exact_and_rejects_near_miss` /
  `member_builtin_id_is_name_exact_and_rejects_near_miss`. (An earlier
  revision of this fix added `global_builtin_id_checked`/
  `member_builtin_id_checked` fail-closed wrapper functions around this
  lookup; review found their internal re-verification guard structurally
  UNREACHABLE — the `BuiltinId` they re-checked was always self-consistent by
  construction — so the wrappers were dead code overstating a "structural
  guard" that never actually fired, and were removed; every catalog consult
  site in `resolver.rs` now calls `global_builtin_id`/`member_builtin_id`
  directly.) **Qualified-intrinsic bypass investigation:** the IR CAN
  represent a fully-qualified platform call (`System.CreateGuid()` parses as
  an ordinary `Member { receiver: "System", method: "CreateGuid" }`); no
  special-case code was needed for the bypass because `Framework`-singleton
  receivers (`System`/`Session`/`NavApp`/...) are classified unconditionally
  in `infer_receiver_type`'s Step 1 (before any variable/source lookup) and
  `resolve_member`'s `Framework` arm is catalog-or-`Unknown` only — it never
  consults source candidates, so a local procedure structurally cannot shadow
  a qualified platform call. `tests/r0-corpus/ws-builtin-shadow/` fixture (5
  scenarios, asserted via 5 `tests/program_resolve_harness.rs` Test-21 cases
  with exact route/evidence/target assertions) + `tests/r0-corpus/ws-builtin-
  shadow-arity/` fixture (1 scenario, Test-22) + 2 `resolver.rs` unit tests
  (genuine shadow + cross-TableExtension ambiguity) + 2 catalog-layer unit
  tests (near-miss-name fail-closed regression, asserted directly against
  the phf-backed lookups). Verified: all pre-existing `resolve_member`/
  `resolve_bare` tests still green; `cargo test --workspace` (no `CDO_WS`)
  fully green; `cargo clippy --release --all-features -- -D warnings` clean;
  `cargo fmt --check` clean. No `engine::l3`/`engine::l2` import added.

### Added
- **Plan 1B.3b Task 4 (CAPSTONE): the fresh engine stands alone — L3 oracle
  retired from validation, verified + honestly documented**
  (`CHANGELOG.md`; no source changes — verification + docs only) — closes
  1B.3b and the whole 1B.3 resolution arc. 1B.3b retires the L3 oracle from
  the fresh resolver's **validation**. As of this task the engine is
  validated by three things, NONE of which call L3 at run time:
  (a) **committed, anonymized, frozen L3-verdict goldens** — Member/Interface
  (`cdo-anon.json`), ImplicitTrigger (`cdo-trigger-anon.json`), EventFlow
  (`cdo-event-anon.json`) — keyed by per-site target identity, which is the
  source of COMPLETENESS evidence; the CDO-scale floor is active on the
  gated/internal runner that has the CDO workspace, public CI validates the
  goldens' metadata (schema version, non-empty, `genuine_wrong==42` against
  the committed manifest) without needing the workspace; (b) the
  **L3-independent contracts** — `coverage_holds`, `evidence_overclaim`,
  `abi_unmapped` (`abi_ingestion_integrity`), and `route_applicability`
  (carrying the Task-2-ported fan-out applicability teeth) — these are
  SOUNDNESS checks: every emitted route is individually well-formed and
  applicable, re-derived independently of any L3 projection, plus the
  Histogram + real-unknown-rate ceiling; (c) **always-run synthetic semantic
  fixtures** (`tests/fixtures/semantic-golden/`, `implicit-trigger/`,
  `fanout-applicability/`, the EventFlow two-stage-join fixture) that need no
  `CDO_WS` at all. Stated plainly, per the plan's honesty framing: this is
  **not first-principles semantic correctness** — it is the FROZEN
  HISTORICAL L3 verdict (captured before retirement) plus the L3-independent
  contracts plus fixtures. The teeth prove SOUNDNESS; the frozen goldens
  carry COMPLETENESS; neither alone would be enough. L3-minting moved
  entirely to the dev-only `mint-goldens` tool (`src/bin/mint-goldens.rs` +
  `src/program/l3_mint.rs`, gated behind `CDO_WS`+`CDO_ANON_KEY` or
  `REGEN_TEMP_GOLDENS=1`); `src/engine/l3` itself STAYS in the tree
  unchanged — it remains the `aldump`/L4/L5 backbone, a separate consumer
  from the fresh resolver; `builtins.rs::global_builtins` (clean-room global
  builtin catalog membership, sourced from `engine::l3::global_builtins`
  data, not logic) remains the one sanctioned `engine::l3` data dependency
  inside `src/program/resolve/`. The fixed, committed anonymization salt
  (`CDO_ANON_KEY` fallback test key) keeps the frozen goldens byte-reproducible;
  `ENFORCE_CDO_WS=1` hard-fails (rather than silently skipping) a
  gated/internal run that loses its `CDO_WS` or hits a zero-site audit; a
  workspace-SHA drift warning (when the live `CDO_WS` content no longer
  matches the SHA the goldens were minted from) is informational only —
  the audits load the frozen goldens regardless, so drift does not fail the
  build.
  **Capstone verification performed for this task** (binding requirement,
  not just narrative): `cargo test --workspace` with no `CDO_WS` set —
  **1610 tests passed, 0 failed**, across 159 test-result blocks (lib +
  every integration test binary + doctests), fully green without the
  oracle; `cargo clippy --release --all-features -- -D warnings` — clean,
  zero warnings; `cargo fmt --check` — clean, no file needs reformatting;
  `grep -rnE "use .*engine::l3|use .*engine::l2" src/program/resolve/` —
  the only hits are in `builtins.rs` (two `use` statements plus one doc
  comment naming the same exception), confirming zero other `engine::l3`/
  `engine::l2` imports anywhere under `src/program/resolve/`. The five
  frozen CDO audits/teeth were each run SINGLY (not as the full suite, which
  cannot run in parallel — unrelated pre-existing constraint) against the
  real, currently-dirty CDO workspace with `CDO_WS` +
  `ENFORCE_CDO_WS=1`, all green and deterministic: `cdo_l3_semantic_audit_no_fresh_wrong`
  (`genuine_wrong=42` exact manifest match, `paired=11377` checked sites,
  `fresh_wrong=174`→`fresh_ahead_dispatch=132`+`genuine_wrong=42`);
  `cdo_trigger_audit_frozen_load` (`matches=185`, `fresh_wrong=0`);
  `cdo_event_audit_frozen_load` (`matched_pairs=2`, `pair_l3_only=0`);
  `route_applicability_zero_violations` (`total_routes=17241`,
  `violations=0`, `abi_unmapped=0`); `fan_out_applicability_zero_violations`
  (all four fan-out violation counters `0`, non-vacuous
  `routes_checked[interface=28 instance_builtin=449 implicit_trigger=958
  event=2284]`). No workspace-SHA drift warning printed on this run.
  **Out of scope for 1B.3b** (explicitly deferred, tracked in the roadmap):
  `genuine_wrong=42` underlying disambiguation (mostly L3-error-on-builtins);
  full `fresh⊆l3` partial-recall validation; the same-arity-type overload
  DISPATCH (Cat-D, 17 divergences); the snapshot double-include root cause;
  table/page/database trigger-events as EventFlow; `BindSubscription`
  activation; the receiver-gap buckets; a workspace-pinning operational doc.
  **The fresh engine now stands alone**: it validates itself, at run time,
  without ever calling into `project_l3*` — L3 is reachable only from
  `src/engine/l3` (the unrelated `aldump` backbone) and from the opt-in
  dev-mint path.

- **Plan 1B.3b Task 2: port fan-out applicability teeth (soundness) into `route_applicability`**
  (`src/program/resolve/semantic_golden.rs`, `tests/program_resolve_harness.rs`,
  `tests/fixtures/fanout-applicability/` NEW; commits `dfec53e` + `1ee0e8e`) —
  ports the four fan-out applicability predicates that previously lived ONLY
  inside the (Task-3-deleted) dual-run gates' FreshOnly branches into
  `route_applicability`, now running over EVERY fan-out route in
  `resolve_full_program`'s full edge set instead of only the FreshOnly-vs-L3
  subset: Interface (`DispatchShape::Polymorphic`) via
  `interface_route_applicable`; instance-builtin/enum-static Catalog `Builtin`
  routes (`PageInstance::`/`ReportInstance::` via
  `instance_builtin_route_applicable`, `Enum::` via the `Enum` member-builtin
  catalog directly); ImplicitTrigger (`DispatchShape::Multicast`) via
  `implicit_trigger_route_applicable` (`Validate` sites fall back to the
  documented table/extension-identity check); EventFlow via the already-`pub`,
  L3-free `differential::verify_event_subscriber_route`. New private
  `build_fan_out_site_context` re-walks the same parsed call sites
  `resolve_full_program` resolves to recover the Interface/`RecordOp`
  call-site context (`FanOutSiteContext`) `Edge`/`Route` cannot carry —
  keyed by `SiteId` so it lines up 1:1 with the edges (incl. all five DML ops
  — Insert/Modify/Delete/Rename/Validate — via `record_op_kind_for_method`);
  fails CLOSED (counts a violation) when no context is recovered for a
  Polymorphic/Multicast edge. `ApplicabilityReport` gains four SOUNDNESS
  counters (`interface_applicability_violations`/`instance_builtin_violations`/
  `implicit_trigger_violations`/`event_violations`, summed by
  `fan_out_violations()`) plus four `*_routes_checked` non-vacuity denominators
  — documented as SOUNDNESS (every emitted route is individually
  well-formed/applicable), distinct from the frozen L3-validated goldens'
  COMPLETENESS. `is_clean()` now requires all six violation counters to be
  zero. 12 new unit tests prove each predicate's positive AND
  fabricated-negative case bites (hand-built `Edge`/`Route`/`FanOutSiteContext`
  fixtures) plus the fail-closed-on-missing-context cases. New on-disk fixture
  `tests/fixtures/fanout-applicability/` exercises all four dispatch kinds
  end-to-end through `resolve_full_program` (Test 20,
  `fan_out_applicability_zero_violations`): `violations==0` on the fixture AND
  (env-gated) on the real CDO workspace — `total_routes=17241`, `violations=0`,
  `routes_checked interface=28/instance_builtin=449/implicit_trigger=958/event=2284`
  (non-vacuous), deterministic. `differential.rs`/`applicability.rs` untouched
  (every predicate needed was already `pub`); `project_l3*` and the dual-run
  gates stay intact for Task 3.

- **Plan 1B.3b Task 1: committed anonymized frozen goldens (all dispatch kinds) + dev-mint tool + `ENFORCE_CDO_WS` guard**
  (`src/program/resolve/anon.rs` NEW, `src/bin/mint-goldens.rs` NEW,
  `src/program/resolve/semantic_golden.rs`, `src/program/resolve/differential.rs`,
  `src/program/resolve/mod.rs`, `tests/program_resolve_harness.rs`,
  `tests/fixtures/implicit-trigger/` NEW, `tests/goldens/semantic-edges/cdo-anon.json`,
  `cdo-trigger-anon.json`, `cdo-event-anon.json`, `implicit-trigger-fixture.json`,
  `.gitignore`, `Cargo.toml`) — the C1 FREEZE that precedes 1B.3b's L3-oracle
  removal (Task 3): every L3-derived correctness baseline the gate module
  depends on is now a COMMITTED, ANONYMIZED, frozen artifact instead of a
  live L3 mint on every run. `anon::anon(domain, s)` is a domain-separated,
  versioned, HMAC-SHA256 keyed hash (`site:v1`/`target:v1`/`trigger-op:v1`/
  `event-pair:v1`); the key comes from the non-committed `CDO_ANON_KEY` env
  var (a committed fallback test key keeps `cargo test --workspace` and the
  synthetic fixtures deterministic without ever anonymizing real CDO data —
  see `anon.rs`'s module docs for the full governance writeup). The dev-mint
  tool (`cargo run --release --bin mint-goldens`, `CDO_WS`+`CDO_ANON_KEY` set)
  is the LAST sanctioned L3 use: it mints + anonymizes the three committed
  goldens (`cdo-anon.json` Member/Interface via `mint_l3_validated_golden`,
  `cdo-trigger-anon.json` ImplicitTrigger via the newly-`pub`
  `project_l3_implicit_trigger_in_scope`, `cdo-event-anon.json` EventFlow via
  the new `CanonicalKey`-keyed `project_l3_event_rows` — sidesteps L3's
  proprietary `stable_routine_id` scheme so the fresh side can independently
  re-derive the same identity) and the gitignored local de-anon map
  (`cdo-deanon-map.json`, `AnonId -> plaintext`, for root-causing a failing
  anonymized diff). `run_cdo_semantic_audit` now LOADS the committed golden
  and anonymizes the fresh side at audit time instead of calling `project_l3`
  live — zero `engine::l3` imports in any `run_cdo_*_audit` function. Two new
  audits (`run_cdo_trigger_audit`/`run_cdo_event_audit`) prove the same
  mechanism for ImplicitTrigger/EventFlow (mechanism-proof scope only — the
  zero-tolerance gates for those dispatch kinds remain the live, CDO-gated
  `run_implicit_trigger_harness`/`run_event_flow_gate`, unchanged, until
  Task 3). The `ENFORCE_CDO_WS=1` hard-fail guard (`cdo_ws_or_enforce`/
  `enforce_audit_ran` in the test harness) makes a missing `CDO_WS`, a
  missing/invalid frozen golden, or a zero-site audit PANIC on the
  gated/internal runner instead of silently skipping — no fail-open. A new
  unconditional, no-`CDO_WS`-needed test validates the three committed
  goldens' metadata (schema version, non-empty, `genuine_wrong==42` via the
  pre-existing `known-genuine-divergences.json` manifest) for public CI. The
  always-run `event_fixture_two_stage_join` fixture test and a new
  `implicit_trigger_fixture_resolves_exact_target_set` fixture test both
  moved off live L3 entirely (`project_fresh_event_rows`/
  `mint_fresh_golden_for_kind` are pure fresh-side, no `engine::l3` build) —
  the always-run, L3-INDEPENDENT semantic coverage these two dispatch kinds
  keep after L3 retirement. Verified frozen==live against the real CDO
  workspace: `genuine_wrong=42` (exact manifest match), EventFlow
  `matched_pairs=2`/`pair_l3_only=0` (matches the documented thin-oracle
  baseline), both audits deterministic across reruns.

- **Plan 1B.3a Task 4 (CAPSTONE): L3-validated semantic edge golden + CDO audit + route-applicability contract**
  (`src/program/resolve/semantic_golden.rs` NEW, `src/program/resolve/mod.rs`,
  `tests/program_resolve_harness.rs`, `tests/fixtures/semantic-golden/`,
  `tests/goldens/semantic-edges/fixture.json`) —
  captures the post-L3 correctness floor before L3 retirement in 1B.3b.
  `mint_l3_validated_golden` (LAST SANCTIONED L3 ORACLE USE) projects L3
  targets per call site into a committed `SemanticGolden` JSON, keyed by
  column-ignoring `GoldenSiteKey` (mirrors `match_sites` strong key; omits
  column because L3 uses UTF-16 cols while fresh uses byte cols).
  `assert_against_semantic_golden` classifies every site into `match`,
  `fresh_wrong`, `fresh_missing`, `fresh_extra`, `fresh_novel`, or
  `golden_missing`; the critical class is `fresh_wrong` (fresh confidently
  resolved to the wrong target — undetectable by Histogram alone).
  `route_applicability` verifies the structural witness↔evidence contract on
  every route and delegates ABI check to `abi_ingestion_integrity`.
  Three new tests: Test 14 (in-repo fixture golden: fresh_wrong=0 and
  fresh_missing=0, regenerable via `REGEN_TEMP_GOLDENS=1`), Test 15
  (route-applicability: violations=0 and abi_unmapped=0 on fixture + env-gated
  CDO), Test 16 (CDO/L3 semantic audit: fresh_wrong ≤ 200 ceiling recorded
  2026-06-30 as 174 — Method/Interface dispatch divergences; deterministic
  SHA-256 digest committed as CDO audit fingerprint).

- **Plan 1B.3a Task 3: Obligation-coverage inventory + `resolve_full_program` + taxonomy'd self-reported metric**
  (`src/program/resolve/full.rs` NEW, `src/program/resolve/mod.rs`,
  `src/bin/aldump.rs`, `tests/program_resolve_harness.rs`,
  `tests/fixtures/full_program_fixture/`) —
  adds `ObligationId` (stable `CallSite` / `Publisher` enum), `Obligation`,
  `ClassifiedEdge`, `Coverage`, `ProgramReport`, `coverage_holds`,
  `is_primary_scope`, `obligation_inventory`, and `resolve_full_program`
  (clean-room, no L3 oracle).  The **COVERAGE CONTRACT** is distinct-id SET
  equality between parsed obligations and classified edges: `coverage_holds`
  fails iff any obligation is silently dropped or any spurious edge appears.
  `--program-call-graph-stats` in `aldump` now prints the whole-program and
  primary-scoped taxonomy'd histograms + coverage + ABI integrity as JSON.
  Three new tests: Test 11 (fixture, 3 call sites + 1 publisher, all buckets
  checked), Test 12 (contract unit: dropped/extra obligation caught), Test 13
  (env-gated CDO gate: coverage holds, `abi_unmapped==0`, primary rate ≤ 7%,
  deterministic across two runs).

### Removed
- **Plan 1B.3b Task 3: remove the L3 oracle (`project_l3*`) from the fresh
  resolver's gates — the engine is now self-validated**
  (`src/program/resolve/differential.rs`, `src/program/resolve/semantic_golden.rs`,
  `src/program/mod.rs`, `src/program/l3_mint.rs` NEW, `src/bin/mint-goldens.rs`,
  `tests/program_resolve_harness.rs`) — deletes the six L3-oracle projection
  functions (`project_l3`, `project_l3_sites`, `project_l3_in_scope`,
  `project_l3_member_in_scope`, `project_l3_implicit_trigger_in_scope`,
  `project_l3_event_rows`) and the four live dual-run "fresh vs L3"
  comparison gates (`run_harness`/`run_site_harness`/`run_resolution_harness`/
  `run_member_resolution_harness`/`run_implicit_trigger_harness`/
  `run_event_flow_gate`, plus their `DiffReport`/`ResolutionReport`/
  `MemberResolutionReport`/`ImplicitTriggerResolutionReport`/
  `EventFlowGateReport` report types) from `differential.rs`. Their coverage
  is now provided entirely by the 1B.3b Tasks 1-2 replacements: the frozen,
  committed, anonymized semantic/trigger/event goldens
  (`run_cdo_semantic_audit`/`run_cdo_trigger_audit`/`run_cdo_event_audit`) +
  `coverage_holds` (Bare/Member), the L3-INDEPENDENT fixture tests
  (`event_fixture_two_stage_join`, `implicit_trigger_fixture_resolves_exact_target_set`),
  and the ported fan-out applicability teeth (`route_applicability`,
  `fan_out_applicability_zero_violations`). The three projections still
  needed to MINT those frozen goldens (`project_l3`,
  `project_l3_implicit_trigger_in_scope`, `project_l3_event_rows`) moved to
  a new module, `src/program/l3_mint.rs` (OUTSIDE `src/program/resolve`) —
  the lone surviving L3-oracle access point in the library, called only by
  the dev-mint tool (`src/bin/mint-goldens.rs`) and the opt-in
  `REGEN_TEMP_GOLDENS=1` fixture-regen test path. `differential.rs` and
  `semantic_golden.rs` now carry ZERO `engine::l3`/`engine::l2` imports; the
  sole remaining `engine::l3` import anywhere under `src/program/resolve` is
  `builtins.rs`'s clean-room `global_builtins` membership-DATA dependency
  (documented as the sanctioned exception). `match_sites`/`SiteMatch`/
  `witness_contract_holds` survive (generic, L3-INDEPENDENT) for their own
  unit tests and `route_applicability`'s witness-contract check respectively.
  `cargo test --workspace` (no `CDO_WS`) fully green on the surviving
  contracts; the frozen CDO audits + `route_applicability` verified green
  and deterministic (run singly with `CDO_WS`+`ENFORCE_CDO_WS=1` — the full
  CDO suite still can't run in parallel, unrelated to this task).

### Fixed
- **(resolve) Split CDO/L3 semantic-audit `fresh_wrong` into adjudicated classes**
  (`src/program/resolve/semantic_golden.rs`, `src/program/resolve/differential.rs`,
  `tests/program_resolve_harness.rs`, `tests/goldens/semantic-edges/known-genuine-divergences.json`) —
  The old `fresh_wrong ≤ 200` ceiling conflated two fundamentally different classes.
  Three-case adjudication in `is_fresh_ahead_dispatch`:
  (1) `l3 ⊆ fresh` — fresh is a superset, more precise;
  (2) all L3 targets are Interface (kind=11) and all fresh targets implement them;
  (3) `fresh ⊆ l3` — fresh partially resolved a compound call (partial-correct, not wrong).
  Result on CDO: `fresh_wrong=174 → fresh_ahead_dispatch=132 genuine_wrong=42`.
  The 42 genuine_wrong are `fresh=builtin (kind=255)` vs `L3=source-routine` **disjoint**
  disagreements on the same callee text — and since the callees are genuine AL builtins
  (`message`/`confirm`/`clear`/`strlen`/`copystr`, `PageInstance::*`/`Record::*`), for most
  of them fresh is **likely correct and L3 is the side in error**; the audit treats L3 as
  the floor by construction, so they land in `genuine_wrong` regardless of which side is
  right (an UPPER bound on fresh errors — confirming the direction is 1B.3b work). All 42
  are enumerated in the committed manifest. Hard gate: `genuine_wrong_count ≤ manifest_count`
  (42) — any NEW disjoint divergence not in the manifest fails CI. fresh_ahead_dispatch (132)
  is always ALLOWED. NOT a clean win.
  `fresh_missing=191` characterization: page_rec=115 codeunit_implicit_rec=24 trigger=38 other=14.
- **(resolve) `witness_contract_holds` made `pub(crate)` in `differential.rs`**;
  duplicate `route_witness_contract_holds` in `semantic_golden.rs` removed — now delegates
  to the single canonical implementation.
- **`resolve_object_run` target-not-found emits `Unknown` (not phantom `AbiSymbol`)**
  (`src/program/resolve/resolver.rs`) —
  the "target not found in any indexed app" arm was constructing an
  `AbiSymbol { app: caller_app_ref, … }` route.  Because the raw ABI index
  only contains dep-app entries (not the workspace app), this caused
  `abi_ingestion_integrity` to report 30 "unmapped" routes.  Fixed to emit
  `RouteTarget::Unresolved + Evidence::Unknown` (honest resolution failure).
- **`build_program_graph` deduplicates `objects` and `routines` after sorting**
  (`src/program/build.rs`) —
  in multi-app workspaces where a sibling app's compiled `.app` lands in
  `.alpackages`, the same source files could be parsed twice (once as
  workspace app, once as embedded dep), producing duplicate `RoutineNodeId`
  entries.  `emit_event_flow_edges` then emitted duplicate publisher edges,
  inflating `histogram.total` by ~60% above the obligation count while coverage
  still held (HashSet de-dup).  Fixed by adding `dedup_by` after `sort_by` for
  both vectors.

- **Plan 1B.3a Task 2: ABI ingestion-integrity invariant + Histogram source/catalog/external split**
  (`src/program/resolve/abi_check.rs` NEW, `src/program/resolve/mod.rs`,
  `src/program/resolve/edge.rs`, `src/program/abi_ingest.rs`,
  `tests/program_resolve_harness.rs`) —
  adds `pub mod abi_check` with `RawAbiIndex` (FRESH re-parse of raw `SymbolReferenceAbi`
  DTOs, independent of `ProgramGraph.routines`), `AbiIntegrityReport`,
  `abi_ingestion_integrity` (per-edge ABI route → raw-index lookup),
  `abi_ingestion_integrity_from_graph` (full-coverage form: checks every SymbolOnly
  `RoutineNode` against the raw index by reconstructing the `AbiRoutineKey` exactly as
  `resolver.rs::make_routine_route` would), and `run_abi_integrity_check` (CDO harness).
  Splits `Histogram.resolved: usize` into `resolved_source` / `resolved_catalog` /
  `resolved_abi_external` (keyed on best-evidence tier across default-firing routes:
  `Evidence::Source` → `resolved_source`, `Evidence::Catalog` → `resolved_catalog`,
  `Evidence::Abi | Evidence::Opaque` → `resolved_abi_external`); `real_unknown_rate`
  unchanged. Makes `object_kind_from_abi_type` and `read_symbol_reference_from_app`
  `pub(crate)`. Five tests: 4 fixture (no env required) + 1 env-gated CDO gate asserting
  `abi_unmapped == 0` and determinism.

- **Plan 1B.3a Task 1: Cached overload-safe ABI ingestion + structured `AbiRoutineKey`**
  (`src/program/abi_ingest.rs` NEW, `src/program/build.rs`, `src/program/node.rs`,
  `src/program/node_extract.rs`, `src/program/resolve/edge.rs`,
  `src/program/resolve/resolver.rs`, `src/snapshot/snapshot.rs`) —
  adds `sig_fp: u64` (FNV-1a fingerprint of param-type sequence) to `RoutineNodeId`
  so same-name overloads with different parameter types are distinct nodes;
  replaces stringly-typed `AbiSymbol { app, symbol_key }` in `RouteTarget` and
  `Witness` with structured `AbiRoutineKey { app, object_type, object_number,
  object_name_lc, routine_name_lc, params_count, param_type_fp, routine_kind,
  event_kind }`; introduces `AbiCache` (process-level `Mutex<HashMap>` keyed by
  `(guid, name, publisher, version)`) and `ingest_abi` which parses SymbolOnly dep
  `.app` SymbolReference.json into `ObjectNode` + `RoutineNode` entries during
  `build_program_graph`; adds `app_path: Option<PathBuf>` to `AppUnit`;
  adds `abi_routine_kind` + `abi_event_kind` fields to `RoutineNode` (always `None`
  for source routines). Four unit tests cover: dep nodes in graph, workspace-only
  graph unchanged, cache-hit across rebuild cycles, local/internal skip.

- **Phase-4b Task 5: Independent event-route teeth + honest framing**
  (`src/program/resolve/differential.rs`, `tests/program_resolve_harness.rs`) —
  adds `verify_event_subscriber_route`: for each fresh EventFlow `Routine` route,
  independently re-reads the subscriber's raw `[EventSubscriber]` `AttributeIr`
  from the `ParsedUnit` IR at gate time (NOT `RoutineNode.event_subscribers`, the
  index's cached parse that built the edge — that would be circular). Checks:
  (1) at least one `[EventSubscriber]` attribute freshly parses to the expected
  `(publisher_object_type, publisher_name, event_name)` triple; (2) subscriber
  `params_count ≤ publisher params_count` (parameter prefix check). FAIL →
  `unverified_extra` (zero-tolerance, asserted 0 in the CDO gate).
  `unverified_extra` is the sixth zero-tolerance gate assertion. Unit tests prove
  non-circularity: passing a `ParsedUnit` with the attribute absent (simulating
  corrupt raw IR) returns FAIL even though the index's cached `event_subscribers`
  would still say PASS — the function demonstrably reads from raw IR.

  **Honest framing (CDO DocumentOutput/Cloud workspace):** on CDO,
  `l3_event_row_count=2` in-scope resolved event rows (CDO is an extension app —
  L3 resolves an event pair only when BOTH publisher and subscriber are
  workspace-indexed source routines; base-app publishers arrive via
  SymbolReference as `AbiSymbol` routes and are not L3-"resolved"). Fresh matched
  both (100% recall of a thin in-scope oracle). The STRUCTURAL coverage —
  arity-FP reconciliation, multiple `[EventSubscriber]` attrs, dispatch conditions
  (Manual/SkipLicense), InternalEvent non-shipping — is carried by the in-repo
  `tests/fixtures/events/` fixture workspace, not the CDO dual-run. `Manual`
  subscribers are conditional `may-edges`; default reachability does NOT traverse
  them. NOT full event-modeling completion: table/page/database trigger-events,
  `BindSubscription` activation, cross-app resolved pairs remain for 1B.3.
  Fixes misleading `l3_sub_lookup` comment: "Stage 1 will still match" is WRONG
  for subscriber-key collisions — reworded to state the real exposure and why it
  is not a problem in practice.

- **Phase-4b Task 4: Structural dual-run event gate** (`src/program/resolve/differential.rs`,
  `tests/program_resolve_harness.rs`, `tests/fixtures/events/`) — adds `run_event_flow_gate`
  with a two-stage arity-FP-reconciled join: Stage 1 = arity-agnostic `EventPairKey`
  set-diff (`pair_l3_only` / `pair_fresh_only`); Stage 2 = within matched keys, arity
  comparison to detect `l3_false_positive_arity_mismatch` (L3 arity-blind last-wins
  picks wrong overload) / `l3_arity_unknown` (accepted) / `l3_regression` (genuine
  disagreement).  Every `pair_fresh_only` is machine-categorized: `l3_maybe_upgrade` /
  `multiple_attr_l3_gap` / `internal_event_non_shipping`.  Five zero-tolerance CDO gate
  assertions: `pair_l3_only=0`, `l3_regression=0`, `fresh_only_uncategorized=0`,
  `fresh_unprojectable=0`, `l3_unprojectable=0` — all pass on CDO.  Fixture workspace
  (`tests/fixtures/events/`) exercises all structural scenarios: overloaded publisher
  (L3 last-wins arity-FP), SkipOnMissingLicense subscriber, multi-`[EventSubscriber]`
  handler (L3 reads only first), InternalEvent subscriber (L3 classifies as "maybe").

- **Phase-4b Task 3: Publisher-anchored `EventFlow` `Multicast` edge emission**
  (`src/program/resolve/resolver.rs`, `src/program/resolve/stub.rs`) — adds
  `emit_event_flow_edges(graph, index, body_map) -> Vec<Edge>`: sweeps all publisher
  event routines in the program graph and emits one `EdgeKind::EventFlow` +
  `DispatchShape::Multicast` edge per publisher, with routes built from
  `ResolveIndex::subscribers_of` (Task 2).  Each route carries the subscriber's
  dispatch conditions (`ManualBinding` / `SkipOnMissingLicense` / …) and a
  `Witness::SourceSpan` (or `AbiSymbol` for SymbolOnly deps).  A publisher with
  zero subscribers emits an empty-routes edge → `classify_obligation` →
  `HonestEmpty`.  Wired into `resolve_program` (stub assembly point); exported from
  `program::resolve`.  Five unit tests cover the manual-binding reachability contract,
  HonestEmpty, non-manual default reachability, and determinism.

- **Phase-4 Task 4: Consolidated Phase-4 fan-out gate + honest scope framing**
  (`tests/program_resolve_harness.rs`) — adds `phase4_fanout_matches_or_beats_l3`,
  a single CDO gate that runs both the member harness (Member + instance-builtin +
  Interface) and the implicit-trigger harness (ImplicitTrigger Multicast) and asserts
  all six zero-tolerance conditions simultaneously: `regression_unexplained=0`,
  `evidence_overclaim=0`, `unverified_extra=0` on each harness, plus the adjudicated
  member divergence cap (≤56).  Prints a unified breakdown separating what Phase 4
  closed from what is explicitly deferred.

  **Phase 4 closes (scoped sub-phase, NOT full spec-§7 whole-program completion):**
  - *Interface Polymorphic fan-out* — `resolve_member` fans out to all known
    implementers; every Routine route is applicability-gated via
    `interface_route_applicable` (method/trigger/kind-level, IR-anchored);
    wrong-overload routes fail → `unverified_extra`; ambiguous overloads →
    `Route{Unresolved, Unknown}` (no guessed route).  `regression_interface=0`
    (drained), `fresh_ahead_interface` routes gate-proven.
  - *ImplicitTrigger Multicast* — `resolve_implicit_trigger` gated vs L3
    `DispatchKind::ImplicitTrigger` oracle; `matched=167`,
    `fresh_ahead_trigger` + `fresh_ahead_validate_fanout` routes applicability-proven;
    empty-target sites → `extra_site` (no triggers on table, benign).
  - *Object/Enum instance-builtins* — CurrPage/CurrReport framework singletons and
    typed-variable Page/Report receivers gated via `instance_builtin_route_applicable`;
    Enum-static dispatch gated via `member_builtin`; `fresh_ahead_instance_builtin=243`,
    `fresh_ahead_enum_static` routes gate-proven; `unverified_extra=0`.

  **Explicitly excluded (honest scope — not claimed as closed):**
  - *EventFlow (Phase 4b)* — deferred: oracle qualification, `ManualBinding`
    property, canonical event key, and reachability honesty for `Manual` subscribers
    (conditional may-edges, not unconditional Multicast) are outstanding; no event
    edges ship to the graph until the qualified oracle gate exists.
  - *Deferred to 1B.3*: `regression_page_rec` (Page/PageExt implicit-Rec
    source-table gap), `regression_compound_receiver` (chained receiver type
    propagation), `regression_codeunit_implicit_rec` (Codeunit TableNo/TestRunner
    implicit-Rec), `trigger.missing_site=78` (L3 ImplicitTrigger sites with no fresh
    peer), and 17 Cat-D divergences (same-object different-procedure overload
    disambiguation).

  Paired-subset results on CDO DocumentOutput/Cloud workspace:
  Member — `matched=7178`, `regression_unexplained=0`, `unverified_extra=0`,
  `verified_win=2790`, `fresh_ahead_instance_builtin=243`, `divergence=56` (cap);
  Trigger — `matched=167`, `regression_unexplained=0`, `unverified_extra=0`.

- **Phase-4 Task 3: ImplicitTrigger Multicast gating** (`src/program/resolve/differential.rs`,
  `tests/program_resolve_harness.rs`) — adds `run_implicit_trigger_harness` comparing fresh
  `resolve_implicit_trigger` (RecordOp sites: insert/modify/delete/validate) against the L3
  oracle filtered to `DispatchKind::ImplicitTrigger`.  Key fixes: L3 callsite_id is the
  `PRecordOperation.id`, not `PCallSite.operation_id` (separate numbering namespace) — built
  direct `op_by_id` map from `L3Routine.record_operations`; callee_fp constructed as
  `"{record_variable_name}.{op}"` to match fresh's raw Member expression text.  Fresh-only
  gating: Validate routes (field=None always fails applicability) classified by table-identity
  check → `fresh_ahead_validate_fanout`; Insert/Modify/Delete routes gate via
  `implicit_trigger_route_applicable` → `fresh_ahead_trigger`; empty-target sites (no triggers
  on table) → `extra_site` (benign).  CDO result on DocumentOutput/Cloud workspace:
  `matched=167`, `regression_unexplained=0`, `evidence_overclaim=0`, `unverified_extra=0`.
- **Phase-4 Task 2: Interface Polymorphic fan-out** (`src/program/resolve/resolver.rs`,
  `src/program/resolve/differential.rs`) — `resolve_member` now implements the
  `ReceiverType::Interface { name_lc }` arm: fans out to all known implementers via
  `ResolveIndex::implementers_of`, resolving each via `resolve_in_object`.  For each
  implementer: SymbolOnly tier delegates directly (arity matching impossible);
  source-tier checks the arity-matched overload count — exactly 1 resolves to a Routine
  route, 0 or >1 emits `Route{Unresolved, Unknown}` (Rule 1: no reachability black hole;
  Rule 2: no guessed route to an ambiguous overload).  Returns `(Polymorphic, routes)`.
  Gate (`run_member_resolution_harness`): added `DispatchKind::Interface` to the L3 oracle
  filter; extended `fresh_combined` to carry site arity and original routes; wired
  `interface_route_applicable` in the FreshOnly handler so every Routine route emitted for
  an interface call is applicability-checked (`fresh_ahead_interface` or `unverified_extra`).
  CDO result on DocumentOutput/Cloud workspace: `regression_interface=0` (drained),
  `unverified_extra=0`, `regression_unexplained=0`, `divergence=56` (cap raised from 45;
  11 new divergences are fan-out sites where fresh emits N targets and L3 emits 1).

### Fixed
- **Phase-4 Task 1: FreshOnly gate discriminator bug** (`src/program/resolve/differential.rs`) —
  The `run_member_resolution_harness` FreshOnly bucketing incorrectly applied the
  `instance_builtin_route_applicable` predicate to ALL FreshOnly sites with non-empty targets,
  not just instance-builtin fan-out routes.  Direct single-dispatch routes (Routine/AbiSymbol
  targets from `resolve_in_object`) were misclassified as `unverified_extra` instead of
  `extra_site`, producing 1223 false `unverified_extra` entries on CDO.  Fix: discriminate
  FreshOnly sites by their canonical target type — routes with `CanonicalTarget::kind=255`
  (Builtin) and `"PageInstance::"` / `"ReportInstance::"` prefix are instance-builtin fan-out
  routes (gate via `instance_builtin_route_applicable` with kind derived from the BuiltinId
  prefix); `"Enum::"` prefix routes are enum-static fan-out (gate via `member_builtin`);
  all other non-empty routes are direct single-dispatch and go to `extra_site`.  Additionally
  handles `Framework(PageInstance/ReportInstance)` receivers (CurrPage/CurrReport singletons)
  by deriving `ObjectKind` from the BuiltinId prefix rather than from the receiver type.
  CDO gate result: `unverified_extra=0`, `fresh_ahead_instance_builtin=243` (3 typed-var
  Object + 240 Framework/CurrPage singletons), `extra_site=1229`, `regression_unexplained=0`,
  `evidence_overclaim=0`, `missing_site=0`, deterministic.

### Added
- **Phase-3 Task 5: Member-resolution gate vs L3** (`src/program/resolve/differential.rs`,
  `tests/program_resolve_harness.rs`) — `run_member_resolution_harness(&Path) ->
  MemberResolutionReport` wires `infer_receiver_type` + `resolve_member` (Tasks 1–4) into
  the dual-run harness for every workspace `CalleeShape::Member` site, then compares against
  the L3 oracle filtered to `PCallee::Member` origin with `dispatch_kind ∈ {Method, Builtin,
  CodeunitRun}`.  Regression bucketing mirrors Phase 2: `regression_interface` (Phase-4
  fan-out), `regression_enum_static` (enum-static deferred), `regression_page_rec`
  (`Record{None}` — Page/PageExt implicit-Rec table gap), `regression_scalar` (Primitive
  by-design), two new named deferral buckets: `regression_compound_receiver` (chained dotted
  receiver e.g. `CurrPage.SubPage.Page` — Phase-4; 47 on CDO) and
  `regression_codeunit_implicit_rec` (Codeunit with `TableNo`/`Subtype=TestRunner` implicit
  `Rec` parameter not captured in IR; 24 on CDO).  CDO gate result (honest paired-subset):
  `regression_unexplained=0`, `evidence_overclaim=0`, `verified_win=2744` (fresh resolved
  2744 sites L3 left empty), `matched=7164`, `missing_site=0` (vs Phase-2 baseline of 3397
  — the capstone metric showing Phase-3 coverage), `divergence=45` (adjudicated: fresh more
  precise than L3 on resolved targets).  Determinism asserted by two consecutive runs.
  `MemberResolutionReport` has 18 fields.
- **Phase-3 Task 3: Object/SelfObject member dispatch** (`src/program/resolve/resolver.rs`) —
  `resolve_member` now handles `ReceiverType::Object{kind, name_lc}` and `ReceiverType::SelfObject`.
  Object dispatch: resolves the target object via `graph.resolve_object`, then calls
  `resolve_in_object` for arity-matched procedure lookup.  Special case: `Codeunit.Run(arity≤1)`
  dispatches to the codeunit's `OnRun` entry trigger (mirrors `resolve_object_run` entry-trigger
  semantics).  SelfObject dispatch: `resolve_in_object` on the calling object itself.
  Both arms produce `Exact` shape with `Source`/`Abi`/`Unknown` evidence matching the target
  tier; OnRun-absent → Opaque boundary route.  Five new unit tests cover all branches.
  Addresses ~800–1200 previously-Unknown member sites.
- **Phase-2 Bare/Run resolution gate vs L3** (`src/program/resolve/differential.rs`,
  `src/program/resolve/resolver.rs`, `src/program/resolve/extract.rs`,
  `tests/program_resolve_harness.rs`, Phase 2 Task 6) — `run_resolution_harness(&Path)
  -> ResolutionReport` wires the real `resolve_bare` / `resolve_object_run` resolvers
  into the dual-run harness and compares against the L3 oracle filtered to in-scope
  dispatch kinds (Direct/Builtin/CodeunitRun/PageRun/ReportRun/Unresolved). New
  `ResolutionReport` struct with 16 fields bucketing: `matched`, `regression_unexplained`
  (gate: 0), `regression_implicit_rec` (deferred), `regression_cross_app` (deferred to
  1B.3 ABI lookup), `evidence_overclaim` (gate: 0), `unverified_extra` (always 0 by
  design; witness quality is covered globally by `evidence_overclaim`), `verified_win`,
  `divergence`, `missing_site`, `extra_site`. Two root causes investigated and fixed:
  (1) AL overloaded procedures share the same `RoutineNodeId` — BodyMap last-write-wins
  stored only one overload's params, causing all other arities to fail → `resolve_in_object`
  now falls back to first candidate when `candidates.len() > 1` (overload signal); (2)
  FreshOnly sites with non-empty targets reclassified as `extra_site` (legitimate
  fresh-only wins from interface-dispatch contexts excluded from the L3 in-scope filter).
  Also added `target_is_name: bool` to `CalleeShape::ObjectRun` and updated `classify_call`
  to use `ExprKind::DatabaseReference` for static ObjectRun target extraction. New
  `is_cross_app_regression` helper documents the dep-boundary SymbolReference gap. CDO
  gate (honest paired-subset result): `regression_unexplained=0`, `evidence_overclaim=0`,
  `unverified_extra=0`, `verified_win=1827`, `divergence=38` (all adjudicated — see
  task-6-report.md), `regression_implicit_rec=90` (Phase 3 deferred). The raw rates
  `fresh_unknown=4.5%` vs `l3_unknown=65.1%` are NOT comparable: denominators differ
  (fresh=4795 in-scope Bare/Run sites vs L3=8196 in-scope edges; `missing_site=3397`
  are L3 Direct/Member-dispatch sites fresh defers to Phase 3) and fresh emits Builtin
  targets while L3 builtin edges carry `to=None`. Honest result: on the paired subset
  (`matched=4304`), fresh has 0 unexplained regressions and 1827 verified wins over L3.
  Whole-branch fix wave added: symmetric paired-subset assertion
  (`total_regressions <= verified_win`), bounded divergence cap (`divergence <= 38`),
  permanent divergence summary print, and honesty comments on `unverified_extra` and
  `is_implicit_rec_regression`. Determinism asserted by two consecutive runs.
- **L3 PCallSite projection + Phase-1 site-parity gate** (`src/program/resolve/differential.rs`,
  `src/program/resolve/extract.rs`, `tests/program_resolve_harness.rs`, Phase 1 Task 4) —
  `project_l3_sites(&Path) -> Vec<CanonicalEdge>` projects every L3 `PCallSite` (not `CallEdge`)
  to a site-level oracle. `run_site_harness(&Path) -> DiffReport` compares fresh structured
  call-site classification (`CalleeShape`) against that oracle and buckets extras into
  `extra_recordop` / `extra_commit` / `extra_implicit_rec` / `extra_unexplained`.
  `extract_sites_for_routine` added to `extract.rs` (per-routine scoping to prevent double-
  counting when multiple same-named triggers exist in one object). Three root causes
  investigated and fixed on the CDO workspace: (1) ancestor `.alpackages` CDO dep with
  identical `AppId` polluted fresh set → `ws_file_set` filter; (2) multi-same-name-trigger
  double-counting → per-routine extraction; (3) report-dataitem-trigger implicit-Rec
  approximation → `dataitem_source_table.is_some()` guard. CDO gate: `matched=13431`,
  `missing_site=0`, `unaligned=0`, `extra_unexplained=0`, `extra_recordop>0`; determinism
  asserted by two consecutive runs.
- **Dual-run differential harness + `aldump --program-call-graph-stats`**
  (`src/program/resolve/differential.rs`, `src/bin/aldump.rs`, Phase 0 Task 7) —
  `run_harness(&Path) -> DiffReport` wires the full pipeline (snapshot →
  ProgramGraph → fresh stub resolve → workspace-scoped canonical projection →
  L3 oracle projection → span-based site matcher → diff buckets). `DiffReport`
  fields: `fresh_total_all_apps`, `fresh_total_workspace`, `l3_edges`, `matched`,
  `regression`, `missing_site`, `extra_site`, `unaligned`. Phase-0 baseline:
  stub resolves nothing → `regression == matched` (all paired sites regress); this
  is the gap Phases 1–4 will close. `aldump --program-call-graph-stats <workspace>`
  prints the `DiffReport` as JSON. CDO gate: `matched > 1000` and `unaligned < 5%`
  confirm the Tasks 4–6 key encodings align on real data; determinism asserted by
  two consecutive runs.
- **L3 → canonical oracle adapter** (`src/program/resolve/differential.rs`,
  Phase 0 Task 5) — `project_l3(&Path) -> Vec<CanonicalEdge>` runs the existing
  L3 resolver over a workspace and projects its `CallEdge`s into the same
  `CanonicalEdge` shape as `project_fresh`, enabling set-diff in the Task 6/7
  harness.  PAnchor line/col are 0-based (same basis as the fresh side);
  columns are UTF-16 vs byte (documented in the function doc, handled by the
  matcher).  Shared helpers extracted: `callee_fp`, `object_kind_str_to_tag`,
  `make_canonical_key` — both projections call these so encodings cannot drift.
  CDO-gated test confirms >1000 edges projected and every site has a real span.
- **CDO whole-program node-graph robustness + app-qualification gate** (`tests/program_graph.rs`) —
  integration test (`CDO_WS`-guarded) that runs `build_program_graph` over the real CDO
  dependency snapshot, asserts panic-free completion, and verifies the resulting graph is
  deep (>500 objects, >2000 routines) and app-qualified (nodes span ≥2 apps) with objects
  deterministically sorted by `NodeId`. On CDO the graph spans 21 apps with 23,432 objects
  and 259,260 routines. Capstone gate for Plan 1B.1.
- **`ProgramGraph` + topology-scoped object index** (`src/program/graph.rs`,
  `src/program/build.rs`) — `build_program_graph(&AppSetSnapshot)` interns all
  apps, extracts object/routine nodes via `parse_snapshot`, wires real dependency
  topology from `declared_deps` (GUID-match preferred, name+version fallback), and
  exposes `resolve_object(from, kind, name)` that searches only `from`'s transitive
  dependency closure — never flat-global. Adds `AppRegistry::find_by_name` helper.
- **Whole-program node graph** (`src/program/`) — app-qualified canonical
  `NodeId`s + topology index over the snapshot (Plan 1B.1). Also adds
  `Hash, Ord, PartialOrd` to `al_syntax::ir::ObjectKind` (plain C-like enum,
  safe and free).
- **Content-addressed source cache** (`src/snapshot/cache.rs`) — `cached_source(app_path)`
  stores the extracted `Vec<SourceFile>` from embedded `.app` packages as
  `<OS-cache-dir>/al-ch-snapshot-cache/<blake3-hex>.json`; the content hash
  is the key so stale reads are structurally impossible. `EmbeddedAppProvider`
  now routes through the cache. `SourceFile` gains `Serialize`/`Deserialize`.
- **Snapshot robustness gate** (`tests/snapshot_robustness.rs`) — `cdo_snapshot_deep_parse_is_panic_free`:
  env-guarded (`CDO_WS`) integration test that builds the full CDO app-set snapshot
  and deep-parses it; asserts no panic and >1000 files parsed (Plan 1A §3.7 gate).
- **App-set snapshot ingestion substrate** (`src/snapshot/`) — per-app source
  acquisition with identity verification + trust tiers (Spec 1 / Plan 1A).
- **`snapshot::parse_snapshot`** — deep-parse of snapshot source into the owned
  IR. `parse_snapshot(&AppSetSnapshot) -> Vec<ParsedUnit>` walks every
  source-bearing `AppUnit` in parallel (local rayon pool, 32 MiB worker stack —
  the `al_syntax` lowerer recurses deeper than the default Windows thread stack
  on large BC packages) and yields `ParsedUnit { app, files: Vec<ParsedFile> }`
  holding the owned `al_syntax::ir::AlFile` per source file. Symbol-only boundary
  units contribute no output; their ABI feeds later resolution.

### Changed
- **Pinned the toolchain (`rust-toolchain.toml` → 1.96.0).** CI floated `dtolnay/
  rust-toolchain@stable` while gating on `cargo clippy -- -D warnings`, so every new
  clippy release that adds lints could break CI with no code change (it did: 1.96 added
  `unnecessary_sort_by` / `useless_conversion` cases the 1.94 dev box never saw). The pin
  makes CI deterministic and matches local dev: `ci.yml` keeps `dtolnay/rust-toolchain@
  stable` (a base install with rustfmt/clippy), but every `cargo` command runs under the
  toml-pinned version via the rustup override, so the file is the single source of truth.
  Bump deliberately + clear new lints in the same PR. Also fixed the 1.96 lints surfaced:
  3 `sort_by` → `sort_by_key(Reverse(..))`
  (descending sorts preserved), 2 redundant `.into_iter()` in `chain(..)`.
- **Cleared the clippy `-D warnings` debt + whole-crate edition-2024 rustfmt** (CI gate
  prerequisites for merging `feat/owned-syntax-ir` → `master`). The edition-2024 upgrade
  enabled let-chains, so clippy's `collapsible_if` flagged ~155 `if x { if let … }` nests
  (master @ 2021 never saw these); `cargo clippy --fix` collapsed them to let-chains.
  Remaining handled by hand: 2 `never_loop`s (`for f in … { return Err }` → `if let
  Some(f) = …next()`), `strip_prefix`/`clamp`/`from_ref`/`&Path`/`needless_range_loop`/
  `redundant_guard` rewrites, doc-list indentation, and `#[allow]` with rationale for the
  inherent ones (`too_many_arguments` on document-envelope builders, `type_complexity` on
  parallel index maps, `large_enum_variant`, `enum_variant_names` where `Event` is the AL
  domain term). ~22 dead-code items (telemetry `dedup` module, detector `INVALIDATING_OPS`,
  `is_edge_kind`, never-read data-model fields, etc.) were triaged as future-design
  scaffolding and kept under targeted `#[allow(dead_code)]` with notes — none were obsolete.
  Then a one-time `cargo fmt` normalized the 277 stale edition-2021-formatted files (the
  per-file `rustfmt` hook keeps them clean afterward). `cargo clippy --release -- -D
  warnings`, `cargo fmt --check`, and `cargo test --workspace` all green.

### Fixed
- **Deterministic dependency order + GUID-then-name topology matching.**
  `load_all_apps` now sorts its output by the AppId 4-tuple (GUID, name, publisher,
  version) before returning, making `AppRef`/`NodeId` numbering reproducible across
  machines and filesystems (charter C8). Topology wiring in `build_program_graph`
  previously fell through to name+version only when the dep carried no GUID; it now
  tries GUID first and falls through to name+version when the GUID match yields
  `None` — closing the gap where a dep carries a GUID but the matching snapshot unit
  has an empty `id.guid`.
- **Dependency apps now carry their real unique GUID (and publisher).** `AppMetadata`
  parsed only `name`/`version` from `NavxManifest.xml`, dropping the `App@Id` (the app's
  only globally-unique identity) and `Publisher` — so `SnapshotBuilder` built dependency
  `AppId`s with `guid: ""`, leaving cross-app node identity leaning on name+version
  uniqueness. `parse_manifest` now also extracts `Id` → `AppMetadata.app_id` and
  `Publisher`, and the dependency `AppId` is built from the `.app`'s authoritative manifest
  (the workspace already read its own `id` from `app.json`). Local-provider matching now
  prefers GUID when known. The identity foundation Plan 1B builds on is now truly unique.
  The same manifest-enrichment pass fixes two more workarounds: (a) dependency `AppUnit`s
  now carry a REAL compilation basis (`Runtime`/`Platform`/`Application` from the manifest)
  instead of an empty `CompilationContext::default()` — note the source-level `#if`
  preprocessor symbols are still NOT recoverable from a `.app` (that needs SymbolReference
  reconciliation, a later phase); (b) `AppMetadata` + every `AppUnit` now carry the app's
  **declared dependencies** (each with its GUID, from the manifest `<Dependencies>` /
  app.json), so Plan 1B's resolution can be dependency-topology-aware instead of flat-global.
  `AppDependency` gains `app_id` (parses the app.json / manifest `id`).
- **Member-trigger names (`Object::Member`) were truncated to the object half.** The
  grammar's `_trigger_name` was an inlined `seq(id, '::', id)`, so the `name` field of
  `trigger_declaration` was `multiple:true` and included the anonymous `::` token; the
  lowerer's `field("name")` returned only the FIRST node (`UserTours`), silently dropping
  `::ShowTourWizard`. Introduced a named `member_trigger_name` node (`object` / `member`
  fields) so `name` binds a single value (`multiple:false`, no `::` in its type set), and
  the lowerer now joins it to the full qualified `Object::Member` name. Grammar issue #4
  closed. (No member triggers in the test corpus → zero golden divergence; +1 named kind
  → 388, new node-types hash `90f25499…`.)

### Changed
- **tree-sitter-al grammar: case-pattern field-pollution cleanup.** Case branches no
  longer leak spurious fields. Two grammar-level root causes, both fixed in the owned
  grammar (`tree-sitter-al` submodule):
  1. `field('pattern', $._case_pattern)` wrapped an *inlined* `repeat` whose members
     included the `,` separators, so the `pattern` field distributed over the comma
     tokens — `children_by_field_name("pattern")` returned anonymous `,` nodes and the
     owned-IR lowerer panicked on `case 1, 2:`. Introduced `_case_pattern_item =
     seq(field('pattern', $._single_pattern), optional(','))` so the `pattern` field
     binds a single value node, never a separator. `case_branch`,
     `preproc_split_case_branch`, `preproc_split_case_extended`, and
     `preproc_conditional_case_patterns` all consume `_case_pattern_item`.
  2. The `in`-as-case-pattern arm was an inline `seq(field('left',…), field('operator',
     …), field('right',…))` inside `_single_pattern`, so `left`/`operator`/`right`
     leaked onto every case node. Replaced with the existing named `$.in_expression`;
     the now-unnecessary `[$._single_pattern, $.in_expression]` conflict was removed.
  Net effect on `node-types.json`: −876 lines of field pollution; named-kind count
  unchanged at 387 (`_case_pattern_item` is inlined, `in_expression` already existed).
  The lowerer's defensive `is_named()` filter is kept as defense-in-depth. Regenerated
  the raw vocab (`gen-syntax`, new node-types hash `8f9b7013…`). Zero al-sem differential
  divergence. (Reviewed: gpt-5.5 + gemini-3.1-pro.)
- **Upgraded to Rust edition 2024** (from 2021) across all three crates — it is 2026 and
  edition 2024 is the current stable (rustc 1.94). `cargo fix --edition` applied the
  migrations: `unsafe extern "C"` (the al-syntax grammar FFI), `unsafe { std::env::set_var
  / remove_var }` (now unsafe in 2024 — a real parallel-test environment race the edition
  surfaces), and an over-conservative `if let/else`→`match` rewrite (tidied back to
  `if let … else`). Added a workspace `rustfmt.toml` with `edition = "2024"` as the SINGLE
  source of truth — `gen-syntax` and the editor `rustfmt` hook no longer hardcode an
  edition. Full `cargo build`/`test --workspace` green under 2024.

### Fixed
- **`raw_kind_round_trips` stale assertion** — it pinned `NAMED_KIND_COUNT == 386`, but
  the generated const is `387` (the `call_statement` grammar node added a named kind;
  the const regenerated, the test literal did not). Went unnoticed because root
  `cargo test` doesn't run member-crate tests without `--workspace`. Fixed to 387; run
  `cargo test --workspace` going forward.

### Changed
- **`gen-syntax` now rustfmts its generated Rust output** (`raw_kind.rs` / `field.rs` /
  `nodes.rs` / `mod.rs`), so the checked-in generated code is canonical AND stable across
  regenerations — a developer's `cargo fmt` produces the same bytes the generator does
  (no fmt/gen-syntax ping-pong). Mirrors how rust-analyzer formats its ungrammar-
  generated syntax nodes. Recommended CI guard: `cargo run -p xtask -- gen-syntax &&
  git diff --exit-code`. (Reviewed: gpt-5.5 + gemini-3.1-pro.)

### Added
- **Serde-skip drift gate.** The IR L2 feature snapshot (`tests/ir_l2_snapshot.rs`) now
  digests the `Debug` representation of each routine's `PFeatures` instead of serde
  JSON, so it covers the `#[serde(skip)]` (and `PartialEq`-excluded) fields a serialized
  golden cannot see — `PRecordOperation.in_until_condition` / `run_trigger`,
  `PCFNNode.source_range` / `is_case_else`, `PVarAssignment.rhs_identifier`. Four such
  load-bearing fields silently broke during the migration because the old byte gate
  (serde + PartialEq) was blind to them. A `debug_digest_catches_serde_skip_drift` proof
  test demonstrates the blind spot (two ops differing only in `in_until_condition`
  serialize identically and compare equal, yet their Debug digests differ).
- **Parenless statement calls are now call-hierarchy edges.** `parse_file_ir` captures
  every `ExprKind::Call`, including the parenless forms (`Initialize;`, `Rec.Find;`,
  `Modify;`) the old `call_expression`-only query missed. A procedure invoked only as
  `MyProc;` is now a real incoming/outgoing call edge and no longer mis-flagged as
  unused; parenless record builtins simply don't resolve to a user procedure. (Deferred
  completeness fast-follow from the Phase 4 zero-diff port.)
- **Grouped variable declarations yield every name.** `A, B: T` now produces a variable
  for BOTH `A` and `B` (the old query captured only the first, leaving trailing names as
  untracked receivers / false unknowns). Quoted grouped names are handled too.

### Removed
- **The engine's `tree-sitter` dependency is gone — `al-syntax` is the SOLE
  tree-sitter linker (Phase 5 SEAL complete).** Deleted the test-only legacy L2
  "dual-run oracle" (`dual_run_support.rs`, `tests/ir_dual_run.rs`) and the legacy
  tree-sitter L2 body-walk (`engine/l2/{body_walk,cfn,classify}.rs` + the tree-sitter
  fns in `mod`/`scope`/`node_util`/`control_context`/`operation_order`/`l2_workspace`),
  keeping the tree-sitter-free production helpers. Removed `tree-sitter` +
  `streaming-iterator` from `[dependencies]`. The engine consumes `al_syntax::parse`
  exclusively; `cargo tree -i tree-sitter` now shows only `al-syntax`.
  - The L2 single-routine analyzers (`control_context::analyze_named_routine`,
    `operation_order::analyze_named_routine_order`) + the `features_for_named_routine`
    test entry now build `PFeatures` via the owned IR
    (`l2_workspace::ir_features_for_named_routine`); the l2 / l2cc / l2order vector +
    oracle tests and `temp_state_capture` were converted to the IR path (no tree-sitter).
  - The migration-era `tests/ir_object_set_parity.rs` (IR-vs-tree-sitter set parity, a
    Phase-2/3 cutover precondition) is retired — its invariant is permanently satisfied.
  - Rebaselined 2 synthetic L2 vectors: the IR no longer emits an UNQUOTED qualified-enum
    VALUE (`Codeunit::A` → `a`) as a `condition_reference`. The legacy capture was a
    tree-sitter token-shape artifact (it captured a bare `identifier` but never a quoted
    value); an object/enum name is a compile-time constant, not a runtime variable, so
    dropping it is more accurate (reviewed: gpt-5.5 + gemini-3.1-pro). No production
    golden impact (the corpus's only such case is quoted).

### Changed
- **R0 identity snapshot (`engine::snapshot` / `aldump`) now derives from the owned IR**
  (`al_syntax::parse`) instead of its own tree-sitter walk (Phase 5 step). Object/
  routine identity (stable ids, signature fingerprints, normalizedSignatureHash,
  canonicalSignatureText) reuses the shared `engine::ids` algorithms, so R0 identity
  equals production identity. Byte-identical to the prior output — the R0 goldens pass
  unchanged. Removed `extract_from_tree` + the tree-sitter object/routine/param walkers.
- **`workspace_diagnostics` "No object declaration found" now uses the owned IR**
  (`al_syntax::parse(...).objects.is_empty()`) instead of a direct tree-sitter
  root-children scan (Phase 5 step). The diagnostic now matches exactly what the
  engine indexes (including objects nested under a `namespace`, which the old
  direct-child check missed). Removed the tree-sitter `Parser` + `root_has_object_declaration`.

### Removed
- **The legacy tree-sitter LSP parser is gone (Phase 4 complete).** Deleted `AlParser`
  + the 6 S-expr queries' consumers in `parser.rs`, the tree-sitter
  `analysis::calculate_complexity`, and the legacy CST metric walk in `main.rs`. The
  entire LSP front-end (parser / handlers / indexer / analysis / CLI metrics) now runs
  on the owned `al-syntax` IR. The AlParser differential is replaced by a forward
  digest snapshot golden of `parse_file_ir` over the r0-corpus
  (`tests/parser-ir-goldens/projection.snapshot`, regen via `REGEN_TEMP_GOLDENS=1`);
  the parser unit tests now exercise `parse_file_ir`.

### Fixed
- **`al_syntax::parse` no longer panics on a multi-value `case` branch.** tree-sitter-al
  v3 tags the `,` separators between a case branch's values with the `pattern` field, so
  `children_by_field(Pattern)` returned anonymous `,` tokens; lowering one as an
  expression hit `RawKind::from_raw(",")` and panicked ("unknown node kind") — a real
  crash reachable from the production parser on real BC code (e.g. `SalesPost`). The
  case-pattern lowering now filters to named nodes (added `RawNode::is_named`).

### Added
- **IR-owned L2 feature snapshot gate (`tests/ir_l2_snapshot.rs`).** Serializes the
  full `PFeatures` (loops / ops / record-ops / calls / field-accesses / record-vars /
  nesting / branching / unreachable / identifier+condition refs / variables /
  var-assignments / the `statement_tree` CFN) of every r0-corpus routine via
  `project_routine_features_ir`, digested into `tests/ir-l2-goldens/l2_features.snapshot`
  (REGEN with `REGEN_TEMP_GOLDENS=1`). This is the deepest L2 contract as a Rust-OWNED
  baseline — it replaces the migration-era legacy-vs-IR dual-run oracle without
  ossifying against the deleted tree-sitter walk.
- **`al_syntax::lookup_symbol_properties` facade (Phase 4, step 3).** A semantic,
  owned-types CST-backed lookup for a table field's / page action's properties
  (`SymbolDeclKind`, `SymbolProperties`). The IR models a field's number/name/type/
  class but not arbitrary per-field properties, and doesn't model actions — so these
  two niche LSP requests (`fieldProperties` / `actionProperties`) call this facade
  rather than bloating the always-parsed IR. tree-sitter stays inside `al-syntax`; no
  `tree_sitter` type crosses the boundary.
- **Owned-IR projection of the LSP front-end `ParsedFile` (Phase 4, step 1).**
  `parser::parse_file_ir(source)` produces the same `ParsedFile` (definitions / calls /
  variables / event subscribers+publishers / framework-invoked / object) as the legacy
  tree-sitter `AlParser`, but sourced entirely from `al_syntax::parse` — no S-expr
  queries. It is the ZERO-DIFF projection: it deliberately reproduces the legacy query
  set (`call_expression`-only calls, first-name-only multi-name vars, the legacy
  object-kind coverage), proven byte-identical to the legacy parser across all 335
  in-repo r0-corpus files by a new differential unit test
  (`ir_projection_matches_legacy_over_r0_corpus`). Correctness gains the IR enables
  (parenless statement calls, all multi-name vars) are deliberate fast-follows.
- **`RoutineDecl.name_origin`** (al-syntax IR): the origin of the routine's NAME
  identifier (vs the whole-routine `origin`), for an LSP call-hierarchy item's
  `selection_range` (e.g. an event publisher's procedure-name range).

### Changed
- **LSP front-end production paths now run on the owned IR (Phase 4, step 3).**
  `handlers::field_properties`/`action_properties` call the al-syntax facade;
  the CLI `--analyze` per-procedure metrics (`main::extract_metrics_ir`) iterate the
  IR and use the canonical IR cyclomatic-complexity walker
  (`parser::routine_complexity_ir`); `analysis`'s complexity unit tests assert against
  that IR walker. The tree-sitter `analysis::calculate_complexity` + the legacy
  `AlParser` (and its 6 S-expr queries) remain ONLY as the differential-test oracle
  behind `#[allow(dead_code)]`, deleted next (Phase 4.4) when the differential becomes
  an IR-output snapshot golden.
- **L3 is now tree-sitter-free (Phase 3 complete).** `l3_workspace::project_file` no
  longer takes a tree-sitter `root` — it iterates the owned IR directly
  (`ir_file.objects` → `o.routines`), sourcing every routine's kind / attributes /
  access / body / params / return / norm-hash / source-anchor / cc-params /
  entry-temp-guard / enclosing-member from the IR. Both callers
  (`assemble_workspace` / `assemble_workspace_units`) stopped creating a tree-sitter
  `Parser` and parsing source; the IR (already produced once upstream) is the sole
  input. The IR routine set is byte-identical to the former tree-sitter routine set
  (591/591 on the corpus, malformed routines included), so the iteration switch is a
  zero-golden-change refactor. Removed ~560 lines of now-dead legacy CST extractors
  (`extract_object_name`, `index_table`, `collect_routine_nodes`, `enclosing_member_of`,
  the body-guard matchers, …); l3_workspace.rs is warning-clean.
- **L3 object & table metadata are now owned-IR-driven.** `l3_workspace::project_file`
  sources object name/number, properties (SourceTable/PageType/Subtype/
  InherentCommitBehavior/SourceTableTemporary/TableNo), `extends` target,
  `implements` interfaces, page controls, and table fields/keys/TableType from the
  owned IR (matched by start byte; legacy tree-sitter extractors only as a defensive
  fallback). New IR: `ObjectDecl.{extends_target, implements, page_controls, fields,
  keys}` + `PageControl` / `FieldDecl`. Validated byte-identical via the L3 goldens.
  (Residual tree-sitter in L3: per-routine params/attrs/kind/access metadata, object
  globals, and two body-pattern guards — `entry_temp_guard` + the table temp-contract
  `IsTemporary` guard — still walk the CST; next increment.)
- **L3 routine features are now owned-IR-driven (the last production `body_walk`
  caller is gone).** `l3_workspace::project_file` sources each routine's `PFeatures`
  from `project_routine_features_ir` (matched by start byte; a defensive legacy
  fallback only on a corpus-impossible byte-miss). The legacy `body_walk` /
  `project_routine_features` now survive ONLY as the dual-run validation oracle.

### Fixed
- **IR CFN nodes carry `source_range`** (was always `None`). The L4 branch-aware
  field-load walker reads this serde-skipped field to attribute field accesses to the
  right block level; without it, the walker reconstructed a too-narrow range from
  op/callsite leaves only and dropped statement-level field reads — diverging the L4
  cross-call `requiredLoadedFieldsAtEntry` / `dirtyAtExit` summaries. Now populated
  from each statement/block/branch IR origin, byte-identical to the legacy `cfn.rs`.
- **`RecordRef` / `RecordId` are no longer misclassified as `Record` variables.** The
  IR's record-variable test used `type.starts_with("record")`, which wrongly matched
  the distinct `RecordRef` type — seeding its record ops a spurious `Known(false)`
  temp_state via the backfill. The record-VARIABLE test now requires `Record`
  followed by whitespace/`"` (or exactly `Record`); the record-OP RECEIVER set stays
  inclusive (so `RecRef.DeleteAll` is still captured as a record op, as in legacy).

### Added
- **tree-sitter-al `call_statement` grammar node + engine integration.** A parenless
  no-arg call (`Initialize;`) — a bare identifier in statement position that owns its
  `;` — now parses as a `call_statement` node, structurally distinct from an
  ERROR-recovery bare identifier (which has no terminator and stays raw). This lets the
  owned-IR lowerer capture parenless procedure calls as call-graph edges WITHOUT
  mistaking parse-error debris for a call (the moat-polluting case). The IR lowerer
  lowers `call_statement` to a parenless Call (anchored on the callee identifier so the
  source anchor is byte-identical to the pre-grammar form); a bare identifier in
  statement position is treated as debris / semicolon-less and is NOT a call. The legacy
  tree-sitter walks (the dual-run oracle + the L3 emitter) treat `call_statement`
  transparently (unwrap to the function child), preserving byte parity. Grammar
  designed + reviewed with gpt-5.5 + gemini-3.1-pro; parenful `Foo()` and parenless
  member `Rec.Find;` are unchanged. Known residual: a parenless call written WITHOUT a
  trailing `;` (a semicolon-less final statement, rare) is not captured — never a false
  edge, and no worse than the legacy walk which captured no parenless calls at all.
- **Report dataitems modelled in the owned IR.** `ObjectDecl.report_dataitems`
  (`(name, source-table)` pairs) and `RoutineDecl.dataitem_source_table` (a dataitem
  trigger's implicit-`Rec` table) let the IR-driven L2 path seed a report dataitem
  trigger's implicit `Rec` (typed to its enclosing dataitem's source table) and the
  dataitem-name record vars across all the report's routines — parity with the legacy
  `report_dataitem_source_table` / `report_dataitem_record_vars`. Nested dataitems use
  innermost-wins (None when the innermost dataitem's table is absent, matching legacy).

### Changed
- **L2 emitter is now fully owned-IR-driven — no tree-sitter CST walk.**
  `l2_workspace::project_file` and `project_named_routine` iterate the owned AL
  syntax IR (`al_syntax::parse`) directly: objects, routines, metadata, parameters
  and per-routine `features` all come from the IR, and `project_workspace` no longer
  parses tree-sitter at all. Preconditions proven over the r0-corpus before cutover
  (object set 404/404, routine set 591/591, `(type,number,name)` 404/404,
  `parse_incomplete` 591/591); feature output is byte-identical to the legacy
  body_walk on every well-formed routine. `project_named_routine` dropped its
  `tree: &Tree` parameter. Added `al_syntax::ir::RoutineDecl.parse_incomplete` and
  `ir_walk::ir_object_type` to support the cutover.

### Fixed
- **Malformed-routine statementTree no longer carries a phantom `other` node.**
  The legacy tree-sitter ERROR-recovery emitted a spurious `{kind:"other"}`
  statement_tree child for a stray token inside a body; the IR cleanly drops the
  ERROR token. Rebaselined the one affected Rust-owned golden
  (`ws-callsite-resolutions`).

## [0.9.3] - 2026-06-26

The tree-sitter-al v3 compliance work. (v0.9.1 and v0.9.2 were tagged during the
migration; the new release test gate correctly blocked both before publishing
any binaries — v0.9.1 on the engine port, v0.9.2 on a CI-only test-harness gap —
so this is the first published v3-compliant build.)

### Fixed
- **cli-b diff differential tests are CI-safe.** They byte-compare against
  goldens in the sibling al-sem repo (`AL_SEM_DIR`, default `U:\Git\al-sem`) and
  previously panicked when that checkout was absent. They now skip when the
  goldens are not present — matching `al2dump_smoke` — so the release test gate
  (which has no al-sem) passes while dev machines still run them as the safety net.
- **Enriched-hover field/action property extraction broken against tree-sitter-al
  v3.** v0.9.0 was built by CI against the grammar repo's default branch, which had
  advanced to v3.0.0+ where a declaration's properties/triggers are wrapped in a
  `body` field (a `declaration_body` node) instead of being direct children.
  `extract_all_properties` only iterated direct children, so `al-call-hierarchy/fieldProperties`
  and `al-call-hierarchy/actionProperties` (the enriched-hover backend) returned no
  properties. It now descends into the `body` field when present, with a fallback
  to direct children for older grammars.
- **`object_body` node rename.** tree-sitter-al v3 renamed `object_body` to
  `declaration_body`; the L3 workspace name-walk now accepts both so it still stops
  at the declaration body boundary.
- **Full L2/L3 traversal port to the v3 node shapes.** v3 inserts wrapper nodes
  that broke every flat (direct-child) traversal while recursive walks kept
  working. All affected sites now descend the wrappers, restoring byte-identical
  L2/L3 projections (the R0/R1a differential goldens pass with zero divergences):
  - **statements** — a `code_block`'s statements (and a `repeat`/case-branch body)
    are nested in a `statement_block`. A shared `block_statements` helper flattens
    it inline (preserving trailing trivia order). Fixes the L5 transaction
    detectors that reported **zero** candidates (d40 transitive-load, d46
    commit-in-lifecycle, d47 io-unsafe-txn, d49 uncommitted-write-before-ui, d51
    retry-side-effect), the CFN `statementTree`, unreachable-statement detection,
    and the temp-table guard scan.
  - **case branches** — wrapped in a `case_body`; the CFN builder now reads
    branches from it (the `case_else_branch` stays a direct child).
  - **object properties** — `Subtype`/`SourceTable`/`FieldClass` live under
    `declaration_body`; object-property and field-class reads descend it.
  - **object-global var sections** — nested in `declaration_body`; global record
    variable extraction descends it.
  - **statement-position calls** — a parenless method call's parent is now the
    `statement_block`; `is_pure_statement_parent` accepts it, so calls like
    `Customer.SetRecFilter;` and `with`-receiver `Modify` are no longer mis-read as
    field accesses / dropped.
  - **object-run result-consumed** — a bare call statement's parent is the
    `statement_block`; classified as not-consumed like the old `code_block` case.
  - **member-trigger enclosing member** — a field/action/dataitem trigger's parent
    is now a `*_body` wrapper (declaration_body / report_body / ...); resolution
    steps up through it to the named member, while object-level triggers (OnRun)
    stay member-less.

### Changed
- **Grammar compliance with tree-sitter-al v3.0.1.** Source now builds and passes
  the full test suite against the v3 grammar (the `tree-sitter-al` submodule is
  updated to v3.0.1). CI builds against the grammar's default branch, so this keeps
  the source compliant with the latest parser.

### CI
- **Release pipeline now runs the test suite as a prerequisite.** `release.yml`
  gained a `test` job (`cargo test --release --all-targets`) that both build jobs
  depend on, so a tag whose tests fail against the grammar produces no binaries and
  no GitHub release. This closes the gap that let v0.9.0 ship the broken hover.

## [0.9.0] - 2026-06-26

### Changed
- **tree-sitter-al bumped to v2.6.0 (`cddeb82`).** Clean upgrade from v2.5.2-shim
  (`89b1d05`): it parses the full BC repo set (not just the base app) via new additive
  node kinds for construct-internal preprocessor patterns (`preproc_pragma_only`,
  `preproc_conditional_{option_members,labels,rendering}`, `analysisviews_section`,
  `ternary_expression`, `preproc_split_if_then_begin_else_shared`). Unwrapped code parses
  byte-identically, so engine queries needed no change. Cross-app resolution is unchanged
  on CDO (4 unknown / 13689 = 0.029%) and resolves slightly MORE on DC (resolved
  18791→19103, unknown flat at 83 / 0.252%). All cli-a detector findings/evidence/factIds
  and the (source-only) workspace fingerprint are unaffected by the grammar.

### Fixed
- **Implicitly-invoked procedures no longer flagged `unused-procedure`**
  ([al-lsp-for-agents#20](https://github.com/SShadowS/al-lsp-for-agents/issues/20)).
  Local procedures were always tagged `DefinitionKind::Procedure` and the
  `[EventSubscriber]` attribute was parsed into a separate list that never
  updated the definition's kind, so the unused-procedure exclusion was dead code
  for workspace subscribers. Subscribers are now reconciled to
  `DefinitionKind::EventSubscriber`, and an audit-surfaced class of related false
  positives is excluded too: `[Test]` methods, test handlers (`[ConfirmHandler]`,
  `[MessageHandler]`, `[PageHandler]`, ...), and public event publishers
  (`[IntegrationEvent]`/`[BusinessEvent]`, whose subscribers live in downstream
  apps that aren't loaded). `[InternalEvent]` publishers stay flagged when
  orphaned — they can only be subscribed within the same app, so an unused one is
  genuine dead code. Tracked per file in a new `implicitly_invoked` set cleared in
  `remove_file` alongside the definitions. Validated on real Document Output
  source: removes 21 false-positive public event publishers in one app while
  still flagging real dead procedures.
- **`.gitattributes`: force `eol=lf` on `tests/**/*.md` goldens.** The gate PR-summary
  (`*.prsummary.md`) and r0 goldens are byte-compared, but `*.md` lacked the `eol=lf` rule
  its `*.json`/`*.sarif`/`*.txt`/`*.html` siblings already have, so on a
  `core.autocrlf=true` checkout they materialized as CRLF and byte-mismatched the LF engine
  output (`gate_prsummary_differential`, `gate_suppress_baseline_differential`). Added the
  missing rule to match the existing pattern.
- **`.gitattributes`: force `eol=lf` on `tests/**/*.html` goldens.** The cli-a html
  differential goldens are byte-compared, but `*.html` lacked the `eol=lf` rule its
  `*.json`/`*.sarif`/`*.txt` siblings already have, so on a `core.autocrlf=true` checkout
  they materialized as CRLF and byte-mismatched the LF engine output. Added the missing
  rule to match the existing pattern.
- **Cloud-review remediation (engine-d22 branch review).** Three findings fixed:
  - `compound_call_result_receiver` validated text before the call's `(` but not after its `)`,
    so `GetCustomer().Name` (receiver of `GetCustomer().Name.Trim()`) was mis-typed as
    `GetCustomer`'s return type, silently dropping the trailing `.Name` — a false resolution.
    Now balance-walks from the first `(` to its matching `)` and declines unless that `)` is the
    final char (accepts arg-list dots/nesting like `Func(a.b, G(x))`; rejects `Func().Field` /
    `Func().Other()`). Regression test added.
  - `compound_receiver_shape` truncated the diagnostic tag with a raw `[..120]` byte slice, which
    panics when byte 120 is not a UTF-8 char boundary (localized AL identifiers are non-ASCII).
    Now floors to a char boundary — honors the "engine never panics" contract.
  - `extract_record_variables` (local record vars) still scanned only direct `var_section`
    children, so a `#if`-guarded local record var was missed while the object-global paths
    (fixed earlier) were not. Now uses `var_section_declarations`, mirroring them.
- **Preprocessor-guarded object globals are now extracted.** A global variable declared inside a
  `#if`/`#else` block in a var section — `var #if BC24 NoSeriesMgt: Codeunit "No. Series" #else
  NoSeriesMgt: Codeunit NoSeriesManagement #endif` (ubiquitous in BC version-compat code) — was
  invisible to both object-global extractors (scalar + record), which only scanned direct
  `var_section` children and skipped the `preproc_conditional_var` wrapper. Every member call on
  such a global (`NoSeriesMgt.GetNextNo(...)`) degraded to `Unknown{UntrackedReceiver}`. A new
  `var_section_declarations` helper descends through the preprocessor wrappers; same-name branches
  are de-duplicated first-wins (mutually exclusive at compile time). DC deps-loaded:
  realUnknownRate 0.304% → 0.252% (unknown 100→83).

### Added
- **`Version`/`File` static receivers + `CompanyProperty`/`SessionInformation` singletons.**
  `Version.Create(...)` and `File.Exists(...)`/`File.Open(...)` now resolve via the static-type
  interception (File/Version value-type catalogs); `CompanyProperty.DisplayName()` and
  `SessionInformation.*` resolve via the Step-2c singleton interception (new `CompanyProperty`
  framework kind with its 3-method catalog; `SessionInformation` kind already existed). DC
  deps-loaded: realUnknownRate 0.337% → 0.304% (unknown 111→100).
- **`this.OwnMethod()` self-instance calls resolve.** A bare `this` receiver (the modern-AL
  self-instance qualifier, e.g. `this.CTSCDNUpdateeDocumentStatus(...)` in a PageExtension) now
  types as the new `ReceiverType::SelfObject` and dispatches the method among the CALLER routine's
  own object's procedures (by `object_id`) — so it resolves for ANY object kind, including
  PageExtension/TableExtension that have no `ObjectKind` variant. The object-dispatch resolution
  tail was factored into a shared `resolve_method_in_object` helper. DC deps-loaded:
  realUnknownRate 0.36% → 0.337% (unknown 118→111).
- **Enum/option VALUE references (`::`) resolve as enum receivers.** An enum member-access
  expression used as a receiver — `Rec."Document Type"::Order.AsInteger()`,
  `Enum::"CDC Translate To Type"::Item.AsInteger().ToText()`, `EMailLog."Linked to Table"::Customer.AsInteger()`
  — now types as `Framework{Enum}` so `.AsInteger()`/`.Ordinals()`/`.Names()` classify `builtin`.
  The `enum_receiver` helper (generalized from the prior `Enum::`-only handler) covers the
  static-type, type-value, and field-value forms; object-ID `::` refs (`Codeunit::"X"`,
  `Page::"X"`, …) are excluded (they yield Integer, not enum). `framework_method_return_type`
  now maps Enum `AsInteger` → Integer so the `.AsInteger().ToText()` chain resolves. Big win on
  document-type-heavy code: **DC deps-loaded realUnknownRate 1.00% → 0.36% (unknown 330→118)**;
  CDO 0.037% → 0.029%.
- **Enum type NAME as a static receiver.** A bare/quoted identifier that names an Enum object,
  used as a receiver — `"CDO Send on Posting".FromInteger(x)`, `MyEnum.Names()` — now types as
  `Framework{Enum}` (resolved via a symbol-table `object_by_type_name("Enum", …)` lookup), so its
  static methods classify `builtin`. A real variable of the same name shadows it. CDO deps-loaded:
  untracked-receiver 2→1, realUnknownRate 0.044% → 0.037%.
- **Text/Code table fields resolve as Text receivers; field-kind resolution unified.** A
  Text/Code-typed table field used as a member receiver — `"Azure Blob Private Endpoint URL".Trim()`
  (implicit Rec), `CollectedErrors."Additional Information".Contains(...)` (declared record) —
  now types as `Framework{Text}` so its Text methods classify `builtin`. The field-type→kind
  mapping (blob/media/enum/option/text/code) is now a single shared `field_receiver_kind` helper
  used by BOTH the declared-record (`compound_field_receiver_kind`, renamed from
  `compound_blob_media_field_kind`) and implicit-Rec (`implicit_rec_field_builtin_kind`) paths,
  so they can no longer drift. CDO deps-loaded: compound-receiver 4→3, untracked-receiver 3→2,
  realUnknownRate 0.058% → 0.044%.
- **`Enum::"X"` static-type receivers.** `Enum::"CDO Module Type".Ordinals()` / `.Names()` —
  a static enum TYPE reference via the generic `Enum::` qualifier — now types as `Framework{Enum}`
  so its static methods classify `builtin` via the EnumType catalog (and `Ordinals`/`Names` chain
  to List). Only the literal `Enum::` form matches; a value reference `SomeEnum::Value` is left
  untouched. CDO deps-loaded: compound-receiver 6→4, realUnknownRate 0.073% → 0.058%.
- **`System` pseudo-singleton receiver.** `System.GetCollectedErrors()`, `System.Today()`, and
  the other qualified forms of AL's global runtime functions now classify `builtin` via a new
  `System` framework singleton (75-method catalog from the compiler `System` surface), wired
  into the Step-2c singleton interception alongside `Session`/`Database`/`NavApp`. CDO
  deps-loaded: untracked-receiver 5→3, realUnknownRate 0.088% → 0.073%.
- **`Text`/`Code`/`Label` static receivers + `this.<member>` self-qualifier.** Two Phase-A
  receiver-typing additions: (1) the static-type-receiver interception (previously Xml-only) now
  also covers `Text`/`Code`/`Label`, so `Text.CopyStr(...)` and the other Text data-type static
  methods classify `builtin` via the Text catalog when no variable shadows the bare type name;
  (2) a `this.<member>` receiver (the AL self-instance qualifier) strips the `this.` prefix and
  re-infers on the remainder, so `this.DialogWindow.Open()` resolves via the `DialogWindow`
  object global (Dialog). CDO deps-loaded: compound-receiver 8→6, untracked-receiver 9→5,
  realUnknownRate 0.131% → 0.088%.
- **`ControlAddIn`-typed variables resolve as control-add-in receivers.** A variable or
  parameter declared `ControlAddIn "X"` (e.g. `HTMLEditor: ControlAddIn "CDO.Editor"`,
  `editorAddIn: ControlAddIn "CDO.Editor"`) now classifies as the `ControlAddIn` framework
  receiver, so its member calls (`HTMLEditor.InitEditor(...)`, page-callback methods) classify
  `builtin` — JS-side platform invocations with no in-AL target — instead of
  `Unknown{NonObjectReceiverType}`. Same honest classification already applied to page
  UserControl receivers. CDO deps-loaded: non-object-receiver-type 6→0, realUnknownRate
  0.175% → 0.131%.

### Fixed
- **Quoted identifiers containing `(`/`[`/`.` parse as simple receiver names.**
  `simple_receiver_name` rejected any quoted identifier whose inner text contained `(` or `[`,
  misclassifying common BC field/var receivers like `"Request Page (xml)"`, `"Amount (LCY)"`,
  `"A.B"` as compound `call-result` expressions — so `"Request Page (xml)".CreateOutStream(...)`
  and friends fell to `Unknown{CompoundReceiver}`. Those characters are LEGAL inside an AL quoted
  identifier; only an embedded `"` (e.g. `"A"."B"`) signals a real compound. Now resolves the
  member call on the quoted field (Blob/stream intrinsics, etc.). CDO deps-loaded:
  compound-receiver 17→8, realUnknownRate 0.241% → 0.175%.
- **Compound framework chains accept RecordRef/FieldRef/KeyRef bases.** The single-hop
  framework-chain resolver (`compound_framework_property_kind`) only matched a
  `Framework{kind}` base, so `RecRef.Field(n).SetRange(...)` and `SourceRecRef.KeyIndex(1).M()`
  — whose base `RecRef` infers to the DEDICATED `ReceiverType::RecordRef` variant, not
  `Framework{RecordRef}` — fell to `Unknown{CompoundReceiver}`. A new `framework_kind_of` helper
  maps the dedicated `RecordRef`/`FieldRef`/`KeyRef` receiver-type variants to their catalog
  kind, so the chain resolves (`RecRef.Field(n)` → FieldRef → `SetRange`/`SetFilter` builtin).
  CDO deps-loaded: compound-receiver 22→17, realUnknownRate 0.278% → 0.241%.

### Added
- **Enum/Option table fields resolve as enum-value receivers.** An Enum/Option-typed table
  FIELD used as a member receiver — `Rec."eSeal Service".Ordinals()`,
  `EMailTemplateLine."Mail Importance".AsInteger()`,
  `EMailTemplateHeader."Report Selection Usage".AsInteger()` — now types as the new
  `Framework{Enum}` value-instance receiver (catalog `AsInteger`/`FromInteger`/`Names`/`Ordinals`
  from the compiler `EnumType` surface). The field-of-record compound resolver, previously
  blob/media-only, now recognizes enum/option fields via first-token data-type matching (covers
  native `Enum "X"` and dep-ABI `format_type` output). `framework_method_return_type` maps Enum
  `Names`/`Ordinals` → List, so the chained `Rec."eSeal Service".Ordinals().Count()` resolves.
  CDO deps-loaded: compound-receiver 31→22, realUnknownRate 0.343% → 0.278%.
- **Xml framework type names resolve as static receivers.** `XmlElement.Create(...)`,
  `XmlDocument.ReadFrom(...)`, `XmlDeclaration.Create(...)`, `XmlText.Create(...)` invoke STATIC
  factory/utility methods on the framework type itself. When the bare type name has no declared
  variable shadowing it, Phase A now types it as `Framework{Xml}` (an explicit allow-list of Xml
  value types — EXCLUDES `XmlPort`, an AL object type), so Phase B classifies the static method
  via the shared Xml builtin catalog. `framework_method_return_type` also maps the Xml `Create*`
  factories → Xml, so chained `XmlElement.Create(Name).AsXmlNode()` resolves. CDO deps-loaded:
  untracked-receiver 17→9, compound-receiver 35→31, realUnknownRate 0.431% → 0.343%.
- **Named return values are tracked as in-scope variables.** A procedure with a NAMED return
  value — `procedure CreateDefaulteDocsSendCode() SendCode: Record "CDO Send Code"` — exposes
  that name as a usable variable inside the body (`SendCode.Insert()`, `SendCode.GetX()`). The
  routine scope projection now seeds the named return as a record variable (when record-typed)
  AND a general scalar variable (any type: `Codeunit`/`Interface`/framework), mirroring a local
  declaration. Member calls on a named return now resolve instead of falling to
  `Unknown{UntrackedReceiver}`. CDO deps-loaded: untracked-receiver 28→17, realUnknownRate
  0.511% → 0.431%.
- **`ALDUMP_DEBUG_UNKNOWN` diagnostic** — `--l3-unknown-breakdown-cross-app` now honors the
  `ALDUMP_DEBUG_UNKNOWN` env var (set to `1` for all, or a substring to filter by receiver
  shape) to dump each residual unknown edge's owning object/routine + receiver shape + method
  to stderr. The work-list tool for locating the exact source behind each breakdown bucket.
- **Report dataitem names resolve as record variables.** AL lets you reference a report
  `dataitem(Name; "Source Table")` BY NAME as a record typed to its source table — e.g.
  `"Sales Header Filter".GetView()` / `.GetFilters()` / `.SetRange(...)` for
  `dataitem("Sales Header Filter"; "Sales Header")`. The dataitem name is in scope across ALL
  of the report's routines (report-level procedures + sibling dataitem triggers), so the routine
  projection now seeds EVERY dataitem's name as a record variable typed to its source table
  (`record_types` pass-1 resolves the `table_id` by name). Distinct from the per-dataitem
  implicit `Rec` of a dataitem trigger. Member calls on dataitem-named records now classify
  `builtin` instead of `Unknown{UntrackedReceiver}`. CDO deps-loaded: untracked-receiver 57→28,
  realUnknownRate 0.723% → 0.511%.

### Changed
- **Codeunit `TableNo` seeds an implicit `Rec`.** A codeunit with a `TableNo` property runs
  against an implicit `Rec` of that table (its `OnRun(var Rec)` parameter; `Rec` is exposed
  unqualified inside the codeunit), so `Rec.<proc>()` / `Rec.<field>` in such a codeunit now
  resolve instead of falling to `Unknown{UntrackedReceiver}`. `TableNo` is read in the routine
  projection (NAME or NUMBER) and set as the seeded `Rec`'s `table_name`; `record_types` pass-1
  now resolves either form via `resolve_table_ref_to_id`. CDO untracked-receiver 81→57,
  realUnknownRate 0.898% → 0.723%; DC untracked 153→85, 1.71% → 1.49% (DC has many TableNo
  processing codeunits).

### Added
- **Framework method/property return chains** — extends the single-hop framework-property
  compound resolver to framework METHOD calls that return a framework type:
  `JsonToken.AsValue()` → JsonValue, `XmlNode.AsXmlElement()` → Xml, `RecordRef.Field(n)` →
  FieldRef, `ErrorInfo.CustomDimensions` → Dictionary, etc. So a chain like
  `JTok.AsValue().AsInteger()` / `RecRef.Field(n).Value()` classifies `builtin` instead of
  `Unknown{CompoundReceiver}`. New `framework_method_return_type` map; `compound_framework_property_kind`
  now handles both the property and method-call form of `<prop>`. These AL framework conversions
  are deterministic (the return type never varies), so resolution is precise. CDO deps-loaded:
  compound-receiver 53→35, realUnknownRate 1.03% → 0.898%.
- **Single-hop call-result compound receivers** (Feature C2, engine-d22). A
  compound receiver `Func().Method(...)` — a member call ON THE RESULT of a bare
  own-object procedure with a KNOWN return type — now types the receiver as that
  return type and dispatches the method on it, instead of degrading to a
  `compound-receiver::call-result` unknown. `compound_call_result_receiver` in
  `receiver_type.rs` parses the bare `<Name>` (text before the first `(`, declining
  any `.`-bearing / non-bare form), resolves it to EXACTLY ONE same-name routine in
  the caller's object (mirroring `infer_call_expr_return_type`'s single-match
  precision gate; overloaded / absent / global-only names decline), reads its
  `return_type`, and classifies it via `parse_object_type_ref` (Object kinds) /
  `classify_receiver` (Record / framework kinds). PRECISION-FIRST: it DECLINES on
  ANY uncertainty — no return type, an Interface/Enum return, a primitive scalar /
  `Variant` / unparseable return — so a wrong return-type guess never masks a real
  hole. Example win: `HelperRec(Customer).FindSet()` (where `HelperRec(): Record
  Customer`) now classifies the `FindSet` as a Record `builtin`.
- **Single-hop framework-property compound receivers** (Feature C1, engine-d22).
  A compound receiver `<fw>.<prop>.<method>()` where the base types as a
  `Framework{kind}` and `<prop>` is a framework-returning property of that kind
  (e.g. `HttpClient.DefaultRequestHeaders.Add('k','v')`,
  `HttpResponseMessage.Content.ReadAs(...)`) now resolves to the property's
  framework type and classifies the method via the builtin catalog instead of
  degrading to a `CompoundReceiver` unknown. New `framework_property_type(kind,
  property_lc)` in `member_builtins.rs` maps the well-known Http* property returns
  (`HttpClient.DefaultRequestHeaders : HttpHeaders`, `Http{Request,Response}Message.{Content,Headers}`,
  `HttpContent.Headers`); `compound_framework_property_kind` in `receiver_type.rs`
  wires it as a single-hop compound resolver alongside the existing blob/media and
  CurrPage-control compound paths.
- **AL platform-type builtin catalogs — non-object-receiver win** (Feature A,
  engine-d22). The `non-object-receiver-type` unknown bucket previously included
  member calls on AL platform value types (`Notification`, `ErrorInfo`, `Text`,
  `RecordId`, etc.) that have real builtin method surfaces but were not wired into
  the resolver's builtin catalog. 26 new `ReceiverBuiltinKind` variants + `phf_set!`
  catalogs (method counts: Notification 9, ErrorInfo 18, ModuleInfo 7, RecordId 2,
  BigText 6, SecretText 3, DataTransfer 9, SessionSettings 9, Text/Code/Label 32,
  Date 6, DateTime 3, Time 5, Guid 3, Integer 1, Decimal 1, Boolean 1, Duration 1,
  BigInteger 1, Byte 1, File 28, FileUpload 2, NumberSequence 7, Version 6,
  FilterPageBuilder 11, SessionInformation 4). `classify_receiver` now also strips
  length suffixes (`Text[1024]` → `text`, `Code[20]` → `code`). `Code` and `Label`
  alias to the `Text` kind. Sourced from `tools/gen-al-builtins/out/member_builtins.json`.

### Changed
- **L3 analysis scopes to one app at nested-`app.json` boundaries** (multi-app / monorepo
  support). The disk assembly (`assemble_l3_workspace_from_disk`, used by `aldump` + the
  cross-app stats) previously fail-closed when a workspace contained more than one `app.json`
  anywhere in its tree — so a monorepo with a root app plus nested sub-apps (e.g. Continia
  Document Capture: root + `Modules/Purchase Contracts/{Base,Integration}`) could not be
  analyzed at all. New `discover_al_files_app_scoped` treats a child directory carrying its
  own `app.json` as a SEPARATE project (the AL compiler's own semantics) and does NOT descend
  into it, so the targeted app's source is analyzed in isolation; each nested app is analyzed
  by pointing the workspace at its own root. The `count_app_json > 1` guard is dropped from
  this path (a missing/id-less root `app.json` still fail-closes via `read_root_app_guid`).
  The GATE keeps its own stricter multi-app provider check (`workspace_diagnostics`) — only
  the analysis path is relaxed. Unblocks Document Capture (28.4k edges, source-only
  realUnknownRate 1.83%) and its module apps.

### Fixed
- **Quoted scalar variable names strip their quotes** (consistency with parameter and
  record-variable extraction). `extract_variables` (locals) and `extract_object_globals` keyed
  a `quoted_identifier` variable by its raw text INCLUDING quotes (`"file blob"`), but
  `simple_receiver_name` returns the inner unquoted name (`file blob`), so a member call on a
  quoted scalar variable `"My Var".M()` missed the variable lookup → `Unknown{UntrackedReceiver}`.
  New `decl_name_lc` helper strips quotes on both scalar sites, matching the param/record-var
  treatment. (No metric change on CDO — its residual untracked names are Blob FIELDS, not
  quoted variables — but removes the latent asymmetry.)
- **Grouped multi-name variable declarations capture every name.** The AL grammar's
  `variable_declaration` multi-name arm (`A, B, C : Type;`) emits one `name` field per
  variable, but `scope.rs` read only `child_by_field_name("name")` (the FIRST), silently
  dropping `B`/`C` across all four extraction sites (local vars, object globals, local record
  vars, object-global record vars). Trailing names in a group were therefore untyped →
  `Unknown{UntrackedReceiver}` on any member call (and invisible to L5 detectors). New
  `decl_name_nodes` helper iterates `children_by_field_name("name", …)`; each declared name
  becomes its own symbol. CDO deps-loaded: untracked-receiver 147→136, realUnknownRate
  4.4941% → 4.4182%. No fixture uses grouped decls, so all goldens stay byte-stable.
- **Dependency symbols: recurse `Namespaces[]`** — the single biggest cross-app resolution
  hole. `engine::deps::symbol_reference::parse_symbol_reference` read only TOP-LEVEL object
  arrays (`Pages`, `Codeunits`, `Tables`, …). BC 24+ apps (every modern Microsoft + ISV
  `.app`) nest objects under `Namespaces[]` nodes, so the parser dropped almost the entire
  dependency object/routine/table set (Microsoft Base Application 28.0: top-level Pages = 10,
  recursive = 2609 — ~99% lost). `raw_objects` now recurses every `Namespaces[]` level via
  `collect_raw_objects`. Combined with the three resolution fixes below, drove CDO
  deps-loaded realUnknownRate **6.6767% → 4.4941%** (unknown 933→628, resolved 7390→7952,
  external 304→15, record-table-procedure 296→0). Flat (pre-BC24) `.app`s are unaffected
  (no `Namespaces` node → no recursion), so all existing goldens stay byte-stable.

### Changed
- **Member-of-member Blob/Media field receivers resolve.** A compound receiver
  `<recvar>.<field>` where `field` is a `Blob`/`Media`/`MediaSet` field of the record's table
  (`DOTempBlob.Blob.CreateOutStream(...)`, `PDFDocument."File Blob".CreateInStream(...)`) now
  classifies the field intrinsic as `builtin` instead of `Unknown{CompoundReceiver}`.
  `infer_receiver_type` splits on the LAST `.`, resolves the base record's table, and looks up
  the field — reusing the Blob/Media catalogs. Deeper chains (`CurrPage.<Part>.Page`) still
  decline (the base is itself compound). CDO deps-loaded: compound-receiver 243→170,
  realUnknownRate 2.88% → 2.34%.
- **Table procedures (not just triggers) seed the implicit `Rec`.** `implicit_base_receiver`
  only registered the implicit current record for table/tableextension TRIGGERS, but AL exposes
  the table's fields and procedures unqualified inside ANY of its methods. Broadened to table
  procedures, so (a) bare record-builtin calls (`Modify()`, `SetRange()`, …) in a table
  procedure are correctly captured as RECORD OPERATIONS on `Rec` instead of phantom
  global-builtin call edges; (b) explicit `Rec.<proc>()` and bare field accesses resolve. CDO
  deps-loaded: untracked-receiver 136→81, realUnknownRate 3.208% → 2.88% (266 phantom builtin
  call edges reclassified to record operations — a more accurate call graph, not lost edges).
  Regenerated `ws-d40` r1a/r2a goldens (the one fixture with a table procedure) — adds its
  implicit `Rec` record variable; no call-graph/coverage/detector golden changed.
- **Blob / Media field receivers resolve to field intrinsics.** A `Blob`/`Media`/`MediaSet`
  table FIELD used as a member receiver — bare on the implicit `Rec` (`"File Blob".CreateInStream(...)`)
  or as a declared `Blob` variable — now classifies the field intrinsic
  (`CreateInStream`/`CreateOutStream`/`HasValue`/`Length`; media import/export/query) as
  `builtin`. New `ReceiverBuiltinKind::Blob`/`Media` + catalogs; `classify_receiver` maps the
  type names; `infer_receiver_type` resolves a bare blob/media field of the implicit Rec's
  table.
- **Bare calls resolve against the implicit `Rec` (SourceTable) procedures.** AL treats an
  unqualified call in page/table code as `Rec.<proc>()`, so a bare call to a SourceTable
  procedure is legal (e.g. `GetTemplateVariantCaption()` in a page bound to the table that
  defines it; `Navigate()` resolving to the base table's `Navigate`). `PCallee::Bare` now adds
  a fallback (after own-object and extends-target, before global-builtin/`BareUnresolved`):
  resolve the caller object's implicit table (Table self / Page `SourceTable` / extension
  base) ∪ its TableExtensions via `resolve_by_name_and_arity_multi`. Own-object procedures are
  still tried FIRST so they shadow a same-named table procedure. New
  `implicit_rec_table_object_id` helper (NAME- or NUMBER-form table ref). CDO deps-loaded:
  bare-unresolved 169→0, realUnknownRate 4.4182% → 3.208% (resolved +170). The fallback only
  binds to a REAL name+arity match, so it cannot invent edges.
- **Record member dispatch searches base table ∪ its TableExtensions.** A `TableExtension`
  procedure is globally callable on the base record in AL but lives under the extension's own
  object id, so `routines_in_object(base_table)` missed it (false `Unknown{RecordTableProcedure}`).
  Added `SymbolTable::table_extension_object_ids` (TableExtensions indexed by extends-target
  name AND number) + `resolve_by_name_and_arity_multi` (one candidate pool over a set of
  object ids); `dispatch_record` now unions the base table with every TableExtension extending
  it. Resolves e.g. CDO's `Rec.CDOOpenEmail()` (defined in a CDO `TableExtension` on a base
  BC table).
- **Numeric `SourceTable` / extends-target resolution.** Dependency `.app` symbols encode a
  page's `SourceTable` and an extension's extends target as the table's object NUMBER (e.g.
  `"5992"`); native AL source uses the table NAME. `record_types::resolve_table_ref_to_id`
  resolves both forms — a numeric ref routes through `object_by_type_number("Table", n)`
  (type-qualified) → name → `L3Table.id`. Lets a PageExtension's implicit `Rec` bind to its
  base page's SourceTable when that base page is a dependency object.
- L3 implicit-`Rec`/`xRec` receiver typing: a member call on the implicit record now types as
  `ReceiverType::Record` whenever a `record_variables` entry exists for it, REGARDLESS of
  whether its table object id resolves (a cross-app SourceTable leaves `table_id` None). Phase
  B then decides honestly (builtin → `builtin`; table procedure on an unresolved table →
  `RecordTableProcedure`). Mirrors the existing table-id-independent decision for declared
  record vars. Diagnostic: `RecordTableProcedure` edges now carry a `receiver_shape` sub-cause
  tag (`table-unresolved::…` vs `proc-not-found::…`) for `--l3-unknown-breakdown[-cross-app]`.

### Added
- **Page-control resolution — `CurrPage.<control>…` member calls.** New `L3Object.page_controls`
  (`L3PageControl { name, kind: Part/SystemPart/UserControl, target }`), populated from BOTH the
  native AL layout (tree-sitter `part_section`/`systempart_section`/`usercontrol_section`) and
  dependency `.app` symbols (`Controls[]` integer `Kind`: 6=Part → subpage page NUMBER via
  `RelatedPagePartId.Id`, 10=UserControl → add-in name via `RelatedControlAddIn`; recursed through
  nested controls). `SymbolTable::page_controls_for(object_id)` merges a PageExtension's own
  controls with its base page's. At resolution, `currpage_control_receiver` (a "Step 0" in
  `infer_receiver_type`) resolves:
  - `CurrPage.<Part>.Page.<m>()` / `CurrPage.<Part>.<m>()` → the subpage **Page object's** procedure
    (subpage found by NAME in native source, NUMBER in dep symbols; Phase B dispatches the Page
    receiver's method by name+arity — object-run is Codeunit-gated, so this is a plain procedure
    lookup).
  - `CurrPage.<UserControl>.<m>()` → a control-add-in `builtin` edge (below).
  CDO deps-loaded: compound-receiver 170→62, realUnknownRate **2.336% → 1.548%** (resolved +63,
  builtin +37; total edges unchanged). No fixture exercises page controls, so all goldens stay
  byte-stable.
- **`CurrPage.<UserControl>.<method>()` resolves to a control-add-in `builtin` edge.**
  A page `usercontrol(Body; "Some AddIn")` accessed as `CurrPage.Body.SetContent(...)`
  is a platform/JS-side control-add-in invocation with no in-AL target. Phase A's
  `currpage_control_receiver` now types a `UserControl` control as the new
  `ReceiverBuiltinKind::ControlAddIn` framework receiver; Phase B's `dispatch_framework`
  classifies EVERY method on it as `builtin` (we cannot enumerate an add-in's JS method
  surface, and these are genuine platform calls — never real-`unknown`, and not the
  runtime-typed `dynamic` dispatch). Previously these declined to
  `Unknown { CompoundReceiver }`. Test in `tests/l3cg_page_part_dispatch.rs`.
- **Extension bare-call resolver**: when a bare call in a `PageExtension` /
  `TableExtension` / `ReportExtension` / `EnumExtension` is not found in the caller's own
  object, the resolver now falls back to the EXTENDS-TARGET base object's procedures before
  emitting `Unknown{BareUnresolved}`. Order: own-object → extends-target base → global
  builtin → `BareUnresolved`. Adds `SymbolTable::object_by_id` (exact-id index) and
  `extends_base_object` helper in `call_resolver.rs`. CDO cross-app (deps-loaded): unknown
  943 → 933 (−10 bare-unresolved edges now resolved); source-only: unchanged (CDO base
  pages are dep objects, only visible when `.alpackages` are loaded).
- `aldump --l3-unknown-breakdown-cross-app <workspace>`: the DEPS-LOADED, PRIMARY-scoped
  unknown breakdown — the north-star work-list. Same merged-model + primary-edge scoping as
  `--l3-call-graph-stats-cross-app`, but attributes every residual TRUE-`unknown` edge to its
  `UnknownReason` (`byReason` / `receiverShapeDetail` / `bareCallDetail` /
  `frameworkMethodDetail`) so the real whole-program holes can be targeted directly rather
  than inferred from the source-only breakdown. Fail-closed → message + empty breakdown.
- `aldump --l3-call-graph-stats-cross-app <workspace>`: deps-loaded, PRIMARY-scoped
  honest-taxonomy histogram. Builds the cross-app merged model (workspace `.al` source +
  dep `.app`s under `.alpackages`), runs call resolution with the real declared/fetched dep
  ledger, then scopes the histogram to **primary (workspace) edges only** — edges whose
  `from` routine is NOT a dep routine (`dep_routine_ids = {r | r.app_guid ∈
  fetched_app_guids}`). Same JSON shape as `--l3-call-graph-stats` plus `depAppsLoaded`.
  This is the honest whole-program real-`unknown` rate (dep symbols present for resolution;
  dep-internal call sites excluded from the denominator). CDO baseline (10 dep apps loaded):
  source-only 6.88% → deps-loaded primary 6.75% (resolved 7120→7380 +260; unknown 961→943
  -18; external reclassified from unknown 558→304 with cross-app resolution active).

### Changed
- L3 member dispatch: a `Variant`-typed receiver now classifies `dynamic` (spec §6
  honest taxonomy — the held type is runtime-determined) instead of real-`unknown`.
  `ReceiverType::Dynamic` + `dynamic_method` emit a `dispatch_kind = Dynamic` edge. CDO:
  non-object-receiver-type 70→68, realUnknownRate 6.89%→6.88% (no new resolved edges).

### Fixed
- **Witness reachability via reverse-BFS valid-node set** in `reconstruct_witness_paths`
  (Case C inherited-fact BFS): the per-edge `can_reach` memoized check (which scanned
  the full direct-∪-inherited capability cone per node, calling `fact_equivalent` ~750k
  times per root on the CDO app) is replaced by a **one-shot reverse-BFS** computed once
  per `reconstruct_witness_paths` call. Carrier nodes (those with a direct fact equivalent
  to the target) are found by scanning `direct_facts_by_routine` (far fewer facts than the
  inherited cone). A reverse-BFS from those carriers over the new `incoming_edges` index
  (reverse of `typed_edges`, built once in `build_fingerprint_indexes`) computes
  `valid_nodes: HashSet<&str>` — the set of nodes that can reach `fact` in the forward
  call graph. The per-edge prune is now an O(1) `valid_nodes.contains(to)` check.
  Correctness: `facts_by_routine[N].any(equiv fact)` ≡ "N is an ancestor-of-or-equal-to
  some carrier in the forward graph" ≡ "N ∈ reverse-BFS from carriers" — the valid set is
  identical. All goldens and contracts remain byte-stable. CDO `alsem analyze` wall time
  ~20 min → < 1 min.
- **Skip non-ordering witness reconstruction** in `compute_digest_effects_for_ordering`:
  the ordering engine only grades `DB_INSERT / DB_MODIFY / DB_DELETE / COMMIT / HTTP /
  FILE / UI_CONFIRM / UI_MESSAGE / UI_WINDOW_OPEN / ERROR_THROW`; for all other effect
  types it treats effects with empty `via_paths` and `owner == routine_id` as direct
  (empty `CallChain`). The new `ordering_witness_only: bool` parameter to `digest_query`
  (passed `true` from `compute_digest_effects_for_ordering`, `false` from all other paths)
  skips `reconstruct_witness_paths` for non-ordering-relevant effect types, emitting the
  effect with empty `via_paths`. Digest shape and `scoped_guarantees` are unchanged; the
  R4-F and CLI-B goldens remain byte-stable.
- **Parent-pointer arena BFS** in `reconstruct_witness_paths` (Case C inherited-fact
  witness): replaced the cloned `State { routine, hops: Vec<WitnessHop>, visited:
  HashSet<String> }` (cloned in full on every edge expansion) with a `Node { routine,
  hop, parent, depth }` arena + `VecDeque<usize>` index queue. Visited-set check is now
  O(depth) via a `Vec<String>` parent-chain walk (one allocation per *popped* node, shared
  across all out-edge checks for that node). Path materialisation walks parents on
  completion only (rare). Eliminates the `O(depth * out_degree)` per-expansion clone of
  both the `HashSet<String>` and the `Vec<WitnessHop>` that dominated the per-state cost
  (~46 µs/state). Eliminates per-expansion allocation overhead; all existing goldens and
  contracts remain byte-identical. (CDO `analyze` wall time is dominated by the total
  number of `(root, fact)` BFS invocations on large workspaces, which this change does not
  address — see next milestone.)
- L5 ordering/digest witness reconstruction no longer blows up on dense call graphs
  (the Record-table-procedure + implicit-Rec dispatch edges densified out-degree, which
  made `alsem analyze` effectively non-terminating on the CDO app — 15k+ CPU-s). Three
  behavior-preserving fixes (all `*.l3*`/r4f/digest/cli-b goldens byte-stable): (1)
  **reachability-directed pruning** in `reconstruct_witness_paths` — a frontier edge whose
  target cannot reach the target fact (per the already-computed `facts_by_routine` cone)
  is skipped, discarding the dead-end subtrees that exhausted the 25k-state budget (was
  ~83% of calls hitting the cap → 0%); (2) out-edges **pre-sorted once** at index build
  instead of cloned+sorted per BFS state; (3) `compute_ordering_facts` restricted to roots
  whose cone carries an IO/UI effect (the only roots that can yield an ordering label),
  via the new `compute_digest_effects_for_ordering` — skipped roots produce empty ordering
  facts, so the result is identical.

### Added
- **AL singleton-type static receivers → builtins** (`src/engine/l3/member_builtins.rs`,
  `src/engine/l3/receiver_type.rs`): `infer_receiver_type` Step 2c now intercepts the
  AL platform singleton type names (`IsolatedStorage`, `Session`, `NavApp`,
  `TaskScheduler`, `Database`, `Page`, `Report`) in addition to the existing
  `CurrPage`/`CurrReport` intercepts, before emitting `UntrackedReceiver`. Five new
  `ReceiverBuiltinKind` variants are added (`IsolatedStorage` 5 methods,
  `Session` 19, `NavApp` 16, `TaskScheduler` 5, `Database` 29); `Page`/`Report` bare-name
  singletons reuse the existing `PageInstance`/`ReportInstance` catalogs. Phase B's
  existing `Framework` arm dispatches via the catalogs: catalog hit → `builtin`,
  catalog miss → `Unknown { FrameworkMethodNotInCatalog }` (honest gap). The
  variables-first check (Step 2) is preserved — a user variable named `Session` correctly
  shadows the singleton. 6 new tests in `tests/l3cg_singleton_static_dispatch.rs`.
  CDO `DocumentOutput/Cloud` (13,971 total edges): `unknown` 1,093 → 963 (−130),
  `builtin` 5,079 → 5,209 (+130), `resolved` UNCHANGED at 7,120 (pure reclassification);
  `realUnknownRate` 7.82% → 6.89% (−0.93 pp). Breakdown: `page` −50, `isolatedstorage`
  −38, `report` −16, `session` −13, `navapp` −9, `taskscheduler` −4.
- **Name residual unknowns in `--l3-unknown-breakdown`** (`src/engine/l3/call_resolver.rs`,
  `src/engine/l3/receiver_type.rs`, `src/engine/l3/resolution_class.rs`, `src/bin/aldump.rs`):
  the `BareUnresolved` path now threads the lowercased call name onto `CallEdge::unknown_method_name`
  so the breakdown can emit a per-name count histogram (`bareCallDetail`). Untracked-receiver
  `other` shapes now embed the actual variable name in the shape tag
  (`"other::<name>"` instead of a flat `"other"`) and compound-receiver `member-of-member`
  shapes embed the receiver expression (truncated to 120 chars), so `receiverShapeDetail`
  surfaces concrete identifiers. `unknown_breakdown` returns a 4-tuple (adding `bareCallDetail`
  split from the framework-method detail); `aldump` emits the new field. **Purely diagnostic —
  zero resolution/classification changes, zero golden changes.** On CDO (13,971 edges, 1,093
  true unknowns): 188 `bare-unresolved` names are now named; all 188 are user-defined
  application procedures (none are genuine platform globals — confirmed against the AL 18.0
  compiler DLL's ClassDocumentationResources); the untracked-receiver `other` bucket (252
  edges) now shows concrete names including `IsolatedStorage` (38), `Page` (50), `Report`
  (16), `Session` (13), `NavApp` (9), `TaskScheduler` (4) — a road-map for future typed-
  receiver static-method resolution.

- **Task 6a — Implicit Rec/xRec receiver resolution** (`src/engine/l3/receiver_type.rs`):
  `infer_receiver_type` Step 2b now checks `routine.record_variables` BEFORE yielding
  `UntrackedReceiver`. For Table/Page/TableExtension/PageExtension objects, pass 3 of
  `record_types::resolve_routine_record_types` sets `table_id` on the implicit `Rec`/`xRec`
  record variable. Step 2b finds this entry (case-insensitive name match, `table_id == Some`),
  walks it through `symbols.table_by_id` → `symbols.object_by_type_name("Table", name)`, and
  returns `ReceiverType::Record { table_object_id: Some(..) }` so Phase B can dispatch both
  catalog builtins (`TableCaption`, `FieldNo`, etc.) and real user table procedures. A codeunit
  with an undeclared `Rec` (no effective own table → `table_id == None`) stays
  `Unknown { UntrackedReceiver }` (correct: no false resolution). The previously deferred
  `implicit_rec_table_procedure_deferred` test in `tests/l3cg_record_dispatch.rs` has been
  promoted from "stays unknown" to "now resolves". Four new tests in
  `tests/l3cg_implicit_rec_dispatch.rs` cover: table trigger resolves, builtin stays builtin,
  page-via-SourceTable resolves, and codeunit stray Rec stays unknown.
- **Task 6a — Receiver-shape sub-characterization in `--l3-unknown-breakdown`**:
  Added `receiver_shape: Option<String>` field to `CallEdge` (DIAGNOSTIC-only, never projected
  to golden output). `InferredReceiver` now carries `receiver_shape: Option<String>` set by
  Phase A helpers: `compound_receiver_shape` (classifies `member-of-member` / `call-result` /
  `indexed` / `other`) for `CompoundReceiver` edges, and `untracked_receiver_shape` (classifies
  `implicit-rec` / `currpage` / `currreport` / `other`) for `UntrackedReceiver` edges. Phase B's
  `Unknown` arm propagates the shape onto the emitted edge. `resolution_class::unknown_breakdown`
  now returns a 3-tuple adding `receiverShapeDetail` (keyed by `"{reason}::{shape}"`), and
  `aldump --l3-unknown-breakdown` exposes this as `"receiverShapeDetail"` in the JSON output.
- **Phase 3 — Record table-procedure dispatch** (`src/engine/l3/call_resolver.rs`): member
  calls on `Record <Table>`-typed variables where the method is NOT a built-in intrinsic are
  now resolved to the table's user-defined procedure. The resolver looks up the receiver's
  table object id via `routine.record_variables` (resolved by `record_types` pass 1/3) then
  falls back to parsing the declared type via `record_types::record_table_name_of`, then calls
  `resolve_by_name_and_arity` with full arity/overload disambiguation. Edges become
  `resolution=resolved`, `dispatchKind=method`, `to=<routine-id>`. CDO `DocumentOutput/Cloud`
  impact: `record-table-procedure` unknown edges 806 → 66 (−740), `resolved` 6358 → 7098
  (+740), `realUnknownRate` 15.68% → 10.39% (−5.29 pp). Residual 66 unknowns are genuine
  non-resolvable cases: implicit `Rec` in table triggers (deferred to Task 6 — the implicit
  `Rec` is NOT in `routine.variables` so Step 2 returns UntrackedReceiver before Phase 3
  fires), plus calls on record vars from unindexed external tables. Detector delta vs 1867
  baseline: PENDING (analysis in progress; no new golden failures; oracle invariants pass).
  Contract oracle (Invariant 2: every resolved `to` exists in the symbol table) verified.
  Deferred: implicit-Rec table-trigger dispatch (requires Task 6 ReceiverType lattice).
  New tests in `tests/l3cg_record_dispatch.rs` (5 tests: resolve, builtin-unchanged,
  missing-stays-unknown, implicit-rec-deferred, arity-overload).
- L3 call-graph contract oracle (`tests/l3cg_oracles.rs` Invariant 11): a bare call to an
  AL platform GLOBAL function (Task 2 catalog) classifies `builtin` on the BARE path
  (dispatchKind "builtin"), is disjoint from `resolved` (no edge is both builtin and
  resolved), and a genuine non-global bare miss STILL classifies `unknown` (the catalog
  never swallows a real hole). Locks the clean-reclassification baseline before the
  graph-expansion phases. CDO `DocumentOutput/Cloud` cumulative after Tasks 1-3:
  `realUnknownRate` 23.6% → 15.68%, unknown 3295 → 2191, builtin 3639 → 4743, resolved
  unchanged at 6360 (pure reclassification, zero new resolved edges); `alsem analyze`
  1867 findings (detector baseline for the graph-expansion FP checks).
- Generated AL global-builtin catalog (`src/engine/l3/global_builtins.rs`): offline
  generator (`tools/gen-al-builtins/`) extracts all 785 distinct compiler-intrinsic method
  names from the AL compiler DLL's `ClassDocumentationResources` embedded resource
  (source: `Microsoft.Dynamics.Nav.CodeAnalysis.dll`, AL extension `ms-dynamics-smb.al-18.0.2293710`,
  97 types). The catalog is a `phf::phf_set!` checked into source; the generator is
  offline/manual (not in CI). Bare calls not resolved to the caller's own object whose
  name matches any catalog entry are reclassified from `unknown` (BareUnresolved) to
  `builtin` — a pure reclassification (no new resolved-to-routine edges). CDO impact on
  `DocumentOutput/Cloud`: bare-unresolved dropped 1247 → 188 (−1059), unknown total
  3295 → 2236, `realUnknownRate` 23.6% → 16.0%; resolved count unchanged at 6360.
- L3 call-graph: intrinsic built-in catalog (`src/engine/l3/member_builtins.rs`, `phf`
  perfect-hash) for Record / RecordRef / FieldRef / KeyRef + framework types (Json*,
  Http*, In/OutStream, TextBuilder, Dialog, List, Dictionary, Xml*). AL's
  compiler-intrinsic member methods (not present in any `.app` `SymbolReference.json`)
  now classify as `builtin` on the member resolution path instead of `unknown`. Phases
  1–2 of the call-graph resolution redesign (`docs/superpowers/specs/2026-06-13-call-graph-resolution-redesign.md`).
- Honest resolution taxonomy classifier (`src/engine/l3/resolution_class.rs`) +
  `aldump --l3-call-graph-stats` measurement harness reporting per-bucket edge counts
  and the real-`unknown` edge rate (the north-star metric).
- `aldump --l3-unknown-breakdown` + resolver-attributed `UnknownReason` on every
  `unknown` edge: attributes the residual real-`unknown` rate to its causes
  (bare-unresolved / record-table-procedure / untracked-receiver / compound-receiver
  / framework-method-not-in-catalog / non-object-receiver-type / enum-static /
  callee-unknown / interface-no-impl). The work-list for the typed-resolution phases.
  Measured on CDO (3295 unknown): bare-unresolved 1247, untracked-receiver 881,
  record-table-procedure 812, compound-receiver 243, non-object-receiver-type 70,
  framework-method-not-in-catalog 39, interface-no-impl 2, enum-static 1.
- `aldump --l3-unknown-breakdown` now includes `"frameworkMethodDetail"` in the JSON
  output: a per-`(KindName::method)` breakdown of `framework-method-not-in-catalog`
  edges, sourced from the new `CallEdge.unknown_method_name` diagnostic field. Helps
  identify specific catalog gaps without full call-graph inspection.
- Member-builtin catalog expanded from compiler JSON (`member_builtins.json`) closing
  all 18 `framework-method-not-in-catalog` unknown edges on the CDO workspace (from 39
  pre-global-builtin reclassification). Key additions: RecordRef `setrecfilter` + 26
  new Builtin entries; Record 14 new methods (arefieldsloaded, currentcompany,
  fullyqualifiedname, istemporary, readconsistency, readisolation, recordlevellocking,
  relation, securityfiltering, setascending, setbaseloadfields, tablename, truncate,
  loadfields); FieldRef 11 new enum-reflection methods; Json* types 35+ methods
  (GetArray/GetObject/GetText etc., SelectTokens, clone, YAML variants); Http*
  types expanded with certificate, cookie, secret-URI support; TextBuilder capacity
  methods; Dialog confirm/error/message/strmenu; XML types full union of all Xml*
  compiler types (60+ net-new entries). Pure reclassification — resolved count
  unchanged. CDO after: `framework-method-not-in-catalog` = 0, unknown 2209→2191,
  realUnknownRate 15.8%→15.7%.
- **CurrPage / CurrReport receiver resolution → Page / Report-instance builtins**
  (`src/engine/l3/member_builtins.rs`, `src/engine/l3/receiver_type.rs`): the two
  AL language singletons `CurrPage` and `CurrReport` — which are not declared variables
  but are the current page / report instance inside triggers — were classified as
  `Unknown { UntrackedReceiver }` with receiver-shape `currpage`/`currreport`. They
  are now intercepted in `infer_receiver_type` Step 2c (before `UntrackedReceiver` is
  emitted) and mapped to `ReceiverType::Framework { kind: PageInstance }` /
  `ReceiverType::Framework { kind: ReportInstance }`. Two new `ReceiverBuiltinKind`
  variants (`PageInstance` — 19 methods; `ReportInstance` — 36 methods) are added to
  the member-builtin catalog, sourced from `member_builtins.json` `"Page"` and
  `"ReportInstance"` arrays. Phase B's Framework arm dispatches via the catalog: a
  hit emits `builtin`; a miss emits `Unknown { FrameworkMethodNotInCatalog }` (an
  honest catalog gap, not a regression). Pure reclassification — `resolved` count
  unchanged. CDO `DocumentOutput/Cloud` after: `untracked-receiver::currpage` 319 → 0,
  `untracked-receiver::currreport` 15 → 0, builtin 4745 → 5079 (+334), unknown
  1427 → 1093 (−334), `realUnknownRate` 10.21% → 7.82% (−2.39 pp). Four new tests
  in `tests/l3cg_currpage_dispatch.rs`.

### Changed
- **Member-call resolution refactored to the ReceiverType lattice** (Phase A infer + Phase B
  dispatch) — `src/engine/l3/receiver_type.rs` (new) + `src/engine/l3/call_resolver.rs`. The
  deeply-nested string-keyed if/else ladder in `resolve_call_site`'s `PCallee::Member` arm
  (including the verbose surgical Record-table-procedure block) is replaced by a clean
  two-phase typed resolver: `infer_receiver_type(receiver, routine, symbols) -> ReceiverType`
  (a type lattice: Object / Interface / Enum / Record / RecordRef / FieldRef / KeyRef /
  Framework / Primitive / Unknown), then `dispatch(receiver_type, method, ctx) -> Vec<CallEdge>`
  (one match arm per variant). The surgical Record special-casing is ABSORBED into the Phase-B
  Record arm, preserving the catalog-builtin-FIRST ordering (a Record intrinsic like `SetRange`
  stays `builtin` even when the receiver's table is out-of-source). Strangler-Fig Phase A/B:
  wiring only — no new inference sources. Behavior-preserving (ZERO golden changes; CDO
  `DocumentOutput/Cloud` unchanged at resolved 7098 / builtin 4743 / unknown 1451 /
  realUnknownRate 10.39%). New direct unit tests on `infer_receiver_type` prove each lattice
  variant is inferred for a representative declared type.
- L3 taxonomy refactor: replaced the stringly-typed `CallEdge.dispatch_kind: String` /
  `resolution: String` (a TS-port hangover) with strict Rust enums `DispatchKind` /
  `Resolution` (`src/engine/l3/taxonomy.rs`). `Resolution::Unknown(UnknownReason)` folds
  the former `unknown_reason` side-field into the enum payload, so every `unknown` edge
  carries a compiler-enforced cause ("unattributed" is now structurally impossible);
  added `UnknownReason::DynamicObjectRunTarget` for the dynamic object-run edge.
  `enum.as_str()` reproduces the exact golden strings at the projection boundary — the
  refactor is internal-only and fully byte-stable (zero golden changes).
- L3 member-call resolution: a Record/framework receiver whose method is a recognized
  intrinsic now resolves to `builtin` (and leaves `unresolvedCallsites`). Non-intrinsic
  Record methods (real table procedures) remain `unknown`, pending Phase 3. Rebaselined
  the moved L3 call-graph + L3 coverage goldens (builtin reclassification only; no new
  resolved-to-routine edges) and updated the r2b `coverageMatrix.builtin` oracle
  (18→49). `KNOWN_DIVERGENCES.json` stays `[]`.
- **Test oracle: al-sem byte-parity RETIRED.** The engine is now Rust-owned; tests assert
  Rust-owned baselines + structural contracts, not equality vs the al-sem TS reference.
  The builtin reclassification correctly propagates downstream: r3a2 L4-summary phantom
  `unresolved-call` uncertainties removed (matrix 99→58); the `--require-dependencies`
  gate preflight reports coverage complete on builtin-only fixtures (exit 4→0, 28 rows;
  12 genuinely-degraded fixtures keep exit 4); and the `ws-txn-d48-pos` d48 finding's
  confidence rises `possible`→`likely` (a phantom `HttpClient.Send` uncertainty removed).
  See CLAUDE.md "Testing Philosophy & Goldens". Legacy al-sem-byte-parity tests
  (cli-b digest/fingerprint/prove/snapshot, r3a1, r4f_snapshot, gate_prsummary preflight
  oracles) are pending migration to Rust-owned baselines.

### Fixed
- Implicit-Rec argument bindings now flow `sourceTempState` (a pre-existing gap from the
  d22 implicit-Rec work): a trigger forwarding the implicit `Rec` to a record-mutating
  helper (`OnAfterInsert → Helper(Rec) → Rec.Modify()`) now resolves the cross-call
  inherited effect's temp-state to `Known(false)` instead of degrading to `Unknown`. The
  d22 work had rebaselined the d40 golden to expect `Known(false)` but never wired the
  temp-state through the binding, leaving r3a2/r4/gate red at the branch baseline.
- Rebaselined goldens after the iter-2 detector-gap fixes (G-13..G-19). Only **G-15**
  (d3 ignores field-writes/post-Init reads after a `Get`; d42 excludes PK-only fields)
  moved finding content; G-13/G-14/G-16/G-17/G-18/G-19 moved no in-repo goldens. The
  moves are all d3 suppressions/shrinks: (a) `ws-d8-commit-in-tx` — the d3 `rootCause`
  / `fixHint` field-set shrinks from `[last posting date, no., status posted]` to
  `[no.]` (the two written fields are excluded; the PK read `no.` survives), finding
  count unchanged; (b) `ws-txn-d46-pos` (if-not-`Get`-then-`Init`/`Insert` and
  `if Get then write` construct/upgrade patterns), `ws-txn-d47-pos-*` and
  `ws-txn-d49-pos-*` (write-after-`Get`: field `:= …; Modify()`), and
  `ws-rollup-multi-detector` (write-after-`FindSet`) — the d3 finding is now fully
  SUPPRESSED, dropping it from cli-a json/html/terminal/stats, gate SARIF/PR-summary,
  and the gate exit-code matrix (`--fail-on` info/low/medium for those default-slot
  fixtures now exits 0, not 1). The gate-suppress anti-degenerate witness
  (`ws-inline-suppress` `UnsuppressedD3`, which reads the Normal field `Name`) was
  CONFIRMED to survive G-15; its companion `SuppressedIo`/`WrongDirectiveIo` d3
  findings were write-after-`Get` and are now correctly suppressed, lowering the
  inline-suppress SARIF totals 7→5 (unsuppressed) and 6→4 (suppressed) while the d47
  suppression invariant (2→1) is unchanged. Extended the `REGEN_TEMP_GOLDENS` regen
  path to the cli-a stats and gate PR-summary/exit-code harnesses, and hardened the
  cli-a json/html/terminal/stats regen to ALWAYS write the in-repo vendored override
  (never al-sem) and only when the engine output differs from the resolved baseline,
  keeping the vendored set minimal. al-sem stays FROZEN; no L2/L3 ripple this iteration
  (the L2/L3rt differential is byte-identical); no symbol-reader/cache surface moved
  (`cli_c_cache` green) → no cache-version bump; `KNOWN_DIVERGENCES.json` stays `[]`.
- Rebaselined the in-repo differential goldens after the G-1..G-12 detector-gap fixes.
  Two content classes moved: (a) **G-4** d1 transitive-loop `rootCause` text now names
  the terminal routine ("… reaches <op> in Z, which has no loop of its own — the
  operation runs once per iteration of that loop.") on `ws-d1` (r4) and
  `ws-d1-multi-caller` (r4 / cli-a json+html+terminal / gate-sarif) — a field-level
  change to `rootCause` only; presence, severity, ids, rootCauseKeys, and fingerprints
  are byte-identical. (b) **G-12** d3 now suppresses the PK-only existence-check `Get`
  in `ws-inline-suppress`'s `UnsuppressedD3`; the gate-suppress anti-degenerate witness
  was preserved by editing that fixture so the routine reads a Normal field (`Name`)
  after the `Get`, yielding a genuine d3 finding — gate-suppress SARIF/PR-summary and
  the `ws-inline-suppress` L2 feature golden were rebaselined accordingly. Added
  `REGEN_TEMP_GOLDENS` regen branches to the gate-suppress and L2-features differential
  harnesses (mirroring the existing gate-sarif / cli-a / r4 / l3rt regen paths). No
  symbol-reader/cache surface moved (`cli_c_cache` green) → no cache-version bump;
  `KNOWN_DIVERGENCES.json` stays `[]`.

### Fixed
- Detector-audit class A + Singleton BUG-5 (docs/detector-audit.md):
  `d4-repeated-lookup-in-loop` fixed on two fronts. (1) **Temp gate** — a repeated
  identical lookup on a provably `temporary` record (`temp_state` Known(true)) is
  an in-memory read with no SQL round-trip to hoist and no longer fires (same
  `is_known_temp` gate as d1/d2/d33; new `tempRecord` skip stat).
  Suppression-direction exact: the same shape on a physical record still fires
  (control in `tests/gap_audit_d4.rs`). (2) **BUG-5 duplicate finding id** — the
  id `d4/{routine}/{loop}/{varLower}` omitted the literal lookup key, so two
  distinct keys each repeated 2+ times on the same (routine, loop, variable)
  produced colliding ids. The literal key is now appended to the id ONLY when a
  variable has multiple qualifying key groups, so single-key findings keep their
  pre-fix ids byte-identical (existing d4 goldens verified unmoved, r4
  differential green).
- Detector-audit classes A + C (docs/detector-audit.md): `d2-event-fanout-in-loop`
  no longer false-fires when an event subscriber's in-loop db ops are all
  structurally non-actionable. Three guards now mirror d1's terminal/op selection:
  (1) **Next-terminator (G-1)** — a subscriber's own `until <var>.Next() = 0`
  terminator is the loop's cursor advancement, not a db op; (2) **virtual/system
  table (G-6)** — a subscriber reading `AllObjWithCaption`/`Field`/`Integer`/… hits
  the platform's in-memory metadata store, not SQL; (3) **temporary record** — an op
  provably on a `Known(true)` temporary record does no physical-db work (mirrors
  d33's temp gate). The three filters are applied in `D2Policy::terminals_at` (so
  transitive callees are covered too), and the `any_db_subscriber` aggregation now
  keys off the supplementary walk yielding a Complete path to a SURVIVING db op — so
  a subscriber touching ONLY terminator/virtual/temp ops is no longer counted as a
  db subscriber. The `is_terminator_next` / `is_known_temp` helpers were promoted
  from d1.rs to `detectors/mod.rs` (`pub(crate)`) for reuse; d1 imports them
  unchanged. Suppression-direction exact: a REAL db op (e.g. `Modify` on a physical
  record) inside a subscriber loop still fires (control in
  `tests/gap_audit_d2_guards.rs`).
- Detector-audit class B (docs/detector-audit.md): d21/d37/d39 no longer false-fire
  on the implicit `Rec` inside table-LEVEL `OnInsert`/`OnModify`/`OnDelete`/`OnRename`
  triggers, where the AL platform loads `Rec` before the trigger body runs AND
  auto-persists it afterwards (`OnInsert`/`OnModify`/`OnRename` write `Rec` to the
  table; `OnDelete` deletes it, making "validate without persist" moot). The
  `is_platform_loaded_trigger_rec` gate's `Table`/`TableExtension` arm (previously
  field-level `OnValidate` only) now also recognizes those four table-level trigger
  names — covering d21 (read-without-load), d37 (validate-without-persist), and
  d11 which share the gate — and a new `is_auto_persist_trigger_rec` signal makes
  d39 (record-left-dirty-across-chain) skip a table-level trigger caller that
  forwards `Rec` by-var to a dirty helper (new `autoPersistTriggerRec` skip stat).
  Suppression-direction exact: trigger kind + Table/TableExtension object +
  receiver `Rec` only — the same ops in a non-trigger procedure or on a non-`Rec`
  record inside the trigger still fire (controls in
  `tests/gap_audit_b_table_triggers.rs`; G-9/G-14 page/field-trigger behavior
  unchanged).
- G-19 (docs/engine-gaps.md): d1/d3/d10 no longer fire on a keyword-less by-`var`
  `Record` parameter of a **`local`** procedure when its temporariness is
  CLOSED-WORLD PROVEN: the routine is `local` (AL language rule — callable only
  within its owning object), every same-object call site that could name it is
  resolved (no parse-incomplete sibling bodies, no unresolved or unclassifiable
  name-matching calls), it has at least one resolved caller, every caller edge is
  a binding-carrying kind (`direct`/`method`), and every caller's argument
  binding for that parameter is `Known(true)` temporary — directly or
  recursively through another closed-world-proven `local` forwarding parameter
  (cycles ground to NOT-proven). New `engine::l5::closed_world_temp` module
  computes the proven `(routineId, paramIndex)` set once in the detector
  context; the d3/d10 temp gates consult it next to the existing `Known(true)`
  gate, and d1's per-path resolver
  (`resolve_temp_along_path_closed_world`) resolves a proven PD frame to
  `Known(true)` — so the intra-callee shape downgrades to `info` exactly like
  any other proven-temp record (~12 CDO false positives: GetUpgradeData,
  MergePdfInBatches/ProcessMergeBatch Temp Blob, TempAut*). Suppression-
  direction safe — every uncertainty fails the proof and keeps firing:
  public/internal routines (open world), any physical/unknown caller argument,
  unresolved same-object name-matching calls, dynamic/interface/event edges,
  event subscribers and triggers (runtime-invoked), zero-caller dead locals
  (no vacuous proof), and RE-11 colliding routine ids. The open-world shapes'
  recommended SOURCE fix remains adding the `temporary` keyword to the
  parameter (contract-trust `Known(true)` — covered by a regression guard).
  Tests: `tests/gap_g19_temp_param.rs` (proof + 7 firing controls + keyword
  guard); `temp_state_path` / `temp_state_substitution` /
  `temp_state_param_forwarding` / `gap_g13_temp_gate` stay green.
- G-18 (docs/engine-gaps.md): `d1-db-op-in-loop` no longer attributes a loop to an
  op when the loop is on a SIBLING call path, not on the actual path to the op.
  Root cause: the internal routine id (`compute_routine_id`) carries no member
  discriminator, so two same-name same-signature triggers in one object (e.g. two
  page actions, each `trigger OnAction()`) collide on the id — and with it every
  derived call-site id (`{rid}/cs{n}`). The combined graph files BOTH bodies'
  edges under the one shared `from` key, and d1's root-edge lookup (by callsite id
  alone) could pick the SIBLING action's edge for the LOOPING action's in-loop
  call site — walking a straight-line chain the loop is not on (the CDO batch-7
  `eDocumentsConfigExists` IsEmpty ×2 false positives, loop mis-attributed from a
  separate `RunReport`-style looping action). d1's root-edge match now also
  requires the edge's TARGET routine to carry the call site's own callee name
  (`edge_target_matches_callsite_callee`): the resolver is name-keyed, so a
  genuinely-own `direct`/`method` edge always matches — the guard only ever
  filters cross-body edges under a colliding id and can never suppress a genuine
  transitive finding (un-nameable object-run/unknown callees and out-of-source
  targets are accepted unchanged; implicit-trigger edges never reach the guard —
  their callsite ref is an op id). A real in-loop chain THROUGH a colliding
  trigger and the vanilla transitive shape both keep firing at unchanged severity
  (`tests/gap_g18_transitive_loop.rs`); `gap_g1`/`gap_g4` stay green. The
  underlying routine-id collision itself (which also conflates `routine_by_id` /
  `call_site_by_id` views for colliding triggers) is documented in
  docs/engine-gaps.md G-18 as residual follow-up.
- G-17 (docs/engine-gaps.md): `d33-unfiltered-bulk-write` no longer fires when the
  filter was provably applied by (a) an in-source helper defined ON the receiver's
  own TABLE — the real-world G-3 miss: `LineReport.SetEMailTemplateLineFilter(Rec);
  LineReport.DeleteAll();` passes the filter-VALUE source by value while the helper
  filters its implicit self record (bare `SetRange(...)` in a table method), a shape
  G-3's by-`var`-argument summary could never match because the call resolver's
  `parse_object_type_ref` has no `Record` keyword, so record-receiver member calls
  never resolve to table procedures (the G-3 root cause). The G-3 gate
  (`record_filtered_by_call_before` in `src/engine/l5/detectors/mod.rs`) now adds a
  receiver-method tier that joins receiver-var `table_id` → in-source table
  procedure by name (ALL same-name candidates must net-filter the implicit self —
  last `SetRange`/`SetFilter`/`Reset` event on the self, as bare calls,
  `Rec.`-member calls, or `Rec` record ops, must be a filter); and (b) the page
  builtin `CurrPage.SetSelectionFilter(<var>)` (matched structurally: a member call
  to `SetSelectionFilter` whose bound argument is the bulk-op record — the platform
  copies the page's row selection onto it as filters). Suppression-direction safe:
  no-filter, non-filtering receiver method, receiver method whose net effect is
  filter-then-`Reset`, and `SetSelectionFilter` on a DIFFERENT record all keep
  firing (`tests/gap_g17_d33_filters.rs`); `tests/gap_g3_interproc_filter.rs` stays
  green. TableExtension-defined helpers and dependency-table helpers stay
  unrecognized (conservative; the ABI side is G-17's deferred lower-priority part).
- G-16 (docs/engine-gaps.md): `d11-modify-without-get` / `d21-read-without-load` no
  longer fire "never loaded" when the record provably was. Two extensions of G-10,
  both suppression-direction safe: (a) the callee-load summary
  (`record_loaded_by_call_before` in `src/engine/l5/detectors/mod.rs`) now follows a
  BOUNDED multi-hop wrapper chain (`MAX_LOAD_WRAPPER_HOPS = 3` callee hops) — every
  hop is the same resolved-binding by-`var` join as G-10, so
  `FindTemplate -> FindTemplateWithReportID -> FindSet`, forwarded boolean facade
  loaders, and `GetBySystemId` inside a wrapper now count, while a load 4+ hops down,
  an unresolved callee, a by-value binding, or a chain that only filters all keep
  firing (Get-or-Insert facades like `InsertIfNotExists` were already covered at one
  hop since `Init`/`Insert` are recognized load ops). (b) NEW record-assign-as-load
  gate `record_loaded_by_assignment_before`: a whole-record assignment
  `RecB := RecA` strictly before the op loads `RecB` when `RecA` is provably loaded
  AT the assignment point — a recognized load op / loading call before it, the
  platform-loaded trigger `Rec` (G-9), a parameter record (the detectors' own
  caller-loaded skip), or a further assignment from a loaded var (chain bounded at
  `MAX_ASSIGN_CHAIN_DEPTH = 3` links). Backed by a new internal-only
  `PVarAssignment.rhs_identifier` (serde-skipped like G-1's `in_until_condition`,
  excluded from `PartialEq` — L2 feature goldens stay byte-identical) that is set
  ONLY when both assignment sides are bare identifiers, so field writes and
  expression RHS never suppress. Controls in `tests/gap_g16_deep_wrappers.rs` prove
  no-load, deep-non-loading-chain, beyond-bound-load, assign-from-unloaded,
  assign-after-op, and RHS-loaded-after-assignment all still fire;
  `tests/gap_g10_load_wrappers.rs` stays green.
- G-15 (docs/engine-gaps.md): `d3-missing-setloadfields` no longer fires when the fields
  touched after a retrieval are only WRITTEN, and `d42-cross-call-wrong-setloadfields`
  no longer counts PRIMARY-KEY fields as must-be-loaded. Three exact sub-class
  suppressions, everything else keeps firing: (a) a field access whose source position
  AND member name match a recorded assignment LHS (`PVarAssignment` is anchored at the
  statement start, which IS the LHS member expression's start) is a WRITE target —
  writes need no SetLoadFields, so they no longer count toward d3's
  "accessed-without-load" witness (RHS reads sit at different positions and keep
  counting); (b) an intervening `Init()` record op or `Clear(<var>)` bare call between
  the retrieval and the access closes d3's access window (new `WINDOW_CLOSING_OPS` —
  the access reads the re-initialised buffer, not the loaded row; `deriveLoadStates`'s
  `INVALIDATING_OPS` is unchanged since `Init` does not clear the SetLoadFields
  selection); (c) d42 now drops the callee parameter table's PK (first key) fields from
  `requiredLoadedFieldsAtEntry` — the PK is always loaded regardless of SetLoadFields —
  reusing G-12's d3 exclusion via the new shared `primary_key_field_names_lc` +
  `normalize_load_field_arg` helpers in `src/engine/l5/detectors/mod.rs` (new `pkOnly`
  skip counter). Genuine reads of non-PK normal fields still fire (controls in
  `tests/gap_g15_d3_d42_writes.rs`; `tests/gap_g12_d3_refinements.rs` stays green).
- G-14 (docs/engine-gaps.md): `d11-modify-without-get`, `d21-read-without-load`, and
  `d37-validate-without-persist` no longer fire on the implicit `Rec` inside page field
  `OnLookup` / `OnAssistEdit` triggers — the G-9 trigger set
  (`PAGE_TRIGGERS_REC_LOADED` in `src/engine/l5/detectors/mod.rs`) missed the two
  field-level lookup triggers even though the AL platform loads `Rec` before they run
  and the page framework persists a `Validate` performed inside `OnLookup`. The gate
  stays exact and structural (trigger kind + Page/PageExtension + receiver `Rec`);
  non-trigger procedures and non-`Rec` receivers keep firing (controls in
  `tests/gap_g14_onlookup_triggers.rs`). No golden moved.
- G-13 (docs/engine-gaps.md): `d10-self-modifying-loop` and `d39-record-left-dirty-across-chain`
  no longer fire on `Known(true)` TEMPORARY records — they were never added to the temp-state
  epoch's gate set (d1/d3/d33/d36/d37/d40 were). d10 now skips a mutating op on the iterating
  record when `op.temp_state` is Known(true) (same gate as d33): an in-memory cursor self-modify
  is safe — cursor corruption only applies to physical SQL cursors. d39 now skips a forwarded
  binding when `binding.source_temp_state` is Known(true) (same gate as d40): a temporary record
  left Validate-dirty across a helper chain has no SQL consequence. Both gates are exact-match
  on Known(true) — physical and Unknown records keep firing (suppression-direction safe; proven
  by controls in `tests/gap_g13_temp_gate.rs`). Both detectors gain a `tempRecord` skip counter.
- G-8 (docs/engine-gaps.md): a codeunit-global `temporary` record FORWARDED by-var into a
  helper (e.g. `TempErrors: Record "Error Message" temporary;` passed to a local
  `LogError(var Errors: Record ...)` that does the db op) no longer resolves "temp state
  uncertain". Root cause: the L2 argument-binding builder only matches the routine's OWN
  params/locals, so an arg naming an object-global record var was emitted
  `sourceKind: "unknown"` with NO `sourceTempState` — both the L4 PD substitution
  (`substitute_pd_temp_state`) and the L5 per-path resolver (`resolve_temp_along_path`)
  collapse a missing binding source to `Unknown`, so the helper's PD op stayed
  "uncertain" even though the global carries the exact structural `temporary` keyword.
  Fix (`src/engine/l3/l3_workspace.rs`, inside the existing RV-8 relabel block, AFTER the
  Task-3 global promotion): backfill an `"unknown"` binding whose arg text is a BARE
  identifier matching a promoted-global record var — and whose innermost declaration IS
  that global (a same-named scalar param/local shadows it → skipped, conservative) — with
  `sourceKind: "global"`, the promoted per-routine record-var id, and the global's own
  `tempState` (Known(true) only ever from the `temporary`-keyword signal Task 3 captured;
  a NON-temp global backfills Known(false) and keeps firing). Direct ops on globals
  (Task-3 promotion), keyword-temp by-var params (Task 8 / RV-3 contract-trust), and the
  keyword-less by-var PD-at-path-root → Unknown behaviour were verified CORRECT and are
  regression-guarded. Tests: `tests/gap_g8_residual_temp.rs` (forwarded temp global →
  info, forwarded non-temp global keeps firing, plus the Case A/B ground-truth guards).
  No in-repo golden moved (no golden fixture forwards an object-global record var).

### Changed
- G-7 (docs/engine-gaps.md): `d1-db-op-in-loop` findings whose EVERY path root routine is
  provably dead are now DOWN-CONFIDENCED — confidence drops one notch (likely → possible)
  and the rootCause gains "(looping routine appears unreachable from any entry point; see
  d14-dead-routine)" (CDO triage batch 4 — `UpgradeOutputProfileOnDocsWorker`, whose only
  caller is commented out). Deliberately NOT suppression: d14's dead-determination has its
  own open-world false positives (the engine is source-only — reflection-style invocation,
  unmodeled dispatch), so the finding KEEPS FIRING at the same severity, id, rootCauseKey,
  and fingerprint (the fingerprint hashes the rootCauseKey, not the rootCause text or
  confidence — suppression baselines are unaffected). The dead signal is d14's EXACT
  emission criteria, factored into the shared `provably_dead_routine_ids` /
  `classify_routine` (`src/engine/l5/detectors/d14.rs` — forward-BFS unreachable from the
  entry-point closure + `local`/app-scoped-`internal` access + not a Test object + not a
  property-expression host + not itself a root); d14's own output and stats are
  byte-unchanged by the refactor. The check runs POST-merge across ALL merged paths
  (canonical + additionalPaths): any live — or merely unprovable (public, Test object,
  page-hosted) — path root keeps full confidence. New d1 stats bucket
  `downConfidencedDeadRoutine`. d1 only for now (the gap's evidence is d1-only; other
  detectors can adopt the shared helper if triage shows volume). Covered by
  `tests/gap_g7_dead_routine.rs` (down-confidence + firing/severity preservation + live /
  public / mixed-live-and-dead controls). Moves d1 confidence/rootCause text and the d1
  stats shape in r4/cli-a/gate goldens only for dead-rooted fixtures; rebaseline deferred
  to the consolidated gap-fix rebaseline task.
- G-4 (docs/engine-gaps.md): `d1-db-op-in-loop` PURE-TRANSITIVE findings — the terminal
  op's own routine has NO loop around the op; the loop lives purely in an ancestor — now
  say so explicitly. The rootCause names the terminal routine and attributes the loop to
  the ancestor: `"A loop in X reaches <Op> on <Table> in Z, which has no loop of its own —
  the operation runs once per iteration of that loop."` (previously the terminal routine
  was never named, so the text read as if the op's own routine looped — CDO triage
  batches 7, 10). WORDING ONLY, deliberately NOT suppression: these findings are
  genuinely real (the op runs once per ancestor iteration — real SQL cost), so presence,
  severity, confidence, ids, rootCauseKeys, and fingerprints are all unchanged; a direct
  in-loop op and a transitive terminal op sitting inside the CALLEE's own loop keep the
  original wording byte-identical. The optional confidence-notch lowering was skipped
  (wording-only, per the gap's conservative scope). Covered by
  `tests/gap_g4_transitive_wording.rs` (new wording + firing/severity preservation +
  both unchanged-wording controls). Moves the d1 rootCause TEXT in r4/cli-a/gate-sarif
  goldens for transitive fixtures (`ws-d1`, `ws-d1-multi-caller`); rebaseline deferred to
  the consolidated gap-fix rebaseline task (field-level diff confirms only `rootCause`
  diverges).

### Fixed
- G-5 (docs/engine-gaps.md): findings no longer render the WRONG table name in their
  rootCause when a `tableextension`'s OWN object number collides with a real table's
  number in the same app (CDO triage batches 2, 3 — ops on `MergeTableTopBottom` /
  `HtmlTableStyle` / `HtmlTableStyleLine` reported as `CDOReturnShipmentHeader` /
  `CDOPurchaseReceiptHeader` / `CDOJobExt`, which are tableextension NAMES). Root cause:
  a `tableextension` declaration is indexed as an `L3Table` stub whose internal id reuses
  the EXTENSION's object number (`${appGuid}/table/${extNumber}` — kept so
  `merge_extension_fields` can find the extension's fields), so it COLLIDES with a real
  table sharing that number and clobbered it in every LAST-wins id lookup
  (`describe_table` tier 1 then rendered the extension's name). Fix: new
  `L3Table::is_extension_stub` marker + REAL-over-stub collision preference in every
  table lookup map — `SymbolTable` (`tables_by_name`/`tables_by_id`), the shared
  `table_by_id_preferring_real` helper consumed by `DetectorContext::table_by_id` (both
  source-only and cross-app builds), the HTML formatter's table-label map, and the policy
  engine's `tables_by_id`. Within the same kind (real/real, stub/stub) LAST-wins is
  preserved (al-sem parity); the `merge_extension_fields` algorithm itself is untouched
  (stays in lockstep with its projected twin). Name-correctness only: finding presence,
  severity, ids, and fingerprints are unchanged (the op's `table_id` STRING is identical —
  only the rendered name was wrong). Covered by `tests/gap_g5_wrong_table_name.rs`
  (collision repro in both assembly orders + sequential/transitive multi-subloop
  regression guards). No in-repo golden moved; the real-app (CDO) rebaseline remains with
  the consolidated gap-fix rebaseline task.
- G-3 (docs/engine-gaps.md): `d33-unfiltered-bulk-write` no longer fires on a
  `DeleteAll`/`ModifyAll` whose receiver was provably filtered by a helper procedure call
  earlier in the routine (CDO triage batches 9, 10 — `SetTemplateFilter(Rec)`,
  `SetMergeFieldFilter(Rec)`-style helpers, ~5 FPs). Implemented as
  `record_filtered_by_call_before` (`src/engine/l5/detectors/mod.rs`), the filter analog of
  G-10's load gate, consulted by d33 after its intraprocedural `was_filtered_before` scan.
  It REUSES the G-10 one-hop callee-summary join — extracted into the shared
  `callee_applies_op_to_by_var_arg` helper (resolve the callsite's callee via
  `resolved_call_edge_by_callsite`, join `argument_bindings` with
  `upgraded_bindings_by_callsite` requiring `binding_resolution == "resolved"` +
  `callee_parameter_is_var`, then inspect the callee's `record_operations` on the by-var
  parameter) — with a filter predicate: the callee's NET effect on the parameter must be
  filtered, i.e. its last `SetRange`/`SetFilter`/`Reset` op (by source position) on that
  parameter is a filter (`RECORD_FILTER_OPS` — the exact set d33 applies intraprocedurally,
  now shared), not a `Reset`. A caller-side `Reset` between the helper call and the bulk op
  also voids that call (mirrors `was_filtered_before`'s Reset semantics). One hop only;
  suppression-direction safe: no filter call, a non-filtering callee, a by-value binding,
  an unresolved callee, a filter call AFTER the bulk write, a callee that filters then
  Resets, and a caller-side Reset after the helper all keep firing. Covered by
  `tests/gap_g3_interproc_filter.rs` (helper-SetRange + helper-SetFilter suppressions; six
  controls). No in-repo golden moved by this change (full `cargo test` divergence-checked);
  the real-app (CDO) rebaseline remains with the consolidated gap-fix rebaseline task.
- G-10 (docs/engine-gaps.md): `d11-modify-without-get` / `d21-read-without-load` no longer
  fire when the record WAS loaded by a call that isn't a literal `Get`/`Find` record op
  (CDO triage batches 1, 10, 11, 12 — `GetBySystemId` ×4, `FindTemplate`-style wrappers,
  `InsertIfNotExists`, var-out facade loaders). Two structural tiers, both implemented in
  the shared `record_loaded_by_call_before` gate (`src/engine/l5/detectors/mod.rs`),
  consulted by d11/d21 after their intraprocedural `loaded_before` scan: (1) **platform
  built-in loaders** — a member call `<var>.GetBySystemId(...)` strictly before the
  mutating/reading op counts as a load (exact-name allowlist `PLATFORM_LOADER_METHODS`,
  case-insensitive, receiver must match the record variable exactly; `GetBySystemId` is
  not in the L2 record-op map so it surfaces as a call site, invisible to the old scan);
  (2) **one-hop callee load summary** — when the record was passed as an argument whose
  binding RESOLVED to a by-`var` record parameter of a workspace callee
  (`resolved_call_edge_by_callsite` + `upgraded_bindings_by_callsite`, the same join
  d37/d39/d40 use), and that callee's own body performs a recognized load op
  (`RECORD_LOAD_OPS` — the exact set d11/d21 apply intraprocedurally, now shared) on that
  parameter, the record is loaded after the call. This covers custom `FindXxx`/`GetXxx`
  wrappers, `InsertIfNotExists` (Insert is a recognized load), and var-out facade loaders
  in one mechanism, and is the load analog of G-3's planned filter summary (one hop, callee
  body only, reusable pattern). Suppression-direction safe: an unresolved callee, a
  by-value binding (the callee loads its own copy), a different variable, a non-loading
  callee, or a call AFTER the op all keep firing. Covered by
  `tests/gap_g10_load_wrappers.rs` (GetBySystemId + by-var helper-load suppressions for
  both detectors; controls: no load, load after the op, load on a different record,
  filter-only callee, by-value callee load, unresolved callee — all still fire). No
  in-repo golden moved by this change (full `cargo test` divergence-checked); the
  real-app (CDO) rebaseline remains with the consolidated gap-fix rebaseline task.
- G-2 (docs/engine-gaps.md): runtime-implied tempness is now inferred from the exact
  `not IsTemporary → Error` structural guard, removing the dominant post-epoch temp-related
  FP class (CDO triage batches 1, 9, 11 — ~15 FPs: `CDO File` ops, `EmbedFiles`,
  `UpdateFromXml`, signature templates). Two sub-features, both AST shape matches (no
  string-sniffing, no dataflow): (1) **self-guarding temp table** — a table whose
  OnInsert/OnModify/OnDelete/OnRename trigger contains a TOP-LEVEL
  `if not Rec.IsTemporary[()] then Error(...)` guard is temporary BY RUNTIME CONTRACT
  (every instance errors otherwise), so `index_table` now sets `L3Table.is_temporary`
  exactly like `TableType = Temporary` and the existing table-level override upgrades all
  ops on it to `Known(true)`; (2) **entry-guard temp routine** — a routine whose FIRST
  executable statement is `if not <X>.IsTemporary[()] then Error(...)` where `<X>` is a
  record var/param (incl. promoted globals) or the implicit `Rec`/`xRec` proves `<X>`
  temporary for the whole body (the guard dominates it), captured at L3 assembly as
  `L3Routine.entry_temp_guard_receiver` and applied as a new override pass in
  `record_types.rs` (after var/op temp derivation, alongside the table-level override).
  The guard matcher (`is_temporary_error_guard` in `l3_workspace.rs`) accepts only the
  exact shape: an `if` with NO else whose condition is `not <recv>.IsTemporary[()]` (or
  `<recv>.IsTemporary[()] = false`) with a bare-identifier receiver and a zero-argument
  IsTemporary, and whose then-branch is an `Error(...)` call (directly or a
  `begin Error(...); end` block with exactly that one statement). Suppression-direction
  safe — both signals PROVE tempness (the code errors at runtime otherwise), upgrades are
  purely additive toward `Known(true)`; any deviation (guard not the first statement,
  nested/non-top-level table guard, non-negated condition, `exit` instead of `Error`)
  leaves the state untouched → detectors keep firing. Covered by
  `tests/gap_g2_runtime_temp.rs` (table-contract resolution + d1 downgrade, paren-less +
  OnDelete variants, entry-guard param resolution + d33 suppression on a guarded global;
  controls: plain table, non-negated trigger, unguarded routine, guard-not-first,
  exit-then-branch — all keep firing). No in-repo golden moved by this change (no fixture
  contains an IsTemporary guard); the real-app (CDO) rebaseline remains with the
  consolidated gap-fix rebaseline task.
- G-12 (docs/engine-gaps.md): `d3-missing-setloadfields` no longer fires on four clean FP
  sub-classes from the CDO triage (batches 1, 8, 10/12). The "unloaded fields accessed"
  computation now (1) excludes the table's PRIMARY-KEY fields (first key — `L3Table.keys[0]`
  member names; the PK is always loaded regardless of SetLoadFields), (2) excludes
  **FlowField** fields (`field_class == "FlowField"` — an uncovered FlowField read needs
  `CalcFields`, d22's domain, not `SetLoadFields`), and (3) consequently suppresses the
  existence-check shapes (`exit(Rec.Get(...))`, `if Rec.Get(...) then exit;` + Init/PK-write/
  Insert) where no normal field is read after the Get — the accessed set is empty, so there is
  no witness. (4) The missed pre-Get `SetLoadFields` was a quote-normalization gap, not an
  ordering gap: `derive_load_states` already walks ops in source order, but the L2 body walk
  records `SetLoadFields("Unit Price")` arguments with their quotes while field accesses are
  stored unquoted, so a quoted load argument never covered the later access — load-set
  arguments are now trimmed + outer-quote-stripped + lowercased (`normalize_load_field_arg`)
  for `SetLoadFields`/`AddLoadFields`. Suppression-direction safe: only PK / FlowField names
  resolved against the table model are excluded (unresolved names stay in the accessed set),
  a Get reading BOTH a PK and an uncovered normal field still fires (missing list names the
  normal field only), and quote normalization only ever ENLARGES coverage matching (fewer
  false "incomplete"s, never a new finding). Covered by `tests/gap_g12_d3_refinements.rs`
  (PK-only, FlowField-only, two existence-check shapes, quoted+plain pre-Get SetLoadFields
  suppressions + uncovered-read, PK+normal, FlowField+normal, incomplete-pre-Get controls
  that must keep firing). In-repo gate/r4 goldens with d3 findings may move only where a
  finding's premise no longer holds — the real-app (CDO) rebaseline remains with the
  consolidated gap-fix rebaseline task.
- G-6 (docs/engine-gaps.md): SQL-cost detectors no longer fire on ops targeting BC
  VIRTUAL/system tables (`AllObj`, `AllObjWithCaption`, `Field`, `Key`, `Object`,
  `Object Metadata`, `Table Metadata`, `Page Metadata`, `Codeunit Metadata`,
  `Report Metadata`, `Database Locks`, `Session`, `Active Session`, `Integer`, `Date`) —
  these have NO physical SQL backing (they read the platform's in-memory metadata store),
  so an in-loop read of one is never a SQL round-trip (CDO triage batch 5, 6 FPs:
  `AllObjWithCaption`/`Field` reads in loops flagged "type not loaded"). The suppression is
  a shared exact-name gate (`VIRTUAL_SYSTEM_TABLES` allowlist + `is_virtual_system_table` +
  `op_targets_virtual_system_table` in `src/engine/l5/detectors/mod.rs`, same pattern as
  G-9's `is_platform_loaded_trigger_rec`): the op's type did NOT resolve to a workspace
  table (a user table with a colliding name is physical → keeps firing) AND the record
  variable's DECLARED type name matches the allowlist exactly (case-insensitive). Consulted
  by `d1-db-op-in-loop` (direct in-loop branch — new `virtualTable` skip stat, present only
  when non-zero — AND `terminals_at`, so virtual ops no longer fire transitively from an
  ancestor loop) and `d4-repeated-lookup-in-loop` (candidate filter). `d3`/`d33` need no
  gate: they already bail on unresolved-table ops, and a virtual table never resolves in the
  source-only workspace. Suppression-direction safe: only the exact-name allowlist is
  skipped; a loaded physical table and a NOT-loaded table with any other name keep firing.
  Covered by `tests/gap_g6_virtual_tables.rs` (d1 direct + transitive suppression, d4
  suppression, loaded-physical / unloaded-non-virtual / repeated-normal-lookup controls).
  No in-repo golden moved — full `cargo test` is green (no fixture performs record ops on a
  virtual table); the real-app (CDO) rebaseline remains with the consolidated gap-fix
  rebaseline task.
- G-11 (docs/engine-gaps.md): `d20-unreachable-after-exit` no longer fires when the only
  thing after an unconditional `exit(...)`/`Error(...)`/`CurrReport.Quit` is comment or
  pragma trivia — `exit(0); // note` (trailing inline comment), an own-line comment after
  the exit, and the comment-trailed single-line / conditional-fall-through exit shapes from
  the CDO triage (~6 FPs, batches 4/7/11/12) all stop firing. Root cause: the L2
  unreachable-after-exit scan (`src/engine/l2/body_walk.rs`, code_block entry) collected
  `named_children` as "statements", and in the V2 grammar `comment` / `multiline_comment` /
  `pragma` nodes are named children of `code_block` — so a comment was flagged as the "next
  statement" after the exit. The scan now filters that trivia out, so d20 fires ONLY when
  the terminator is unconditional AND an actual executable statement follows it in the same
  block. The other two triaged shapes were already structurally correct in the Rust engine
  (a bare single-line `exit(expr)` body has no following sibling; a conditional
  `if … then exit(x)` sibling is an `if_statement`, which `unconditional_exit_kind` never
  classifies) — locked in by tests. Suppression-direction safe: a REAL statement after an
  unconditional exit still fires, including when a comment sits between the exit and the
  dead statement. Covered by `tests/gap_g11_d20_position.rs` (trailing/own-line comment,
  single-line body, conditional fall-through suppressions + unconditional-exit,
  unconditional-Error and comment-between controls that must keep firing). No in-repo
  golden moved — full `cargo test` is green (no fixture exercises a comment-after-exit
  shape); the real-app (CDO) rebaseline remains with the consolidated gap-fix rebaseline
  task.
- G-1 (docs/engine-gaps.md): `d1-db-op-in-loop` no longer fires on the `Next()` that IS the
  `until <var>.Next() = 0` terminator of the very loop being iterated — that `Next()` is the
  loop's own per-iteration cursor advancement (removing it breaks the loop), never an
  actionable db op (the single largest crit/high FP class in the CDO triage, ~15+ FPs). The
  suppression is an exact structural proof: the L2 body walk now marks a record op whose node
  sits inside the `condition` field of its NEAREST enclosing `repeat_statement`
  (`PRecordOperation.in_until_condition`, serde-skipped so every feature-level golden stays
  byte-identical; forwarded through `L3RecordOperation`), and d1 skips
  `op == "Next" && in_until_condition` in BOTH its direct in-loop branch and `terminals_at`
  (so a callee's own terminator no longer fires transitively from an ancestor loop either).
  Suppression-direction safe: only a proven terminator `Next` is skipped — a real db op in
  the loop body, a mid-body `Next()` advancing a DIFFERENT cursor, and the cursor-opening
  `FindSet` inside an outer loop all keep firing (no non-Next op is ever suppressed). Covered
  by `tests/gap_g1_next_terminator.rs` (terminator suppression — direct, nested-opener and
  transitive — plus in-body Modify and second-cursor Next controls). No in-repo golden moved:
  the direct terminator-Next was already absent from every fixture golden (the pre-existing
  pre-loop cursor-opener heuristic covered the simple `FindSet → repeat → until Next` shape)
  and no fixture exercises the transitive/nested-opener shapes; the real-app (CDO) rebaseline
  remains with the consolidated gap-fix rebaseline task. The L2 baseline-vector comparison
  (`tests/l2_vectors.rs`) compares the serialized contract surface only — `PRecordOperation`
  gained a manual `PartialEq` that excludes the serde-skipped internal flag.
- G-9 (docs/engine-gaps.md): `d11-modify-without-get`, `d21-read-without-load` and
  `d37-validate-without-persist` no longer fire on the implicit `Rec` inside page triggers
  (`OnValidate`, `OnAction`, `OnAfterGetRecord`, `OnDrillDown`, `OnAfterGetCurrRecord`) or
  table field `OnValidate` triggers — the AL platform has already loaded `Rec` before those
  triggers run, and a field `OnValidate` calling `Validate(...)` on a sibling field is normal
  field-chain validation whose persistence is the caller's job (the single largest medium/low
  FP class in the CDO triage, ~40+ FPs). The suppression is an exact structural gate
  (`is_platform_loaded_trigger_rec` in `src/engine/l5/detectors/mod.rs`): routine
  `kind == "trigger"` + owning object type Page/PageExtension (page trigger-name set) or
  Table/TableExtension (`OnValidate`) + op receiver `Rec` (case-insensitive); anything
  uncertain keeps firing (suppression-direction safe). Each detector reports the skip under
  a new `triggerRec` stats key (omitted when zero, so existing stats output is unchanged).
  Covered by `tests/gap_g9_trigger_rec.rs` (page-trigger + table-field-trigger suppression,
  plus non-trigger and non-Rec controls that must keep firing). No in-repo golden moved —
  no r4/cli/r3a fixture exercises trigger-Rec for these detectors.

### Added
- Metamorphic soundness oracle for the temp-state epoch (Task 14 / ts14 — RV-2, the
  mechanical guard for the whole epoch's suppression direction; `tests/temp_state_oracle.rs`).
  The oracle encodes the governing property: adding the `temporary` modifier to a record
  declaration can only make that record MORE temporary, so the analyzer's findings may only
  be REMOVED or DOWNGRADED under the edit — never ADDED, never UPGRADED — with ONE carve-out
  (RV-1): FlowField `CalcFields`/`SetAutoCalcFields` findings are INVARIANT (a temp record's
  FlowField still evaluates its CalcFormula against the physical flow targets, a real SQL
  round-trip, so they must keep firing at the same severity). For each of five standalone
  inline fixtures (DeleteAll buffer, Modify-in-loop, Blob CalcFields, FlowField CalcFields,
  and a Get/Modify physical-op control) it runs the FULL default detector set in-process
  (`assemble_and_resolve_default` + `run_detectors`) over the ORIGINAL source and over a
  mechanically `temporary`-edited copy (the edit appends ` temporary` to the targeted
  `Record "Name"` declaration, shifting no later anchor), then compares the two `Finding`
  sets by a stable `(detector, file, line, col)` key: suppression fixtures must show edited
  ⊆ original under "removed or downgraded" (and must actually soften); the FlowField fixture
  must be byte-identical (key + severity). A corpus-wide guard asserts no addition / no
  upgrade across every fixture. Purely additive (new test file, no `src` change, no golden
  movement); a red here is a genuine product-soundness signal, not a golden to refresh.
- RecordRef `GetTable` / `OpenTemporary` local-only `tempState` derivation (Task 12 / ts12,
  Component 4 / G6). The L3 record-type resolution pass now derives a `RecordRef` variable's
  `tempState` from two structurally deterministic call patterns — `RecRef.Open(no, true)`
  (OpenTemporary form → `Known(true)`), `RecRef.Open(no)` / `RecRef.Open(no, false)` (plain
  Open → `Known(false)`), and `RecRef.GetTable(SomeRec)` (inherits `SomeRec`'s resolved
  `tempState` from the routine's `record_variables`). CONSERVATIVE: derivation only fires
  when the routine has NO branching (`has_branching == false`) AND the call site is outside
  any loop (`loop_stack.is_empty()`). Anything uncertain (conditional, in-loop, unknown
  second arg for `Open`, unresolved source for `GetTable`) → `Unknown` (engine still fires;
  never wrongly `Known(true)`). OUT OF SCOPE by design: `Copy(..., ShareTable)` aliasing
  (cross-routine, speculative — documented non-goal). The pass is purely additive — it only
  sets temp on ops that were previously `Unknown`; the table-level and page-level overrides
  that run after it can still upgrade to `Known(true)` independently.

### Changed
- Vendored the rebaselined cli-a/cli-c goldens in-repo + restored the FROZEN al-sem
  archive (Task 16 / ts16 follow-up — the never-modify-al-sem rule). The cli-a html/json/
  terminal byte goldens and the cli-c cache fixtures had been regenerated in place inside the
  external (frozen) al-sem checkout; that violates the hard rule that al-sem is never modified.
  The 7 rebaselined files now live in-repo under `tests/cli-a-goldens/{html,json,terminal}/`
  and `tests/cli-c-goldens/cache/` (a self-contained 5-file fixture-cache + classification.json
  + dry-run.txt). The four harnesses (`cli_a_{json,terminal,html}_differential`,
  `cli_c_cache_differential`) gained a `resolve_golden`/local-dir resolver that prefers the
  in-repo override and falls back to the frozen al-sem path when no local override exists — so
  only the rebaselined fixtures read local; all ~unchanged cli-a goldens still read al-sem
  untouched. al-sem restored clean (0 modified files).
- Golden REBASELINE for the temp-state-tracking epoch + symbolReader cache bump 17→18
  (Task 16 / ts16). The temp-state epoch (Tasks 0–14) changed finding/projection CONTENT by
  design; the goldens are now Rust-OWNED baselines (the TS oracle is retired) and were
  REGENERATED from the current engine via a new env-gated (`REGEN_TEMP_GOLDENS`) regen path
  added to each differential harness (byte-parity suites write the engine output string;
  structural-JSON suites re-serialize the engine projection in the existing on-disk form).
  `KNOWN_DIVERGENCES.json` stays `[]` (divergences are NOT allowlisted — the diff was reviewed
  finding-by-finding). Suites moved: `r2a` L3 record-types (3 goldens — promoted object-global
  record vars now bind a tableId, `resolvedRecordVarTableIds` 228→232); `r2.5b-rt` cross-app
  (1 — `depBoundRecordVars` 2→6 from ABI/native dep-source promoted record vars); `r3a2`
  summary-core (11 — PD substitution flips inherited `tempState` parameter-dependent→known/
  unknown + `effectKey` tempfrag `p<i>`→`t`/`f`/`u`); `r3a3` cone-coverage (2 — `tempState`
  flips + `recordVariableId` now bound on previously-unbound ops); `r3a5` cross-app summary
  (1 — same flips + dep-routine `recordVariableId` bindings); `r3b` wrapped-parity (consumes the
  r3a5 golden); `r4` findings, `gate-sarif`, and `cli-a` html/json/terminal (the
  `ws-d1-multi-caller` d1 rootCause dropped "(temp state uncertain)" — now resolves physical via
  all callers; severity unchanged). The `cli-a-*` byte goldens + the `cli-c` cache fixtures were
  rebaselined and VENDORED in-repo (see the follow-up entry above) so the frozen al-sem archive
  stays unmodified. Relaxed the `r3a5_projection_is_byte_stable` `!contains("r0/")` sub-assertion (a
  too-strict heuristic the designed cross-app promotion legitimately invalidates — a promoted
  dep record var binds `recordVariableId: "r0/<hash>/rv/<name>"`, an internal id that
  canonically carries the `r0/` model-instance prefix); the determinism (a == b) and stable
  routine-id checks remain. The `symbolReader` cache version (`cache_prune.rs`) is bumped 17→18
  (the symbol-reader surface now carries promoted/ABI record vars with bound tableIds, so prior
  caches must invalidate); `cli_c_cache_differential` + its fixture cache updated to "18".
- d1 (`db-op-in-loop`) RV-1 CalcFields/FlowField gate (Task 11 / ts11 — the headline
  false-negative fix of the temp-state epoch). A `CalcFields`/`SetAutoCalcFields` on a
  record d1 resolved to TEMPORARY now downgrades to `info` ONLY when EVERY named field
  argument resolves (via the table model) to `field_class != "FlowField"` (a
  Blob/Normal field load on a temp record is genuinely in-memory). If ANY field arg is
  a FlowField — OR any field arg is UNRESOLVABLE (name not in the table, `table_id`
  None, table not indexed, or no capturable field args) — d1 KEEPS FIRING at normal
  severity with the honest note "(temporary record, but FlowField calculation queries
  the flow targets)". Rationale: a TEMPORARY record's FlowField is still computed by
  evaluating its CalcFormula against the (physical) flow-target tables — a real SQL
  round-trip, host tempness irrelevant. Previously the blanket temp downgrade wrongly
  suppressed temp FlowField CalcFields (a false negative). SOUNDNESS: the gate only
  ever PREVENTS a downgrade (keeps firing) when uncertain — it never newly suppresses a
  finding; the only behaviour change is temp FlowField CalcFields now fires (removes the
  false-negative). The CDO motivating case `Files.CalcFields("File Blob", …)` (Blob →
  in-memory) still downgrades correctly. Gate works for cross-app tables (`field_class`
  is modeled on both native `L3Field` and ABI `AbiField`).
- d1 (`db-op-in-loop`) now consumes the PATH-RESOLVED temp state instead of the
  terminal op's RAW `temp_state` (Task 10 / ts10, Component 3, RV-6 — the first real
  detector behaviour change of the temp-state epoch). For each finding, d1 calls
  `resolve_temp_along_path` over THAT finding's evidence path: resolved `Known(true)`
  → downgrade to `info` (existing suppression); resolved `Known(false)` → fires at
  normal severity with NO temp note (honest physical); resolved `Unknown` → "(temp
  state uncertain)" + normal severity (existing uncertain behaviour). A terminal op
  that is ALREADY `Known(_)`/`Unknown` (non-PD) resolves immediately with no stepping,
  so behaviour is UNCHANGED for it; only PD-terminal (by-var param) findings gain
  per-path precision — previously they fell to "(temp state uncertain)", now they
  resolve to a precise verdict per caller path.
- `resolve_temp_along_path` now enforces the L4 edge-kind ALLOWLIST (Task 10 / ts10,
  RV-6 soundness). It takes an `edge_kind_by_callsite` lookup (callsite id → resolved
  edge kind, derived from the combined graph d1 already holds) and, before stepping a
  hop, checks the kind is in `{direct, method, implicit-trigger}`; ANY other kind
  (`dynamic | interface | codeunit-run | report-run | page-run | event-dispatch`) or a
  callsite missing from the map STOPS the chase → `Unknown` (sound = fires). Without
  this guard a PD chased down a dynamic/interface/run hop with a `Known(true)`-sourced
  binding would resolve `Known(true)` where L4 returns `Unknown` — an unsound
  divergence that could SUPPRESS a real finding. Mirrors `substitute_pd_temp_state`.
- d1 merge-tie rule (Task 10 / ts10, RV-6). `merge_by_terminal` collapses every path
  sharing a terminal op into one finding; post path-resolution, two paths can DISAGREE
  on the temp-derived severity (caller-A path → info/temporary; caller-B path →
  normal/physical). The WORST severity now wins (deterministic, conservative — never
  let a temp path hide a physical path's finding) AND the temp note lists BOTH verdicts
  ("temp state varies by caller: physical via B; temporary via A", sorted). Reconciled
  before the merge so the canonical lift carries the worst severity + dual-verdict note.
- DESIGNED golden moves (deferred to Task 16 rebaseline): d1/r4 + downstream
  (cli-a json/html/terminal, gate SARIF) goldens move for multi-caller PD-terminal
  findings — temp-derived severity/note changes only (e.g. `ws-d1-multi-caller` drops
  its "(temp state uncertain)" note because all callers pass a physical record;
  severity unchanged). No non-PD finding moves; no non-temp severity changes.

### Added
- Shared per-PATH temp-state resolver `resolve_temp_along_path` (Task 9 / ts9,
  Component 3, RV-6) in `src/engine/l5/path_temp_resolve.rs`. A path-walker terminal
  db-op may carry `temp_state = ParameterDependent(i)` (depends on param `i` of the
  routine the op lives in); that symbolic index is only resolvable along a CONCRETE
  caller chain, so the SAME op reached from two different callers can resolve
  differently (per-finding truth: caller passing a temp local → `Known(true)`;
  caller passing a physical var → `Known(false)`). The helper starts from the
  terminal op's `TempStateKind`, then steps ONE frame toward the path ROOT per
  `ParameterDependent` level — using each hop's `callsite_id` to look up the parent
  routine's `argument_bindings` and applying the SAME substitution table as the L4
  per-callsite fold (`Some(Known(v))` → `Known(v)`; `Some(PD(j))` → `PD(j)` then chase
  `j` in the next frame up; `Some(Unknown)` / `None` / missing binding / missing
  callsite → `Unknown`). Still-PD at the path root (the op's tempness depends on an
  entry param with no caller in this path) → `Unknown`. The callee-param index RV-6
  asks the walker to expose per hop is DERIVED at resolve time from the L3 routine map
  (the same `ctx.routine_by_id` d1 builds) rather than added as a new serialized field
  — so NO walker/`EvidenceStep` struct changed and no R3a/trace/R4 golden moves.
  `WalkResult.path` orientation confirmed ROOT→TERMINAL. Sound by construction: only
  resolves to `Known(true)` when a concrete binding source on the path is itself
  `Known(true)`; all uncertainty → `Unknown` (fires). The helper is SHARED and not yet
  wired into any detector (d1 wiring is Task 10), so detector behaviour is unchanged.
- Param-source argument-binding resolution at the L4 PD substitution (Task 8 /
  ts8, RV-7 binding gap). When a caller FORWARDS its OWN record parameter as the
  argument (e.g. `procedure A(var Rec: Record X)` calls `Helper(Rec)`), the
  inherited effect's tempness depends on the CALLER's param, not a concrete var.
  A record-typed parameter is already present in the caller's L2
  `enclosing_record_variables`, so the forwarded-param arg's binding already
  carries `source_temp_state` = that caller param's own temp_state. The
  `substitute_pd_temp_state` PD arm (`summary_runner.rs`) now RE-SYMBOLIZES:
  `Some(ParameterDependent(j))` → `ParameterDependent(j)` (chaining the symbolic
  dependency UPWARD to the caller's own param index) instead of collapsing to
  `Unknown`. A forwarded `temporary`-keyword param still yields `Known(true)`,
  a by-value param `Known(false)`, and a genuinely-unknown / nameless source
  `Unknown`. Sound by construction: re-symbolizing PD→PD only PROPAGATES a
  symbolic dependency — it never invents `Known(true)`; a PD chasing itself
  around a recursive cycle stays PD (monotone) and the JACOBI fixed point
  converges because the effect_key includes the PD index, keeping the state
  space finite (verified: self-recursion + 2-cycle forwarding fixtures converge,
  no `MAX_FIXED_POINT_ITERATIONS` regression).
- Per-callsite substitution of `ParameterDependent` temp states at L4 effect
  inheritance (Task 7 / ts7, G5, RV-7) — when a caller folds in a callee
  `DbEffect` whose `temp_state` is `ParameterDependent(i)`, the CALLEE-frame index
  `i` (meaningless in the caller's frame) is now RESOLVED per-callsite through the
  caller's argument binding for callee param `i`, instead of being copied
  verbatim. In `summary_runner::compose_routine` the db-effects fold now branches
  on the callee effect's temp_state: a `ParameterDependent(i)` effect is rewritten
  via the new `substitute_pd_temp_state` helper and re-keyed with `effect_key_of`
  before insertion; non-PD (`Known`/`Unknown`) effects fold unchanged as before.
  Substitution table over `binding.source_temp_state`: `Some(Known(true))` →
  `Known(true)`, `Some(Known(false))` → `Known(false)`, `Some(Unknown)` /
  `Some(PD(_))` → `Unknown`, and `None` (the caller's-own-param-source / RV-7
  binding gap, resolved properly in Task 8) → `Unknown`. Event-dispatch edges (no
  `callsite_id`) and edge kinds with no modeled binding semantics
  (`interface | codeunit-run | report-run | page-run | dynamic`) → `Unknown`;
  only `direct | method | implicit-trigger` carry usable bindings.
  Sound by construction: substitution only NARROWS symbolic → binding-derived, all
  uncertainty becomes `Unknown` (fires), and `Known(true)` is produced ONLY from a
  binding source that is itself `Known(true)` — suppression stays gated on
  `Known(true)`. Re-keying naturally dedupes by `(op, tableId, operationId,
  tempfrag)`: identical substitution results merge while divergent "mixed caller"
  results stay DISTINCT (e.g. one caller passing a temporary local and one passing
  a physical local to the same callee op yield two distinct inherited effects,
  `Known(true)` and `Known(false)`). The per-op resolved-state space is finite, so
  the JACOBI fixed point stays bounded (no `MAX_FIXED_POINT_ITERATIONS` regression).

### Changed
- Scope-honest argument-binding `sourceKind` (Task 8 / ts8, RV-8). The L2 binding
  builder labels any non-parameter record-var arg `"local"` because object globals
  are only PROMOTED into a routine's `record_variables` later, at L3. After
  promotion runs (`l3_workspace.rs`), a binding whose source matches a PROMOTED
  GLOBAL record var (`scope == Some("global")`) is now RELABELED from `"local"` to
  `"global"`, removing the diagnostic mislabel. Only `"local"` bindings are
  eligible — `"parameter"` / `"implicit-rec"` / `"expression"` are untouched.
  Behavior-preserving: `d39`'s persistable-source allowlist now accepts `"global"`
  alongside `"local"` (a promoted global is a real caller var, persistable exactly
  like a local; the persist-after check matches by name regardless of scope), and
  `static_arg`'s named-source allowlist already accepted `"global"`. No detector's
  outcome changes for the global case.
- R3a-2 structural oracle `every_inherited_effect_traces_to_a_callee_effect` and
  the via-precedence oracle `merged_via_is_the_max_over_contributing_sources`
  (`tests/r3a2_oracles.rs`) now match inherited effects to their callee source via
  the substitution-aware `callee_key_sources_inherited` relation: a callee
  `parameter-dependent` effect (tempfrag `p<i>`) is a valid source for an inherited
  effect whose tempfrag was SUBSTITUTED (the invariant `op|tableId|operationId`
  prefix matches; only the tempfrag changed). Without this, Task 7's per-callsite
  re-keying would trip the old byte-equality invariant for PD-touching SCCs.

- ABI (dependency) temp capture + net-new per-param record-var temp-state modeling
  (Task 6 / ts6, G7, RV-4) — brings the cross-app `.app` symbol path to native+ABI
  shape parity so a detector behaves identically whether a record flows through a
  workspace routine or a dependency routine:
  - `parse_symbol_reference` (`symbol_reference.rs`) now READS the temp markers it
    previously ignored: `AbiParameter.is_temporary` from the param
    `TypeDefinition.Temporary == true`, and `AbiTable.is_temporary` from the
    table-level property `{"Name":"TableType","Value":"Temporary"}` (exact
    case-insensitive value match via the new `raw_table_is_temporary` helper —
    mirrors how `parse_field` reads `fieldclass`; NO string-sniffing). Verified
    against a real Continia Core 29.0 SymbolReference.json. (A return-type
    `Temporary` marker is intentionally not modeled — `AbiRoutine` has no return-temp
    slot and no consumer; documented in-source.)
  - The ABI projection (`projection.rs`) forwards the markers: `ProjectedParameter`
    gains `is_temporary`, `ProjectedTable` gains `is_temporary`, both populated in
    `project_abi_to_index`.
  - The ABI→L3 projection (`cross_app_l3.rs`) now SYNTHESIZES `record_variables` for
    record-typed parameters of dep routines (previously `record_variables: []`),
    each with a base `temp_state` per the native rule (mirroring
    `l2::scope::extract_record_variables`): `Temporary` marker → `Known(true)`;
    by-var record param WITHOUT marker → `ParameterDependent(param_index)`;
    by-value record param → `Known(false)`. Each var carries `is_parameter = true`,
    `parameter_index`, `scope = Some("parameter")`, and a `table_name` derived from
    the param `type_text` (`record_types::record_table_name_of`). `dep_table_to_l3`
    now forwards `is_temporary`, so the merged-whole `resolve()` runs the SAME
    table-level override (Task 4) — a param typed on a `TableType = Temporary` dep
    table resolves to `Known(true)`. ONE precedence rule everywhere; falls to the
    base temp_state (no override) when the type text yields no table name (engine
    never throws). Suppression-safe: `Known(true)` only from exact markers, every
    uncertain case stays `PD`/`Unknown`.
- Page `SourceTableTemporary = true` capture + implicit `Rec`/`xRec` `Known(true)`
  override (Task 5 / ts5, G4, RV-8):
  - `project_file` (`l3_workspace.rs`) now reads the `SourceTableTemporary` property
    for Page and PageExtension objects via `read_object_property`, setting
    `L3Object.source_table_temporary = Some(true)` on an exact case-insensitive match
    against `"true"` (trim + lowercase); `Some(false)` when present but not `"true"`;
    `None` when absent. Never `.contains()` / string-sniffing; engine never throws.
    `L3Object` is not serialised into any gate surface, so this never moves a golden.
  - Page-level override pass added to `resolve_routine_record_types` (`record_types.rs`),
    running after the table-level override: when the current object's
    `source_table_temporary == Some(true)`, every record op whose
    `record_variable_name` (lowercased) is `rec` or `xrec` is force-upgraded to
    `Known(true)`. Both `rec` AND `xrec` (RV-8: xRec alongside Rec). Purely ADDITIVE
    toward `Known(true)` — never downgrades; `SourceTableTemporary = true` is a
    structural page property that cannot be carried by physical-source pages, so the
    upgrade is sound (suppression-safe direction).
- Native `TableType = Temporary` capture + table-level override precedence
  (Task 4 / ts4, G3, RV-8):
  - `index_table` (`l3_workspace.rs`) now reads the object-level `TableType`
    property via `read_object_property` and sets `L3Table.is_temporary = true`
    on an EXACT case-insensitive match (trim + lowercase + `== "temporary"`;
    never `.contains()` / string-sniffing). A missing/other value → `false`
    (conservative). This is the only allowed temp signal — a structural property
    read. `L3Table` is not serialised into any gate surface, so this never moves
    a golden.
  - Final override pass in `resolve_routine_record_types` (`record_types.rs`),
    running AFTER all `table_id` resolution (declared vars, ops, lexical fallback,
    implicit Rec/xRec pass-3): for every record op whose resolved table is
    `is_temporary`, force `temp_state = Known(true)`, and likewise for the matching
    record VARIABLE. The "one precedence rule everywhere" — table-level temp WINS
    over keyword / no-keyword / by-value / by-var / `ParameterDependent(i)`. So a
    by-var PARAM of a temp table reports `Known(true)`, not the L2-stamped `PD(i)`
    (RV-8). Purely ADDITIVE toward `Known(true)`: only upgrades, never downgrades a
    `Known(true)` and never forces `Known(false)`. Table lookup uses the existing
    `SymbolTable::table_by_id`.
  - `TableView::is_temporary()` test-facing accessor.
- `extract_object_global_record_vars` in `scope.rs` (Task 2 / ts2, G1): captures
  the `temporary_keyword` on object-level `var_section` record variable declarations,
  producing `PRecordVariable` with `temp_state = Known(true/false)` and
  `scope = Some("global")`.  Non-record vars are skipped; `preproc_conditional_var_block`
  and dataitem-scoped var sections are conservative gaps (fall to Unknown, RV-8).
  Not yet wired into L3 projection (Task 3).
- Additive model fields for temp-state tracking epoch (Task 1 / ts1):
  - `PRecordVariable.scope: Option<String>` (`"local"` | `"parameter"` |
    `"global"`; `skip_serializing_if` keeps goldens stable; populated by later tasks).
  - `L3RecordVariable.scope: Option<String>` — forwarded from L2; field-allowlisted
    L3 projection never reaches goldens.
  - `L3Table.is_temporary: bool` (default `false`) — additive; L3Table is not
    serialised into any gate surface.
  - `L3Object.source_table_temporary: Option<bool>` (default `None`) — additive;
    L3Object is not serialised into any gate surface.
  - `AbiTable.is_temporary: bool` (default `false`) — slot for ABI temp capture
    (populated by Task 6).
  - `AbiParameter.is_temporary: bool` (default `false`) — slot for parameter
    `temporary` modifier (populated by Task 6).
  - `RawTypeDef.temporary: Option<bool>` (`#[serde(rename = "Temporary")]`) —
    deserialises the `Temporary` field from `SymbolReference.json`; consumed by
    Task 6.

### Fixed
- Object-global record vars are now promoted into EACH routine's
  `record_variables` during L3 assembly (Task 3 / ts3, G2), and member-var record
  operations re-derive their `temp_state` from the promoted set — the root-cause
  fix for the CDO false-critical class (a codeunit member
  `Files: Record "CDO File" temporary;` was never seen by the L2 body walk, so
  `Files.DeleteAll()` carried `tempState = Unknown`, fired a false critical, and
  d1 stamped "(temp state uncertain)"). Promotion honors AL shadowing: a routine's
  own param/local of the same name shadows the global (innermost wins). Shadowed
  globals are NOT promoted, keeping `record_variables` NAME-UNIQUE — which
  preserves the documented pass-1 `var_index_by_name` last-wins invariant in
  `record_types.rs` (a name-duplicated list would let the global clobber the
  local). The op `temp_state` backfill lives in `record_types.rs` pass-2a: when an
  op matches its declaring record var, `op.temp_state` is copied from that var
  (alongside the existing `table_id` / `record_variable_id` derivation).
- `record_types.rs` pass 2b `variable_decl_by_name` map changed from last-wins
  (unconditional `insert`) to first-wins (`entry().or_insert()`) so that a
  procedure-local declaration always shadows an object-global with the same name
  — the correct AL innermost-scope rule and a prerequisite for the tempState
  backfill epoch (RV-5).

## [0.7.0] - 2026-05-06

### Added
- Anonymous, opt-out failure-diagnostics telemetry (Azure App Insights).
  - Captures resolution misses, parser errors, indexer issues, and handler outcomes.
  - All AL identifier names hashed with a per-installation 32-byte salt that stays local.
  - Three disable mechanisms: `DO_NOT_TRACK=1`, `--no-telemetry`, `~/.al-call-hierarchy/config.json` `telemetry.enabled=false`.
  - Off by default in debug, test, and CI builds.
  - LSP request `al-call-hierarchy/telemetryStatus` for runtime introspection.
  - Schema documented in `docs/telemetry.md`.
  - Fire-and-forget export: `BatchSpanProcessor` on a dedicated tokio current-thread runtime; HTTP calls are non-blocking, individual export failures are silently dropped, and LSP request threads are never affected by network state. 10s/5s reqwest timeouts cap any single HTTP call; shutdown is bounded to a 3s budget.

## [0.5.0] - 2026-03-22

### Changed
- **BREAKING: Migrated to tree-sitter-al V2 grammar** — all tree-sitter queries and parsing logic updated for the rewritten grammar
  - `procedure name:` and `trigger_declaration name:` now hold `(identifier)`/`(quoted_identifier)` directly (no `(name)`/`(trigger_name)` wrapper nodes)
  - `member_expression` field renamed from `property:` to `member:`
  - `parameter` field renamed from `parameter_name:` to `name:`
  - Individual `*_property` nodes replaced by unified `property` node with `name:` and `value:` fields
  - `preproc_split_codeunit_declaration` renamed to `preproc_split_declaration`
- **tree-sitter-al is now a git submodule** instead of an external sibling directory — clone with `--recurse-submodules`
- `build.rs` defaults to `tree-sitter-al` (submodule) instead of `../tree-sitter-al`

### Removed
- `field_access` query pattern — merged into `member_expression` with `quoted_identifier` as member
- `named_trigger` / `onrun_trigger` handling — unified into `trigger_declaration`
- `extract_trigger_name()` helper — no longer needed with V2 grammar
- `property_display_name()` helper — replaced by reading `property_name` field directly

### Fixed
- EventSubscriber detection now correctly handles V2 attribute-as-sibling model (attributes are siblings of procedures, not children)

## [0.2.0] - 2025-02-03

### Added
- **Event Subscriber Integration**: Event subscribers are now shown in the call hierarchy
  - Parses `[EventSubscriber]` attributes to extract publisher object and event name
  - Event subscribers appear as "callers" in `incomingCalls` for the subscribed events
  - Shows `[EventSubscriber]` tag in the call hierarchy detail

- **Code Lens Support**: Reference counts and quality metrics displayed above procedures
  - Shows "N references | complexity: X, lines: Y, params: Z" lens above each procedure/trigger definition
  - Displays cyclomatic complexity, line count, and parameter count for each procedure
  - Highlights procedures with 0 references as potential dead code
  - Click to navigate to the references (via `al-call-hierarchy.showReferences` command)

- **Unused Procedure Detection**: Diagnostics for procedures with no callers
  - Publishes `HINT` severity diagnostics for unused procedures
  - Excludes triggers and event subscribers (they're called implicitly)
  - Tagged with `UNNECESSARY` for IDE-specific rendering (strikethrough, etc.)

- **Code Quality Diagnostics**: Warnings for potential code quality issues
  - High fan-in warning: procedures called by more than 20 other procedures
  - Long method warning: procedures spanning more than 50 lines
  - Diagnostics published at `INFORMATION` severity

- **External .app dependency support**: The server now resolves calls to procedures defined in compiled .app packages
  - Automatically parses `app.json` to discover declared dependencies
  - Finds matching .app files in the `.alpackages` folder with version matching
  - Extracts procedure definitions from `SymbolReference.json` inside .app files
  - Shows "(from AppName)" in call hierarchy for resolved external calls
  - Supports all standard BC object types: Codeunits, Tables, Pages, Reports, etc.

### Changed
- **Memory optimization**: `ExternalSource.app_version` now uses interned `Symbol` instead of `String`, reducing memory usage when loading large .app dependencies (~50-100MB savings for BC base apps)

### New capabilities
- `textDocument/codeLens` - Returns reference counts for all procedures in a file
- Diagnostics publishing via `textDocument/publishDiagnostics`

### New modules
- `app_package.rs` - Parser for .app files (ZIP with 40-byte NAVX header)
- `dependencies.rs` - Dependency discovery and resolution from app.json

### Dependencies
- Added `zip` crate for .app file extraction
- Added `roxmltree` crate for NavxManifest.xml parsing
