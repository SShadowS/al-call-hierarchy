# Permanent perf-tracing infrastructure — FINAL spec (synthesized)

Status: FINAL. Synthesis of the pre-review draft (this file's git history) and
gpt-5.6-sol's design review (2026-07-18, via pi delegate; every load-bearing
claim source-verified before adoption — see "Verified findings" below).
Motivation unchanged: three measurement waves each re-added and swept the same
throwaway instrumentation; this makes it permanent, env-gated, zero-cost off.

## Verified findings that shaped this spec (and the next experiment)

1. **Walker cut-result explosion (VERIFIED, worse than reported).** Once
   `nodes_visited >= max_nodes`, `visit` appends a `NodeBudgetCut` result —
   with CLONED `steps` + `uncertainties` vectors — for EVERY remaining edge
   at EVERY level of the recursion (`src/engine/l5/path_walker.rs:201-227`).
   d1 discards every non-`Complete` result, and the Wave-2c memo now RETAINS
   these bags per callee for the whole run (`d1.rs` walk_memo). This is the
   strongest unmeasured hypothesis for the residual 8020 time AND the
   45-52 GB peaks. It is a MEASUREMENT TARGET first (hot counters below),
   fix (`CompleteOnly` streaming walk mode) licensed only by the numbers.
2. **d1-only is a valid cheap isolation run**: d1 demands
   SUMMARIES|CORE_SUMMARIES|CLOSED_WORLD_TEMP but not spans
   (`l5/detectors/mod.rs:941-953`) — substrate + d1 observable without the
   other 54 detectors or a 2h full-default run.
3. **B2 cones ≠ Jacobi**: cones feed `FullRoutineSummary`; the Jacobi block
   independently produces core `RoutineSummary` (uncertainties/roles). B2 is
   an RSS lever, not a Jacobi lever. Jacobi levers = representation
   (core-summary interning wedge: `EffectIx`/`UncertaintyIx` + bitsets,
   B1-narrow before B1-wide) + domain decomposition (split set-like
   db-effects/uncertainties from the small parameter-role lattice;
   semi-naive only for proven-monotone domains).
4. Rejected as next moves (adopted from the review, reasons in the pi
   transcript): Salsa/incrementality (cold-run irrelevant), corpus
   partitioning (cannot split an SCC), parallel detectors (multiplies
   working sets at 50 GB peaks), B3 (refunds the wrong cost), silent caps.

## Module: `src/engine/perf_trace.rs` (bespoke, no new deps)

NOT the `tracing` crate: we need 5 primitives + exact deterministic output +
Windows RSS + crash-safe checkpointing; a subscriber ecosystem buys none of
that and adds a process-global surface. ~250-350 lines. Fully separate from
product telemetry (local file only, no upload, no source text).

### API

```rust
pub enum Detail { Stages, Jacobi, Hot }         // cumulative tiers
pub fn enabled(d: Detail) -> bool;              // OnceLock<Option<Config>>, one load
pub fn run(name: &'static str) -> RunGuard;     // top-level process span
pub fn span(cat: &'static str, name: impl Into<TraceName>) -> SpanGuard;
pub fn counter(name: &'static str, v: u64);
pub fn counter_delta(name: &'static str, dv: u64);
pub fn instant_lazy(cat: &'static str, name: &'static str,
                    build: impl FnOnce() -> serde_json::Value);  // structs
pub struct LocalCounters { .. }   // plain u64 fields for hot loops;
                                  // flush(category) at scope end
pub fn maybe_exit_after(span_name: &str);  // generalizes the old early-exit
```

Hot-loop rule: test `enabled(Hot)` ONCE outside the loop, thread
`Option<&mut LocalCounters>` down. No per-node atomics/clocks/allocs.

### Disabled-path contract

`ALSEM_TRACE` absent/`0`: no files, no output, no clocks, no K32 calls, no
allocation, no JSON; a single predictable OnceLock test at coarse sites.
"Zero measurable" is an ACCEPTANCE TEST: Criterion micro + repeated DO runs,
<1% pipeline-level guard.

### RSS

Direct FFI: `GetCurrentProcess` + `K32GetProcessMemoryInfo` with
`PROCESS_MEMORY_COUNTERS_EX` (WorkingSetSize, PeakWorkingSetSize,
PrivateUsage). Span begin/end capture + signed delta; documented as
two-instant observations, not live-allocation accounting. Non-Windows: None.

### Output: Chrome Trace Event JSON (chrome://tracing / Perfetto)

