# Four Outstanding Items — Closure Plan

> **For agentic workers:** Items 1–2 are user-gated housekeeping executed inline (no subagents needed).
> Item 3 is an SDD arc: REQUIRED SUB-SKILL superpowers:subagent-driven-development, one implementer per task.
> Item 4 is a scoping/brainstorm phase: REQUIRED SUB-SKILL superpowers:brainstorming — no implementation until its spec is approved.

**Goal:** Close the four post-roadmap items: (1) worktree/branch/stash housekeeping, (2) the four untracked files, (3) retirement of the legacy al-sem parity scaffolding, (4) BC-Brain integration scoping.

**Architecture:** Items 1–2 are batched gated commands. Item 3 makes the test suite fully self-contained (zero reads from `U:\Git\al-sem` at test time, zero skip-gates) while keeping every Rust-owned golden untouched and the live `alsem` product CLI intact except for the removal of its hidden legacy `--dump-model` parity stub. Item 4 produces a written ingestion-contract spec before any code.

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

> **Revision v2.1 (2026-07-05):** task list rewritten after external review round 1, then patched
> after round 2 (gpt-5.5-pro + gemini-3.1-pro, both thinking-high, both rounds). Round-2 patches:
> final-state count comparison (not Task-3.0 baseline), live-only-golden dry-run split, line-9 CLI
> scope wording, `dump[-_]model` + explicit bun audit commands, witness `git check-ignore` gate,
> cache-consts comment-only clarification, `tests/fixtures/** -text` byte-preservation rule. Round-1 findings incorporated: witness-diff requirement
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

