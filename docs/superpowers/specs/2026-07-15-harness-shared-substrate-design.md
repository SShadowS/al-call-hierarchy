# CDO Harness Shared-Substrate Refactor — Design

**Date:** 2026-07-15
**Status:** Approved (Approach A: library-level context-threading refactor)

## Problem

`scripts/cdo-gate`'s first leg (`cargo test --release --test program_resolve_harness
-- --test-threads=1`) takes ~97 s. Profiling (nextest per-test timings,
single-threaded, release, real CDO workspace) shows 177 of 187 tests are
milliseconds; **10 CDO-gated tests account for ~89 s**, and each one
independently rebuilds the same substrate (snapshot → parse → program graph →
full resolve, ~3-7 s per build on CDO) because every library entry point takes
`&Path`:

| Entry point | Redundancy |
|---|---|
| `resolve_full_program(&Path)` (`src/program/resolve/full.rs:920`) | called directly by 3+ tests, and internally by every helper below |
| `run_route_applicability(&Path)` (`semantic_golden.rs:2139`) | builds its OWN snapshot+graph+raw-ABI, **then also calls `resolve_full_program(&Path)`** — ~2 full builds per call; called identically by 2 tests (`route_applicability_zero_violations`, `fan_out_applicability_zero_violations`) |
| `run_cdo_semantic_audit(&Path)` (`semantic_golden.rs:2223`) | 1 full build |
| `run_cdo_trigger_audit(&Path)` / `run_cdo_event_audit(&Path)` (`semantic_golden.rs:2407/2472`) | 1 full build each |
| `run_abi_integrity_check(&Path)` (`abi_check.rs:427`) | 1 full build; its caller `abi_ingestion_integrity_cdo_gate` does a SECOND manual snapshot→graph→parse→stub-resolve |

The root cause is the library API shape, not the tests: nothing lets a caller
compose these checks over one shared substrate.

**License to share:** the harness's determinism test already proves two
back-to-back `resolve_full_program(&ws)` runs produce identical
histograms/obligations, so read-only tests consuming one shared build is
behavior-equivalent by the engine's own tested contract.

## Design (Approach A — context-threading)

### 1. `src/program/resolve/full.rs`

- Promote `ProgramContext` and `build_context` from `pub(crate)` to `pub`.
  The struct stays **opaque** (fields remain `pub(crate)`); external consumers
  never poke at `snap`/`graph`/`parsed` directly.
- Add `pub fn resolve_full_program_with(&ProgramContext) -> ProgramReport` —
  the existing body of `resolve_full_program` minus the `build_context` call.
  `resolve_full_program(&Path)` becomes a thin wrapper:
  `build_context(root).map(|ctx| resolve_full_program_with(&ctx))`.

### 2. `src/program/resolve/semantic_golden.rs`

Each `run_*(&Path)` splits into a substrate-taking core + a path wrapper that
delegates (wrappers keep the exact current signature and behavior — aldump,
`mint-goldens`, and all fixture tests are untouched):

- `run_route_applicability_on(ctx: &ProgramContext, report: &ProgramReport) -> ApplicabilityReport`
  — drops BOTH its internal snapshot/graph rebuild AND its internal
  `resolve_full_program` call; raw-ABI index is derived from `ctx`.
- `run_cdo_semantic_audit_on(report: &ProgramReport, ws: &Path) -> CdoSemanticAuditReport`
  — golden load/HMAC/drift-stamp logic unchanged; `ws` retained only for the
  drift-stamp check against the workspace.
- `run_cdo_trigger_audit_on` / `run_cdo_event_audit_on` — same pattern.

### 3. `src/program/resolve/abi_check.rs`

- `run_abi_integrity_check_on(ctx: &ProgramContext, report: &ProgramReport) -> AbiIntegrityReport`;
  path wrapper delegates.

### 4. Harness (`tests/program_resolve_harness.rs`)

- One shared cache:
  ```rust
  struct CdoShared { ctx: ProgramContext, report: ProgramReport }
  static CDO_SHARED: OnceLock<Option<CdoShared>> = OnceLock::new();
  fn cdo_shared() -> Option<&'static CdoShared> { /* cdo_ws_or_enforce + build once */ }
  ```
- The ~10 hot CDO tests switch from `run_*(&ws)` to the `_on` cores over
  `cdo_shared()`. `abi_ingestion_integrity_cdo_gate`'s second manual build
  reuses `ctx.graph`/`ctx.parsed` via the shared context (its stub-resolve
  remains its own computation).
- **Exclusions (unchanged by design):** the determinism test keeps its two
  independent `resolve_full_program(&ws)` builds — that independence IS the
  assertion. Ignored diagnostic dumps keep their own builds.

### Non-goals

- `--test-threads=1` stays (user decision; protocol unchanged).
- No memoization inside `build_context` (hidden global state in production
  code, stale-cache hazard for LSP/watcher consumers — rejected).
- No change to what any test asserts.

## Error handling

`cdo_shared()` mirrors `cdo_ws_or_enforce()` semantics: `None` (skip) when
`CDO_WS` is unset and unenforced; panic under `ENFORCE_CDO_WS=1`; panic if the
one shared build fails (every consumer would have failed identically).
`OnceLock<Option<...>>` caches the skip verdict too, so non-CDO runs stay
zero-cost.

## Memory

Single-threaded gate: holding one `ProgramContext` + `ProgramReport` for the
process lifetime replaces N sequential rebuild-and-drop cycles of the same
data — peak RSS is flat or lower.

## Validation contract

1. Whole harness green: `cargo test --release --test program_resolve_harness
   -- --test-threads=1` with `CDO_WS` + `ENFORCE_CDO_WS=1` — 187 passed.
2. **Metric identity:** the printed north-star numbers (histograms, coverage,
   audit digests) must be byte-identical to the pre-refactor run.
3. Fixture-only tests and all other suites unaffected: full `cargo nextest run`
   green (2447).
4. `cargo clippy --release --all-targets --all-features` clean.
5. Measurement: gate leg-1 wall time before (~97 s) vs after, same machine,
   plain cargo, release.

## Expected outcome

~10-14 redundant CDO builds collapse to 1 shared build (~5-7 s) + per-test
residual work (golden loads, HMAC verification, assertions). Gate leg-1
estimated at ~30-45 s from ~97 s.
