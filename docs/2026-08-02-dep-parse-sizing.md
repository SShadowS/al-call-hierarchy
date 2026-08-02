# Dependency parse — sizing the follow-up lever

Scoping measurement for the dep parse-artifact cache
(`docs/superpowers/specs/2026-08-01-preflight-verdict-cache.md` §6), taken before
building it. **No fix here.** One design is falsified outright and the other is
sized with a live risk that must be resolved before it is worth starting.

Instrumentation was temporary and is NOT committed (stashed on
`perf/dep-parse-census`): two atomics inside `al_syntax::parse` splitting its
tree-sitter half from its lowering half, plus a `DepLayer` node count.

## Why this lever exists at all

The whole-program resolve **never walks dependency bodies**, verified at source:

- `resolve_full_program_from_parts` Phase 1 filters
  `graph.apps.find(&unit.app) == Some(primary_app_ref)` AND
  `ws_file_set.contains(&pf.virtual_path)` (`full.rs:805-812`) — primary source only.
- Phase 2 is `emit_event_flow_edges` over `graph.routines` — **declarations**, not
  bodies.
- `DeclSurface::build` iterates every parsed unit but reads only
  `obj.routines` → `RoutineMeta::from_decl` (`decl_surface.rs:75-98`) — declarations.

So ~11,856 dependency files are parsed and lowered in full on every run, and only
their declarations are ever consumed.

## FALSIFIED: "lower declarations only, skip dep bodies"

The attractive no-cache design — never build body IR for dep units — is bounded by
the lowering share, and that share is the minority:

| half of `al_syntax::parse` | CPU ms (summed across threads), DO | share |
|---|---:|---:|
| **tree-sitter parse** | **21,293** | **74.6 %** |
| IR lowering | 6,867 | 25.4 % |

(11,856 files; two runs agreed within 20 %: 21,830/7,435 and 21,293/6,867.)

Three quarters of the cost is producing the CST, which is unavoidable if the file
is read at all — and skipping body lowering would not recover all of the remaining
quarter either, since object/routine shells still lower. **The ceiling on this
design is well under 25 % of `parse_snapshot`, against a change to the lowerer —
the single highest-risk file in the repo (the only place that reads raw
tree-sitter). Not worth it.** A cache skips both halves, so it dominates.

## SIZED, with a live risk: the dep parse-artifact cache

| population, DO | count |
|---|---:|
| dependency files parsed | 11,856 |
| dependency objects (`DepLayer::dep_objects`) | **11,165** |
| dependency routines (`DepLayer::dep_routines`) | **126,640** |

What a hit would save: `parse_snapshot` ~1,139 ms + `dep_layer` ~142 ms ≈
**1,280 ms**, on EVERY run whose dependencies are unchanged — including the edit
loop, which the verdict cache explicitly does not help.

What it must persist, per dependency app, keyed on the `.app` blake3 (pure by Q2):
the extracted `ObjectNode`/`RoutineNode` sets, the `DeclSurface` contribution for
those 126,640 routines, and the per-file `ParseStatus`/recovered paths.

**The risk, stated plainly: ~126k routine records is a 40–60 MB artifact, and
`serde_json` deserialization plus allocating 126k String-bearing structs plausibly
costs 150–400 ms — a material fraction of the 1,280 ms saved.** This is exactly
the "cache that costs more to load than to recompute" failure the spec named. It
is not resolvable by reasoning; it needs a round-trip measurement.

**Go/no-go before building:** serialize a representative `DepLayer` (11,165
objects / 126,640 routines) and time load. Under ~200 ms, build it. Approaching
~600 ms, don't — or switch the format to a compact binary encoding first and
re-measure. `AppRef` is a per-run interning index, so the artifact must store app
identity symbolically and re-intern on load regardless of format.

## A cheaper lever this measurement exposed

`preflight.snapshot_build` is now the LARGEST preflight item on a warm verdict-cache
hit (~470 ms of DO's ~1,020 ms run), and most of it is `cached_source` loading
~11,856 dependency source texts out of the extraction cache — **source that a warm
hit never parses**. The verdict-cache key needs app identities and `.app` content
hashes, not source text.

Deriving the key from a LIGHT snapshot (same `load_all_apps` discovery and the same
`app_content_hash`, minus source materialization for dependency units) would cut
most of that ~470 ms. Note this is NOT the "cheaper pre-snapshot key" the spec
rejects: that rejection was about a SECOND discovery implementation that can drift
from the real one. This is the same implementation with a step skipped, and the
workspace's own source still has to be read for its content fold.

Smaller than the artifact cache, much cheaper to build, and it compounds with the
verdict cache already shipped.
