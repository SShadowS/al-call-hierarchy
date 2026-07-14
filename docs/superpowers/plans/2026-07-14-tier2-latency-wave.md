# Tier-2 LSP Latency Wave + dep-node RSS Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Cut a real rung-1 keystroke-save from ~23 ms to ~2 ms (incremental
`decl_by_id`/`incoming` patch + rung-scoped diagnostics), take ~100–150 ms off
cold start and every north-star run (parallel per-file resolve), and reclaim
~49 MB steady-state RSS (drop the duplicated dep node Vecs, rebuild on rung-2).

**Architecture:** Four independent-gate changes to the LSP incremental pipeline
(`src/lsp/updater.rs`, `src/lsp/snapshot.rs`, `src/lsp/diagnostics.rs`,
`src/server.rs`) plus the whole-program resolver's Phase-1 loop
(`src/program/resolve/full.rs`). Every task carries its own byte-parity gate;
the shared anchor is the CDO north-star JSON SHA-256
`0a3b85bc832ff0a3e77acee118d203edbf62827dc37617c8d9315fe52d5cb7d0`.

**Tech Stack:** Rust, rayon, existing test harnesses (`tests/lsp_incremental_parity.rs`,
`benches/lsp_pipeline.rs`, `tests/perf_bounds.rs`).

**Requirements source:** `.superpowers/sdd/tier2-lsp/investigation.md` (measured
findings, designs, and the NO-GO/DEFER rationale for items C and F2 — read it
before starting any task).

## Global Constraints

- Branch: `feat/tier2-latency-wave` off `master` @ `4a3da07`.
- **North-star parity**: after EVERY code task, `aldump --program-call-graph-stats <CDO_WS>`
  JSON must hash to SHA-256 `0a3b85bc832ff0a3e77acee118d203edbf62827dc37617c8d9315fe52d5cb7d0`
  (`CDO_WS = u:\Git\DO.Support-SlowDOSetup\DocumentOutput\Cloud`). Command:
  `cargo run --release --bin aldump -- --program-call-graph-stats u:\Git\DO.Support-SlowDOSetup\DocumentOutput\Cloud > $env:TEMP\ns.json; (Get-FileHash $env:TEMP\ns.json -Algorithm SHA256).Hash`
  (verify against the CLAUDE.md-documented invocation shape first; if the dump
  includes a non-deterministic preamble, hash only the JSON payload the same way
  the investigation did).
- Full `cargo test` green at every commit; ZERO golden regenerations (a golden
  diff means the change is wrong, not the golden).
- `cargo clippy --all-targets --all-features` clean; format touched files with
  `rustfmt <file>` (NEVER `cargo fmt`).
- CHANGELOG.md entry per code task; commit trailer
  `Co-authored-by: Copilot <223556219+Copilot@users.noreply.github.com>`.
- Never `git add -A`; never stage `.panel/`, `demo-out/`, `finish-*.ps1`,
  `scripts/peak_rss.py`.
- Measured baselines to beat (CDO, best-of-7 unless noted): rung-1
  `apply_batch_scoped` 11.4 ms; `compute_all` 11.4 ms; cold start ~2.79 s;
  steady-state RSS ~630 MB.
- H-10 law amendment: `src/lsp/snapshot.rs:18-22` documents `incoming`/
  `decl_by_id` as "always rebuilt WHOLESALE" — Task 1 amends that contract; the
  doc comment MUST be rewritten to state the new rung-1-local patch rule and
  point at the parity gate that licenses it.

---

### Task 1: Rung-1 incremental `decl_by_id`/`incoming` patch + `EdgeRef.file: Arc<str>` (items B + F5)

**Files:**
- Modify: `src/lsp/snapshot.rs` (EdgeRef, build_incoming, push_edge_targets, H-10 doc at :18-22)
- Modify: `src/lsp/updater.rs` (`apply_rung1_core` ~:660, `Updater` struct state)
- Modify: `src/lsp/handlers.rs` + any other `EdgeRef.file` consumers (compile-driven)
- Test: `tests/lsp_incremental_parity.rs` (index-equality + cross-file-duplicate fixture)

**Interfaces:**
- Produces: `EdgeRef { file: Arc<str>, idx: u32 }`; `apply_rung1_core` returns, in
  addition to the snapshot, the per-save delta `Rung1Delta { files: Vec<String>,
  affected_ids: Vec<RoutineNodeId> }` (`affected_ids` = every id whose
  `incoming` Vec changed — the union of removed and added edge targets). Task 2
  consumes `Rung1Delta`. Store it next to the snapshot swap (see Task 2's
  consumption) — for THIS task it may be computed and returned but plumbed no
  further than `apply_batch_scoped`'s return value.

