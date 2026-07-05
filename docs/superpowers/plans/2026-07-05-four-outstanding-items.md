# Four Outstanding Items — Closure Plan

> **For agentic workers:** Items 1–2 are user-gated housekeeping executed inline (no subagents needed).
> Item 3 is an SDD arc: REQUIRED SUB-SKILL superpowers:subagent-driven-development, one implementer per task.
> Item 4 is a scoping/brainstorm phase: REQUIRED SUB-SKILL superpowers:brainstorming — no implementation until its spec is approved.

**Goal:** Close the four post-roadmap items: (1) worktree/branch/stash housekeeping, (2) the four untracked files, (3) retirement of the legacy al-sem parity scaffolding, (4) BC-Brain integration scoping.

**Architecture:** Items 1–2 are batched gated commands. Item 3 makes the test suite fully self-contained (zero reads from `U:\Git\al-sem` at test time, zero skip-gates) while keeping every Rust-owned golden and the live `alsem` product CLI untouched. Item 4 produces a written ingestion-contract spec before any code.

**Tech Stack:** git, PowerShell/bash, Rust (cargo test harnesses), GitHub Actions.

## Global Constraints

- Never merge/push to `master` without explicit user request. Stage only named paths; never `git add -A`.
- All destructive git operations (worktree remove --force, branch -D, stash drop) execute ONLY after the user's explicit go for that step group.
- `U:\Git\al-sem` is frozen: never write into it; test-time reads are what Item 3 eliminates. One-time read-only copies OUT of it for fixture vendoring are permitted.
- All in-repo golden directories are Rust-owned live baselines — Item 3 deletes NONE of them.
- Item 3 gates per task: `cargo test --workspace` green, `cargo clippy --release --all-features --all-targets -- -D warnings` clean, CDO frozen-baseline byte-identical (aldump JSON SHA-256 `67910e99…913f4f`, L3 digest `b545ae11…64f9`, real-unknown 0/18108, ambiguousResolved 0, genuine_wrong 0, recoveredFiles 0).
- `rustfmt <file>` per file, never `cargo fmt`. CHANGELOG.md updated per behavior-affecting task.

---

## Item 1 — Worktree / branch / stash housekeeping (user-gated batch)

All evidence gathered 2026-07-05. Everything below is destructive; execute each step group only on the user's go.

### Evidence table — 9 worktrees, all disposable

| Worktree | State | Verdict |
|---|---|---|
| `Temp/alch-parent` | detached at `6b56107` (ancestor of master); 2534 working-tree deletions + 1 submodule-pointer mod | Junk. Nothing salvageable — the commit is safe in history. |
| `Temp/…/scratchpad/wt-5cc7c2a` | 2 modified golden snapshots | Scratch from a merged arc. |
| `Temp/…/scratchpad/wt-b3ad100` | 1 modified `program_resolve_harness.rs` | Scratch. |
| `Temp/…/scratchpad/wt-parent` | clean | Removable (submodule guard still requires `--force` on git 2.53). |
| `Temp/…/scratchpad/wt-prefix` | 1 modified `resolver.rs` | Scratch. |
| `Temp/…/scratchpad/wt-verify-merge` | 2 modified files | Merge-verify scratch; the merge landed. |
| `.claude/worktrees/agent-a774227…` | 190 modified goldens + 28 untracked; branch has 2 unmerged engine-d22 commits | Remove worktree, KEEP branch (see step 1.3). |
| `.claude/worktrees/agent-aa0f7dd…` | clean; branch fully merged | Removable; branch deletable. |
| `U:/Git/al-call-hierarchy-wt-b3ad100` | 1 modified `program_resolve_harness.rs` | Scratch duplicate of wt-b3ad100. |

### Step 1.1 — Remove all 9 worktrees ⏸ USER GATE

