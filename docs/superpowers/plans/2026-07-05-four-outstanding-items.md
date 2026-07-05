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

## Item 1 — Worktree / branch / stash housekeeping — ✅ DONE 2026-07-05 (9 worktrees removed; 11 branches deleted incl. 3 superseded-unmerged + 2 remote; 14 discardable stashes dropped, 6 WIP kept; cargo clean reclaimed 91G — U: now 136G free)

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

> **Revision v2 (2026-07-05):** task list rewritten after external review round 1 (gpt-5.5-pro +
> gemini-3.1-pro, both thinking-high). Round-1 findings incorporated: witness-diff requirement
> (anti-laundering), fixture-orphaning check, display-vs-capability version split for VERSION001,
> `ALCH_*` env prefix, behavior-decisions-before-regen task order, broadened capstone audit,
> CLAUDE.md:128-130 stale source-of-truth line, `alsem`-CLI scope-wording contradiction, CDO-gate
> clarification for metadata-bearing tasks.

**Scope (from the 2026-07-05 inventory):** no product/runtime code depends on al-sem. The legacy surface is exactly:
(a) 5 test files reading fixtures/goldens live from `U:\Git\al-sem` (all skip-gated);
(b) empty `KNOWN_DIVERGENCES.json` + ~600–700 LOC of allowlist scaffolding across 13 harnesses;
(c) 14 dormant `#[ignore]` `AL_SEM_DIR` refresh functions (~500–700 LOC, `r2_5b_refresh.rs` is 100% refresh);
(d) ~120 LOC of src parity shims (`src/engine/gate/version.rs`, `cache_prune.rs:61–72`, `format_json.rs:238–248`, `src/bin/alsem.rs` `--dump-model` stub);
(e) 2 migration-era docs (~2500 lines).

**NOT in scope:** all 24 MB of in-repo goldens (Rust-owned baselines — every one stays). The `alsem` product CLI remains a live product — only its hidden legacy `--dump-model` parity stub is removed (Task 3.2); nothing else in the CLI changes.

**North-star acceptance:** `cargo test --workspace` runs the identical set of tests on a machine where `U:\Git\al-sem` does not exist — zero skips (mechanically verified, not asserted), and the broadened legacy-token audit (Task 3.6) is clean outside `docs/history/` and this plan file (whose al-sem path mentions are inventory, exempt by name).

**CDO-gate clarification:** the frozen-baseline byte-identical requirement holds absolutely for Tasks 3.0, 3.1, 3.3, 3.4, 3.5. For Task 3.2 (which changes emitted version metadata) the gate is: aldump JSON + L3 digest EXPECTED byte-identical (version.rs is gate-output plumbing, not aldump plumbing) — if they move, the diff must consist solely of the approved version-field change, itemized in the task report; any other byte is a stop-the-line failure.

### Task 3.0 — Preflight witness (run while `U:\Git\al-sem` is still present)

1. Record the al-sem checkout's HEAD commit (`git -C U:/Git/al-sem rev-parse HEAD`) in the witness manifest.
2. Run the 5 live-read test files with the checkout present and PROVE none took its skip path (the gates are silent `return`-style skips, invisible to cargo — instrument by asserting on their skip-log lines or temporarily panicking the skip arms). All 5 must run their real assertions and pass.
3. Copy the live-read al-sem-side inputs to a witness area `.superpowers/sdd/alsem-witness/` (gitignored): `scripts/cli-b-goldens/diff/**`, `scripts/r2c-goldens/ws-d2.l3eg.golden.json`, `scripts/r1a-goldens/ws-d2.l2.golden.json`, the cli-c cache fallback set, plus a file listing of the fixture trees to be vendored (`test/fixtures/ws-d2`, the cli-c policy fixture workspaces) with per-file SHA-256.
4. Verify `REGEN_TEMP_GOLDENS=1` regen paths exist and work for the affected harnesses BEFORE anything is deleted (dry-run into a temp dir; must reproduce the current in-repo goldens byte-identically).

**Gate:** witness manifest complete; full suite green; CDO byte-identical (nothing changed yet).

### Task 3.1 — Fix the `policySource` absolute-path defect + rebaseline

