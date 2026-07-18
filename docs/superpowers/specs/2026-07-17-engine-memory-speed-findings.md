# Engine memory + speed design review — findings (2026-07-17)

Status: FINAL (all phases complete; externally reviewed by gpt-5.6-sol +
gemini-3.1-pro-preview, dispositions in §6)

Mission: explain why `alsem analyze` on Microsoft Base Application 28.0 source
(8,020 `.al` files, one app, zero deps, 3 cheap detectors
d61/d62/d64) never finished in 10 minutes at 7+ GB RSS, then judge whether the
design that causes it is the right one. Two tracks: A = compat-preserving
refactors; B = compat-breaking architecture.

All measurements: `release-fast` build at master `45d06e4` (worktree
`design-engine-memory-speed`), Windows 11, 93.7 GB RAM, 2026-07-17.
Corpus: Base App 28.0.46665.48632 source extracted from the `.app` (120 MB,
8,020 files); slices are first-N-alphabetical basename copies of `src/`.
System Application 28.0 (1,309 files) is the mid-size reference.

## 1. Scaling curve (Phase 1a)

`alsem analyze <ws> --detector d61-…,d62-…,d64-… --format json`, peak RSS via
20 ms sampler (slices) / `PeakWorkingSet64` monitor (8020).

| files | wall s | peak RSS MB | MB/file |
|------:|-------:|------------:|--------:|
| 1000  | 12.2   | 699         | 0.70    |
| 2000  | 26.9   | 2,159       | 1.08    |
| 2674  | 35.8   | 2,866       | 1.07    |
| 5400  | 166.3  | 9,766       | 1.81    |
| 8020  | **DNF — killed at 2,400 s** | **35,797** | ≥4.5 |

(System Application 1,309 files: 14.7 s, 420 MB — healthy.)

Local log-log exponents — time: ~1.5 (1k→5.4k), **≥6.7** (5.4k→8k);
RSS: ~1.7 (2.7k→5.4k), **≥3.2** (5.4k→8k). A regime change, not smooth
superlinearity: quadratic-plus terms dominate once the corpus is dense enough.
120 MB of source becomes ≥35.8 GB resident — a ~300× blowup.

## 2. Per-stage attribution (Phase 1b)

Throwaway env-gated probes (`ALSEM_STAGE_TIMING=1`): stderr stage marks with
wall time + RSS (K32GetProcessMemoryInfo), placed in `gate/run.rs`,
`l3_workspace.rs`, `detector_context.rs`, `registry.rs`, `resolve/full.rs`.

### slice-2674 (49.5 s total, peak 2,866 MB)

| stage | s | ΔRSS MB | notes |
|-------|---:|--------:|-------|
| fresh_coverage preflight | 20.1 | +578 peak, drops to 26 | build_context 19.0 s |
| L3 assemble (read+parse+project) | 7.9 | +637 | parse accum 5.7 s SEQUENTIAL |
| L3 resolve | 1.4 | +104 (+560 transient) | |
| L4 symbol_table | 0.7 | +473 | |
| L4 resolve_calls+event+combined | 0.4 | +140 | |
| L4 cones | 2.2 | +526 (+150 transient) | |
| L4 summaries+indexes (clones) | 0.7 | +289 | |
| L4 transaction_spans | 1.2 | +12 | |
| L4 compute_summaries (2nd SCC+Jacobi) | 5.1 | +624 | |
| detectors d61+d62+d64 | 0.3 | ~0 | |
| projection/suppression/format tail | 7.1 | +466 | |

### slice-5400 (235.8 s instrumented, peak 9,767 MB)

| stage | s | ΔRSS MB |
|-------|---:|--------:|
| fresh preflight | 39.6 | transient 1,171, drops to 32 |
| L3 assemble | 15.3 | +1,265 |
| L3 resolve | 2.5 | +167 (+1,100 transient) |
| L4 symbol_table | 1.4 | +980 |
| L4 calls/event/combined | 0.9 | +290 |
| L4 cones | 12.0 | +2,179 (peak 5,395) |
| L4 summaries+indexes | 4.4 | +1,330 |
| L4 transaction_spans | 13.2 | +55 |
| **L4 compute_summaries** | **116.1** | **+3,307** |
| detectors | 0.8 | ~0 |
| tail | 20.3 | — |

### 8020 (instrumented, in progress at draft time)

| stage | s | RSS at end MB |
|-------|---:|--------:|
| fresh preflight | 61.9 | transient 1,821, drops to 37 |
| L3 assemble+resolve | 36 | 4,470 total after symbol_table |
| L4 graph stages | 2 | 4,961 |
| L4 cones | 63 | 16,423 (**+11.4 GB**) |
| L4 summaries+indexes | 31 | 21,823 (**+5.4 GB**) |
| L4 transaction_spans | **198** | 22,184 |
| L4 compute_summaries | **>5,000 — still inside the block when the 90-min cap killed the run** | plateau 35,663 (peak) |

The instrumented 8020 run entered the compute_summaries block at t=392 s and
was still inside it at the 5,400 s kill — >83 min in one stage, RSS flat at
35.7 GB from ~t=2000 s on (pure CPU). Two independent 8020 runs (uninstrumented
40-min cap, instrumented 90-min cap) both died in this block.

Time bombs at scale, in order: `compute_summaries` (5.1 s → 116 s → **>5,000 s
at 8k, never completed** — the regime change), `transaction_spans`
(1.2 → 13.2 → 198 s), cones (2.2 → 12 → 63 s).
Memory bombs: cones (+11.4 GB at 8k), FullRoutineSummary clone pass (+5.4 GB),
compute_summaries (+6+ GB), L3 model (~4.5 GB — linear but heavy,
~0.55 MB/file).