```bash
cd /u/Git/al-call-hierarchy
git worktree remove --force "C:/Users/SShadowS/AppData/Local/Temp/alch-parent"
git worktree remove --force "C:/Users/SShadowS/AppData/Local/Temp/claude/U--Git-al-call-hierarchy/653c030b-4798-4d43-adf2-42069c78d6c7/scratchpad/wt-5cc7c2a"
git worktree remove --force "C:/Users/SShadowS/AppData/Local/Temp/claude/U--Git-al-call-hierarchy/653c030b-4798-4d43-adf2-42069c78d6c7/scratchpad/wt-b3ad100"
git worktree remove --force "C:/Users/SShadowS/AppData/Local/Temp/claude/U--Git-al-call-hierarchy/653c030b-4798-4d43-adf2-42069c78d6c7/scratchpad/wt-parent"
git worktree remove --force "C:/Users/SShadowS/AppData/Local/Temp/claude/U--Git-al-call-hierarchy/653c030b-4798-4d43-adf2-42069c78d6c7/scratchpad/wt-prefix"
git worktree remove --force "C:/Users/SShadowS/AppData/Local/Temp/claude/U--Git-al-call-hierarchy/653c030b-4798-4d43-adf2-42069c78d6c7/scratchpad/wt-verify-merge"
git worktree remove --force "U:/Git/al-call-hierarchy/.claude/worktrees/agent-a774227bcad2385e0"
git worktree remove --force "U:/Git/al-call-hierarchy/.claude/worktrees/agent-aa0f7dd49009aae0b"
git worktree remove --force "U:/Git/al-call-hierarchy-wt-b3ad100"
git worktree prune
git worktree list   # expect: only U:/Git/al-call-hierarchy
```

If any removal leaves a directory behind (git 2.53 submodule quirk), delete the leftover directory manually and re-run `git worktree prune`. Disk note: 6 of these carry `target/` directories — expect a multi-GB reclaim on U:/Temp.

### Step 1.2 — Delete merged branches ⏸ USER GATE

All verified `git log <branch> --not master` empty on 2026-07-05:

```bash
git branch -d engine graphify-export backup/pre-graphify-integration \
  worktree-agent-a00ee980f60dada98 worktree-agent-aa0f7dd49009aae0b \
  feat/program-1b3a-abi-ingestion-agent feat/bcbrain-export-unknown-reason
# remote counterparts (origin = SShadowS fork — verified push target):
git push origin --delete engine feat/bcbrain-export-unknown-reason
```

### Step 1.3 — Unmerged branches: user decision per branch

| Branch | Unmerged | Assessment |
|---|---|---|
| `backup/pre-amend-1dd60d1` | 1 commit | Backup of a pre-amend state; the amend landed long ago. Superseded — recommend delete (`-D`). |
| `worktree-agent-a19c709662bf50c32` | 1 commit | Old agent scratch. Inspect `git show`, then decide. |
| `worktree-agent-a774227bcad2385e0` | 2 commits (engine-d22 Phase-4 receiver work) | Functionality re-landed via later arcs. Recommend delete after a one-line `git log -p` skim confirms nothing unique. |

### Step 1.4 — Stash triage ⏸ USER GATE

20 stashes. Drop the 13 self-labeled discardables **by message match, highest index first** (indices shift after each drop — the commands below are ordered accordingly and each message is unique):

```bash
# verify message before each drop:
git stash list
git stash drop 'stash@{18}'  # r1b-cleanup: stray repo-wide cargo-fmt reformats (cosmetic)
git stash drop 'stash@{17}'  # r1b-t3: accidental fmt reformat of LSP files (discard)
git stash drop 'stash@{16}'  # stray r0-goldens manifest provenance bump
git stash drop 'stash@{14}'  # stray cargo-fmt reformats of non-E1 files (discardable)
git stash drop 'stash@{13}'  # stray cargo-fmt reformat app_package (discardable)
git stash drop 'stash@{12}'  # stray cargo-fmt churn (8 files, cosmetic) - discardable
git stash drop 'stash@{11}'  # r3a3-goldens-restore-tmp
git stash drop 'stash@{9}'   # wip-regen-phase2 (engine-d22 era)
git stash drop 'stash@{8}'   # junk-partial-adcf360-discard-me
git stash drop 'stash@{7}'   # throwaway-debug-instrumentation
git stash drop 'stash@{5}'   # xRec-seeding-reverted-no-benefit
git stash drop 'stash@{3}'   # regen-eol-churn-discardable
git stash drop 'stash@{2}'   # cli-b-digest-regen-discardable
git stash drop 'stash@{0}'   # buggy receiver_tier script output - discard
```

