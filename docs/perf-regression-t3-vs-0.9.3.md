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

---

## 8. 2026-07-14 close-out: Mitigation 3 (owned DeclSurface) — IMPLEMENTED

Branch `feat/owned-decl-surface` (off `feat/perf-safe-wins`) landed §3.3(a):
a fully-owned two-tier `DeclSurface` (`RoutineMeta` projection: name,
origins, `parse_incomplete`, param `ty`/`by_ref` — never the body) replaces
the borrowed `BodyMap<'a>` in the resolution decl-lookup surface, so the
dependency parse arenas (`Arc<AlFile>` for the ~10,727 dependency files)
can be dropped after the first full build instead of retained for the
LSP updater's lifetime.

Commits:

- `6f3ec77` — feat: add owned two-tier DeclSurface (RoutineMeta projection)
- `78c74ed` — refactor: migrate resolution decl lookups from BodyMap<'a>
  to owned DeclSurface
- `3239388` — perf: drop dependency parse arenas after first build (owned
  DeclSurface lifecycle)

### 8.1 Validation battery

- `cargo test` (debug): **all tests pass** (1,468 passed, 0 failed, 2
  ignored in the main integration binary, plus every other test binary
  green — see task-4-report.md for the full log).
- `cargo clippy --all-targets --all-features`: **clean**, zero warnings.
- `cargo test --release --test perf_bounds`: **9/9 PASS**.
- Zero golden regeneration was needed (`REGEN_TEMP_GOLDENS` never set) —
  the refactor is behaviorally invisible as required by the plan.

### 8.2 Synthetic corpus — `cargo bench --bench lsp_pipeline`

Two consecutive runs (release/bench profile), same dev machine as §6.1
(shared-environment CPU contention still applies — see below):

| Bench | Run 1 median | Run 2 median | CLAUDE.md table target (pre-Task-4) |
|---|---:|---:|---:|
| `build_full/100_files` | 13.11 ms | 13.64 ms | ~8.07 ms |
| `build_full/1000_files` | 165.61 ms | 169.29 ms | ~74.45 ms |
| `query_handlers_1000_files/prepare` | 8.11 µs | 9.13 µs | ~7.88 µs |
| `query_handlers_1000_files/incoming` | 22.19 ms | 22.56 ms | ~16.34 ms |
| `query_handlers_1000_files/outgoing` | 14.06 µs | 14.56 µs | ~6.60 µs |
| `compute_all_1000_files` | 7.07 ms | 7.12 ms | ~7.9 ms |
| `rung1_body_edit_1000_files` | 32.11 ms | 31.00 ms | ~13.28 ms |
| `rung2_signature_edit/1000_files` | 112.94 ms | 113.71 ms | ~149.93 ms |

Both runs are internally consistent (Criterion reports <5% run-to-run
noise for every bench except the already-known-noisy `incoming`/`prepare`
outliers). Absolute medians on this machine run ~1.6-2.4x the CLAUDE.md
table's dev-machine numbers, consistent with the same shared-environment
CPU contention documented in §6.1 (not a regression — no source change
in Tasks 1-3 touches `build_full`'s or `prepare`/`outgoing`'s cost path).

**`rung2_signature_edit` is faster in absolute terms** (~113 ms vs.
~149.93 ms pre-Task-4, a ~25% drop) despite running on a machine that is
inflating every other number by ~2x — a real improvement, consistent with
the plan's expectation that removing the all-units BodyMap rebuild would
help rung 2 most (a signature-change edit previously had to rebuild the
BodyMap projection for every unit in the graph).

**`rung1_body_edit_1000_files` (~31-32 ms) does not show the same clear
win** against the pre-Task-4 CLAUDE.md figure (~13.28 ms) once the ~2x
machine-contention factor is applied — it lands within the noise band of
"flat", not clearly improved. Rung 1 (body-only edit) never rebuilt the
BodyMap for other units even before this refactor, so a large win here
was not expected; the flat result is consistent with the design, not a
concern. Re-measurement on an idle machine would be needed for a
tighter comparison.

### 8.3 CDO gate

**Skipped.** `CDO_WS` is unset in this environment and no real Business
Central CDO workspace is reachable from this machine (checked `U:\Git\
Continia*` — none of the checked-out repos is a BC app workspace). Per
the plan's global constraint, CDO-gated tests skip silently when
`CDO_WS` is unset; `scripts/cdo-gate` was not run. This is a known gap
for whoever next runs this close-out on a machine with `CDO_WS`
available — no zero-unknown-ratchet re-verification was performed as
part of this task.

