# Test-Crate Consolidation Design

**Date:** 2026-07-15
**Goal:** Slash integration-test link count (151 → 12 targets) by consolidating
`tests/*.rs` files into ~9 umbrella test crates, cutting the full-suite
touch-relink time (currently 36.7 s with rust-lld after touching `src/lib.rs`).

## Motivation

The crate has 151 top-level `tests/*.rs` files, each its own crate + link
target. After any `src/` edit, `cargo test`/`cargo nextest run` re-links all
~155 binaries. Measured 2026-07-15 (dev profile, warm cache, touch
`src/lib.rs`): 36.7 s with rust-lld, 52.9 s with link.exe. Link count — not
link speed — is now the dominant lever: each target pays rustc front-end +
metadata + link overhead. Test *runtime* is unaffected by consolidation
(nextest parallelism is per-test, not per-binary).

## Approach (decided)

**Physical umbrella directories.** Cargo auto-discovers `tests/<dir>/main.rs`
as a single test target named `<dir>` (this is why `tests/common/` — no
`main.rs` — is already not a target). Each group's files `git mv` into a
subdirectory; a new `main.rs` holds only `mod` declarations. No `Cargo.toml`
changes; `autotests` stays default (a newly added `tests/*.rs` still runs —
no silent-skip footgun).

Rejected alternatives:
- `autotests = false` + `[[test]]` shims with `#[path]` re-includes: no file
  moves, but a new `tests/*.rs` would silently never run.
- Full helper dedup into a shared test-support crate (46× `repo_root`, 30×
  `goldens_dir` duplicates): touches ~100 files' internals; deferred as an
  optional follow-up. Consolidation does NOT require it — each moved file
  becomes its own module, so duplicate private helpers coexist fine.

## Grouping (151 files → 9 umbrellas + 3 standalone)

| Target | Members | Count |
|---|---|---|
| `gap` | `gap_*` | 27 |
| `cli` | `cli_*`, `al2dump_smoke`, `aldump_smoke`, `gate_*`, `d1_downgraded_to_info_oracle` | 23 |
| `l3` | `l3cg_*`, `l3cov_*`, `l3eg_*`, `l3rt_*`, `cross_app_l3_*`, `global_builtins_catalog` | 24 |
| `l2_ir` | `l2*`, `ir_*`, `encoder_vectors` | 12 |
| `temp_state` | `temp_state_*` | 13 |
| `r25_abi` | `r2_5a_*`, `r2_5b_*` | 14 |
| `r3` | `r3a*`, `r3b_*` | 22 |
| `r4` | `r4_differential`, `r4f_*` | 7 |
| `lsp` | `lsp_incremental_parity`, `program_graph`, `snapshot_robustness`, `telemetry_integration`, `telemetry_privacy_lint`, `perf_support_smoke` | 6 |
| standalone | `program_resolve_harness` (525 KB; cdo-gate runs it `--test-threads=1`), `perf_bounds` (CI timing gate — owns its process), `differential` (112 KB) | 3 |

Standalone rationale: `program_resolve_harness` and `perf_bounds` are invoked
by name in `scripts/cdo-gate` / `.github/workflows/ci.yml` and have special
execution requirements (serial threads; timing bounds that must not share a
process with unrelated load under libtest). `differential` is large enough to
be its own compile unit.

## Mechanics per umbrella

1. `git mv tests/<file>.rs tests/<umbrella>/<file>.rs` for each member.
2. Create `tests/<umbrella>/main.rs`: one `mod <file>;` per member (module
   name = file stem), nothing else — except:
   - If any member used `#[path = "common/cdo.rs"]`/`common/regen.rs`
     includes (41 files do today), `main.rs` hoists them once:
     `#[path = "../common/cdo.rs"] mod cdo_common;` (etc.), and members
     replace their include with `use crate::cdo_common::…`.
   - `telemetry_integration.rs`'s crate-level `#![cfg(feature = "telemetry")]`
     is deleted from the file and expressed as
     `#[cfg(feature = "telemetry")] mod telemetry_integration;` in `lsp`'s
     `main.rs` (inner attributes are illegal in non-root modules).
3. Member files are otherwise unchanged (their private duplicate helpers are
   module-scoped — no collisions, no forced dedup).

## Env-mutation hazard (cli umbrella)

`cli_a_html/json/terminal_differential.rs` call
`std::env::set_var("ALCH_DRIVER_VERSION_OVERRIDE", …)`/`remove_var`. This
already races TODAY under plain `cargo test` (libtest runs a file's tests on
multiple threads in one process); merging widens the blast radius. Fix as
part of the move: the cli umbrella's shared module gets
`pub static ENV_LOCK: Mutex<()>`, and every env-mutating test wraps
set→run→remove in the lock (poisoning-tolerant:
`lock().unwrap_or_else(PoisonError::into_inner)`). Safe under both nextest
(process-per-test, lock uncontended) and libtest (serialized).

## Script/CI/doc updates

- `scripts/cdo-gate`: `--test program_resolve_harness` line unchanged. The
  `--test program_graph --test snapshot_robustness` line becomes
  `cargo test --release --test lsp program_graph:: snapshot_robustness::`
  (libtest name-prefix filters; module names come from the file stems).
- `.github/workflows/ci.yml`: `--test perf_bounds` unchanged; audit for any
  other `--test <name>` references and update to umbrella + filter form.
- `CLAUDE.md` Testing section: note the umbrella layout and that
  `tests/common/*` sharing now happens via one hoisted `mod` per umbrella.
- `CHANGELOG.md`: Changed entry.

## Validation

- **Per-umbrella conservation gate:** before each move, record the group's
  test count (`cargo nextest list -E 'binary(<old names>)'` sum); after,
  `cargo nextest run -E 'binary(<umbrella>)'` must report the same count,
  zero failures. One commit per umbrella, each independently green.
- **Whole-suite:** total test count must remain 2,609 (current full-suite
  count); `cargo clippy --release --all-targets --all-features` clean
  (CI lints all test crates).
- **CDO gate** (`scripts/cdo-gate`) at the end — `program_resolve_harness`
  is unmoved but the gate's second line changed.

## Measurement (plain cargo, never nextest, per perf-measurement rule)

Re-run the A/B that produced the baseline: warm build, touch `src/lib.rs`,
time `cargo test --no-run`. Baseline 36.7 s (rust-lld). Record before/after
in `docs/perf-regression-t3-vs-0.9.3.md` (new subsection). Expected: ~140
fewer front-end+link invocations; report the measured number, whatever it is.
Also record the known trade-off honestly: editing one member file now
recompiles its whole umbrella (front-end is per-crate single-threaded), so
single-test iteration in the biggest umbrellas (`gap`, `l3`, `r3`) gets
somewhat slower — acceptable per the "balanced ~6-10 umbrellas" decision.

## Out of scope

- Helper dedup across members (follow-up; see Rejected alternatives).
- Any change to `benches/` (3 `harness = false` targets, unaffected).
- Merging the 3 standalone targets.
