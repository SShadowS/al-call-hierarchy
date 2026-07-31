# Cone fact keys become `Arc<str>` — measurement ledger

The post-allocator re-census of `context.capability_cones` (still the largest
single span at ~23 % of an 8020 run) split it as:

| item | ms | share of the span |
|---|---:|---:|
| singleton SCAN | 2,173.6 | 27 % |
| **`fact_cone`** (65,822 calls, **6,556,465 merges**) | 1,687.8 | 21 % |
| singleton FOLD | 1,041.1 | 13 % |
| `bfs` (503 calls) | 959.9 | 12 % |
| direct | 499.1 | 6 % |
| dedup / scc / graph / derived_fold / coverage / emit | ~630 | 8 % |

`fact_cone` is the second item, and its shape was the one the singleton walk had
already been fixed for one level down: `merge_cone` took an **owned `String`**, so
every one of the 6,556,465 calls passed `key.clone()` — a fresh heap copy of a
string that already existed, immutably, in the successor cone being read.

## Change

`ConeFacts` is `BTreeMap<Arc<str>, ConeFactEntry>`, and `merge_cone` takes the key
**borrowed** (`&Arc<str>`), looking up through `Arc<str>: Borrow<str>` and cloning
the `Arc` only on an insert. `Arc<str>` orders through `str`'s `Ord` exactly as
`String` does, so the map's iteration order — which every downstream consumer
depends on — is unchanged by construction.

Only the ~121,387 dist-0 entries (one per direct fact) still mint a key, from
`direct`'s own `String`s. **6,435,078 heap key copies per 8020 run, gone**, and a
cone entry copied into N predecessor cones now shares one allocation instead of
holding N.

`inherited_facts_by_bfs`'s `seen` set stays `BTreeSet<String>` and pays one
`to_string()` per newly-seen cone key: its sibling branch feeds it `direct`'s own
`String` keys, and the walk runs 503 times per run, so it is not on a hot path.

## MEASURED — paired A/B (C = allocator only, D = C + this), 3 pairs, 8020

Every population is byte-identical between the two binaries — `calls` 6,556,465,
`tiebreaks` 1,243,002, `cone_entries` 12,170,325, `wins` 10,522,793, `bests`
10,030,145 — which is the proof that the walk's structure is untouched and only
its allocation behaviour changed.

| phase | C med | D med | Δ | per-pair D/C |
|---|---:|---:|---:|---|
| **`fact_cone`** (the target) | 1,976.8 ms | **1,783.0** | **−9.8 %** | 0.90 / 0.92 / 0.86 |
| `phases_total` | 9,338.2 | 8,926.1 | −4.4 % | 0.93 / 1.02 / 0.94 |
| `walk` | 8,630.2 | 8,261.2 | −4.3 % | 0.92 / 1.01 / 0.94 |
| `scan` (control — untouched) | 2,581.6 | 2,659.1 | +3.0 % | 0.98 / 1.06 / 1.03 |
| `singleton` (control) | 4,007.6 | 4,020.5 | +0.3 % | 0.93 / 1.04 / 0.99 |
| `derived_fold` (control) | 232.8 | 231.7 | −0.5 % | 0.98 / 1.02 / 0.99 |
| `coverage_cone` (control) | 69.0 | 69.7 | +1.0 % | 0.98 / 1.02 / 0.99 |

**The controls put this probe's noise floor at about ±3 %.** `fact_cone` is the
only phase outside it with all three pairs pointing the same way, so **−9.8 % is
the claim**. `phases_total`/`walk` at −4.4 % have one contrary pair each and sit
barely above the floor — read them as "roughly −4 %", not as a figure.

Three small untouched phases moved more than the floor in the WRONG direction —
`scc` +23 %, `dedup` +19.6 %, `graph` +5.7 % — on spans of 100–320 ms. Nothing in
this change touches them; that is what noise looks like at that span size, and it
is reported rather than dropped.

**`context.capability_cones` (trace span): 9,634 / 9,107 / 9,357 ms → 8,945 /
9,259 / 8,772**, per-pair 0.928 / 1.017 / 0.937 — median −6.3 % with one contrary
pair. **Peak RSS 4,960 / 4,963 / 4,962 → 4,950 / 4,950 / 4,925 MB**, all three D
runs below all three C runs, ≈ −17 MB. **Whole-run wall is flat** (37.2/36.8/36.9 →
37.1/37.1/36.9 s) — a ~200 ms saving is not visible in a 37 s run, and is not
claimed from it.

DO: `fact_cone` 5.9 → 5.2 ms (−11.9 %), `phases_total` 64.1 → 61.0 (−4.8 %), peak
flat at ~1,579 MB. Same direction, absolute sizes far below anything claimable.

## The allocator swap masked most of this

Under the platform allocator, 6.4 M heap copies were worth far more than they are
now. The previous commit's own warning — *"this does NOT excuse allocation churn;
it makes a future churn regression cheaper and therefore harder to see"* — is
demonstrated here, one commit later, on the very next change: the same 6,435,078
removed copies buy −9.8 % of one phase instead of the much larger number the
pre-mimalloc profile would have shown.

The allocation COUNT is the noise-free part of this claim, exactly as it was in the
side-facts and cone-singleton ledgers. It is also now the ONLY part that will stay
legible as the allocator gets better at hiding it.

## Gates

DO `f022f677d2650b2399fc3aa5a7625bc6c078d90dd51cdb80e1e3705808fee3ea` and 8020
`36151bf67e17620724abb6b2cdbad55bcf8f97ffe3c3237782a0cf4c25ecc5fb`, both exact.
`scripts/check-goldens` green (9 targets, 0 failed, zero files under `tests/`
moved), 1,716 lib tests green, `cargo clippy --all-targets --all-features` clean.
