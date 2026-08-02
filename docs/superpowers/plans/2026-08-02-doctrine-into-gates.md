# Doctrine Into Gates — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Convert the repo's written-but-unenforced doctrine into automated gates, closing the specific defects that let CI stay red for two days across five merges.

**Architecture:** Enforcement lives in versioned, shared layers (`scripts/git-hooks/pre-commit`, `scripts/`, `tests/`, `.github/workflows/ci.yml`) because `.claude/` is gitignored except `commands/` and therefore binds nobody but this machine. Guidance lives in local skills/agents. Where a claim is mechanically checkable it becomes an `assert`, not prose — the governing idea is that a stale doc naming a version, a consumer, or a number should die as a test failure rather than be hunted by a reviewer.

**Tech Stack:** Bash (Git Bash on Windows, bash on ubuntu CI), Rust 1.96 / `cargo`, Python 3 (existing precedent: `scripts/peak_rss.py`), GitHub Actions, `gh` CLI.

## Global Constraints

Every task's requirements implicitly include this section.

- **Format per-file with `rustfmt <file>`. NEVER bare `cargo fmt`** (whole-crate churn). `cargo fmt --check` is read-only and allowed.
- **Never `git add -A` / `git add .`** — stage only intended paths.
- **Lint bar is CI's:** `cargo clippy --release --all-targets --all-features -- -D warnings`.
- **Byte-identity gates**, exact, for any change that can reach engine output:
  - DO `f022f677d2650b2399fc3aa5a7625bc6c078d90dd51cdb80e1e3705808fee3ea`
  - 8020 `36151bf67e17620724abb6b2cdbad55bcf8f97ffe3c3237782a0cf4c25ecc5fb`
- **`scripts/check-goldens` green with ZERO files under `tests/` moved.**
- **A test must pin the USE, not the helper; hand-state its precondition; and carry a recorded discrimination proof** (break it, watch it fail, revert, watch it pass, record both). A proof that passes is evidence about the TEST, not the code.
- **Scripted multi-site edits must assert their match count** (`assert s.count(old) == 1`) and check what sits immediately before the anchor.
- **CHANGELOG.md must be updated** for feature additions, bug fixes, breaking changes.
- **Never push or merge to `master` without an explicit request.**
- Worktrees have no submodule checkout: export `TREE_SITTER_AL_PATH=U:/Git/al-call-hierarchy/tree-sitter-al`. `cargo run -p xtask -- gen-syntax` **cannot** run from a worktree.
- Do not pipe a gate through `| tail` — the exit code becomes `tail`'s. Redirect to a log and grep it.

---

### Task 1: Pre-commit `cargo fmt --check` gate

The two-day CI outage in full: `cargo fmt --check` is CI's **first** step, it failed in 26 s, and so clippy / gen-syntax / test / build / perf_bounds never ran at all across five merges. A per-file `rustfmt` `PostToolUse` hook already exists in `.claude/settings.local.json` (matcher `Write|Edit|MultiEdit`, `.rs$`), but it cannot see edits made through `Bash` — which is exactly how this repo's documented `scripted-edit` workflow applies multi-site patches, and exactly how the offending edit landed. The commit is the one chokepoint every edit path converges on.

**Files:**
- Modify: `scripts/git-hooks/pre-commit:59-66` (insert before the golden-path early exit)
- Modify: `.claude/skills/scripted-edit/SKILL.md` (append a mandatory final step)

**Interfaces:**
- Consumes: nothing.
- Produces: nothing consumed by later tasks.

- [ ] **Step 1: Read the current hook and confirm the insertion point**

Run: `sed -n '59,70p' scripts/git-hooks/pre-commit`

Expected: `set -uo pipefail`, then `golden_paths='...'`, then the `if ! git diff --cached --name-only | grep -qE "$golden_paths"; then exit 0; fi` block. The fmt gate must go **before** that early `exit 0`, because a commit touching only e.g. `src/main.rs` formatting still matters while a commit touching no golden path exits early today.

- [ ] **Step 2: Insert the fmt gate**

Insert immediately after the `set -uo pipefail` line (line 59), before the `golden_paths=` assignment:

```bash
# ── Formatting, first and cheapest ───────────────────────────────────────────
# CI's FIRST step is `cargo fmt --check`; when it fails, CI stops there and
# NOTHING else runs — clippy, gen-syntax, tests, build and perf_bounds are all
# skipped. That state persisted for two days across five merges on 2026-07-31..
# 08-02. The `.claude/` PostToolUse rustfmt hook covers Edit/Write, but a
# scripted (Bash) edit — this repo's documented `scripted-edit` workflow — is
# invisible to it. The commit is the chokepoint every path shares.
#
# Read-only (`--check`), so this does not violate the "never `cargo fmt`" law.
if git diff --cached --name-only | grep -qE '\.rs$'; then
    if ! cargo fmt --check; then
        echo "pre-commit: cargo fmt --check FAILED (CI's first step) — fix with:" >&2
        echo "  rustfmt <each touched file>     # never bare 'cargo fmt' (CLAUDE.md)" >&2
        exit 1
    fi
fi
```

Note the honest edge, and accept it: `cargo fmt --check` reads the working tree, not the index, so unformatted *unstaged* edits on top of a formatted staged file will fail this gate. That matches the repo's "the tree is formatted" doctrine.

- [ ] **Step 3: Prove the gate fires (discrimination proof, direction 1)**

```bash
printf 'pub fn x( ) ->u8{1}\n' >> src/lib.rs
git add src/lib.rs
git commit -m "temp: prove the fmt gate fires"
```

Expected: commit REJECTED, stderr contains `cargo fmt --check FAILED`.

- [ ] **Step 4: Prove the gate passes when formatted (direction 2)**

```bash
rustfmt src/lib.rs
git add src/lib.rs
git commit -m "temp: prove the fmt gate passes"
```

Expected: commit SUCCEEDS. Then undo the scratch commit and the scratch code:

```bash
git reset --soft HEAD~1
git restore --staged src/lib.rs
```

