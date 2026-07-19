# Wave 2 measurements — SCC anatomy, Jacobi telemetry, detector-loop profile (2026-07-18)

Status: **ALL MEASUREMENTS COMPLETE, INCLUDING WAVE-2a.** M1 done (8020 +
5400 SCC anatomy comparison; trigger-edge over-approximation hypothesis
VERIFIED by source-sampling), M1b done (58-pass 8020 Jacobi telemetry), 5400
detector-loop profile done (§3a), dominator-detector root cause VERIFIED
(§4a), M2 done (§4 — the pre-fix full-default 8020 run does not finish
inside a 2.5h cap; d1 alone consumes it), Wave-2a fixes landed and
re-measured (§6 — T1/T2 give d1 2.8× at 5400 but full-default 8020 is
STILL DNF; walk-graph SIZE, not per-step cost, is now the limiter). Next:
Wave-2b — trigger-edge builder parity (licensed, §2/§6), then re-measure.

## 1. Purpose

Wave 2 (B1 interned id universe, B2 SCC-shared lazy cones — findings doc §7)
is compat-breaking internally (output-stable, but the memory model changes
end to end). Project doctrine is measure-first: "measure the population
before building taxonomy for it" bit the call-graph roadmap twice (CLAUDE.md,
Argtype-dispatch-and-page-catalog / Pageext-merge-and-final-residual arcs) and
the same discipline governs this arc. Before writing a Wave-2 plan, this
document collects the three measurements §8 of
`docs/superpowers/specs/2026-07-17-engine-memory-speed-findings.md` named as
next-session follow-ups, plus a fourth measurement the Wave-1 outcome (§7b)
surfaced: with the substrate-construction and Jacobi regime-change terms
fixed, the full-default 8020 run still does not finish (killed at a 60-min
cap) because the bottleneck moved into the 54-detector L5 loop itself. Wave-2
scope should be set by what these four measurements show, not by the ranked
proposal's a-priori ordering.

Corpus and build match the findings doc throughout: Base App
28.0.46665.48632 source (8,020 `.al` files, one app, zero deps),
`release-fast` build, same machine.

## 2. M1: SCC anatomy (DONE)

**Question:** the findings doc's H-1 correction (§6 item 14, gpt-5.6-sol)
flagged that the 846-member largest SCC's internal density was unmeasured —
a single over-approximated event/interface/trigger edge can fuse otherwise
separate components, and SCC size is the direct multiplier behind H-5's
O(routines × cone) memory term. This measurement answers: how dense is the
846-member SCC internally, and which edge kind dominates its intra-SCC
edges?

**Instrumentation:** `src/engine/l5/detector_context.rs:453-482`, gated on
`ALSEM_STAGE_TIMING=1` and firing once per `build_detector_context` call when
`CORE_SUMMARIES` is demanded (the same demand-driven substrate gate W1.0
added). It finds the largest SCC by member count, then walks each member's
outgoing edges (`graph.edges_by_from`), counting only edges whose target is
also a member of that same SCC, bucketed by `edge.kind`.

**Corpus totals** (`SCCSTATS`, `detector_context.rs:445-452`): 100,941
routine nodes, 82,613 SCCs, 342 of them recursive, largest SCC = 846
members.

**Largest-SCC intra-edge anatomy** (`SCCANATOMY`):

| Edge kind | Intra-SCC edges | Share |
|---|---:|---:|
| direct | 1,134 | 43.4% |
| implicit-trigger | 1,077 | 41.3% |
| method | 298 | 11.4% |
| event-dispatch | 45 | 1.7% |
| interface | 35 | 1.3% |
| codeunit-run | 21 | 0.8% |
| **Total** | **2,610** | 100% |

`max_intra_outdegree = 143` — one member has 143 intra-SCC outgoing edges,
against an 846-member component where the theoretical max is 845.

**5400-slice comparison:** the same probe, run on the 5400-file slice (the
mid-size corpus used throughout the findings doc's per-stage attribution,
§2/§7b; run log: `.superpowers/sdd/w2-jacobi-5400-analysis.md`), found a much
smaller largest SCC. `SCCSTATS` on this slice: 60,550 routine nodes, 50,178
SCCs, 236 recursive (828 recursive members, mean recursive-SCC size ≈3.5),
largest = 84 members — against the 8020 totals above (100,941 / 82,613 / 342
/ 846). `SCCANATOMY`: 157 intra-edges, max_intra_outdegree=20.

| Corpus | Largest SCC | Intra-edges | Edges/member | direct | implicit-trigger | method | event-dispatch | interface | codeunit-run | max_intra_outdegree |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 5400 | 84 | 157 | 1.87 | 108 (68.8%) | 28 (17.8%) | 16 (10.2%) | 2 (1.3%) | 2 (1.3%) | 1 (0.6%) | 20 |
| 8020 | 846 | 2,610 | 3.09 | 1,134 (43.4%) | 1,077 (41.3%) | 298 (11.4%) | 45 (1.7%) | 35 (1.3%) | 21 (0.8%) | 143 |

**Observation (hypothesis-strengthening, not proof):** at 5400, the biggest
SCC is a direct-call cluster, not a trigger-dominated one — 108 of its 157
intra-edges (69%) are ordinary `direct` calls, `implicit-trigger` a distant
second at 28 (17.8%), and `event-dispatch` only 2 edges (not primarily an
event-driven cycle either). The trigger-edge dominance seen at 8020 —
41.3%, nearly matching `direct`'s 43.4% — does not appear at moderate corpus
size. The implicit-trigger SHARE of intra-SCC edges nearly doubles between
the two corpora (17.8% → 41.3%) while `direct` calls' share falls (68.8% →
43.4%) over the same span: trigger edges are a minority contributor to the
SCC at moderate corpus size, and come to dominate the fusion budget only at
full corpus density. That is consistent with the hypothesis that
implicit-trigger over-approximation is specifically what BRIDGES
previously-separate components into the 846-member SCC, rather than the SCC
simply growing because direct-call density grows proportionally with corpus
size — sharpened by this comparison, since a proportional-growth story would
predict `direct`'s share holding roughly steady, not falling by half. It
remains evidence, not proof: the verification step is still sampling
implicit-trigger intra-SCC edges against real source (below).

