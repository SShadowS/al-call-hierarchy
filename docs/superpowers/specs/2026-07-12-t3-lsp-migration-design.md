# T3 — LSP surface migration onto the program engine (design spec)

- **Date:** 2026-07-12
- **Status:** Approved design (brainstorm complete; next step: implementation plan)
- **Decision:** Migrate the LSP surface onto `src/program/resolve/` + the al-syntax IR
  and DELETE the legacy `graph.rs`/`indexer.rs`/`parser.rs` pipeline. True per-file
  incremental updates via a two-rung soundness ladder. One cutover arc, all features,
  adjudicated differential parity gate before deletion.
- **Parent plan:** `docs/superpowers/plans/2026-07-10-deep-review-remediation.md` (Tier 3)

## 1. Context

Four verified HIGH findings and several mediums all live in one place — the legacy LSP
pipeline — and are structural to it, not independent bugs:

- **H-10:** `graph.rs` `remove_file` (`graph.rs:762-832`) deletes entire
  `incoming_calls` entries per defined qname (containing OTHER files' live call sites);
  reindex re-links only the saved file's own calls; no repair pass. One whitespace save
  permanently destroys cross-file incoming edges.
- **H-11:** the pipeline interns identifiers case-sensitively (`graph.rs:443-445`) in a
  case-insensitive language — invisible edges, false "unused".
- **H-12:** positions are UTF-8 byte columns served as (LSP-default) UTF-16;
  `position_encoding` never negotiated; no converter anywhere on the LSP path.
- **H-13:** `path_to_uri` (`protocol.rs:52-85`) hand-encodes a ~5-char subset;
  non-ASCII paths (e.g. `…/Løsninger/`) produce broken URIs; decode side
  percent-decodes everything (asymmetric).
- **Mediums:** diagnostics published once at startup, never refreshed or cleared;
  watcher overflow events dropped; deps frozen at startup; no debounce (didSave +
  watcher double-reindex per save); reindex parses while holding both locks.

Meanwhile `src/program/resolve/` (the fresh whole-program resolver, 0.0000%
real-unknown on CDO) already supersedes the legacy engine for the CLI/aldump path.
The LSP surface is the last consumer of the old engine.

### Measured facts driving the decision (2026-07-12, release build, dev machine)

- Full engine pipeline (`aldump --program-call-graph-stats`) on CDO (551 workspace
  files + 20 deps, 43,375 whole-program edges): **~5.3–5.6s wall-clock**, warm
  dependency-source cache. Output SHA-256 byte-matches the frozen baseline
  `0a3b85bc832ff0a3e77acee118d203edbf62827dc37617c8d9315fe52d5cb7d0`.
- `RoutineNodeId` is **content-addressed** (`src/program/node.rs:132`): object identity
  + `name_lc` + `enclosing_member_lc` + `params_count` + `sig_fp`. Editing one file
  does not renumber other files' nodes — the property that makes sound incrementality
  feasible.
- The engine is case-insensitive end-to-end (`name_lc` keys throughout) — H-11 is
  structurally absent.
- `RoutineDecl` carries both `origin` (whole decl) and `name_origin`
  (`crates/al-syntax/src/ir/decl.rs:133-136`, doc comment: LSP selection_range) with
  line/col points. `BodyMap` maps `RoutineNodeId → (&RoutineDecl, virtual_path)`.
- Every resolved workspace edge carries a caller-site `CanonicalSpan` (0-based line,
  0-based UTF-8 **byte** column) + `RouteTarget::Routine(NodeId)`. No incoming index
  exists — one O(E) pass builds it.
- Phase-1 (Call/Run/ImplicitTrigger) resolves **workspace-caller** obligations only
  (targets anywhere incl. deps); Phase-2 (EventFlow) covers all apps' publishers. The
  workspace-caller scope is exactly the scope the legacy LSP served.
- Legacy server realities that lower the bar: `didSave`-only sync
  (`text_document_sync.change=NONE`; unsaved buffers invisible today), single-threaded
  request loop, diagnostics push-once-stale. Legacy is instant-but-corrupt after every
  save (H-10); a briefly-stale-but-correct snapshot strictly dominates.

## 2. Decisions (user-approved, 2026-07-12)

| # | Question | Decision |
|---|----------|----------|
| Q1 | Migrate vs patch | **Migrate**; delete legacy graph/indexer/parser path |
| Q2 | Incremental model | **True per-file incremental** — two-rung soundness ladder + permanent incremental-vs-batch differential gate |
| Q3 | Position encoding | **Negotiate both**: serve utf-8 natively when client offers it; else utf-16 via real converter |
| Q4 | v1 scope | **One cutover arc**, ALL existing features mapped, legacy deleted at arc end; no user-facing flag |
| Q5 | Parity story | **Adjudicated differential harness** (scaffolding on the branch, deleted with legacy); permanent = new-handler goldens + incremental-vs-batch gate + frozen CDO SHA |
| Q6 | Buffer sync | **Save-only v1** (didSave + watcher, today's semantics); didChange overlays designed-for, deferred |

## 3. Architecture: layered snapshot, swap-only publication

New `src/lsp/` backend module behind the existing `server.rs` loop.

Core state is an immutable snapshot, published by atomic `Arc` swap, never mutated
in place:

```text
LspSnapshot {
  program:        ProgramGraph        // layered: dep layer (immutable between
                                      // .alpackages changes) + workspace layer
  parsed:         per-file ParsedUnit // IR + text; Arc per file so unchanged
                                      // files are shared across snapshots
  edges_by_file:  Map<VirtualPath, Arc<Vec<ClassifiedEdge>>>
                                      // the caller file OWNS its outgoing edges
  incoming:       Map<RoutineNodeId, Vec<EdgeRef>>
                                      // DERIVED index — rebuilt O(E) wholesale
                                      // on every update, never edited surgically
  decl_index:     per-file interval index over RoutineDecl
                                      // position → enclosing/at-name routine
  encoding:       negotiated PositionEncoding + lazy per-line UTF-16 tables
}
```

- Queries read the current snapshot: sub-ms, the only lock is an Arc clone.
- `CallHierarchyItem.data` carries a serialized content-addressed `RoutineNodeId`
  (stable across edits — strictly better than legacy's `{object, procedure}` strings).
- **Law (the H-10 lesson):** no derived index is ever surgically edited. `incoming`
  is rebuilt from the edge buckets on every update (43k edges → a few ms). The
  incremental-edge-surgery bug class cannot recur.

## 4. Incremental engine: two-rung soundness ladder

A dedicated updater thread consumes a debounced (~100ms), per-file-coalesced event
queue fed by both `didSave` and the watcher (today's double-reindex collapses to one).

**Rung 1 — body-only edit (the common case; target < 100ms save-to-swap):**

1. Re-parse the saved file F.
2. Compute F's **definition-surface fingerprint** — a hash of everything OTHER files'
   resolution can consult: object identities, routine signatures (incl.
   `params_count`, `sig_fp`, visibility), table fields + their types, enum values,
   extends/implements targets, event-publisher signatures, and resolution-relevant
   object properties (SourceTable, TableNo, …).
3. If the fingerprint is unchanged, only F's own obligations are re-extracted and
   re-resolved against the unchanged indexes. Replace F's edge bucket, rebuild
   `incoming`, swap the snapshot.

*Soundness argument:* resolving a site in file G ≠ F reads (a) G's own IR
(caller-side context: variables, with-state, named returns) and (b) definition
surfaces of all files/ABI. It never reads F's body. Therefore a body-only change in
F can only change F's own edges.

*This argument is pinned, not assumed:* the implementation plan MUST include a
**resolver-read audit task** that enumerates every read the resolver performs and
classifies it caller-side vs surface-side; the fingerprint's field list is derived
from that audit. The incremental-vs-batch gate (section 8) continuously refutes the
claim thereafter.

**Rung 2 — definition surface changed** (rename, signature change, field/enum change,
object property change, file add/delete): rebuild the workspace layer of the graph +
indexes (dep layer and parses of unchanged files reused via Arc), re-resolve all
workspace obligations. Runs in the background; queries serve the previous snapshot
until the swap. Estimate ~1–2s on CDO-size (parse dominates the measured 5.3s; the
actual snapshot/parse/build/resolve split is an early plan task, and the rung-2
target is re-pinned from that measurement).

**Rung 3 (degenerate) — `.alpackages` change or watcher overflow:** full rebuild
including the dep layer (rare; fixes the deps-frozen-at-startup and
overflow-dropped mediums).

Engine-side changes required (all **additive** — the `aldump
--program-call-graph-stats` path is untouched and the frozen CDO SHA must stay
byte-identical):

- a single-file resolve entry point (extract + resolve obligations for one
  ParsedFile against an existing context),
- the dep-layer / workspace-layer split of `ProgramGraph` assembly,
- per-file bucketing of resolved edges.

## 5. Handler mapping

| Feature | Engine source |
|---|---|
| `prepareCallHierarchy` | `decl_index` position→routine; `range`/`selectionRange` from `RoutineDecl.origin`/`name_origin` |
| `incomingCalls` | `incoming[id]` → group by `edge.from`; caller item via BodyMap decl+path; `fromRanges` = site spans |
| `outgoingCalls` | F's edge bucket filtered `from == id`; targets per route (see taxonomy below) |
| `codeLens` | decls per file + `incoming` counts; complexity/params from `analysis.rs` (already IR-direct, reused as-is) |
| unused-procedure diagnostics | `incoming` count 0 + issue-#20 rule port (rules re-anchored on engine data: event subscribers, public API, …) |
| event-subscribers-as-incoming | Phase-2 EventFlow edges — a publisher's incoming calls include its subscribers, now whole-program |
| `fieldProperties` / `actionProperties` / `telemetryStatus` | already graph-independent — untouched |
| `dependencyDocumentSymbol` | engine dep ABI nodes (replaces legacy `dependency_objects`) |
| `eventPublishersInFile` / `eventReferenceAtPosition` | per-file IR events + engine dep ABI nodes |

**Outgoing-target semantics — the honest taxonomy reaches the editor:**

- `Routine(NodeId)` → real item; a dep with embedded source gets REAL navigable spans
  (legacy never could).
- `conditionalResolved` / `ambiguousResolved` candidate sets → one outgoing item per
  candidate (the closed set is shown, not hidden).
- Symbol-only dep target (`Witness::AbiSymbol`) → item with zero-range at the dep
  identity, matching the legacy external-definition fallback shape (exact legacy
  fallback behavior checked at implementation time).
- `honestDynamic` / `honestEmpty` → no target emitted (fail-closed; legacy showed
  nothing here either).

Scope parity: Phase-1 = workspace callers only — the same scope legacy served.
Call-hierarchy queries initiated from INSIDE dep files stay out of v1.

## 6. Protocol fixes (mandatory regardless of migration path)

**H-12 — position encoding:** handler internals stay byte-native; conversion happens
ONLY at the protocol boundary. Per-file lazy line tables (built from
`ParsedFile.text`) convert byte↔UTF-16 columns. At `initialize`, negotiate: if the
client's `general.positionEncodings` offers `utf-8`, advertise and serve natively
(zero conversion); otherwise advertise `utf-16` and convert. New fixtures cover
`æøå`/emoji lines under both encodings.

**H-13 — URI encoding:** rewrite `path_to_uri` on the `percent_encoding` crate
(already a dependency) with a proper path-segment encode set; keep Windows drive
normalization; add a round-trip property test covering `Løsninger`, `#`, `%`,
spaces, and emoji.

## 7. Server model

- Request loop: single-threaded, unchanged — queries are sub-ms snapshot reads.
- Updater: one dedicated thread + the debounced coalescing queue. Parsing NEVER
  happens under a request-facing lock (kills the reindex-under-both-locks medium).
- Watcher overflow → rung-3 full rescan (kills the silent-drop medium).
- Diagnostics: recomputed after every snapshot swap, diffed against last-published
  state; changed URIs re-published and emptied URIs CLEARED (kills publish-once-stale
  and adds the missing clear).
- Buffer sync: `didSave` + watcher only (v1). The snapshot model accepts an overlay
  file source later without redesign (didChange deferred, section 12).

## 8. Parity + testing story

**Adjudicated differential harness (scaffolding — lives only on the migration branch):**

- Runs BOTH backends in-process over the fixture corpus (+ CDO, env-gated via the
  `CDO_WS`/`ENFORCE_CDO_WS` convention), driving identical request scripts
  (prepare/incoming/outgoing/codeLens/diagnostics per routine).
- Normalized JSON responses diffed with a taxonomy:
  - `MATCH`
  - `NEW_BETTER` — justified classes ONLY: case-fold hit (H-11), cross-app target,
    H-10-repaired edge, dep-source span. Each class enumerated and count-pinned,
    L3-harness style.
  - `REGRESSION` — legacy found it, new backend missed it. **Gate: REGRESSION = 0.**
- The harness is deleted together with the legacy engine at arc end — a buggy oracle
  does not outlive its refutation (al-sem retirement doctrine).

**Permanent artifacts:**

- Rust-owned goldens for the new handlers over fixtures (regen via value-gated
  `REGEN_TEMP_GOLDENS=1 cargo test`).
- The **incremental-vs-batch differential gate**: after scripted edit sequences
  (body edits, signature changes, file add/delete, mixed), the incrementally
  maintained snapshot's edge set must equal a fresh batch rebuild byte-for-byte.
  CI on fixtures; CDO env-gated. This is the permanent H-10 insurance and reuses
  the dual-run pattern that built the resolver.
- Frozen CDO SHA `0a3b85bc…` re-measured byte-identical at arc end.
- New Unicode regression coverage: `Løbenr` symbols, `æøå`/emoji positions,
  `Løsninger` paths — the classes the current ASCII-only tests never exercised
  (all four H-bugs are latent in code existing tests do not reach).

## 9. Performance gates

`tests/perf_bounds.rs` is rewritten (it currently pins legacy handler signatures,
the `Arc<RwLock<Indexer>>` shape, and legacy semantics). New targets, same 3x-bound
CI convention, same synthetic 1000-file corpus (`tests/perf_support/`):

| Operation | Target |
|---|---|
| Initial full index (1000-file corpus) | < 2s |
| prepare/incoming/outgoing query | < 1ms |
| Rung-1 save-to-swap (body edit) | < 100ms |
| Rung-2 (surface change, background) | < 2s corpus; re-pinned from the measured stage split |

Real-world reference point: ~5.3s full pipeline on CDO (551 files + 20 deps, warm
dep cache). The snapshot/parse/build/resolve split measurement is an early plan
task; rung targets are pinned from data, not guesses.

## 10. Deletions at arc end

| Path | Fate |
|---|---|
| `src/graph.rs` (2477 lines) | deleted |
| `src/indexer.rs` (1247) | deleted |
| `src/parser.rs` (1459, incl. the r0-corpus projection golden) | deleted (golden retired with it) |
| `src/handlers.rs` (2365) | rewritten on the new backend |
| `src/server.rs`, `src/watcher.rs`, `src/protocol.rs`, `src/config.rs`, `src/main.rs` | survive, modified |
| `src/analysis.rs`, `src/types.rs`, `src/app_package.rs`, `src/dependencies.rs` | untouched (already IR-direct / still feed the engine dep path) |
| `src/language.rs` legacy queries | stay (documented `.scm` editor-highlighting reasons) |

## 11. Known risks

1. **Fingerprint completeness** is THE correctness risk of rung 1. Mitigations: the
   resolver-read audit task pins the field list; the incremental-vs-batch gate
   refutes it continuously; any doubt fails toward rung 2 (fail-closed).
2. Issue-#20 unused-procedure rules are heavily test-pinned; the port is
   behavior-sensitive. The differential harness covers it.
3. The dep-layer/workspace-layer graph split is new engine surface — must stay
   additive; the frozen CDO SHA is the tripwire.
4. The rung-1 <100ms budget is unproven on CDO-size until the stage split is
   measured. If single-file resolve exceeds it, the budget moves; the ladder stands.

## 12. Out of scope / deferred (with wake conditions)

- **didChange overlays** (in-memory buffer sync): deferred; wake = user demand for
  unsaved-buffer freshness. The snapshot model takes an overlay source without
  redesign.
- **Call hierarchy initiated from inside dep files** (dep-caller edges): deferred;
  wake = a user navigating dep source expecting incoming/outgoing there. Requires
  widening Phase-1's caller scope (deliberately skipped today at `full.rs:643-648`).
- **Unicode-fold moat task** (212 raw `to_ascii_lowercase` in `src/program/`): stays
  in the deferred follow-up pool — it is an ENGINE task and the one legitimate
  future SHA-mover; not part of this arc.
- **L3 `this.X` shadow** (`receiver_type.rs:187`): unaffected — L3 remains the
  advisory-only legacy ENGINE axis; this arc retires the legacy LSP pipeline, not L3.
- The remaining follow-up pool items (`.superpowers/sdd/t{0,1,2,4}-minors-for-final-review.md`).

## 13. Binding constraints (inherited)

- Frozen CDO baseline SHA `0a3b85bc832ff0a3e77acee118d203edbf62827dc37617c8d9315fe52d5cb7d0`
  byte-identical at every gate (`scripts/cdo-gate`, `ENFORCE_CDO_WS=1`).
- Fail closed: an unprovable edge is Unknown/Ambiguous, never a guessed resolve.
- Rust-owned goldens; value-gated regen; per-file `rustfmt`; clippy
  `--all-targets --all-features -- -D warnings`; stage only named paths; CHANGELOG
  per change; never push/merge to master without an explicit request.
- Execution: subagent-driven (SDD) with parallel worktree lanes, commit-before-gate
  law, build-token serialization, Opus whole-branch review before merge.
