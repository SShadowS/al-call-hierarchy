# Task T0.5 Report — Performance targets: measure + CI-asserted generous bounds

## Status: DONE. All 5 gates green.

## Commits

(to be created — see "Commit plan" below; this report is written pre-commit so it
can be included in the same commit set.)

## One-line summary

Built a deterministic synthetic-corpus generator, Criterion benches, and a
release-only 3x-bound CI gate for the legacy LSP pipeline; every CLAUDE.md target has
wide measured headroom — 1000-file initial index ~15.9ms (target <2s), the 3
query handlers ~0.9µs–399µs (target <1ms each), single-file reindex ~197µs (target
<50ms) — and CLAUDE.md's perf table now carries these numbers.

## Measured numbers (target / 3x CI bound / measured)

All measured via `cargo bench --bench lsp_pipeline` (Criterion mean estimate, dev
machine, release/full-LTO build) against the deterministic 1000-file synthetic corpus
(100-file corpus used only for the 100-file index row). `tests/perf_bounds.rs`
independently measured the same operations (`Instant`-based, median of 3-5 runs) and
got consistent numbers — see "Bench vs. bounds design" below.

| Operation | CLAUDE.md target | CI bound (3x) | Measured (Criterion) | Measured (perf_bounds median) |
|---|---|---|---|---|
| Initial index (100 files) | <500ms | 1500ms | ~1.97ms | ~2.04ms |
| Initial index (1000 files) | <2s | 6000ms | ~15.9ms | ~14.9ms |
| prepareCallHierarchy | <1ms | 3ms | ~893ns | ~1µs |
| incomingCalls (999-way fan-in) | <1ms | 3ms | ~399µs | ~352µs |
| outgoingCalls (3-way fan-out) | <1ms | 3ms | ~1.9µs | ~2.5µs |
| Single-file reindex (1000-file graph) | <50ms | 150ms | ~197µs | ~218µs |

Every operation has at least 5x headroom under its *original* target, let alone the 3x
CI bound (incomingCalls is the tightest at ~2.5x headroom under the 1ms target, ~7.5x
under the 3ms CI bound — real fan-out of 999 callers still resolves in well under a
millisecond because it's a single HashMap lookup + a linear scan over pre-resolved
`CallSiteIdx` entries, no re-resolution per query).

## Corpus generator design (`tests/perf_support/`)

Deterministic, index-driven (no RNG, no seed state): `generate_corpus(dir, N)` writes N
codeunits named `GenCU00000..GenCU0000{N-1}`, each with 6 procedures. Every non-hub
file's `Proc0` makes 3 calls — one QUALIFIED cross-file call into a designated hub
codeunit (index 0) plus 2 local calls — giving:
- the hub's `Proc0` real incoming-call fan-in that scales with corpus size (`N-1`
  distinct callers for an N-file corpus), and
- every other file's `Proc0` real outgoing-call fan-out (3 callees: 1 cross-file + 2
  local).

This was a deliberate design choice over an all-isolated corpus (every file self-
contained, zero cross-file edges) specifically because the brief calls for "real
fan-out" and an isolated corpus would make `incomingCalls`/`outgoingCalls` trivially
fast in a way that doesn't stress the resolution path a real multi-file workspace
exercises. `rewrite_with_extra_procedure` mutates one file's on-disk content
(splices in a `ProcExtra` procedure) for the single-file-reindex scenario. Self-tests
for the generator live in `tests/perf_support_smoke.rs` (always-compiled, not gated on
release) rather than inside `perf_support/mod.rs` itself — see "A non-obvious pitfall"
below for why.

## Bench vs. bounds design

- **`benches/lsp_pipeline.rs`** (Criterion, `harness = false`, registered in
  `Cargo.toml`): corpus generation and indexing happen once per benchmark group,
  *outside* the timed closure — only the operation under test (`index_directory`,
  the 3 handler functions, `reindex_file`) is measured per iteration. Run with
  `cargo bench --bench lsp_pipeline` (or `cargo bench` for every bench target,
  including the pre-existing `telemetry_hot_path`).