**Design (from the investigation — binding):**

1. `EdgeRef.file: String` → `Arc<str>`. In `push_edge_targets`, take
   `file: &Arc<str>` and `Arc::clone` it per EdgeRef instead of
   `file.to_string()`. `edges_by_file`'s key stays `String`; build one
   `Arc<str>` per file per build/patch call, up front. `EVENT_EDGES_KEY`
   becomes a shared static `Arc<str>` (e.g. `LazyLock<Arc<str>>`) or is built
   once per call — either is fine, but every consumer comparing
   `EdgeRef.file == EVENT_EDGES_KEY` must keep compiling (compare `&*ref.file`
   with the `&str` constant).
2. `decl_by_id` incremental patch, duplicate-safe. `decls_by_file` can hold
   cross-file duplicate `RoutineNodeId`s (the
   `dedup_routines_preserving_genuine_overloads` population), and today's
   wholesale `build_decl_by_id` iterates a HashMap — the duplicate winner is
   ALREADY nondeterministic across builds. The patch must preserve the
   invariant "every id in the union of `decls_by_file` is present, mapped to
   one of its declaring files' entries" — not any specific winner. Rule:
   - `Updater` gains `decl_multiplicity: HashMap<RoutineNodeId, u32>` (count of
     declaring FILES per id), rebuilt wholesale at rung-2/rung-3 alongside the
     wholesale `build_decl_by_id`, patched at rung-1.
   - Rung-1, per changed file F: for each id in OLD `decls_by_file[F]` not in
     the NEW decl list: decrement multiplicity; if 0, remove from map + count;
     if >0, the id survives in another file — if the current `decl_by_id[id]`
     entry has `virtual_path == F`, re-derive it by scanning `decls_by_file`
     for any surviving declaring file (O(workspace) but only on the rare
     duplicate-eviction path). For each id in the NEW list: insert/overwrite
     `decl_by_id[id]` with the new entry; increment multiplicity only if the
     id was not in the OLD list.
3. `incoming` incremental patch: clone `cur.incoming` (cheap once EdgeRefs are
   `Arc<str>`), then for changed file F: remove every `EdgeRef` with
   `&*e.file == F` from each target's Vec (targets derivable from
   `cur.edges_by_file[F]`'s routes — reuse `push_edge_targets`'s
   per-edge-dedup rule in reverse), then push the new file's edges via the same
   `push_edge_targets`. `publisher_fanout` and `event_edges` are untouched at
   rung 1 (Arc-forward `cur.publisher_fanout` — it is derived ONLY from
   `event_edges`, which rung 1 never changes; this replaces today's
   recompute-anyway, and the doc comment at updater.rs:708-716 must be updated
   to say so).
   - **Ordering caveat**: today's wholesale `build_incoming` iterates a HashMap,
     so per-target Vec order is ALREADY nondeterministic; every consumer sorts
     (`handlers.rs:172,196-197`). The parity test must therefore compare
     per-target Vecs as SORTED multisets, not raw order.
4. Rewrite the H-10 doc (snapshot.rs:18-22 + `build_decl_by_id`/`build_incoming`
   docs) to record the amended law: wholesale at rung-2/3, provably-F-local
   patch at rung-1, licensed by the parity gate below.

**Steps:**

- [ ] **Step 1: Write the failing parity tests.** In `tests/lsp_incremental_parity.rs`
  add: (a) `rung1_patched_indexes_match_wholesale_rebuild` — build a workspace,
  apply a rung-1 body edit via the existing harness, then assert
  `snap.decl_by_id` key-set equality + per-id "entry is one of the declaring
  files' decls" against a fresh `build_decl_by_id(&snap.decls_by_file)`, and
  `snap.incoming` equality against a fresh
  `build_incoming(&snap.edges_by_file, &snap.event_edges)` with per-target
  Vecs sorted (`(file, idx)` key) before comparison, plus `publisher_fanout`
  equality. (b) `rung1_cross_file_duplicate_routine_id_survives_edit` — a NEW
  fixture with two workspace files declaring the same genuine-overload
  `RoutineNodeId` (mirror the `dedup_routines_preserving_genuine_overloads`
  fixture shape); edit ONE file's body; assert the id is still present in
  `decl_by_id` and resolves to a surviving declaration; then REMOVE the decl
  from the edited file entirely (still a body-only edit is impossible here —
  removing a decl changes the def-surface fingerprint and escalates to rung 2;
  instead delete the OTHER duplicate scenario: assert multiplicity bookkeeping
  via a second body edit round-trip). Keep the test at what rung-1 can actually
  express — the duplicate-EVICTION path is only reachable when a decl
  disappears without a fingerprint change, which cannot happen (fingerprint
  covers decls); ASSERT that in a comment and test the decrement path via the
  rung-2 wholesale rebuild instead (multiplicity map rebuilt — assert it
  matches a from-scratch count).