`tests/cli-c-policy-goldens/ws-policy-custom.custom.{human.txt,json}` embed `auto:U:\Git\al-sem\test\fixtures\…\al-sem.policy.yaml`. Root cause: the CLI emits the auto-detected policy path as an absolute machine path — a reproducibility defect in product output. **Contract (exact, not "or similar"):** `policySource` becomes the policy file's path relative to the analyzed workspace root, forward slashes on all platforms, no drive letters, no `.`/`..` segments; a policy file outside the workspace root falls back to its bare filename (and this case is unit-tested). If the field also surfaces in SARIF, it is emitted as a SARIF-relative artifact reference, not a bare string with an ambiguous base. Rebaseline the cli-c policy goldens; CHANGELOG under Fixed. Runs BEFORE the vendoring regen so the moved fixtures regenerate path-clean on the first pass.

### Task 3.2 — Retire the src parity shims (identity decisions BEFORE any broad regen)

- **Display vs capability split (the round-1 core finding):** the version a golden pins for byte-stability is DISPLAY metadata; any diagnostic or compatibility logic must evaluate the engine's ACTUAL version/capability. Concretely:
  - `src/engine/gate/version.rs`: SARIF/JSON `driver.version` becomes `CARGO_PKG_VERSION`. The test-only display override is renamed `ALCH_DRIVER_VERSION_OVERRIDE` (engine-owned `ALCH_` namespace — NOT `ALSEM_*`, which reads as legacy) and is documented as cosmetic/golden-stability-only. Every in-repo golden embedding `0.0.12` rebaselines once, with the harnesses pinning the override so future `Cargo.toml` bumps cause zero golden churn.
  - `src/engine/gate/format_json.rs:238–248` (VERSION001 suppression): first establish what VERSION001 actually evaluates. If it currently reads the DISPLAYED version, that is the root defect — repoint it at the actual engine version, after which the suppression shim is dead code and is deleted (no false diagnostics under a cosmetic override, no suppression needed). If VERSION001 turns out to be semantically tied to the override in some other way, STOP and record the finding — do not blindly delete.
- `src/engine/gate/cache_prune.rs:61–72`: keep the fingerprint semantics; rename `AL_SEM_RELEASE`/`AL_SEM_DEV_FINGERPRINT` to `ALCH_RELEASE`/`ALCH_DEV_FINGERPRINT`. Grep `.github/workflows/`, scripts, and docs for the old names and update in the same commit (a stale CI env name = permanent cache misses). The `cache-versions.ts`-pinned consts stay (they version OUR cache format now) with comments rewritten to state that. Old env names are removed outright, not aliased — consequence is a one-time cache re-fingerprint, not a correctness hazard, and this is stated in the CHANGELOG.
- `src/bin/alsem.rs` `--dump-model` stub: delete the hidden flag and its CONFIG_ERROR stub ("use the TS CLI" is a dead instruction); update any CLI-contract test asserting the rejection.

**Gate:** full suite green; clippy; CDO per the clarification above (expected byte-identical; only the itemized version-field diff is acceptable if not).

### Task 3.3 — Vendor the live-read corpora in-repo (witness-diffed)

**Files:**
- Create: `tests/fixtures/ws-d2/` (copied bytes from the frozen checkout) + `PROVENANCE.md` (source path, al-sem HEAD commit from Task 3.0, copy date, "historical fixture, not a live oracle")
- Create: `tests/fixtures/cli-c-policy/` (the fixture workspaces `cli_c_policy_differential.rs` enumerates) + `PROVENANCE.md`
- Create: `tests/cli-b-goldens/diff/`, the missing `tests/cli-c-goldens/cache/` fallback entries, and in-repo homes for the two aldump/al2dump goldens — all regenerated from THIS engine's output via the Task-3.0-verified regen paths
- Modify: `tests/aldump_smoke.rs`, `tests/al2dump_smoke.rs`, `tests/cli_b_diff_differential.rs`, `tests/cli_c_policy_differential.rs`, `tests/cli_c_cache_differential.rs` — point at the in-repo paths, DELETE the skip-gates (tests now hard-require their inputs)