### Inside the Jacobi block (accum probes, slice-5400)

Block total 115.5 s. Recursive-SCC Jacobi alone: snapshot deep-clone 5.9 s +
compose 31.8 s + **stable-projection + serialized-JSON fingerprint change test
49.3 s** = 87 s — three quarters of the block, spent on just 828 routines
living in recursive SCCs. The rest ≈ 28 s is the 50k singleton compose passes
plus the surrounding index builds. `span_backward_bfs` accum = 0.09 s — the
13 s/198 s transaction-spans stage is per-op re-AGGREGATION over the visited
sets, not the BFS itself.

### The regime change is SCC growth

`SCCSTATS` (second Tarjan, identical graph to the cone pass):

| corpus | routine nodes | recursive SCCs | recursive members | max SCC |
|-------:|------:|---:|-----:|----:|
| 5400   | 60,550  | 236 | 828   | **84**  |
| 8020   | 100,941 | 342 | 1,764 | **846** |

Between 5.4k and 8k files the corpus becomes dense enough that a 846-member
SCC forms (10× the 5400 maximum). Jacobi cost ≈ iterations(≈diameter) ×
members × summary-size, all three of which grow with SCC size — a ~100×
step on the biggest SCC alone, matching the observed >65 min block. The
iteration cap is 1000 (`summary_runner.rs:34`), so a non-converging giant SCC
costs up to 1000 × K compose+project+serialize passes before the cap-hit
diagnostic fires.

The fresh program-engine preflight handles the SAME corpus in ~62 s / 1.8 GB
transient and frees it (spec §3 sequencing works). The L3/L4/L5 substrate is
the entire problem.

## 3. Hypothesis verdicts (Phase 2)

Method: direct source reads + three read-only investigator agents (string-id
sweep; L2/L3 + FingerprintIndex; pipeline/ownership map) + the two external
models of §6. Items marked "investigator" carry their file:line evidence;
load-bearing claims were re-verified before adoption.

### H-1 string-id explosion — CONFIRMED (with one correction)
- Internal ids are long heap Strings: RoutineId =
  `"{modelInstanceId}/{sha256hex}"` = 81 bytes in production analyze (the gate
  model-instance id is 16 chars — `gate/model_instance_id.rs:95`; the ~67-byte
  figure applies to the `r0` default) (`src/engine/ids.rs:192-194`),
  object id `"{guid}/{type}/{num}"` ≈ 50 (`ids.rs:84-86`); stable routine id =
  `"{stableObjectId}#{sha256hex}"` ≈ 115 (`ids.rs:197-202`).
- Cloned into every L3Routine (id, stable_routine_id, object_id — l3_workspace.rs:317-440),
  every CallEdge (from/to/callsite_id/operation_id — `call_resolver.rs:109-132`),
  every cone map key, every summary map key, every anchor
  (`PAnchor.source_unit_id` + `syntax_kind` Strings per anchor —
  `l2/features.rs:33-45`; anchors sit on every call site, operation site,
  statement-tree node, condition ref, var assignment).
- Child ids EMBED the full routine id as a literal prefix: op/callsite/loop
  ids are `"{routine_id}/op{N}"`/`"/cs{N}"` (`l2/ir_walk.rs:836/908/1135`), so
  the 64-hex hash is re-copied into every child id; each callsite id then
  lives ~3× in L2 alone (id→expr maps + statement-tree CFN leaves,
  `ir_walk.rs:840/851/910/1290/1343`) before `features.call_sites.clone()`
  re-allocates it into L3. `CallEdge::base` mints 3-4 fresh id Strings per
  edge (`call_resolver.rs:135-140`). Investigator estimate: ~90-110 heap id
  Strings of 60-110 B for ONE 10-callsite routine through L2+L3 alone; L5
  adds 1,233 `.clone()` sites across 84 files. Per-callsite retained weight
  ≈ 1.5-2.5 KB, dominated by per-argument-binding `PAnchor`s (each owning a
  `source_unit_id` String).
- CORRECTION to the prompt's premise: `string-interner` is a DEAD dependency —
  declared (`Cargo.toml:29`) but zero `string_interner::` imports repo-wide
  (leftover of the deleted legacy LSP pipeline). The fresh program engine does
  its own interning of APPS only (`AppRegistry` → `AppRef(u32)`,
  `src/program/node.rs:10-23`) and avoids the blowup via COMPACT STRUCTURED
  identity (`ObjectNodeId` = AppRef + enum + i64, `RoutineNodeId` adds one
  short name String + numeric sig_fp — `node.rs:117/174`), not interning. The
  L2/L3/L4/L5 engine interns NOTHING. CLAUDE.md's "Core Patterns: String
  interning (string-interner): All symbol names deduplicated" is STALE —
  follow-up: correct it.

### H-2 per-detector FingerprintIndex rebuild — CONFIRMED but NOT the blowup
- `FingerprintIndex::build` (`l5/fingerprint.rs:64-85`): 2 borrow maps + a
  stable map cloning `r.id` AND `r.stable_routine_id` per routine. ALL 54
  registered detectors call it at the top of their `detect_dNN` (plus
  `gate/policy/policy_engine.rs:155`); `DetectorContext` carries NO
  fingerprint index (`detector_context.rs:55-175`), so nothing shares it —
  on CDO-scale (~18k routines) a full-detector run makes ~1.9M avoidable
  String allocations. Fix is a one-liner-shaped hoist (build once in ctx).
