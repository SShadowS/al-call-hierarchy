# Test-Crate Consolidation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Consolidate 151 top-level `tests/*.rs` integration-test crates into 9 umbrella test targets (+3 unchanged standalone), slashing link count and full-suite touch-relink time.

**Architecture:** Cargo auto-discovers `tests/<dir>/main.rs` as one test target named `<dir>`. Each group's files `git mv` into a subdirectory and become `mod`s of a tiny `main.rs`. Shared `tests/common/*.rs` `#[path]` includes hoist to one `mod` per umbrella. No `Cargo.toml` changes.

**Tech Stack:** Rust, cargo, cargo-nextest (validation only — NEVER for perf measurement), PowerShell (Windows).

**Spec:** `docs/superpowers/specs/2026-07-15-test-crate-consolidation-design.md`

## Global Constraints

- Branch: `feat/test-crate-consolidation` off `master`. Never merge/push without explicit user request.
- One commit per task, trailer: `Co-authored-by: Copilot <223556219+Copilot@users.noreply.github.com>`. Stage ONLY intended paths — never `git add -A`. NEVER stage `.cargo/`, `.config/`, `.panel/`, `demo-out/`, `scripts/finish-*.ps1`.
- Format touched .rs files with `rustfmt <file>` per file — NEVER `cargo fmt`.
- **Conservation gate (every umbrella task):** the SAME `cargo nextest list` regex expression must return the SAME test count before and after the move (the regex matches both the old binary names and the new umbrella name), and `cargo nextest run -E 'binary(<umbrella>)'` must be green. Run counts from repo root.
- Perf measurement uses plain `cargo` (`cargo test --no-run`), never nextest.
- Standalone targets `program_resolve_harness`, `perf_bounds`, `differential` are NOT moved.
- Total suite count must remain constant end-to-end (record the exact number in Task 1, re-verify in Task 11).

## Shared Recipe Details (referenced by every umbrella task — the task lists give the exact per-group inputs)

**R1 — count gate.** Before moving:
```powershell
(cargo nextest list -E "binary(/<REGEX>/)" 2>$null | Select-String '^\s{4}\S').Count
```
Record the number. After the move, re-run the SAME command — must be equal.

**R2 — move.** `git mv tests/<file>.rs tests/<umbrella>/<file>.rs` for each member (create the directory first: `New-Item -ItemType Directory tests/<umbrella>`).

**R3 — main.rs.** Create `tests/<umbrella>/main.rs` with exactly the content given in the task.

**R4 — common-include hoist.** For each member listed as a `regen`/`cdo` includer, delete these two lines from the member file:
```rust
#[path = "common/regen.rs"]
mod regen;
```
(or the `common/cdo.rs` / `mod cdo;` pair) and insert in their place:
```rust
use crate::regen;
```
(or `use crate::cdo;`). Leave any surrounding doc comments untouched. The umbrella's `main.rs` carries the single hoisted `#[path = "../common/regen.rs"] mod regen;` (and/or cdo) declaration.

**R5 — include_str fixup.** For each member listed as an `include_str!` user, prefix `../` inside every non-comment `include_str!("` / `include_bytes!("` argument (they are source-file-relative): `include_str!("r0-vectors/…")` → `include_str!("../r0-vectors/…")`. Then grep the whole umbrella dir for `include_str!("` without `../` to catch stragglers:
```powershell
Select-String -Path tests/<umbrella>/*.rs -Pattern 'include_(str|bytes)!\("(?!\.\./)' | Where-Object { $_.Line -notmatch '^\s*//' }
```
Expected: no matches.

**R6 — validate + commit.**
```powershell
cargo nextest run -E "binary(<umbrella>)"          # green, count per R1
rustfmt tests/<umbrella>/main.rs                    # plus any member you edited
git add tests/<umbrella>
git commit -m "refactor(tests): consolidate <group> into tests/<umbrella>/ umbrella crate"
```
(`git mv` already staged the renames; `git add tests/<umbrella>` picks up main.rs + edits.)

---

### Task 0: Branch + baseline

