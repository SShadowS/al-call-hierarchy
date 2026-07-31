# `alsem` swaps its global allocator — measurement ledger

The profile attribution (`docs/2026-07-31-profile-attribution.md`) ended with a
result that pointed away from every structural lever on the list: **16.6 % of an
8020 run is `free()`** (`gate.teardown` 13.8 % + `context.ctx_drop` 2.8 %), on top
of allocation cost spread through every other span. A workload whose *teardown*
alone outweighs `context.compute_summaries` is not asking for another data-structure
rewrite first; it is asking what the allocator costs.

So this was measured before anything was built: it is eight lines, and it is a
global multiplier on every span at once.

## Method, and why the usual control does not exist here

Paired A/B, binaries built from ONE tree, run **alternately** (A C A C …) so
session drift falls on both members of each adjacent pair, 4 pairs on 8020 and 5
on DO. Ratios are reported per pair as well as as a median.

**There is no untouched control span for this change** — that is the honest
limitation, and it is inherent: an allocator swap touches every allocation in the
process, so no span can be held out. The compensations are (a) alternating order,
(b) reporting every pair rather than only the median, and (c) reporting peak RSS,
which drift does not move.

- A = master `e7cc156`, platform allocator.
- B = A + `mimalloc`, default options. (Intermediate; see the trade below.)
- C = B + `purge_delay = 0`. **This is what ships.**

## Why `purge_delay = 0` is load-bearing, not tuning

Plain mimalloc (B) is a **peak-RSS REGRESSION on a small workspace**. mimalloc
holds freed OS pages for 10 ms by default before returning them, which amortizes
at Base App scale and does not at customer-workspace scale:

| corpus | A peak | B peak (default delay) | C peak (`purge_delay = 0`) |
|---|---:|---:|---:|
| DO | 1,598 MB | **1,717 MB (+7.4 %)** | **1,577 MB (−1.3 %)** |
| 8020 | 5,320 MB | 5,012 MB (−5.8 %) | ~4,945 MB (−7.1 %) |

Shipping B would have repeated exactly the failure this repo already recorded once
(the uncertainty substrate cost DO +0.4 MiB silently) at 300× the size. C costs
roughly **7–9 % of the wall-clock win** — 8020 35.2 → 37.8 s and DO 2,869 →
3,123 ms, measured directly — and buys a configuration that regresses **neither**
axis on **either** corpus. That trade is taken deliberately.

