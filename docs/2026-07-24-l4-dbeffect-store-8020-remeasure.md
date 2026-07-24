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
`.parameter_roles` / capability facts). So the 24 GB — and the ~74 s to build it — are waste on the
analyze path, held only for the oracle.

## RSS win — deferred (by design + user decision, 2026-07-24)

The db-effect RSS win (the 24 GB) requires a **consumer migration** (point the analyze path at the
bundle's borrowing view), which is only possible once the old-solver oracle is gone. Per the plan's
sequencing and the user's decision it is **folded into Part B (B1)**: after B1 deletes the old Jacobi
and flips the differential to a frozen baseline, the analyze path consumes the `SummaryBundle` lazily
(projection + the differential keep a materializing path; `db_effects` stays queryable via the
bundle + the A4 `ReverseEffectIndex`). Expected at B1: `compute_summaries` ~87 s → ~13 s and −24 GB.

The remaining **~16 GB is `context.capability_cones`** — a separate, pre-existing base-assembly cost
(cone propagation, `compose_cone_over_graph`), NOT the db-effect store. Per the user's decision it is
tackled as a follow-up task (C1) — diagnosis in `.superpowers/sdd/C1-cones-diagnosis.md`. Whole-process
peak is floored by cones + the ~5 GB workspace IR, so the literal "<1 GB whole-process" target is only
approachable after both B1 (−24 GB) and C1 (−~11 GB) land.

## perf gate (Step 3)

`tests/perf_bounds.rs`'s `compute_summaries_v2_within_bound` (1000-file, NON-recursive corpus) was
re-measured: medians 57.2 / 80.0 / 67.8 / 71.3 ms — overlapping the prior ~76 ms baseline. Not
materially different (the store redesign's win is on dense **recursive** SCCs, which this corpus
lacks by construction), so the 230 ms (3×) bound is kept, comment updated to record the re-measure
(commit `0f397d8`). No memory assertion added — deferred to post-B1, when the db_effects RSS actually
drops.
