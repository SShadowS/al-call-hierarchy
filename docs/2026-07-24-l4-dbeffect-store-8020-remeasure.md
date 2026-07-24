# L4 db-effect Store Redesign — 8020/CDO re-measure (Task A5)

Branch `feat/l4-summary-redesign`, measured at HEAD after Part A (A1–A5) of the
`docs/superpowers/plans/2026-07-22-l4-dbeffect-store-and-retirement.md` plan.
All measurements per-process (working set via `K32GetProcessMemoryInfo(GetCurrentProcess())` —
`src/engine/perf_trace.rs:132`), corroborated by PowerShell `Get-Process alsem` `PeakWorkingSet64`.

## CDO real-workspace parity (Step 1 — mandatory before Part B)

`cdo_whole_program_v2_parity` (`tests/l4_summary_differential.rs`) — v2 (the new interned
columnar EffectStore, all of A1–A4) vs the retained OLD Jacobi solver, complete-`RoutineSummary`
parity for **every** routine on the real Continia Document Output workspace.

```
CDO_WS=U:/Git/DO.Support-SlowDOSetup/DocumentOutput/Cloud ENFORCE_CDO_WS=1 \
  cargo test --release --test l4_summary_differential cdo_whole_program_v2_parity -- --nocapture
```
Result: **PASS** (`ENFORCE_CDO_WS=1`, so genuinely ran — not skipped). v2 is byte-identical to old
on real cross-app AL source. (Modest scale — the DO app workspace; the 8020 synthetic corpus below
is the perf-scale proof.)

## 8020 synthetic corpus — full `alsem analyze` (Step 2)

`alsem analyze <corpus-8020> --detector d1-db-op-in-loop --format json`, `release-fast`, hot trace.

| metric | pre-arc (Phase 1, 2026-07-22) | post-Part-A (this measure) |
|---|---:|---:|
| `analyze.total` (WALL) | 1322 s | **620 s** |
| `context.compute_summaries` span | 540.7 s | 87.1 s |
| — `db_solver_ms` (phase-split) | 516 523 ms (~517 s) | **11 764 ms (~11.8 s)** |
| — `roles_ms` | 813 ms | 875 ms |
| peak process working set | ~40.5 GB | 39.9 GB |
| EXITCODE | 0 | 0 |

**The db-effect solver compute hit the seconds target** — 517 s → 11.8 s (and the old Jacobi it
replaced was ~729 s). The SCC-shared `EffectSetId` store (A3) collapsed the per-member materialization
that dominated the closed-form solver's constant factors.

### Why the `compute_summaries` span is still 87 s (not ~13 s), and peak RSS is still ~40 GB

Phase-split shows the actual solver work is `db_solver_ms` + `roles_ms` ≈ **13 s**. The remaining
~74 s of the span, and ~24 GB of the peak, are the **compat shim** at `summary_runner.rs:1405`:

```rust
summary.db_effects = bundle.db_effects(rix).map(|e| e.to_owned()).collect();
```

This re-materializes a per-member owned `Vec<DbEffect>` for every routine (re-expanding the shared
store into ~tens of millions of owned `DbEffect`), purely so the returned `HashMap<String,
RoutineSummary>` keeps the legacy shape the retained old-solver **differential oracle** compares
against. Trace attribution (per-span `rss_mb`):

| span | resident RSS |
|---|---:|
| `context.capability_cones` | 16.4 GB |
| `context.compute_summaries` | 38.9 GB (**+24 GB** from the shim) |

The analyze path never *reads* those `db_effects` (no detector under `src/engine/gate/` or
`src/engine/l5/` reads `RoutineSummary.db_effects`; it consumes only `.uncertainties` /
`.parameter_roles` / capability facts). So the 24 GB — and the ~74 s to build it — were waste on the
analyze path, held only for the oracle. (**Since fixed at B1** — the analyze path no longer calls the
shim at all; see the next section for the measured outcome. The shim itself survives, unchanged, for
the projection + differential callers that really do read `db_effects`.)

## RSS win — LANDED at B1 (was "deferred" when this section was first written)

> **Superseded.** This section originally recorded the db-effect RSS win as *deferred by design*.
> It has since **landed** — B1's consumer migration shipped and was re-measured. The prediction
> below is kept only because the measured outcome is checked against it.