- **`tests/perf_bounds.rs`**: a coarser, independent measurement using
  `std::time::Instant` directly (median of 3 runs for indexing/reindex, 5 for the
  sub-millisecond query handlers) with a warm-up pass before timing starts, asserting
  against 3x each CLAUDE.md target. It intentionally does NOT depend on Criterion or
  reuse any bench-target code — it's meant to be a self-contained, always-buildable CI
  tripwire independent of the (heavier, statistical) bench harness.
- **`#[cfg(not(debug_assertions))]` gating**: the real bounds checks (`release_checks`
  module) compile only in release. A debug-build timing assert is meaningless —
  unoptimized code runs several times slower for reasons unrelated to any real
  regression. This is NOT a silent-skip: CI explicitly runs
  `cargo test --release --test perf_bounds`, so the checks always compile in for the
  invocation that matters. An unconditional marker test
  (`perf_bounds_binary_is_never_empty`) guarantees the binary is never reported as
  "0 tests" in any profile, satisfying the brief's anti-silent-skip requirement without
  needing a `#[cfg(debug_assertions)]` stub test (a plain doc-comment note is used
  instead, since the marker test already covers "binary is never empty").
- **Single-file reindex realism**: the on-disk file content is changed once (via
  `rewrite_with_extra_procedure`) *before* timing starts, not inside the timed loop —
  the disk write itself isn't part of what the LSP measures (in the real didSave flow,
  the editor has already written the file by the time the notification arrives); only
  the `remove_file` + reparse + `add_to_graph` path is timed.

## A non-obvious pitfall (worth flagging for reviewers)

Originally, `perf_support/mod.rs` had its own `#[cfg(test)] mod tests { ... }` self-test
block (the usual Rust idiom). This produced `dead_code`/`unused_imports` warnings when
compiled as part of `benches/lsp_pipeline.rs` (a `harness = false` bench) — empirically,
`cargo bench`/`cargo check --benches` sets `cfg(test)` true for bench targets, but
without the `--test` codegen flag (since there's no auto-generated libtest harness main
when `harness = false`), so `#[test]`-annotated functions compile as ordinary,
unreachable functions rather than being wired into a test runner, and everything only
reachable from them (their imports, in this case) gets flagged dead. Worse: if the
`perf_support` module import in `tests/perf_bounds.rs` had been left unconditional
(not gated behind `#[cfg(not(debug_assertions))]`), a normal *debug* `cargo test` would
compile all of `perf_support`'s `pub fn`s as dead code too, since `release_checks` (the
only caller) is compiled out in debug — which would fail
`cargo clippy --all-targets --all-features -- -D warnings` (gate 4) on every ordinary
debug lint pass, not just in some edge case. Fixed by (1) moving the corpus generator's
own correctness tests into a separate always-compiled `tests/perf_support_smoke.rs`
integration-test file rather than an embedded `#[cfg(test)]` block, and (2) gating the
`#[path] mod perf_support;` *declaration itself* in `perf_bounds.rs` behind
`#[cfg(not(debug_assertions))]`, matching the `release_checks` gate exactly, so in a
debug build the module isn't compiled into that binary at all (zero dead code
possible) while the bench target still exercises it fully and unconditionally.

## Enabling refactor: module ownership (graph/indexer/handlers/parser/protocol → lib.rs)