- Investigator sweep also confirmed this is the ONLY whole-workspace
  per-detector rebuild (no detector re-runs event/reverse/symbol/summary
  builds; remaining per-detector maps are routine-local). One sibling waste:
  `build_cross_extension_subscribers` is recomputed 3× per run (d43.rs:382,
  d44.rs:59, d45.rs:51 — all DEFAULT detectors) instead of living in ctx.
- With 3 selected detectors the loop cost is negligible (0.3-0.8 s total).
  O(detectors × workspace) is real waste for 40+ detector runs but irrelevant
  to the Base-App DNF.

### H-3 L2 PFeatures weight + duplication into L3 — CONFIRMED (linear but heavy)
- In the analyze path there is no separate L2 workspace: `project_file`
  (`l3_workspace.rs:553`) parses and projects straight into `L3Routine`, which
  carries ~12 Vec fields of String-bearing structs (call_sites with
  argument_texts + argument_infos + argument_bindings, operation_sites,
  statement_tree, identifier_references, condition_references, var_assignments,
  …: `l3_workspace.rs:317-440`).
- Resident L3 model: ~640 MB at 2,674 files, ~4.5 GB at 8,020 — linear,
  dominated by per-site Strings + anchors.
- Nuance (investigator-verified): the per-routine L2 `features` value is a
  TRANSIENT local dropped each loop iteration (`l3_workspace.rs:762-785`) —
  there is no persisted parallel L2 workspace, so the body-walk data lives
  ONCE (in L3Routine), not twice. But the assembly `.clone()`s
  (`:894/:925/:1021-1031`) are avoidable outright: `features` is owned and
  the only post-clone read is a DIFFERENT field (`features.variables`,
  `:969`), so disjoint partial MOVES are legal — free allocation win.
- The assembly parse loop is SEQUENTIAL on one big-stack thread
  (`l3_workspace.rs:1185-1198`) — 5.7 s/11.0 s accum parse at 2.7k/5.4k while
  the fresh engine parses the same files with rayon `par_iter`
  (`src/snapshot/parse.rs:85-108`).

### H-4 event-graph pair costs — FALSIFIED (as a major contributor)
- `build_event_graph` (`l3/event_graph.rs:243-360`) is linear: one pass over
  publishers, one over subscribers, one edge per subscriber; no
  publishers×subscribers product. 0.1 s / +40 MB at 8k.

### H-5 L4 cones / witness residual — CONFIRMED, and WORSE than suspected
- `compose_cone_over_graph` (`l4/capability_cone.rs:1754`) →
  `compose_inherited_cones` (`:1646`): per-routine `inherited:
  Vec<CapabilityFact>` = full transitive fact cone; coverage cone
  `unknown_targets` clones every incomplete downstream routine-id String into
  EVERY ancestor (`:1624-1626`, `:1708-1709`). O(routines × cone) memory —
  +11.4 GB at 8k.
- Recursive-SCC members each run a BFS over their SCC siblings that PULLS (not
  recurses into) each downstream SCC's whole precomputed cone, cloning cone
  reps per key (`:1687-1688` → `inherited_facts_by_bfs` `:1461-1525`) —
  O(K × (K + Σ downstream-cone-sizes)) per SCC of size K, bounded by the cone
  pull, not a full-graph walk (external-review correction adopted).
- `FullRoutineSummary` then CLONES every cone's inherited facts + direct facts
  again (`l5/detector_context.rs:237-251`) — +5.4 GB more.
- Per-fact String/JSON churn inside cone merging: `rep_key` builds a
  `§`-joined String with `serde_json::to_string` of ValueSource + extra on
  every merge tie-break and direct-dedup comparison
  (`capability_cone.rs:1188/:1223/:1318`), and `inherited_output_sort_key`
  allocates a String per fact for sorting (`:1372`).
- Scope clarification (investigator): L4 is already flow-insensitive — cones
  and summaries operate on the SCC condensation with deduped fact SETS; there
  is no per-witness path enumeration here. The witness-perf arc's deferred
  "§7 flow-insensitive" residual is an L5 path-walker concern, distinct from
  these L4 costs.
- `compute_summaries` (the SECOND Tarjan + Jacobi pass — recomputed for
  uncertainties, `parameter_roles_by_routine` (d3/d17/d37/d39/d40/…), and the
  cap-hit diagnostics — `detector_context.rs:363-426`,
  `l4/summary_runner.rs:817+`): every Jacobi iteration DEEP-CLONES all
  in-progress SCC summaries (`summary_runner.rs:1067`) and, per member per
  iteration, builds a full stable-id PROJECTION of prev+next summaries and
  compares their serialized-JSON fingerprint strings
  (`:1091-1097` → `stable_summary_fingerprint`, `l4/summary.rs:580-600` — a
  JSON-string build, NOT a SHA hash; the cost is projection + allocation +
  serialization). 116 s at 5.4k; the 8k regime change lives here.
- HIDDEN GLOBAL SCAN inside the loop (found by the external review, verified):
  every `compose_routine` call scans the ENTIRE global
  `graph.uncertainty_edges` list and filters by `ue.from`
  (`summary_runner.rs:483-486`) — an O(members × iterations ×
  global-uncertainty-edges) term. The 5.4k slice already reports 10,169
  unknown resolution edges (the 8k count was never reached — run killed), so
  this term alone is plausibly the bulk of the 66+ min block. Same per-call
  rebuild pattern in `cfg_walker` per-parameter indexes.
