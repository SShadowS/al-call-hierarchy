# alsem `analyze` Hang Fix + Parallelization Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `alsem analyze` complete on real BC workspaces (CDO currently never finishes in 10+ min) by eliminating dead ordering-facts work on the default path, and parallelizing the per-root digest query for runs that DO need ordering facts.

**Architecture:** Two independent fixes ordered algorithmic-first, per the investigation
(`.superpowers/sdd/alsem-parallel/investigation.md`): (1) `compute_ordering_facts` — 43.6 s
for 120/635 roots, never completing for all 635 — runs EAGERLY in `build_detector_context`
but its ONLY consumers (d47/d49/d51) are opt-in detectors absent from the default set; make
it lazy (`OnceLock`), restoring al-sem's own documented lazy semantics (`ctx.getOrderingFacts()`,
memoized — see the field doc in `detector_context.rs:146-153`). (2) `digest_query`'s per-root
loop is embarrassingly parallel (immutable inputs, entries re-sorted by `routine_id` at the
end) — rayon `par_iter` gives ~N_core speedup on the ordering path when a preset selects
d47/d48/d49.

**Tech Stack:** Rust, rayon (already a dependency), `std::sync::OnceLock`.

## Global Constraints

- Branch: `feat/alsem-analyze-parallel` off `feat/tier1-perf-quick-wins` tip (`6f9668a`) — Tier-1 branch is not yet merged; do NOT branch off master.
- Byte-stable output: SARIF/JSON/goldens must be UNCHANGED — every golden and differential test passes without regeneration. If a golden diff appears, that is a bug in the change, not a rebaseline candidate.
- `rustfmt <file>` per touched file, NEVER `cargo fmt`.
- `cargo clippy --all-targets --all-features` clean per task.
- CHANGELOG.md updated per code task (Keep a Changelog; group under Fixed/Changed).
- Commits: stage only intended paths (never `git add -A`; never stage `.panel/`, `finish-cleanup.ps1`, `finish-t1-cleanup.ps1`, `scripts/peak_rss.py`). Trailer: `Co-authored-by: Copilot <223556219+Copilot@users.noreply.github.com>`.
- Measurement workspace: `u:\Git\DO.Support-SlowDOSetup\DocumentOutput\Cloud` (CDO). Baseline: default `analyze --format json` DNF (>10 min, 1 thread 100% CPU); phase profile in `.superpowers/sdd/alsem-parallel/investigation.md`.

---

### Task 1: Lazy `ordering_facts` (`OnceLock`) — fixes the default-analyze hang

**Files:**
- Modify: `src/engine/l5/detector_context.rs` (field ~154, `get_ordering_facts` ~177, `build_detector_context` tail ~431+457, `build_detector_context_cross_app` tail ~671)
- Test: in-file `#[cfg(test)]` mod of `src/engine/l5/detector_context.rs` (create if absent)
- Modify: `CHANGELOG.md`

**Interfaces:**
- Consumes: `crate::engine::l5::ordering_facts::{compute_ordering_facts, OrderingFacts}` (unchanged), `L3Resolved` (unchanged).
- Produces: `DetectorContext::get_ordering_facts(&self) -> &HashMap<String, OrderingFacts>` — signature UNCHANGED (d47/d49/d51 need no edits). Field `ordering_facts` becomes `OnceLock` and a new private `ordering_source: Option<&'a L3Resolved>` field is added; Task 2 does not touch these.

- [ ] **Step 1: Write the failing test**