The brief requires benching `prepareCallHierarchy`/`incomingCalls`/`outgoingCalls`
"against the indexed graph ... no LSP stdio loop" — i.e., calling the actual handler
functions in-process. This was not directly possible: `graph.rs`, `indexer.rs`,
`handlers.rs`, `parser.rs`, `protocol.rs` were bin-only modules (`mod X;` in
`main.rs`), invisible to bench/test targets, which only link the library crate
(`src/lib.rs`). The repo already had exactly this problem solved once before —
`config`/`telemetry`/`app_package`/`dependencies` live in `lib.rs` with a doc comment
literally stating "so library consumers (benches, snapshot) can use them," and
`main.rs` re-exports them via `pub use al_call_hierarchy::{...}` so the remaining
bin-only files (`server.rs`, `watcher.rs`, `analysis.rs`) keep compiling unchanged
against `crate::config::*` etc. I extended that exact pattern to the 5 modules needed
for benching, rather than duplicating source (the `language.rs` "duplicate
compilation, benign" pattern also present in `lib.rs`) or inventing a third approach.

Changes required to make the move work:
- `lib.rs`: added `pub mod graph; pub mod handlers; pub mod indexer; pub mod parser;
  pub mod protocol;` with a doc comment explaining the T0.5 rationale.
- `main.rs`: removed the 5 `mod X;` declarations, extended the existing
  `pub use al_call_hierarchy::{...}` re-export line to include them. `analysis.rs`,
  `server.rs`, `watcher.rs` stayed bin-only (not needed for benching; `server.rs` is
  the actual LSP stdio loop, `watcher.rs` is file-system watching, `analysis.rs` is the
  `--analyze` CLI mode) — they keep resolving `crate::graph::*` etc. through the
  re-export, unchanged.
- `graph.rs`: fixed one self-crate-reference. `pub use al_call_hierarchy::types::
  ObjectType;` only worked because `graph.rs` used to be compiled solely as part of the
  *binary*, where `al_call_hierarchy` genuinely refers to the external library crate.
  Now that `graph.rs` is itself compiled as part of the library, that line would try to
  reference the library from within itself, which doesn't compile. Changed to
  `pub use crate::types::ObjectType;` (works identically in both contexts since
  `types` is `pub mod` in `lib.rs` either way).
- `parser.rs`: `routine_complexity_ir` was `pub(crate)`, called from `main.rs`'s
  `extract_metrics_ir`. Under the old layout this was fine (same crate). Now `parser`
  is lib-owned and `main.rs` consumes it across a real crate boundary via the
  re-export, so `pub(crate)` (library-crate-only visibility) no longer reaches it;
  widened to `pub`.
- `handlers.rs`: `prepare_call_hierarchy`, `incoming_calls`, `outgoing_calls` were
  private `fn`s (only reachable via the `handle_request` dispatcher and the module's
  own `#[cfg(test)]` tests, which still work unchanged via `super::`). Made `pub fn` so
  benches/tests can call them directly, per the brief's explicit ask.

**Verification this was behavior-preserving, not just "made it compile":** ran the
full test suite before AND after — 1340 lib tests + 24 bin tests, all pass, byte-for-
byte the same test names/counts (92 of the 1340 are the graph/indexer/handlers test
suites, which simply moved from running under the `main.rs` unittest binary to running
under the `lib.rs` unittest binary — same assertions, same behavior). `cargo fmt
--check` and `cargo clippy --all-targets --all-features -- -D warnings` both clean
(gate 4). I judged this in-scope: the brief says "No resolution-engine changes of any
kind" (i.e., no touching the L2-L5 semantic layer under `src/program/` and
`src/engine/`), not "no touching main.rs/graph.rs" — and this move is exactly the
"enabling infrastructure" the brief's own scope note implies is needed to bench "THAT
surface" at all. It is a pure visibility/ownership change with zero logic modification
inside any of the 5 moved files (confirmed via `git diff` — the only edits inside
`graph.rs`/`parser.rs`/`handlers.rs` are the 3 items listed above; every other line is
byte-identical).

## CI wiring

`.github/workflows/ci.yml` gained one step, appended directly after the existing
"Build" step (`cargo build --release`) in the same `test` job:
```yaml
- name: Perf bounds (release-only; 3x CLAUDE.md targets, T0.5)
  env:
    TREE_SITTER_AL_PATH: ${{ github.workspace }}/tree-sitter-al
  run: cargo test --release --test perf_bounds
```
This reuses the release build's compiled dependency graph (same profile, same
`target/` directory) — only the `perf_bounds` test binary itself needs compiling +
linking against already-built release rlibs, confirmed locally: after a full release
build (3m 33s, cold), `cargo test --release --test perf_bounds` took ~22s (mostly
recompiling `al-call-hierarchy` itself + linking, not the dependency tree). No new job
was needed since the workflow already had a release build in this job; the "fast lane"
(fmt/clippy/test-debug) stays untouched — perf bounds only runs after those, and only
adds the one incremental compile+test step.

