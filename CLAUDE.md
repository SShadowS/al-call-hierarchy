# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

AL Call Hierarchy is a high-performance LSP server providing call hierarchy functionality for AL (Microsoft Dynamics 365 Business Central) language. It uses tree-sitter for parsing and provides sub-millisecond query responses via pre-computed call graphs.

## Build Commands

```bash
cargo build                    # Debug build
cargo build --release          # Optimized release (full LTO)
cargo build --profile release-fast  # Faster build with thin LTO
cargo test                     # Run tests
rustfmt path/to/file.rs        # Format a file (NEVER `cargo fmt` — whole-crate churn)
cargo clippy --all-targets --all-features  # Lint
```

## Running

```bash
# LSP mode (default) - communicates via stdio
cargo run

# CLI mode for testing/debugging
cargo run -- --project /path/to/al-project --no-lsp

# With verbose logging
cargo run -- --project /path/to/al-project --verbose
```

## Prerequisites

- Rust 1.75+
- tree-sitter-al V2 grammar (included as git submodule at `tree-sitter-al/`)
  - Clone with `git clone --recurse-submodules`, or run `git submodule update --init` after clone
  - Override path with `TREE_SITTER_AL_PATH` env var if needed

## Architecture

**Data Flow:**
```
AL Source Files → Tree-sitter Parser (language.rs) → Parsed Definitions/Calls (parser.rs)
    → Call Graph Builder (indexer.rs) → Call Graph (graph.rs) → LSP Server (server.rs)
    ← File Watcher (watcher.rs) for incremental updates
    → Request Handlers (handlers.rs) → Resolver (resolver.rs) → LSP Responses
```

**Key Modules:**
- `main.rs` - CLI entry point, argument parsing
- `server.rs` - LSP server initialization and main loop
- `handlers.rs` - LSP request handlers (prepareCallHierarchy, incomingCalls, outgoingCalls)
- `graph.rs` - Call graph data structures with O(1) lookups
- `parser.rs` - Tree-sitter AL parsing, extracts definitions and call sites
- `indexer.rs` - Parallel file indexing using rayon
- `language.rs` - Tree-sitter queries for definitions, calls, event subscribers, variables
- `watcher.rs` - File system watcher for incremental updates
- `resolver.rs` - Call resolution logic (qualified and unqualified calls)
- `app_package.rs` - Parser for .app files (extracts SymbolReference.json)
- `dependencies.rs` - Dependency resolution from app.json and .alpackages

**Core Patterns:**
- **String interning** (`string-interner`): All symbol names deduplicated for memory efficiency
- **Parallel parsing** (`rayon`): Thread-local parsers process files concurrently
- **Incremental updates** (`notify`): Only re-parse changed files

## Performance Targets

| Operation | Target |
|-----------|--------|
| Initial index (100 files) | < 500ms |
| Initial index (1000 files) | < 2s |
| prepareCallHierarchy | < 1ms |
| incomingCalls | < 1ms |
| outgoingCalls | < 1ms |
| File change update | < 50ms |

## Key Data Structures

```rust
QualifiedName { object: Symbol, procedure: Symbol }  // Unique procedure identifier
Definition { file, range, object_type, object_name, name, kind }  // Procedure/trigger location
CallSite { file, range, caller, callee_object, callee_method }  // Call location with context
```

## Tree-sitter-al V2 Grammar Notes

This project uses the V2 tree-sitter-al grammar. Key differences from V1:

- **No wrapper nodes**: `procedure name:` and `trigger_declaration name:` fields hold `(identifier)` or `(quoted_identifier)` directly (no `(name)` or `(trigger_name)` wrapper)
- **Parameter field renamed**: `parameter_name:` → `name:`
- **Member expression field renamed**: `property:` → `member:`
- **`field_access` removed**: merged into `member_expression` with `quoted_identifier` as member
- **`named_trigger`/`onrun_trigger` removed**: unified into `trigger_declaration`
- **Attributes are siblings**: `attribute_item` nodes are siblings of `procedure`, not children. The EVENT_SUBSCRIBERS query matches attributes separately and resolves the adjacent procedure in Rust code.
- **Unified `property` node**: Individual `*_property` nodes replaced by a single `property` node with `name: (property_name)` and `value:` fields
- **`preproc_split_codeunit_declaration` renamed**: now `preproc_split_declaration`

