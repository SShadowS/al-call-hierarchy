# `context.compute_summaries` — census ledger

Attribution for the span that was the largest UNMEASURED item in the 8020 profile.
No fix is proposed here; this document is the measurement the fix has to be built
against. Probe: `ALSEM_SUMMARIES_CENSUS=1` (added on branch `perf/summaries-census`).

## Probe shapes (never compared with each other)

1. **8020 trace probe** — `ALSEM_TRACE=1 ALSEM_TRACE_DETAIL=hot` whole-run
   `alsem analyze <corpus-8020> --format json`, span totals aggregated from the B/E
   records. 8020 = BC Base App, 100,941 routines.
2. **Phase census** — `ALSEM_SUMMARIES_CENSUS=1`, one stderr block emitted at the
   span boundary in `build_detector_context`. Timers only fire when the census is
   on (`start()`/`add_since()`); populations are accumulated in plain locals and
   folded into the atomics ONCE per call, so no per-edge atomic exists on the hot
   path.
3. **DO identity probe** — `alsem analyze <DO_WS> --format json --deterministic`,
   SHA-256 of stdout.

`analyze.total` swings ±80 s on this machine. **Only same-run shares are claimed
below.** Three 8020 runs of this branch measured `context.compute_summaries` at
10.33 s, 8.00 s and 8.14 s while `analyze.total` moved 75.76 / 64.17 / 73.59 s —
the span tracks the machine, not the branch.

## The probe does not distort what it reports

Same binary, same corpus, census OFF vs ON:

| run | `ALSEM_SUMMARIES_CENSUS` | `context.compute_summaries` | `analyze.total` | `context.capability_cones` |
|---|---|---:|---:|---:|
| `trace-8020-off.json` | off | 10.22 s | 79.43 s | 14.84 s |
| `trace-8020-census3.json` | on | 8.14 s | 73.59 s | 11.19 s |

The census-ON run is FASTER, and every other span moved with it by the same
~1.25×. That is the machine, and it means the probe's own cost is below the noise
floor — it is not silently inflating the number it reports (the failure the
`cones_census` probe actually hit, and the reason every timer here is gated).

## Byte identity

Both corpora, `--deterministic`, both matching the committed gate hashes exactly.
DO was run twice — census OFF and census ON — and produced the same bytes both
times, so the probe changes no output on either setting.

```
f022f677d2650b2399fc3aa5a7625bc6c078d90dd51cdb80e1e3705808fee3ea  DO
36151bf67e17620724abb6b2cdbad55bcf8f97ffe3c3237782a0cf4c25ecc5fb  8020
```

## Attribution (8020, `logs/err-8020-census3.log`)

`phases_total` 7,902.8 ms against a traced span of 8.14 s in the same run: the
census accounts for **97 %** of the span, so nothing large is hiding outside the
probes.

Populations: 100,941 routines · 100,010 Tarjan SCCs · 100,922 members assembled.

### Level 1 — the span