Add at the end of `src/engine/l5/detector_context.rs` (or extend the existing tests mod if one exists):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// Laziness contract: `build_detector_context` must NOT compute ordering facts
    /// (the OnceLock starts empty); first `get_ordering_facts()` call computes and
    /// memoizes a map EQUAL to a direct `compute_ordering_facts(resolved)` run.
    #[test]
    fn ordering_facts_are_lazy_and_parity_with_direct_compute() {
        // Empty workspace: cheap, and exercises the full lazy path end-to-end.
        let resolved = crate::engine::l3::l3_workspace::L3Resolved {
            workspace: crate::engine::l3::l3_workspace::L3Workspace {
                objects: Vec::new(),
                tables: Vec::new(),
                routines: Vec::new(),
            },
            root_classifications: Vec::new(),
            primary_app: None,
            infra_diagnostics: Vec::new(),
        };
        let ctx = build_detector_context(&resolved);
        assert!(
            ctx.ordering_facts.get().is_none(),
            "ordering facts must not be computed eagerly"
        );
        let via_ctx = ctx.get_ordering_facts();
        let direct = crate::engine::l5::ordering_facts::compute_ordering_facts(&resolved);
        assert_eq!(via_ctx.len(), direct.len());
        assert!(
            ctx.ordering_facts.get().is_some(),
            "first access must memoize"
        );
    }
}
```

NOTE: if `L3Resolved`/`L3Workspace` field sets differ from the above (check the real
struct), fix the literal — the TEST INTENT (OnceLock empty → access computes → memoized,
parity with direct compute) is what matters. If constructing `L3Resolved` requires more
than this, reuse whatever fixture helper `src/engine/l5/registry.rs`'s tests mod (~480)
uses to build one.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib detector_context::tests::ordering_facts_are_lazy -- --nocapture`
Expected: COMPILE FAILURE (`ordering_facts.get()` — HashMap has no `get()` with 0 args / field is not a OnceLock yet). That is the failing state.

- [ ] **Step 3: Implement the lazy field**

In `src/engine/l5/detector_context.rs`:

(a) Change the field (~line 154) and its doc:

```rust
    /// R4-F Stage-5b — the L4.5 ordering facts the d47/d49/d51 detectors consume,
    /// keyed by `StableRoutineId`. Computed LAZILY on first `get_ordering_facts()`
    /// access and memoized — exactly al-sem's `ctx.getOrderingFacts()` semantics.
    /// Only d47/d49/d51 (opt-in detectors) read it, so a default `analyze` run
    /// never pays the snapshot→digest→ordering cost (measured 43.6 s+ on CDO —
    /// the "alsem never completes" hang; see
    /// `.superpowers/sdd/alsem-parallel/investigation.md`).
    pub ordering_facts:
        std::sync::OnceLock<HashMap<String, crate::engine::l5::ordering_facts::OrderingFacts>>,
    /// The resolved model `get_ordering_facts()` computes from. `None` for the
    /// cross-app context (whose ordering facts are ALWAYS empty — d13/d16/d17
    /// never read them; matches the previous eager `HashMap::new()`).
    pub ordering_source: Option<&'a L3Resolved>,
```

(b) Replace `get_ordering_facts` (~line 177):

```rust
    /// The L4.5 ordering facts, keyed by `StableRoutineId`. Lazily computed on
    /// first access (memoized via `OnceLock` — thread-safe for future parallel
    /// detector runs). d47/d49/d51 look up their reportable routine's facts here
    /// exactly as al-sem's `ctx.getOrderingFacts()`.
    pub fn get_ordering_facts(
        &self,
    ) -> &HashMap<String, crate::engine::l5::ordering_facts::OrderingFacts> {
        self.ordering_facts.get_or_init(|| match self.ordering_source {
            Some(resolved) => {
                crate::engine::l5::ordering_facts::compute_ordering_facts(resolved)
            }
            None => HashMap::new(),
        })
    }
```

(c) In `build_detector_context` (~line 431): DELETE the eager
`let ordering_facts = ... compute_ordering_facts(resolved);` line and its comment; in the
struct literal replace `ordering_facts,` with:

```rust
        ordering_facts: std::sync::OnceLock::new(),
        ordering_source: Some(resolved),
```