## Adding New AL Constructs

1. Update tree-sitter queries in `language.rs` (DEFINITIONS, CALLS, EVENT_SUBSCRIBERS, VARIABLES)
2. Update parsing logic in `parser.rs`
3. Test against fixtures in `tests/fixtures/`

## Resolution Coverage

| Call Pattern | Status |
|--------------|--------|
| Local procedures | Yes |
| Qualified calls (Object.Method) | Yes |
| Record methods | Partial |
| Event subscribers | Yes |
| External .app dependencies | Yes |

## Project Direction & The Moat

The product's moat is **precise whole-program call-graph resolution** for AL. The
north-star metric is the **real-`unknown` edge rate** on real BC apps (measure with
`aldump --l3-call-graph-stats <workspace>`): drive it toward zero, where the residual
is provably dynamic. The honest resolution taxonomy is `resolved` / `builtin` (platform
intrinsic, not a hole) / `dynamic` (runtime-typed) / `external` (dependency object) /
`unknown` (a TRUE failure — the signal to eliminate). See the call-graph resolution
redesign spec under `docs/superpowers/specs/`.

## Testing Philosophy & Goldens

- **The al-sem TypeScript reference is RETIRED.** This engine began as a faithful Rust
  port of al-sem (TS), validated by byte-for-byte differential goldens. That era is over.
  The engine is now **Rust-owned**: correctness and resolution precision take priority
  over reproducing al-sem's output.
- **No byte-to-byte parity with al-sem.** Tests assert **Rust-owned baselines** (goldens
  regenerated from THIS engine) and **structural CONTRACTS** (invariants that hold
  regardless of which engine is "right" — e.g. "every `builtin` edge's method is in the
  catalog", "no edge is both `builtin` and `resolved`"). When the Rust engine is
  intentionally MORE accurate than al-sem (resolving record/framework built-ins, removing
  phantom uncertainties, etc.), that divergence is CORRECT — rebaseline the Rust-owned
  golden; do not chase al-sem.
- **We control all downstream consumers.** Every program consuming engine output (CLI
  formats, snapshots, fingerprints, SARIF, digests, prove/diff) is ours to change. Output
  shape may be refactored freely when it improves the product — update the consumers and
  their goldens together.
- **Goldens regen:** `REGEN_TEMP_GOLDENS=1 cargo test` rewrites Rust-owned goldens.
  Inspect the diff is intended before committing. Manifest "matrix" oracles hold
  Rust-owned numbers; update them to the current Rust value when the engine intentionally
  improves.
- **`U:\Git\al-sem` is a frozen historical reference, not a live oracle** — never read
  goldens from it or write into it at test time. Tests still pointing at the al-sem repo
  (some cli-b differentials, r3a1/r4f) are LEGACY and should be migrated to in-repo
  Rust-owned goldens or contract assertions when touched.
- **`KNOWN_DIVERGENCES.json`** is a legacy port-parity artifact; it is not the mechanism
  for Rust-ahead-of-al-sem behavior (that is just the new correct baseline).

## Development Guidelines

- **CHANGELOG.md must be updated** after making any feature additions, bug fixes, or breaking changes
- Follow [Keep a Changelog](https://keepachangelog.com/) format
- Group changes under: Added, Changed, Deprecated, Removed, Fixed, Security
- **Format per-file with `rustfmt <file>`**, never `cargo fmt`. Stage only intended paths;
  never `git add -A`. Never push or merge to `master` without an explicit request.