| phase | ms | % |
|---|---:|---:|
| `scc_loop` | 6,806.1 | **86.1 %** |
| `finish` (`universe.freeze` + `SummaryBundleBuilder::finish`) | 714.2 | 9.0 % |
| `prologue` (six scaffolding maps) | 366.9 | 4.6 % |
| `field_index` (the caller's own build) | 15.5 | 0.2 % |

The prologue — the part that looks expensive because it clones 100 k `String` ids
into four maps — is 4.6 %. `base_summaries` (100,922 `base_intraprocedural_summary`
calls) is 215.0 ms of it; `routines_by_id` 17.7 ms, `body_avail` 16.8 ms,
`stable_map` 35.1 ms, `interner` 52.6 ms, `uncertainty_edges_by_from` 4.9 ms,
`build_rvid_by_opid` 24.9 ms (54,584 entries), `seed_fixed_leaf_rows` 0.0 ms.

### Level 2 — inside the SCC loop

| phase | ms | % of span |
|---|---:|---:|
| `db_solver` (`solve_scc_db_effects`) | 5,639.9 | **71.4 %** |
| `roles` (`run_one_scc_roles`, the surviving JACOBI fixpoint) | 777.8 | 9.8 % |
| `assemble` (member `RoutineSummary` build + `v2_map.insert`) | 388.4 | 4.9 % |

### Level 3 — inside `db_solver`

| phase | ms | % of span |
|---|---:|---:|
| `side_facts` (`solve_side_facts`) | 3,676.5 | **46.5 %** |
| `out_assemble` | 401.6 | 5.1 % |
| `write_rows` (`push_terminal_set` + `materialize_member_row`) | 387.6 | 4.9 % |
| `via` (`reconstruct_via`) | 376.4 | 4.8 % |
| `pd_reach` (`solve_pd_reachability`) | 241.6 | 3.1 % |
| `union` (`closed_form_union`) | 177.9 | 2.3 % |
| `eff_sccs` (`effective_sccs`) | 128.3 | 1.6 % |
| `pd_via` (`attribute_pd_substituted_via`) | 34.8 | 0.4 % |
| unattributed remainder | ~215 | 2.7 % |

### Level 4 — inside `side_facts`

| phase | ms | % of span |
|---|---:|---:|
| the per-member EDGE loop | 1,773.4 | **22.4 %** |
| the final per-member ASSEMBLE loop | 1,724.1 | **21.8 %** |
| the uncertainty-edge loop | 21.8 | 0.3 % |
| the base-summary read | 5.0 | 0.1 % |
| unattributed remainder | ~152 | 1.9 % |

## What the cost actually is

Populations from the same run:

| counter | 8020 | DO |
|---|---:|---:|
| edges scanned in the edge loop | 150,211 | 7,296 |
| `shared.insert(uncertainty_key(u), u.clone())` calls | **4,397,866** | 13,962 |
| `opaque-callee` uncertainties pushed | 1,821 | 20 |
| elements cloned by `shared_vec.clone()` | **3,687,409** | 12,858 |
| elements passed to `dedupe_uncertainties` | **3,708,222** | 13,965 |

**The edge loop's cost is not the edges.** 1,773.4 ms over 150,211 edges is
11.8 µs per edge, which no edge scan costs. The loop does 4,397,866 `shared`
inserts — **29.3 per edge** — because for every external edge it re-folds the
settled callee's ENTIRE `uncertainties` vector into the SCC-shared map, and each
insert builds a fresh `String` key (`uncertainty_key`) and deep-clones an
`Uncertainty` (five `Option<String>` fields). Callee uncertainty vectors are
themselves accumulated unions, so the re-fold grows along call chains.

**The assemble loop is the same data a second time.** `shared_vec.clone()` per
member copies 3,687,409 `Uncertainty` records, and `dedupe_uncertainties` then
sorts and collapses 3,708,222 of them.

Together the two loops are **~8.1 M `Uncertainty` deep clones and ~4.4 M key
`String` builds per run**, inside a span of ~8 s. For scale, the output they feed
(`ctx.uncertainties_by_node`, measured by `ALSEM_UNCERTAINTY_CENSUS`) is 27,037
nodes over **19,311 distinct values / 10,112 distinct sets** — the same
over-materialization shape the transaction-spans arc found (261.8 M strings for a
1,061-span answer).

## Two things this census FALSIFIED before anything was built

1. **The `settled.clone()` in `solve_scc_db_effects`'s multi-sibling general path
   never runs.** `multi_eff_sccs=0` on 8020 AND on DO — every Tarjan SCC
   decomposes into exactly one effective SCC, so the workspace-sized
   `HashMap<String, RoutineSummary>` clone this code is written to survive is
   dead on both corpora. `local_settled=0.0ms`. It is a correctness path with no
   measured population; do not size work against it.
2. **The roles JACOBI fixpoint is not the cost on 8020** — 777.8 ms, 9.8 %. It is
   the one remaining fixpoint in the file and reads like the expensive thing; it
   is not, at this scale.

## DO is a different shape — this is a Base-App item

DO (4,842 routines): whole span is **130.0 ms**, and inside it `roles` (54.8 ms,
42.2 %) slightly EXCEEDS `db_solver` (52.2 ms, 40.1 %), with `side_facts` at
16.4 ms / 12.6 %. The 8020 ranking (side_facts 46.5 %, roles 9.8 %) does not hold
on a customer-sized workspace. Any fix here is a survive-Base-App item, exactly
like B2 — its DO effect will round to nothing, and its DO effect must not be
negative.

## Artifacts

All under `logs/` on branch `perf/summaries-census`:
`err-8020-census.log` (level 1), `err-8020-census2.log` (level 2),
`err-8020-census3.log` (levels 3–4), `do-err-on.log` (DO),
`trace-8020-off.json` (census-off control), `trace-8020-census{,2,3}.json`.
