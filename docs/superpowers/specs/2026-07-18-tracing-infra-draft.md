# Permanent tracing infrastructure — design draft (pre-external-review)

Status: DRAFT — written before gpt-5.6-sol's input lands; the final spec
synthesizes both. Motivation: three measurement waves each hand-re-added and
then swept the SAME throwaway instrumentation (stage marks + RSS, atomic
accumulators, per-detector timing, SCC stats, Jacobi per-pass telemetry).
That dance costs a build cycle per direction-check and leaves the tree
blind between waves. This makes the instrumentation PERMANENT, env-gated,
zero-cost when off.

## Requirements (distilled from Waves 1-2c — these are the things we
actually re-added every time)

R1. Stage spans: named begin/end with wall-clock + RSS (current/peak) at
    each boundary. The Wave-1 `STAGE name t=..s rss_mb=.. peak_mb=..` shape
    was exactly right for grep + timeline reconstruction.
R2. Counter accumulators: named atomic duration/count slots usable from
    parallel code (parse vs projection, Jacobi clone/compose/change-test
    splits, span BFS).
R3. Per-detector spans in the registry loop (name + wall; RSS optional —
    detector-local RSS deltas were never decisive).
R4. One-shot structural dumps, separately gated (verbose): SCC stats +
    largest-SCC anatomy (+ optional edge-pair sampling), Jacobi per-pass
    frontier telemetry for SCCs ≥ threshold.
R5. Output: (a) stderr lines exactly like today (grep-friendly, zero deps);
    (b) OPTIONAL Chrome trace-event JSON (speedscope/chrome://tracing
    loadable) written to ALSEM_TRACE_OUT for flame/timeline views — derived
    from the same span events, no second instrumentation surface.
R6. Zero-cost off: one relaxed atomic u8 bitmask load per instrumentation
    site (OnceLock-initialized from env at first touch); no formatting, no
    clock reads, no RSS syscalls when the bit is off. No output-contract
    change when off (byte-stable goldens trivially).
R7. Windows-first RSS via K32GetProcessMemoryInfo (the Wave-1 probe's
    extern block, proven); no-op stub elsewhere.
R8. No new deps for the core. The `tracing` crate is NOT justified for this:
    we need ~5 primitives, deterministic stderr text, and an exact JSON
    shape — a ~200-line bespoke `src/trace.rs` covers it without pulling a
    subscriber ecosystem into a golden-disciplined CLI. (Re-evaluate if we
    ever want per-span structured fields consumed by external tooling.)

## Env contract (draft)

- `ALSEM_TRACE` = comma list of `stages`, `detectors`, `counters`, `structs`,
  or `all`. Absent/empty → everything off (the single-load fast path).
- `ALSEM_TRACE_OUT` = path; when set AND any category on, ALSO emit Chrome
  trace-event JSON there (complete-event `ph:"X"` records built at span end;
  counters as `ph:"C"`; process RSS sampled at span boundaries only).
- The old ad-hoc vars (`ALSEM_STAGE_TIMING`, `ALSEM_EXIT_AFTER_SCCSTATS`)
  are RETIRED; `ALSEM_TRACE_EXIT_AFTER=<span-name>` generalizes the early
  exit (measurement runs that only need substrate stats).

## Module sketch (`src/trace.rs`)

```rust
bitflags-ish consts: STAGES=1, DETECTORS=2, COUNTERS=4, STRUCTS=8;
fn mask() -> u8                      // OnceLock, parsed once from env
#[inline] fn on(cat: u8) -> bool     // single relaxed load + and
pub struct Span { name, t0, cat }    // returned guard
pub fn span(cat: u8, name: &str) -> Option<Span>   // None when off
impl Drop for Span                   // emits stderr line + JSON record
pub fn accum(slot: usize, d: Duration)             // atomic slots, named
pub fn dump_accums()
pub fn structural(name: &str, f: impl FnOnce() -> String)  // lazy build,
                                     // only invoked when STRUCTS on
pub fn maybe_exit_after(name: &str)
```

Instrumentation points (~17, the exact sites the waves proved out):
gate/run.rs (fresh preflight, model-id, L3 assemble+resolve, run_detectors,
format), resolve/full.rs (snapshot/parse/dep-layer/build/resolve),
l3_workspace.rs (assemble from disk, resolve; parse/projection accums),
detector_context.rs (symbol table, calls, event graph, combined graph,
cones, summaries+indexes, closed-world, spans, compute_summaries block,
SCC structural dump), summary_runner.rs (Jacobi clone/compose/change-test
accums + per-pass telemetry ≥100-member SCCs), registry.rs (per-detector
spans), transaction_spans.rs (BFS accum).

## Open questions for the external review

- Chrome-trace JSON worth it v1, or stderr-only + a post-processing script?
- Sampling for RSS (boundary-only is the draft) vs periodic sampler thread?
- Per-detector RSS deltas: include or skip (draft: skip)?
- Should `structs` dumps carry machine-readable JSON lines instead of the
  current eprintln prose (draft: JSON-lines, greppable AND parseable)?
