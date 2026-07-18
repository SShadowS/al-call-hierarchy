# Permanent perf_trace Infrastructure Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the permanent env-gated perf-tracing layer specified in `docs/superpowers/specs/2026-07-18-tracing-infra.md` (THE SPEC — every task implements a section of it verbatim; read it first, it is the requirements document), then run the decisive traced d1-only 8020 experiment it unlocks.

**Architecture:** One new module `src/engine/perf_trace.rs` + ~20 instrumentation points across the analyze pipeline, in three detail tiers. Chrome Trace Event JSON output with crash-safe checkpointing; optional stderr mirror. Zero-cost disabled (acceptance-tested).

**Tech Stack:** Rust, no new dependencies (serde_json already present).

## Global Constraints

- THE SPEC governs: API, env contract, output format, instrumentation-point list, hot-loop discipline (`enabled(Hot)` once outside loops, `Option<&mut LocalCounters>` threaded), disabled-path contract.
- Byte-stable: goldens clean; full suite green; clippy clean; DO analyze output identical with tracing OFF **and ON** (ON only adds the side file).
- rustfmt per file, never `cargo fmt`. Explicit staging. No stash. No `| tail`. Foreground subagent commands only.
- CHANGELOG entry at the capstone (this is a real shipped feature, unlike the swept probes).
- Dispatch: T1 Opus, T2 Sonnet, T3 Opus, T4 Sonnet, T5 orchestrator+Sonnet, T6 orchestrator. Reviewers: Opus T1/T3, Sonnet T2/T4.

---

### Task 1: The perf_trace module

**Files:** Create `src/engine/perf_trace.rs`; register in `src/engine/mod.rs`.

- [ ] TDD: unit tests FIRST for (a) env parsing (off default; `1|chrome`; detail tiers cumulative; sample/scc-min defaults 64/100), (b) Chrome JSON validity (write spans/counters/instants to a temp file via the real writer, parse back with serde_json, assert event shapes incl. immediate-`B`-write), (c) killed-run checkpoint (drop the writer mid-span, file still parses, open span visible), (d) disabled path emits nothing and allocates nothing observable (no file created).
- [ ] Implement per THE SPEC's API sketch: Config OnceLock, Detail tiers, RunGuard/SpanGuard (Drop emits `E` + RSS args), counter/counter_delta, instant_lazy, LocalCounters, maybe_exit_after, the K32 RSS block (PROCESS_MEMORY_COUNTERS_EX), the checkpointing writer (BufWriter + flush on coarse-span close + timed interval), stderr mirror behind ALSEM_TRACE_STDERR.
- [ ] Gates: module tests + full suite + goldens + clippy; rustfmt; commit `feat(engine): perf_trace — permanent env-gated tracing (spec 2026-07-18)`.

### Task 2: Stages-tier instrumentation (~20 points)

**Files:** `src/engine/gate/run.rs`, `src/engine/l3/l3_workspace.rs`, `src/engine/l5/detector_context.rs`, `src/engine/l5/registry.rs`.

- [ ] Add exactly THE SPEC's named spans (analyze.total … detector.<name>) + the l3 parse/projection LocalCounters + the STRUCTS SCC instant_lazy in detector_context (largest-SCC anatomy, from the historical probe shape — recover via `git show c8836e7^:src/engine/l5/detector_context.rs` for reference, reimplement through instant_lazy).
- [ ] Gates: full suite + goldens + clippy + a manual smoke: `ALSEM_TRACE=1 ALSEM_TRACE_STDERR=1` DO run → stderr stage lines + parseable JSON; then DO diff OFF-vs-ON on the analysis output (must be identical). rustfmt; commit.

### Task 3: Jacobi + Hot tiers

**Files:** `src/engine/l4/summary_runner.rs`, `src/engine/l5/detectors/d1.rs`, `src/engine/l5/path_walker.rs`.

- [ ] Jacobi tier per THE SPEC: per-recursive-SCC span ≥ SCC_MIN, per-pass domain-split cardinality counters, final unique-universe/overlap dump for the largest SCC. No per-singleton events.
- [ ] Hot tier per THE SPEC: d1 pre-walk census (candidates, distinct callee roots — emitted BEFORE walking starts); WalkTraceStats LocalCounters threaded `Option<&mut>` through walk_evidence (nodes, edges examined, results by stop kind, retained totals); memo hit/miss + retained-size counters + RSS checkpoint every N misses in d1. The decisive ratio (unused_cut_results/all_retained) must be computable from the emitted counters.
- [ ] walk_evidence signature grows an optional stats param — update ALL callers (d1/d2/d46/d48 pass None unless Hot; grep `walk_evidence(`).
- [ ] Gates: full suite + goldens + clippy + memoized≡fresh d1 test still green; DO OFF-vs-ON diff. rustfmt; commit.

### Task 4: Acceptance gates

- [ ] Disabled-path Criterion bench (or a timed-loop test if wiring Criterion is disproportionate — justify in report): pipeline-level <1% guard on DO, traced-off vs pre-branch baseline binary.
- [ ] Windows RSS smoke test (values nonzero, peak ≥ current).
- [ ] Commit test additions.

### Task 5: The decisive experiment (orchestrator runs the long piece)

- [ ] Quiet-machine check (LoadPercentage < ~15 sustained; else report and wait/flag).
- [ ] Traced d1-only 8020: `ALSEM_TRACE=1 ALSEM_TRACE_DETAIL=hot ALSEM_TRACE_FILE=<scratch>/d1only-8020.json` + `--detector d1-db-op-in-loop`, detached with the monitor script, 2h cap.
- [ ] Extract: census, stop-kind histogram, the decisive ratio, memo retention totals, RSS timeline, Jacobi block numbers. Write `.superpowers/sdd/d1only-verdict.md` with the branch decision per THE SPEC's final section (CompleteOnly mode vs complete-path-multiplicity vs Jacobi wedge).

### Task 6: Capstone

- [ ] Measurements doc §9 (experiment outcome + next-step license), CHANGELOG (`Added`: perf_trace), OUTSTANDING (tick tracing item context; queue update per verdict). Commit docs; merge decision with user. NOTE: no probe sweep this time — the tracing layer SHIPS.

## Self-review notes
- The spec is the single source of truth; tasks deliberately thin to avoid drift.
- T3's walk_evidence signature change is the only cross-detector touch; None-passing keeps d2/d46/d48 byte-identical and cost-free.
- T5 may be blocked by machine load — that is a report, not a failure.