Then hand-remove the `pub fn x` line from `src/lib.rs` and confirm `git status --short` shows `src/lib.rs` clean. **Record both outcomes** (rejected / accepted) in the commit message.

- [ ] **Step 5: Add the mandatory format step to the `scripted-edit` skill**

Append to `.claude/skills/scripted-edit/SKILL.md` a final numbered step:

```markdown
## Final step (MANDATORY): format what you touched

A scripted edit goes through `Bash`, so the `PostToolUse` rustfmt hook never
sees it. Formatting is CI's FIRST step; when it fails nothing else on CI runs.

    rustfmt <each file the script touched>
    cargo fmt --check          # read-only; never bare `cargo fmt`

This is the exact path that took CI red for two days across five merges
(2026-07-31..08-02): the new module was formatted, the file it was wired into
was not.
```

- [ ] **Step 6: Commit**

```bash
git add scripts/git-hooks/pre-commit .claude/skills/scripted-edit/SKILL.md
git commit -m "fix(hooks): gate commits on cargo fmt --check

CI's first step is cargo fmt --check; when it fails nothing else runs.
That state held for two days across five merges. The PostToolUse rustfmt
hook covers Edit/Write but not Bash-scripted edits, which is how the
offending edit landed. Discrimination proof recorded: unformatted commit
REJECTED, formatted commit ACCEPTED."
```

Note `.claude/` is gitignored except `commands/`, so the SKILL.md edit is local-only and will not appear in the commit — that is expected; stage it anyway so the command is copy-pasteable and git simply ignores it.

---

### Task 2: `scripts/ci-steps` as the single source of truth

Local clippy was run all session as `cargo clippy --all-targets --all-features`; CI runs `cargo clippy --release --all-targets --all-features -- -D warnings`. Two axes stricter, discovered only at the end. A `ci-local` *skill* quoting those strings would be a **third** copy that can drift; a script *called by* CI makes drift impossible.

**Files:**
- Create: `scripts/ci-steps`
- Modify: `.github/workflows/ci.yml:40-52,62-80` (replace inline `run:` bodies)
- Modify: `CLAUDE.md` (Build Commands section — the Lint line)

**Interfaces:**
- Consumes: nothing.
- Produces: `scripts/ci-steps <fmt|clippy|gen-syntax|test|build|perf-bounds|all>`, used by Task 8's capstone note.

- [ ] **Step 1: Create `scripts/ci-steps`**

```bash
#!/usr/bin/env bash
# scripts/ci-steps — the EXACT commands CI runs, in one place.
#
# `.github/workflows/ci.yml` calls this script rather than inlining command
# strings, so a local run and a CI run cannot drift. Before this existed they
# had: CLAUDE.md and every local session used
# `cargo clippy --all-targets --all-features` while CI used
# `--release --all-targets --all-features -- -D warnings`, and the gap was
# found only after CI had been red for two days.
#
# Usage: scripts/ci-steps <step>
#   fmt | clippy | gen-syntax | test | build | perf-bounds | all
set -uo pipefail

step="${1:-}"

run_fmt() { cargo fmt --check; }

run_clippy() { cargo clippy --release --all-targets --all-features -- -D warnings; }

# NOTE: cannot run from a git worktree — worktrees have no submodule checkout,
# so `tree-sitter-al/src/node-types.json` is absent. Run it in the main checkout.
run_gen_syntax() {
    cargo run -p xtask -- gen-syntax || return 1
    git diff --exit-code crates/al-syntax/src/raw/generated
}

run_test() { cargo test --workspace; }

run_build() { cargo build --release; }

run_perf_bounds() { cargo test --release --test perf_bounds; }

case "$step" in
    fmt)         run_fmt ;;
    clippy)      run_clippy ;;
    gen-syntax)  run_gen_syntax ;;
    test)        run_test ;;
    build)       run_build ;;
    perf-bounds) run_perf_bounds ;;
    all)
        rc=0
        for s in fmt clippy gen-syntax test build perf-bounds; do
            echo "=== ci-steps: $s ==="
            if ! "$0" "$s"; then
                echo "ci-steps: FAILED at '$s'" >&2
                rc=1
            fi
        done
        exit "$rc"
        ;;
    *)
        echo "usage: scripts/ci-steps <fmt|clippy|gen-syntax|test|build|perf-bounds|all>" >&2
        exit 2
        ;;
esac
```

Note `all` deliberately does NOT stop at the first failure — the whole point of this plan is that a step-1 failure must not hide steps 2-6.

- [ ] **Step 2: Make it executable and verify each step individually**

```bash
chmod +x scripts/ci-steps
export TREE_SITTER_AL_PATH=U:/Git/al-call-hierarchy/tree-sitter-al
scripts/ci-steps fmt    && echo "fmt OK"
scripts/ci-steps clippy > /tmp/cl.log 2>&1; echo "clippy exit=$?"
```

Expected: both exit 0 on current master.

- [ ] **Step 3: Point `ci.yml` at the script**

Replace each step's `run:` body (keep the `name:`, `env:` and comment blocks exactly as they are — they carry review history):

```yaml
      - name: Check formatting
        run: scripts/ci-steps fmt
```
```yaml
        run: scripts/ci-steps clippy
```
```yaml
        run: scripts/ci-steps gen-syntax
```
```yaml
        run: scripts/ci-steps test
```
```yaml
        run: scripts/ci-steps build
```
```yaml
        run: scripts/ci-steps perf-bounds
```

- [ ] **Step 4: Fix the CLAUDE.md Lint line**

In CLAUDE.md's "Build Commands" block, replace:

```
cargo clippy --all-targets --all-features  # Lint
```

with:

```
scripts/ci-steps clippy        # Lint — CI's EXACT bar (release + -D warnings)
```

This is the highest ROI-per-minute change in the plan: the canonical doc specified the *weaker* command, so every session ran it and was wrong by documentation.

- [ ] **Step 5: Verify the script is what CI will actually execute**

Run: `grep -c "scripts/ci-steps" .github/workflows/ci.yml`

Expected: `6`.