- [ ] **Step 1:** `git checkout master; git checkout -b feat/test-crate-consolidation`
- [ ] **Step 2:** Record the whole-suite baseline count and the link-time baseline:
```powershell
cargo nextest list 2>$null | Select-String '^\s{4}\S' | Measure-Object | % Count   # expect ~2609; record EXACT number
cargo test --no-run 2>$null | Out-Null                                             # warm
(Get-Item src\lib.rs).LastWriteTime = Get-Date
Measure-Command { cargo test --no-run 2>$null } | % TotalSeconds                    # record: touch-relink baseline (~36.7s)
```
Save both numbers into the task report (no commit needed).

### Task 1: `r4` umbrella (pilot — smallest group with the regen hoist)

**Files:** Move `r4_differential.rs`, `r4f_digest_effects.rs`, `r4f_ordering_facts.rs`, `r4f_return_summaries.rs`, `r4f_root_classifications.rs`, `r4f_scoped_guarantees.rs`, `r4f_snapshot.rs` → `tests/r4/`. Create `tests/r4/main.rs`.
**regen includers (R4):** ALL 7 members.
**include_str users (R5):** none (verify with the R5 grep anyway).

- [ ] **Step 1 (R1):** regex `^r4` — record count.
- [ ] **Step 2 (R2):** move the 7 files.
- [ ] **Step 3 (R3):** create `tests/r4/main.rs`:
```rust
//! Umbrella test crate: R4 effect-summary suites (test-crate consolidation,
//! 2026-07-15 spec). One link target instead of seven.

#[path = "../common/regen.rs"]
mod regen;

mod r4_differential;
mod r4f_digest_effects;
mod r4f_ordering_facts;
mod r4f_return_summaries;
mod r4f_root_classifications;
mod r4f_scoped_guarantees;
mod r4f_snapshot;
```
- [ ] **Step 4 (R4):** hoist the regen include in all 7 members.
- [ ] **Step 5 (R5, R6):** grep clean; `cargo nextest run -E "binary(r4)"` green with Step-1 count; rustfmt touched files; commit.

### Task 2: `gap` umbrella

**Files:** Move all 27 `gap_*.rs` files → `tests/gap/`: `gap_audit_b_table_triggers`, `gap_audit_d2_guards`, `gap_audit_d20_break`, `gap_audit_d22_implicit_rec`, `gap_audit_d29_run_trigger`, `gap_audit_d37_modifyall`, `gap_audit_d4`, `gap_audit_e_filter_load`, `gap_g1_next_terminator`, `gap_g2_runtime_temp`, `gap_g3_interproc_filter`, `gap_g4_transitive_wording`, `gap_g5_wrong_table_name`, `gap_g6_virtual_tables`, `gap_g7_dead_routine`, `gap_g8_residual_temp`, `gap_g9_trigger_rec`, `gap_g10_load_wrappers`, `gap_g11_d20_position`, `gap_g12_d3_refinements`, `gap_g13_temp_gate`, `gap_g14_onlookup_triggers`, `gap_g15_d3_d42_writes`, `gap_g16_deep_wrappers`, `gap_g17_d33_filters`, `gap_g18_transitive_loop`, `gap_g19_temp_param`.
**regen/cdo includers:** none. **include_str users:** none (R5 grep anyway).

- [ ] **Step 1 (R1):** regex `^gap` — record count.
- [ ] **Step 2 (R2, R3):** move; `tests/gap/main.rs` = doc comment (`//! Umbrella test crate: detector gap audits.`) + one `mod <stem>;` line per member in the order listed above.
- [ ] **Step 3 (R5, R6):** grep clean; `binary(gap)` green with count; commit.

### Task 3: `temp_state` umbrella

**Files:** Move all 13 `temp_state_*.rs` → `tests/temp_state/`: `temp_state_abi`, `temp_state_calcfields`, `temp_state_capture`, `temp_state_d1_path`, `temp_state_oracle`, `temp_state_page`, `temp_state_param_forwarding`, `temp_state_path`, `temp_state_promotion`, `temp_state_recordref`, `temp_state_shadowing`, `temp_state_substitution`, `temp_state_tabletype`.
**regen/cdo includers:** none. **include_str users:** none.

- [ ] **Step 1 (R1):** regex `^temp_state` — record count.
- [ ] **Step 2 (R2, R3):** move; `tests/temp_state/main.rs` = doc comment + 13 `mod` lines.
- [ ] **Step 3 (R5, R6):** grep clean; `binary(temp_state)` green with count; commit.

### Task 4: `l2_ir` umbrella