Kept for user review (real WIP, ambiguous value): `stash@{1}` (beyond-1b3b lookup-precedence WIP), `@{4}` (engine-d22 review-fix WIP), `@{6}` (perf-profiling markers), `@{10}` (engine-g8 temp-state WIP), `@{15}` (r2.5a .app reader WIP), `@{19}` (engine-branch genesis WIP). All predate arcs that re-landed their functionality — likely all droppable, but each deserves a 30-second `git stash show -p` look.

### Step 1.5 — Disk survey (non-destructive, run anytime)

```bash
du -sh /u/Git/al-call-hierarchy/target /u/Git/tree-sitter-al 2>/dev/null
# cargo clean in the main repo only if the user wants the reclaim; next build repays with a full LTO rebuild (~minutes).
```

---

## Item 2 — The four untracked files — ✅ DONE 2026-07-05 (workflow fixed+committed `416489e`; both cleanup scripts and the stale coverage plan deleted)

### 2.1 `.github/workflows/build-and-deploy.yml` — FIX + COMMIT (recommended) or delete

Purpose is real (ship engine binaries into `SShadowS/claude-code-lsps` plugin repo) but the draft has 3 defects:

1. `branches: [main]` — this repo's default branch is `master`; the workflow would never fire. Change to `branches: [master]`.
2. `uses: dtolnay/rust-action@stable` (3 occurrences) — nonexistent action. Change to `uses: dtolnay/rust-toolchain@stable` (what ci.yml/release.yml already use).
3. No test gate — release.yml refuses to build untested binaries; this workflow should too. Add the same `test` job from release.yml and give all three build jobs `needs: test`.

Also required before it can work: the `CLAUDE_CODE_LSPS_TOKEN` secret must exist in the GitHub repo settings (user action — a fine-grained PAT with write access to `SShadowS/claude-code-lsps`).

Commit: `ci: build-and-deploy workflow — ship binaries to claude-code-lsps (fixed branch trigger, toolchain action, test gate)`.

### 2.2 `cleanup-grammar-branch.ps1` + `cleanup-owned-ir.ps1` — DELETE

Every target verified gone on 2026-07-05: `feat/owned-syntax-ir` branch deleted, `U:\Git\al-call-hierarchy-ir` worktree gone, grammar `owned-ir-grammar-fixes` branch gone, the reformat stash dropped. Both scripts are dead weight; they were never committed, so deletion is final — which is fine, they have nothing left to do.

```bash
rm /u/Git/al-call-hierarchy/cleanup-grammar-branch.ps1 /u/Git/al-call-hierarchy/cleanup-owned-ir.ps1
```

### 2.3 `docs/superpowers/plans/2026-03-28-test-coverage-improvement.md` — DELETE (stale)

Written pre-owned-IR-migration; its line references and architecture assumptions (tree-sitter-direct parser tests) no longer describe the codebase. Never committed. If a coverage push is still wanted, it should be brainstormed fresh against the IR front-end — a stale plan is worse than no plan.

```bash
rm /u/Git/al-call-hierarchy/docs/superpowers/plans/2026-03-28-test-coverage-improvement.md
```

---

## Item 3 — Legacy al-sem parity retirement (SDD arc, branch `feat/alsem-parity-retirement`)

**Scope (from the 2026-07-05 inventory):** no product/runtime code depends on al-sem. The legacy surface is exactly:
(a) 5 test files reading fixtures/goldens live from `U:\Git\al-sem` (all skip-gated);
(b) empty `KNOWN_DIVERGENCES.json` + ~600–700 LOC of allowlist scaffolding across 13 harnesses;
(c) 14 dormant `#[ignore]` `AL_SEM_DIR` refresh functions (~500–700 LOC, `r2_5b_refresh.rs` is 100% refresh);
(d) ~120 LOC of src parity shims (`src/engine/gate/version.rs`, `cache_prune.rs:61–72`, `format_json.rs:238–248`, `src/bin/alsem.rs` `--dump-model` stub);
(e) 2 migration-era docs (~2500 lines).