- [ ] **Step 6: Commit**

```bash
git add scripts/ci-steps .github/workflows/ci.yml CLAUDE.md
git commit -m "chore(ci): single source of truth for CI commands

CLAUDE.md documented 'cargo clippy --all-targets --all-features' while CI
ran '--release --all-targets --all-features -- -D warnings'. Every session
ran the documented command and was wrong by documentation. ci.yml now calls
scripts/ci-steps, so local and CI cannot drift. 'all' does not stop at the
first failure, so a step-1 break can no longer hide steps 2-6."
```

---

### Task 3: Root-cause and pin `CACHE_VERSION_GRAMMAR`

`src/engine/gate/cache_prune.rs:40` reads `"tree-sitter-al-v2.5.2-native"` while the pinned grammar is **v3.2.0**. It is **not prose** — line 190 uses it in the `expected` version tuple that decides whether a cached dependency artifact is current or should be pruned. Investigate before fixing: this may be inert (if nothing writes those artifacts) or a live cache-invalidation defect (if something does).

**Files:**
- Investigate: `src/engine/gate/cache_prune.rs:36-60,185-200`
- Modify: `src/engine/gate/cache_prune.rs:40` (only if the investigation says so)
- Create: `src/engine/gate/cache_prune.rs` test module addition (the pin)

**Interfaces:**
- Consumes: nothing.
- Produces: nothing.

- [ ] **Step 1: Establish whether anything WRITES a dependency-cache artifact**

```bash
grep -rn "artifactKey\|artifact_key\|header.*versions\|dev_fingerprint" src/ --include=*.rs | grep -v cache_prune.rs
grep -rn "fn prune_cache\|classify_artifact_for_prune" src/ --include=*.rs
```

Record the answer in the task's commit message. Two outcomes:
- **A writer exists** → the stale constant is a live defect: artifacts minted under v3.2.0 carry a `grammar` value that no longer matches any current-version tuple, so classification is wrong. Fix the constant AND pin it.
- **No writer exists** (only the pruner) → the constant is inert today. Still pin it, and say plainly in the CHANGELOG that it is inert, so the pin is not mis-sold as a bug fix.

- [ ] **Step 2: Write the failing pin test**

Add to `src/engine/gate/cache_prune.rs`'s `#[cfg(test)] mod tests`:

```rust
/// `CACHE_VERSION_GRAMMAR` participates in the dependency-cache version tuple
/// (`expected` in `classify_artifact_for_prune`), so it must track the grammar
/// this engine actually links. It did not: it read
/// `"tree-sitter-al-v2.5.2-native"` while the pinned grammar was v3.2.0 —
/// never bumped across two grammar upgrades, and nothing failed.
///
/// This is the executable form of that doc claim. A version literal that can be
/// compared against its source should die as an assert, not be hunted as prose.
///
/// DISCRIMINATION PROOF (record when adding): set the constant back to
/// `"tree-sitter-al-v2.5.2-native"` and this test FAILS; restore and it passes.
#[test]
fn cache_version_grammar_tracks_the_linked_grammar() {
    let pkg = std::fs::read_to_string(
        concat!(env!("CARGO_MANIFEST_DIR"), "/tree-sitter-al/package.json"),
    )
    .expect("tree-sitter-al/package.json must be present (submodule checked out)");
    let v: serde_json::Value = serde_json::from_str(&pkg).expect("package.json parses");
    let grammar_version = v["version"].as_str().expect("package.json has a version");

    assert!(
        CACHE_VERSION_GRAMMAR.contains(grammar_version),
        "CACHE_VERSION_GRAMMAR is {CACHE_VERSION_GRAMMAR:?} but the linked grammar \
         is v{grammar_version} — bump the constant (it keys dependency-cache \
         invalidation, it is not a comment)"
    );
}
```

- [ ] **Step 3: Run it and watch it FAIL**

Run: `cargo test -p al-call-hierarchy --lib cache_version_grammar_tracks`

Expected: FAIL — `CACHE_VERSION_GRAMMAR is "tree-sitter-al-v2.5.2-native" but the linked grammar is v3.2.0`.

- [ ] **Step 4: Fix the constant**

```rust
pub const CACHE_VERSION_GRAMMAR: &str = "tree-sitter-al-v3.2.0-native";
```

- [ ] **Step 5: Run the test and the golden gate**

```bash
cargo test -p al-call-hierarchy --lib cache_version_grammar_tracks
bash scripts/check-goldens > /tmp/g.log 2>&1; echo "goldens exit=$?"
grep -E "^test result" /tmp/g.log | grep -vc "0 failed"
```

Expected: test PASSES; goldens exit 0; the `grep -vc` prints `0`.

**If a golden moved**, stop and triage before regenerating: the `cli-c-policy` / `gate-*` families can carry a `cache prune` report, and a version-tuple change is exactly the kind of thing that legitimately moves one. Use the `golden-diff-triager` agent.

- [ ] **Step 6: Byte-identity gate**

```bash
export TREE_SITTER_AL_PATH=U:/Git/al-call-hierarchy/tree-sitter-al
cargo build --profile release-fast --bin alsem > /tmp/b.log 2>&1; echo "build=$?"
./target/release-fast/alsem.exe analyze 'U:/Git/DO.Support-SlowDOSetup/DocumentOutput/Cloud' \
  --format json --deterministic 2>/dev/null | sha256sum
```

Expected: `f022f677d2650b2399fc3aa5a7625bc6c078d90dd51cdb80e1e3705808fee3ea`.

- [ ] **Step 7: Commit**

```bash
git add src/engine/gate/cache_prune.rs CHANGELOG.md
git commit -m "fix(cache): CACHE_VERSION_GRAMMAR tracks the linked grammar, pinned by a test

It read 'tree-sitter-al-v2.5.2-native' against a v3.2.0 grammar, never
bumped through two upgrades, and nothing failed. It is not a comment: it
keys the dependency-cache version tuple in classify_artifact_for_prune.
Now asserted against tree-sitter-al/package.json, so a future grammar bump
fails a test instead of silently rotting. Discrimination proof recorded."
```