**Interpretation (hypothesis, not conclusion):** the component is SPARSE —
2,610 intra-edges over 846 members is ~3.1 edges/member, nowhere near the
~845/member a dense component would show. Density alone does not explain why
it is ONE component rather than many smaller cyclic ones; a handful of
bridging edges is enough to fuse arbitrarily many otherwise-separate cycles
into a single SCC. `implicit-trigger` edges — the data-is-control-flow
trigger edges from the call-graph resolver — are 41.3% of the intra-SCC
total, second only to `direct` calls and close behind it. That makes
trigger-edge over-approximation the leading suspect for WHY the component
fuses: if an implicit-trigger edge is wired more broadly than the real
runtime trigger relationship (e.g. an OnValidate treated as reachable from
every write site rather than the specific field/table path that actually
fires it), a small number of such edges could be bridging clusters that
would otherwise resolve as separate, much smaller SCCs — directly shrinking
the Jacobi problem (H-5, §2 "regime change") at its root, which would be
cheaper than any algorithmic fix to the Jacobi loop itself. The
`max_intra_outdegree=143` hub is also unexplained — worth checking whether
it is a legitimate shared routine (e.g. a common logging/error path called
from everywhere) or itself a trigger-edge fan-out artifact.

This is a hypothesis to VERIFY, not a finding to act on: the next step is
sampling some of the 1,077 implicit-trigger intra-SCC edges against real
Base App source and checking whether each is a genuine possible trigger
firing or an over-approximation. No such sampling has been done yet.

**Update — VERIFIED (Wave-2a Task 4).** Source:
`.superpowers/sdd/w2-trigger-edge-verdict.md`. The hypothesis is CONFIRMED:
the 846-SCC's implicit-trigger intra-edges are over-approximated, and the
over-approximation is exactly the shape hypothesized above — an OnValidate
treated as reachable from every validate site rather than the specific
field path that actually fires it.

The advisory builder (`src/engine/l3/implicit_edges.rs:29-70`,
`build_implicit_trigger_edges`) omits two preconditions the fresh resolver's
already-correct predicate (`src/program/resolve/applicability.rs:159-227`,
`implicit_trigger_route_applicable`) already gets right:
1. **RunTrigger gate** (`applicability.rs:166-168`): every mutating op
   defaults `RunTrigger := false` in AL, so `Insert()`/`Modify()`/`Delete()`/
   explicit `(false)` fire no trigger at runtime — the advisory builder
   emits an edge regardless. The data is already captured
   (`L3RecordOperation.run_trigger`, `l3_workspace.rs:270-274`).
2. **Field-specific OnValidate targeting** (`applicability.rs:184-195`): the
   target's `enclosing_member_lc` must equal the validated field. The
   advisory builder never consults it — worse, `routine_in_object`'s
   name-only lookup (`symbol_table.rs:270-273`) keys ALL of a table's
   per-field `OnValidate` triggers on the same `"{object}::OnValidate"`
   string, so every one of a table's N field triggers collides on that key
   and only one survives (last-wins). Every `Validate(anyField)` on table T
   resolves to a single arbitrary collision-winner node.

**Sample (whole-population distribution, not a sub-sample):** of the 1,077
intra-SCC implicit-trigger edges, 1,046 (97.1%) target OnValidate, 17 (1.6%)
OnInsert, 14 (1.3%) OnDelete. 20 sampled `(from → to)` pairs were checked
against real Base App source directly — **all 20 confirmed
over-approximated by target-collapse**: e.g. `Sales Header::
SetShipToCustomerAddressFieldsFromShipToAddr` fires 7 *distinct*-field
`Validate(...)` calls (Ship-to Country/Region, Location, Salesperson,
Shipping Agent, Shipping Agent Service, Tax Area, Tax Liable) — 7 different
real field triggers — all collapsed onto the same one `Sales Header::
OnValidate` graph node. Structurally confirmed: Sales Header declares 93
field `OnValidate` triggers, Item Journal Line 43, all collapsing onto one
node each — directly explaining `max_intra_outdegree=143` as a collapsed
OnValidate hub, and `Item Journal Line::OnValidate → itself` self-loops as
real per-field cross-validations collapsing onto their own hub.

**Aggregate mechanism split:** field-collapse (missing precondition 2)
accounts for 1,046/1,077 = 97.1% of the intra-SCC trigger edges — the
SCC-fusion driver; the RunTrigger gate (missing precondition 1) accounts for
the remaining 31 (2.9%, pure phantom edges on Insert/Delete). The fix is
bringing `build_implicit_trigger_edges` to parity with the fresh resolver's
existing predicate, using data the advisory model already captures.

