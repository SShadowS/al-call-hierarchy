# Preflight on the Fresh Resolver — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `alsem analyze`'s preflight warning/exit gate re-keys from the legacy L3
no-deps coverage to the authoritative fresh resolver, killing the misleading
"1045 unresolved callsite(s)" warning on DO and closing the fail-closed
silent-clean hole.

**Architecture:** A new `FreshCoverage` status struct + `fresh_coverage(ws)` entry on
the program-engine side (`src/program/resolve/full.rs`, reusing `build_context` /
`resolve_full_program_with`), consumed by a rewritten `evaluate_preflight` with a
first-class could-not-verify state. `gate/run.rs` computes it FIRST and drops the
`ProgramContext` before L3 assembly (memory sequencing). Formatter-visible
`opaqueApps` follows the fresh snapshot.

**Tech Stack:** Rust; existing program engine + gate; cargo test / nextest.

**Spec:** `docs/superpowers/specs/2026-07-17-preflight-fresh-coverage-design.md`
(2-round reviewed). Read it before starting.

## Global Constraints

- Format touched files with `rustfmt <file>` — NEVER `cargo fmt`.
- `cargo clippy --all-targets --all-features` clean after every task.
- CHANGELOG.md updated (Task 6, under **Changed** — CI-visible exit-code re-key).
- Stage in logical groups; never `git add -A`. Never push/merge to master unrequested.
- Goldens: `scripts/check-goldens --regen` then INSPECT the diff, then
  `scripts/check-goldens` green (the pre-commit hook enforces on gate/test paths;
  note the hook keys on detector/fixture/golden paths — run check-goldens manually
  when only `src/engine/gate/` changed).
- Package name is `al-call-hierarchy` (HYPHEN) in `-p` args.
- Do NOT pipe long test/gate runs through `| tail` (masks exit code); redirect to a
  log or use separate commands.
- Message strings are exact contracts (tests pin them):
  - clean: `resolution coverage verified`
  - degraded prefix: `analysis coverage degraded — ` + comma-joined clauses in fixed
    order: `{n} unknown resolution edge(s)` → `coverage contract violated` →
    `{n} recovered file(s)` → `{n} symbol-only dependency app(s): {names}`
  - could-not-verify: `coverage could not be verified: {reason}`
  - fail-closed fallback reason: `workspace contained no readable AL source units`

## Substrate reference (verified 2026-07-17, master 0adb3ec)

- `src/program/resolve/full.rs`: `resolve_full_program(ws) -> Option<ProgramReport>`
  (:920) = `build_context(ws)?` + `resolve_full_program_with(&ctx)` (:931).
  `build_context(ws) -> Option<ProgramContext>` (:1062) erases errors via
  `.build().ok()?` and `graph.apps.find(...)?`. `ProgramReport` (:177) already has
  `primary_histogram: Histogram`, `coverage: Coverage`, `recovered_files: Vec<String>`.
  Helper `coverage_holds(&Coverage) -> bool` exists (:221 area).
- `Histogram.unknown: usize` (`src/program/resolve/edge.rs:686`).
- `AppSetSnapshot { apps: Vec<AppUnit> /* [0] = workspace */, workspace_app: AppId, world }`
  (`src/snapshot/snapshot.rs:59`). `AppUnit { id: AppId, source: Option<SourceRoot>
  /* None = symbol-only */, declared_deps: Vec<AppDependency>, .. }` (:33).
  `AppId { guid, name, publisher, version }` (`src/snapshot/identity.rs:5`).
  `AppDependency.app_id: String` = dep GUID (`src/dependencies.rs:14`).
- `src/engine/gate/run.rs`: L3 assembly early-return call sites `empty_output_result(args, &version)`
  at :201 and :206; `let coverage = resolved.project_coverage_disk(ws_path);` at :358;
  `evaluate_preflight(coverage.unresolved_callsites.len(), &coverage.opaque_apps,
  args.require_dependencies)` at :426; exit gate at :441
  (`exit::PREFLIGHT_FAILED` > findings > clean); `empty_output_result` (:466) builds an
  all-empty `AnalysisCoverage` and unconditionally returns `Ok((out, exit::CLEAN, None))`.
- `src/engine/gate/preflight.rs`: current `evaluate_preflight(usize, &[String], bool)
  -> PreflightResult`. `PreflightResult` has NO consumers of its
  `unresolved_callsites`/`opaque_apps` fields outside preflight.rs itself (grepped) —
  safe to reshape. run.rs uses only `.degraded/.failed/.message`.