---

### Task 4: Output-identity gate (cold / warm / cache-disabled)

The preflight verdict cache is days old, and a cache serving a stale verdict is the one failure class nothing currently detects — every golden family runs with the cache in whatever state the runner left it. This makes the invariant executable and permanent.

**It doubles as the empirical determinism check for parallel detectors**, at no extra cost: the three runs it compares are three separate executions of a binary that now runs ~54 detectors on a rayon pool. If that parallelism were order-nondeterministic, `cold == warm` would fail. That is why no separate "run it twice" leg is needed here, and why a `determinism-reviewer` agent is deferred rather than built — an assert catches actual nondeterminism on every CI run with zero reviewer judgment.

**Files:**
- Create: `tests/cli/preflight_cache_identity.rs`
- Modify: `tests/cli/main.rs` (add the `mod` line)

**Interfaces:**
- Consumes: `ALSEM_PREFLIGHT_CACHE_DIR`, `ALSEM_NO_PREFLIGHT_CACHE` (from `src/program/resolve/preflight_cache.rs`).
- Produces: nothing.

- [ ] **Step 1: Confirm the umbrella's member-registration pattern**

Run: `head -30 tests/cli/main.rs`

Expected: a list of `mod <member>;` items. The new member is added the same way.

- [ ] **Step 2: Write the test**

Create `tests/cli/preflight_cache_identity.rs`:

```rust
//! The preflight verdict cache must never change output.
//!
//! `alsem analyze` output is byte-gated, and the cached `FreshCoverage` payload
//! is formatter-visible (`opaque_apps` flows into the JSON coverage block). So
//! a COLD run, a WARM run (served from cache) and a CACHE-DISABLED run must all
//! produce identical bytes. Nothing else in the suite covers the warm path:
//! every golden family runs with the cache in whatever state the runner left it.

use std::process::Command;

fn run(ws: &str, cache_dir: Option<&std::path::Path>, disabled: bool) -> String {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_alsem"));
    cmd.args(["analyze", ws, "--format", "json", "--deterministic"]);
    match cache_dir {
        Some(d) => {
            cmd.env("ALSEM_PREFLIGHT_CACHE_DIR", d);
        }
        None => {
            cmd.env_remove("ALSEM_PREFLIGHT_CACHE_DIR");
        }
    }
    if disabled {
        cmd.env("ALSEM_NO_PREFLIGHT_CACHE", "1");
    } else {
        cmd.env_remove("ALSEM_NO_PREFLIGHT_CACHE");
    }
    let out = cmd.output().expect("alsem runs");
    assert!(out.status.success(), "alsem exited {:?}", out.status);
    String::from_utf8(out.stdout).expect("stdout is utf-8")
}

#[test]
fn cold_warm_and_disabled_runs_are_byte_identical() {
    let ws = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/r0-corpus/ws-d2-uncertain");
    let cache = tempfile::tempdir().expect("tempdir");

    // COLD: the cache dir is empty, so this run computes and then stores.
    let cold = run(ws, Some(cache.path()), false);

    // Precondition, stated executably: the cold run actually WROTE an entry, so
    // the warm run below is genuinely served and not a second cold run. Without
    // this the test would pass vacuously if caching silently stopped working.
    let entries = std::fs::read_dir(cache.path()).expect("cache dir readable").count();
    assert_eq!(entries, 1, "the cold run must persist exactly one cache entry");

    // WARM: same dir, entry present.
    let warm = run(ws, Some(cache.path()), false);

    // DISABLED: cache bypassed entirely.
    let disabled = run(ws, None, true);

    assert_eq!(cold, warm, "a warm cache run must be byte-identical to a cold one");
    assert_eq!(
        cold, disabled,
        "a cache-disabled run must be byte-identical to a cached one"
    );
}
```

- [ ] **Step 3: Register the member**

Add to `tests/cli/main.rs`, in the existing `mod` list, keeping alphabetical order if the file is ordered:

```rust
mod preflight_cache_identity;
```

- [ ] **Step 4: Run it**

Run: `cargo test --test cli preflight_cache_identity`

Expected: PASS. If it fails on the fixture name, list `tests/r0-corpus/` and pick a workspace that `alsem analyze` exits 0 on; update the `ws` constant.

- [ ] **Step 5: Discrimination proof**

Break the cache lookup so a warm run recomputes-but-differs. In `src/program/resolve/preflight_cache.rs`, temporarily make `lookup` return a wrong verdict:

```rust
// TEMPORARY BREAK — revert after the proof
pub fn lookup(_key: &str) -> Option<FreshCoverage> {
    Some(FreshCoverage { unknown: 999, coverage_holds: true, recovered_files: 0, opaque_apps: vec![] })
}
```

Run: `cargo test --test cli preflight_cache_identity`
Expected: **FAIL** on `cold == warm`.

Revert the break, re-run, expect PASS. Record both outcomes.

- [ ] **Step 6: Commit**

```bash
git add tests/cli/preflight_cache_identity.rs tests/cli/main.rs CHANGELOG.md
git commit -m "test(cli): pin cold/warm/cache-disabled output identity

The preflight verdict cache's payload is formatter-visible (opaque_apps
reaches the JSON coverage block), and no existing family exercises the warm
path. Hand-states its precondition (asserts the cold run persisted exactly
one entry, so the warm run is genuinely served rather than a second cold
run). Discrimination proof recorded: a lookup returning a wrong verdict
makes it FAIL."
```

---

### Task 5: `scripts/trace_summary.py`

Rebuilt from scratch twice; a memory note says "worth rebuilding, it is ~40 lines", and the third rebuild will differ subtly from the first two. The durable capability is ranking spans by **SELF** time rather than inclusive total — that is what surfaced 24.8 % of the run (18.9 s) sitting in spans nothing covered, which inclusive ranking had hidden for this track's whole history.

**Files:**
- Create: `scripts/trace_summary.py`
- Modify: `.claude/skills/perf-probe/SKILL.md` (one pointer line)