**Files:** Move 12 files → `tests/l2_ir/`: `encoder_vectors`, `ir_l2_snapshot`, `ir_lowering_audit`, `ir_robustness`, `l2_receiver_oracles`, `l2_vectors`, `l2cap_oracles`, `l2cap_vectors`, `l2cc_oracles`, `l2cc_vectors`, `l2order_oracles`, `l2order_vectors`.
**regen includers (R4):** `ir_l2_snapshot` only.
**include_str users (R5):** `encoder_vectors`, `l2_vectors`, `l2cap_vectors`, `l2cc_vectors`, `l2order_vectors`.

- [ ] **Step 1 (R1):** regex `^(encoder_vectors|ir_|l2)` — record count.
- [ ] **Step 2 (R2, R3):** move; `tests/l2_ir/main.rs` = doc comment + `#[path = "../common/regen.rs"] mod regen;` + 12 `mod` lines.
- [ ] **Step 3 (R4, R5, R6):** hoist regen in `ir_l2_snapshot`; `../`-prefix the 5 include_str users; grep clean; `binary(l2_ir)` green with count; commit.

### Task 5: `l3` umbrella

**Files:** Move 24 files → `tests/l3/`: `cross_app_l3_aldump_cli`, `cross_app_l3_poison`, `cross_app_l3_smoke`, `global_builtins_catalog`, `l3cg_call_result_dispatch`, `l3cg_currpage_dispatch`, `l3cg_extends_bare_dispatch`, `l3cg_framework_property_compound`, `l3cg_implicit_rec_dispatch`, `l3cg_member_builtins`, `l3cg_oracles`, `l3cg_page_part_dispatch`, `l3cg_record_dispatch`, `l3cg_report_dataitem_rec`, `l3cg_resolution_vectors`, `l3cg_scalar_vectors`, `l3cg_singleton_static_dispatch`, `l3cg_stats_smoke`, `l3cov_oracles`, `l3cov_vectors`, `l3eg_oracles`, `l3eg_vectors`, `l3rt_oracles`, `l3rt_vectors`.
**regen/cdo includers:** none.
**include_str users (R5):** `l3cg_resolution_vectors`, `l3cg_scalar_vectors`, `l3cov_vectors`, `l3eg_vectors`, `l3rt_vectors`.

- [ ] **Step 1 (R1):** regex `^(cross_app_l3|global_builtins_catalog|l3)` — record count.
- [ ] **Step 2 (R2, R3):** move; `tests/l3/main.rs` = doc comment + 24 `mod` lines.
- [ ] **Step 3 (R5, R6):** `../`-prefix the 5 include_str users; grep clean; `binary(l3)` green with count; commit.

### Task 6: `r25_abi` umbrella

**Files:** Move 14 files → `tests/r25_abi/`: `r2_5a_abi_native_vectors`, `r2_5a_aldump_cli`, `r2_5a_attr_vectors`, `r2_5a_differential`, `r2_5a_oracles`, `r2_5a_stable_id_vectors`, `r2_5b_cg_differential`, `r2_5b_cg_oracles`, `r2_5b_cov_differential`, `r2_5b_cov_oracles`, `r2_5b_eg_differential`, `r2_5b_eg_oracles`, `r2_5b_rt_differential`, `r2_5b_rt_oracles`.
**regen includers (R4):** `r2_5a_differential`, `r2_5b_cg_differential`, `r2_5b_cov_differential`, `r2_5b_eg_differential`, `r2_5b_rt_differential`.
**include_str users (R5):** `r2_5a_abi_native_vectors`, `r2_5a_attr_vectors`, `r2_5a_stable_id_vectors`.

- [ ] **Step 1 (R1):** regex `^r2_5|^r25_abi$` — record count.
- [ ] **Step 2 (R2, R3):** move; `tests/r25_abi/main.rs` = doc comment + `#[path = "../common/regen.rs"] mod regen;` + 14 `mod` lines.
- [ ] **Step 3 (R4, R5, R6):** hoist regen ×5; `../`-prefix include_str ×3; grep clean; `binary(r25_abi)` green with count; commit.

### Task 7: `r3` umbrella

