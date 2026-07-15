# CDO Harness Shared-Substrate Refactor — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Collapse the ~10-14 redundant CDO substrate rebuilds in `program_resolve_harness` to one shared build by making the library's `run_*`/`resolve_full_program` entry points composable over a `ProgramContext` + `ProgramReport`.

**Architecture:** Approach A from `docs/superpowers/specs/2026-07-15-harness-shared-substrate-design.md` — promote `ProgramContext`/`build_context` to `pub`, split each `&Path`-taking entry point into a substrate-taking `_on` core plus a thin path wrapper that delegates (zero behavior change for existing consumers), and give the harness one `OnceLock<Option<CdoShared>>` that the hot CDO tests consume.

**Tech Stack:** Rust; `std::sync::OnceLock`; existing engine modules (`src/program/resolve/{full,semantic_golden,abi_check,differential}.rs`).

## Global Constraints

- Branch: work directly on `master`? NO — create `feat/harness-shared-substrate` off `master` in Task 0. Never merge/push without explicit user request.
- One commit per task, trailer: `Co-authored-by: Copilot <223556219+Copilot@users.noreply.github.com>`. Stage ONLY intended paths — never `git add -A`. NEVER stage `.cargo/`, `.config/`, `.panel/`, `demo-out/`, `finish-*.ps1`.
- Format touched .rs files with `rustfmt <file>` per file — NEVER `cargo fmt`.
- Path wrappers keep their EXACT current signatures and failure semantics (default-report on snapshot failure, `None`, etc.) — aldump, `mint-goldens`, fixture tests, and `graphify_export` are untouched.
- The harness's resolve-determinism test (its two independent `resolve_full_program(&ws)` builds, around `tests/program_resolve_harness.rs:2793`) and the three `#[ignore]`d diagnostic dumps (lines ~3146/3228/3279) are NOT rewired — leave them byte-identical.
- Audit-internal determinism re-runs (`audit2` at harness lines ~3768/3858/3920) SWITCH to the `_on` core over the same shared report: they keep asserting audit-projection determinism; resolve-layer determinism stays covered by the dedicated test. Call this out in the CHANGELOG.
- Metric identity: all printed north-star numbers/digests must match the Task 0 baseline capture byte-for-byte (modulo timing lines).
- Perf measurement uses plain `cargo` (never nextest). CDO env for gate runs (PowerShell): `$env:CDO_WS='u:\Git\DO.Support-SlowDOSetup\DocumentOutput\Cloud'; $env:ENFORCE_CDO_WS='1'`.
- `cdo_shared()` panics if the one shared build fails (every consumer would have failed identically); skips (returns `None`) exactly when `cdo_ws_or_enforce()` does.

---

### Task 0: Branch + baseline metric capture (no code commit)

- [ ] **Step 1:** `git checkout master; git checkout -b feat/harness-shared-substrate`
- [ ] **Step 2:** Capture the pre-refactor printed metrics and timing (release, single-threaded, hot tests only — nocapture output IS the baseline artifact):
```powershell
$env:CDO_WS='u:\Git\DO.Support-SlowDOSetup\DocumentOutput\Cloud'; $env:ENFORCE_CDO_WS='1'
New-Item -ItemType Directory -Force .superpowers\sdd\harness-substrate | Out-Null
cargo test --release --test program_resolve_harness -- --test-threads=1 --nocapture 2>&1 |
  Out-File .superpowers\sdd\harness-substrate\baseline-metrics.txt -Encoding utf8
Select-String -Path .superpowers\sdd\harness-substrate\baseline-metrics.txt -Pattern 'test result'
```
Expected: `ok. 187 passed`. Record the wall time (prior measurement: ~97 s). This file is the metric-identity oracle for Task 5 — do not lose it. (It lives under the gitignored `.superpowers/` tree; no commit.)

### Task 1: `full.rs` — public `ProgramContext` + `resolve_full_program_with`

**Files:**
- Modify: `src/program/resolve/full.rs` (struct at ~:1017, `build_context` at ~:1030, `resolve_full_program` at ~:920)
- Test: `tests/program_resolve_harness.rs` (new fixture-based equivalence test)

