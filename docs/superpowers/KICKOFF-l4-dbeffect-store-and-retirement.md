# KICKOFF — L4 db-effect Store Redesign + Old-Solver Retirement (Phase 1.5)

**Paste this whole file as your first message in a new session. It is self-contained. Execute it
fully autonomously to completion — do not stop between tasks or phases.**

---

## Mission

Execute the implementation plan `docs/superpowers/plans/2026-07-22-l4-dbeffect-store-and-retirement.md`
end to end, on branch **`feat/l4-summary-redesign`** (already checked out; HEAD has all prior work).
Deliver: the L4 db-effect solver made fast (measured 517s → seconds) and compact (~40GB → <300MB) via
a shared interned columnar `EffectStore` + a bidirectional query index, then the old Jacobi solver
retired so ONE path remains. Then a final whole-branch review. Do NOT merge to master — stop at the
merge decision and hand it to the user.

## Read these FIRST (absolute paths), in order

1. `docs/superpowers/plans/2026-07-22-l4-dbeffect-store-and-retirement.md` — the plan (Tasks A1–A5, B1). Your task list.
2. `docs/superpowers/specs/2026-07-22-l4-dbeffect-store-and-retirement-design.md` — the design (rev 4, review-converged). **This is the requirements source**; its ⟨rev⟩ notes are binding.
3. `.superpowers/sdd/progress.md` — the durable ledger. The `l4-summary-fixpoint-redesign` section (Tasks 1–11b) is **DONE** — do NOT redo it. The `l4-dbeffect-store` work is NEW; append a fresh ledger section for it and record one line per completed task.
4. `CLAUDE.md` (repo + `~/.claude/CLAUDE.md`) — project + user rules.

## Current state (do not re-verify by redoing work)

- Branch `feat/l4-summary-redesign` contains the whole Phase-1 arc (the closed-form v2 db-effect
  solver, byte-identical to the old Jacobi, cutover done, goldens clean) + the phase-split instrument.
- Phase-1 measured on the 8020 corpus: `compute_summaries` 924s → 541s (the 729s db_effects Jacobi is
  gone); residual attributed (instrumented, decisive) to the **db-effect solver = 517s / ~40GB**
  materialization constant-factors (roles fixpoint is 0.8s — a non-issue). This arc fixes that.
- The plan **supersedes** the original plan's Task 12/13 (Salsa is MOOT once R3b is deleted in B1).
- Nothing is mid-flight; no running processes.

## Workflow — Subagent-Driven Development (SDD), fully autonomous

Invoke the **superpowers:subagent-driven-development** skill and follow it. Continuous execution: run
ALL tasks A1→A2→A3→A4→A5→B1 without pausing for check-ins. The ONLY reasons to stop: a genuine
BLOCKED state you cannot resolve (after consulting pi — see below), or all tasks complete.