**Interfaces:**
- Consumes: Chrome-trace JSON emitted by `ALSEM_TRACE=1 ALSEM_TRACE_FILE=<path>`.
- Produces: `scripts/trace_summary.py <trace.json> [...]`, used by Task 8's docs.

- [ ] **Step 1: Create the script**

```python
#!/usr/bin/env python3
"""Summarize an alsem Chrome-trace: SELF time first, inclusive second.

SELF time (a span's duration minus its nested children on the same tid) is the
column that ranks work. Ranking by inclusive total hid 24.8% of an 8020 run --
18.9 s inside `analyze.total` and `l4_l5.run_detectors`, two long-lived brackets
whose children do not tile them -- for this track's entire history.

Usage: scripts/trace_summary.py <trace.json> [<trace.json> ...]
"""
import json
import sys
from collections import defaultdict


def agg(path):
    """Return (inclusive_ms, self_ms, count, peak_mb) keyed by span name."""
    with open(path, "r", encoding="utf-8") as f:
        events = json.load(f)
    stack = defaultdict(list)          # tid -> [name, start_ts, child_us]
    inclusive, selft, count = defaultdict(float), defaultdict(float), defaultdict(int)
    peak = 0
    for e in events:
        tid = e.get("tid", 0)
        if e.get("ph") == "B":
            stack[tid].append([e["name"], e["ts"], 0.0])
        elif e.get("ph") == "E":
            if not stack[tid]:
                continue           # unmatched E (truncated trace) -- ignore
            name, ts0, kids = stack[tid].pop()
            dur = e["ts"] - ts0
            inclusive[name] += dur / 1000.0
            selft[name] += (dur - kids) / 1000.0
            count[name] += 1
            if stack[tid]:
                stack[tid][-1][2] += dur
            pm = (e.get("args") or {}).get("peak_mb")
            if pm and pm > peak:
                peak = pm
    return inclusive, selft, count, peak


def main():
    if len(sys.argv) < 2:
        print(__doc__)
        return 2
    for path in sys.argv[1:]:
        inclusive, selft, count, peak = agg(path)
        root = inclusive.get("analyze.total", 0.0) or 1.0
        print(f"\n=== {path}   analyze.total={root/1000:.1f}s   peak_mb={peak} ===")
        print(f"{'span':<46}{'self ms':>10}{'incl ms':>10}{'n':>6}{'self%':>8}")
        for name, ms in sorted(selft.items(), key=lambda kv: -kv[1]):
            if ms < 40:
                continue
            print(f"{name:<46}{ms:>10.1f}{inclusive[name]:>10.1f}"
                  f"{count[name]:>6}{100.0*ms/root:>7.1f}%")
    return 0


if __name__ == "__main__":
    sys.exit(main())
```

- [ ] **Step 2: Verify against a real trace**

```bash
export TREE_SITTER_AL_PATH=U:/Git/al-call-hierarchy/tree-sitter-al
cargo build --profile release-fast --bin alsem > /tmp/b.log 2>&1
ALSEM_TRACE=1 ALSEM_TRACE_DETAIL=hot ALSEM_TRACE_FILE=/tmp/t.json \
  ./target/release-fast/alsem.exe analyze \
  'U:/Git/DO.Support-SlowDOSetup/DocumentOutput/Cloud' --format json > /dev/null 2>&1
python scripts/trace_summary.py /tmp/t.json
```

Expected: a table whose top rows on DO are `preflight.*` spans, and in which
`analyze.total`'s SELF time is a few ms (it is fully attributed as of
`e7cc156`). If `analyze.total` self is large, the trace predates attribution.

- [ ] **Step 3: Point `perf-probe` at it**

Add one line to `.claude/skills/perf-probe/SKILL.md` in its reporting step:

```markdown
Read the numbers with `python scripts/trace_summary.py <trace.json>` — it ranks
by SELF time, which is the column that finds work. Inclusive ranking hid 24.8%
of an 8020 run.
```

- [ ] **Step 4: Commit**

```bash
git add scripts/trace_summary.py
git commit -m "chore(perf): version the trace summarizer, self-time ranked

Rebuilt ad hoc twice; a memory note says 'worth rebuilding, ~40 lines'.
Ranking spans by SELF time rather than inclusive total is what surfaced
24.8% of an 8020 run sitting in spans nothing covered. Sits beside
scripts/peak_rss.py, the established precedent."
```

---

### Task 6: `scripts/disc-proof.py` + `discrimination-proof` skill

The highest-recurrence defect class here: CLAUDE.md records "5× in one arc, caught by review every time and the suite never", and this session's two hand-rolled runs exposed **three tests that passed while the code was broken** plus a false doc claim. The doctrine is written; the runnable form is missing.

**Files:**
- Create: `scripts/disc-proof.py`
- Create: `.claude/skills/discrimination-proof/SKILL.md`

**Interfaces:**
- Consumes: nothing.
- Produces: `python scripts/disc-proof.py <spec.json>`.

- [ ] **Step 1: Create the driver**