**Interfaces:**
- Produces: `pub struct ProgramContext` (fields stay `pub(crate)`; new `pub fn graph(&self) -> &ProgramGraph`, `pub fn parsed(&self) -> &[ParsedUnit]`), `pub fn build_context(workspace_root: &Path) -> Option<ProgramContext>`, `pub fn resolve_full_program_with(ctx: &ProgramContext) -> ProgramReport`. Tasks 2-4 consume all three.

- [ ] **Step 1: Write the failing test** — append to `tests/program_resolve_harness.rs` (near the other fixture tests, after the `full_program_fixture` helpers ~line 1100):
```rust
/// Shared-substrate refactor (2026-07-15 spec): the context-taking core and
/// the path wrapper must produce identical reports — the wrapper IS
/// `build_context` + `resolve_full_program_with`, so any divergence means the
/// split leaked behavior.
#[test]
fn resolve_full_program_with_matches_path_wrapper_on_fixture() {
    use al_call_hierarchy::program::resolve::full::{build_context, resolve_full_program_with};

    let fixture =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/semantic-golden");
    let via_path = resolve_full_program(&fixture).expect("wrapper must succeed on fixture");
    let ctx = build_context(&fixture).expect("build_context must succeed on fixture");
    let via_ctx = resolve_full_program_with(&ctx);

    assert_eq!(via_path.histogram, via_ctx.histogram, "histogram must match");
    assert_eq!(
        via_path.primary_histogram, via_ctx.primary_histogram,
        "primary histogram must match"
    );
    assert_eq!(
        via_path.edges.len(),
        via_ctx.edges.len(),
        "edge count must match"
    );
}
```
- [ ] **Step 2: Run test to verify it fails**
Run: `cargo test --test program_resolve_harness resolve_full_program_with_matches -- --nocapture`
Expected: COMPILE FAIL — `build_context`/`resolve_full_program_with` are not public / do not exist.
- [ ] **Step 3: Implement** in `src/program/resolve/full.rs`:
  1. `pub(crate) struct ProgramContext` → `pub struct ProgramContext` (leave every FIELD `pub(crate)`).
  2. `pub(crate) fn build_context` → `pub fn build_context` (add `#[must_use]` if missing).
  3. Add accessors to `impl ProgramContext` (create the impl block right under the struct):
```rust
impl ProgramContext {
    /// The assembled whole-program graph (shared-substrate consumers only).
    #[must_use]
    pub fn graph(&self) -> &ProgramGraph {
        &self.graph
    }

    /// The parsed units backing `graph` (shared-substrate consumers only).
    #[must_use]
    pub fn parsed(&self) -> &[ParsedUnit] {
        &self.parsed
    }
}
```
  4. Split `resolve_full_program`: move its ENTIRE body after the `build_context` call into a new `pub fn resolve_full_program_with(ctx: &ProgramContext) -> ProgramReport` (the body already destructures `&ctx` — change the destructure source from the local `ctx` to the parameter; everything else moves verbatim). The wrapper becomes:
```rust
#[must_use]
pub fn resolve_full_program(workspace_root: &Path) -> Option<ProgramReport> {
    let ctx = build_context(workspace_root)?;
    Some(resolve_full_program_with(&ctx))
}
```
  Keep `resolve_full_program`'s existing doc comment on the wrapper; give `resolve_full_program_with` a doc noting it is the substrate-taking core (steps 5-8 of the wrapper's documented pipeline).
- [ ] **Step 4: Run test to verify it passes**
Run: `cargo test --test program_resolve_harness resolve_full_program_with_matches`
Expected: PASS. Also: `cargo test --lib resolve` green (in-crate resolve unit tests unaffected).
- [ ] **Step 5: Commit**
```powershell
rustfmt src\program\resolve\full.rs; rustfmt tests\program_resolve_harness.rs
git add src\program\resolve\full.rs tests\program_resolve_harness.rs
git commit -m "refactor(resolve): public ProgramContext + resolve_full_program_with core"
```
(with trailer)

### Task 2: `semantic_golden.rs` + `differential.rs` — substrate-taking cores

**Files:**
- Modify: `src/program/resolve/semantic_golden.rs` (`run_route_applicability` :2139, `run_unknown_include_sender_plus1_subscribers_preflight` :2186, `run_cdo_semantic_audit` :2223, `run_cdo_trigger_audit` :2407, `run_cdo_event_audit` :2472, `mint_fresh_golden_for_kind` :1484)
- Modify: `src/program/resolve/differential.rs` (`project_fresh_event_rows` :487)

