# Transaction-spans arc — measurement ledger

Every number this arc claims, with the probe that produced it. Plan:
`docs/superpowers/plans/2026-07-31-transaction-spans-interning.md`.

## Probe shapes (never compared with each other)

1. **DO identity probe** — `alsem analyze <DO_WS> --format json --deterministic`,
   SHA-256 of stdout. The byte-identity oracle for every task. DO = the real Continia
   customer workspace (4,842 routines), `U:/Git/DO.Support-SlowDOSetup/DocumentOutput/Cloud`.
2. **8020 trace probe** — `ALSEM_TRACE=1 ALSEM_TRACE_DETAIL=hot` whole-run
   `alsem analyze <corpus-8020> --format json`, span totals aggregated from the B/E
   records. 8020 = BC Base App, 8,020 `.al` files / 100,941 routines.
3. **Population census** — `ALSEM_TXSPAN_CENSUS=1`, stderr line from
   `compute_transaction_spans`. Counts only; no timing.

`ALSEM_TRACE_DETAIL=hot` ALONE — not `stages,hot` (parse falls back to Stages and the
Hot counters gate off). 8020 `analyze.total` swings **±80 s** run to run on this machine;
only same-run SPAN totals carry a claim.

## Baseline

Branch base: `cb9e67b` (master). Binary: `--profile release-fast`, built with
`TREE_SITTER_AL_PATH=U:/Git/al-call-hierarchy/tree-sitter-al` (worktree has no submodule
checkout of its own).

### DO identity baseline

```
f022f677d2650b2399fc3aa5a7625bc6c078d90dd51cdb80e1e3705808fee3ea  logs/txspan-base-do.json
```

This is the SAME hash the uncertainty-substrate arc recorded in `CHANGELOG.md` for DO
(`f022f677…`), so the baseline reproduces committed evidence rather than merely being
self-consistent.

### Prior-run attribution that motivated the arc

From `logs/trace-d1final.json` in the main checkout (8020, post-d1-memory arc, default
preset) — the measurement that put this module on the critical path:

| span | wall | % of `analyze.total` (195.32 s) |
|---|---:|---:|
| `context.transaction_spans` | 76.16 s | 39.0 % |
| `preflight.fresh_coverage` | 21.54 s | 11.0 % |
| `context.capability_cones` | 21.15 s | 10.8 % |
| `detector.d1-db-op-in-loop` | 20.61 s | 10.6 % |
| ├ `search_loops_cohorts` › `scoring` | 16.67 s | 8.5 % |
| └ `assemble_cohort_findings` | 1.21 s | 0.6 % |
| `context.compute_summaries` | 11.61 s | 5.9 % |

`context.transaction_spans` retains almost nothing (`rss_delta_mb` 73) — this is a TIME
lever, not a memory one.

### Baseline span totals (this branch, this machine)

`logs/trace-txspan-base.json`, 8020, whole run, default preset:

| span | wall | % of `analyze.total` (206.43 s) |
|---|---:|---:|
| `preflight.fresh_coverage` | 71.81 s | 34.8 % |
| `context.transaction_spans` | **58.90 s** | **28.5 %** |
| `context.capability_cones` | 18.55 s | 9.0 % |
| `detector.d1-db-op-in-loop` | 13.73 s | 6.7 % |
| `context.compute_summaries` | 8.91 s | 4.3 % |

**Read the two traces together, not one over the other.** Against the earlier
`trace-d1final.json` run, `context.transaction_spans` moved 76.16 → 58.90 s and
`preflight.fresh_coverage` moved 21.54 → **71.81 s** — a +50 s swing in a span this
branch does not touch. That is the recorded ±80 s machine noise (plus, for preflight
specifically, a plausible cold-file-cache effect: this was the first 8020 run after a
fresh worktree checkout). Consequences for this arc, stated up front:

- The arc's claim is the `context.transaction_spans` DELTA measured against ITS OWN
  same-branch baseline (58.90 s), not any share-of-total figure.
- `analyze.total` movement is NOT this arc's evidence and will not be claimed as such.
- `preflight.fresh_coverage` is now nominally the largest span. It is not re-ranked as
  the next target on one cold-cache observation — that needs its own warm re-measure.

### Population census

`ALSEM_TXSPAN_CENSUS=1`:

| corpus | `template_calls` | `templates` (BFS walks) | `visited_total` | `spans_emitted` | `payload_strings` | `materialized_strings` | mean cone |
|---|---:|---:|---:|---:|---:|---:|---:|
| DO | 51 | 49 | 3,600 | 56 | 8,230 | 243,111 | 73.5 |
| 8020 | 955 | 927 | 129,350 | 1,061 | 2,390,888 | **261,772,789** | 139.5 |

