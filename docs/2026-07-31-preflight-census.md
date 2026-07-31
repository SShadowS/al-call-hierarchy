# `preflight.fresh_coverage` — census ledger

The largest span nobody had ever looked inside. `src/program/` — the fresh
resolver, the moat — carried **zero** `pt::span` calls, so the whole preflight
was one opaque number.

It matters far more than the 8020 profile suggested: on **DO, the real customer
workspace, it is 83.4 % of the entire `analyze` run.**

## Method

Six spans added inside `build_context_res` / `fresh_coverage`
(`preflight.snapshot_build`, `.parse_snapshot`, `.dep_layer`, `.assemble_graph`,
`.resolve_full`, `.opaque_closure`, `.ctx_drop`). Spans ARE the census here — one
`OnceLock` read with tracing off — so this is permanent attribution, not a probe.
`preflight.ctx_drop` makes an already-happening drop explicit, the same pattern
`gate.teardown` and `context.ctx_drop` established; `ProgramReport` borrows
nothing from `ctx`, so the order is free to choose.

3 runs per corpus, `release-fast`, medians. `preflight.fresh_coverage`'s own self
time goes to **0.7 ms (DO) / 3.0 ms (8020)** — fully attributed.

## MEASURED — DO (real customer workspace), 3.17 s run

| span | ms | % of RUN | runs |
|---|---:|---:|---|
| **`preflight.parse_snapshot`** | **1,139.2** | **35.9 %** | 1175/1139/1109 |
| `preflight.resolve_full` | 535.6 | 16.9 % | 565/536/529 |
| `preflight.snapshot_build` | 458.8 | 14.5 % | 455/459/460 |
| `preflight.ctx_drop` | 248.7 | 7.8 % | 242/256/249 |
| `preflight.dep_layer` | 141.9 | 4.5 % | 166/141/142 |
| `preflight.assemble_graph` | 116.4 | 3.7 % | 135/113/116 |
| `preflight.opaque_closure` | 0.0 | 0.0 % | 0/0/0 |
| **total** | **2,642** | **83.4 %** | |

## MEASURED — 8020 (BC Base App), 32.4 s run

| span | ms | % of RUN | runs |
|---|---:|---:|---|
| **`preflight.resolve_full`** | **1,838.4** | **5.7 %** | 1906/1835/1838 |
| `preflight.parse_snapshot` | 786.4 | 2.4 % | 786/786/852 |
| `preflight.snapshot_build` | 439.7 | 1.4 % | 653/438/440 |
| `preflight.ctx_drop` | 282.1 | 0.9 % | 279/284/282 |
| `preflight.assemble_graph` | 154.0 | 0.5 % | 151/157/154 |
| `preflight.dep_layer` | 0.0 | 0.0 % | 0/0/0 |
| **total** | **3,503** | **10.8 %** | |

## The two corpora have OPPOSITE shapes — this was never one lever

On DO the top item is `parse_snapshot` (35.9 % of the run) and `dep_layer` costs
141.9 ms; on 8020 the top item is `resolve_full` and `dep_layer` is **zero**.
The cause is the corpus shape, and it is not subtle:

| | primary `.al` files | dependency `.app` packages |
|---|---:|---:|
| DO | 551 | **11** (Base App, System App, Business Foundation, 5× Continia, …) |
| 8020 | 8,020 | **0** |

`parse_snapshot` parses every SOURCE-BEARING app in the snapshot, dependencies
included, and BC 24+ dependency `.app`s ship embedded source. 8020 parses 8,020
files in 786.4 ms — **0.098 ms/file** — so DO's 1,139.2 ms is on the order of
**11,600 files, of which 551 are the primary app.** Roughly **95 % of DO's
preflight parse is dependency source**, re-parsed from scratch on every run.

**This falsifies the sizing on the parked "Preflight shared parse" item.** That
entry says the duplicated work is the primary app's parse only (correct — deps
parse once, in the fresh pass) and sizes it at "407 files of a dep-dominated
4.8 s resolve → sub-second saving". At the measured rate the primary app's 551
files are **~54 ms**, not sub-second. Shared parse is a ~54 ms item on DO. It
should be re-labelled, not built.

## What the preflight actually returns

Four scalars — `unknown`, `coverage_holds`, `recovered_files`, `opaque_apps` —
and then throws the entire semantic model away (`preflight.ctx_drop`). Of those,
`recovered_files` needs only the parse and `opaque_apps` only the snapshot;
`unknown` and `coverage_holds` are what force the full resolve.

So on DO the analyze CLI spends 2.64 s of a 3.17 s run building, resolving and
destroying a whole-program model of ~11,600 files — the overwhelming majority of
them **dependency files that did not change since the last run** — to produce four
numbers.

## Where the ceiling actually is

Not in making any of these phases faster. In not redoing them:

- **Dependency-parse caching across runs.** The dependency `.app` set is
  content-addressed already and changes only when a package is upgraded. Ceiling on
  DO: most of `parse_snapshot`'s 1,139 ms + `dep_layer`'s 142 ms.
- **Caching `FreshCoverage` itself**, keyed on a CONTENT hash of the workspace +
  dependency set. Ceiling on DO: the whole **2.64 s / 83.4 %** on a warm repeat —
  but only on an identical-input rerun (CI, a second `--format`, a no-op re-run);
  any primary-source edit is a guaranteed miss by construction, so this does NOT
  help the edit loop. `src/snapshot/cache.rs` — a live content-addressed on-disk
  cache for embedded `.app` source EXTRACTION, keyed on blake3 of the whole `.app`
  — is the house pattern to copy (atomic tmp+rename, fall through on a corrupt
  entry, never fatal). Note what it means: dep source TEXT is already cached across
  runs; the ~11,600-file PARSE of it is not.

**Two corrections to an earlier draft of this section, both verified at source:**

- **`AbiCache` is NOT a cross-run lever.** It is a process-level
  `Mutex<HashMap<(guid,name,publisher,version), Arc<SymbolReferenceAbi>>>`
  (`src/program/abi_ingest.rs:121-135`). `build_context_res` constructing it fresh
  per call costs nothing across runs — an in-memory cache cannot survive a process
  either way. It would matter only if `build_dep_layer` ran twice in one process
  (the LSP updater, not `alsem analyze`). Worse, its key is VERSION-based, not
  content-based, so persisting it as-is would be unsound: rebuilding a dep `.app`
  at the same version with different content is routine. And on DO the deps ship
  embedded source, so they are parsed as source, not ingested as ABI — this path
  is a slice of `dep_layer`'s 141.9 ms, not of the 1,139 ms parse.
- **`compute_gate_model_instance_id` is unusable as a cache key.** It hashes
  `"{guid}@{version}"` plus one `ws:<relPosix>` string per discovered file
  (`src/engine/gate/model_instance_id.rs:82-88`) — file NAMES, never file CONTENT.
  Edit any file body and the id is unchanged. A cache keyed on it would serve a
  stale verdict on every edit. It is a model-instance id, not a content hash.

Both are unbuilt and unmeasured here. This document is the attribution they would
have to be built against.

## Gates

DO `f022f677d2650b2399fc3aa5a7625bc6c078d90dd51cdb80e1e3705808fee3ea` and 8020
`36151bf67e17620724abb6b2cdbad55bcf8f97ffe3c3237782a0cf4c25ecc5fb`, both exact.
`scripts/check-goldens` green (9 targets, 0 failed, zero files under `tests/`
moved), `cargo clippy --all-targets --all-features` clean.