- NEW (not in the prompt's suspect list): `compute_transaction_spans`
  (`l5/transaction_spans.rs:175+`) runs a full backward-cone BFS PER COMMIT
  OPERATION and stores `routines_in_span: Vec<String>` per span
  (`:204-221`) — O(commit-sites × graph) time and memory. 198 s at 8k.

### H-6 double substrate — CONFIRMED but SEQUENTIAL; actually a TRIPLE parse
- `run_analyze_with_exit` runs `fresh_coverage` first (`gate/run.rs:204`);
  the fresh ctx (snapshot + parsed IR + graph) peaks 1.8 GB at 8k and drops
  to ~37 MB before L3 assembly starts (`resolve/full.rs:1249-1261` — spec §3
  sequencing holds). Zero resident overlap. Not a memory bomb; a wall-time
  tax that a shared substrate would refund.
- Investigator sweep sharpened the count: the workspace is parsed THREE times
  per analyze — #1 `parse_snapshot` (whole program incl. dependency source,
  `full.rs:1096`), #2 L3 `project_file` (`l3_workspace.rs:561`), #3
  `compute_workspace_diagnostics` re-parses every workspace file just to test
  `.objects.is_empty()` (`gate/workspace_diagnostics.rs:119`) — plus two more
  non-parse disk passes (inline suppression re-reads finding-bearing files,
  `gate/inline_suppression.rs:187`; `project_coverage_disk`). And
  `SymbolTable::build` runs TWICE (`l3_workspace.rs:1402` in `resolve()`,
  again in `detector_context.rs:203`). The whole fresh model — including a
  full dependency-source resolve — survives only as ~4 scalars + an
  opaque-apps list (`FreshCoverage`).

## 4. What is actually resident at peak (structural)

At 8k: ~4.5 GB L3 model (linear) + ~0.5 GB symbol table/graphs + ~11.4 GB
cones + ~5.4 GB summary clones + ~6+ GB compute_summaries working set
+ spans/indexes — i.e. >75 % of peak RSS is the L4 inherited-cone/summary
substrate, all of it String-keyed and clone-multiplied, none of it consumed by
the 3 selected detectors beyond map lookups.

Ownership map (investigator-verified): `DetectorContext` BORROWS `&L3Resolved`
(`routine_by_id`/`table_by_id`/`call_site_by_id` hold references into
`resolved.workspace`, `detector_context.rs:254-262`), so the full L3 model
cannot be freed for the whole L4+L5 phase. The derived layers are CLONED
rather than moved at every hand-off — cone→`summaries`
(`c.inherited.clone()`), `calls`→edge-index + bindings clones,
`core_summaries`→`parameter_roles`/`uncertainties_by_node` clones with the
full `core_summaries` materialized just to copy two fields out — and during
the build the source AND its cloned subset are transiently co-resident
(`calls` + its clones; `cones` + `summaries`), which is the RSS spike between
the cone mark and ctx return.

## 5. Design evaluation (Phase 3)

Verdict per hot spot: ACCIDENTAL (fixable in place, Track A) vs ARCHITECTURAL
(the data model is wrong for scale, Track B).

### 5.1 Accidental waste (Track A candidates)

**A0 — the selected detectors never consume the substrate that kills the run
(external-review find, source-verified).** `build_detector_context` eagerly
builds cones, summaries, spans, closed-world temp, event-flow indexes and the
second Tarjan+Jacobi for EVERY run (`l5/registry.rs:237-238`), but d62 and d64
reference `ctx` ZERO times and d61 reads only `ctx.event_graph`,
`ctx.routine_by_id` and `ctx.resolved_call_edge_by_callsite` (verified by
grep over `l5/detectors/d61.rs`/`d62.rs`/`d64.rs`). The entire 66+ min /
~17 GB cone-summary-span construction is dead weight for the motivating
production case. Fix: per-detector substrate requirements (a declared
capability set per detector; context fields built lazily/OnceLock on first
consumer). Contract question to settle: summarize cap-hit diagnostics are
currently emitted unconditionally — decide whether they are part of the
always-on output contract (if yes, keep a cheap path for them or scope them
to runs that build summaries).

**A1 — transaction_spans: compute the span ONCE per commit ROUTINE.**
`backward_cone(commit_routine_id, …)`, `aggregate_span` and `span_roots_of`
depend only on the routine, yet all three run per commit OPERATION
(`l5/transaction_spans.rs:204-221`). Accum probes show the BFS itself is
0.09 s at 5.4k — the 13 s/198 s cost is the per-op re-aggregation over the
visited set (each span walking every visited routine's summary facts) plus
`routines_in_span: Vec<String>` clones. Hoist per routine, share the
aggregation; expected: 198 s → tens of seconds, spans memory shared.

**A2 — Jacobi loop: cheap change test + dirty-frontier rounds + indexed
scans.** The fixed-point change test builds a full stable-id projection of
prev+next and compares serialized-JSON fingerprint strings, per member, per
iteration (`summary_runner.rs:1091-1097`, `stable_summary_fingerprint`
`l4/summary.rs:580` — JSON string build, not a hash), and deep-clones the
whole in-progress map per iteration (`:1067`). Measured at 5.4k:
projection+fingerprint 49.3 s, clone 5.9 s, compose 31.8 s — the change test
costs MORE than the actual dataflow work. Fixes, all preserving SYNCHRONOUS
Jacobi semantics (the trajectory is load-bearing on cap-hit, so no
asynchronous worklist):
- non-allocating comparison (structural `PartialEq` on `RoutineSummary`
  (`l4/summary.rs:165`) or a cheap canonical key) — must first prove
  equivalence with the fingerprint, which deliberately OMITS some fields
  (trace/parity tests, DO/CDO differential);
- `std::mem::take(&mut in_progress)` for the snapshot instead of `.clone()`;
- round-preserving DIRTY-FRONTIER evaluation: in round k+1 recompute only
  members whose direct callee inputs changed in round k (unchanged members
  carry over) — full-pass semantics without full-pass cost;
- index `graph.uncertainty_edges` by `from` ONCE (the per-compose global scan
  at `summary_runner.rs:483-486` is O(members × iterations × global
  uncertainties) — likely the single biggest term at 8k), and hoist
  `cfg_walker`'s per-parameter per-call index rebuilds to per-routine.
On the 8k corpus's 846-member SCC this set is the difference between
finishing and the observed >66 min grind.

**A3 — kill the duplicate summary/cone clones.** `FullRoutineSummary` clones
every cone's `inherited` + direct facts (`detector_context.rs:237-251`,
+5.4 GB at 8k); `resolved_call_edge_by_callsite` clones every resolved
`CallEdge` (`:333-341`). MOVE out of the source maps where ownership allows
(`cones.remove`), Arc/indices only where sharing is genuine. Expected:
−5+ GB peak, zero output change.

