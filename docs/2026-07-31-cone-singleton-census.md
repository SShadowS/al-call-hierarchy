# `inherited_facts_for_singleton` — census ledger

The second half of `context.capability_cones`' attributed cost (`ALSEM_CONES_CENSUS=1`,
extended here from one number to four). The prior round left this at **4.8 s over
100,419 calls** with nothing saying what inside it cost — and precomputing
`edge_sort_key` against it bought only 7 %, which proved the cost was elsewhere
without saying where.

## Attribution (8020, `logs/cone-8020.log`, baseline binary)

`compose` 9,236.6 ms, of which `singleton` **4,908.2 ms** and `fact_cone`
2,105.5 ms; `bfs` is 1,300.8 ms over 503 calls.

| phase inside the singleton walk | ms | share |
|---|---:|---:|
| candidate SCAN (every out-edge's callee cone into `best`) | 2,995.4 | 61 % |
| derived FOLD (`fold_fact` over the survivors) | 1,605.9 | 33 % |
| raw materialization (`retag` + `sort_inherited`) | **0.0** | 0 % |

The raw path is **dead on the analyze path** — `ConeOutput::DerivedOnly` skips it —
so every optimization aimed at `retag`/`sort_inherited` would have measured nothing.

## The walk is not edge-bound, and never was

| population | 8020 |
|---|---:|
| calls | 100,419 |
| out-edges walked | **136,952** (1.36 per call) |
| `(key, entry)` cone pairs scanned | **12,170,325** (121 per call) |
| `best` insertions — each a `String` key clone | **10,522,793** |
| surviving `best` entries (= `fold_fact` calls) | **10,030,145** |

1.36 out-edges per call: there is no deep walk here to shorten, which is the third
time in this arc a hop-count framing has been falsified by counting the population
instead. The cost is that **86.5 % of the 12.17 M scanned cone entries WIN** their
comparison and insert — so the scan was allocating 10.5 M `String` copies of keys
that already live in `cones` for the whole walk. The fold's 1,605.9 ms is 10.03 M
`fold_fact` calls, most of which reach `store.interner.intern(rid)`.

## Change made

`best` is now `BTreeMap<&'g str, Best<'g>>` — keys borrowed from `cones` rather
than cloned. That is the whole change; **10,522,793 `String` allocations per 8020
run, gone.**

## MEASURED — paired A/B, alternating runs, 3 each, medians

| phase | base | fix | Δ |
|---|---:|---:|---:|
| `singleton` | 4,214.4 ms | **3,592.6 ms** | **−14.8 %** |
| `compose` | 8,089.0 ms | 7,702.9 ms | −4.8 % |
| `phases_total` | 10,081.4 ms | 9,803.9 ms | −2.8 % |
| `bfs` (control) | 1,085.1 ms | 1,185.9 ms | +9.3 % |
| `fact_cone` (control) | 1,889.5 ms | 2,101.0 ms | +11.2 % |

**Read this A/B more carefully than the `compute_summaries` one: both controls
moved ~+10 % in the OPPOSITE direction.** Neither is touched by this change, so
that is drift, not effect — the session got slower as it ran (`bfs` rises
monotonically across the six runs). Comparing each fix run against its immediate
base NEIGHBOUR, which cancels most of that drift, gives −10.7 % / −21.4 % / −9.4 %:
consistently negative, but with a spread wide enough that **−14.8 % should be read
as "somewhere around −10 to −20 %", not as a precise figure.** The noise-free part
of the claim is the allocation count.

## What is left, with its population already counted

- **A single-cone fast path.** The calls split **46,373 with ZERO cone-bearing
  out-edges / 30,160 with exactly ONE / 23,886 with two or more**. A one-cone call
  needs no merge at all: `best` ends up a key-for-key copy of that single callee
  cone, which is already a `BTreeMap` in the same key order, so the fold could
  iterate it directly and skip building `best` entirely. That covers 30 % of calls
  and **4,158,131 of the 12,170,325 scanned entries** (the other 8,012,194 belong
  to genuine multi-cone merges and would not move).
- **The fold's 10,030,145 `fold_fact` calls** — 1.6 s, and the bulk of it is
  `store.interner.intern(rid)` per fact. This is the same shape the d1 arc found
  (92 M interns) and the same shape the uncertainty substrate fixed one layer up:
  the answer is an interned id carried on the cone entry, not a re-intern per fold.
  Not built here; it needs its own round.

## Gates

DO `f022f677d2650b2399fc3aa5a7625bc6c078d90dd51cdb80e1e3705808fee3ea` and 8020
`36151bf67e17620724abb6b2cdbad55bcf8f97ffe3c3237782a0cf4c25ecc5fb`, both exact
(8020 reproduced on all 6 A/B runs). `scripts/check-goldens` green with zero files
under `tests/` moved, clippy clean, 1,716 lib tests green.