> **Task 3.0 finding (2026-07-05, executed): `U:\Git\al-sem` was already gone.** The checkout was
> archived to `U:\Git\al-sem-OBOLETE` on 2026-06-11 (its own HEAD: "ARCHIVED — engine re-derived in
> Rust… repo frozen"). The implementer bridged it with a temporary directory junction (reverted) to
> capture the witness. Consequences threaded into the tasks below: (i) the arc's premise is STRONGER —
> the 5 live-read tests have silently skipped on this machine since 2026-06-11 and NEVER ran in CI
> (GHA checks out only al-call-hierarchy + tree-sitter-al), so they have validated nothing anywhere
> for weeks; (ii) Task 3.3 vendors from the WITNESS area (`.superpowers/sdd/alsem-witness/`, 53 files),
> not from live al-sem — no junction needed downstream; (iii) Task 3.6's "temporarily rename al-sem"
> is already the machine's real state — final verification runs directly. Two more findings: the
> cli-c-cache al-sem fallback corpus is stale dead code (retire, don't vendor — Task 3.3/3.5), and two
> extra live-read fixtures (`ws-diff-rename`, `ws-diff-removed-field`) the brief missed (vendor in 3.3).

### Task 3.0 — ✅ DONE 2026-07-05 — Preflight witness (no commits; witness untracked by design)

1. Record the al-sem checkout's HEAD commit (`git -C U:/Git/al-sem rev-parse HEAD`) in the witness manifest.
2. Run the 5 live-read test files with the checkout present and PROVE none took its skip path (the gates are silent `return`-style skips, invisible to cargo — instrument by asserting on their skip-log lines or temporarily panicking the skip arms). All 5 must run their real assertions and pass.
3. Copy the live-read al-sem-side inputs to a witness area `.superpowers/sdd/alsem-witness/` (gitignored): `scripts/cli-b-goldens/diff/**`, `scripts/r2c-goldens/ws-d2.l3eg.golden.json`, `scripts/r1a-goldens/ws-d2.l2.golden.json`, the cli-c cache fallback set, plus a file listing of the fixture trees to be vendored (`test/fixtures/ws-d2`, the cli-c policy fixture workspaces) with per-file SHA-256.
4. Verify `REGEN_TEMP_GOLDENS=1` regen paths exist and work for the affected harnesses BEFORE anything is deleted (dry-run into a temp dir). Two cases: for goldens that already exist in-repo, the dry-run must reproduce the current repo bytes; for the live-only goldens (cli-b diff, the two aldump/al2dump goldens, the cli-c cache fallback set), it must reproduce the Task-3.0 witness copies byte-identically.

**Gate:** witness manifest complete; `git check-ignore .superpowers/sdd/alsem-witness/` confirms the witness area is ignored (add to `.gitignore` if not); full suite green; CDO byte-identical (nothing changed yet).

### Task 3.1 — Fix the `policySource` absolute-path defect + rebaseline

`tests/cli-c-policy-goldens/ws-policy-custom.custom.{human.txt,json}` embed `auto:U:\Git\al-sem\test\fixtures\…\al-sem.policy.yaml`. Root cause: the CLI emits the auto-detected policy path as an absolute machine path — a reproducibility defect in product output. **Contract (exact, not "or similar"):** `policySource` becomes the policy file's path relative to the analyzed workspace root, forward slashes on all platforms, no drive letters, no `.`/`..` segments; a policy file outside the workspace root falls back to its bare filename (and this case is unit-tested). If the field also surfaces in SARIF, it is emitted as a SARIF-relative artifact reference, not a bare string with an ambiguous base. Rebaseline the cli-c policy goldens; CHANGELOG under Fixed. Runs BEFORE the vendoring regen so the moved fixtures regenerate path-clean on the first pass.

### Task 3.2 — ✅ DONE 2026-07-05 (7 commits, review PASS, zero golden churn) — Retire the src parity shims (identity decisions BEFORE any broad regen)

> **Pre-dispatch scouting finding (2026-07-05 — the version string does THREE jobs via one function):**
> `alsem_version()` is read by (A) the JSON/events display envelope field (`alsemVersion`/`al_sem_version`
> — the KEY stays, out of scope; only the VALUE is at issue), (B) SARIF `driver.version`, AND (C) the
> cache-header `analyzer` stamp at `cache_prune.rs:187`, which participates in cache-validity
> fingerprinting. Confirmed golden reality: the display goldens (A/B, ~187 files) are ALREADY pinned to
> test sentinels by the harnesses (`cli-a-json-v1`, `HTML_VERSION_OVERRIDE`, …) — they never contained
> `0.0.12`. The ONLY goldens embedding `0.0.12` are the 18 cache-header fixtures (role C). So the honest
> fix is a DECOUPLING, and it is low-churn.

- **Decouple display from cache fingerprint (roles A/B vs C):**
  - `src/engine/gate/cache_prune.rs`: introduce a dedicated `CACHE_ANALYZER_VERSION: &str = "0.0.12"` const (a cache-format version we own, bumped only when a behavior change should invalidate caches) and use it at :187 instead of `alsem_version()`. **Keep the value `"0.0.12"` verbatim → ZERO cache-golden churn.** Comment: versions OUR cache format, decoupled from the display version.
  - `src/engine/gate/version.rs`: rename `alsem_version()` → `driver_version()`, default `env!("CARGO_PKG_VERSION")` (honest identity, currently `0.9.3`); rename the override env `AL_SEM_VERSION_OVERRIDE` → `ALCH_DRIVER_VERSION_OVERRIDE` (engine-owned; documented cosmetic/golden-stability-only). Update the 3 call sites (`run.rs:390,498`, SARIF path) + the ~4 cli-a harnesses that set the env (`cli_a_json/html/terminal/stats_differential.rs`) to the new env name; their sentinel VALUES stay, so those 187 display goldens do NOT churn.
  - **Blast-radius guard:** BEFORE editing, grep every golden for the display field's literal value to confirm NO non-overridden gate test embeds the old `0.0.12` default as its display version (scouting found none — `"alsemVersion":"0.0.12"` empty — re-verify; if one exists, pin its harness rather than let CARGO_PKG_VERSION leak into a golden).
- **VERSION001 — the STOP-and-record outcome (scouting resolved this):** the Rust engine currently NEVER emits VERSION001. `project_diagnostics` (format_json.rs:245) just maps diagnostics — there is NO runtime suppression conditional keyed on the override; the al-sem `versionDiagnostic()`/`cachedDiagnostic` side-channel was never ported. There is nothing to "un-suppress." Behavior-preserving action: keep NOT emitting it, and rewrite the format_json.rs:238–248 doc comment to drop the "replicate al-sem's override suppression" framing, stating plainly that this engine does not implement the al-sem VERSION001 cache-version diagnostic. No behavior change, no golden change.
- `src/engine/gate/cache_prune.rs:61–72` (cache env): rename `AL_SEM_RELEASE`/`AL_SEM_DEV_FINGERPRINT` to `ALCH_RELEASE`/`ALCH_DEV_FINGERPRINT`. Grep `.github/workflows/`, scripts, and docs for the old names and update in the same commit (a stale CI env name = permanent cache misses). Old env names removed outright, not aliased — one-time cache re-fingerprint, not a correctness hazard; state in CHANGELOG. The `cache-versions.ts`-originated consts stay (they version OUR cache format now); rewrite their comments to drop the "MUST match al-sem" framing (there is no `.ts` file in this repo — comment references only).
- `src/bin/alsem.rs` `--dump-model` stub: delete the hidden flag and its CONFIG_ERROR stub ("use the TS CLI" is a dead instruction); update any CLI-contract test asserting the rejection.

**Commit discipline:** one commit per concern (cache-analyzer decouple / driver_version rename / VERSION001 comment / cache-env rename / dump-model removal) so each is independently reviewable, not one tangled mega-commit.

**Gate:** full suite green; clippy; CDO byte-identical (display/cache-env plumbing, disjoint from the call-graph harness — the aldump stats JSON SHA-256 `67910e99…913f4f` must NOT move; if it does, stop). Cache goldens byte-identical (decouple keeps `0.0.12`); display goldens byte-identical (sentinels unchanged).

### Task 3.3 — ✅ DONE 2026-07-05 (5 commits, review PASS, suite green with no al-sem on disk) — Vendor the live-read corpora in-repo (witness-diffed)

**Source of vendored bytes (corrected 2026-07-05 after inspecting the witness):** Task 3.0 captured fixture TREES only as SHA-256 *listings* (`.superpowers/sdd/alsem-witness/fixture-listings/`), not as byte copies — the fixture bytes live in the archived checkout `U:\Git\al-sem-OBOLETE\test\fixtures\<name>` (all 13 present: `ws-d2`, `ws-diff-rename`, `ws-diff-removed-field`, and the 10 `ws-policy-*` workspaces). GOLDEN bytes ARE in the witness (`.superpowers/sdd/alsem-witness/scripts/{cli-b-goldens,cli-c-goldens,r1a-goldens,r2c-goldens}`). So: copy fixture TREES from `al-sem-OBOLETE`, verifying each file's SHA-256 against the witness listing; regenerate GOLDENS from THIS engine and witness-diff them against the witness `scripts/` copies. `al-sem-OBOLETE` is a frozen archive — read-only, never write to it.

**Files:**
- Create: `tests/fixtures/ws-d2/` + `PROVENANCE.md` (source path, al-sem HEAD `cfea6149…` from Task 3.0, copy date, "historical fixture, not a live oracle")
- Create: `tests/fixtures/cli-c-policy/` (the 10 fixture workspaces `cli_c_policy_differential.rs` enumerates) + `PROVENANCE.md`
- Create: `tests/fixtures/ws-diff-rename/` and `tests/fixtures/ws-diff-removed-field/` (the two extra live-read fixtures Task 3.0 found — `cli_b_diff_differential.rs` reads them directly) + `PROVENANCE.md`
- Create: `tests/cli-b-goldens/diff/` and in-repo homes for the two aldump/al2dump goldens (`ws-d2.l3eg`, `ws-d2.l2`) — regenerated from THIS engine's output via the Task-3.0-verified regen paths
- Modify: `tests/aldump_smoke.rs`, `tests/al2dump_smoke.rs`, `tests/cli_b_diff_differential.rs`, `tests/cli_c_policy_differential.rs`, `tests/cli_c_cache_differential.rs` — point at the in-repo paths, DELETE the skip-gates (tests now hard-require their inputs)

**cli-c-cache fallback — RETIRE, do not vendor (Task 3.0 finding 2):** the al-sem-side cache golden set (`$AL_SEM_DIR/scripts/cli-c-goldens/cache/`) is STALE — a symbolReader 17→18 version bump post-dates it, so the current engine cannot regenerate it, and the in-repo local override (`tests/cli-c-goldens/cache/`, already current + regen-verified in Task 3.0) always wins the local-else-fallback in `cache_goldens_dir()`. Vendoring it would commit content the engine can't reproduce — a Rust-owned-doctrine violation. Instead: delete the `al_sem_cache_goldens_dir()` / `al_sem_dir()` fallback branch in `cli_c_cache_differential.rs`, leaving the local override as the sole path. FIRST confirm the test's INPUT fixture-cache (the `fixture_cache_dir()` the prune runs against) is in-repo; if it too reads from al-sem, vendor it from the witness. This removes the last `AL_SEM_DIR` read from this file (dovetails with Task 3.5).

**Byte-preservation (round-2 finding):** this repo's `.gitattributes` forces `eol=lf` on several `tests/**` patterns and the checkout runs `core.autocrlf=true` — committing the vendored fixtures without an explicit rule can rewrite their line endings, shifting every tree-sitter byte offset and silently invalidating the goldens generated from them. Add `tests/fixtures/** -text` to `.gitattributes` (ordered after the existing `tests/**` rules — last match wins), commit, then verify each committed fixture file's SHA-256 matches the Task-3.0 witness listing via `git show HEAD:<path> | sha256sum`.

**Anti-laundering protocol (mandatory, the round-1 headline finding):**
1. **Fixture-orphaning check first:** before regenerating anything, verify each vendored fixture tree is self-contained — if a workspace references sibling context (`.alpackages`, dependency `.app`s, adjacent config) that lived outside the copied tree in al-sem, vendor that too. Detection: run the harness against the vendored copy and compare its output against the same harness run against the source tree in `U:\Git\al-sem-OBOLETE\test\fixtures\<name>` (reachable directly, or recreate the junction `New-Item -ItemType Junction -Path U:\Git\al-sem -Target U:\Git\al-sem-OBOLETE`; remove after) — they must be byte-identical modulo the Task-3.1 path normalization.
2. **Witness diff:** every regenerated golden is diffed against its Task-3.0 witness copy. Permitted differences: the Task-3.1 `policySource` normalization and the Task-3.2 version fields — each diff hunk classified and listed in the task report. ANY other difference is a latent divergence that the old skip-gated setup masked: stop, investigate, resolve on the merits (engine bug → fix; intentional Rust-ahead behavior → document the adjudication) before proceeding.
3. **Graph-shape assert:** for graph-bearing outputs (the ws-d2 l3eg golden), the unknown/resolved edge counts in the regenerated golden must equal the witness's — the specific silent-degradation vector (orphaned fixture → resolution downgrades → regen blesses the loss) is checked by number, not by eyeball.

**Gate:** full suite green with `AL_SEM_DIR` pointing at a nonexistent path AND with the harnesses' hardcoded-path constants gone (grep-verified); CDO byte-identical.

### Task 3.4 — ✅ DONE 2026-07-05 (2 commits, review PASS/PASS, token gone repo-wide) — Remove KNOWN_DIVERGENCES machinery

Delete `KNOWN_DIVERGENCES.json` (currently `[]` — empty). In each of the 13 harnesses (`differential.rs`, `r4_differential.rs`, `r2_5a_differential.rs`, `r2_5b_{cg,cov,eg,rt}_differential.rs`, `r3a{1,2,2_trace,3,4,5}_differential.rs`) the allowlist machinery is named `AllowEntry` (struct) + `load_allowlist()` (loader) — delete both plus their two-part gate: (a) fail on any non-allowlisted divergence, (b) fail on any UNUSED allowlist entry. With the allowlist empty, collapsing (a) to a direct byte-assert of the diff being empty is behavior-preserving (no divergence is tolerated today), and (b) vanishes with the allowlist. The r3a4/r3a5 "allowlist must be empty" exit gates become vacuous — delete them too. Remove the `KNOWN_DIVERGENCES.json` paragraph from CLAUDE.md. All 13 suites must stay green (strict assert already holds since the allowlist is empty).

### Task 3.5 — ✅ DONE 2026-07-05 (2 commits, review PASS/PASS, AL_SEM_DIR gone from suite) — Remove the 14 refresh functions + `r2_5b_refresh.rs`

> **Pre-dispatch scouting (2026-07-05).** DELETION CRITERION: delete every `#[ignore]`d refresh fn that reads `AL_SEM_DIR` or shells `bun run` — 14 total: `differential.rs`, `r2_5a_differential.rs`, `r3a4_differential.rs`, `r3a5_differential.rs`, `r4_differential.rs`, `cli_a_{html,json,terminal}_differential.rs`, `cli_c_events_differential.rs`, and the cli_b refresh set — plus delete `tests/r2_5b_refresh.rs` (whole file, `refresh_r2_5b_goldens_from_al_sem`). KEEP `cli_a_stats_differential.rs:345` `refresh()` — it regens purely from Rust output (no `AL_SEM_DIR`, no bun); it is the MODEL.
>
> **"Gains a regen path" (6 harnesses).** These lack `REGEN_TEMP_GOLDENS` and would lose their only regen when the al-sem refresh is deleted: `r2_5a_differential.rs`, `r2_5b_{cg,cov,eg}_differential.rs`, `r3a4_differential.rs`, `cli_c_events_differential.rs`. Give each a Rust-regen path mirroring the in-repo pattern (`differential.rs` has 12 such sites; `r2_5b_rt_differential.rs` has 3): at the existing actual-vs-golden comparison, `if env REGEN_TEMP_GOLDENS is set { write the ACTUAL Rust output to the golden path; skip the assert } else { assert as today }`. These paths are INERT under normal `cargo test` (env-gated) so they cannot affect the suite. The other 7 affected harnesses already have `REGEN_TEMP_GOLDENS` — leave those regen paths, delete only their al-sem refresh fn.

The al-sem/Bun refresh functions (`AL_SEM_DIR`-driven) are deleted wholesale; `tests/r2_5b_refresh.rs` deleted entirely. Also update `tests/r0-goldens/README.md` (drop the `AL_SEM_DIR` refresh instructions). After this task: `grep -rn "AL_SEM_DIR" tests/ src/` is empty and `grep -rn "bun " tests/` shows no live `bun run` invocation (retired-note comments referencing the archived TS tool are acceptable only if they carry no runnable instruction — prefer removing them).

### Task 3.6 — Docs + capstone

- Move `docs/engine-migration.md` + `docs/engine-gaps.md` to `docs/history/` with an ARCHIVED header.
- CLAUDE.md: rewrite the "Testing Philosophy & Goldens" al-sem bullets (the "LEGACY tests still pointing at the al-sem repo" sentence becomes false — delete it) AND fix the stale grammar-migrations line "the goldens are the al-sem TS reference output … source of truth" (CLAUDE.md:128–130), which contradicts the Rust-owned doctrine.
- **Zero-skip mechanical verification:** `U:\Git\al-sem` is ALREADY absent on this machine (Task 3.0 finding 1), so the final branch state runs in the target environment directly — run the full suite as-is and confirm the five former live-read test files run their real assertions (no skips) and pass. Cross-check by grepping those five files for any surviving skip-string/early-return pattern (there must be none — the gates were deleted in 3.3). Optional stronger proof: temporarily create the junction (`New-Item -ItemType Junction -Path U:\Git\al-sem -Target U:\Git\al-sem-OBOLETE`), rerun, confirm identical pass/run counts, remove the junction — proves the in-repo paths win even when a checkout reappears.
- **Legacy-token audit is a TRIAGE, not a purge** (revised 2026-07-05 after a dry run — "grep returns nothing" was wrong; it flags legitimate provenance, changelog, and technical-oracle references that MUST stay). Run:
  ```bash
  rg -n --hidden --glob '!target/**' --glob '!docs/history/**' \
     --glob '!docs/superpowers/plans/2026-07-05-four-outstanding-items.md' \
     --glob '!CHANGELOG.md' --glob '!**/PROVENANCE.md' \
     'AL_SEM_DIR|AL_SEM_VERSION_OVERRIDE|AL_SEM_RELEASE|AL_SEM_DEV_FINGERPRINT|DEFAULT_ALSEM_VERSION|KNOWN_DIVERGENCES|dump[-_]model' .
  ```
  These HARD tokens (live env vars / deleted consts / deleted mechanisms) must return NOTHING — they denote a live dependency or a deleted thing (already true after 3.2/3.4/3.5; re-confirm). CHANGELOG.md and PROVENANCE.md are exempt (they legitimately document the removal / record fixture provenance).
  Then TRIAGE these softer patterns by hand — clean the STALE/DEAD/MISLEADING, KEEP the legitimate:
  - `rg -n 'use the TS CLI' src/` → **investigate + clean.** `src/bin/alsem.rs:700` `ORDER_REJECTION` ("digest --order … use the TS CLI") points users at the retired tool. Probe whether `digest --order` is a live-but-unsupported flag or dead like `--dump-model`; if dead, remove the flag+rejection; if the rejection must stay, reword to drop the TS-CLI pointer (state it's unsupported, full stop).
  - `rg -n 'U:[/\\]Git[/\\]al-sem\b' src/ tests/` (word-boundary — do NOT match `al-sem-OBOLETE`) → these are doc-comments citing where a golden was ORIGINALLY sourced. KEEP as provenance but reword any that imply a LIVE read ("goldens live at U:\Git\al-sem\…") to past-tense provenance ("originally derived from al-sem's …; now Rust-owned in tests/…"). `src/engine/root_classification.rs:3` "byte-parity oracle: U:\Git\al-sem" → reword to past-tense (the oracle is retired).
  - `rg -n '\b[Bb]un\b' tests/ src/` → **KEEP** the ICU/DUCET collation-oracle references (`src/engine/ids.rs`, `src/engine/gate/cbor.rs`, `src/engine/l5/ordering_facts.rs` — "matches Bun's localeCompare" documents the collation algorithm, NOT al-sem parity) and the "no Bun required" statements (they describe the good offline state). CLEAN only the two stale retired-note comments the arc left: `tests/cli_c_cache_differential.rs:548` and any "this used to shell bun run" note carrying no purpose now.
  - `rg -n 'AL_SEM_VERSION_OVERRIDE' tests/` → `tests/gate-goldens/manifest.json` (`alsemVersionPin`) + `tests/gate_sarif_differential.rs:9` describe the OLD al-sem SARIF-capture method; the gate SARIF path uses the `--sarif-version-override` CLI flag today. Reword both to drop the dead env-var name (state the current flag-based method).
  The success criterion is: **no live al-sem runtime dependency, no deleted-token reference, no user-facing pointer to the retired TS tool, no doctrine claiming al-sem is the source of truth** — legitimate historical/provenance/algorithm-oracle notes survive, correctly framed as past-tense.
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