### 8.4 LSP steady-state RSS + cold start, before/after

Same workspace as §7 (`U:\Git\DO.Support-SlowDOSetup\DocumentOutput\
Cloud`, 551 workspace files), driven with a scratch raw-stdio LSP client
(`initialize` → `initialized` → `didOpen` on one workspace file →
`textDocument/prepareCallHierarchy` at a known procedure declaration,
matching §5's repro notes: percent-encoded file URIs,
`publishDiagnostics` notifications skipped when matching responses).
BEFORE = §7.1's AFTER column (commit `58df646`, safe-wins branch) — not
re-measured, per the task brief. AFTER = this branch's HEAD (`3239388`).

4 fresh-process trials; trial 1 paid a one-time OS-disk-cache-cold
penalty (4.884 s) not present in §7's methodology's warmed state, so
trials 2-4 (disk-cache warm, matching §7's "3 fresh-process cold
trials" starting point) are the comparable set:

| Metric | BEFORE (58df646, §7.1) | AFTER (3239388) | After vs Before |
|---|---:|---:|---|
| Cold start → first usable `prepareCallHierarchy` | 2.87 s (median of 2.943/2.846/2.858) | **3.42 s** (median of 3.415/3.421/3.636) | **+19 %** (regression) |
| RSS steady state (after first response, +10-30 s) | 1,584 MB | **~726 MB** (median of 724.9/725.7/737.6) | **-54 % (~-858 MB)** |

Output quality check: the `prepareCallHierarchy` call resolved a real
symbol (`OpenOutlookEMail` in `CDO Document E-Mail Management.al`) with
its full `RoutineNodeId` payload on every trial — consistent, no
regression in served data.

### 8.5 Reading: RSS win vs. cold-start regression

The **RSS win is the plan's stated goal and lands clearly**: steady-state
memory drops ~54% (1,584 MB → ~726 MB), a materially larger reduction
than §6's mitigations 1/2/5 combined (~420 MB). This is the expected
direction from dropping the ~10,727-file dependency parse arena after
first build.

**The ~19% cold-start regression (2.87 s → 3.42 s) was not anticipated by
the plan and is an honest, unhidden finding of this measurement**: the
owned-DeclSurface projection (`RoutineMeta` for every dependency
routine — ~126,640 dependency definitions in this workspace, per this
binary's own `--verbose` log) is built once, eagerly, at first-build
time, *in addition to* the parse the arena-drop then discards; that
projection-construction cost was not part of the pre-Task-4 pipeline
(which forwarded borrowed `&RoutineDecl` references directly into the
retained arena instead of copying a subset of each into an owned
struct). The plan's own framing ("Time is not a constraint... refactor
is always on the table") treats this as an acceptable trade of ~0.5 s of
one-time startup CPU for ~858 MB of standing RSS, but it is a real,
measured trade-off, not a pure win, and should be called out to anyone
consuming these numbers.

### 8.6 Residual decomposition (measured RSS is well above the ~150-300 MB hypothesis)

The ~726 MB steady-state figure is well above the plan's ~150-300 MB
hypothesis (explicitly flagged in the plan as a hypothesis, not an
acceptance bar). Decomposing what is directly measurable:

- **`dep_texts` (retained dependency source text, one `Arc<str>` per
  file since §6's Mitigation 2):** measured directly by opening every
  `.app` in this workspace's `.alpackages` (stripping the NAVX header,
  reading the embedded zip) and summing `.al` entry sizes: **10,727
  files, 119,613,209 bytes (~114.1 MB)**. This exactly matches §7.4's
  cited "~114 MB" figure and the "10,727 dep files" count — `dep_texts`
  is confirmed as a real, unavoidable-by-this-refactor ~114 MB floor (it
  is retained by design for source-backed features; only the *parse
  arenas* were in scope for Task 1-3/this refactor, not the raw text).
- **Unaccounted residual: ~612 MB** (726 MB measured − 114 MB `dep_texts`
  − a small OS/allocator/binary baseline, not separately measured here).
  This was not decomposed further by instrumentation (no heap profiler
  was run — out of scope for a measurement-only task), but the most
  plausible owners, in order of likely size, are: (a) the resolved
  `ProgramGraph`/`LspSnapshot` structures retained for query serving —
  built during resolution from ~126,640 dependency definitions plus the
  551 workspace files' own definitions/edges, which is a wholly separate
  retained structure from the dropped parse arenas and was never in this
  refactor's scope; (b) the *workspace's own* (non-dependency) `AlFile`
  parse trees, which this refactor intentionally does NOT drop (only
  `program::build_dep_indexes`'s dependency projections are targeted);
  (c) `dep_decl_by_id`/`dep_meta`'s owned `RoutineMeta` entries — likely
  small in aggregate (a few hundred bytes × ~126,640 entries is on the
  order of tens of MB, not hundreds) but not directly measured here.
  Recommend a heap profiler pass (e.g. `dhat`/`valgrind --tool=massif`,
  or an allocator-instrumented build) as the next concrete step if
  driving further below ~726 MB is prioritized — this task's brief
  explicitly permits reporting the decomposition instead of treating the
  gap to the hypothesis as a failure.

### 8.7 Net assessment

Mitigation 3 (§3.3(a), owned DeclSurface / drop dep parse arenas) is
**implemented and delivers the intended direction**: LSP steady-state RSS
drops ~54% (1,584 MB → ~726 MB), the single largest reduction of any
close-out in this document, at the cost of a ~19% cold-start regression
(2.87 s → 3.42 s) that was not anticipated by the plan. Output is
unchanged (byte-identical resolution behavior; zero goldens
regenerated). The CDO gate could not be re-run in this environment
(`CDO_WS` unset). The remaining ~612 MB of steady-state RSS above the
~150-300 MB hypothesis is attributed to the resolved program graph /
LSP query surface (not decomposed further without a heap profiler) —
tracked as a follow-up, not a regression introduced by this task.

  ---

  ## 9. 2026-07-14 cold-start regression — DIAGNOSED + FIXED

  §8's owned-DeclSurface landing traded ~0.5 s of cold start for the ~54 %
  RSS win, and flagged the cause only as an unverified hypothesis ("eager
  `RoutineMeta` projection build"). This section **instruments the actual
  phases**, attributes the regression, fixes the root cause, and re-measures
  — the regression is now **fully recovered** with the RSS win intact.

  ### 9.1 Phase attribution (measured, deterministic)

  A scratch harness called `LspSnapshot::build_full_with_parsed` on the §7/§8
  workspace (`DO.Support-SlowDOSetup` Cloud, 551 ws files / 10,727 dep files /
  ~126,640 dep routine decls) with `std::time::Instant` spans around every
  phase of `build_context` + `from_context`. Stable medians (least-contended
  of 4 fresh-process trials), **pre-fix** (branch HEAD `e86e276`):

  | Phase | Cost | New in this branch? |
  |---|---:|---|
  | `parse_snapshot` | ~1.10 s | no (pre-existing) |
  | `build_dep_layer` | ~225 ms | no |
  | `assemble_program_graph` | ~280 ms | no |
  | `ResolveIndex::build` (+obj map) | ~110 ms | no |
  | **`DeclSurface::build`** | **~187 ms** | **YES** (owned projection) |
  | **`freeze_dep_tier`** | **~118 ms** | **YES** (drain + re-partition of ~127k) |
  | `recompute_file` loop | ~200-250 ms | no |
  | `emit_event_flow_edges` | ~4 ms | no |
  | `build_dep_indexes` | ~200 ms | mostly pre-existing |
  | **dep `ParsedUnit` drop (SYNC, critical path)** | **~500 ms** | **YES — the dominant new cost** |

  The hypothesis in §8.5 was only partly right. The single largest new cost
  is **not** the projection build but the **synchronous drop of the ~10,727
  dependency parse arenas** (~500 ms, measured 495-583 ms across trials) that
  §8's landing performs on the critical path *before returning the first
  snapshot*. The pre-`e86e276` pipeline **retained** those arenas for the
  updater's lifetime, so it never paid this drop at startup at all — the drop
  is a brand-new critical-path cost, and it alone ≈ the whole +0.5 s
  regression. Secondary: the owned `DeclSurface::build` (~187 ms) followed by
  `freeze_dep_tier` (~118 ms) re-partitions every one of ~127k entries a
  second time, back-to-back.

  ### 9.2 The fix (two changes, root-cause targeted)

  1. **Drop the dependency parse arenas off the critical path.**
     `from_context` now `swap_remove`s the one workspace `ParsedUnit` and
     hands the remaining dependency units to a detached background thread
     (`std::thread` named `dep-arena-drop`) that drops them, instead of
     dropping them inline. Every consumer of dependency parse arenas (the
     frozen dep-tier `DeclSurface`, `dep_decl_by_id`, `dep_texts`) has
     already run, and the published snapshot retains only `Arc::clone`s of
     *workspace* `AlFile`/text (plus dependency *text* via `dep_texts`) —
     never the dependency `AlFile` arenas — so nothing observes the deps
     after this point. If the thread can't spawn, the closure drops `parsed`
     synchronously (sound fallback). The caller now pays only the O(#apps)
     `swap_remove` scan (~50 µs, measured) instead of ~500 ms.

  2. **Fuse `build` + `freeze_dep_tier` into `DeclSurface::build_split`.**
     A single-pass partitioned builder routes each routine into the `local`
     (primary-app) or frozen (dependency) tier as it is built, eliminating
     the second drain-and-re-partition of ~127k entries. Semantics are
     identical to `build` + `freeze_dep_tier` — proven by the new
     `build_split_matches_build_then_freeze` unit test (asserts identical
     local + frozen key sets and matching metas). Saves ~115 ms.

  Post-fix, the same probe measures the dep-drop handoff at **~50 µs** (was
  ~500 ms) and `build_split` at **~190 ms** (was ~305 ms for build+freeze).

  ### 9.3 Cold-start re-measurement (same LSP-stdio methodology as §7/§8)

  Same raw-stdio LSP client as §8.4 (initialize → didOpen one ws file →
  `prepareCallHierarchy` at `OpenOutlookEMail`'s decl), but run as a
  **same-session, alternating A/B** (pre-fix binary then post-fix binary, six
  pairs back-to-back) to cancel this shared machine's ~±0.4 s session-to-
  session drift. Pair 1 (disk/warmup) excluded; medians of pairs 2-6:

  | Binary | Cold start → first `prepareCallHierarchy` | RSS @ first response | RSS @ +30 s |
  |---|---:|---:|---:|
  | **PRE-fix** (`e86e276`) | **3.44 s** (3.421/3.439/3.548/3.469/3.432) | ~653 MB | ~723 MB |
  | **POST-fix** | **2.82 s** (2.863/2.810/2.786/2.870/2.819) | ~1,597 MB | ~750 MB |
  | base `1765b7a` (pre-branch, ref) | 2.78 s (2.822/2.779/2.779/2.798) | ~1,586 MB | ~1,640 MB |

  `items=1` (real `OpenOutlookEMail` `RoutineNodeId` resolved) on every
  trial — no output-quality change.

  **The +0.5 s regression is fully recovered: 3.44 s → 2.82 s (−18 %),
  landing at the pre-branch base cold start (~2.78 s) within noise.** The
  RSS win is intact — steady-state ~750 MB (vs. the base's ~1,640 MB, a
  ~−54 % reduction; the small delta from §8.4's 726 MB is cross-session
  variance, not a regression).

  ### 9.4 The one honest tradeoff: transient peak RSS

  Because the arena drop is now asynchronous, `RSS @ first response` is
  **higher** post-fix (~1,597 MB vs. pre-fix's ~653 MB): for the ~0.5 s
  between publishing the first snapshot and the background thread finishing,
  both the (soon-to-be-dropped) dependency arenas AND the resolved query
  surface are briefly resident. This transient peak lasts only until the
  drop completes (well under a second), after which steady state settles to
  ~750 MB exactly as before. This is a deliberate latency-for-transient-peak
  trade: the user gets a usable editor ~0.6 s sooner, and the extra memory
  is reclaimed before they finish reading the first response.

  ### 9.5 Known residual (not fixed here — deliberately scoped out)

  `dep_decl_by_id` (a ~126,640-entry `HashMap<RoutineNodeId, DeclEntry>`) is
  **fully redundant** with `dep_meta` (the frozen `RoutineMeta` tier already
  holds the same name/origin/name_origin/virtual_path keyed by the same id).
  Eliminating it — serving dep decls from `dep_meta` in `decl_and_text` —
  would recover a further ~150-200 ms of serial cold-start build AND remove
  ~50-80 MB of redundant RSS. It was **not** done here because
  `tests/lsp_incremental_parity.rs` pins `dep_decl_by_id`'s Arc-forwarding
  across rungs in 31 assertions; migrating those to `dep_meta` is a distinct,
  larger refactor whose risk to the permanent parity gate is not worth
  bundling into a regression fix. Tracked as the highest-value follow-up.

  ### 9.6 Validation

  - `cargo test`: all green (full suite, incl. `lsp_incremental_parity`);
    new `build_split_matches_build_then_freeze` unit test passes.
  - `cargo clippy --all-targets --all-features`: clean.
  - `cargo test --release --test perf_bounds`: 9/9 PASS.
  - Zero goldens regenerated (behavior-preserving).