The option index is derived, not hardcoded blind: `libmimalloc-sys` exports no
`mi_option_purge_delay` constant (`mi_option_eager_commit_delay`, its lower
neighbour, is behind that crate's `v2` feature), so it is `mi_option_use_numa_nodes
- 1`, pinned by a `const` assertion against the two unconditionally-exported
neighbours — a numbering shift fails to COMPILE. **The semantic check is the
measurement**: a wrong index cannot move peak RSS, and setting it in code
reproduces the `MIMALLOC_PURGE_DELAY=0` env-var control to within 2 MB
(1,579 vs 1,577 MB, against 1,724 MB at the default).

## MEASURED — 8020 (BC Base App, 100,941 routines), 4 alternating pairs

**Whole run: `analyze.total` per-pair C/A = 0.605 / 0.637 / 0.571 / 0.563,
median 0.588 → −41.2 %** (54.0/55.4/61.5/65.1 s → 32.6/35.3/35.1/36.7 s). Every
pair negative, no exception, and A's own spread across the four runs (54 → 65 s)
is larger than the spread between the pairs — which is exactly why the pairing
matters.

**Peak RSS: 5,291 / 5,299 / 5,330 / 5,327 MB → 4,961 / 4,961 / 4,962 / 4,961 MB
= −6.6 % (−350 MB).** Note C's four values agree to within 1 MB while A's spread
39 MB: immediate purging makes the peak nearly deterministic.

| span (self) | A med | C med | Δ |
|---|---:|---:|---:|
| `context.capability_cones` | 10,061 ms | 8,170 | −18.8 % |
| **`gate.teardown`** | 8,519 | **1,104** | **−87.0 %** |
| `context.compute_summaries` | 5,869 | 4,094 | −30.2 % |
| `preflight.fresh_coverage` | 4,886 | 3,666 | −25.0 % |
| `gate.format` | 2,987 | 1,244 | −58.4 % |
| `detector.d2-event-fanout-in-loop` | 2,381 | 700 | −70.6 % |
| `context.build_total` (self) | 2,136 | 1,195 | −44.0 % |
| `gate.project_filter_scope_baseline_suppress` | 1,755 | 1,000 | −43.1 % |
| **`context.ctx_drop`** | 1,704 | **314** | **−81.6 %** |
| `l4_l5.role_scope_and_sort` | 1,681 | 988 | −41.2 % |
| `search_loops_cohorts` (self) | 1,620 | 1,267 | −21.8 % |
| `l3.parse_project_parallel` | 1,414 | 1,308 | −7.5 % |

The two spans that are pure `free()` fall by 87 % and 82 %, which is the
prediction the attribution made and the closest thing to a control this
measurement has: the change should hit deallocation hardest, and it does.

**Two spans did not improve, and are reported rather than dropped.**
`l3.discover_read` +0.9 % (per-pair 0.95/1.08/0.92/1.01) is disk-bound and flat,
as expected. `detector.d33-unfiltered-bulk-write` measures +10.5 % median on
per-pair ratios 1.14/1.13/**2.29**/0.87 — a 291 ms span with one wild outlier;
that is noise at this span size, not a finding, and it is too small to
distinguish from one either way.

## MEASURED — DO (real customer workspace), 5 alternating pairs

The small corpus must not regress — that is a standing rule here, and plain
mimalloc broke it (see the table above). With `purge_delay = 0` it does not.

**Whole run: `analyze.total` per-pair C/A = 0.733 / 0.720 / 0.762 / 0.794 / 0.775,
median 0.762 → −23.8 %** (4.38/4.37/4.04/4.10/4.16 s → 3.21/3.14/3.08/3.25/3.22 s).
Five pairs, all negative, tight spread.

**Peak RSS: 1,602 / 1,604 / 1,604 / 1,603 / 1,602 MB → 1,580 / 1,581 / 1,581 /
1,579 / 1,580 MB = −1.4 % (−23 MB).** A small win, not a regression — the whole
point of the option.

DO's only span above the 150 ms reporting floor is `preflight.fresh_coverage`
(3,394 → 2,668 ms, −21.4 %); the rest of its run is below it.

## What this does NOT do

- **It does not excuse allocation churn.** Every allocation the engine makes is
  still an allocation; this makes each one cheaper, which makes a future churn
  regression *harder to see*, not less real. Keep counting allocations — the
  counts in the side-facts and cone ledgers remain the noise-free part of those
  claims.
- **It is scoped to the `alsem` binary.** A `#[global_allocator]` is
  per-executable: the library, the LSP server (`src/main.rs`), `aldump`, the
  benches and every test target keep the platform default. The LSP server is a
  long-lived process with different allocator trade-offs and was **not measured**;
  `aldump` would be a safe follow-up but is likewise unmeasured here.
- **It is one machine, one OS.** These figures are Windows 11, whose default heap
  is the least favourable case for this workload. CI is `ubuntu-latest` against
  glibc `malloc`; expect a smaller gain there. Nothing in the claim depends on the
  size of the Linux gain, but nothing here measures it either.

## Gates

DO `f022f677d2650b2399fc3aa5a7625bc6c078d90dd51cdb80e1e3705808fee3ea` and 8020
`36151bf67e17620724abb6b2cdbad55bcf8f97ffe3c3237782a0cf4c25ecc5fb`, both exact —
which is the strong form of "an allocator cannot change output", checked rather
than assumed. `scripts/check-goldens` green with zero files under `tests/` moved;
`cargo clippy --all-targets --all-features` clean.