- Test pins to rebaseline (`tests/cli/gate_prsummary_differential.rs`):
  `anti_degenerate_preflight_exit_four` (:586), warning-substring oracle
  `"unresolved callsite"` (:636), `oracle_exit_precedence_preflight_wins_over_findings`
  (:653); golden matrix `tests/gate-goldens/exit-codes.json`.
- Fixtures: `tests/r0-corpus/ws-baseapp-closure/` has a REAL symbol-only `.app`
  (`Microsoft_Base Application_24.0.0.0.app` in `.alpackages/`) — the opaque-warning
  fixture. `tests/r0-corpus/ws-e2e/` is the neutral no-deps fixture (clean case).
- Formatters read `AnalysisCoverage.opaque_apps` (json :305+, terminal :314/:449,
  html coverage line) — the run.rs override (Task 4) reaches all three without
  touching formatter code.

## File Structure

```
src/program/resolve/full.rs        # Task 1: build_context_res; Task 2: FreshCoverage,
                                   #   fresh_coverage(), opaque_dependency_closure()
src/engine/gate/preflight.rs       # Task 3: rewrite (new states, wording, tests)
src/engine/gate/run.rs             # Task 4: fresh-first wiring + opaque override;
                                   #   Task 5: empty_output_result could-not-verify
tests/cli/gate_prsummary_differential.rs  # Tasks 4-5: rebaseline pins + new tests
tests/gate-goldens/exit-codes.json # Task 4: regen (deliberate)
CHANGELOG.md                       # Task 6 (Changed)
docs/OUTSTANDING.md                # Task 6: tick the item
```

---

### Task 1: `build_context_res` — Result-returning context builder

Today `build_context` erases the snapshot-build error (`.build().ok()?`) and the
primary-app lookup failure (`?` on Option). The spec requires the real error text in
the could-not-verify message. Pure refactor: add the `Result` variant, delegate the
`Option` one.

**Files:**
- Modify: `src/program/resolve/full.rs` (:1062 `build_context`)
- Test: colocated `#[cfg(test)]` in `full.rs` (grep `mod tests` — add there, or create
  one at file end following crate style)

**Interfaces:**
- Produces: `pub fn build_context_res(workspace_root: &Path) -> Result<ProgramContext, String>`
  — same behavior as `build_context`, errors carry text. `build_context` becomes
  `build_context_res(ws).ok()` (behavior-preserving). Consumed by Task 2.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn build_context_res_preserves_error_text_for_missing_workspace() {
    let err = build_context_res(std::path::Path::new(
        "Z:/definitely/not/a/workspace/xyzzy",
    ))
    .expect_err("nonexistent workspace must be Err, not Ok");
    assert!(!err.is_empty(), "error text must be non-empty");
}

