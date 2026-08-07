# Dependency pack cache — design

Status: DESIGN, pre-implementation.
Supersedes the follow-up sketch in `2026-08-01-preflight-verdict-cache.md` §6.
Sizing evidence: `docs/2026-08-02-dep-parse-sizing.md`.
Span census: `docs/2026-07-31-preflight-census.md`.

Reviewed adversarially in two rounds by `claude-fable-5`. Every codebase claim below was
verified against source. Two review claims failed verification and the corrections are kept
in place rather than quietly dropped — see §7 (the `CompilationContext` rationale) and §13
(the span attribution behind the format choice).

---

## §1 — The problem

`alsem analyze` fully parses and lowers every dependency source file on every run. On DO
that is 11,856 files producing 11,165 objects and 126,640 routines, costing
`preflight.parse_snapshot` 1,139.2 ms plus `preflight.dep_layer` 141.9 ms ≈ **1,280 ms**.

**Only declarations are ever consumed from dependencies.** Verified at three sites:

- `resolve_full_program_from_parts` Phase 1 filters to the primary app AND the workspace
  file set (`src/program/resolve/full.rs:805-812`)
- Phase 2 `emit_event_flow_edges` walks `graph.routines` — declarations
- `DeclSurface::build` reads only `obj.routines` → `RoutineMeta::from_decl`
  (`src/program/resolve/decl_surface.rs:75-98`)

`DeclSurface`'s own doc states the intent: `RoutineMeta` holds "EXACTLY the fields
resolution reads (audited): never the routine body — dropping the dep parse arenas is the
whole point" (`decl_surface.rs:10-11`).

So the declaration-native representation already exists. The residual cost is that
*producing* it routes through a full parse, every run.

## §2 — Workload premise

**One-off analyses of many different workspaces**, not an edit loop and not CI reruns of one
repo.

The already-shipped preflight verdict cache does not help this workload: its key covers the
workspace's own app identity and source content, so a different workspace always misses.

What repeats is the DEPENDENCY set. Base Application and System Application are static
artifacts per (BC version, localization) pair. Many customer apps target the same base
version; a user works within few localizations; Microsoft ships a bounded number of versions.

**Therefore the pack store is a small library of Microsoft base versions**, with cardinality
≈ (versions × localizations touched). This premise is the human's domain knowledge, adopted
deliberately over the review's "cross-workspace hit rate is unmeasured" objection. It is
recorded here as a premise so that a later observation of low hit rate is recognised as
falsifying it, rather than as a surprise.

## §3 — Scope

**In:** a per-dependency-app, content-keyed, fail-closed on-disk artifact ("pack") holding
the extracted declaration products, wired in as a third ingestion route.

**Out:** persisting any classified edge (unsound, §4); packs for symbol-only apps (the ABI
route is a slice of `dep_layer`'s 141.9 ms, not of the 1,139 ms parse — de-scoped in the
prior spec and still de-scoped); pack pruning (§10); the resident daemon (§11).

## §4 — The purity boundary

Dependency-derived PARSE/DECLARATION products ARE a pure function of dependency content:
`extract_nodes(app_ref, &pf.file, tier, …)` reads only the parsed file and its tier
(`src/program/build.rs:89-97`).

Dependency-derived EDGES are NOT. `ResolveIndex::build` populates `subscribers_map` by
iterating ALL of `graph.routines` with no app scoping
(`src/program/resolve/index.rs:285-300`), and `emit_event_flow_edges` emits one route per
subscriber (`resolver.rs:3006-3045`). A primary-app subscriber attaches to a dependency
publisher, so dep-internal edges keyed on dep bytes alone would be unsound.

**Packs persist nodes and `RoutineMeta`. Edges are always recomputed.**

## §5 — Architecture: the pack IS the ingestion route

Not a cache bolted beside `build_dep_layer`, but a third route inside it, symmetric with the
two that exist:

| route | population | today |
|---|---|---|
| source-parse + extract | source-bearing dep apps | Step 2 |
| `ingest_abi` | symbol-only dep apps | Step 2b |
| **pack-load** | **`EmbeddedSource`-tier dep apps with a pack hit** | **new** |

Per app: hit → nodes and `RoutineMeta` from the pack, source never read; miss → parse and
extract as today, then store a pack.

This seam is also where the daemon later plugs in: `lsp::updater`'s `Arc<DepLayer>` reuse and
a pack load become the same abstraction at different lifetimes.