```python
#!/usr/bin/env python3
"""Run discrimination proofs: break the code, watch the test FAIL, revert, watch it PASS.

A test that has never been seen to fail is not evidence. And a proof that comes
back GREEN-WHEN-BROKEN is evidence about the TEST, not the code -- diagnose it,
do not accept it. In the 2026-08-02 arc, 3 of 5 proofs came back green and all
three were real test defects.

Spec file: JSON list of
  {"label": str, "file": str, "test": str, "old": str, "new": str, "count": 1}

Usage: python scripts/disc-proof.py <spec.json>
Exit 0 iff every proof is GOOD (fails when broken, passes when restored).
"""
import io
import json
import subprocess
import sys


def run_test(filt):
    r = subprocess.run(
        ["cargo", "test", "-p", "al-call-hierarchy", "--lib", filt],
        capture_output=True, text=True,
    )
    return r.returncode == 0


def main():
    if len(sys.argv) != 2:
        print(__doc__)
        return 2
    specs = json.load(open(sys.argv[1], encoding="utf-8"))
    ok = True
    for s in specs:
        path, old, new = s["file"], s["old"], s["new"]
        want = s.get("count", 1)
        orig = io.open(path, encoding="utf-8").read()
        n = orig.count(old)
        if n != want:
            # An unasserted scripted break proves nothing, and its green run
            # reads exactly like a passing test. rustfmt reflowing an anchor has
            # silently produced this in the past.
            print(f"BAD   {s['label']:48} PATCH-NOT-UNIQUE (found {n}, want {want})")
            ok = False
            continue
        io.open(path, "w", encoding="utf-8", newline="\n").write(orig.replace(old, new, 1))
        broken_passes = run_test(s["test"])
        io.open(path, "w", encoding="utf-8", newline="\n").write(orig)
        restored_passes = run_test(s["test"])
        assert io.open(path, encoding="utf-8").read() == orig, f"{path} not restored!"
        good = (not broken_passes) and restored_passes
        ok = ok and good
        print(f"{'GOOD' if good else 'BAD':5} {s['label']:48} "
              f"broken={'PASS(!)' if broken_passes else 'FAIL':8} "
              f"restored={'PASS' if restored_passes else 'FAIL(!)'}")
    print("ALL PROOFS GOOD" if ok else "SOME PROOFS BAD -- diagnose the TEST, not just the code")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
```

- [ ] **Step 2: Verify it on a known-good pair**

Create `/tmp/proof.json` targeting a guard that exists today:

```json
[{"label": "self-hash verification in lookup",
  "file": "src/program/resolve/preflight_cache.rs",
  "test": "self_hash_mismatch_recomputes",
  "old": "    if expect != entry.self_hash {",
  "new": "    if false && expect != entry.self_hash {",
  "count": 1}]
```

Run: `python scripts/disc-proof.py /tmp/proof.json`

Expected: `GOOD ... broken=FAIL restored=PASS`, then `ALL PROOFS GOOD`, exit 0.

- [ ] **Step 3: Write the skill**

Create `.claude/skills/discrimination-proof/SKILL.md`:

```markdown
---
name: discrimination-proof
description: Prove a new test can actually fail — break the code, watch the test fail, revert, watch it pass, record both outcomes. Use whenever adding or changing a test, and always before claiming a guard is pinned.
---

Run `python scripts/disc-proof.py <spec.json>` (spec shape in the script's docstring).

## The three ways a proof lies

1. **The anchor no longer matches** — `rustfmt` reflowed the target text, so the
   scripted break patched nothing and the green run looks like a pass. The driver
   asserts the match count for this reason; never bypass it.
2. **The corruption destroys the oracle** — e.g. a test asserting `!= 999` whose
   break rewrites the 999 itself. It then passes for the wrong reason. Corrupt a
   field the assertion does NOT read.
3. **The break is redundant with a second code path** — a real property of the
   code, not a defect. Record it as a stated LIMIT of that test.

## Reading the result

`broken=FAIL restored=PASS` is the only good outcome. **A proof that passes while
broken is evidence about the TEST.** In the 2026-08-02 arc, 3 of 5 came back green
and every one was a real test defect — one was fixed by corrupting a different
field, one by constructing a well-formed entry from a foreign binary instead of
tampering in place, and one by adding a row that isolated the property.

Record BOTH outcomes in the commit message. A test whose proof was never run is
not pinned.
```

- [ ] **Step 4: Commit**

```bash
git add scripts/disc-proof.py
git commit -m "chore(test): version the discrimination-proof driver

CLAUDE.md's longest testing paragraph mandates break/revert proofs; the
runnable form was missing, so it was hand-rolled twice this arc. Those two
runs exposed three tests that passed while the code was broken. The driver
asserts each break's match count, because an unasserted scripted break
proves nothing and its green run reads exactly like a pass."
```

---

### Task 7: Doctrine guards in the existing PreToolUse hook

Two precise-pattern, zero-judgment blocks of things CLAUDE.md flatly forbids, added as entries alongside the existing `| tail` guard rather than as new machinery. Misfire risk is effectively zero because legitimate uses of these spellings do not exist under this repo's rules.

**Files:**
- Modify: `.claude/settings.local.json` (`hooks.PreToolUse`, the `Bash` matcher array)

**Interfaces:** none.

- [ ] **Step 1: Add the `git add -A` guard**

Append a new entry to the `PreToolUse` `Bash` matcher array:

```bash
cmd=$(jq -r '.tool_input.command // empty'); echo "$cmd" | grep -qE 'git add ([-]A|--all|\.)([[:space:]]|$)' && { echo 'Blocked: CLAUDE.md — stage only intended paths, never git add -A/. (the tree currently carries untracked scratch files).' >&2; exit 2; }; exit 0
```

The anchor `([[:space:]]|$)` is load-bearing: without it, `git add ./src/x.rs` would match `\.` and be blocked.

- [ ] **Step 2: Add the bare `cargo fmt` guard**

```bash
cmd=$(jq -r '.tool_input.command // empty'); echo "$cmd" | grep -qE 'cargo fmt' | true; echo "$cmd" | grep -qE 'cargo fmt' && ! echo "$cmd" | grep -qE 'cargo fmt[^&|;]*--check' && { echo 'Blocked: CLAUDE.md — never bare `cargo fmt` (whole-crate churn). Use `rustfmt <file>`, or `cargo fmt --check` (read-only).' >&2; exit 2; }; exit 0
```

Note: `Bash(cargo fmt:*)` is currently in the permissions **allow** list, so the permission system will wave through the exact command doctrine forbids — which is the argument for this guard.

- [ ] **Step 3: Verify both directions**

Ask for each of these and confirm the outcome:

| command | expected |
|---|---|
| `git add -A` | BLOCKED |
| `git add ./src/lib.rs` | allowed |
| `cargo fmt` | BLOCKED |
| `cargo fmt --check` | allowed |
| `rustfmt src/lib.rs` | allowed |

- [ ] **Step 4: No commit**