**NOT in scope:** all 24 MB of in-repo goldens (Rust-owned baselines — every one stays); the `alsem` product CLI itself.

**North-star acceptance:** `cargo test --workspace` runs the identical set of tests on a machine where `U:\Git\al-sem` does not exist — zero skips, zero al-sem path references outside `docs/history/`.

### Task 3.1 — Vendor the live-read corpora in-repo

**Files:**
- Create: `tests/fixtures/ws-d2/` (copy of `U:\Git\al-sem\test\fixtures\ws-d2`)
- Create: `tests/fixtures/cli-c-policy/` (copy of the al-sem fixture workspaces `cli_c_policy_differential.rs` enumerates)
- Create: `tests/cli-b-goldens/diff/` (regenerated from Rust output via the harness's refresh path — NOT copied from al-sem)
- Create: `tests/cli-c-goldens/cache/` missing entries (the fallback set), regenerated from Rust output
- Modify: `tests/aldump_smoke.rs`, `tests/al2dump_smoke.rs`, `tests/cli_b_diff_differential.rs`, `tests/cli_c_policy_differential.rs`, `tests/cli_c_cache_differential.rs` — point at the in-repo paths, delete the skip-gates

**Method:** input FIXTURES (AL source workspaces) are copied bytes from the frozen al-sem checkout — a one-time read-only migration. GOLDENS are regenerated from THIS engine's output (the tests currently pass byte-identical against the al-sem copies, so regen-from-Rust and copy produce the same bytes — regen is chosen because it exercises the `REGEN_TEMP_GOLDENS=1` path that becomes the only refresh mechanism). The two aldump/al2dump goldens (`ws-d2.l3eg`, `ws-d2.l2`) are likewise regenerated in-repo (`tests/goldens/` or the harness's existing dir).

**Gate:** full suite green with `AL_SEM_DIR` set to a nonexistent path — proves no live read remains in these five files.

### Task 3.2 — Fix the `policySource` absolute-path defect + rebaseline

`tests/cli-c-policy-goldens/ws-policy-custom.custom.{human.txt,json}` embed `auto:U:\Git\al-sem\test\fixtures\…\al-sem.policy.yaml`. Root cause: the CLI emits the auto-detected policy path as an absolute machine path — a reproducibility defect, not just a golden problem. Make `policySource` workspace-relative in the product output (`src/engine/gate/` policy plumbing), rebaseline the goldens (now containing `auto:al-sem.policy.yaml` or similar relative form), CHANGELOG under Fixed.

### Task 3.3 — Remove KNOWN_DIVERGENCES machinery

Delete `KNOWN_DIVERGENCES.json`; in each of the 13 harnesses (`differential.rs`, `r4_differential.rs`, `r2_5a_differential.rs`, `r2_5b_{cg,cov,eg,rt}_differential.rs`, `r3a{1,2,2_trace,3,4,5}_differential.rs`) delete the local `KnownDivergence` struct + `load_known_divergences()` + gating, collapsing each comparison to a plain byte-assert. The r3a4/r3a5 "allowlist must be empty" exit gates become vacuous — delete them too. Remove the `KNOWN_DIVERGENCES.json` paragraph from CLAUDE.md.

### Task 3.4 — Remove the 14 refresh functions + `r2_5b_refresh.rs`

Every harness keeps (or gains) a `REGEN_TEMP_GOLDENS=1` regen path that regenerates from RUST output; the al-sem/Bun refresh functions (`AL_SEM_DIR`-driven) are deleted wholesale. Note `cli_a_stats_differential.rs:345` already regens from Rust — it is the model. Delete `tests/r2_5b_refresh.rs` entirely. After this task: `grep -r "AL_SEM_DIR" tests/ src/` is empty.

