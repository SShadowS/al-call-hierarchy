# The 8020 profile, re-ranked and fully attributed

Taken at master `5918b4b`, immediately after the `perf/side-facts` and
`perf/cone-singleton` arcs moved two of the three spans the previous lever list
was ordered by. The list in `docs/OUTSTANDING.md` was stale by construction, so
this re-measures before anything is chosen — and the re-measurement found that
the largest region in the profile had never been on any list, because no span
covered it.

## Method

`ALSEM_TRACE=1 ALSEM_TRACE_DETAIL=hot`, `alsem analyze <corpus-8020> --format json`,
`release-fast`. Spans aggregated from the B/E records into **inclusive total** and
**self** (exclusive of nested spans on the same tid) — the self column is the one
that ranks work, and it is what exposed the gap.

**Only same-run shares are claimed.** Four baseline runs of the SAME binary
measured `analyze.total` at 56.5 / 63.5 / 88.9 / 91.6 s — a 1.6× spread on one
machine — while every span's share of the run stayed within about a point. An
absolute millisecond figure from this machine means nothing; a share means
something.

## The baseline ranking, by SELF time (4 runs, median share)

| region | self % | per-run | @76.2 s median |
|---|---:|---|---:|
| `context.capability_cones` | 17.8 % | 16.1/16.2/19.3/19.9 | 13.5 s |
| **`analyze.total` — UNATTRIBUTED** | **15.5 %** | 16.5/16.0/15.0/14.2 | **11.8 s** |
| `context.compute_summaries` | 10.5 % | 10.4/11.2/9.9/10.7 | 8.0 s |
| **`l4_l5.run_detectors` — UNATTRIBUTED** | **9.3 %** | 8.6/9.3/9.5/9.4 | **7.1 s** |
| `preflight.fresh_coverage` | 7.8 % | 8.3/8.2/7.0/7.4 | 5.9 s |
| `gate.format` | 4.8 % | 5.3/5.0/4.3/4.6 | 3.6 s |
| `detector.d2-event-fanout-in-loop` | 4.4 % | 3.8/4.3/4.6/4.5 | 3.3 s |

**24.8 % of the run — 18.9 s — was inside two spans that named none of it**, more
than the largest named lever. `analyze.total` and `l4_l5.run_detectors` are both
long-lived brackets whose children do not tile them.

Two things the same table falsifies without any new work:

- **d1 is off the lever list.** `docs/OUTSTANDING.md` ranked `detector.d1`'s
  `scoring` at 10.56 s / 13.9 %, third overall. It measures **1.8 %** here
  (`search_loops_cohorts` self 2.9 % + `scoring` 1.8 % + `assemble_cohort_findings`
  1.7 % ⇒ d1's whole tree is ~6.4 %). That entry was written against a profile
  taken before the d1 interning fix; it should never have survived into the
  post-fix list.
- **`preflight.fresh_coverage` is real and steady at ~8 %**, not the file-cache
  artifact the earlier note allowed for — it holds 7.0–8.4 % across four runs
  including the two slow ones.

## The attribution

Four spans added (`gate.model_instance_id`, `gate.teardown`,
`context.build_total`, `context.ctx_drop`, `l4_l5.role_scope_and_sort`). They are
the census: `pt::span` costs a single `OnceLock` read when tracing is off, so
this is permanent attribution rather than a probe to be removed.

`gate.teardown` and `context.ctx_drop` required making implicit drops explicit.
That is a reorder, not a behaviour change: the structures were already being freed
inside those spans (`_analyze_span` is the first local declared in
`run_analyze_with_exit`, so it drops LAST, after every structure below it), and the
engine has exactly two `Drop` impls, both `perf_trace`'s own guards. Drop order is
forced by the borrows — `paired` holds `&Finding`s into `run.findings` and `idx`
borrows `resolved.workspace`.

**Result: `analyze.total` self 12,995 ms → 2.9 ms, `l4_l5.run_detectors` self
8,456 ms → 0.2 ms.** Both regions are now fully named.

| new span | @89.8 s median | self % |
|---|---:|---:|
| **`gate.teardown`** | 12,430 ms | **13.8 %** |
| `context.build_total` (its own unspanned stretches) | 3,226 ms | 3.6 % |
| **`context.ctx_drop`** | 2,502 ms | **2.8 %** |
| `l4_l5.role_scope_and_sort` | 2,244 ms | 2.5 % |
| `gate.model_instance_id` | 51 ms | 0.1 % |

**`gate.model_instance_id` is falsified as a suspect.** It was the one named
candidate for `analyze.total`'s self time — a SECOND full `discover_al_files` disk
walk of the workspace on top of the one `l3.discover_read` already does. It costs
51 ms. The duplicated walk is real and remains a (tiny) redundancy; it is not a
lever.

## What the 18.9 s actually was

**`gate.teardown` + `context.ctx_drop` = 16.6 % of the run — 14.9 s — is
`free()`.** Deallocating the L3 model, the detector context (cones, summaries,
spans), the findings and the projection index costs more than
`context.compute_summaries` computes them in, and nearly as much as
`context.capability_cones`.

This is a structural property of the workload, not a defect in any one function:
the engine's resident model is millions of small `String`s, and the process is a
batch CLI that exits immediately afterwards.

## The re-ranked lever list (3 attribution runs, median self share)

| region | self % | @89.8 s |
|---|---:|---:|
| `context.capability_cones` | 19.1 % | 17,164 ms |
| **`gate.teardown`** | 13.8 % | 12,430 ms |
| `context.compute_summaries` | 9.8 % | 8,840 ms |
| `preflight.fresh_coverage` | 8.0 % | 7,179 ms |
| `detector.d2-event-fanout-in-loop` | 4.5 % | 4,048 ms |
| `gate.format` | 4.2 % | 3,793 ms |
| `context.build_total` (self) | 3.6 % | 3,226 ms |
| `gate.project_filter_scope_baseline_suppress` | 3.1 % | 2,802 ms |
| `search_loops_cohorts` (self) | 3.1 % | 2,749 ms |
| **`context.ctx_drop`** | 2.8 % | 2,502 ms |
| `gate.workspace_diagnostics` | 2.6 % | 2,318 ms |
| **`l4_l5.role_scope_and_sort`** | 2.5 % | 2,244 ms |

## Gates

DO `f022f677d2650b2399fc3aa5a7625bc6c078d90dd51cdb80e1e3705808fee3ea` and 8020
`36151bf67e17620724abb6b2cdbad55bcf8f97ffe3c3237782a0cf4c25ecc5fb`, both exact.
`scripts/check-goldens` green with zero files under `tests/` moved;
`cargo clippy --all-targets --all-features` clean.
