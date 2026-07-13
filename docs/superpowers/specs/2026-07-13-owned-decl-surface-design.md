# Owned DeclSurface: drop dep IR arenas from LSP steady state

**Date:** 2026-07-13
**Status:** Approved design, pending implementation plan
**Supersedes:** `docs/perf-regression-t3-vs-0.9.3.md` §3.3(a) as literally written
(see "Why the doc's literal §3.3(a) is insufficient" below)

## Problem

After the perf safe-wins branch (Arc-shared text + single parse + manifest-first
dedup), the LSP server still retains every dependency file's parsed `AlFile`
arena (~10,700 files, ~99 MB Base App alone — the perf doc's §3.1 item A1) in
`Updater.parsed: Vec<ParsedUnit>` for its entire lifetime. Steady-state LSP RSS
remains ~2 GB against the ~82 MB v0.9.3 baseline.

The retention driver is `BodyMap<'a>` (`src/program/resolve/body_map.rs`):

- It maps `RoutineNodeId → (&'a RoutineDecl, virtual_path)` by **borrowing**
  every `ParsedUnit` (workspace AND deps).
- Rung-2 resolution reads dep `RoutineDecl`s through it — candidate parameter
  metadata for overload/arg dispatch (`resolver.rs` ~551: "BodyMap FIRST,
  ABI-aware fallback ONLY on BodyMap miss") and witness spans
  (`resolve_routine_target`, `resolver.rs` ~137). A source-tier BodyMap miss
  is deliberately surfaced as `Unknown` (`resolver.rs` ~192) — so simply
  dropping dep parses would turn every workspace→dep edge into an `Unknown`
  and destroy the north-star metric.
- Its borrowed lifetime also forces a full `BodyMap::build` over ALL parses
  (~200–350 ms on CDO scale) on **every rung-1 save** — a documented
  architectural frustration (`src/lsp/updater.rs` module doc, ~lines 23–68:
  caching the map across rung-1 calls is impossible because the borrow chain
  would be invalidated by `splice_file`).

### Why the doc's literal §3.3(a) is insufficient

§3.3(a) proposes forwarding `dep_decl_by_id`/`dep_texts` across rung 2 so dep
parses need not be retained. But those two maps are only the *published query
surface*; rung-2 **resolution itself** additionally reads dep `RoutineDecl`s
through `BodyMap` (param metadata + spans). Forwarding the two maps alone
would starve dispatch of dep param metadata. The complete fix is §3.3(a) +
§3.3(b) combined: an owned, compact per-routine projection of exactly what
resolution reads, after which the parse trees can be dropped.

### Key exploration findings the design relies on

- **`AppRef` indices are already stable across rungs 1/2.** The `DepLayer`
  carries one `AppRegistry` (all apps interned once); `assemble_program_graph`
  **clones** it into every assembled `ProgramGraph` (`src/program/node.rs`
  ~16–33). Only rung 3 builds a fresh `DepLayer`/registry, and rung 3 rebuilds
  everything from disk anyway. So `RoutineNodeId` keys frozen at startup stay
  valid across rung-1/rung-2 rebuilds without any identity redesign.
- Rung 2 does **not** re-walk dep bodies: obligations are extracted from
  workspace files only (`recompute_file` over the primary unit;
  `emit_event_flow_edges` reads the graph + index + decl lookups).
- `dep_texts` values are already owned `Arc<str>` (perf safe-wins Task 1) and
  `DeclEntry` is fully owned — only `BodyMap` is lifetime-infected.

## Design

Replace the borrowed `BodyMap<'a>` with a fully-owned **`DeclSurface`**, then
drop dependency `ParsedUnit`s after the first full build.

### Core data model

```text
RoutineMeta                        // owned projection of one routine decl
  = exactly the fields resolution reads through BodyMap today:
    - parameter metadata (what candidate_param_infos / arg dispatch reads)
    - origin + name_origin spans (witness / DeclEntry construction)
    - virtual_path
    - (whatever else the compile-enforced audit surfaces — see Testing)
  NOT the routine body. Compact: decl-sized, not arena-sized.

DeclSurface                        // replaces BodyMap<'a>; no lifetime param
  - workspace tier: HashMap<RoutineNodeId, RoutineMeta>
      rebuilt from workspace parses per rung (small; per-file splicing is a
      possible later optimization, not required here)
  - dep tier: Arc<HashMap<RoutineNodeId, RoutineMeta>>
      built ONCE at startup / rung 3, frozen, forwarded by Arc::clone
  - lookup: workspace tier first, then dep tier (a RoutineNodeId's app
    disambiguates; the two tiers are disjoint by construction)
```

`BodyMap<'a>` is **deleted**. Every consumer (`resolver.rs`, `receiver.rs`,
`arg_dispatch.rs`, `full.rs`, `stub.rs`, `differential.rs`,
`lsp/snapshot.rs`, `lsp/updater.rs`, and all their tests) retypes to
`&DeclSurface`. Constructor keeps the `build(graph, parsed)` shape so test
migration is mechanical.

**Stop-and-reassess rule:** if the retyping audit finds any resolution path
that genuinely reads a dep routine **body** through BodyMap, implementation
stops and the design is revisited (exploration indicates none does — body
walks live in `extract.rs` against parsed files directly).

### Lifecycle

- **Startup (`build_full_with_parsed`):** parse everything (unchanged) →
  build graph → build `DeclSurface` (both tiers) → resolve → build
  `LspSnapshot` → **drop dep `ParsedUnit`s**. The updater receives only the
  workspace unit + the frozen dep tier. Cold-start peak still pays the parse;
  steady state does not.
- **Rung 1 (body edit):** workspace tier rebuilt from workspace-only
  `self.parsed`; dep tier `Arc::clone`. (Side-effect win: the ~200–350 ms
  all-units `BodyMap::build` per save disappears.)
- **Rung 2 (signature change):** fresh graph assembled from the cloned
  `DepLayer` (AppRefs stable) → workspace tier rebuilt; dep tier,
  `dep_decl_by_id`, `dep_texts` all `Arc::clone` forwarded. The "would
  dangle" recompute at `updater.rs` ~469–476 is deleted.
- **Rung 3 (deps changed):** full rebuild from disk (unchanged semantics) —
  new `DepLayer`, new frozen dep tier; fail-closed on error (keep serving old
  snapshot + old dep tier).
- **CLI / aldump / `resolve_full_program`:** builds the `DeclSurface`
  in-scope exactly where it builds `BodyMap` today; parses stay alive for the
  process's short lifetime as before. Behavior and memory profile unchanged.

### Invariants preserved

1. **Source-tier surface miss stays `Unknown`** (`resolver.rs` ~192's
   integration-bug posture). No silent fallback is added.
2. **Last-write-wins duplicate-key semantics** of `BodyMap::build` (see
   `body_map.rs` insert doc) reproduced with the same iteration order, so
   ambiguous-sibling span behavior does not shift.
3. **Edge classification unchanged** — dep routines still resolve as
   source-tier hits (the surface hit replaces the BodyMap hit one-for-one);
   the `Histogram` taxonomy (`resolvedSource` vs `resolvedAbiExternal` etc.)
   must be byte-identical on every fixture and on CDO.
4. **Dep tier immutable post-freeze** (`Arc`, no interior mutability) — same
   soundness argument as the Task-2 `Arc<AlFile>` sharing.
5. **Stale `ItemData` across rung 3** keeps today's fail-closed hashmap-miss
   behavior (out of scope to strengthen; noted as a pre-existing theoretical
   edge in the exploration).

### Rejected alternatives

- **ABI-ify embedded deps** (populate `abi_params` on dep `RoutineNode`s and
  ride the existing BodyMap-miss → ABI fallback): shifts dep edges' evidence
  class from Source toward ABI, perturbing the north-star histogram taxonomy.
  Rejected — distorts the metric's meaning.
- **Literal §3.3(a) only** (forward the two published maps, keep arenas):
  does not release the memory; see above.
- **Per-dep-file artifact records** (§3.3(b) file-granular,
  `dep_artifact_l4`-style): same information, more moving parts, no extra
  benefit for this goal. Remains the natural shape for mitigation 4's
  persistent disk cache later; this design does not preclude it.

## Testing & acceptance

1. **Compile-enforced read-surface audit (Task 0):** deleting `BodyMap`
   forces every consumer through the compiler; `RoutineMeta`'s fields are
   derived from that audit, not guessed.
2. **Unit:** port `body_map.rs`'s test suite to `DeclSurface` with verbatim
   scenarios and expected values (duplicate names across objects, member
   triggers, last-write-wins, empty units).
3. **Behavioral parity:** full `cargo test` with **zero golden changes**
   (`REGEN_TEMP_GOLDENS` must not be needed); parity gate
   `tests/lsp_incremental_parity.rs` green.
4. **Drop-proof tests:** after `build_full_with_parsed`, updater state holds
   only the workspace `ParsedUnit`; dep tier `Arc::ptr_eq` across rung-1 and
   rung-2 snapshot swaps.
5. **Perf:** `cargo bench --bench lsp_pipeline` (rung 1/2 expected to
   improve); release-mode `tests/perf_bounds.rs` green.
6. **CDO gate:** `scripts/cdo-gate <CDO_WS>` — zero-unknown ratchet,
   `ambiguousResolved` pin, and coverage contract must hold. This is the
   tripwire for any field missing from `RoutineMeta`.
7. **Memory acceptance (the point of the exercise):** real stdio LSP session
   (perf doc §5 repro commands) against
   `U:\Git\DO.Support-SlowDOSetup\DocumentOutput\Cloud`, steady-state RSS
   before/after. Expectation per §3.3: from ~2,000 MB to ~150–300 MB
   (one shared text copy + graph + indexes). Measured numbers get appended to
   `docs/perf-regression-t3-vs-0.9.3.md` as a §7 close-out.