**Do not** split dependency and workspace routines into different types. `ResolveIndex::build`
(`index.rs:285-300`), `emit_event_flow_edges` (`resolver.rs:3006-3045`) and
`inject_platform_event_publishers` all iterate one unified `graph.routines`; the `tier` field
already carries the distinction.

## §6 — Pack contents

Per dependency app:

- format version, key echo, self-hash — mirroring `preflight_cache::Entry`'s integrity
  discipline (`preflight_cache.rs:276-289`)
- app identity **symbolically** (guid / name / publisher / version), re-interned on load.
  `AppRef` is a per-run interning index (`build.rs:76-77`); persisting one yields
  silently-wrong graphs, not errors.
- per source file, in extraction order: virtual path, `ParseStatus`, and that file's
  extracted `ObjectNode` / `RoutineNode` contributions
- the `DeclSurface` contribution — `RoutineMeta` per routine
- recovered-file paths

Stored **pre-dedup**, per-file order preserved, so the existing Step 4 sort and
`dedup_routines_preserving_genuine_overloads` run identically over the merged population.

## §7 — The pack key

blake3, every field length-prefixed (without prefixing, `("ab","c")` and `("a","bc")` fold
identically):

domain tag ‖ pack format version ‖ `EXTRACTION_FINGERPRINT` (§8) ‖ the app's `.app` content
hash ‖ the app's `TrustTier` ‖ the app's `CompilationContext`.

`preflight_cache.rs:20-26` states the governing rule: there is no payload-level defence, a
well-formed entry under a correct key is indistinguishable from a correct one without
recomputing, so **all soundness lives in the key**.

Component notes:

- **`TrustTier` is a real input.** `extract_nodes` takes it (`build.rs:89-97`), it is stamped
  into every node, and it drives dedup behaviour.
- **`CompilationContext` is aspirational, and the spec says so.** `al_syntax::parse` takes
  only text (`crates/al-syntax/src/parse.rs:10-19`), the lowerer union-reads every `#if`
  branch, and a dependency's `preproc_symbols` is empty by construction —
  `context_from_metadata` hardcodes `BTreeSet::new()` because the manifest does not record
  them (`src/snapshot/compilation.rs:29-37`). Included because `runtime`/`platform`/
  `application` ARE populated and a future evaluating lowerer would make symbols live. It is
  a constant today; do not cite it as a live input.

**Packs are restricted to `TrustTier::EmbeddedSource` units.** Non-primary source-bearing
apps also include multi-app workspace siblings backed by *walked* source, which have no
`.app` hash — and the snapshot layer's `content_hash` for walked source is explicitly
unusable as identity: it folds only concatenated file text with no path and no length prefix,
colliding on rename and on re-splitting the same bytes across file boundaries
(`preflight_cache.rs:162-168`, which refuses to reuse it). Siblings churn constantly and are
not the base-version library this design exists for. Leaving "source-bearing" unqualified
would be a soundness hole.

## §8 — `EXTRACTION_FINGERPRINT`

### Why not the engine binary hash

The preflight cache keys on the whole binary, which "cannot rot in the first place — there is
nothing to bump" (`preflight_cache.rs:110-125`). But it invalidates every pack on every
`cargo build`, so during active engine development the cache never hits and the base-version
library premise (§2) never holds.

### Why not a hand-curated file list

Because that is a hand-bumped constant with extra steps, and this repo has a documented
corpse: `CACHE_VERSION_GRAMMAR` read `"tree-sitter-al-v2.5.2-native"` while the pinned
grammar had already moved to v3.2.0 — unbumped across two upgrades, nothing failed, until a
dedicated test was added (`preflight_cache.rs:117-125`).

The first draft of this design listed `crates/al-syntax/src/`, `node_extract.rs`, `node.rs`
and the grammar. Tracing `node_extract.rs:3-14`'s actual imports shows that closure **missing
five files**: `src/program/sig_fp.rs` (computes the `sig_fp` in every persisted
`RoutineNodeId`), `src/program/resolve/event.rs` (subscriber/publisher attribute parsing,
serialized into `RoutineNode.event_subscribers`), `src/program/resolve/edge.rs`
(`AbiEventKind`/`AbiRoutineKind`, payload fields), `src/program/resolve/receiver.rs`
(`unquote_identifier`), and `src/program/resolve/decl_surface.rs` (`RoutineMeta::from_decl`).
The next `use` added silently exits any such list.

### The decision: coarse closure plus a behavioural canary

**Closure** — hash the contents of:

- `crates/` (all of it, including the al-syntax lowerer, IR and raw vocabulary)
- `src/program/`
- `src/snapshot/` (covers `embedded.rs` / `cache.rs`, which determine which entries a `.app`
  yields and how virtual paths are named — the `.app` blake3 alone does not fix that)
- the tree-sitter-al grammar's **source files** (`grammar.js`, generated `parser.c`), never a
  version string
- the tree-sitter **runtime** crate's lockfile entry — error recovery lives in the core
  library, not the grammar, so a runtime upgrade can flip `ParseStatus::Recovered`/`Clean`,
  which changes persisted `parse_incomplete` and recovered-file paths
- the pack serialization module itself

This deliberately excludes `src/engine/`, `src/lsp/` and `src/bin/`, where most commits land.
Over-invalidation costs one ~1,280 ms reparse per base version; a stale pack costs
correctness.

**Canary** — a golden test that runs `extract_nodes` + `RoutineMeta::from_decl` over a pinned
fixture corpus and byte-compares the serialized pack against a committed artifact. This
guards the *behaviour*, not the file list, so a change anywhere in the true closure fails
loudly and forces a fingerprint review. Both mechanisms ship; neither alone is sufficient.

## §9 — Storage discipline

One file per key under `<os-cache>/alsem/dep-pack-v1/`, mirroring `preflight_cache.rs`
throughout: atomic tmp+rename, every abnormal state (missing, unreadable, schema mismatch,
key mismatch, fingerprint mismatch, self-hash mismatch) is a silent miss logged at debug, a
write failure never breaks a run, environment variables override the directory and disable
the cache entirely.

**Only fully-extracted results are stored.** A file whose parse was `Recovered` IS
deterministic content and is safe to cache; a transiently failed `.app` extraction is not.
This mirrors preflight rule 3 — `Ok` cached, `Err` never (`preflight_cache.rs:29-38`).

**Inherited trust caveat, stated not hidden:** a pack built from a stale
`al-ch-snapshot-cache` entry is poisoned even under a perfect key. That cache is the one
`preflight_cache.rs:66-70` calls out as having no override and no pruning. Packs inherit its
soundness; hardening it is out of scope here and noted as a live risk (§13).

## §10 — Light snapshot (prerequisite, not adjacent)

For dependency units, compute the pack key **without materializing source text** — the `.app`
content hash is already available via `snapshot::embedded::app_content_hash`. On a hit that
app's source is never read.

Three reasons this is a prerequisite rather than a nice-to-have:

1. **Without it the saving is not realized.** `preflight.snapshot_build` is 458.8 ms on DO,
   most of it loading 11,856 dependency source texts a hit never parses.
2. **It structurally enforces the declarations-only invariant.** "Only declarations are
   consumed" is true today and unenforced. On a hit the dependency's `ParsedFile`s simply do
   not exist (`parse_snapshot` skips units whose `source` is `None`,
   `src/snapshot/parse.rs:75-95`), so a future body-reading feature fails loudly instead of
   silently reading stale data.
3. **It has standalone value.** It cuts `snapshot_build` for the already-shipped verdict
   cache even if packs are later abandoned.

**Required before it ships:** a test asserting the verdict cache's `cache_key` is byte-equal
between a full and a light snapshot of the same workspace. `source_identity` takes the
`EmbeddedSource` branch today (`preflight_cache.rs:177`); a light unit with `source: None`
falls to the `app_path` branch (`preflight_cache.rs:186-192`). Both yield the `.app` blake3,
but any structural difference in the canonical fold would silently zero the shipped cache's
hit rate.

## §11 — Two consumers that must be rewired

These are the design's real failure points — the data is listed above, the wiring is where it
breaks.

1. **`DeclSurface` frozen-tier injection.** `DeclSurface::build` independently iterates
   parsed units (`decl_surface.rs:75-98`); on a pack hit those units do not exist. The pack
   carries `RoutineMeta`, but something must inject it into the frozen tier
   (`decl_surface.rs:66-68`). Miss this and every dependency routine metadata lookup degrades
   to a decline — a silent quality regression, not an error.
2. **`recovered_file_paths`.** It iterates parsed units (`src/snapshot/parse.rs:61-76`) and
   is the load-bearing absence-proof diagnostic: its doc states "**Any** current or future
   absence/`ProvenAbsent`-shaped claim in this engine MUST consult this diagnostic … before
   treating a file's content as complete" (`parse.rs:47-60`). On a hit, the pack's recovered
   paths must merge into that function's output or a warm run under-reports recovery.

Every other consumer of `parsed` for dependency units must be audited the same way.