**Anti-laundering protocol (mandatory, the round-1 headline finding):**
1. **Fixture-orphaning check first:** before regenerating anything, verify each vendored fixture tree is self-contained — if a workspace references sibling context (`.alpackages`, dependency `.app`s, adjacent config) that lived outside the copied tree in al-sem, vendor that too. Detection: run the harness against the vendored copy and compare its output against the same harness run against the original al-sem-path fixture — they must be byte-identical modulo the Task-3.1 path normalization.
2. **Witness diff:** every regenerated golden is diffed against its Task-3.0 witness copy. Permitted differences: the Task-3.1 `policySource` normalization and the Task-3.2 version fields — each diff hunk classified and listed in the task report. ANY other difference is a latent divergence that the old skip-gated setup masked: stop, investigate, resolve on the merits (engine bug → fix; intentional Rust-ahead behavior → document the adjudication) before proceeding.
3. **Graph-shape assert:** for graph-bearing outputs (the ws-d2 l3eg golden), the unknown/resolved edge counts in the regenerated golden must equal the witness's — the specific silent-degradation vector (orphaned fixture → resolution downgrades → regen blesses the loss) is checked by number, not by eyeball.

**Gate:** full suite green with `AL_SEM_DIR` pointing at a nonexistent path AND with the harnesses' hardcoded-path constants gone (grep-verified); CDO byte-identical.

### Task 3.4 — Remove KNOWN_DIVERGENCES machinery

Delete `KNOWN_DIVERGENCES.json`; in each of the 13 harnesses (`differential.rs`, `r4_differential.rs`, `r2_5a_differential.rs`, `r2_5b_{cg,cov,eg,rt}_differential.rs`, `r3a{1,2,2_trace,3,4,5}_differential.rs`) delete the local `KnownDivergence` struct + `load_known_divergences()` + gating, collapsing each comparison to a plain byte-assert. The r3a4/r3a5 "allowlist must be empty" exit gates become vacuous — delete them too. Remove the `KNOWN_DIVERGENCES.json` paragraph from CLAUDE.md.

### Task 3.5 — Remove the 14 refresh functions + `r2_5b_refresh.rs`

Every harness keeps (or gains) a `REGEN_TEMP_GOLDENS=1` regen path regenerating from RUST output (Task 3.0 verified these); the al-sem/Bun refresh functions (`AL_SEM_DIR`-driven) are deleted wholesale. Note `cli_a_stats_differential.rs:345` already regens from Rust — it is the model. Delete `tests/r2_5b_refresh.rs` entirely. After this task: `grep -r "AL_SEM_DIR" tests/ src/` is empty.

### Task 3.6 — Docs + capstone

- Move `docs/engine-migration.md` + `docs/engine-gaps.md` to `docs/history/` with an ARCHIVED header.
- CLAUDE.md: rewrite the "Testing Philosophy & Goldens" al-sem bullets (the "LEGACY tests still pointing at the al-sem repo" sentence becomes false — delete it) AND fix the stale grammar-migrations line "the goldens are the al-sem TS reference output … source of truth" (CLAUDE.md:128–130), which contradicts the Rust-owned doctrine.
- **Zero-skip mechanical verification:** temporarily rename `U:\Git\al-sem` (restore after), run the full suite, and assert the test COUNT equals the with-checkout count from Task 3.0 — plus grep the five former live-read files for any surviving skip-string/early-return pattern.
- **Broadened legacy-token audit** (round-1 finding — the old grep was too weak):
  ```bash
  rg -n --hidden --glob '!target/**' --glob '!docs/history/**' \
     --glob '!docs/superpowers/plans/2026-07-05-four-outstanding-items.md' \
     'U:[/\\]Git[/\\]al-sem|AL_SEM_DIR|AL_SEM_VERSION_OVERRIDE|AL_SEM_RELEASE|AL_SEM_DEV_FINGERPRINT|DEFAULT_ALSEM_VERSION|KNOWN_DIVERGENCES|use the TS CLI|dump-model' .
  ```
  must return nothing. (`bun` is checked case-sensitively in tests/ only — the word appears legitimately elsewhere.)
- Final gates: full suite, clippy, CDO frozen-baseline byte-identical, CHANGELOG. Whole-branch review, then the 4-option merge menu.
- Delete the witness dir `.superpowers/sdd/alsem-witness/` only AFTER the whole-branch review passes.

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