**Files:** Move 22 files → `tests/r3/`: `r3a0_unfetched_dep_opaque`, `r3a1_differential`, `r3a1_oracles`, `r3a1_vectors`, `r3a2_branch_aware`, `r3a2_differential`, `r3a2_oracles`, `r3a2_trace_differential`, `r3a2_trace_vectors`, `r3a2_vectors`, `r3a3_differential`, `r3a3_oracles`, `r3a3_vectors`, `r3a4_differential`, `r3a4_oracles`, `r3a4_vectors`, `r3a5_differential`, `r3a5_oracles`, `r3b_incremental_equality`, `r3b_incremental_nondeterminism`, `r3b_minimality`, `r3b_wrapped_parity`.
**regen includers (R4):** `r3a1_differential`, `r3a2_differential`, `r3a2_trace_differential`, `r3a3_differential`, `r3a4_differential`, `r3a5_differential`.
**include_str users (R5):** `r3a1_vectors`, `r3a2_trace_vectors`, `r3a2_vectors`, `r3a3_vectors`, `r3a4_vectors`.

- [ ] **Step 1 (R1):** regex `^r3` — record count.
- [ ] **Step 2 (R2, R3):** move; `tests/r3/main.rs` = doc comment + `#[path = "../common/regen.rs"] mod regen;` + 22 `mod` lines.
- [ ] **Step 3 (R4, R5, R6):** hoist regen ×6; `../`-prefix include_str ×5; grep clean; `binary(r3)` green with count; commit.

### Task 8: `cli` umbrella (env-lock hardening included)

**Files:** Move 23 files → `tests/cli/`: `al2dump_smoke`, `aldump_smoke`, `cli_a_html_differential`, `cli_a_json_differential`, `cli_a_stats_differential`, `cli_a_terminal_differential`, `cli_a_with_evidence`, `cli_b_diff_differential`, `cli_b_digest_differential`, `cli_b_digest_exit_oracles`, `cli_b_fingerprint_differential`, `cli_b_fingerprint_oracles`, `cli_b_prove_differential`, `cli_b_snapshot_differential`, `cli_c_cache_differential`, `cli_c_events_differential`, `cli_c_policy_differential`, `cli_p1_enclosing_member`, `cli_p1_inventory`, `d1_downgraded_to_info_oracle`, `gate_prsummary_differential`, `gate_sarif_differential`, `gate_suppress_baseline_differential`.
**regen includers (R4):** `al2dump_smoke`, `aldump_smoke`, `cli_a_html_differential`, `cli_a_json_differential`, `cli_a_stats_differential`, `cli_a_terminal_differential`, `cli_b_diff_differential`, `cli_b_digest_differential`, `cli_b_fingerprint_differential`, `cli_b_prove_differential`, `cli_b_snapshot_differential`, `cli_c_cache_differential`, `cli_c_events_differential`, `cli_c_policy_differential`, `gate_prsummary_differential`, `gate_sarif_differential`, `gate_suppress_baseline_differential` (17).
**include_str users:** none (R5 grep anyway).

- [ ] **Step 1 (R1):** regex `^(cli|al2dump_smoke|aldump_smoke|gate_|d1_downgraded)` — record count.
- [ ] **Step 2 (R2, R3):** move; create `tests/cli/main.rs`:
```rust
//! Umbrella test crate: CLI/gate differential suites (test-crate
//! consolidation, 2026-07-15 spec).
//!
//! `ENV_LOCK` serializes the process-global `std::env` mutation in the
//! `cli_a_*` differentials (`ALCH_DRIVER_VERSION_OVERRIDE`). Under nextest
//! (process-per-test) the lock is uncontended; under plain `cargo test`
//! (libtest threads share this process) it is what makes those tests sound —
//! they raced even as separate files whenever one file ran multi-threaded.

use std::sync::{Mutex, MutexGuard, PoisonError};

#[path = "../common/regen.rs"]
mod regen;

pub static ENV_LOCK: Mutex<()> = Mutex::new(());

/// Hold this for the entire set-var → run → remove-var span of any test that
/// mutates process env. Poisoning-tolerant: a panicked holder must not
/// cascade-fail unrelated tests.
pub fn env_guard() -> MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(PoisonError::into_inner)
}

mod al2dump_smoke;
mod aldump_smoke;
mod cli_a_html_differential;
mod cli_a_json_differential;
mod cli_a_stats_differential;
mod cli_a_terminal_differential;
mod cli_a_with_evidence;
mod cli_b_diff_differential;
mod cli_b_digest_differential;
mod cli_b_digest_exit_oracles;
mod cli_b_fingerprint_differential;
mod cli_b_fingerprint_oracles;
mod cli_b_prove_differential;
mod cli_b_snapshot_differential;
mod cli_c_cache_differential;
mod cli_c_events_differential;
mod cli_c_policy_differential;
mod cli_p1_enclosing_member;
mod cli_p1_inventory;
mod d1_downgraded_to_info_oracle;
mod gate_prsummary_differential;
mod gate_sarif_differential;
mod gate_suppress_baseline_differential;
```
- [ ] **Step 3 (R4):** hoist regen in all 17 listed members.
- [ ] **Step 4 (env lock):** find every `#[test]` fn that calls `std::env::set_var`/`remove_var`:
```powershell
Select-String -Path tests/cli/cli_a_html_differential.rs,tests/cli/cli_a_json_differential.rs,tests/cli/cli_a_terminal_differential.rs -Pattern 'set_var|remove_var'
```
In each such test fn, insert as the FIRST statement:
```rust
let _env = crate::env_guard();
```
If the set/remove happens inside a non-test helper fn called by multiple tests, put the guard in the CALLING test fns (one guard per test, held for the whole test body) — never inside the helper (re-entrancy deadlock).
- [ ] **Step 5 (R5, R6):** grep clean; `cargo nextest run -E "binary(cli)"` green with Step-1 count; ALSO run the merged env-mutating tests under libtest to prove the lock works multi-threaded:
```powershell
cargo test --test cli cli_a_ 2>&1 | Select-String 'test result'
```
Expected: `ok`, 0 failed. rustfmt touched files; commit.