- [ ] **Step 2: Run the new tests — expect FAIL** (or compile error on the new
  helpers): `cargo test --test lsp_incremental_parity`.
- [ ] **Step 3: Implement** per the design above, compile-driven for every
  `EdgeRef.file` consumer.
- [ ] **Step 4: Full validation.** `cargo test` (all, zero goldens),
  clippy, north-star SHA check per Global Constraints.
- [ ] **Step 5: Measure.** Re-run the investigation's rung-1 probe methodology
  (best-of-7 `apply_batch_scoped` on CDO with a `FileSaved` on an unmodified
  file, `big_stack` — see investigation.md "Method") — record before (11.4 ms)
  / after (< 1 ms expected). Also re-run `cargo bench --bench lsp_pipeline`
  rung-1 rows and `cargo test --release --test perf_bounds`.
- [ ] **Step 6: Commit** `perf(lsp): incremental rung-1 decl_by_id/incoming patch + Arc<str> EdgeRef` — stage only the touched src/test files + CHANGELOG.md.

---

### Task 2: Rung-scoped diagnostics recompute (item D / F4)

**Files:**
- Modify: `src/lsp/diagnostics.rs` (add `compute_for_files`, keep `compute_all`)
- Modify: `src/lsp/updater.rs` (plumb `Rung1Delta` through the `on_swap` callback)
- Modify: `src/server.rs` (`publish_diagnostics_diff` ~:427, `on_swap` wiring ~:369)
- Test: `src/lsp/diagnostics.rs` unit tests + a differential test

**Interfaces:**
- Consumes: Task 1's `Rung1Delta { files, affected_ids }`.
- Produces: `on_swap` callback signature becomes
  `Fn(&LspSnapshot, &SwapScope)` where
  `enum SwapScope { Full, Rung1(Rung1Delta) }` (rung-2/3 pass `Full`).
  `compute_for_files(snap, enc, cfg, files: &BTreeSet<String>) -> HashMap<String, Vec<Diagnostic>>`
  computes ONLY those virtual_paths' diagnostics (same shape as `compute_all`,
  restricted key set). `DiagnosticsState` gains
  `diff_partial(touched: HashMap<String, Vec<Diagnostic>>) -> Vec<(String, Vec<Diagnostic>)>`
  which diffs ONLY the supplied uris, leaving all other published state intact.

**Design:**
- On `SwapScope::Rung1`, the recompute set = the edited files ∪ every file
  containing a decl whose `effective_incoming_count` could have changed = the
  `virtual_path`s of `decl_by_id[id]` for each `affected_ids` id that resolves
  to a workspace decl. Only the unused-procedure rule is cross-file, and it
  depends on exactly `effective_incoming_count` (`incoming` + `publisher_fanout`);
  rung 1 never changes `publisher_fanout` (Task 1 Arc-forwards it), so
  `affected_ids` (incoming-delta targets) is a complete cover. State this
  argument in the code comment.
- `compute_all` stays for `Full` swaps and the initial publish; extract the
  shared per-file body into a helper both paths call so they can never drift.

**Steps:**

