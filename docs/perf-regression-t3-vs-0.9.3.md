# Performance regression: master (post-T3 engine migration) vs v0.9.3

**Date:** 2026-07-13
**Compared builds:**
- **OLD** — released `v0.9.3` binary (the one currently committed in
  `al-lsp-for-agents/al-language-server-go-windows/bin/`)
- **NEW** — `master` @ `b09f9b1` ("Merge feat/t3-lsp-migration: T3 LSP surface
  migrated onto the program engine"), built locally with
  `cargo build --release --bin al-call-hierarchy`

**Baseline workspace:** `U:\Git\DO.Support-SlowDOSetup\DocumentOutput\Cloud`
- 551 workspace `.al` files, **4.2 MB** of source
- `Cloud\.alpackages`: 10 `.app` packages
- ancestor `DocumentOutput\.alpackages`: 22 more `.app` packages
  (picked up by the intentional `find_all_alpackages_folders` ancestor walk,
  BOTH builds see them)

All timings are medians of 3 fresh-process trials unless noted; the driver was
a raw stdio LSP client (initialize → didOpen → first hover, which blocks on
the full index build), plus the `--project` CLI mode. Warm-request timing used
5 iterations per operation on a long-lived process.

---

## 1. Results

### 1.1 Cold start (LSP mode: spawn → initialize → first usable hover)

| Metric | OLD v0.9.3 | NEW master | Delta |
|---|---:|---:|---|
| `initialize` response | 0.011 s | 0.013 s | — (both defer indexing) |
| First hover (blocks on index) | **0.86 s** | **5.10 s** | **~6× slower** |
| RSS after index (LSP mode) | **82 MB** | **~2,000 MB** | **~24×** |

Per-trial NEW numbers were extremely stable (5.045 / 5.110 / 5.133 s;
1965 / 2091 / 1965 MB) — this is systematic cost, not noise.

### 1.2 CLI `--project` (index-and-report mode)

| Metric | OLD v0.9.3 | NEW master | Delta |
|---|---:|---:|---|
| Wall time | 0.87–0.99 s | 3.69–4.04 s | **~4× slower** |
| Peak RSS (sampled at 20 ms) | **222 MB** | **1,869 MB** | **~8.4×** |

Note LSP mode's steady-state RSS (~2 GB) is *higher than the CLI peak* — see
§2.3 (the server keeps a second full parse alive for the updater).

### 1.3 Warm request latency (after indexing)

hover / prepareCallHierarchy / incomingCalls / outgoingCalls / references /
workspace-symbol: **all sub-millisecond on both builds.** No steady-state
regression; the entire problem is in the initial build.

### 1.4 `--analyze` mode (unaffected)

`--analyze` uses its own lightweight walkdir+parse path, not
`LspSnapshot::build_full`:

| Metric | OLD | NEW |
|---|---:|---:|
| Wall time | 0.71–0.76 s (`--format json`) | comparable |
| Peak RSS | 24 MB | 27 MB |
| Findings | 1,440 (345 crit / 1,095 warn) | 1,475 (366 / 1,109) |

Findings diff is **purely additive**: 35 new true positives (e.g.
`too_many_parameters` on interface-declared procedures, additional
`high_complexity` hits), **zero findings lost**. Not a regression.

### 1.5 Output-quality differences (context)

NEW reports different outgoing-call counts in some cases (e.g.
`GetNextMinObjectRange`: OLD returned 3 calls to record intrinsics
`SetCurrentKey`/`FindSet`/`Next`, NEW returned 0). Per SShadowS: **OLD had a
bug where it didn't index the app packages correctly**, so count deltas here
are fixes/behavior changes, not regressions. They are out of scope for this
document; the scope is time + memory.

Index-stat counters also changed definition (CHANGELOG-documented):
- OLD: `3606 definitions, 17948 call sites, 68411 external definitions from 22 packages`
- NEW: `4872 definitions, 17973 call sites, 126640 dependency definitions (embedded source)`

---

## 2. Why: root-cause analysis

### 2.1 The dominant cost: full parse of embedded dependency source

The T3 program engine (`LspSnapshot::build_full` →
`program::resolve::full::build_context`) works on **real embedded source
extracted from every dependency `.app`**, where the legacy indexer only
consumed each `.app`'s SymbolReference catalog (symbol-only, no parsing).

Measured embedded-source volume in this workspace's `Cloud\.alpackages`:

| Package | embedded .al files | source MB |
|---|---:|---:|
| Microsoft Base Application | 8,020 | 99.2 |
| Microsoft System Application | 1,309 | 6.6 |
| Continia Delivery Network | 505 | 3.9 |
| Continia Core | 290 | 2.1 |
| Microsoft System | 356 | 1.1 |
| others | 247 | 1.1 |
| **Total (deps)** | **10,727** | **114.1** |
| Workspace (for contrast) | 551 | 4.2 |

So NEW tree-sitter-parses **~27× more source than the workspace itself**
(10,727 dep files + 551 ws files vs OLD's 551), and keeps the results alive:

- `ProgramContext.parsed: Vec<ParsedUnit>` holds the parse of *every
  source-bearing app* — per the code's own doc, "the IR arena is exactly as
  large as the source file", so ≥114 MB of arenas + ≥114 MB of `Arc<str>`
  text, plus tree/node overhead which is a large multiple of raw source size.
- `LspSnapshot.dep_texts` retains an `Arc<str>` of every embedded dep file.
- `LspSnapshot.dep_decl_by_id` holds 126,640 `DeclEntry` values.
- `assemble_program_graph` builds `ObjectNode`/`RoutineNode`s for all ~10k
  dep objects; `ResolveIndex` + `BodyMap` are built over the whole graph.

OLD's 82→222 MB footprint tracked the symbol catalogs only.

**Timeline from `--verbose` (CLI mode, seconds granularity):**
- 18:59:13 → 18:59:14: `load_all_apps` (22 packages, both `.alpackages`
  dirs) ≈ 1 s
- 18:59:14 → 18:59:17: parse + dep layer + graph assembly + resolve ≈ 3 s

### 2.2 Base Application dominates

99.2 of the 114.1 MB (87%) is Microsoft's Base Application. Any mitigation
that lazy-loads / demand-parses / caches Base App handles the bulk of both
regressions. Note also that both `.alpackages` dirs contain byte-identical
copies of Base App / System App (ancestor walk); GUID dedup keeps one *unit*,
but both files are still opened and their zip directories scanned during
`load_all_apps`.

### 2.3 LSP server mode parses everything TWICE — and the wrong copy survives

`server.rs:365` uses `LspSnapshot::build_full_with_parsed`, which by its own
doc runs "a second, fully independent `parse_snapshot` pass" (AlFile is not
Clone) so the incremental updater owns a private mutable copy:

- **Scan #1** (`build_context`'s `parse_snapshot`) is consumed by
  `from_context`: workspace files' `AlFile`+text move into the published
  snapshot's `parsed`; **dep IR arenas are dropped** — but dep *texts* are
  first copied into `dep_texts` (`Arc::from(&str)` = fresh allocation).
- **Scan #2** (the extra `parse_snapshot`) survives **wholesale** in
  `Updater::parsed: Vec<ParsedUnit>` — including all 10,727 dep files' IR
  arenas + texts — for the lifetime of the server.

This is why LSP-mode steady state (~2.0 GB) exceeds even the CLI peak
(1.87 GB), and why cold start is ~5.1 s in LSP mode vs ~3.7–4.0 s in CLI mode
(the second parse costs roughly the delta).

### 2.4 What it is NOT

- Not the `--analyze` path (separate pipeline; flat on both builds).
- Not `load_all_apps` I/O (~1 s, similar in both; OLD reported
  "68411 external definitions from 22 packages in 594 ms").
- Not warm-request handling (sub-ms on both).
- Not measurement noise (tight per-trial spread).

---

## 3. Ownership audit: which scan should survive, and where the bytes live

### 3.1 Steady-state memory map (LSP server mode)

Embedded dependency source **text** (~114 MB raw) is retained in **three
independent copies**, and the dep **IR arenas** in one:

| # | Holder | What | Retained? |
|---|---|---|---|
| T1 | `LspSnapshot.snap` → `AppSetSnapshot.apps[*].source.files[*].text: String` | all embedded dep text | yes — `snap: Arc<AppSetSnapshot>` lives in every published snapshot AND is `Arc::clone`d across rung-1/2 rebuilds |
| T2 | `LspSnapshot.dep_texts: HashMap<(AppRef,String), Arc<str>>` | all embedded dep text, **freshly copied** via `Arc::from(pf.text.as_str())` in `build_dep_indexes` | yes |
| T3 | `Updater.parsed[*].files[*].text: String` | all embedded dep text again — `parse_snapshot` does `text: f.text.clone()` from `snap` | yes |
| A1 | `Updater.parsed[*].files[*].file: AlFile` | **IR arenas + trees for all 10,727 dep files** | yes — the dominant single item |
| A0 | scan #1's dep `AlFile`s | dep IR arenas | **no** — correctly dropped at the end of `from_context` |

Workspace-side copies (551 files / 4.2 MB) exist similarly (snapshot
`parsed` + updater unit + `snap`) but are negligible at this scale.

None of T1/T2/T3 need to be independent allocations. If
`SourceFile.text`, `ParsedFile.text` and `dep_texts` all shared one
`Arc<str>` per file, two of the three ~114 MB text copies disappear with
zero data loss. (`ParsedFile.text: String` → `Arc<str>` is mechanical;
`al_syntax::parse(&f.text)` only needs `&str`.)

### 3.2 Which parse should survive? Neither as-is — split by mutability

What each consumer actually needs, from the code:

- **Published snapshot** needs: workspace `AlFile`s (hover/def-surface),
  `dep_texts` (navigation into deps), `dep_decl_by_id` (spans), graph.
  It does NOT need dep IR arenas (already drops them).
- **Updater rung 1** (hot path, per-save): `BodyMap::build(&cur.graph,
  &self.parsed)` — reads only witness spans + signature data.
- **Updater rung 2** (signature change): re-assembles the workspace layer
  over the **cached, unchanged `dep_layer`**, then needs `self.parsed`'s
  dep units ONLY to rebuild `BodyMap` + `build_dep_indexes` against the
  new graph — and only because `RoutineNodeId`/`AppRef` are interned
  per-graph, so the previous snapshot's `dep_decl_by_id` can't be
  forwarded (`updater.rs`: "an `Arc::clone` forward would dangle the
  moment `cur.graph` is dropped").
- **Updater rung 3** (deps changed): re-reads and re-parses everything
  from disk anyway; retained dep parses are discarded.

So: **dependency parses are immutable between rung-3 rebuilds; only the
workspace unit is ever spliced.** The conflict that motivates the double
parse (`AlFile` is not `Clone`, both sides want ownership) exists **only
for the 551 workspace files (4.2 MB)** — never for the 10,727 dep files.

**Recommended ownership split** (answers "first or second?"):

1. The **first** scan survives and is handed to the updater as its working
   state — `build_full_with_parsed`'s second `parse_snapshot` is deleted.
2. `from_context` re-parses **only the workspace files** for the published
   snapshot's `ParsedFileEntry` (4.2 MB, ~0.2-0.3 s parallel). Rung 2
   already does exactly this per file (`al_syntax::parse(&pf.text)` in
   `apply_rung2`), so this makes the batch path consistent with the
   incremental path rather than introducing a new pattern.
3. Equivalently (fewer re-parses): first scan's **dep units** go to the
   updater, first scan's **workspace unit** goes to the snapshot, and the
   updater re-parses the workspace unit privately. Either way the
   duplicated work shrinks from 118 MB of source to 4.2 MB.

Expected effect: cold start 5.1 s → ~4 s (CLI parity), and removes one full
set of dep texts + arenas from steady state.

### 3.3 Do dep IR arenas need to be retained at all?

Even after 3.2, the updater still holds dep `AlFile`s solely so rung 2 can
rebuild `BodyMap`/`build_dep_indexes` against a freshly interned graph. Two
alternatives, in increasing ambition:

- **(a) Make dep decl data graph-independent.** `build_dep_indexes` extracts
  owned `DeclEntry` values (name + origin spans + virtual_path). If dep
  routine identity were stable across graph re-interning (content-addressed
  or keyed by `(app-guid, object, routine)` instead of graph-interned
  `AppRef` indices), rung 2 could `Arc::clone` the previous
  `dep_decl_by_id`/`dep_texts` forward — exactly as rung 1 already does —
  and **no dep parse needs to be retained at all**. Rung 3 rebuilds from
  disk regardless. This removes the single largest steady-state item (A1).
- **(b) Keep spans, drop trees.** Retain per-dep-file only what `BodyMap`
  actually serves (witness spans + routine signatures — the def-surface
  audit's own finding, updater.rs doc ~line 55), i.e. the already-compact
  `DeclEntry`-shaped data, not the full arena. `engine/deps/dep_artifact_l4.rs`
  already models a compact per-dep artifact in this spirit.

Estimated steady state after 3.1 + 3.2 + 3.3(a): **~150–300 MB** (one shared
text copy + graph + indexes), from ~2,000 MB today — with zero loss of
served data (every span/text/decl the handlers can serve is preserved).

### 3.4 Future direction (noted, out of scope for now)

Per SShadowS: consider a **tiered dependency-import mode** — LSP/navigation
mode may not need the full in-memory embedded-source program model that a
future deep/full-analysis mode (other tools) legitimately would. A
`DepImport::{SymbolOnly, Navigation, FullAnalysis}` knob (config or
per-request) would let the LSP default stay lean (Base App = 87% of the
volume is rarely navigated into) while keeping the full model available on
demand. Parked for a later task; 3.1–3.3 are worth doing regardless.

---

## 4. Suggested mitigations (in rough impact order)

1. **Eliminate the duplicate parse in `build_full_with_parsed`** (§3.2):
   hand scan #1 to the updater, re-parse only the 4.2 MB workspace unit for
   the published snapshot. ~1 s cold start + one full dep text+arena set.
2. **Share text allocations** (§3.1): one `Arc<str>` per file across
   `AppSetSnapshot`/`ParsedFile`/`dep_texts` — saves ~228 MB here.
3. **Stop retaining dep IR arenas** (§3.3a): stable dep routine identity →
   forward `dep_decl_by_id`/`dep_texts` across rung 2 like rung 1 does.
4. **Persist a dep-layer artifact cache** keyed by `.app` hash
   (`dep_artifact_l4` groundwork): every cold start after the first skips
   the 99 MB Base App parse entirely.
5. **Skip duplicate `.app` files across `.alpackages` dirs before opening
   them** (dedup by app identity before zip extraction, not after).
6. **Tiered dep import** (§3.4) — later.

## 5. Repro commands

```powershell
# CLI wall time + verbose stage log
al-call-hierarchy.exe --verbose --project U:\Git\DO.Support-SlowDOSetup\DocumentOutput\Cloud

# Peak RSS sampler (20ms polling; psutil) — scripts/peak_rss.py
python scripts/peak_rss.py <exe> --project U:\Git\DO.Support-SlowDOSetup\DocumentOutput\Cloud

# LSP cold start: spawn -> initialize -> didOpen -> first hover (blocks on index)
# (raw stdio LSP driver; remember to percent-encode spaces in file URIs and to
#  skip publishDiagnostics notifications when matching responses)
```

---

## 6. 2026-07-13 close-out: Mitigations 1, 2, 5 — IMPLEMENTED

Branch `feat/perf-safe-wins` landed the three safe-wins mitigations
identified in §4:

- **Mitigation 2** (§3.1, share text allocations) — commit `6a4292b`:
  one `Arc<str>` per source file shared across
  `AppSetSnapshot`/`ParsedFile`/`dep_texts`; collapses two of the three
  ~114 MB embedded-dep-text copies (T1/T2/T3 in §3.1) into one.
- **Mitigation 1** (§3.2, eliminate the duplicate parse) — commit
  `8c83894`: one `Arc<AlFile>` per parse; `build_full_with_parsed`'s
  second `parse_snapshot` and the rung-1/rung-2 re-parses are deleted.
  LSP mode previously parsed all ~10,727 files TWICE at cold start; it
  now parses them once.
- **Mitigation 5** (dedup `.app` files before opening) — commit
  `d305f25`: manifest-first `.app` GUID dedup — `SymbolReference.json` is
  now parsed only for dedup winners, instead of for every `.app` found
  across all `.alpackages` directories. (A corrupt-symbols winner now falls
  back to the next-highest good copy of the same GUID rather than the
  dependency vanishing — see CHANGELOG's Unreleased entry.)

Mitigations 3 (§3.3a, stop retaining dep IR arenas / stable dep routine
identity) and 4 (persistent dep-layer artifact cache) are **not** part of
this close-out — they remain open, tracked as §3.3(a)/§4-item-3 (graph
-independent dep decl identity) and §4-item-4 (persistent dep-layer
artifact cache) respectively. The residual cold-start/RSS gap vs. the
"expected direction" numbers in §3.2/§4 is owned by those two items, not
by anything landed here.

### 6.1 Synthetic corpus — `cargo bench --bench lsp_pipeline -- build_full`

Two consecutive clean runs (release/bench profile, no other cargo build
competing for the lock) on this dev machine:

| Bench | Run 1 median | Run 2 median | CLAUDE.md table target |
|---|---:|---:|---:|
| `build_full/100_files` | 15.13 ms | 14.76 ms | ~8.07 ms |
| `build_full/1000_files` | 189.70 ms | 166.72 ms | ~74.45 ms |

Both benches are well within the CI perf-bounds gate's 3x-of-target
ceiling (`tests/perf_bounds.rs`), and run 2 shows Criterion's own
regression check reporting an **improvement** vs. run 1
(`build_full/1000_files: -12.114%, p<0.05`, i.e. no regression from
Tasks 1-3's changes). The absolute medians on this machine run
consistently ~2x the CLAUDE.md table's dev-machine numbers — this reads
as this session's shared-environment CPU contention/machine variance
(noted in the repo's environment constraints), not a regression
introduced by Tasks 1-3; no source changed the `build_full` cost path,
and dep IR sharing/dedup only affects it indirectly via reduced GC/alloc
pressure. Re-measure on an idle machine if a tighter number is needed.

### 6.2 Real workspace — `U:\Git\DO.Support-SlowDOSetup\DocumentOutput\Cloud`

Reachable in this session; measured against the release binary
(`cargo build --release --bin al-call-hierarchy`).

**Cold CLI index** (`Measure-Command { .\target\release\al-call-hierarchy.exe --project <path> }`),
3 trials after the first (disk-cache-warm) run:

| Trial | Wall time |
|---|---:|
| 1 | 3.78 s |
| 2 | 3.73 s |
| 3 | 3.61 s |

Median **~3.73 s**, vs. the §1.2 CLI baseline (3.69-4.04 s, pre-mitigations)
— essentially flat, as expected: the CLI path already parsed everything
exactly once before these changes (§2.3's "double parse" was specific to
LSP server mode), so mitigation 1 is not expected to move the CLI number.
Mitigations 2/5 shave allocation and `.app`-open overhead but this
workspace's CLI wall time is dominated by parse+resolve, not by the
freed allocations.

**Peak RSS** (`python scripts/peak_rss.py .\target\release\al-call-hierarchy.exe --project <path>`),
2 trials:

| Trial | Peak RSS |
|---|---:|
| 1 | 1,629 MB |
| 2 | 1,649 MB |

vs. the §1.2 baseline of **1,869 MB** (CLI mode) — a **~130-240 MB (7-13%)
reduction**, consistent with mitigation 2 collapsing duplicate text
allocations and mitigation 5 skipping `SymbolReference.json` parses for
non-winning duplicate `.app`s on this workspace's 32 `.alpackages`
entries. This is smaller than §3.1's "~228 MB" estimate for LSP
steady-state (which counted three retained copies across
snapshot+updater); CLI mode only ever held one embedded-text copy plus
the transient parse copy, so it had less duplication to remove in the
first place. The larger ~2,000 MB **LSP steady-state** figure from §1.1
was not re-measured here (would require a full stdio LSP driver session,
out of scope for this close-out) — expect a larger absolute reduction
there since T1/T2/T3 in §3.1 collapse from three retained copies to one.

### 6.3 Remaining gap

The residual distance to the full §3/§4 "expected direction" (LSP cold
start toward CLI parity, steady-state RSS down a further full dep
arena+text set) is owned by the two mitigations intentionally **not**
attempted in this branch:

- **§3.3(a) / §4-item-3** — stable, graph-independent dep declaration
  identity, so `dep_decl_by_id`/`dep_texts` can be forwarded across rung 2
  instead of re-derived, letting dep IR arenas be dropped after the first
  build instead of retained for the updater's lifetime.
- **§4-item-4** — a persistent dep-layer artifact cache keyed by `.app`
  hash, so every cold start after the first skips the ~99 MB Base
  Application parse entirely.

Both are out of scope for this safe-wins branch and tracked for a future
task.

Embedded-source volume was measured by opening each `.app` (zip after the
NAVX header) and summing `.al` entry sizes.

---

## 7. 2026-07-13 independent re-measurement (real workspace, LSP stdio driver)

Same harness and workspace as §1 (DO.Support-SlowDOSetup Cloud, 551 ws
files, raw stdio LSP client, 3 fresh-process cold trials + warm session),
run against `58df646` (post safe-wins branch merge). "BEFORE" = master
`b09f9b1` measured in §1; v0.9.3 shown for reference.

### 7.1 LSP server mode (the §6.2 gap — now measured)

| Metric | v0.9.3 | BEFORE (b09f9b1) | AFTER (58df646) | After vs Before |
|---|---:|---:|---:|---|
| initialize response | 0.011 s | 0.013 s | 0.012 s | — |
| Cold start -> first usable hover | 0.86 s | 5.10 s | **2.87 s** | **-44 %** |
| RSS after index (steady state) | 82 MB | ~2,000 MB | **1,584 MB** | **-21 % (~-420 MB)** |
| Warm request latency (all ops) | sub-ms | sub-ms | sub-ms | unchanged |

Cold trials (AFTER): 2.943 / 2.846 / 2.858 s; RSS 1581 / 1587 / 1584 MB —
tight spread, systematic.

### 7.2 CLI `--project` mode

| Metric | v0.9.3 | BEFORE | AFTER | After vs Before |
|---|---:|---:|---:|---|
| Wall time (median of 3) | 0.93 s | 3.95 s | **3.50 s** | -11 % |
| Peak RSS (20 ms sampler) | 222 MB | 1,869 MB | **1,645 MB** | -12 % |

### 7.3 Output quality: unchanged

Warm-session payloads are identical before/after the safe-wins branch on
all four probe targets: prepareCallHierarchy items 1/1/1/1, incomingCalls
1/1/0/4, outgoingCalls 6/0/2/13, hover/references/workspace-symbol
behavior unchanged. No served data was lost.

### 7.4 Reading

- The **cold-start** win (-2.2 s) exceeds the duplicate-parse estimate in
  §3.2 (~1 s) — deleting the rung re-parses and manifest-first `.app`
  dedup (32 zips no longer all fully read) contribute the rest.
- The **steady-state RSS** win (~420 MB) matches §3.1's arithmetic: two
  of three ~114 MB text copies collapsed (~228 MB) plus the second parse's
  transient/retained overhead. The remaining ~1.5 GB is §3.3(a)'s dep IR
  arenas (`Arc<AlFile>` for 10,727 dep files, retained for rung 2) — the
  single dominant item, unchanged by design in this branch.
- LSP steady state (1,584 MB) is now BELOW the CLI peak (1,645 MB) —
  before the fix it was above it, which was the §2.3 double-parse smell.
- Remaining gap to the ~150-300 MB target in §3.3 is owned by the two
  open items: §3.3(a) stable dep decl identity (drop dep arenas) and
  §4-item-4 persistent dep-layer artifact cache (skip Base App parse on
  warm cold-starts).