The db-effect RSS win (the 24 GB) required a **consumer migration** (point the analyze path at the
bundle's borrowing view), which was only possible once the old-solver oracle was gone. Per the plan's
sequencing and the user's decision it was **folded into Part B (B1)**: after B1 deleted the old Jacobi
and flipped the differential to a frozen baseline, the analyze path consumes the `SummaryBundle` lazily
(projection + the differential keep a materializing path; `db_effects` stays queryable via the
bundle + the A4 `ReverseEffectIndex`). Predicted at B1: `compute_summaries` ~87 s → ~13 s and −24 GB.

**Measured at B1** (8020, `release-fast` @`a0cd348`, FULL detector set, EXIT=0, WALL 620 s → 366 s):

| metric | post-Part-A | post-B1 (measured) |
|---|---:|---:|
| `context.compute_summaries` `rss_delta` | 24 250 MB | **477 MB** (24 GB → 0.47 GB — sub-GB target hit) |
| `context.compute_summaries` wall | 87 s | **15.7 s** (the ~74 s shim materialization is gone) |
| whole-process peak working set | 39.9 GB | **18.1 GB** |

The prediction held on both axes. What remained after B1 was **~10 GB in
`context.capability_cones`** (`rss_delta` 9 989 MB on that same full-detector run) — a separate,
pre-existing base-assembly cost (cone propagation, `compose_cone_over_graph`), NOT the db-effect
store. That became follow-up task C1 (diagnosis in `.superpowers/sdd/C1-cones-diagnosis.md`).

## C1 — `context.capability_cones` (the remaining cone cost)

Measured in the **d8-only** run shape (`--detector d8-commit-in-transaction`), which is what every
C1 measurement uses; that shape reads the pre-C1 cone span at 10 941 MB where B1's full-43-detector
run read 9 989 MB — a run-shape/allocator difference, not a discrepancy. Compare C1 numbers only
against other C1 numbers.

| metric (8020, d8-only, `release-fast`) | pre-C1 | post-C1 Task 3 | post-C1 Task 4 |
|---|---:|---:|---:|
| `context.capability_cones` `rss_delta` | 10 941 MB | 2 151 MB | *controller re-measure pending* |
| whole-process peak working set | 17 055 MB | 9 593 MB | *controller re-measure pending* |

- **Task 3** stopped materializing the per-routine raw `Vec<CapabilityFact>` inherited cone on the
  analyze path (`ConeOutput::DerivedOnly` — the compact `ConeDerivedStore` carries every analyze
  consumer): −8.8 GB on the span, −7.5 GB whole-process peak.
- **Task 4** closed the residual the `C1_CONE_CENSUS=1` byte census then attributed
  (`.superpowers/sdd/c1-residual-census.md`): 74% of the 2 151 MB was `fact_cones` entries for
  call-graph **root** SCCs that the walk's refcount-free could never reach. Census, same corpus,
  before → after:

  | census line | before (41d418a) | after |
  |---|---:|---:|
  | `fact_cones` residual at walk exit | 17 864 SCCs / 2 224 901 entries / **1 598.87 MB** | **0 / 0 / 0.00 MB** |
  | `direct_in` transient duplicate | 79.66 MB | *structure deleted* |
  | `direct` dedup-keyed walk input | 63.70 MB (held to function exit) | 58.33 MB (dropped at last use) |
  | `capability_facts_direct` (retained) | 71.56 MB | 64.61 MB |
  | `size_of::<CapabilityFact>()` | 408 B | 368 B |
  | `grand_total_bytes` (retained after the build) | 157.74 MB | 150.79 MB |

  The census is a deterministic BYTE count of specific structures, not an RSS measurement — the
  authoritative post-Task-4 8020 RSS/peak re-measure is the controller's and is not recorded here
  yet. (`capability_facts_direct`'s "after" additionally reflects the census's own Task-4 fix:
  a `&'static str` field's content is shared program-wide and is no longer charged per fact. The
  like-for-like number under the OLD accounting is 67.42 MB.)

Whole-process peak is floored by the cones plus the ~5 GB workspace IR (`l3.assemble_resolve`
3 386 MB and `l3.parse_project_parallel` 2 771 MB are the largest spans post-Task-3), so the literal
"<1 GB whole-process" target is not reachable by B1 + C1 alone — those L3 spans are the next floor.

## perf gate (Step 3)

`tests/perf_bounds.rs`'s `compute_summaries_v2_within_bound` (1000-file, NON-recursive corpus) was
re-measured: medians 57.2 / 80.0 / 67.8 / 71.3 ms — overlapping the prior ~76 ms baseline. Not
materially different (the store redesign's win is on dense **recursive** SCCs, which this corpus
lacks by construction), so the 230 ms (3×) bound is kept, comment updated to record the re-measure
(commit `0f397d8`). No memory assertion added — deferred to post-B1, when the db_effects RSS actually
drops. **Still not added** now that B1 (and C1) have landed: the gate remains wall-clock-only, so no
in-repo test would catch a re-materialization regression by memory. Open item.