- [ ] **Step 1: Failing differential test.** In diagnostics.rs tests: build a
  fixture snapshot (reuse an existing fixture builder in the module's tests),
  apply a rung-1-shaped change (recompute one file via the updater harness or
  synthesize the delta), and assert
  `compute_for_files(..., cover_set)` merged over the previous full map ==
  `compute_all(...)` on the new snapshot, for a fixture where an
  unused-procedure diagnostic flips in a NON-edited file (file A calls B.Proc;
  edit A's body to delete the call; B.Proc becomes unused — the cross-file
  sharp edge).
- [ ] **Step 2: Run — expect FAIL/compile error.**
- [ ] **Step 3: Implement** `compute_for_files` + `diff_partial` + `SwapScope`
  plumbing (updater → server). Rung-2/3 and initial publish keep the exact
  current full path.
- [ ] **Step 4: Full validation.** `cargo test`, clippy, north-star SHA (should
  be trivially unaffected — no resolver change — but the gate is cheap).
- [ ] **Step 5: Measure.** Instrument a CDO rung-1 save end-to-end (rung-1 apply
  + diagnostics publish): expect ~23 ms → ~2 ms total. Record numbers.
- [ ] **Step 6: Commit** `perf(lsp): rung-scoped diagnostics recompute on rung-1 swaps`.

---

### Task 3: Parallel per-file resolve (item E / F7)

**Files:**
- Modify: `src/program/resolve/full.rs` (Phase-1 loop ~:788)
- Modify: `src/lsp/snapshot.rs` (`from_context`'s per-file loop) and
  `src/lsp/updater.rs` (`apply_rung2`'s per-file loop ~:487) if they share the
  same shape (they call `recompute_file` per file — parallelize identically).
- Test: existing `resolve_file_obligations_matches_full_run_per_file_and_in_concatenation`,
  `build_full_edges_match_resolve_full_program`, `build_full_is_deterministic_across_two_builds`.

**Design:**
- Each `resolve_file_obligations` call reads only `&graph`/`&index`/`&surface`/
  `&obj_node_map` (all `Sync` borrows) and returns its own `FileResolution` —
  embarrassingly parallel. Replace the serial `for pf in &unit.files` with
  `unit.files.par_iter().map(|pf| (pf, resolve_file_obligations(...))).collect::<Vec<_>>()`
  then fold the results IN THE ORIGINAL `unit.files` ORDER (indexed par_iter
  preserves order through collect — same pattern as the witness arc's Task 3).
  The post-loop accumulator inserts (`obligation_id_set` etc., full.rs:802+)
  happen in the sequential fold, byte-identical to today.
- **Stack depth**: resolve recursion overflows default worker stacks on real BC
  files (investigation "Method" note). Check how the existing rayon parse pool
  is configured (grep `big_stack` / `stack_size` in `src/`) and run the resolve
  par_iter on a pool with the same stack size (e.g. a shared
  `rayon::ThreadPoolBuilder::new().stack_size(32 * 1024 * 1024)` pool via
  `pool.install(|| ...)`) — do NOT use the global default pool blindly; verify
  by running the full CDO build in a test/probe.
- **Interner check**: confirm no `&mut` interner or other shared-mutable state
  is reached from `resolve_file_obligations` (the compiler enforces this via
  `Sync` bounds — a compile success plus the determinism tests is the proof).
- Apply the same pattern to `from_context`'s and `apply_rung2`'s
  `recompute_file` loops (same immutability argument; rung-2 drops ~150 ms
  proportionally).

**Steps:**

- [ ] **Step 1: Parallelize `resolve_full_program` Phase 1** as above; run
  `cargo test --test program_resolve_harness` (needs CDO_WS for full coverage;
  at minimum the in-repo fixtures) + the three named tests. Expect PASS.
- [ ] **Step 2: North-star SHA gate** (Global Constraints) — MUST be
  `0a3b85bc…`. Any drift = ordering bug; fix, never rebaseline.
- [ ] **Step 3: Parallelize `from_context` + `apply_rung2` loops**; `cargo test`
  full; clippy.
- [ ] **Step 4: Measure.** Cold start on CDO (docs §9/§10 methodology) before/
  after — expect ~100–150 ms off ~2.79 s; `aldump --program-call-graph-stats`
  wall time before/after. `cargo bench --bench lsp_pipeline` build_full rows +
  `cargo test --release --test perf_bounds`.
- [ ] **Step 5: Commit** `perf(resolve): parallelize per-file resolve loops (ordered collect)`.

---

### Task 4: Drop duplicated dep node Vecs; rung-2 rebuilds them (item A, drop-and-rebuild variant)

**Files:**
- Modify: `src/lsp/snapshot.rs` (`LspSnapshot.dep_layer` field — replace with what rung-2 actually needs)
- Modify: `src/lsp/updater.rs` (`apply_rung2` ~:465 `assemble_program_graph(&cur.dep_layer, …)`)
- Modify: `src/program/build.rs` (possibly split `DepLayer` or add a rebuild entry point)
- Test: existing `lsp_incremental_parity` rung-2 coverage + a new rung-2-after-drop equality test

**Design (investigation's cheaper sound variant — binding; the three-segment
façade is explicitly REJECTED for this arc):**
- `dep_layer.dep_routines` (126,640 nodes) + `dep_objects` (11,165) are 96%
  duplicated inside `graph.routines`/`graph.objects` and retained on EVERY
  snapshot solely so `apply_rung2` can call
  `assemble_program_graph(&cur.dep_layer, …)`. Stop retaining them: after
  `from_context` finishes assembling, store a `DepLayerSlim` (the cheap fields:
  `apps: AppRegistry`, `topology`, `friends`, plus WHATEVER ELSE
  `assemble_program_graph` reads besides the node Vecs — read `build.rs`'s
  `DepLayer` struct and `assemble_program_graph`'s body and enumerate) and the
  inputs needed to REBUILD the node Vecs on demand (the investigation names the
  retained `AbiCache`/dep `ParsedUnit`s path via `build_dep_layer` ≈ 225 ms —
  verify what `build_dep_layer` actually consumes and what is already retained
  on `LspSnapshot`/`Updater`; `snap: Arc<AppSetSnapshot>` is already there).
- On rung-2: rebuild the dep node Vecs (`build_dep_layer`-equivalent), assemble
  as today. **`AppRef` stability is the sharp edge**: the rebuilt layer must
  re-intern apps in the IDENTICAL order (`AppRegistry::intern` order mirrors
  `snap.apps` order — build.rs:29-32 documents this), so `AppRef`s in the
  forwarded `dep_meta`/`decl_by_id`/`incoming` maps stay valid. Assert this in
  a test: rung-2 after the change produces a graph whose
  `apps` registry equals the original's, and whose routines/objects Vecs are
  byte-equal to a never-dropped build.
- **Reconstruction-by-filtering `graph` is UNSOUND** (synthetic platform
  publishers are dep-attributed and would double-inject —
  build.rs:400-473). Rebuild from source inputs only.
- If measurement shows `build_dep_layer` rebuild meaningfully regresses rung-2
  beyond ~150 ms + ~225 ms ≈ 375 ms, that is the accepted trade (rung-2 =
  infrequent signature-change saves); record it honestly in the CHANGELOG and
  docs. If it regresses MORE than that, stop and investigate before committing.

**Steps:**

- [ ] **Step 1: Failing equality test.** New test (in `tests/lsp_incremental_parity.rs`
  or snapshot.rs tests): build full snapshot, apply a rung-2 (signature-change)
  edit, assert the resulting snapshot's `graph.routines`/`graph.objects`/
  `apps`/`edges_by_file`/`decl_by_id` equal those from the SAME edit applied on
  a build that retained the full dep_layer (i.e. capture expected values before
  implementing the drop — or structure the test as: full rebuild from edited
  sources == rung-2 result, which is the stronger existing-parity shape; PREFER
  the latter if `lsp_incremental_parity` already has that harness).
- [ ] **Step 2: Implement** `DepLayerSlim` + rung-2 rebuild path.
- [ ] **Step 3: Full validation.** `cargo test` (zero goldens), clippy,
  north-star SHA `0a3b85bc…` (the resolver consumes the assembled graph — the
  strongest gate this task has).
- [ ] **Step 4: Measure on CDO.** Steady-state RSS before/after (expect ~630 →
  ~580 MB); rung-2 latency before/after (expect ~150 ms → ~375 ms, the accepted
  trade); rung-1 unaffected. `cargo test --release --test perf_bounds`.
- [ ] **Step 5: Commit** `perf(lsp): drop duplicated dep node Vecs; rung-2 rebuilds dep layer (~49 MB RSS)`.

---

### Task 5: Measurement close-out (docs §13)

**Files:**
- Modify: `docs/perf-regression-t3-vs-0.9.3.md` (append §13, style of §10-§12)
- Modify: `CHANGELOG.md` if close-out numbers differ from per-task entries

**Steps:**

- [ ] **Step 1: Final CDO measurement pass.** Median-of-5: rung-1 save
  end-to-end (apply + diagnostics), rung-2 latency, cold start, steady-state
  RSS, `aldump --program-call-graph-stats` wall time, `compute_all`-equivalent
  per-save diagnostics cost. Final north-star SHA check.
- [ ] **Step 2: Append §13** with the journey table (baselines from Global
  Constraints vs final), per-task attribution, the honest re-scope note (Tier-2
  relabelled: latency wave + one RSS task; C NO-GO at 18.7 MB measured vs 40-80
  claimed; F2 deferred), and remaining backlog (A-façade variant dead, M4
  dep-layer cache interaction note: Task 4's `DepLayerSlim` is a step TOWARD
  M4's serialization boundary).
- [ ] **Step 3: Commit** `docs: tier-2 latency wave close-out (section 13)`.

---

## Self-Review Notes

- Task order is dependency-driven: 1 → 2 (delta plumbing), 3 and 4 independent
  of each other but AFTER 1-2 to keep rung-1/rung-2 measurement attribution
  clean. 4 touches `apply_rung2` which 3 also touches (par loop) — land 3
  first, 4 rebases on it.
- Interaction guard (from the investigation): do NOT half-adopt item C's
  interning inside Task 1 — `RoutineNodeId` keys stay `String`-based; only
  `EdgeRef.file` changes type.