`B`/`E` spans, `i` one-shots, `C` counters/RSS, `M` metadata; µs monotonic
timestamps; pid + stable synthetic tid. CRASH-SAFE: the `B` event is written
immediately; the file is checkpointed after every coarse stage and on a
timed interval, so a cap-killed run still shows the active span. Trace-write
failures fail OPEN (never fail analysis). Explicitly NOT a sampling
flamegraph — nested timeline only; stack-level CPU attribution stays with
ETW/WPA if ever needed.

Delta vs the review: `ALSEM_TRACE_STDERR=1` additionally mirrors
coarse-stage lines to stderr in the Wave-1 `STAGE name t=..s rss_mb=..`
shape — grep-proven during the waves; opt-in so default channels stay
untouched.

### Env contract

```
ALSEM_TRACE=0|off              default
ALSEM_TRACE=1|chrome           enabled, Chrome JSON
ALSEM_TRACE_FILE=<path>        default alsem-trace-<pid>.json
ALSEM_TRACE_DETAIL=stages|jacobi|hot   (cumulative; default stages)
ALSEM_TRACE_SAMPLE_EVERY=N     per-walk timing sample rate (default 64)
ALSEM_TRACE_SCC_MIN=N          per-SCC detail threshold (default 100)
ALSEM_TRACE_STDERR=1           mirror coarse spans to stderr (opt-in)
ALSEM_TRACE_EXIT_AFTER=<span>  early-exit after named span closes
```

### Permanent instrumentation points (~20)

- `gate/run.rs` `run_analyze_with_exit`: analyze.total,
  preflight.fresh_coverage, l3.assemble_resolve, l4_l5.run_detectors,
  gate.workspace_diagnostics, gate.project_filter_scope_baseline_suppress,
  gate.coverage, gate.format.
- `l3/l3_workspace.rs`: l3.discover_read, l3.parse_project_parallel,
  l3.resolve (+ parse/projection LocalCounters).
- `l5/detector_context.rs`: context.symbols_resolve_calls,
  context.event_combined_graph, context.capability_cones,
  context.transaction_spans, context.core_scc_tarjan,
  context.compute_summaries, context.final_indexes; STRUCTS: SCC stats +
  largest-SCC anatomy instant_lazy.
- `l4/summary_runner.rs` (Detail::Jacobi only): one span per recursive SCC ≥
  SCC_MIN; per-pass counters (pass, dirty in/out, recomputed, cardinality by
  DOMAIN — db_effects/uncertainties/roles split); final unique-universe +
  member-overlap dump for the largest SCC (feeds the B1-narrow decision).
  Never per-singleton events or per-member clocks.
- `l5/registry.rs`: detectors.total + `detector.<name>` child span per
  detector with findings count + RSS delta.
- `l5/detectors/d1.rs` + `path_walker.rs` (Detail::Hot): the d1 walk census
  BEFORE walking (candidates, distinct callee roots) + aggregate counters
  (memo hits/misses, nodes visited, edges examined, results by
  Complete/CycleCut/DepthCut/NodeBudgetCut/DeadEnd, retained WalkResults +
  steps totals per memo entry, walks hitting the node bound, RSS checkpoint
  every N memo misses). The decisive ratio: unused_cut_results /
  all_retained_results.

### Acceptance gates for the tracing change itself

Goldens byte-stable; DO diff with tracing OFF **and ON** (ON must not change
analysis output, only emit the side file); parseable-Chrome-JSON unit test;
Windows RSS smoke test; killed-run checkpoint test (valid JSON with open
span); disabled-path Criterion bench (<1%).

## The decisive experiment this unlocks (next after landing)

Traced **8020 d1-only** run (`--detector d1-db-op-in-loop`,
ALSEM_TRACE_DETAIL=hot, quiet machine): census before walking; aggregate
stop-kind counts + retained-memo size; checkpoint every 1000 memo misses;
distinguishes Jacobi-vs-d1, too-many-callees-vs-expensive-walks,
complete-path-explosion-vs-ignored-cut-explosion, memo-RSS-vs-summary-RSS in
ONE run. Branch on the result: cut-explosion → `CompleteOnly` streaming walk
mode (outputs preserved — after budget exhaustion no child can be visited,
so skipping ignored cut records preserves d1's complete paths; prove via the
memoized≡fresh oracle + goldens + DO + a synthetic high-fan-out fixture);
complete-path multiplicity → measure before promising wins (output includes
additionalPaths; work is bounded below by output size); Jacobi → the
population instrumentation already lands with Detail::Jacobi, then the
core-summary interning wedge.