### Task 9: `lsp` umbrella (cdo hoist + telemetry cfg)

**Files:** Move 6 files → `tests/lsp/`: `lsp_incremental_parity`, `perf_support_smoke`, `program_graph`, `snapshot_robustness`, `telemetry_integration`, `telemetry_privacy_lint`.
**cdo includers (R4, cdo variant):** `program_graph`, `snapshot_robustness`.
**include_str users:** none (R5 grep anyway).

- [ ] **Step 1 (R1):** regex `^(lsp|perf_support_smoke|program_graph|snapshot_robustness|telemetry_)` — record count. NOTE: `lsp_incremental_parity` contains CDO-gated tests that silently skip without `CDO_WS` — the count is stable either way as long as Step 1 and validation run with the same environment (do NOT set `CDO_WS` for the gate).
- [ ] **Step 2 (R2):** move the 6 files.
- [ ] **Step 3:** delete line `#![cfg(feature = "telemetry")]` from `tests/lsp/telemetry_integration.rs` (inner attributes are illegal in non-root modules).
- [ ] **Step 4 (R3):** create `tests/lsp/main.rs`:
```rust
//! Umbrella test crate: LSP-surface + telemetry suites (test-crate
//! consolidation, 2026-07-15 spec).

#[path = "../common/cdo.rs"]
mod cdo;

mod lsp_incremental_parity;
mod perf_support_smoke;
mod program_graph;
mod snapshot_robustness;
// Was a crate-level `#![cfg(feature = "telemetry")]` when this file was its
// own crate; expressed here because inner attributes can't live in a
// non-root module.
#[cfg(feature = "telemetry")]
mod telemetry_integration;
mod telemetry_privacy_lint;
```
- [ ] **Step 5 (R4):** hoist the cdo include (`#[path = "common/cdo.rs"] mod cdo;` → `use crate::cdo;`) in `program_graph.rs` and `snapshot_robustness.rs`.
- [ ] **Step 6 (R5, R6):** grep clean; `binary(lsp)` green with count; additionally `cargo nextest run -E "binary(lsp)" --features telemetry` green (proves the cfg hoist); commit.

### Task 10: Scripts, CI, docs

**Files:** Modify `scripts/cdo-gate`, `.github/workflows/ci.yml` (audit), `CLAUDE.md`, `CHANGELOG.md`.