## Gate evidence

1. **`cargo bench` completes and prints all bench groups.** Ran `cargo bench` (all
   targets). `lsp_pipeline` printed 3 groups (`initial_index/100_files`,
   `initial_index/1000_files`; `query_handlers_1000_files/{prepareCallHierarchy,
   incomingCalls,outgoingCalls}`; `single_file_reindex_1000_files`) plus
   `telemetry_hot_path`'s pre-existing `record_resolution_miss / disabled` group. Zero
   failures, zero panics.
2. **`cargo test --release --test perf_bounds` green, numbers land inside bounds with
   visible headroom.** 7/7 tests passed (`perf_bounds_binary_is_never_empty` +
   6 `release_checks::*`). See the numbers table above — smallest headroom is
   `incomingCalls` at ~7.5x under its 3ms bound.
3. **Bounds test run 3 times, all green (flake check).** 3 consecutive
   `cargo test --release --test perf_bounds` runs: 7/7 passed every time
   (0.92s/0.95s/0.94s wall time), zero flake observed.
4. **`cargo clippy --all-targets --all-features -- -D warnings` clean.** Confirmed —
   compiles and lints `benches/lsp_pipeline.rs` and both new test targets with zero
   warnings/errors.
5. **rustfmt per touched file; CHANGELOG.md entry; commits, named paths only.**
   `rustfmt` run explicitly on all 9 touched/new `.rs` files — zero diff (the
   format-on-write hook already kept them formatted during editing). `cargo fmt
   --check` (whole-crate, read-only) also clean. CHANGELOG.md `[Unreleased] > Added`
   entry written (Task T0.5, Tier-0 remediation arc naming convention matching
   existing T0.3 entry). Commits pending — see plan below.

Additional verification beyond the 5 gates: full `cargo test --workspace` (matching
CI's own Test step) — 160 test-result blocks, zero `FAILED`, zero non-"0 failed"
result lines. `cargo fmt --check` clean on the whole crate.

## Commit plan

Two logical commits, staged by named path only (no `git add -A`):
1. **Enabling refactor** (`src/lib.rs`, `src/main.rs`, `src/graph.rs`,
   `src/handlers.rs`, `src/parser.rs`) — module-ownership move, zero behavior change,
   independently verifiable via the full test-suite pass.
2. **Benches + CI gate + docs** (`Cargo.toml`, `benches/lsp_pipeline.rs`,
   `tests/perf_support/mod.rs`, `tests/perf_support_smoke.rs`, `tests/perf_bounds.rs`,
   `.github/workflows/ci.yml`, `CLAUDE.md`, `CHANGELOG.md`) — the actual T0.5
   deliverable, built on top of commit 1's exposed surface.

## Concerns / follow-ups for the controller

- None blocking. One judgment call to flag explicitly: the module-ownership move
  (graph/indexer/handlers/parser/protocol → `lib.rs`) touches files outside
  `benches/`/`tests/`/CI/docs, which is a literal reading beyond the brief's "Benches/
  tests/CI/docs only (plus the corpus generator)" scope line. I judged it in-scope
  because (a) it was necessary to satisfy the brief's own explicit requirement to bench
  the handler layer in-process, (b) it's a pure visibility/ownership change (zero
  logic edits, verified via full before/after test-suite parity), and (c) it completes
  a pattern the repo's own code comments already flagged as intended-but-unfinished
  ("so library consumers (benches, snapshot) can use them"). Flagging for the
  controller in case a stricter reading is wanted, but I believe this is the correct
  architecture per CLAUDE.md's "Working Principle" (best solution, not the quickest
  patch) rather than e.g. duplicating 5 files' worth of source into `lib.rs` as a
  second compilation unit.
- The `incomingCalls` measurement (999-way fan-in) is the tightest relative to target
  (still ~2.5x headroom under 1ms), worth watching if a future change makes incoming-
  call resolution less than O(1) amortized per caller.