The SDD skill provides its own helper scripts in ITS directory (not the repo's `scripts/`):
`task-brief` and `review-package` live under the subagent-driven-development skill's `scripts/`
(the skill instructions give the path). `scripts/check-goldens` and `scripts/cdo-gate` ARE repo scripts.

Per task:
1. `<skill>/scripts/task-brief docs/.../2026-07-22-l4-dbeffect-store-and-retirement.md <N>` → brief file path.
2. Dispatch a fresh **implementer** subagent (fitting model — see table) with: one line of scene, the
   brief path (its requirements), the relevant spec section, the interfaces from earlier tasks, and
   the report-file path. Hand artifacts as files, not pasted text.
3. Implementer follows **TDD** (failing test first), **SOLID + DRY**, commits, self-reviews.
4. `<skill>/scripts/review-package <BASE> <HEAD>` (BASE = the commit before you dispatched — from the ledger,
   never `HEAD~1`) → dispatch a **task reviewer** (fitting model) with brief + report + package +
   the binding constraints. Two verdicts required: spec-compliance AND code-quality.
5. Fix loop for Critical/Important findings (dispatch fix subagent; re-review). Record Minors in the
   ledger for the final review.
6. Run the task's gate (below) + re-measure where the plan says. Append one ledger line:
   `Task N: complete (commits <base7>..<head7>, review clean) — <one-line what + measured number>`.

After all tasks: dispatch the **final whole-branch review** (most capable model) with a
`review-package <merge-base> <HEAD>`; fix wave via ONE fix subagent if needed; then invoke
**superpowers:finishing-a-development-branch** — present the options and STOP (do not auto-merge; the
user decides merge, per CLAUDE.md "never merge to master without explicit request").

## Engineering principles (instruct every implementer AND reviewer)

- **TDD:** failing test first (against the live differential where applicable), then minimal code to green.
- **SOLID:** single-responsibility modules (`effect_store.rs`, `reverse_index.rs`, universe typestate
  are separate concerns); depend on interfaces (`SummaryBundle`/`DbEffectRef` views, not concrete Vecs);
  the `GrowingEffectUniverse`→`FrozenEffectUniverse` typestate makes the post-freeze contract structural.
- **DRY:** one substitution impl (share `substitute_pd_temp_state`), one via-rank enum, one `key_rank`,
  one interner per id-kind. Reviewers must flag copy-paste and multi-path duplication.

## THE GOLDEN GATE (every Part-A task, verbatim)

The change is representation-only ⇒ **exact output preservation**. The old solver is retained ONLY as
the differential oracle through all of Part A.

```bash
cargo test -p al-call-hierarchy --test l4_summary_differential      # v2 (new store) == old, per routine — MUST be green
cargo test -p al-call-hierarchy --lib db_effect_solver              # unit tests green
bash scripts/check-goldens 2>&1 | tee /tmp/gate.log; grep -iE "fail|mismatch|differ" /tmp/gate.log || echo GOLDENS_CLEAN   # NO regen; do NOT pipe to tail
cargo clippy -p al-call-hierarchy --all-targets                     # clean
rustfmt <each touched file>                                         # never `cargo fmt`
```
A golden that MOVES is v2 diverging from old — root-cause it; NEVER `--regen` to force green. If a
divergence is real, that is a BLOCKED escalation, not a rebaseline.

## Model selection (specify explicitly on every dispatch)

| Task | Nature | Implementer | Reviewer |
|------|--------|-------------|----------|
| A1 intern + cache + sort | mechanical-integration | sonnet | sonnet |
| A2 compact rows + u8 via + lazy view | integration, API surface | sonnet | sonnet |
| **A3 frozen universe + shared SetId + delta + feed-forward** | **the crux, subtle** | **opus** | **opus** |
| A4 ReverseEffectIndex | integration | sonnet | sonnet |
| A5 CDO parity + re-measure + perf gate | measurement + small edit | controller runs measures; sonnet for perf_bounds edit | sonnet |
| **B1 retirement (delete R3b + old Jacobi + baseline + flip)** | **multi-step, blast radius** | **opus** | **opus** |
| Final whole-branch review | architecture | — | **opus** |

Cheapest tier that fits; escalate on BLOCKED. Turn-count beats token price — don't starve a subtle task.

## External review + questions — pi_ask (gpt-5.6-sol + claude-fable-5)

Load pi tools once: `ToolSearch "select:mcp__pi__pi_ask,mcp__pi__pi_models,mcp__pi__pi_cleanup"`.
Use `mcp__pi__pi_ask` for: (a) an adversarial design/impl review of the CRUX tasks **A3 and B1**
before marking them complete; (b) any genuine design question or BLOCKED state during any task.

- Models: `gpt-5.6-sol` and `claude-fable-5`. `thinking: high` (default; lower only for trivia).
- Give **absolute paths** (pi runs from its own workspace). Pass `output_file` for long answers.
- Start FRESH threads (this-session continuation ids won't carry). Point them at the spec + the exact
  source files. `require_evidence` is on by default — an answer reading 0 files is prior-derived; **do
  not trust pi's code claims without source-verifying them yourself** (pi has hallucinated line refs;
  it also silently drops very long prompt bodies — keep prompts tight and file-pointer-based).
- When sol and fable disagree, reconcile against the actual source; when they converge, that's strong
  signal. Fold confirmed findings; record the review file paths in the ledger.

## Measurement discipline (the 8020 re-measures)

The 8020 corpus (~22 min full, ~11 min trimmed) is the perf proof. Gotchas learned the hard way:
- **Build `release-fast` fresh** before each measure (`cargo build --profile release-fast --bin alsem`),
  reflecting the just-committed code. `--release` is only for the perf_bounds gate / north-star.
- **Detached run that survives harness reaping:** launch via PowerShell
  `Start-Process pwsh -ArgumentList '-NoProfile','-File','<probe.ps1>' -WindowStyle Hidden`, write a
  sentinel file (`EXITCODE=... WALL_SECS=...`), and poll the sentinel with a `run_in_background` bash
  loop. To KILL a run you must stop BOTH the launcher pwsh AND its `alsem` child.
- **Trace:** `ALSEM_TRACE=1 ALSEM_TRACE_DETAIL=hot` (NOT `stages,hot` — that gates off Hot counters).
  Read `context.compute_summaries` + the `compute_summaries_v2_phase_split` instant (`db_solver_ms`
  vs `roles_ms`) from the trace JSON. Span names are BARE.
- **Trimmed split-probe** (skip d1, ~11 min): add `ALSEM_TRACE_EXIT_AFTER=context.compute_summaries`
  (flushes trace + clean-exits after that span). Use this for per-step attribution; use the full probe
  for A5's final number.
- Probe scripts already exist in `logs/` (`run-probe-v2.ps1`, `run-probe-v2-split.ps1`). The 8020
  corpus is at `C:/Users/SShadowS/AppData/Local/Temp/claude/U--Git-al-call-hierarchy/66efc3ec-d07b-48e2-8181-95ce2f62dd04/scratchpad/corpus-8020`
  (8020 `.al` files + `app.json`). **Verify it still exists first**; if gone, ask the user for the corpus
  path (do not fabricate a run). If the probe scripts are gone (logs/ is gitignored), recreate them
  (they just set the trace env + run `alsem analyze <corpus> --detector d1-db-op-in-loop --format json`).
- Per-step targets: A1 → most of 517s gone; A2 → compute down (full RSS win NOT yet — lands A3); **A3
  → db-solver seconds + peak RSS <1GB (target <300MB)**; A5 → full-probe confirm.
- Do not run concurrent `cargo build`s during a measure (contention skews it).

## BC containers (available; use where they fit — do not force-fit)

Three Business Central containers are available: **Cronus28, Cronus281, Cronus282** — all
user/pass **`sshadows` / `1234`**. Access via the MCP tools (load with ToolSearch as needed):
`mcp__bc-dev__*` (bcdev_source, bcdev_test_run/discover, bcdev_debug_*, bcdev_status) and `mcp__bc__*`.
Fitting uses for THIS arc (optional, where they add value): (1) pull real AL app source as an
additional real-workspace differential/validation corpus beyond the synthetic 8020 (strengthens the
A5 parity leg if `CDO_WS` is unavailable); (2) sanity-check the A4 hover semantics (down/up DB-touch
queries) against a live BC's actual behavior; (3) parallelize independent validation across the three
containers. This is a Rust-engine arc — the containers are a validation aid, not a required dependency;
skip them for pure-Rust tasks.

## CDO / real-workspace gate (before Part B)

The `CDO_WS`-gated whole-program v2-vs-old parity test + the north-star ratchets run only when `CDO_WS`
points at a real BC workspace. Run `CDO_WS=<path> scripts/cdo-gate` (or the gated differential) — it is
**opt-in per Part-A step but MANDATORY before B1** (a step gated on fixtures only is not proven). If
`CDO_WS` is unset on this machine, note it in the ledger and use a BC-container real-source corpus (see
above) as the real-workspace leg; if neither is available, mark the CDO leg SKIPPED-with-reason and
proceed (do not fabricate a result).

## Guardrails (non-negotiable)

- Never `git add -A` — stage only intended paths. Never push. Never merge to master without explicit
  user request (the finishing step presents options and STOPS).
- `rustfmt <file>` per file, never `cargo fmt`. Package is `al-call-hierarchy` (hyphen) for `cargo test -p`.
- Never blind-regen a golden. Never weaken the differential assertion. Root-cause first (CLAUDE.md).
- No destructive git ops (reset --hard / checkout . / force push) without user confirmation.
- Git Bash for POSIX; PowerShell for the detached runs. Don't use `2>nul`.
- Update `CHANGELOG.md` at the B1 capstone (Added: EffectStore + hover index; Changed:
  compute_summaries perf; Removed: old Jacobi + R3b experiment) and `docs/OUTSTANDING.md`.

## Durable progress / resume-safety

Track every completed task in `.superpowers/sdd/progress.md` (one line: task, commit range, review
status, measured number). After any compaction or restart, trust the ledger + `git log` over memory;
resume at the first task not marked complete. Never re-dispatch a task the ledger marks done.

## Definition of done

A1–A5 + B1 complete and reviewed; live differential green throughout Part A, flipped to the frozen
complete-internal baseline in B1; goldens byte-identical (except inspected trace goldens); DO
byte-identical; 8020 `compute_summaries` in seconds with peak RSS <1GB (target <300MB), re-measured;
old Jacobi + R3b deleted (one path); `cargo build` warning-free; CHANGELOG + OUTSTANDING updated; final
whole-branch review clean. Then present the finish options and stop for the user's merge decision.