### Task 3.5 — Retire the src parity shims (behavior decisions, each on its merits)

- `src/engine/gate/version.rs`: `driver.version` becomes `CARGO_PKG_VERSION` (honest product identity) with env override renamed to an engine-owned name (`ALSEM_VERSION_OVERRIDE`) used by the cli-a harnesses for byte-stable goldens. Rebaseline every golden embedding `0.0.12`.
- `src/engine/gate/format_json.rs:238–248`: the VERSION001-suppression bug-for-bug shim — decide the honest behavior (emit the diagnostic whenever it is true), remove the suppression, rebaseline.
- `src/engine/gate/cache_prune.rs:61–72`: keep the fingerprint semantics, rename `AL_SEM_RELEASE`/`AL_SEM_DEV_FINGERPRINT` to engine-owned names; the `cache-versions.ts`-pinned consts stay (they version OUR cache format now) with comments rewritten to state that.
- `src/bin/alsem.rs` `--dump-model` stub: delete the hidden flag and its CONFIG_ERROR stub ("use the TS CLI" is a dead instruction); update any CLI-contract test that asserted the rejection.

### Task 3.6 — Docs + capstone

Move `docs/engine-migration.md` + `docs/engine-gaps.md` to `docs/history/` with an ARCHIVED header (git history preserves them; the move signals they are narrative, not instruction). Rewrite CLAUDE.md's "Testing Philosophy & Goldens" bullets that reference live al-sem reads (the "LEGACY tests still pointing at the al-sem repo" sentence becomes false — delete it). Final gates: full suite with al-sem absent (rename the checkout temporarily or run with `AL_SEM_DIR=/nonexistent`), clippy, CDO frozen-baseline byte-identical, `grep -ri "U:\\\\Git\\\\al-sem\|AL_SEM_DIR" src/ tests/` empty. CHANGELOG. Whole-branch review, then the 4-option merge menu.

---

## Item 4 — BC-Brain integration scoping (brainstorm first, no code)

**Seam facts (2026-07-05):** bc-brain (`U:\Git\bc-brain`, TS/Bun, layered SQLite KB over MCP) SPEC grounds everything in "a precise whole-program call graph resolved by the AL engine" but no engine artifact contract exists yet. Engine-side surface today: `src/program/graphify_export.rs` (1512 LOC, graph JSON export), aldump L3/CDO outputs, the merged `feat/bcbrain-export-unknown-reason` branch (unknown-reason fields already exported for bc-brain's benefit). bc-brain has layers/subset/serve/eval CLI machinery expecting per-layer artifacts.

**Why brainstorm, not build:** the ingestion contract is a cross-repo product design with open decisions only the user can settle:

1. **Artifact shape** — one graph JSON per app layer (aligned with bc-brain's layer-per-.app model) vs one whole-setup export bc-brain splits itself.
2. **Contract owner** — schema versioned in the engine repo (bc-brain consumes) or in bc-brain (engine conforms)?
3. **Delivery** — engine CLI command (`aldump --bcbrain-export <out>`?) invoked by bc-brain's build pipeline vs bc-brain shelling the engine as a library-style tool.
4. **Content cut** — full edge taxonomy (DispatchShape × Evidence, honest-residual classes) or a distilled subset for KB queries; do trigger/event data-flow edges ship in v1?
5. **Version pinning** — how the artifact records engine version + grammar version + workspace closure hash so bc-brain layers stay reproducible.

**Deliverable:** a brainstorming session producing `docs/SPEC-engine-ingestion.md` in the bc-brain repo (contract owner default: bc-brain, engine conforms) + a matching engine-side plan here (likely: one exporter task extending `graphify_export.rs` or a new `bcbrain_export.rs`, with golden-backed schema tests). Then standard spec → plan → SDD in each repo.

**Recommended order overall:** Item 2 (minutes) → Item 1 (one gated batch) → Item 3 (one SDD arc) → Item 4 (brainstorm, then its own arcs).