## §12 — Ordering and the equivalence gate

`dedup_routines_preserving_genuine_overloads` keeps the first occurrence per key — arbitrary
but stable, where stability comes from one deterministic extraction order. If packs are
loaded per app and concatenated in a different order than `parsed` iteration order,
first-occurrence survivors can differ between cold and warm runs.

**Pin concatenation to `snap.apps` order; preserve per-file extraction order inside each
pack.**

**The merge gate compares final analysis output (edges and findings), not just
`ProgramGraph` bytes.** A `ProgramGraph`-only comparison would pass while §11's `RoutineMeta`
degradation silently changed resolution results.

## §13 — Format and the measurement gate

**Format:** compact binary, record-per-routine (postcard-shaped), pending the gate below. The
shared-string-table design stays in reserve.

**The evidence for this choice, stated honestly.** The review first argued the format is safe
because `assemble_program_graph` already clones the whole population per run
(`build.rs:179-180`) inside a measured span. That span attribution was wrong — the clone sits
in `preflight.assemble_graph` (116.4 ms), not `preflight.dep_layer` (141.9 ms); the two are
disjoint (`full.rs:1119-1122` and `:1136-1139`). Corrected, the bound is tighter, but the
analogy still does not hold: a decode additionally does file I/O, varint parsing, UTF-8
validation on every string, enum discriminant decoding and the guid→`AppRef` re-intern, none
of which a clone pays. A serial decode plausibly lands at 100–250 ms — straddling the gate.

What actually carries the recommendation is **parallelism**: packs are per-app files decoded
inside `build_dep_layer`'s per-app loop, on the same rayon pool `parse_snapshot` already uses
(`src/snapshot/parse.rs:86-95`). So postcard is a hypothesis with a favourable prior, not a
bounded quantity.

**The gate therefore measures the hit-path shape, not a micro-benchmark:** per-app packs,
parallel decode, guid re-intern included, from a cold OS file cache at least once. Under
~200 ms proceed; approaching ~600 ms switch to the string-table format and re-measure. Note
the 600 ms line is a product-quality choice, not a soundness line — even a 300 ms load
against 1,280 ms saved is a net win.

**Unclaimed upside:** on a hit the dependency `AlFile` arenas are never allocated, so
`preflight.ctx_drop` (248.7 ms) shrinks too. The true saving exceeds the 1,280 ms headline.

**Honest ceiling:** `assemble_graph`'s 116.4 ms clone and re-sort survive untouched on every
run. Packs never make dependency cost proportional to workspace size.

## §14 — Build order

1. **Light snapshot** + the key-equality test (§10). Standalone value; prerequisite.
2. **Serialization surface** for `ObjectNode` / `RoutineNode` / `RoutineMeta` and their
   field-type closure. `node_extract.rs` has zero serde today.
3. **Measurement gate** (§13). Placed after 2 because it cannot run without it, and before
   the seam because abandoning here costs derives and a benchmark, not ingestion surgery.
4. **`EXTRACTION_FINGERPRINT`** build script + the canary test (§8). Pure invalidation
   plumbing; not needed to measure, and the component most likely to be redesigned.
5. **Pack seam** in `build_dep_layer` (§5), including both rewires in §11.
6. **Cold-vs-warm output-equivalence gate** (§12) + discrimination proofs.

## §15 — Accepted debt

**No pruning.** This is the third unpruned cache in the repo (`preflight_cache`,
`al-ch-snapshot-cache`, now packs). Accepted deliberately here, unlike the other two, because
pack cardinality is bounded by (BC versions × localizations touched) and pack entries are
*meant* to live indefinitely — that is the base-version library premise of §2. Record the
reasoning, not merely the debt. If §2 is falsified, this decision must be revisited with it.

## §16 — Live risks

- **§2's hit-rate premise is domain knowledge, not a measurement.** Low observed hit rate
  falsifies the design's justification, not just its tuning.
- **Packs inherit `al-ch-snapshot-cache`'s soundness** (§9).
- **The coarse closure over-invalidates** during engine development; if that proves painful,
  narrow it only with the canary in place.
- **`al_syntax::parse` constructs a fresh `tree_sitter::Parser` and calls `set_language` per
  file** (`crates/al-syntax/src/parse.rs:11-14`); there are no thread-local parsers anywhere
  in the tree, despite CLAUDE.md's Core Patterns section claiming "Thread-local parsers
  process files concurrently". Unrelated to packs, inside the same span, cheap to fix,
  tracked separately.