**A4 — parallelize L3 assembly parse.** The per-file parse+project loop is
sequential on one big-stack thread (`l3_workspace.rs:1185-1198`) while the
fresh engine parses the same corpus with rayon (`snapshot/parse.rs:85-108`).
Deterministic merge: parse in parallel into per-file buckets, project in
sorted order. Expected: 36 s → ~8-10 s at 8k.

**A5 — FingerprintIndex once per run, not per detector**
(`l5/fingerprint.rs:64`, 54 call sites): move into `DetectorContext`. Matters
for 40-detector runs, not for the DNF.

**A6 — drop the dead `string-interner` dependency** (`Cargo.toml:29`) — hygiene
(and correct CLAUDE.md's stale "string interning" Core Patterns claim).

**A7 — L3 assembly: partial moves instead of clones.** The per-routine L2
`features` local is owned and dropped at iteration end; the `.clone()`s at
`l3_workspace.rs:894/925/1021-1031` can be disjoint field moves (only
`features.variables` is read afterwards, `:969`). Pure allocation win, zero
behavior change.

**A8 — hoist `build_cross_extension_subscribers` into `DetectorContext`** —
recomputed 3× per run by default detectors d43/d44/d45 (d43.rs:382, d44.rs:59,
d45.rs:51), same pattern as the existing shared `event_flow_indexes`.

**A9 — eliminate parse #3 + redundant disk passes.** `compute_workspace_
diagnostics` re-parses every workspace file to test emptiness
(`workspace_diagnostics.rs:119`); share the L3 assembly's parse products (or a
per-file object-count byproduct) instead. Similarly reuse discovery results
across the ≥4 disk passes.

Track A ceiling: A1-A5 plausibly turn the 8k DNF into a finishing run (the
regime-change terms are A1+A2's constants) and cut several GB (A3), but the
resident model stays O(routines × cone) in facts (cones' `inherited` Vecs +
`unknown_targets`) and O(total-Strings) in ids. On a 2× Base App corpus the
same wall returns.

### 5.2 Architectural (Track B candidates)

**B1 — interned symbol/id universe for the whole engine.** Replace String ids
(routine/object/callsite/operation/event) with u32 indexes into per-kind
arenas (the fresh engine already does this for apps — `AppRegistry`/`AppRef`,
`program/node.rs:10-23`). Cone keys, span sets, uncertainty targets become
`FixedBitSet`/roaring bitmaps over routine indexes. `unknown_targets` at 8k
stops being billions of String bytes and becomes ~1 KB/SCC. Output ids are
materialized ONLY at the projection boundary (SARIF/JSON/fingerprints), so
this is semantically Track A (goldens byte-stable) with Track-B-scale surgery.

**B2 — SCC-shared, lazily materialized cones.** Store ONE cone per SCC
(`Arc`-shared), per-routine views as (SCC cone + local delta) computed on
demand; never materialize per-routine `Vec<CapabilityFact>` for all routines.
Detectors touch a tiny fraction of routines' cones; the d61/d62/d64 run needs
almost none of the 11.4 GB it pays for.

**B3 — single-substrate unification (the strategic one).** The fresh program
engine already parses, builds and resolves the SAME 8k corpus in ~62 s /
1.8 GB transient as a THROWAWAY preflight, while the L3 model rebuilds
everything from its own sequential parse. Unify: one parse, one symbol table,
one resolver (the fresh clean-room one — CLAUDE.md already names it the ONLY
resolution engine), with L4/L5 features projected from the owned IR on top of
the fresh graph, and L3 retired from the analyze path. This also refunds the
double-parse tax (H-6) and gives detectors the moat-grade edges instead of
the L3 oracle's. Prior investigation deferred shared-parse on cost grounds;
the 8k evidence changes the calculus.

**B4 — streaming / bounded-memory detector execution.** With B1+B2, the
remaining resident set is the graph + summaries; per-object features
(`L3Routine.call_sites` etc., ~4.5 GB at 8k) can be projected per object
on demand (or in a bounded LRU) instead of held for the whole run.

Future features: B1/B2 enable parallel detector execution (immutable compact
ctx); B3 enables incremental analyze via the existing `run_one_scc` Salsa seam
and multi-app workspaces (fresh substrate already app-qualified); B4 enables
larger-than-RAM corpora.

## 6. External model review (Phase 4)

Both externals received the same file-based brief (profiling numbers, verified
facts, candidate designs, §5 questions) via the pi delegate CLI
(`--no-skills --no-prompt-templates --no-context-files`, thinking high) with
read access to the repo. Every load-bearing claim below was source-verified
before adoption.

### gpt-5.6-sol — adopted / rejected dispositions

ADOPTED (all source-verified):
1. **A0 (its biggest find): detector selection does not prune substrate
   construction.** d62/d64 use `ctx` zero times; d61 reads only
   `event_graph`/`routine_by_id`/`resolved_call_edge_by_callsite` — verified
   by grep. The DNF case never needed cones/summaries/spans. Promoted to
   W1.0, ranked first.
2. **Hidden global scan in `compose_routine`** — verified at
   `summary_runner.rs:483-486`: every compose scans ALL
   `graph.uncertainty_edges`. Adopted into W1.1 as the likely dominant 8k
   term. Same for `cfg_walker` per-parameter index rebuilds.
3. **Fingerprint correction**: `stable_summary_fingerprint` builds a
   serialized-JSON comparison string, NOT a SHA-256 (`l4/summary.rs:580-600`)
   — my draft was wrong; fixed throughout. Also its caution that the
   fingerprint deliberately omits fields, so a `PartialEq` swap needs an
   equivalence proof first.
4. **RoutineId is 81 bytes in production** (16-char gate model-instance id,
   `gate/model_instance_id.rs:95`), not ~67 — verified, corrected.
5. **BFS overstatement**: `inherited_facts_by_bfs` pulls downstream SCC cones
   without recursing through them — verified, H-5 text corrected.
6. **Second `compute_summaries` is not only for uncertainties** — it also
   feeds `parameter_roles_by_routine` (d3/d17/d37/d39/d40/…) and cap-hit
   diagnostics (`detector_context.rs:403-426`) — verified, corrected.
7. **Span fix must cache the full per-routine SpanTemplate** (aggregation +
   roots + vectors, and the synthetic checked-run seeds too), not just the
   BFS — adopted into W1.2 (matches our 0.09 s BFS measurement).
8. **Preserve synchronous Jacobi; use round-preserving dirty-frontier**, not
   an asynchronous worklist (cap-hit trajectory is load-bearing) — adopted
   into W1.1.
9. **`std::mem::take` / move-out-of-map instead of clone/Arc** where
   ownership allows — adopted into W1.1/W1.3.
10. **B1 staging by domain with lexical-rank preservation** (never compare
    raw intern ordinals where semantics compare strings) — adopted into B1.
11. **B2 reframed as shared fact content + per-routine provenance overlay**
    (per-SCC cones already exist transiently; members differ in
    subject/first-hop/tie-breaks) — adopted.
12. **B3 risk restated**: not fresh-resolver speed but full detector-feature
    parity (bindings, witness tie-breaking, anchors, ordering) — adopted;
    B3 gate = detector-feature inventory + dual-substrate parity harness.
13. Methodology cautions: RSS ≠ live bytes (allocator retention), slice
    composition bias (alphabetical prefixes change topology, not just size),
    instrumentation skew — recorded as caveats in §1/§2; the next-hour
    measurement it proposed (eager vs ablated byte-compare) IS the W1.0
    verification plan.
14. **SCC density unproven**: the 846-member SCC's internal edge count /
    edge-kind histogram not yet measured; a single over-approximated
    event/interface edge may have merged components. Recorded as the first
    follow-up measurement (see §8).

REJECTED / QUALIFIED:
- "IFDS/IDE probably overkill" — agreed, kept out.
- BDDs — deferred unless bitsets prove insufficient (its own position).
- Its suggestion that skipping `compute_summaries` might silently drop
  cap-hit diagnostics is treated as a CONTRACT DECISION, not a blocker
  (recorded in W1.0).

### gemini-3.1-pro-preview — adopted / rejected dispositions

Delivery note: two attempts at `--thinking high` wedged (60 and 45 min, zero
output, killed); `--thinking medium` was rejected by the provider (400
invalid_request_body); the DEFAULT-thinking attempt answered in ~8 min. Its
review is shallower than gpt-5.6-sol's but directionally aligned.

ADOPTED:
1. **Changed-bit at compose level** ("if next adds items absent from prev,
   flag changed — no hashes required") — consistent with W1.1's cheap change
   test; noted as the simplest correct mechanism.
2. **Roaring bitmaps over u32-interned routines for cones/unknown_targets**
   — independently converges with B1 (both externals; strengthens the B1
   ranking).
3. **Structural sharing (Arc/persistent structures) for cone lists** —
   recorded as an implementation OPTION inside B2 (shared content is the
   design; persistent vectors are one mechanism).
4. **B1-first among Track B; A-then-B overall, A2+A1 prioritized** — matches
   the ranked proposal.
5. **B3 parity risks made concrete**: extension-field merge, the G-5
   table-id-collision preference, coverage semantics — added to the B3
   feature-parity audit list.
6. Measurement suggestions (span-closure hit counts, projection profiling,
   heap profiler on the Jacobi loop) — overlap §8; the span hit-count idea is
   folded into follow-up 2.

REJECTED (with evidence):
- Its "verification" of the brief's Fact 4 — "followed by
  `project_summary_to_stable` and `summary_fingerprint` ... sha256: True" —
  repeats the brief's error: `summary_fingerprint` →
  `stable_summary_fingerprint` builds a serialized-JSON comparison string,
  no SHA anywhere on that path (`l4/summary.rs:580-600`). gpt-5.6-sol caught
  this; Gemini rubber-stamped it. A useful calibration datum: its
  "checked the repo" claims are not uniformly trustworthy.
- "35 GB forces system swapping even with 94 GB RAM" — unsupported; RSS was
  flat and the machine never swapped during the 8k runs.

## 7. Ranked proposal (Phase 5)

Ranking principle: kill the regime-change terms first (they decide whether an
8k+ corpus finishes AT ALL), then the resident-memory model (it decides the
corpus ceiling), then the strategic substrate unification (it decides what the
engine can become). Every item preserves resolution semantics; Wave 1 is
byte-stable on goldens, Wave 2 is output-stable but internally breaking, Wave
3 may change output shape (we own all consumers).

### Wave 1 — Track A: make analyze finish (spec-grade, ready for writing-plans)

**W1.0 demand-driven detector substrate (A0) — DO THIS FIRST.** Each detector
declares its required substrates (workspace features / resolved calls / event
graph / combined graph / cones / core summaries / parameter roles /
transaction spans / closed-world temp / event-flow indexes);
`build_detector_context` builds the UNION for the selected set, lazily
(OnceLock-backed fields — the pattern the context already uses for ordering
facts). For the motivating d61/d62/d64 run this skips cones + summary clones
+ spans + closed-world + the entire second Tarjan/Jacobi: the DNF becomes
~2-3 min at 8k (L3 assembly + graphs + detectors) at ~5 GB peak — WITHOUT
touching any algorithm. Verification: byte-compare full-format output
(json/sarif/pr-summary) between eager and demand-driven modes on DO + the
5400 slice, all-detector runs included; settle the cap-hit-diagnostic
contract (always-on vs summaries-consumers-only) explicitly in the plan.

**W1.1 Jacobi: cheap change test + dirty-frontier rounds + indexed scans**
(`l4/summary_runner.rs:1041-1130`). Preserve SYNCHRONOUS round semantics (the
trajectory is load-bearing on cap-hit — no asynchronous worklist):
- Index `graph.uncertainty_edges` by `from` once per compute (kills the
  per-compose global scan at `:483-486`, O(members × iterations × global
  uncertainties) — 10,169 unknown edges already at 5.4k; plausibly the
  biggest single term); hoist `cfg_walker` per-parameter index rebuilds to
  per-routine.
- Replace the projected-JSON-fingerprint change test (`:1091-1097`,
  49.3 s at 5.4k vs 31.8 s of actual compose) with a non-allocating
  comparison, after PROVING equivalence: the fingerprint deliberately omits
  fields, so either show the omitted fields are iteration-invariant or
  compare a matching canonical key. Harness: the `collect_trace` oracle + L4
  goldens + a DO/CDO differential.
- `std::mem::take` the snapshot instead of `.clone()` (`:1067`).
- Round-preserving DIRTY FRONTIER: in round k+1 recompute only members whose
  direct callee inputs changed in round k; carry unchanged members over.
  Full-pass semantics, localized cost.
- Keep `MAX_FIXED_POINT_ITERATIONS`; cap-hit diagnostic behavior preserved
  (fixture with a non-converging SCC).
Expected: block 116 s → ≈25-35 s at 5.4k; the 8k block collapses from >66 min
to minutes even when summaries ARE demanded.

**W1.2 transaction spans: one SpanTemplate per seed routine**
(`l5/transaction_spans.rs:204-221`): hoist BFS + `aggregate_span` +
`span_roots_of` + the shared vectors into a per-routine cached template
(covering BOTH explicit-commit and synthetic checked-run seeds); per-op spans
reference it. Byte-stable (spans are keyed/sorted downstream). Expected:
198 s → tens of s at 8k.

**W1.3 stop the double-materialization**: MOVE cone results into
`FullRoutineSummary` (`cones.remove(&r.id)` transfers `inherited` +
`coverage`; consume `direct_full` likewise — no Arc needed,
`l5/detector_context.rs:237-251`); stop cloning `CallEdge`s into
`resolved_call_edge_by_callsite` (`:333-341`, borrow or index). Expected:
−5.4 GB and −31 s at 8k.

**W1.4 parallel L3 parse** (`l3_workspace.rs:1176-1198`): rayon map per file
(parse + per-file projection into a local Vec) then merge in the SAME
name-sorted order. Determinism preserved by the merge order; `project_file`
touches only its own file's state. Expected: 36 s → ~10 s at 8k.

**W1.5 FingerprintIndex once per run** (`l5/fingerprint.rs:64`): build in
`DetectorContext`, pass by ref. Matters at 40+ detectors (~1.9M avoidable
String allocations on CDO-scale full runs; also cover
`gate/policy/policy_engine.rs:155`).

**W1.6 small-waste bundle**: A7 assembly partial moves, A8
`build_cross_extension_subscribers` hoist, A9 parse-#3/disk-pass
elimination — each independently byte-stable and mechanical.

Combined Wave-1 expectation at 8020: the motivating 3-detector run DNF →
~2-3 min / ~5 GB (W1.0 alone); a FULL-detector run DNF → single-digit
minutes, peak ~30 GB → ~24 GB (cones still resident when demanded). At 5400
full-detector: 236 s → ≈100 s.

### Wave 2 — B1+B2: fix the memory model (output-stable, internally breaking)

**B1 interned id universe.** One arena per id kind (routine/object/callsite/
operation/event/file); engine structs carry u32 newtypes; maps become
`Vec`-indexed; `unknown_targets`/span membership/visited sets become
`FixedBitSet`/roaring over routine indexes. String ids materialize only at the
projection/format boundary (fingerprints, SARIF, JSON) — goldens byte-stable.
Expected: L3 model 4.5 GB → ~1.5 GB; cones' id duplication gone; hash lookups
→ array indexing across L4/L5.

**B2 SCC-shared lazy cones.** The transient per-SCC `fact_cones`/`cov_cones`
already exist (`capability_cone.rs:1663-1724`) — the damage is done by
EXPANDING them into per-routine cloned vectors. Persist the SHARED FACT
CONTENT per SCC (interned fact ids + bitsets after B1) and make per-routine
`inherited` a VIEW (shared content + a compact per-routine provenance overlay
— members legitimately differ in subject, own-direct-fact exclusion,
first-hop retag and tie-breaks, so it's content that is shared, not the
final `CapabilityFact` objects). Materialize only where a detector reads.
Kills the O(routines × cone) resident term (+11.4 GB at 8k).
`unknown_targets` per SCC as bitset (B1); coverage records project on demand.

### Wave 3 — B3: single-substrate unification (strategic)

Retire the L3 model from the analyze path: one snapshot, one rayon parse, one
symbol table, the FRESH resolver's edges (already the product's moat; 62 s /
1.8 GB transient on this corpus), with L4/L5 features projected from the owned
IR onto the fresh graph. The preflight stops being a separate pass — analyze
IS the fresh pipeline plus detector layers. Unlocks: incremental analyze (the
`run_one_scc` Salsa seam), multi-app workspaces, no double parse, one identity
model (RoutineNodeId) end to end. Riskiest assumption: every field of
`L3Routine` the 40+ detectors consume (the BCQuality substrate list) has an
IR-derivable equivalent — needs a feature-parity audit before commitment.
Output shapes (internal ids in rootCauseKey / fingerprints) can be held stable
via the existing stable-id substitution layer, or intentionally re-baselined
(Track B license) — decide at plan time with a differential harness on DO/CDO
as the gate.

### Explicitly rejected
- Caching `FingerprintIndex` across runs, mmap tricks, arena allocators as
  FIRST moves: they optimize constants of a quadratic; the quadratic terms go
  first.
- Bounding/sampling cones for detectors (silent coverage loss — violates the
  no-silent-caps doctrine).
- Asynchronous worklist inside the Jacobi (cap-hit trajectory is
  load-bearing; dirty-frontier keeps synchronous semantics).
- IFDS/IDE reformulation and BDD fact sets — heavier machinery than the
  measured problem needs; revisit only if interned bitsets prove
  insufficient.

## 7b. Wave 1 outcome (measured 2026-07-18, branch `worktree-design-engine-memory-speed`)

All ten Wave-1 tasks landed (commits `9c0ee77..708f000`), goldens byte-stable
throughout, every task independently reviewed. Measured on the same corpora,
same machine, release-fast:

| Run | Before (Wave-1 base) | After Wave 1 |
|-----|---------------------:|-------------:|
| slice-5400, 3 detectors | 236 s / 9.8 GB | **57.9 s / 3.4 GB** |
| 8020, 3 detectors | **DNF (killed 90 min) / 35.8 GB** | **90.3 s / 6.1 GB** |
| 8020, full default set | DNF (killed 90 min) / 35.8 GB, died INSIDE the Jacobi block | **still DNF (killed at a 60-min cap) / 45.2 GB — but the bottleneck MOVED** |
| DO default set (regression) | ~7-11 s | 10.7 s / 1.6 GB (byte-identical output) |

The 3-detector production case is ~60× faster than its kill point and finishes
comfortably; the remaining 90 s at 8020 is dominated by the fresh preflight
(~60 s) — a Wave-3/B3 refund.

The full-default honest read: the pre-wave run died inside the
substrate (compute_summaries, RSS flat at 35.7 GB for 60+ min). The Wave-1 run
CLEARS the substrate — RSS trajectory shows the summaries phase passing
~23.6 GB at ~12 min, peaking 45.2 GB, then oscillating 30→36 GB — and spends
its remaining 40+ min inside the 54-detector L5 loop itself (the witness
path-walker over ~100k routines: the known, explicitly out-of-Wave-1
"flow-insensitive §7" residual named in H-D's scope clarification, plus
detector working sets never before reached at this corpus size). The peak
being HIGHER than pre-wave is survivorship: phases the old run never reached
now allocate on top of the still-materialized cones/summaries. Wave 2 (B1/B2)
owns the resident-memory model; the L5 detector-loop wall at Base-App scale is
its own follow-up (profile the detector loop per-detector before designing —
same doctrine as this review).

Decision (a) held: the only output change in the wave is the absent summarize
cap-hit diagnostics on substrate-skipping selections.

## 8. Measurement caveats + follow-ups

Caveats on the numbers above:
- Slices are alphabetical prefixes — composition changes topology, not just
  size (the 846-SCC forms only in the full corpus). The curve is honest about
  THIS corpus family, not a universal exponent.
- Peak RSS ≠ live bytes (allocator retention, Vec/HashMap capacity slack);
  per-stage ΔRSS attributes allocation pressure, not exact ownership.
- Stage probes add per-member `Instant::now()` costs inside the Jacobi loop
  in the accum-probed build only; the DNF reproduces identically without any
  probes (first 8020 run was uninstrumented).

Follow-up measurements (next session, cheap):
1. 846-SCC anatomy: internal edge count, edge-kind histogram (call vs event
   vs interface), which edge kinds merged formerly-separate components —
   decides whether SCC-merging over-approximation is itself a fixable
   precision bug (one spurious event/interface edge can fuse components).
2. Jacobi telemetry on the big SCC: passes completed, changed-members per
   pass, summary cardinalities — sizes the dirty-frontier win precisely.
3. The W1.0 ablation byte-compare (eager vs demand-driven ctx) on the 5400
   slice — the acceptance gate for Wave 1's first task.