**Estimated effect (not yet measured — re-running Tarjan post-fix is
Wave-2b work):** field-specific targeting shatters each per-table collapsed
hub into its N distinct field-trigger nodes; edges mostly survive (a
validate still fires ITS OWN field's trigger) but become inter-SCC rather
than intra-SCC, since the false cross-field reachability is what bridges
otherwise-separate direct-call clusters at 8020 density. Estimate: the SCC
should collapse toward the ~84-member scale seen on the 5400 slice (§2's
5400-slice comparison above), where implicit-trigger was already only
17.8% of a much smaller component. Residual real cross-validation cycles
(e.g. Item Journal Line's fields validating each other) would remain, but
as small per-table SCCs that don't bridge unrelated codeunits/tables — not
the giant fused component. This is the trigger-edge builder parity fix
named first in §6's Wave-2b queue below.

## 3. M1b: Jacobi frontier telemetry

Status: **DONE** — not reached at 5400 (as expected, see below); measured on
the 8020 corpus, source `w2-8020-default.txt.stderr` (instrumented
full-default run, `ALSEM_STAGE_TIMING=1`; see §4 for the run's full context —
this section covers only the JACOBI lines it produced).

**Question** (findings doc §8 follow-up 2): on the big SCC, how many Jacobi
passes actually run, how many members change per pass, and how do summary
cardinalities move across passes? This sizes W1.1's round-preserving
dirty-frontier win precisely — the current per-compose global scan and
deep-clone-every-pass cost (`l4/summary_runner.rs:483-486`, `:1067`) trend
one way whether the fixed point takes 3 passes or 30, and whether the number
of changed members per pass decays fast (frontier win is large) or stays
broad (frontier win is small).

**Instrumentation:** `src/engine/l4/summary_runner.rs:1199-1215`, gated on
`ALSEM_STAGE_TIMING=1`, firing per pass for every SCC with ≥100 members.
Emits one `JACOBI` line per pass per qualifying SCC:
`scc_members`, `pass`, `dirty_in`, `dirty_out`, `changed`,
`sum_cardinalities` (summed `db_effects.len() + uncertainties.len() +
parameter_roles.len()` over all in-progress summaries that pass).

**5400 result: threshold not reached, as expected.** The w2-jacobi-5400
agent's instrumented run (`.superpowers/sdd/w2-jacobi-5400-analysis.md`)
produced zero `JACOBI` lines. This is correct behavior, not a gap: the
probe's `members.len() >= 100` gate (`summary_runner.rs:1200-1202`) only
fires for recursive SCCs at or above 100 members, and 5400's largest SCC is
84 members (§2 above) — below the gate. The instrumentation itself is
confirmed working (the gate suppressed emission exactly where it should
have); it simply has nothing to measure on this corpus. The 846-member SCC
that motivates this measurement (§8 follow-up 2 of the findings doc) only
exists at 8020 — the per-pass frontier telemetry below comes from the
instrumented 8020 run.

**8020 result: 58 passes to fixed point, frontier decays cleanly to zero.**
The 846-member SCC produced exactly 58 `JACOBI` lines (`SCCSTATS`/
`SCCANATOMY` immediately precede them in the log, byte-identical to §2's
8020 numbers — nodes=100941, sccs=82613, recursive_sccs=342, max_scc=846,
confirming this is the same SCC), pass 58 the first with `changed=false`
and `dirty_out=0` — a genuine fixed point, not a cap-hit (`MAX_FIXED_POINT_
ITERATIONS` is 1000, `summary_runner.rs:34`; nowhere close). Representative
rows (full 58-row log in `w2-8020-default.txt.stderr`):

| Pass | dirty_in | dirty_out | sum_cardinalities |
|---:|---:|---:|---:|
| 1 | 846 | 808 | 90,378 |
| 5 | 781 | 810 | 1,064,220 |
| 10 | 825 | 837 | 2,311,312 |
| 15 | 813 | 825 | 4,010,320 |
| 20 | 826 | 795 | 5,859,053 |
| 25 | 705 | 585 | 6,777,835 |
| 30 | 403 | 316 | 7,023,076 |
| 35 | 236 | 228 | 7,250,266 |
| 40 | 215 | 214 | 7,649,647 |
| 45 | 209 | 188 | 8,186,563 |
| 50 | 154 | 125 | 8,334,373 |
| 55 | 19 | 15 | 8,344,400 |
| 58 | 5 | 0 (fixed point) | 8,344,476 |

**Decay shape (not the smooth exponential the "dirty-frontier win" framing
implicitly hoped for): a long plateau, then a decline, then a long thin
tail.** Passes 1-20 stay in a 750-846 band — the frontier barely shrinks at
all for the first third of the run (846→808→…→795, never dropping below
~750). Real decay only starts around pass 21 (780→316 by pass 30, roughly
halving every ~3 passes) and then the tail from pass 31-58 decays slowly
from 290 down to 0 over 28 more passes. Summed `dirty_in` across all 58
passes = 27,333 member-recomputations, against 58 × 846 = 49,068 if every
pass naively recomputed the full SCC — the dirty-frontier mechanism (W1.1,
already landed) is already saving ~44% of that naive cost, but the saving is
backloaded: it does almost nothing for the first ~20 passes (where the
frontier is still ~90-100% of the SCC) and only pays off in the back half.
This directly sizes the REMAINING dirty-frontier headroom (there isn't much
left to win algorithmically in the plateau) versus the per-member summary
SIZE, which is the term still growing every pass regardless of frontier
width — see below.

**`sum_cardinalities` grows monotonically, never oscillates** — confirmed by
reading every one of the 58 values directly: each pass's total is strictly
greater than the last, ending at 8,344,476 total fact/effect entries across
the 846 in-progress summaries (≈9,863 entries/member on average, though the
distribution across 846 members is unmeasured). This answers the open
question this section originally posed: no oscillation means summary
content is converging cleanly (never shrinking then growing again), which
is consistent with — though not by itself a full proof of — the
fingerprint-equivalence property W1.1's non-allocating change-test swap
needs. The more load-bearing implication is for B1/B2 (findings doc §7):
8.3M live entries at fixed point, on ONE SCC, is direct evidence for H-5's
"O(routines × cone) memory term" and B2's shared-fact-content redesign — the
dirty-frontier mechanism controls how many members get RECOMPUTED per pass,
but not how large each member's summary IS, and that size is what B1
(interning) and B2 (shared cone content) target.

## 3a. Detector-loop profile at 5400 (contention-inflated absolutes, robust ranking)

Status: DONE (5400). Source: `.superpowers/sdd/w2-jacobi-5400-analysis.md`,
same instrumented run as §3 above (`ALSEM_STAGE_TIMING=1`, full-default
detector set, 5,400-file slice).

**Contention caveat — read this before the numbers below.** A second,
unrelated `alsem.exe` measurement ran concurrently on the same machine for
the entire duration of this run (confirmed via `Get-Process`). Total wall
time was 2,607.69 s (~43.5 min), far above the ~4-6 min this corpus normally
takes uncontended. **The absolute per-detector seconds below are
contention-inflated by roughly 4-6x and must not be read as an
isolated-machine baseline.** What IS robust to shared-CPU noise is the
*ranking* — which detectors dominate, and by how many multiples — since
contention scales all detectors' wall time by roughly the same multiplier.
The ranking, not the absolutes, is the load-bearing result of this
measurement.

**Loop total vs. substrate total:**

| Segment | Seconds | % of total wall |
|---|---:|---:|
| Substrate (parse → resolve → L4 setup, pre-detector) | 118.52 | 4.5% |
| Detector loop (`l4_detector_context:end` → `run_detectors:end`) | 2,446.92 | 93.9% |
| Report formatting (`run_detectors:end` → `format:end`) | 42.25 | 1.6% |
| **Total wall** | **2,607.69** | 100% |

The detector loop is where wall time goes on this run — nearly 21x the
substrate cost. Under normal (uncontended) conditions the ratio should be
directionally similar, since both segments share roughly the same
contention multiplier.

**Top 5 slowest detectors** (of 44 that ran; full 44-row table in the source
analysis doc):

| Rank | Detector | Δ (s) |
|---:|---|---:|
| 1 | `d19-unused-parameter` | 988.19 |
| 2 | `d1-db-op-in-loop` | 448.35 |
| 3 | `d12-dead-integration-event` | 424.83 |
| 4 | `d3-missing-setloadfields` | 109.43 |
| 5 | `d54-publish-in-tryfunction-cone` | 107.85 |

**Ranking interpretation (the contention-robust part):** the top 3 —
`d19-unused-parameter`, `d1-db-op-in-loop`, `d12-dead-integration-event` —
together account for 1,861.4 s of the 2,420.9 s summed detector time
(**76.9%**), and dominate the next 12 detectors combined (~430 s) by
**1-2 orders of magnitude**. That dominance pattern is unlikely to be a
contention artifact: a detector running 2 orders of magnitude above the
median is not going to fall out of the top 3 on a clean machine. These three
detectors do categorically more work per file than the rest of the registry
— unused-parameter needs whole-program call-site scanning per parameter;
db-op-in-loop and dead-integration-event both walk substantial
evidence/cone structures — not just running slightly slower.

**Wave-1 Jacobi fixes are holding at 5400, even under contention.** The
`ACCUM` lines for this run:

```
ACCUM jacobi_snapshot_clone total_s=3.31
ACCUM jacobi_compose total_s=22.83
ACCUM jacobi_project_fingerprint total_s=14.31
```

against the findings doc's pre-Wave-1, uncontended slice-5400 baseline
(§2 "Inside the Jacobi block", `2026-07-17-engine-memory-speed-findings.md`):
snapshot clone 5.9 s, compose 31.8 s, fingerprint 49.3 s. Even measured under
heavy shared-CPU contention, all three fell — most strikingly the
JSON-fingerprint change test (W1.1's target), which collapsed from 49.3 s to
14.31 s, a >3x reduction despite the run itself being ~4-6x slower overall
on wall clock. This is consistent with W1.1's non-allocating-comparison and
`mem::take` fixes doing real work at the Jacobi-block level, independent of
machine load.

## 4. M2: full-default 8020 detector-loop profile

Status: **DONE — the run did not finish; that is itself the answer.** Source:
`w2-8020-default.txt.stderr` (instrumented full-default run,
`ALSEM_STAGE_TIMING=1`, 8,020-file corpus). **Contention caveat:** this run
shared the machine with the 5400 run (§3a) for roughly its first 40 minutes
— the substrate-phase seconds below carry the same caveat as §3a's absolutes,
though the substrate phase here (finishing at t=1973.83s, well past the
5400 run's ~43.5 min total wall time) was likely contended for only its
early portion, not throughout.

**Question:** §7b's Wave-1 outcome measured that the full-default-detector
8020 run is no longer dying inside the substrate (RSS trajectory clears
`compute_summaries` around the 12-minute mark, peaking 45.2 GB), but still
does not finish inside a 60-minute cap — the remaining 40+ minutes were
spent inside the 54-detector L5 loop, tentatively attributed to the witness
path-walker plus detector working sets never exercised at this corpus size
before. This run raised the cap to 2.5h and added per-detector stage marks
to identify the cause directly.

**Headline result: killed at the 2.5h cap (elapsed 9000s), peak RSS 49.1 GB,
ZERO of ~55 detectors completed.** `d1-db-op-in-loop` ran ALONE for the
entire detector-loop window — roughly 117 minutes — and never finished. This
is not an inference from timing alone: the STAGE probe only ever prints
`STAGE detector:<name>:end` AFTER a detector completes (confirmed against
§3a's 5400 log, which shows 44 such lines for a run that finished) — and
`w2-8020-default.txt.stderr` contains ZERO `detector:*:end` lines after
`l4_detector_context:end`. No detector after d1 in registry order ever got
a turn; the loop never reached a second detector.

**Substrate timeline (from the raw STAGE lines, verified directly):**

| Stage mark | t (s) | Note |
|---|---:|---|
| `fresh_coverage:begin` | 0.00 | — |
| `fresh_coverage:end` | 5.00 | fresh preflight, drops to 40 MB |
| `l3_assemble_resolve:end` | 12.06 | — |
| `l4:combined_graph:end` | 15.93 | — |
| `l4:cones:end` | 66.73 | RSS 15,894 MB |
| `l4:transaction_spans:end` | 174.65 | spans done — RSS 15,420 MB |
| `l4:compute_summaries:end` | 1,527.45 | **RSS 41,023 MB (peak 41,376 MB)** |
| `l4_detector_context:end` | 1,973.83 | RSS 16,877 MB (peak still 41,536) |

`compute_summaries` (the block containing the 58-pass Jacobi loop above)
ran t=174.65→1,527.45s — **~22.5 minutes, and it FINISHED.** Contrast with
the pre-Wave-1 8020 run (findings doc §2/§7b): that run entered the same
block and was still inside it, non-terminating, when the 90-minute
instrumented cap killed it (>83 min in one stage with RSS flat at 35.7 GB).
Wave 1's Jacobi fixes (W1.1) hold at full 8020 scale, not just at 5400 —
the substrate-construction and Jacobi regime-change terms this document's
§1 named as "fixed" are confirmed fixed at the corpus that used to die
inside them.

**Per-detector top-N table: unobtainable, and unnecessary.** Unobtainable
because zero detectors completed — there is no second wall-time data point
to rank against d1. Unnecessary because §4a already identifies WHY d1 alone
can consume the entire loop without finishing: the per-edge cone
re-allocation inside `expand`→`touches_db_of`→`reachable()`
(`d1.rs:614-632`, `capability_query.rs:133-138`, `full_summary.rs:46-56`)
running inside a 500-node bounded DFS per in-loop call site
(`path_walker.rs:148-249`) scales with cone size `K`, which scales with
graph density — and this run proves that mechanism doesn't just make d1
slow, it can make d1 NOT TERMINATE within a 2.5-hour budget on the full
corpus. That is the same exploding term §4a named, now shown to be severe
enough to dominate a budget an order of magnitude larger than §3a's 5400
absolutes would have predicted by simple extrapolation (448s at 5400 →
117+ min at 8020 is a >15x jump against a ~1.5-1.9x file-count ratio).

**Detector-loop total vs. substrate total:** substrate = 1,973.83s (~32.9
min) to `l4_detector_context:end`; detector loop = at least 9000−1973.83 ≈
7,026s (~117 min) of d1 alone, with no upper bound observable (the loop
never converged — it was killed, not finished). The ratio §7b estimated
from the pre-M2 killed run (~12 min substrate / 40+ min detector) undercounts
both sides: substrate is closer to 33 min at true 8020 scale (cones alone
take 51s here, versus §7b's whole-substrate 12-min estimate — the discrepancy
is likely the earlier estimate being read off a run killed before reaching
this stage cleanly), and the detector-loop side is not "40+ min," it is
"unbounded on this corpus with today's algorithm."

**Peak RSS attribution: split between substrate carryover and detector
working set, and NOW MEASURABLE at each transition.** The `compute_summaries`
phase itself peaks at 41,376 MB — matching §2/H-5's cones+summaries
resident-memory story. Handing off to the detector context DROPS RSS to
16,877 MB (41.4 GB → 16.9 GB, a ~24.5 GB reduction — cones/intermediate
Jacobi state being freed once `FullRoutineSummary`s are finalized). The run
then climbs to a NEW peak of 49.1 GB during d1's 117-minute run — HIGHER
than the substrate's own 41.4 GB peak, and reached entirely inside d1's
per-edge `reachable()` allocations (the detector loop's own working set,
not leftover substrate materialization). This confirms Wave 2's two targets
are BOTH real and largely independent: B1/B2 own the substrate's 41.4 GB
peak (H-5's O(routines × cone) term, directly evidenced by §3's 8.3M live
entries on one SCC); §4a's cone-probe fix owns the NEW 49.1 GB peak that
forms entirely inside the detector loop, on top of whatever substrate
memory Wave 2 eventually reclaims.

**Whether the run finishes within a 2.5-hour cap: NO.** Killed at 9000s
elapsed with zero detectors complete. The cap was deliberately raised from
§7b's 60 minutes specifically to observe whether the loop reaches a fixed
point at all on this corpus — it does not, at least not for `d1` under
today's implementation. This is the strongest available evidence for §4a's
"jump the Wave-2 queue ahead of B1/B2" recommendation: the cone-probe fix is
not an optimization for a slow-but-finishing case, it is required for the
full-default 8020 run to finish at all.

**Cross-reference:** the fixes §4a licenses (fingerprint hoist + structural
id extraction; cone-probe precomputation) are implemented in
`docs/superpowers/plans/2026-07-18-engine-memory-speed-wave2a.md`
(committed) — that plan's T3 re-measures this exact run after both land.

## 4a. Dominator-detector root cause (VERIFIED)

Status: DONE. Source: `.superpowers/sdd/w2-detector-anatomy.md` (full
derivation, candidate-population and per-candidate-work analysis for all
three dominator detectors). This section folds its conclusions in. Its
central claims were independently re-verified against current source below
— every citation was re-read directly by this document's author, not taken
on the source doc's word — because the fix-shape recommendation that follows
depends on it being right.

**Question this answers:** §3a found `d19-unused-parameter` (988s),
`d1-db-op-in-loop` (448s), and `d12-dead-integration-event` (425s) dominate
the 5400 detector loop by 1-2 orders of magnitude, and §4's motivating
context is that `d1` alone ran 77+ minutes at 8020. Why these three,
specifically, and is the cause mechanical (fixable directly) or diffuse
(evidence for the deferred §7 flow-insensitive witness-walker redesign)?

**The shared killer — `FingerprintIndex::fingerprint_of` is O(R) PER CALL,
not O(1) — VERIFIED.** I read `src/engine/l5/fingerprint.rs:64-153` and the
three call sites directly:
- `FingerprintIndex::build` (`fingerprint.rs:64-85`) builds `stable_by_id` —
  one entry per routine with a non-empty `normalized_signature_hash`, so
  ~R entries (~100,941 at 8020) — as a plain unsorted `HashMap`. Confirmed:
  it never constructs a sorted view.
- `fingerprint_of` (`fingerprint.rs:91-153`) is called once per `Finding`:
  `d19.rs:126`, `d12.rs:119`, and `d1.rs:1102-1104` (post-merge, once per
  merged finding) — all three call sites confirmed by direct grep, matching
  the anatomy doc exactly.
- Inside ONE call: `fingerprint.rs:123` re-collects the ENTIRE `stable_by_id`
  map into a fresh `Vec` — EVERY call, confirmed not cached anywhere on the
  struct. `fingerprint.rs:124-128` sorts that Vec (length-desc then
  lexical) — `O(R log R)` String comparisons, every call. `fingerprint.rs:
  134-152` then walks every byte position of the finding's `root_cause_key`
  (`key_len` positions) and at EACH position linear-scans the entire sorted
  Vec via `starts_with` — `O(key_len · R)`, every call.
- So one `fingerprint_of` call costs `O(R log R + key_len · R)`; a detector
  that calls it once per finding costs `O(F_d · R log R)`. Finding count
  `F_d` itself grows roughly linearly with corpus size, so the realized
  growth is `O(R² log R)` — quadratic in routine count, which is exactly
  the "5400→8020 is only a 1.5-1.9× file/routine jump, but wall time jumps
  far more" signature both `d19` and `d12` show. Their own detector bodies
  are otherwise linear (verified: `d19`'s per-routine work is a
  `HashSet` build + O(1) parameter-membership checks, `d19.rs:59-70`;
  `d12`'s is a single O(events) pass, `d12.rs:35-63`) — so for these two,
  the fingerprint tail IS the entire 988s / 425s.
- Note this is a DIFFERENT cost from the one Wave 1 already fixed: W1.5
  (CLAUDE.md's Performance Targets table) hoisted the `FingerprintIndex`
  **build** to once-per-run; it did not touch this per-call re-sort inside
  `fingerprint_of` itself, which fires on every finding regardless of how
  the index was built.

**d1's own explosion — cone re-allocation inside a bounded DFS — VERIFIED.**
I read `d1.rs:614-632`, `capability_query.rs:133-138`,
`full_summary.rs:41-56`, and `path_walker.rs:148-249` directly:
- `expand` (`d1.rs:614-632`) calls `touches_db_of(s)` for every out-edge of
  every node the walk visits (`d1.rs:625-626`).
- `touches_db_of` (`capability_query.rs:133-138`) calls `s.reachable()` and
  scans for the first `table`-kind fact, early-returning on a match.
- `reachable()` (`full_summary.rs:46-56`) — confirmed by direct read —
  allocates a fresh `Vec::with_capacity(direct.len() + inherited.len())`
  and copies BOTH slices into it on every call, before `touches_db_of` gets
  to early-return. The allocation is paid regardless of whether the first
  fact is a hit.
- This runs inside `walk_evidence`'s bounded interprocedural DFS
  (`path_walker.rs:148-249`, confirmed `BOUNDS { max_depth: 20, max_nodes:
  500 }` at `d1.rs:59-61`) — so the allocation happens once per edge
  examined, up to 500 visited nodes × their fan-out, per in-loop call site
  (`d1.rs:899-968`).
- The allocation size, `K`, is a routine's cone size (direct + inherited
  capability facts) — and `K` grows with graph DENSITY (the SCC-fusion
  story in §2), not file count. That is the mechanism behind d1's wall time
  exploding far faster than the file-count ratio: a denser 8020 corpus
  makes every `reachable()` call inside every walk individually more
  expensive, compounding with the walk's own per-callsite node budget.

**Fix shapes (from the anatomy doc; both mechanical, Track-A-style — no
resolution-semantics change, byte-stable):**
1. **Fingerprint fix** (fixes d19 + d12 outright; removes d1's post-merge
   tail): hoist the sort into `FingerprintIndex::build` — sort once, store
   the sorted `Vec` on the struct — and replace the `O(key_len · R)` byte
   scan with structural id extraction (routine ids are fixed-structure, no
   id is a substring of another, so the embedded id(s) in a
   `root_cause_key` can be extracted by splitting on the known delimiter
   rather than testing all R candidates at every byte position). Turns each
   `fingerprint_of` call into `O(key_len)`; detector totals become `O(F_d)`.
2. **Cone-probe fix** (fixes d1's dominant term): precompute a `touches_db:
   bool` once per `FullRoutineSummary` when summaries are built, so
   `expand`/`terminals_at` read a flag instead of re-scanning and
   re-allocating `reachable()` per edge — `O(K)` → `O(1)` per edge examined.
   `reachable()` itself should return a non-allocating iterator/chain
   rather than a fresh `Vec`.

Both are localized (one struct/function each), independent of B1/B2's
memory-model surgery, and — because they are mechanical complexity fixes
rather than algorithm changes — **jump the Wave-2 queue ahead of B1/B2**:
they are cheaper to build, lower-risk (no output-shape change), and
directly address the measured dominator population instead of the general
memory model.

**Measured live context (update — see §4's full M2 results):** at 8020,
uncontended, `d1-db-op-in-loop` ran ALONE inside the L5 detector loop for the
ENTIRE 2.5-hour cap (~117 min) and never finished — this is the concrete
identity behind §4's "detector working sets never before reached at this
corpus size" attribution, now a named detector with a named exploding line
rather than a diffuse suspicion, AND now known to be severe enough to
prevent the full-default run from completing at all, not merely to slow it
down.

## 5. Implications sketch for the Wave-2 plan (open questions, not decisions)

These are forks the M1/M1b/M2 results above should settle before
`superpowers:writing-plans` is invoked for Wave 2. All three measurements
are now in (§2-§4a); the remaining open items below are genuinely open —
M1's own follow-up (source-sampling implicit-trigger edges) has now been
done (§2's update, Wave-2a Task 4) and confirmed the hypothesis. The items
below are updated accordingly — most were answered by that result plus §6's
Wave-2a outcome; what remains open is a smaller, sharper set.

- **Does the trigger-edge precision fix belong in Wave 2 ahead of B1/B2?**
  ANSWERED YES, with a caveat. §2's Task 4 confirmed the over-approximation
  (97.1% of intra-SCC trigger edges are collapsed OnValidate targets) AND
  §6's Wave-2a outcome independently reinforces the "ahead of B1/B2" case:
  T1+T2 (mechanical, Track-A-style, exactly the kind of fix a precision
  change is NOT) already gave d1 a real 2.8× at 5400 and STILL could not
  make the 8020 full-default run finish — the walk-graph SIZE (SCC density),
  not per-step allocation cost, is now the measured limiter. A fix that
  shrinks the SCC attacks that limiter directly; further constant-factor
  work on the Jacobi loop or d1's walk does not. The caveat stands as
  originally written: it IS a call-graph resolution change (the moat),
  carries its own north-star SHA re-verification cost against CDO, and can
  change resolved output shape — §2's estimate that the SCC shrinks toward
  ~84-member scale is not yet a measurement (re-running Tarjan post-fix is
  itself Wave-2b work, gated on goldens + a d43/d44/d45 event-detector FP
  review per the Wave-2b queue in §6).
- **Do the slowest detectors get targeted fixes, or does the whole L5
  witness-walker get the deferred §7 flow-insensitive redesign?** ANSWERED
  for the immediate question, still open for the long tail. §4a found
  concrete, mechanical, verified-against-source root causes for the three
  known dominators (d19, d12, d1) — a shared `FingerprintIndex::
  fingerprint_of` per-call cost and a per-edge cone-reallocation cost inside
  d1's own bounded walk — and §4's M2 run made the stakes concrete: on the
  full 8020 corpus, `d1` alone consumes the ENTIRE 2.5-hour budget and never
  finishes, which is the strongest possible evidence for targeted fixes
  first (a substrate-wide redesign is not even the bottleneck the current
  run can reach — d1 is). No per-detector 8020 table exists or is needed to
  make this call (§4 explains why one is both unobtainable and beside the
  point). What remains genuinely open: whether detectors #2 onward (never
  reached at 8020 — the loop got no further than d1) will show a NEW
  dominator once d1's fix lands, or whether the fingerprint + cone-probe
  fixes clear the whole run. That is exactly what the Wave-2a plan's T3
  re-measure is for; the §7 flow-insensitive redesign is the next lever
  only if T3 shows a new dominator that isn't fingerprint- or
  cone-allocation-shaped.
- **B1 interning staged by domain — does the trigger-edge finding change
  which domain goes first?** gpt-5.6-sol's adopted disposition (findings
  doc §6 item 10) already calls for staging B1 by domain rather than one
  global intern pass. Still not fully settled, but narrower now: §2's Task 4
  confirmed implicit-trigger edges ARE the SCC-fusion cause, but the fix is
  a builder-parity correction (field-specific targeting + RunTrigger gate),
  not evidence that the trigger/event id domain carries unusual
  `unknown_targets`/cone-membership churn independent of the SCC-fusion
  bug — M1b (§3) instead points at cone/summary SIZE as the dominant
  resident-memory driver, largely orthogonal to which id domain is largest.
  Open until B1 is actually staged: whether routine ids or event/trigger ids
  are the higher-value first slice remains a call to make at Wave-2b
  implementation time, not one this measurement round settles.
- **B2 shared-fact-content cones — does shrinking the largest SCC reduce
  B2's expected win, or are the two orthogonal?** B2's target is the
  O(routines × cone) memory term (+11.4 GB at 8k, H-5), which scales with
  SCC size directly for the recursive-SCC BFS-pull path
  (`capability_cone.rs:1687-1688` → `:1461-1525`). §3's 8.3M-entry fixed
  point on the 846-member SCC alone is a concrete, measured data point for
  how large that term gets — strengthening B2's case regardless of the
  SCC-shattering question. §2's Task 4 (now done) confirms the 846-member
  SCC IS expected to shatter once the trigger-edge fix lands (estimated
  toward ~84-member scale) — so B2's win on THIS specific SCC shrinks
  proportionally once that fix lands, but B2 still helps every other SCC's
  cone duplication and is not contingent on any one component's precision.
  §6 sequences this explicitly: trigger-edge fix first, B1/B2 for the
  remaining summary mass after — "both, sequenced," now a plan rather than
  a guess, though the exact post-fix numbers await Wave-2b's own
  re-measurement.

## 6. Wave-2a outcome (measured — post T1+T2 fix landing)

Status: DONE. Two fixes landed from §4a's root-cause analysis
(`docs/superpowers/plans/2026-07-18-engine-memory-speed-wave2a.md`, T1+T2),
both mechanical and byte-stable (goldens + DO differential clean):

- **T1 — `e2e34fc`, "structural stable-id substitution in
  `fingerprint_of`".** Replaces the per-finding re-sort-all-R +
  per-byte all-R `starts_with` scan (`fingerprint.rs:123-152`, the shared
  killer §4a identified) with an O(key_len) structural extraction + HashMap
  probe — ids are fixed-shape `{mid}/{64-hex}`, and equivalence with the old
  behavior is pinned by a scan-oracle unit test.
- **T2 — `136c4e2`, "zero-alloc reachable iteration + memoized touches_db in
  d1".** `touches_db_of` and siblings now early-exit over a chained
  iterator instead of allocating the direct+inherited Vec per probe
  (`reachable()`, §4a's other named exploding line); d1 additionally
  memoizes the per-routine touches-db answer across its `walk_evidence` DFS
  instead of re-probing every edge examined.

**Re-measure** (sources: `.superpowers/sdd/w2a-remeasure.md`, the T3
runbook, plus the post-fix instrumented 8020 run,
`w2a-8020-default.txt.stderr`):

| Run | Before (this doc's own measurements) | After T1+T2 |
|---|---:|---:|
| slice-5400, full-default | 2,608 s (§3a, contended) | **304.2 s** (8.6×) |
| — d19-unused-parameter | 988.19 s | 0.23 s |
| — d12-dead-integration-event | 424.83 s | 0.07 s |
| — d1-db-op-in-loop | 448.35 s | 157.88 s (2.8×; still 87.7% of the loop, 55% of total wall) |
| 8020, 3-detector (d61/d62/d64) | 90.3 s (findings doc §7b, Wave-1 outcome) | **40.9 s** (2.2×) |
| DO, default set | ~10.7 s baseline | 9.02 s (no regression) |
| 8020, full-default | DNF, 2.5h cap / 49.1 GB peak (§4) | **STILL DNF** — 2h cap / 45.2 GB peak; d1 alone ~93 min, never finished |

**Honest conclusion: T1/T2 annihilated the fingerprint quadratic and bought
d1 a real 2.8×, but d1's DFS at 846-SCC density is unbounded in practice on
the full corpus.** At 5400, d1 went from dominant-but-finishing (448s) to
dominant-and-finishing-much-faster (157.9s, still 87.7% of a much smaller
loop) — a genuine, verified win, and d19/d12 are effectively eliminated
(988s/425s → sub-second). At 8020, the fix bought d1 more runway before
hitting the wall, not a finish. The post-fix instrumented run shows the
SAME 58-pass Jacobi trace as §3 — `SCCSTATS`/`SCCANATOMY`/every `JACOBI`
line byte-identical to the pre-fix run (nodes=100941, sccs=82613,
max_scc=846, final `sum_cardinalities`=8,344,476 at pass 58) — confirming
T1/T2 never touched summary-computation semantics, only the detector-loop
consumers of it. Substrate/Jacobi handoff to the detector loop lands at
t=1,599.42s (~26.7 min, RSS 45,158 MB peak ≈ 45.2 GB — the Jacobi block
itself, t=166.11→1,349.23s ≈ 1,183s, is ~74% of that substrate time). Past
that handoff, d1 runs to the 2-hour kill (~93 min) without finishing, and
zero further detectors are reached — confirmed the same way as §4: zero
`detector:*:end` lines appear anywhere past `l4_detector_context:end` in
the raw log. The cone-reallocation fix genuinely works (every step of the
walk is now cheaper, and 5400 proves it), but at 846-member SCC density the
walk GRAPH itself — not the per-step allocation cost T2 targeted — is now
the limiter: T2 made each step cheaper without changing how many steps a
500-node/20-depth bounded walk over a 1,000+-fact-cone graph takes.

**The measured Wave-2b queue, in order:**
1. **Trigger-edge builder parity** (licensed by §2's Task 4 verdict above)
   — field-specific OnValidate targeting + the RunTrigger gate, bringing
   `build_implicit_trigger_edges` to parity with the fresh resolver's
   already-correct `implicit_trigger_route_applicable`. Expected to shatter
   the 846-SCC toward ~84-member scale, shrinking BOTH d1's walk graph (the
   measured limiter above) and the Jacobi block AT THE ROOT, rather than
   continuing to optimize constants inside either. Gated on goldens + a
   d43/d44/d45 event-detector false-positive review (advisory-graph
   semantics change).
2. **§7 flow-insensitive d1-walker redesign** — only if d1 remains
   disproportionately hot after (1) lands and is re-measured; not licensed
   by anything measured so far, since (1) hasn't landed yet.
3. **B1/B2 for summary mass** — 8.34M cardinalities on one SCC (§3) is a
   real resident-memory cost largely independent of whether that SCC
   shatters; still the right target for the memory MODEL once (1)/(2)
   settle the wall-time story, per §5's B1/B2 bullets above.

## References

- `docs/superpowers/specs/2026-07-17-engine-memory-speed-findings.md` — §5.2
  (Track B candidates B1-B4), §7 (Wave 2/3 ranked proposal), §7b (Wave 1
  outcome, the full-default DNF), §8 (the follow-up measurements this
  document answers).
- `docs/OUTSTANDING.md` — "Engine memory/speed Wave 2/3 (Track B)" backlog
  item.
- `.superpowers/sdd/w2-jacobi-5400-analysis.md` — the raw instrumented
  slice-5400 run (§2's 5400-slice comparison, §3's threshold-not-reached
  result, §3a's detector-loop profile all source from this).
- `.superpowers/sdd/w2-detector-anatomy.md` — the d1/d19/d12 complexity
  root-cause derivation §4a folds in and independently re-verifies.
- `w2-8020-default.txt.stderr` — the raw instrumented full-default 8020 run
  (§3's 58-pass JACOBI telemetry, §4's substrate timeline and kill result
  both source from this). Session scratchpad path:
  `C:/Users/SShadowS/AppData/Local/Temp/claude/U--Git-al-call-hierarchy/
  081c8b58-eebc-48da-8528-7f6a3bd5c7cb/scratchpad/w2-8020-default.txt.stderr`
  — an ephemeral temp-directory file, not repo-tracked; if this document
  outlives the session, the raw log may no longer exist at that path (the
  extracted numbers above are the durable record).
- `docs/superpowers/plans/2026-07-18-engine-memory-speed-wave2a.md` — the
  T1/T2 implementation plan §6's fixes come from (committed).
- `.superpowers/sdd/w2-trigger-edge-verdict.md` — Wave-2a Task 4, the
  source-sampling verification §2's "Update — VERIFIED" folds in.
- `.superpowers/sdd/w2a-remeasure.md` — Wave-2a Task 3's re-measure runbook,
  §6's primary source for the slice-5400/8020-3-detector/DO numbers.
- `w2a-8020-default.txt.stderr` — the raw post-fix instrumented full-default
  8020 run (§6's substrate timeline, RSS peak, and byte-identical-JACOBI
  confirmation source from this). Same scratchpad directory as the
  pre-fix log above, same ephemeral-file caveat.

## 7. Wave-2b outcome (trigger-edge builder parity — measured 2026-07-18)

Landed: `a640815` (field-specific OnValidate targeting + RunTrigger gate in
`build_implicit_trigger_edges`, mirroring `implicit_trigger_route_applicable`
exactly — the discovered fresh-side mapping gates only explicit
`RunTrigger=false`; absent maps to Guarded and keeps the edge) + `f9ff427`
(quoted-field normalization guard tests). Zero golden movement (independently
triaged: the committed fixture corpus contains only well-formed trigger
patterns — the pathology is Base-App-density-only). DO findings
byte-identical; d13/d16 `candidatesConsidered` dropped 7361→7296 (65
over-approximated edges pruned; telemetry-only).

**The §2 performance hypothesis is FALSIFIED.** 8020 anatomy before→after:
max_scc 846→797 (−5.8%), intra_edges 2610→2378, implicit-trigger intra
1077→954. Timings flat: slice-5400 full-default 304.2→292.6 s (noise), 8020
3-detector 40.9→45.3 s (noise), DO 9.0→9.5 s (noise). The long 8020
full-default confirmation run was deliberately skipped — with d1's walk graph
essentially unchanged there is no mechanism for a different outcome.

Why the estimate was wrong: the §2 verdict assumed retargeted per-field
OnValidate edges would leave the component. They do not — a hub table's field
triggers live in the same call neighborhoods, so edges RETARGET within the
SCC rather than exit it, and the component is anyway held together by
direct (1067) and method (262) call cycles. The field-collapse fix splits the
per-table super-hub NODE but not the component.

Disposition: the fix STANDS on precision/parity grounds (honest advisory
graph; per-field witness targeting; 65 pruned even on DO). Its performance
claim is retired. The perf queue re-ranks: (1) the §7 flow-insensitive
d1-walker redesign is now the top lever — d1 remains the sole full-default
blocker at 8020 scale; (2) B1/B2 for summary mass and the Jacobi plateau.
See OUTSTANDING.md for the re-ranked queue.

## 8. Wave-2c outcome (d1 walk_evidence memoization — measured 2026-07-18)

Landed: `511845c` — d1's `walk_evidence` memoized per callee (the walk from a
callee is caller-independent; each callsite derives its result by a
prefix + additive-depth transform, proven in the design doc §3 and pinned by
a full-field memoized≡fresh unit test). Byte-identical: goldens clean, DO
diff empty modulo `generatedAt`, two Opus reviews. Collapses d1's walk count
from O(in-loop-callsites) to O(distinct callees).

**The 8020 full-default finish bar is STILL UNMET.** The decisive run was
killed at the 2 h cap (peak 51.9 GB). Attribution is honest-blind: the
instrumentation was swept pre-merge, and this measurement batch ran under
~55% ambient machine load (Defender/WMI/WSL) that inflated even non-d1
control runs (8020 3-det 41→92 s, DO 9.0→10.3 s) — the short-run numbers
from this batch are NOT usable as evidence in either direction.

Open follow-up (needs a quiet machine + a probed build): re-run the 8020
full-default with stage marks to attribute the remaining wall — candidates:
(a) d1's distinct-callee count × 500-node walks is still enormous at 797-SCC
density (the memo removes REDUNDANCY, not the walk itself); (b) the
substrate's Jacobi block under load; (c) a later detector never before
reached. Until that attribution exists, no further perf work is licensed
(measure-before-build doctrine — this arc has now falsified two magnitude
estimates and will not risk a third).

## 9. The decisive d1 attribution (perf_trace runs, 2026-07-19)

Infrastructure: the permanent `perf_trace` layer (spec
`docs/superpowers/specs/2026-07-18-tracing-infra.md`) replaced the throwaway
probes; three traced d1-only 8020 runs (quiet machine) nailed the attribution
the arc had been missing. Full data: `.superpowers/sdd/d1only-verdict.md`.

Facts (run 3, 60s checkpoint series, all cross-checks internally consistent):
- Substrate incl. the full Jacobi block: ~24 min, CLEAN — ruled out as the
  wall. Peak RSS 43.5 GB is set by the substrate (cones), not d1.
- d1 census: 22,169 in-loop callsites → 7,105 walk candidates → 4,116
  distinct callee roots (the Wave-2c memo works: 623 hits / 823 misses at
  kill).
- Walk economics: ~0.3 walks/s (bursty, heavy-tailed; max entry = 2,884
  results); full census ≈ 3 h for d1 alone.
- **THE RATIO: 30.9% cut / 69.1% COMPLETE** — stable across all 20
  checkpoints. 126 complete witness paths per walk on average; retained
  steps ≈ 1.6M at 823/7105 walks.

Verdict: **complete-path multiplicity — d1 is output-bound.** Its semantics
enumerate every complete evidence path (additionalPaths are counted in the
output), so at 797-SCC density the work is bounded below by ~900k witness
paths. No behavior-preserving algorithm removes that; the three prior
optimization waves were correct but could never have closed this.

Consequences for the queue:
1. The next d1 lever is an OUTPUT-SEMANTICS DECISION (user's): cap/summarize
   additionalPaths per finding with an explicit capped-diagnostic (the
   honest-caps doctrine), or first-path + count. Goldens rebaseline; DO/CDO
   triage gates it. Estimated effect: d1 cost collapses to
   O(candidates × first-path) ≈ minutes.
2. CompleteOnly streaming (cut-result elision) is licensed only as a
   SECONDARY trim (≤31%) and only if pursued alongside (1).
3. B1/B2 remain the substrate levers (24 min / 43.5 GB floor for any
   full-default run) — unchanged priority, now cleanly separated from d1.