`.claude/settings.local.json` is gitignored (`.claude/*` with only `!.claude/commands/` re-included). This task is local-only by construction; record it in the plan's completion note rather than in git.

---

### Task 8: CI visibility — post-push report and a scheduled red check

`H2` alone is insufficient: the two-day red persisted *between* pushes, and nothing looks at CI when nobody is pushing. Both halves are informational and non-blocking, so neither carries bypass-training risk.

**Files:**
- Modify: `.claude/settings.local.json` (`hooks.PostToolUse`)
- Create: `scripts/ci-status` (versioned, so the scheduled task is shareable)

**Interfaces:**
- Consumes: `gh` CLI, authenticated.
- Produces: `scripts/ci-status` exiting non-zero when master's last completed CI run is not `success`.

- [ ] **Step 1: Create `scripts/ci-status`**

```bash
#!/usr/bin/env bash
# scripts/ci-status — is master's last COMPLETED CI run green?
#
# Exits non-zero when it is not, so a scheduler can raise an alert. CI was red
# for two days across five merges (2026-07-31..08-02) because nothing looked at
# it between pushes; `gh run list` at push time only reports the PREVIOUS run,
# which is why this scheduled check exists alongside the push-time hook.
set -uo pipefail
branch="${1:-master}"
line=$(gh run list --workflow CI --branch "$branch" --status completed -L1 \
        --json conclusion,headSha,updatedAt \
        --jq '.[0] | "\(.conclusion)\t\(.headSha[0:8])\t\(.updatedAt)"' 2>/dev/null)
if [ -z "$line" ]; then
    echo "ci-status: could not read CI status for $branch" >&2
    exit 2
fi
conclusion=${line%%$'\t'*}
echo "ci-status[$branch]: $line"
[ "$conclusion" = "success" ]
```

- [ ] **Step 2: Verify both directions**

```bash
chmod +x scripts/ci-status
scripts/ci-status master; echo "exit=$?"
```

Expected: prints `success ...` and `exit=0` on current master. To see the failing direction, run it against a branch whose last CI run failed, or temporarily change `success` to `failure` in the final comparison and confirm `exit=1`, then revert.

- [ ] **Step 3: Add the push-time hook**

Append to `.claude/settings.local.json`'s `PostToolUse` array, matcher `Bash`:

```bash
cmd=$(jq -r '.tool_input.command // empty'); echo "$cmd" | grep -qE '^[^#]*git push' && timeout 15 gh run list --workflow CI -L1 --json conclusion,updatedAt --jq '"CI (last completed): \(.[0].conclusion) @ \(.[0].updatedAt)"' 2>/dev/null; exit 0
```

Always `exit 0` — it reports, never blocks, and stale info must never fail a push.

- [ ] **Step 4: Register the scheduled check (manual, user action)**

`scripts/cdo-gate` is already scheduled via Task Scheduler; add a second daily task running:

```
C:\Program Files\Git\bin\bash.exe -lc "cd /u/Git/al-call-hierarchy && scripts/ci-status master"
```

Non-zero exit is the alert signal, matching the existing `cdo-gate` failure channel. **This step is the user's to perform** — it configures their machine, not the repo.

- [ ] **Step 5: Commit**

```bash
git add scripts/ci-status
git commit -m "chore(ci): scripts/ci-status for scheduled red detection

CI stayed red two days across five merges because nothing looked at it
between pushes, and a push-time check only ever reports the PREVIOUS run.
Exits non-zero when master's last completed CI run is not success, so the
existing cdo-gate scheduling habit covers it."
```

---

### Task 9: `stale-doc-hunter` agent

Four findings in one session: a version constant two grammar majors stale, a doc forbidding serialization of a field whose named consumer no longer exists, an OUTSTANDING ranking derived from a superseded profile, and a spec section's own sizing. None of the ten existing agents hunts this — `documentation-engineer` *writes* docs; this one *falsifies* them.

**Files:**
- Create: `.claude/agents/stale-doc-hunter.md`
- Modify: `.claude/skills/arc-capstone/SKILL.md` (dispatch alongside `measurement-auditor`)

**Interfaces:** none.

- [ ] **Step 1: Write the agent**

Create `.claude/agents/stale-doc-hunter.md`:

```markdown
---
name: stale-doc-hunter
description: Falsifies doc claims against the tree — version literals, "consumed by X" claims, numeric/ranking claims from superseded measurements, and fields whose stated constraints name deleted consumers. Read-only; produces evidence, never fixes. Use at arc capstones and before freezing numbers in CHANGELOG/OUTSTANDING.
tools: Read, Grep, Glob, Bash
---

You falsify documentation against the source tree. You never edit anything.

## What to hunt (each shape has burned this repo)

1. **Version literals vs. their source.** e.g. `CACHE_VERSION_GRAMMAR` read
   `"tree-sitter-al-v2.5.2-native"` while the pinned grammar was v3.2.0 — never
   bumped through two upgrades, nothing failed.
2. **"Consumed by X" / "used to key Y" claims where X has zero call sites.**
   e.g. `Origin.ts_id`'s doc forbids serializing it and names L2 op/callsite maps
   as the consumer; that consumer no longer exists and the field is written once
   and read by no production code.
3. **Numeric or ranking claims tied to a superseded measurement.** e.g.
   `OUTSTANDING.md` ranked detector d1 third at 13.9% from a profile taken before
   the fix that made it 1.8%.
4. **Spec/plan sizings contradicted by a later census.**

## Output format — ranked, and PINNABLE first

For each finding: `file:line`, the claim, the artifact that contradicts it, and a
**PINNABLE** tag when the claim could be asserted in a test instead of corrected
in prose.

**Ask "can this be a test instead?" before "how should this sentence read?"** A
version literal, a call-site count, or a numeric threshold should die as an
assert, not a doc edit — a corrected sentence rots again, a test does not.

## Scope discipline

Only claims checkable against the tree. Open-ended "review all docs" produces
noise and gets ignored. If you cannot name the artifact that falsifies a claim,
it is not a finding.
```

- [ ] **Step 2: Wire it into `arc-capstone`**