**Interfaces:**
- Consumes: `ProgramContext`/`build_context`/`resolve_full_program_with` from Task 1. In-module code may read `ctx.snap`/`ctx.graph`/`ctx.parsed`/`ctx.primary_app_ref`/`ctx.ws_file_set` directly (`pub(crate)` fields, same crate).
- Produces (all `pub`, `#[must_use]`, consumed by Task 4):
  - `run_route_applicability_on(ctx: &ProgramContext, report: &ProgramReport) -> ApplicabilityReport`
  - `run_unknown_include_sender_plus1_subscribers_preflight_on(ctx: &ProgramContext) -> usize`
  - `run_cdo_semantic_audit_on(ctx: &ProgramContext, report: &ProgramReport, workspace_root: &Path) -> CdoSemanticAuditReport`
  - `run_cdo_trigger_audit_on(ctx: &ProgramContext, report: &ProgramReport, workspace_root: &Path) -> AnonTriggerAuditReport`
  - `run_cdo_event_audit_on(ctx: &ProgramContext, workspace_root: &Path) -> AnonEventAuditReport`
  - `differential::project_fresh_event_rows_on(ctx: &ProgramContext) -> Vec<CanonicalEventRow>`
  - `mint_fresh_golden_for_kind_on(ctx: &ProgramContext, report: &ProgramReport, kind: EdgeKind) -> SemanticGolden` (may stay `pub(crate)` — only the audits call it)

The recipe is identical for every function: the current body's `SnapshotBuilder{...}.build()` + `build_program_graph` + `parse_snapshot` + `resolve_full_program(workspace_root)` block is DELETED from the core and replaced by reads off `ctx`/`report`; the old `&Path` function becomes a wrapper. Where the old code did `report.edges.into_iter().map(|ce| ce.edge)` on an OWNED report, the core clones instead: `report.edges.iter().map(|ce| ce.edge.clone())` (ms-scale; the seconds are in the builds).