#[test]
fn build_context_matches_res_variant_on_success() {
    // Any committed small fixture workspace works; ws-e2e is the neutral one.
    let ws = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/r0-corpus/ws-e2e");
    assert!(build_context_res(&ws).is_ok());
    assert!(build_context(&ws).is_some());
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p al-call-hierarchy --lib build_context_res`
Expected: FAIL to compile — `build_context_res` not defined.

- [ ] **Step 3: Implement**

Rename the body of `build_context` to `build_context_res`, converting the two erasure
points (everything else verbatim):

```rust
pub fn build_context_res(workspace_root: &Path) -> Result<ProgramContext, String> {
    let snap = (SnapshotBuilder {
        workspace_root: workspace_root.to_path_buf(),
        local_providers: vec![],
    })
    .build()
    .map_err(|e| format!("snapshot build failed: {e:#}"))?;
    // ... existing body unchanged ...
    let primary_app_ref = graph
        .apps
        .find(&snap.workspace_app)
        .ok_or_else(|| {
            format!(
                "workspace app '{}' not present in the assembled program graph",
                snap.workspace_app.name
            )
        })?;
    Ok(ProgramContext { snap, graph, parsed, primary_app_ref, ws_file_set, dep_layer })
}

#[must_use]
pub fn build_context(workspace_root: &Path) -> Option<ProgramContext> {
    build_context_res(workspace_root).ok()
}
```

(If `SnapshotBuilder::build`'s error type isn't `anyhow::Error`, adjust the
`map_err` format to whatever `Display` it has — the requirement is only that the
underlying message survives.)

- [ ] **Step 4: Run to verify pass + no regression**

Run: `cargo test -p al-call-hierarchy --lib build_context` → PASS
Run: `cargo test --test program_resolve_harness -- --test-threads=1` → PASS (pure refactor)

- [ ] **Step 5: Format, lint, commit**

```bash
rustfmt src/program/resolve/full.rs
cargo clippy --all-targets --all-features
git add src/program/resolve/full.rs
git commit -m "refactor(resolve): build_context_res preserves snapshot/build error text"
```

---

### Task 2: `FreshCoverage` + `fresh_coverage(ws)` + opaque closure

**Files:**
- Modify: `src/program/resolve/full.rs`
- Test: colocated `#[cfg(test)]` (same module as Task 1's tests)

**Interfaces:**
- Consumes: `build_context_res` (Task 1), `resolve_full_program_with`,
  `coverage_holds`, `ProgramReport.{primary_histogram, coverage, recovered_files}`.
- Produces (consumed by Tasks 3-5):

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FreshCoverage {
    pub unknown: usize,
    pub coverage_holds: bool,
    pub recovered_files: usize,
    /// Symbol-only dep app NAMES in the primary app's reachable declared-dep
    /// closure (primary excluded), deduped, sorted (name, then guid).
    pub opaque_apps: Vec<String>,
}
pub fn fresh_coverage(workspace_root: &Path) -> Result<FreshCoverage, String>
```

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn fresh_coverage_matches_direct_resolve_on_neutral_fixture() {
    let ws = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/r0-corpus/ws-e2e");
    let fc = fresh_coverage(&ws).expect("neutral fixture resolves");
    let report = resolve_full_program(&ws).expect("same fixture");
    assert_eq!(fc.unknown, report.primary_histogram.unknown);
    assert_eq!(fc.coverage_holds, coverage_holds(&report.coverage));
    assert_eq!(fc.recovered_files, report.recovered_files.len());
    assert!(fc.opaque_apps.is_empty(), "ws-e2e has no dependencies");
}

#[test]
fn fresh_coverage_reports_symbol_only_dep_in_closure() {
    let ws = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/r0-corpus/ws-baseapp-closure");
    let fc = fresh_coverage(&ws).expect("fixture resolves");
    // The committed Microsoft Base Application .app is symbol-only (no embedded
    // source) and declared by the fixture's app.json — it must appear by NAME.
    assert!(
        fc.opaque_apps.iter().any(|n| n.contains("Base Application")),
        "opaque_apps = {:?}",
        fc.opaque_apps
    );
    // The primary app itself must never be listed.
    assert!(!fc.opaque_apps.iter().any(|n| n.is_empty()));
}

#[test]
fn fresh_coverage_err_on_missing_workspace() {
    assert!(fresh_coverage(std::path::Path::new("Z:/no/such/ws")).is_err());
}
```

NOTE: if the Base Application `.app` in that fixture turns out to carry embedded
source (assert fails with empty list), pick another committed `.alpackages` fixture
whose dep is genuinely symbol-only — `grep -rl "alpackages" tests/r0-corpus` and
check with the snapshot (`AppUnit::source == None`). Do NOT weaken the assertion.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p al-call-hierarchy --lib fresh_coverage`
Expected: FAIL to compile.

- [ ] **Step 3: Implement**

```rust
/// Symbol-only dep app names in the primary app's reachable declared-dependency
/// closure. BFS over `AppUnit.declared_deps` GUIDs starting at the workspace app;
/// the snapshot may contain UNRELATED cached packages (`load_all_apps` loads every
/// `.app` in ancestor `.alpackages` without app.json filtering), so an unscoped
/// scan would report noise — and under `--require-dependencies` flip exit 4 on it.
fn opaque_dependency_closure(snap: &AppSetSnapshot) -> Vec<String> {
    use std::collections::{HashMap, HashSet, VecDeque};
    let by_guid: HashMap<String, &AppUnit> = snap
        .apps
        .iter()
        .map(|u| (u.id.guid.to_ascii_lowercase(), u))
        .collect();
    let primary_guid = snap.workspace_app.guid.to_ascii_lowercase();
    let mut seen: HashSet<String> = HashSet::new();
    let mut queue: VecDeque<&AppUnit> = VecDeque::new();
    if let Some(primary) = by_guid.get(&primary_guid) {
        seen.insert(primary_guid.clone());
        queue.push_back(primary);
    }
    let mut opaque: Vec<(String, String)> = Vec::new(); // (name, guid) for stable sort
    while let Some(unit) = queue.pop_front() {
        for dep in &unit.declared_deps {
            let guid = dep.app_id.to_ascii_lowercase();
            if !seen.insert(guid.clone()) {
                continue;
            }
            if let Some(u) = by_guid.get(&guid) {
                if u.source.is_none() {
                    opaque.push((u.id.name.clone(), u.id.guid.clone()));
                }
                queue.push_back(u);
            }
            // A declared dep ABSENT from the snapshot is a real gap, but
            // reporting it is an explicit spec follow-up (OUTSTANDING.md) —
            // not silently widened here.
        }
    }
    opaque.sort();
    opaque.dedup();
    opaque.into_iter().map(|(name, _)| name).collect()
}

pub fn fresh_coverage(workspace_root: &Path) -> Result<FreshCoverage, String> {
    let ctx = build_context_res(workspace_root)?;
    let report = resolve_full_program_with(&ctx);
    let opaque_apps = opaque_dependency_closure(&ctx.snap);
    Ok(FreshCoverage {
        unknown: report.primary_histogram.unknown,
        coverage_holds: coverage_holds(&report.coverage),
        recovered_files: report.recovered_files.len(),
        opaque_apps,
    })
    // ctx (snapshot + graph + parsed) drops HERE — callers hold only the tiny
    // status struct, never the whole semantic model (spec §3 memory sequencing).
}
```

Add the `FreshCoverage` struct (doc comments per the spec's field docs, including
the closure-scoping rationale).

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p al-call-hierarchy --lib fresh_coverage` → PASS (all 3)

- [ ] **Step 5: Format, lint, commit**

```bash
rustfmt src/program/resolve/full.rs
cargo clippy --all-targets --all-features
git add src/program/resolve/full.rs
git commit -m "feat(resolve): FreshCoverage status entry (closure-scoped opaque deps)"
```

---

### Task 3: rewrite `evaluate_preflight`

**Files:**
- Modify: `src/engine/gate/preflight.rs` (full rewrite incl. tests)

**Interfaces:**
- Consumes: `crate::program::resolve::full::FreshCoverage` (Task 2).
- Produces (consumed by Tasks 4-5):

```rust
pub struct PreflightResult {
    pub degraded: bool,
    pub failed: bool,
    pub message: String,
    pub unknown_edges: usize,
    pub opaque_apps: Vec<String>,
    pub verify_error: Option<String>,
}
pub fn evaluate_preflight(
    fresh: &Result<FreshCoverage, String>,
    required: bool,
) -> PreflightResult
```

NOTE: run.rs:426 still calls the OLD signature — this task leaves the crate
temporarily red at that one call site is NOT acceptable; instead, keep the old
function compiling by doing Task 3 and the run.rs call-site switch (Task 4 Step 3a)
in the same commit IF needed. Preferred: implement the new function alongside as
`evaluate_preflight` REPLACING the old one, and fix run.rs:426-430 minimally in this
task (mechanical: pass `&fresh` once Task 4 introduces it — see Step 3 note below).

- [ ] **Step 1: Write the failing tests** (replace the existing 3 tests)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::program::resolve::full::FreshCoverage;

    fn clean() -> FreshCoverage {
        FreshCoverage { unknown: 0, coverage_holds: true, recovered_files: 0, opaque_apps: vec![] }
    }

    #[test]
    fn clean_verified() {
        let pf = evaluate_preflight(&Ok(clean()), true);
        assert!(!pf.degraded && !pf.failed);
        assert_eq!(pf.message, "resolution coverage verified");
    }

    #[test]
    fn unknown_edges_degrade_failed_only_when_required() {
        let fc = FreshCoverage { unknown: 3, ..clean() };
        let pf = evaluate_preflight(&Ok(fc.clone()), false);
        assert!(pf.degraded && !pf.failed);
        assert_eq!(pf.message, "analysis coverage degraded — 3 unknown resolution edge(s)");
        let pf = evaluate_preflight(&Ok(fc), true);
        assert!(pf.failed);
    }

    #[test]
    fn contract_violation_never_reports_clean() {
        let fc = FreshCoverage { coverage_holds: false, ..clean() };
        let pf = evaluate_preflight(&Ok(fc), false);
        assert!(pf.degraded);
        assert_eq!(pf.message, "analysis coverage degraded — coverage contract violated");
    }

    #[test]
    fn recovered_files_degrade() {
        let fc = FreshCoverage { recovered_files: 2, ..clean() };
        let pf = evaluate_preflight(&Ok(fc), false);
        assert_eq!(pf.message, "analysis coverage degraded — 2 recovered file(s)");
    }

    #[test]
    fn opaque_only_degrades_with_sorted_names() {
        let fc = FreshCoverage { opaque_apps: vec!["A App".into(), "B App".into()], ..clean() };
        let pf = evaluate_preflight(&Ok(fc), true);
        assert!(pf.degraded && pf.failed);
        assert_eq!(
            pf.message,
            "analysis coverage degraded — 2 symbol-only dependency app(s): A App, B App"
        );
    }

    #[test]
    fn all_signals_retained_in_fixed_order() {
        let fc = FreshCoverage {
            unknown: 1,
            coverage_holds: false,
            recovered_files: 2,
            opaque_apps: vec!["Dep".into()],
        };
        let pf = evaluate_preflight(&Ok(fc), false);
        assert_eq!(
            pf.message,
            "analysis coverage degraded — 1 unknown resolution edge(s), \
             coverage contract violated, 2 recovered file(s), \
             1 symbol-only dependency app(s): Dep"
        );
    }

    #[test]
    fn could_not_verify_is_first_class_and_never_silent() {
        let pf = evaluate_preflight(&Err("snapshot build failed: boom".into()), false);
        assert!(pf.degraded && !pf.failed);
        assert_eq!(pf.message, "coverage could not be verified: snapshot build failed: boom");
        assert_eq!(pf.verify_error.as_deref(), Some("snapshot build failed: boom"));
        let pf = evaluate_preflight(&Err("boom".into()), true);
        assert!(pf.failed);
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p al-call-hierarchy --lib preflight`
Expected: FAIL to compile (new signature/fields).

- [ ] **Step 3: Implement**

```rust
pub fn evaluate_preflight(
    fresh: &Result<FreshCoverage, String>,
    required: bool,
) -> PreflightResult {
    match fresh {
        Err(e) => PreflightResult {
            degraded: true,
            failed: required,
            message: format!("coverage could not be verified: {e}"),
            unknown_edges: 0,
            opaque_apps: vec![],
            verify_error: Some(e.clone()),
        },
        Ok(fc) => {
            let mut clauses: Vec<String> = Vec::new();
            if fc.unknown > 0 {
                clauses.push(format!("{} unknown resolution edge(s)", fc.unknown));
            }
            if !fc.coverage_holds {
                clauses.push("coverage contract violated".to_string());
            }
            if fc.recovered_files > 0 {
                clauses.push(format!("{} recovered file(s)", fc.recovered_files));
            }
            if !fc.opaque_apps.is_empty() {
                clauses.push(format!(
                    "{} symbol-only dependency app(s): {}",
                    fc.opaque_apps.len(),
                    fc.opaque_apps.join(", ")
                ));
            }
            let degraded = !clauses.is_empty();
            let message = if degraded {
                format!("analysis coverage degraded — {}", clauses.join(", "))
            } else {
                "resolution coverage verified".to_string()
            };
            PreflightResult {
                degraded,
                failed: degraded && required,
                message,
                unknown_edges: fc.unknown,
                opaque_apps: fc.opaque_apps.clone(),
                verify_error: None,
            }
        }
    }
}
```

Update the module doc (`preflight.rs:1-9`): degraded now = fresh unknown edges /
contract violation / recovered files / symbol-only deps / verification failure;
cite the spec path. **run.rs:426 will no longer compile** — if executing tasks
strictly separately, apply Task 4 Step 1's minimal call-site change in THIS commit
(the two tasks are one reviewable unit at the compile boundary; note it in the
commit message).

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p al-call-hierarchy --lib preflight` → PASS (8 tests)

- [ ] **Step 5: Format, lint, commit** (possibly combined with Task 4 Step 1)

```bash
rustfmt src/engine/gate/preflight.rs
cargo clippy --all-targets --all-features
git add src/engine/gate/preflight.rs
git commit -m "feat(gate): preflight consumes FreshCoverage — labelled clauses + could-not-verify state"
```

---

### Task 4: wire `run_analyze` — fresh-first sequencing + formatter opaque override

**Files:**
- Modify: `src/engine/gate/run.rs` (:190-210 area for the fresh-first call; :358
  coverage block; :426-447 preflight/exit)
- Modify: `tests/cli/gate_prsummary_differential.rs` (:586, :636, :653)
- Regen: `tests/gate-goldens/exit-codes.json` + any formatter goldens that move

**Interfaces:**
- Consumes: `fresh_coverage` (Task 2), new `evaluate_preflight` (Task 3).
- Produces: `run_analyze*` behavior — fresh-keyed warning + exit; formatter
  `opaqueApps` sourced from fresh. `empty_output_result` untouched here (Task 5).

- [ ] **Step 1: Fresh-first call + preflight switch**

At the TOP of the analyze flow, before L3 assembly (before the `empty_output_result`
call sites at :201/:206 — the value is also needed by Task 5, so compute it first):

```rust
    // Fresh-first (spec §3 memory sequencing): compute the authoritative coverage
    // and DROP the whole ProgramContext inside fresh_coverage() before L3 assembly
    // — the two semantic models are never resident together.
    let fresh = crate::program::resolve::full::fresh_coverage(ws_path);
```

(`ws_path` = the same workspace `Path` L3 assembly uses; if it is constructed later
in the current code, hoist the construction, not the call.)

Replace the :426 call:

```rust
    let pf = evaluate_preflight(&fresh, args.require_dependencies);
```

Everything downstream (`pf.degraded` → stderr warning, `pf.failed` →
`exit::PREFLIGHT_FAILED`) is unchanged.

- [ ] **Step 2: Formatter opaque override**

At the :358 coverage block:

```rust
    let mut coverage = resolved.project_coverage_disk(ws_path);
    // One dependency universe (spec §3): the formatter-visible opaqueApps follows
    // the FRESH snapshot. The L3 gate path resolves source-only with empty deps
    // (src/engine/l3/coverage.rs:239) — its opaque list is structurally empty, and
    // leaving it would let stderr say "N symbol-only apps" while JSON says [].
    if let Ok(fc) = &fresh {
        coverage.opaque_apps = fc.opaque_apps.clone();
    }
```

- [ ] **Step 3: Build + fix the three pinned tests**

Run: `cargo test --test cli gate_prsummary_differential:: 2>gate.log; grep -E "FAILED|panicked" gate.log | head`

Update, deliberately (read each test's doc comment first):
- `:636` warning-substring oracle: the fixture it drives has NO symbol-only deps and
  fresh-resolves clean → it now expects NO warning (or, if the test's purpose is
  "warning surfaces when degraded", repoint it at `ws-baseapp-closure` and assert the
  substring `symbol-only dependency app`).
- `:586 anti_degenerate_preflight_exit_four`: repoint at `ws-baseapp-closure`
  (symbol-only dep → degraded → `--require-dependencies` → exit 4) so the
  anti-degenerate property ("exit 4 is reachable") survives the re-key.
- `:653 oracle_exit_precedence_preflight_wins_over_findings`: same repoint; the
  precedence property itself is unchanged.
- Exit-codes matrix: `REGEN_TEMP_GOLDENS=1 cargo test --test cli gate_prsummary_differential::`
  then INSPECT `git diff tests/gate-goldens/exit-codes.json` — expect old exit-4
  cells (L3-degraded fixtures) flipping to their finding-driven exit, and NO new 4s
  except genuinely fresh-degraded fixtures. A surprise flip = investigate, don't bless.

- [ ] **Step 4: Full golden sweep**

Run: `scripts/check-goldens --regen` then inspect `git status tests/` +
`git diff tests/cli-a-goldens/ | head -100` — JSON `opaqueApps` may move ONLY for
fixtures with symbol-only deps in their closure. Then `scripts/check-goldens` → green.

- [ ] **Step 5: Format, lint, commit**

```bash
rustfmt src/engine/gate/run.rs
cargo clippy --all-targets --all-features
git add src/engine/gate/run.rs tests/cli/gate_prsummary_differential.rs tests/gate-goldens/ tests/cli-a-goldens/
git commit -m "feat(gate): preflight + formatter opaqueApps keyed to the fresh resolver"
```

---

### Task 5: `empty_output_result` could-not-verify + integration tests

**Files:**
- Modify: `src/engine/gate/run.rs` (:466 `empty_output_result` + its call sites :201/:206)
- Modify: `tests/cli/gate_prsummary_differential.rs` (new tests; follow the file's
  existing run-analyze invocation helpers)

**Interfaces:**
- Consumes: `evaluate_preflight`, the `fresh` value from Task 4 Step 1.
- Produces: fail-closed paths warn + exit 4 under `--require-dependencies`.

- [ ] **Step 1: Write the failing integration tests**

In `gate_prsummary_differential.rs` (use the same invocation helper the neighboring
tests use — grep how `:586` runs analyze and captures `(output, exit, warning)`):

```rust
/// Fresh-clean workspace (no deps, resolves fully) → NO warning, exit driven by
/// findings only. The DO-shaped false-positive case this whole change kills.
#[test]
fn fresh_clean_workspace_emits_no_coverage_warning() {
    let (_out, _exit, warning) = run_analyze_fixture("ws-e2e", /*require_deps=*/ false);
    assert!(warning.is_none(), "clean fixture must not warn: {warning:?}");
}

/// Symbol-only dep → opaque clause warning; exit 4 only under --require-dependencies.
#[test]
fn symbol_only_dep_warns_opaque_and_gates_exit_four() {
    let (_o, exit, warning) = run_analyze_fixture("ws-baseapp-closure", false);
    let w = warning.expect("must warn");
    assert!(w.contains("symbol-only dependency app"), "got: {w}");
    assert_ne!(exit, 4, "fail-open without --require-dependencies");
    let (_o, exit, _w) = run_analyze_fixture("ws-baseapp-closure", true);
    assert_eq!(exit, 4);
}

/// Fail-closed workspace (unanalyzable) → could-not-verify warning, never silent
/// clean; exit 4 under --require-dependencies.
#[test]
fn fail_closed_workspace_could_not_verify() {
    // Reuse an existing fail-closed fixture (grep empty_output_result's tests /
    // the workspace_diagnostics tests for one, e.g. the multi-app or id-less
    // app.json fixture). If none is committed, mint tests/r0-corpus/ws-failclosed/
    // with an app.json lacking "id".
    let (_o, exit, warning) = run_analyze_fixture("<fail-closed-fixture>", false);
    let w = warning.expect("fail-closed must warn, not silent-clean");
    assert!(w.contains("coverage could not be verified"), "got: {w}");
    assert_ne!(exit, 4);
    let (_o, exit, _w) = run_analyze_fixture("<fail-closed-fixture>", true);
    assert_eq!(exit, 4);
}
```

(`run_analyze_fixture` = whatever helper shape the file actually has; `<fail-closed-fixture>`
resolved during implementation — the placeholder is a fixture NAME, every assertion is
complete.)

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --test cli gate_prsummary_differential::fail_closed 2>t.log; grep -E "FAILED|assert" t.log | head`
Expected: `fail_closed_workspace_could_not_verify` FAILS (silent clean today);
the other two may already pass after Task 4 — keep them regardless (regression pins).

- [ ] **Step 3: Implement**

`empty_output_result` gains the fresh result + stops hardcoding clean:

```rust
pub(crate) fn empty_output_result(
    args: &AnalyzeArgs,
    version: &str,
    fresh: &Result<crate::program::resolve::full::FreshCoverage, String>,
) -> Result<(String, u8, Option<String>), String> {
    // ... existing output construction unchanged ...

    // Fail-closed is a can't-analyze state: preflight must say so, never
    // fabricate clean (spec §3). Reason precedence: the fresh resolver's own
    // error when it failed too; else the defined fallback (some fail-closed
    // paths produce ZERO diagnostics — e.g. valid app.json, no readable .al).
    let reason = match fresh {
        Err(e) => e.clone(),
        Ok(_) => "workspace contained no readable AL source units".to_string(),
    };
    let pf = evaluate_preflight(&Err(reason), args.require_dependencies);
    let exit = if pf.failed { exit::PREFLIGHT_FAILED } else { exit::CLEAN };
    Ok((out, exit, Some(pf.message)))
}
```

Call sites :201/:206 pass `&fresh` (available from Task 4 Step 1's hoisted call).
If a richer fail-closed provider diagnostic is trivially in scope at the call sites,
prefer it as the `Ok(_)` arm's reason over the generic fallback — but never join
multiple diagnostics ad hoc.

- [ ] **Step 4: Run to verify pass + sweep**

Run: `cargo test --test cli` → PASS
Run: `scripts/check-goldens` → green

- [ ] **Step 5: Format, lint, commit**

```bash
rustfmt src/engine/gate/run.rs
cargo clippy --all-targets --all-features
git add src/engine/gate/run.rs tests/cli/gate_prsummary_differential.rs tests/r0-corpus/
git commit -m "fix(gate): fail-closed workspaces preflight could-not-verify instead of silent clean"
```

---

### Task 6: capstone — full validation, DO smoke, CHANGELOG, tick

- [ ] **Step 1: Full suite**

Run: `cargo nextest run --release 2>suite.log; tail -3 suite.log`
Expected: all green.

- [ ] **Step 2: DO smoke (the acceptance test)**

```bash
cargo build --profile release-fast --bin alsem
target/release-fast/alsem.exe analyze "U:/Git/DO.Support-SlowDOSetup/DocumentOutput/Cloud" --format json 1>NUL_out.json 2>do-stderr.txt; echo exit=$?
cat do-stderr.txt
```

Expected: NO `analysis coverage degraded` line (DO fresh-resolves unknown=0, its
deps carry embedded source), exit 0/1 per findings; `totalFindings` in the JSON
unchanged vs the 2307 baseline (`jq .payload.summary.totalFindings NUL_out.json`).
Any warning present → STOP, root-cause (opaque closure or recovered files on DO?),
do not ship.

- [ ] **Step 3: North-star guard**

Run: `target/release-fast/aldump.exe --program-call-graph-stats "U:/Git/DO.Support-SlowDOSetup/DocumentOutput/Cloud" | sha256sum`
Expected: `0a3b85bc832ff0a3e77acee118d203edbf62827dc37617c8d9315fe52d5cb7d0`
(build aldump with the same profile first: `cargo build --profile release-fast --bin aldump`).
This change is read-only over resolution — any hash drift = STOP.

- [ ] **Step 4: CHANGELOG under Changed**

```markdown
### Changed
- `alsem analyze` preflight re-keyed from the legacy L3 coverage to the fresh
  resolver (`FreshCoverage`): the degraded warning counts fresh primaryScoped
  `unknown` resolution edges (was an inflated L3 no-deps multiset that included
  Ambiguous/MemberNotFound/ExternalTarget), adds coverage-contract / recovered-file
  / symbol-only-dependency clauses, and gains a first-class "coverage could not be
  verified" state. CI-visible: `--require-dependencies` exit-4 semantics moved —
  previously-degraded workspaces (e.g. DO's false "1045 unresolved callsite(s)")
  now pass clean; fail-closed workspaces newly warn and exit 4 under the flag
  (was silent clean, exit 0); formatter `opaqueApps` now reports the fresh
  snapshot's closure-scoped symbol-only deps. Wording: "unresolved callsite(s)"
  → "unknown resolution edge(s)"; clean message "dependency coverage complete"
  → "resolution coverage verified". Costs one fresh resolve per analyze
  (~+3.8 s on the largest real workspace, sequenced so both engines' models are
  never resident together).
```

- [ ] **Step 5: Tick OUTSTANDING.md + final commit**

Mark the §1 preflight item `- [x]` with the landing commit hash.

```bash
git add CHANGELOG.md docs/OUTSTANDING.md
git commit -m "docs: preflight-fresh-coverage landed (CHANGELOG + OUTSTANDING tick)"
```

---

## Self-review notes (author, at plan time)

1. **Spec coverage:** §1 FreshCoverage → Task 2 (closure scoping, name display,
   sort/dedup, Err text via Task 1); §2 states/wording/clause-order/all-signals →
   Task 3; §3 sequencing + formatter override + fail-closed hole → Tasks 4-5;
   §4 CHANGELOG → Task 6; §5 test inventory → Tasks 3-5 (the three pins + matrix in
   Task 4, integration trio in Task 5, unit matrix in Task 3). Follow-ups stay in
   OUTSTANDING.md (already recorded).
2. **Known compile boundary:** Task 3's signature change breaks run.rs:426 — the
   plan explicitly allows folding Task 4 Step 1 into Task 3's commit.
3. **Open lookups for the implementer (names, not designs):** the exact invocation
   helper in `gate_prsummary_differential.rs`, and the fail-closed fixture name
   (mint `ws-failclosed` if none exists). Both bounded, both flagged inline.
4. **Fixture risk:** ws-baseapp-closure's Base App `.app` assumed symbol-only —
   Task 2 Step 1 carries the explicit verify-or-swap instruction.