(d) In `build_detector_context_cross_app` (~line 671) replace
`ordering_facts: HashMap::new(),` with:

```rust
        ordering_facts: std::sync::OnceLock::new(),
        ordering_source: None,
```

and update the function doc's "`ordering_facts` are EMPTY here" sentence to "`ordering_source`
is `None` here (ordering facts lazily resolve to EMPTY".

(e) Fix any other struct-literal constructors of `DetectorContext` the compiler flags
(tests included) the same way.

- [ ] **Step 4: Run the test + consumers to verify green**

Run: `cargo test --lib detector_context`
Expected: PASS.
Run: `cargo test --lib -- d47 d49 d51 ordering` (the ordering detectors + ordering-facts unit/golden tests)
Expected: PASS — these route through `get_ordering_facts()` and prove the lazy path serves identical content.

- [ ] **Step 5: Full validation**

Run: `cargo test` (full suite — goldens must be untouched) then `cargo clippy --all-targets --all-features`.
Expected: all green, clippy clean, `git status` shows only intended files.

- [ ] **Step 6: Smoke the actual fix on CDO**

Run (PowerShell):
```powershell
cargo build --release --bin alsem
$sw=[System.Diagnostics.Stopwatch]::StartNew(); .\target\release\alsem.exe analyze 'u:\Git\DO.Support-SlowDOSetup\DocumentOutput\Cloud' --format json 1>$env:TEMP\alsem-t1.json 2>$null; $sw.Stop(); "exit=$LASTEXITCODE elapsed=$($sw.Elapsed.TotalSeconds)s"
```
Expected: completes, elapsed ≈ 5–10 s (investigation predicts ~5 s: assemble 1 s + ctx ~1 s + detectors ~4 s). Record the number for Task 3. If it still hangs, STOP — something else is eager; re-instrument, do not proceed.

- [ ] **Step 7: rustfmt, CHANGELOG, commit**

```powershell
rustfmt src/engine/l5/detector_context.rs
```
CHANGELOG.md under `Fixed`:
```markdown
- `alsem analyze` no longer hangs (10+ min, single core pegged) on large workspaces: the
  L4.5 ordering-facts pass (43.6 s+ on CDO, superlinear witness reconstruction) ran eagerly
  in `build_detector_context` although only the OPT-IN d47/d49/d51 detectors read it. It is
  now computed lazily on first `get_ordering_facts()` access (al-sem's own memoized
  `ctx.getOrderingFacts()` semantics), so the default detector set never pays it. Default
  `analyze` on CDO: never-completes → ~<measured> s. Output byte-identical.
```
(fill `<measured>` from Step 6)

```powershell
git add src/engine/l5/detector_context.rs CHANGELOG.md
git commit -m "fix: lazy ordering_facts — alsem analyze no longer hangs on default detector set

Co-authored-by: Copilot <223556219+Copilot@users.noreply.github.com>"
```

---

### Task 2: Parallelize `digest_query` across roots (rayon)

**Files:**
- Modify: `src/engine/l5/digest.rs` (`digest_query` ~2036-2428: extract per-root body into `digest_one_root`, drive with `par_iter`)
- Modify: `CHANGELOG.md`

**Interfaces:**
- Consumes: nothing from Task 1 (independent seam — `digest_query` is called by
  `compute_digest_effects_for_ordering` / `compute_digest_effects_with_ordering` /
  `compute_digest_effects_cli`, all unchanged signatures).
- Produces: `digest_query` signature UNCHANGED. New private
  `fn digest_one_root(rid: &str, snap: &CapabilitySnapshot, idx: &FingerprintIndexes, callsite_by_id_str: &HashMap<&str, &SnapshotCallsiteEvidence>, cs_ctx: &HashMap<&str, Option<&str>>, op_ctx: &HashMap<&str, Option<&str>>, return_summaries: Option<&HashMap<String, RoutineReturnSummary>>, isolated_event_ids: Option<&HashSet<String>>, ordering_witness_only: bool) -> Option<DigestEntryResult>` (adjust the index type name to the real return type of `build_fingerprint_indexes`).

- [ ] **Step 1: Extract `digest_one_root` (pure refactor, still sequential)**

Move the ENTIRE `for rid in roots { ... }` body (from `let Some(display) = ...` through
`entries.push(DigestEntryResult { ... })` at ~2418) into the new function; the early
`continue` becomes `return None`, the final push becomes
`Some(DigestEntryResult { routine_id: rid.to_string(), effects })`. The loop becomes:

```rust
    for rid in roots {
        if let Some(entry) = digest_one_root(
            rid,
            snap,
            &idx,
            &callsite_by_id_str,
            &cs_ctx,
            &op_ctx,
            return_summaries,
            isolated_event_ids,
            ordering_witness_only,
        ) {
            entries.push(entry);
        }
    }
```

The `AccumulatedEffect` local struct, `ORDERING_RELEVANT` const, and `MAX_PATHS` move with
the body (make `MAX_PATHS` a module-level `const MAX_PATHS: usize = 3;` next to
`HARD_PATH_CAP` if the wrapper still references it; otherwise move it into
`digest_one_root`).

- [ ] **Step 2: Verify the refactor is behavior-neutral**

Run: `cargo test --lib digest && cargo test --lib ordering`
Expected: PASS with zero golden diffs. Commit checkpoint is NOT taken yet (refactor + parallelization land as one commit; the sequential extraction is just the reviewable intermediate state — run the tests, then continue).

- [ ] **Step 3: Parallelize the loop**

Replace the loop with:

```rust
    use rayon::prelude::*;
    let mut entries: Vec<DigestEntryResult> = roots
        .par_iter()
        .filter_map(|rid| {
            digest_one_root(
                rid,
                snap,
                &idx,
                &callsite_by_id_str,
                &cs_ctx,
                &op_ctx,
                return_summaries,
                isolated_event_ids,
                ordering_witness_only,
            )
        })
        .collect();
```

Keep the existing tail exactly as-is:

```rust
    // Sort entries by routineId.
    entries.sort_by(|a, b| a.routine_id.cmp(&b.routine_id));
    entries
```

Determinism argument (put a short comment above the `par_iter`): every input is an
immutable `&` (snap/idx/maps); each root's computation is independent and internally
deterministic; `roots` is deduped, so `routine_id` keys are unique and the final
`sort_by(routine_id)` fully determines output order regardless of scheduling. Uses the
GLOBAL rayon pool (no AL-source lowering happens here — the big-stack pool is only for
the CST lowerer; witness BFS is heap-based with `MAX_DEPTH = 64`).

If the compiler rejects `par_iter` because some captured type is not `Sync` (e.g. interior
mutability hiding in `CapabilitySnapshot`): STOP and investigate what carries the `!Sync`
— do not bypass with `unsafe`/`Mutex` wrapping; report in the task report if non-trivial.

- [ ] **Step 4: Full validation (byte-stability is the test)**

Run: `cargo test` (the digest/ordering/scoped-guarantee differential goldens are the real
oracle — any scheduling-order leak shows up as a golden diff) and
`cargo clippy --all-targets --all-features`.
Expected: all green, zero golden diffs, clippy clean.

- [ ] **Step 5: Measure the ordering path on CDO**

The default run no longer exercises this path (Task 1), so measure with the preset that
selects ordering detectors:

```powershell
cargo build --release --bin alsem
$sw=[System.Diagnostics.Stopwatch]::StartNew(); .\target\release\alsem.exe analyze 'u:\Git\DO.Support-SlowDOSetup\DocumentOutput\Cloud' --preset transaction-integrity --format json 1>$env:TEMP\alsem-t2.json 2>$null; $sw.Stop(); "exit=$LASTEXITCODE elapsed=$($sw.Elapsed.TotalSeconds)s"
```

Run once at the Task-1 commit (sequential baseline — expect many minutes; if it exceeds
20 min, kill it (`Stop-Process -Id`), record ">20 min") and once at this commit. Expected
after: the digest phase drops ~N_core (investigation: 8–16×); whole-command wall time
should land in low minutes or better. Record both numbers for Task 3. Also verify the two
JSON outputs are byte-identical when both complete:
`fc.exe /b $env:TEMP\alsem-t2-seq.json $env:TEMP\alsem-t2.json` (adapt temp names).

- [ ] **Step 6: rustfmt, CHANGELOG, commit**

```powershell
rustfmt src/engine/l5/digest.rs
```
CHANGELOG.md under `Changed`:
```markdown
- `digest_query` (the L5 witness/ordering digest behind `--preset transaction-integrity`'s
  d47/d48/d49 and the digest CLI) now processes roots in parallel (rayon). Output is
  byte-identical (independent per-root computation + the existing final sort by
  routineId). CDO transaction-integrity preset: <before> → <after>.
```

```powershell
git add src/engine/l5/digest.rs CHANGELOG.md
git commit -m "perf: parallelize digest_query across roots (rayon, byte-identical)

Co-authored-by: Copilot <223556219+Copilot@users.noreply.github.com>"
```

---

### Task 3: Measurement close-out + residual backlog

**Files:**
- Modify: `docs/perf-regression-t3-vs-0.9.3.md` (append `## 11. alsem analyze hang fix + digest parallelization`)
- Create: `.superpowers/sdd/alsem-parallel/close-out.md` (raw runs; gitignored)

**Interfaces:**
- Consumes: measured numbers recorded in Task 1 Step 6 and Task 2 Step 5.
- Produces: docs only.

- [ ] **Step 1: Consolidate measurements**

Re-run both commands (default + transaction-integrity preset) 3× at the branch tip; take
medians. Write raw runs to `.superpowers/sdd/alsem-parallel/close-out.md`.

- [ ] **Step 2: Append §11 to the perf doc**

Content (fill real numbers):

```markdown
## 11. alsem `analyze` hang fix + digest parallelization (2026-07-14)

Investigation: `.superpowers/sdd/alsem-parallel/investigation.md`. Root cause of the
"never completes" hang: `compute_ordering_facts` (43.6 s for 120/635 roots on CDO;
witness reconstruction is ~O(cone²) on top-of-graph Page roots) ran EAGERLY in
`build_detector_context` although only opt-in d47/d49/d51 read it — pure dead work for
the default detector set.

| Command (CDO) | Before | After |
|---|---|---|
| `analyze --format json` (default set) | DNF (>10 min) | <X> s |
| `analyze --preset transaction-integrity` | <Y or DNF> | <Z> |

Fixes: (1) lazy `OnceLock` ordering facts (al-sem parity semantics restored); (2) rayon
`par_iter` over `digest_query` roots (byte-identical: final sort by routineId).

Residual backlog (measured, deliberately deferred):
- `reconstruct_witness_paths` redundancy: per-(root,fact) BFS with no sharing across a
  root's facts or overlapping cones — the algorithmic cut if transaction-integrity is
  still too slow (share the reverse-BFS `valid_nodes` set, digest.rs:940).
- `compose_snapshot` 2.6–12 s one-time on the ordering path (parallelizable per-routine).
- `d1-db-op-in-loop` 3.37 s — the largest remaining default-set cost.
- Per-detector fan-out in `run_each` (~1.5–2× on 3.9 s; bounded by d1) — low priority.
```

- [ ] **Step 3: Commit**

```powershell
git add docs/perf-regression-t3-vs-0.9.3.md
git commit -m "docs: alsem analyze close-out (hang fix + digest parallelization, section 11)

Co-authored-by: Copilot <223556219+Copilot@users.noreply.github.com>"
```