- [ ] **Step 1: `run_route_applicability`** — new core (verbatim; note `ws_file_set`/`primary_app_ref` come from ctx, NOT recomputed):
```rust
#[must_use]
pub fn run_route_applicability_on(
    ctx: &crate::program::resolve::full::ProgramContext,
    report: &crate::program::resolve::full::ProgramReport,
) -> ApplicabilityReport {
    let raw_abi = build_raw_abi_index_from_snapshot(&ctx.snap, &ctx.graph.apps);
    let index = ResolveIndex::build(&ctx.graph);
    let fan_out_ctx = build_fan_out_site_context(
        &ctx.graph,
        &index,
        &ctx.parsed,
        ctx.primary_app_ref,
        &ctx.ws_file_set,
    );
    let all_edges: Vec<Edge> = report.edges.iter().map(|ce| ce.edge.clone()).collect();
    route_applicability(&all_edges, &raw_abi, &ctx.graph, &index, &fan_out_ctx, &ctx.parsed)
}
```
Wrapper (replaces the whole old body):
```rust
#[must_use]
pub fn run_route_applicability(workspace_root: &Path) -> ApplicabilityReport {
    use crate::program::resolve::full::{build_context, resolve_full_program_with};
    let Some(ctx) = build_context(workspace_root) else {
        return ApplicabilityReport::default();
    };
    let report = resolve_full_program_with(&ctx);
    run_route_applicability_on(&ctx, &report)
}
```
NOTE the old body's manual `ws_file_set` derivation (`snap.apps.first()...`) differs textually from `build_context`'s — verify `build_context` computes the same set (it feeds `resolve_full_program`, which produced identical results per the determinism/coverage tests; if the fixture equivalence test from Task 1 plus this task's Step 6 run green, the sets agree). Also note the old wrapper silently ignored `graph.apps.find` failure with `default()` — `build_context` returning `None` covers the same case.
- [ ] **Step 2: `run_unknown_include_sender_plus1_subscribers_preflight`** — core is one line over ctx:
```rust
#[must_use]
pub fn run_unknown_include_sender_plus1_subscribers_preflight_on(
    ctx: &crate::program::resolve::full::ProgramContext,
) -> usize {
    crate::program::resolve::index::count_unknown_include_sender_plus1_subscribers(&ctx.graph)
}
```
Wrapper: `build_context(workspace_root).map(|ctx| run_..._on(&ctx))` (keeps `Option<usize>` fail-closed semantics).
- [ ] **Step 3: `mint_fresh_golden_for_kind` + `run_cdo_trigger_audit`** — `mint_fresh_golden_for_kind_on(ctx, report, kind)` deletes the snapshot/graph/resolve block; body becomes:
```rust
let edges: Vec<Edge> = report
    .edges
    .iter()
    .map(|ce| ce.edge.clone())
    .filter(|e| e.kind == kind)
    .collect();
let canonical = project_fresh(&edges, &ctx.graph.apps);
build_golden_from_canonical(&canonical)
```
Path wrapper delegates via `build_context` + `resolve_full_program_with` (returns `SemanticGolden::default()` on `None`, matching today). `run_cdo_trigger_audit_on(ctx, report, workspace_root)` is the old body with `mint_fresh_golden_for_kind(workspace_root, ...)` → `mint_fresh_golden_for_kind_on(ctx, report, EdgeKind::ImplicitTrigger)`; everything else (golden load, drift warn against `workspace_root`, anon, digest) unchanged. Wrapper delegates.
- [ ] **Step 4: `run_cdo_semantic_audit`** — `run_cdo_semantic_audit_on(ctx, report, workspace_root)`: delete the internal `SnapshotBuilder`/`build_program_graph` block (use `ctx.snap`/`ctx.graph`) and the internal `resolve_full_program(workspace_root)` call (use the `report` param); `ws_ref` comes from `ctx.graph.apps.find(&ctx.snap.workspace_app)` exactly as before (keep the early-return `Default` arms — they are now unreachable-but-harmless for the `_on` path); `ws_edges` filter becomes `report.edges.iter().filter(...).map(|ce| ce.edge.clone())`. Golden load/overlay/drift/diff/adjudication logic untouched. Wrapper delegates (same `Default` early-returns on `build_context` failure).
- [ ] **Step 5: `project_fresh_event_rows` + `run_cdo_event_audit`** — in `differential.rs`, `project_fresh_event_rows_on(ctx)` keeps `ResolveIndex::build` + `DeclSurface::build` + `emit_event_flow_edges` but reads `&ctx.graph`/`&ctx.parsed` instead of building snapshot/graph/parse; path wrapper delegates. In `semantic_golden.rs`, `run_cdo_event_audit_on(ctx, workspace_root)` swaps `project_fresh_event_rows(workspace_root)` → `project_fresh_event_rows_on(ctx)`; wrapper delegates. (No `report` param — the event audit never used the resolve report.)
- [ ] **Step 6: Verify** — the existing fixture tests ARE the test suite for this task (wrappers must be behavior-identical):
Run: `cargo test --test program_resolve_harness` (fixture subset, no CDO env) and `cargo test --lib semantic_golden differential`
Expected: all green, zero new failures.
- [ ] **Step 7: Commit**
```powershell
rustfmt src\program\resolve\semantic_golden.rs; rustfmt src\program\resolve\differential.rs
git add src\program\resolve\semantic_golden.rs src\program\resolve\differential.rs
git commit -m "refactor(resolve): substrate-taking _on cores for semantic-golden/event helpers"
```

### Task 3: `abi_check.rs` — substrate-taking core

**Files:**
- Modify: `src/program/resolve/abi_check.rs` (`run_abi_integrity_check` :427)

**Interfaces:**
- Consumes: `ProgramContext` (Task 1).
- Produces: `pub fn run_abi_integrity_check_on(ctx: &ProgramContext) -> AbiIntegrityReport` (Task 4 consumes).

- [ ] **Step 1: Implement** — core:
```rust
#[must_use]
pub fn run_abi_integrity_check_on(
    ctx: &crate::program::resolve::full::ProgramContext,
) -> AbiIntegrityReport {
    let raw_index = build_raw_abi_index_from_snapshot(&ctx.snap, &ctx.graph.apps);
    abi_ingestion_integrity_from_graph(&ctx.graph, &raw_index)
}
```
Wrapper: `build_context(workspace_root)` → on `None` return the existing all-zeros `AbiIntegrityReport` literal (preserve failure shape exactly); else delegate.
- [ ] **Step 2: Verify** — `cargo test --test program_resolve_harness abi` (fixture ABI tests 12-14) green; `cargo test --lib abi_check` green.
- [ ] **Step 3: Commit** — `git add src\program\resolve\abi_check.rs` (rustfmt first), message `refactor(resolve): run_abi_integrity_check_on substrate core`.

### Task 4: Harness — shared `OnceLock` substrate + rewire hot tests

**Files:**
- Modify: `tests/program_resolve_harness.rs`

**Interfaces:**
- Consumes: everything produced by Tasks 1-3.
- Produces: `fn cdo_shared() -> Option<&'static CdoShared>` (harness-internal).

- [ ] **Step 1: Add the shared cache** near the top of the harness (after the `cdo` module include / imports):
```rust
/// Shared CDO substrate (2026-07-15 shared-substrate spec): ONE snapshot →
/// graph → parse → full-resolve build, consumed by every read-only CDO test.
/// The engine's own determinism test (two independent builds, below) licenses
/// the sharing; that test and the #[ignore]d dumps deliberately do NOT use
/// this. Panics if the one build fails (every consumer would have failed
/// identically). `None` == the usual CDO_WS skip.
struct CdoShared {
    ctx: al_call_hierarchy::program::resolve::full::ProgramContext,
    report: al_call_hierarchy::program::resolve::full::ProgramReport,
}

static CDO_SHARED: std::sync::OnceLock<Option<CdoShared>> = std::sync::OnceLock::new();

fn cdo_shared() -> Option<&'static CdoShared> {
    CDO_SHARED
        .get_or_init(|| {
            let ws = cdo_ws_or_enforce()?;
            let ctx = al_call_hierarchy::program::resolve::full::build_context(&ws)
                .expect("shared CDO substrate: build_context must succeed on CDO_WS");
            let report =
                al_call_hierarchy::program::resolve::full::resolve_full_program_with(&ctx);
            Some(CdoShared { ctx, report })
        })
        .as_ref()
}
```
- [ ] **Step 2: Rewire the hot tests** (each: replace `let Some(ws) = cdo_ws_or_enforce() else { return; };` + the `run_*(&ws)`/`resolve_full_program(&ws)` call; keep `ws` via `cdo_ws_or_enforce()` ONLY where the `_on` core takes a `workspace_root` param):
  - `abi_ingestion_integrity_cdo_gate` (:876): `let Some(shared) = cdo_shared() else { return; };` then :881 and :996 → `run_abi_integrity_check_on(&shared.ctx)`; the manual histogram block's `SnapshotBuilder`/`build_program_graph`/`parse_snapshot` → `shared.ctx.graph()` / `shared.ctx.parsed()` feeding the existing `resolve_program` stub call unchanged.
  - `cdo_full_program_coverage_and_self_reported_metric` (:1328): report = `&shared.report`.
  - `route_applicability_zero_violations` (:3088) and `fan_out_applicability_zero_violations` (:5227): `run_route_applicability_on(&shared.ctx, &shared.report)`.
  - `cdo_unknown_include_sender_plus1_subscribers_preflight_is_zero` (:3380): `run_unknown_include_sender_plus1_subscribers_preflight_on(&shared.ctx)` (drop the `Option` unwrap — the core returns `usize`).
  - `cdo_l3_semantic_audit_no_fresh_wrong` (:3441 + audit2 :3768): both calls → `run_cdo_semantic_audit_on(&shared.ctx, &shared.report, &ws)` (keep `ws` binding for the param).
  - `cdo_trigger_audit_frozen_load` (:3796 + :3858): → `run_cdo_trigger_audit_on(&shared.ctx, &shared.report, &ws)`.
  - `cdo_event_audit_frozen_load` (:3882 + :3920): → `run_cdo_event_audit_on(&shared.ctx, &ws)`.
  - `cdo_builtin_dispatch_audit_flagged_count_is_pinned` (:10896): report = `&shared.report`.
  - Where a test previously did `report.edges.into_iter()` / consumed the report by value, switch to iterating `shared.report.edges.iter()` with `.clone()` only where an owned value is required.
  - DO NOT touch: the determinism test (~:2793 region), `task2_dump_argtype_dispatch_flips_on_cdo`, `task3_dump_untracked_receiver_sites_on_cdo`, `task3_dump_remaining_ambiguous_resolved_sites_on_cdo`, `cdo_genuine_wrong_is_precedence_adjudicated` UNLESS it calls one of the refactored helpers — check its body; it loads overrides + reads CDO source files and per the call-site map does not call `resolve_full_program(&ws)` directly; leave it alone if so.
- [ ] **Step 3: Fixture-path compile check** — `cargo test --test program_resolve_harness --no-run` then run WITHOUT CDO env: `cargo test --test program_resolve_harness` — all fixture tests green, CDO tests skip (proves `cdo_shared()`'s skip path).
- [ ] **Step 4: CDO run** (the real gate):
```powershell
$env:CDO_WS='u:\Git\DO.Support-SlowDOSetup\DocumentOutput\Cloud'; $env:ENFORCE_CDO_WS='1'
cargo test --release --test program_resolve_harness -- --test-threads=1
```
Expected: `ok. 187 passed` (+ the Task 1 equivalence test = 188 if counted here — use the actual number from Task 0's baseline plus one).
- [ ] **Step 5: Commit** — rustfmt the harness; `git add tests\program_resolve_harness.rs`; message `perf(tests): share one CDO substrate across read-only harness tests`.

### Task 5: Validation, measurement, docs

**Files:**
- Modify: `CHANGELOG.md`, `docs/perf-regression-t3-vs-0.9.3.md`, `.superpowers/sdd/progress.md` (gitignored ledger — update but don't stage)

- [ ] **Step 1 (metric identity):** re-run with `--nocapture` into `after-metrics.txt` (same command as Task 0 Step 2) and diff the METRIC lines against the baseline:
```powershell
$b = Select-String -Path .superpowers\sdd\harness-substrate\baseline-metrics.txt -Pattern 'digest=|histogram|real_unknown|coverage|total=|matches=|l3_total=' | % Line
$a = Select-String -Path .superpowers\sdd\harness-substrate\after-metrics.txt -Pattern 'digest=|histogram|real_unknown|coverage|total=|matches=|l3_total=' | % Line
Compare-Object $b $a
```
Expected: NO differences (empty output). ANY metric drift = STOP, root-cause before proceeding (a `_on` core leaked behavior).
- [ ] **Step 2 (whole repo):** `cargo nextest run` (expect 2448: 2447 + the Task 1 equivalence test) and `cargo clippy --release --all-targets --all-features` clean.
- [ ] **Step 3 (measure):** record leg-1 wall time from Step 1's run (plain cargo, release, single-threaded) vs Task 0 baseline (~97 s). Expected: ~30-45 s.
- [ ] **Step 4 (docs):** CHANGELOG `### Changed` entry (substrate sharing, the `_on` API additions, the audit2-determinism scope note, measured numbers); short addendum in `docs/perf-regression-t3-vs-0.9.3.md` (gate leg-1 before/after + methodology); ledger section in `.superpowers/sdd/progress.md`.
- [ ] **Step 5: Commit** — `git add CHANGELOG.md docs\perf-regression-t3-vs-0.9.3.md`; message `docs: harness shared-substrate measured results`.

## Post-plan (not tasks)

Final whole-branch Fable review (errors+performance focus), then the full CDO gate (`program_resolve_harness` single-threaded AND `cargo test --release --test lsp -- program_graph:: snapshot_robustness::`) before any merge to master — merge only on explicit user request. Nothing is ever pushed.

## Self-Review Notes

- Spec coverage: §1 full.rs (Task 1), §2 semantic_golden (Task 2), §3 abi_check (Task 3), §4 harness + exclusions (Task 4), validation contract 1-5 (Tasks 0/4/5). ✓
- The spec's `run_route_applicability_on` signature listed `(ctx, report)` and event audit implicitly `(ctx, ws)` — plan matches; semantic/trigger audits carry `workspace_root` for the drift-stamp per spec §2. ✓
- Type consistency: `CdoShared{ctx, report}` fields match every `_on` signature; `graph()`/`parsed()` accessors cover the harness's only direct-field needs (abi test's stub block). ✓
- No placeholders; every core's replacement body is shown or is a verbatim-move with the exact deleted block named. ✓
