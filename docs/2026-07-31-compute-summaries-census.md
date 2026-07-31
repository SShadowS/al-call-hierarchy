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


---

# Part 2 — the fix (branch `perf/side-facts`)

Built against the attribution above, not against a guess. Four changes, all inside
the two loops the census named, none of which changes an output byte.

## Probe shape for this part

`analyze.total` and even a single span's absolute ms swing far too much on this
machine to carry a claim (the baseline binary measured `phases_total` at 10,781.6 /
8,877.8 / 8,012.8 ms across three consecutive runs of the SAME binary). So the
claim here is a **paired A/B**: two binaries built from the same tree, run
**alternately** in one session, three runs each, medians compared — plus
`roles`, a phase NO change in this branch touches, reported as the control.

## What changed

1. **`fold_shared` — the SCC-shared fold stops allocating a key and stops cloning
   on a repeat.** `shared.insert(uncertainty_key(u), u.clone())` ran 4,397,866
   times per 8020 run: 4.4 M `format!` allocations plus 4.4 M five-`Option<String>`
   deep clones, for a map holding only the distinct keys. The key is now formed
   into a REUSED buffer and hashed as a `&str` (allocating only for a genuinely
   new key), and a repeat COMPARES the stored record instead of overwriting it.
   Last-write-wins survives by construction: equal ⇒ the overwrite was a no-op;
   different ⇒ it still happens. That is not a corpus-conditional argument.
2. **`dedupe_uncertainties` — sort instead of `BTreeMap<String, _>`.** It was
   building one `format!("{}|{}")` key per element for 3,708,222 elements per run.
   Now a stable sort on [`cmp_uncertainty_key`] plus a keep-LAST pass. The sort is
   on the CONCATENATED `"kind|at"` byte sequence — a `(kind, at)` tuple sort is a
   DIFFERENT order, because `'|'` (0x7C) outranks most identifier bytes — and
   `sort_by`'s stability plus keep-last reproduces `BTreeMap::insert`'s overwrite
   exactly.
3. **The last member of each effective SCC takes `shared_vec` by MOVE.** 100,010
   of 100,922 members are the sole member of their effective SCC, so this alone
   turns 3,687,409 cloned `Uncertainty` records into 507,045 (`shared_moved_elems`
   3,180,364 — the two counters sum to the old population exactly, so the saving
   reads as movement between counters rather than as a vanished number).
4. **Two `get(..).cloned()` sites became `remove(..)`** — `solve_one_effective_scc`'s
   `out` assembly and `compute_summaries_v2_bundle_with_leaves`' member assembly.
   Each was deep-cloning all 3,700,433 output records and dropping the original
   immediately. Both maps are locally owned and dead afterwards, and both key sets
   are distinct by construction (Tarjan members / a `HashMap`'s keys).

## MEASURED — paired A/B, 3 alternating runs each, medians

| phase | base | fix | Δ |
|---|---:|---:|---:|
| `phases_total` (the whole span) | 8,877.8 ms | **6,018.5 ms** | **−32.2 %** |
| `db_solver` | 6,448.5 ms | 3,965.2 ms | −38.5 % |
| `side_facts` | 4,230.2 ms | 2,519.9 ms | −40.4 % |
| ├ its edge loop | 2,017.9 ms | 1,534.9 ms | −23.9 % |
| └ its assemble loop | 2,003.3 ms | 932.5 ms | −53.5 % |
| `db_solver`'s `out_assemble` | 458.1 ms | 17.5 ms | **−96.2 %** |
| the SCC loop's member `assemble` | 429.0 ms | 57.5 ms | −86.6 % |
| **`roles` (control — untouched)** | 858.4 ms | 824.9 ms | −3.9 % |

The control moving −3.9 % is the noise floor these deltas stand above.
Normalizing every run to its own `roles` (which removes the between-run machine
factor entirely) gives the same answer: `side_facts` −38.5 %, `phases_total`
−30.6 %. The fix binary's spread is also much tighter (5,917–6,073 ms vs
8,013–10,782 ms), consistent with the removed allocator pressure.

Allocation counts, which carry no noise at all:

| counter | base | fix |
|---|---:|---:|
| `format!` key allocations in the shared fold | 4,397,866 | ≈ distinct keys only |
| `format!` key allocations in `dedupe_uncertainties` | 3,708,222 | 0 |
| `Uncertainty` records cloned by `shared_vec.clone()` | 3,687,409 | 507,045 |
| `Uncertainty` records cloned by the two `get().cloned()` sites | 2 × 3,700,433 | 0 |

## The change that was a LOSS before it was a win

Change 2 was measured as a **regression** in its first form and kept only after
being fixed. `cmp_uncertainty_key` originally compared two chained byte iterators;
in that form the span's assemble share moved the WRONG way (21.8 % → 24.6 %) —
the byte-at-a-time comparison cost more than the 3.7 M allocations it removed.
Rewriting it to a `memcmp`-backed slice compare with a single boundary byte turned
it around. An isolating A/B (same tree, only `dedupe_uncertainties` swapped, 3
alternating runs each) prices change 2 on its own:

| phase | `BTreeMap` dedupe | sort dedupe | Δ |
|---|---:|---:|---:|
| `side_facts`' assemble loop | 1,351.9 ms | 893.3 ms | **−33.9 %** |
| `side_facts` | 2,882.4 ms | 2,444.2 ms | −15.2 % |
| `roles` (control) | 790.2 ms | 793.5 ms | +0.4 % |
| its edge loop (control) | 1,469.7 ms | 1,490.6 ms | +1.4 % |

Two controls at +0.4 % / +1.4 % against a −33.9 % target: the attribution is the
change, not the machine. **The lesson is that the allocation COUNT was not
sufficient evidence on its own** — removing 3.7 M allocations was a net loss until
the thing replacing them was also fast.

## DO does not regress

DO's whole span went 130.0 ms → 96.7 ms, and `shared_clone_elems` reached **0**
(every DO effective SCC is a singleton, so every member takes the move path). This
is the check the uncertainty-substrate arc failed — that substrate cost DO
+0.4 MiB. This one does not cost DO anything.

## Gates

Byte identity, both corpora, `--deterministic`, with the FINAL binary:

```
f022f677d2650b2399fc3aa5a7625bc6c078d90dd51cdb80e1e3705808fee3ea  DO
36151bf67e17620724abb6b2cdbad55bcf8f97ffe3c3237782a0cf4c25ecc5fb  8020
```

8020 additionally reproduced that hash on all 12 A/B runs (both binaries, both
variants). `scripts/check-goldens` green (9/9 targets), zero files under `tests/`
moved. `cargo clippy --all-targets --all-features` clean.

Four new tests pin `dedupe_uncertainties`' two contracts by HAND-STATING their
preconditions (two records built to share a key while differing in
`interface_name` — the field the key drops — rather than asking production code to
produce a collision), and **both were proven to discriminate, both directions**:
swapping in the tuple comparator fails `sort_is_on_the_concatenated_key_not_the_field_tuple`
with `["a|b", "ab|c"]`, and deleting the `mem::swap` from the keep-last pass fails
`same_key_keeps_the_last_record_not_the_first` with `Some("iface-first")` and
`same_key_run_of_three_keeps_the_last` with `Some("a")`. Restoring each passes.

## A scripted edit silently moved an attribute — clippy caught it, not the tests

Inserting `fold_shared` at the anchor `pub fn solve_side_facts(` put it between
that function's `#[allow(clippy::too_many_arguments)]` and the function itself,
so the attribute silently re-attached to the NEW function and `solve_side_facts`'
own 60-line doc block did too. Every test stayed green, both gate hashes stayed
exact, and `check-goldens` passed — the only signal was one clippy warning
pointing at a function whose signature the diff had not touched. Recorded because
the repo's scripted-edit rule is about match COUNTS, and this edit's counts were
all correct: the hazard was the anchor's POSITION, not its multiplicity. Anchor
above the doc block, or assert on what sits immediately before the anchor.

## What is left in this span

`side_facts` is still 2,519.9 ms — the largest item in the span. Its edge loop's
remaining cost is the 4,397,866 folds themselves, which no longer allocate but
still hash a key and walk a settled callee's whole `uncertainties` vector per
external edge. Eliminating THAT needs the callee's propagatable set carried as an
interned id set rather than re-folded per edge — the same move
`ctx.uncertainties`/`UncertaintyIndex` already made one layer up. Not built here;
sizing it needs its own census round.