### What the census falsified, and what it found

**Falsified — the backward BFS.** 927 walks over **129,350** total visited-routine steps
cannot account for 58.90 s; at even 1 µs per step that is 0.13 s. The plan's Task 1
(interned-ix `SpanIndex`, CSR reverse adjacency, generation-stamped visited array) would
have been a careful rewrite of something that costs nothing. It was NOT built. The
`VecDeque<(String, usize)>` clones and `BTreeSet<String>` visited set are ugly but they run
129 k times, not 261 M.

**Confirmed — the per-visited-routine materialization.** `aggregate_span` calls
`ConeDerivedStore::writes_tables_of` / `publishes_events_of` once per visited routine, and
each call resolves that routine's whole folded-cone window into a fresh `Vec<String>` which
is then inserted element-by-element into a `BTreeSet<String>` and dropped:
**261,772,789 strings allocated and discarded per 8020 run**, 2,023 per visited routine.
(Base App's single enormous SCC is why each routine's folded cone names thousands of
tables/events.) This is the 58.90 s.

**Mostly falsified — the per-op payload clone.** 1,061 spans come from 927 templates, so
only 134 spans carry a duplicated payload: sharing them saves ~13 % of the 2,390,888
retained payload strings, on a span that retains 73 MB. Demoted to an optional, gated task.

The reusable lesson is the one this repo has now written down twice: **measure the
population before building the taxonomy for it.** Two of the three assumed cost centres
did not exist at the assumed scale, and only the census could tell them apart — the profile
attributes time to the SPAN, not to which line inside it.

## Per-task results

### Task R1 — union interned ids in a bitset, resolve once per template

**The primary evidence is a COUNT, not a timing.** `materialized_strings` is a
deterministic census of how many `String`s the union allocates; it carries no noise band
at all:

| 8020 | base `a642bec` | after R1 |
|---|---:|---:|
| `materialized_strings` | 261,772,789 | **1,762,840** (−99.33 %) |
| `context.transaction_spans` | 58.90 s | **1.14 s** (−57.76 s, −98.1 %) |

Every other census figure is byte-for-byte unchanged (`template_calls=955`,
`templates=927`, `visited_total=129350`, `spans_emitted=1061`,
`payload_strings=2390888`, `mean_cone=139.5`) — the walk, the cache behaviour and the
emitted payloads are untouched; only the union's representation changed.

**Identity evidence.**
- DO `--deterministic` SHA-256 `f022f677d2650b2399fc3aa5a7625bc6c078d90dd51cdb80e1e3705808fee3ea`
  — unchanged from the Task-0 baseline and from the hash the uncertainty arc recorded.
- The two non-deterministic 8020 runs differ in exactly **6 bytes**, all inside the
  `generatedAt` timestamp (first differing offset 14,944); the remaining 251,169,090 bytes
  are identical.
- Four discrimination proofs, each PASS → FAIL → PASS:
  1. drop `out.sort()` in `resolve_sorted_ids` →
     `span_union_is_deduped_string_sorted_and_coverage_follows_a_missing_summary` FAILS
     (intern order is not lexicographic — the fixture declares `t/Z` before `t/A`).
  2. missing-summary arm reduced to a bare `continue` → that test AND the pre-existing
     `missing_summary_makes_coverage_incomplete_and_contributes_nothing` FAIL.
  3. `ResBitset::insert_all` uses `=` instead of `|=` → both bitset tests FAIL.
  4. `writes_table_ids_of` returns the whole pool instead of the row window →
     `id_accessors_agree_with_the_string_accessors` FAILS.

  Proof 4 initially reported a false PASS: the patch text did not match after rustfmt
  reflowed the body, so the break was never applied. Recorded because "the break ran and
  the test still passed" and "the break never ran" are indistinguishable from an exit
  code alone — an assert on the patch text is what caught it.

**What is NOT claimed.** `analyze.total` moved 206.43 s → 76.15 s, and **that is not this
change's doing.** `preflight.fresh_coverage` — a span this commit does not touch — moved
71.81 s → 4.89 s in the same pair of runs, which is the cold-file-cache artefact the
baseline section already flagged. The supported claim is the
`context.transaction_spans` delta (−57.76 s) plus the allocation census (−260.0 M
strings); the rest of the wall movement is cache state, not this arc.

## Build hazard recorded

`cargo build` reports **exit 101 with `error: failed to remove file … alsem.exe` /
`Access is denied. (os error 5)`** when a previous `alsem.exe` run still holds the
binary — and a shell wrapper whose last command is a `grep` will report success anyway.
Always verify `target/release-fast/alsem.exe`'s mtime moved before trusting a rebuild.