Add to `.claude/skills/arc-capstone/SKILL.md`, beside the existing `measurement-auditor` dispatch:

```markdown
Dispatch `stale-doc-hunter` in parallel with `measurement-auditor`. Capstones are
when numbers get written down and frozen, which is exactly when a stale one
becomes permanent. Findings tagged PINNABLE get a test, not a prose fix.
```

- [ ] **Step 3: Audition it**

Dispatch `stale-doc-hunter` over `src/engine/gate/cache_prune.rs`, `crates/al-syntax/src/ir/mod.rs` and `docs/OUTSTANDING.md`.

Expected: it independently reports the `ts_id` dead-consumer finding. (`CACHE_VERSION_GRAMMAR` will already be fixed by Task 3 — if it reports that as still stale, Task 3 regressed.) An agent that finds nothing on a corpus with known findings is mis-primed; tighten the checklist and retry.

- [ ] **Step 4: No commit**

`.claude/agents/` is gitignored. Local-only, like Task 7.

---

### Task 10: `xtask gen-syntax` worktree guard

`gen-syntax` run from a worktree fails with a bare `os error 3`, and run from the main checkout it rewrites `crates/al-syntax/src/raw/generated/*` with LF endings where the index holds CRLF — churning a checkout the operator may not have intended to touch. A tool that can dirty a sibling checkout should not rely on operator memory.

**Files:**
- Modify: `crates/xtask/src/main.rs` (the `gen-syntax` entry, at the point it resolves the grammar path)

**Interfaces:** none.

- [ ] **Step 1: Find the path resolution**

Run: `grep -rn "node-types.json\|tree-sitter-al" crates/xtask/src/*.rs`

- [ ] **Step 2: Add the guard**

Before reading `node-types.json`, when the resolved path does not exist:

```rust
if !node_types_path.exists() {
    eprintln!("xtask gen-syntax: {} not found.", node_types_path.display());
    eprintln!();
    eprintln!("This usually means gen-syntax is running from a git WORKTREE:");
    eprintln!("worktrees get no submodule checkout, so tree-sitter-al/ is absent.");
    eprintln!("Run it in the MAIN checkout instead, or set TREE_SITTER_AL_PATH to an");
    eprintln!("already-checked-out grammar. Note it rewrites files in whichever");
    eprintln!("checkout it resolves to.");
    std::process::exit(1);
}
```

- [ ] **Step 3: Verify from a worktree**

```bash
cd <a worktree> && cargo run -p xtask -- gen-syntax; echo "exit=$?"
```

Expected: exit 1, with the worktree explanation — not a bare `os error 3`.

- [ ] **Step 4: Verify the main checkout still works**

```bash
cd U:/Git/al-call-hierarchy && cargo run -p xtask -- gen-syntax && git diff --exit-code crates/al-syntax/src/raw/generated; echo "exit=$?"
```

Expected: exit 0. If `git status` then shows the two generated files as modified with an empty `git diff`, that is the known CRLF/LF artifact — stash it, do not commit it.

- [ ] **Step 5: Commit**

```bash
git add crates/xtask/src/main.rs
git commit -m "fix(xtask): explain the worktree case instead of 'os error 3'

gen-syntax needs the tree-sitter-al submodule, which worktrees do not get.
It failed with a bare path error, and in the main checkout it rewrites
generated files with LF where the index holds CRLF. Fail with instructions."
```

---

## Deferred, with wake conditions

- **`determinism-reviewer` agent** — wake: the first task of the next parallelism arc. The empirical half ships in Task 4 (extend it with a repeated-parallel-run leg once a second parallel path lands). Writing the reviewer now is building taxonomy before the population exists.
- **In-binary preflight-cache stats in the trace JSON** — wake: the next perf arc that measures `analyze` end-to-end. The unbypassable form of a measurement-hygiene guard: emit cache hit/miss into the trace so no launch path can hide a warm replay, and `measurement-auditor` gets an artifact instead of an absence.

## Killed, with reasons (do not revive without new evidence)

- **Golden-edit warning hook** — would fire on a documented-correct action (a new r4 fixture *requires* a hand-committed seed because regen cannot mint one). A hook that fires on correct behaviour trains bypassing. Already triple-covered by the blind-regen guard, the PostToolUse golden-impact reminder, and the pre-commit `check-goldens` gate.
- **`ALSEM_TRACE`-without-cache-disable hook** — cannot see its target: probe env vars live inside `.ps1` files launched via `Start-Process`, invisible to a Bash-command grep. A guard that cannot see its target manufactures false confidence.
- **`doctrine-auditor` agent** — fails its own audition against this session's defect list: the fmt miss was not a doctrine violation, stale docs are `stale-doc-hunter`'s job, and on the clippy mismatch **CLAUDE.md's own Lint line was the wrong one, so a doctrine-auditor would have blessed the defect**. Task 2 fixes the doc instead.
- **`ci-parity` agent** — a step, not a teammate. With Task 2 it is `scripts/ci-steps all` backgrounded from `arc-capstone`.
- **`paired-ab` standalone skill** — would sit between `perf-probe` (run mechanics) and `measurement-auditor` (claim audit), overlapping both. Add a "Comparing two builds" section to `perf-probe` instead: two binaries from one tree, alternating runs, per-pair ratios AND median, a named control, and "if the control drifts, say so and widen the claim".

## Self-review notes

- **Coverage:** Tasks 1–10 cover all 8 of the reviewed BUILD items plus 3 of the 5 MISSED items; the other 2 are in Deferred with wake conditions, and 5 candidates are in Killed with reasons.
- **Local-only tasks (7, 9, and the skill halves of 1/5/6):** `.claude/*` is gitignored except `commands/`, so these bind this machine only. Every *enforcement* item (1, 2, 3, 4, 8, 10) lands in a versioned, shared layer — that split is deliberate, not an oversight.
- **Ordering:** Task 1 first (cheapest, highest payback, closes the actual outage). Task 2 second (makes the class impossible). Task 3 third because it may be a live defect rather than doc rot.