- [ ] **Step 1:** In `scripts/cdo-gate`, replace the line
```bash
if ! cargo test --release --test program_graph --test snapshot_robustness; then
```
with
```bash
if ! cargo test --release --test lsp -- program_graph:: snapshot_robustness::; then
```
(libtest accepts multiple positional filters, OR-ed; module-qualified names come from the file stems).
- [ ] **Step 2:** Audit `.github/workflows/ci.yml` and all of `scripts/` for any other `--test <old-name>` reference: `Select-String -Path .github/workflows/*.yml,scripts/* -Pattern '--test\s+\S+'`. Expected survivors: `program_resolve_harness` (cdo-gate) and `perf_bounds` (ci.yml) — both unmoved. Fix anything else to umbrella+filter form.
- [ ] **Step 3:** Update `CLAUDE.md`'s **Testing** paragraph: after the sentence about `tests/common/{cdo,regen}.rs`, note that integration tests are consolidated into umbrella crates (`tests/{gap,cli,l3,l2_ir,temp_state,r25_abi,r3,r4,lsp}/main.rs`, one link target each; `program_resolve_harness`/`perf_bounds`/`differential` remain standalone), and that each umbrella's `main.rs` hoists the common includes once (members `use crate::cdo`/`crate::regen`).
- [ ] **Step 4:** Add a `### Changed` CHANGELOG entry under `[Unreleased]` describing the consolidation (151 → 12 test link targets, env-lock hardening for the `cli_a_*` differentials, telemetry cfg hoist, cdo-gate invocation change). Numbers from Task 11 get appended there in Task 11.
- [ ] **Step 5:** Commit (`git add scripts/cdo-gate .github/workflows/ci.yml CLAUDE.md CHANGELOG.md`).

### Task 11: Whole-suite validation + measurement + close-out

- [ ] **Step 1 (conservation, whole suite):** `cargo nextest list 2>$null | Select-String '^\s{4}\S' | Measure-Object | % Count` — must equal Task 0's exact baseline. Then `cargo nextest run` — all green.
- [ ] **Step 2 (lint):** `cargo clippy --release --all-targets --all-features` — clean (CI lints all test crates in release).
- [ ] **Step 3 (measure, plain cargo — never nextest):**
```powershell
cargo test --no-run 2>$null | Out-Null                       # warm
(Get-Item src\lib.rs).LastWriteTime = Get-Date
Measure-Command { cargo test --no-run 2>$null } | % TotalSeconds
```
Run 3 times (touch before each); report the median vs Task 0's baseline (36.7 s).
- [ ] **Step 4 (CDO gate):** run `scripts/cdo-gate`'s commands manually (PowerShell; superpowers bash scripts don't run on Windows):
```powershell
$env:CDO_WS='u:\Git\DO.Support-SlowDOSetup\DocumentOutput\Cloud'; $env:ENFORCE_CDO_WS='1'
cargo test --release --test program_resolve_harness -- --test-threads=1     # expect 187 passed
cargo test --release --test lsp -- program_graph:: snapshot_robustness::    # expect 2 passed
```
- [ ] **Step 5:** Append measured numbers to the Task-10 CHANGELOG entry and add a short subsection to `docs/perf-regression-t3-vs-0.9.3.md` (baseline 36.7 s → measured; methodology: warm dev build, touch `src/lib.rs`, `cargo test --no-run`, rust-lld, median of 3; note the trade-off: single-member edits now recompile the whole umbrella). Update `.superpowers/sdd/progress.md`. Commit docs.

---

## Self-Review Notes

- Spec coverage: grouping table (Tasks 1–9, all 151 files accounted: 7+27+13+12+24+14+22+23+6=148 moved + 3 standalone), mechanics (R2–R4), env hazard (Task 8 Step 4), telemetry cfg (Task 9 Step 3), include_str (R5 — spec listed it under validation risk), scripts/CI/docs (Task 10), validation+measurement (Tasks 0, 11). ✓
- Module/binary name collisions: no existing `tests/<name>.rs` matches an umbrella name (verified 2026-07-15). File stems are all valid Rust identifiers. ✓
- The R1 regexes match old AND new binary names (`^r4` matches `r4_differential`…`r4f_*` and umbrella `r4`) — except Task 6, whose alternation covers the rename (`r2_5*` → `r25_abi`), and Tasks 4/5/8/9, whose alternations include the umbrella name via a member prefix (`l2`→`l2_ir` ✓, `l3`→`l3` ✓, `cli`→`cli` ✓, `lsp`→`lsp` ✓). Verified each. ✓
- `differential.rs` (standalone) also matches no umbrella regex (`^(cli|…)` doesn't match `differential`; `^gap`/`^r3`/… don't either). `d1_downgraded` alternation is specific enough to exclude nothing else. ✓
