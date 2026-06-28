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

## tree-sitter-al grammar migrations

The grammar is now **v3** (the project builds against `tree-sitter-al` `main`,
checked out unpinned by CI — `cargo test` runs against whatever HEAD is). v3 added
wrapper nodes that broke the v2 assumptions above.

- **Recursive AST walks survive a grammar bump; flat direct-child iterations break.**
  v3 wraps contents in nodes between a container and its children: `code_block.body`
  → `statement_block` (holds the statements), object/field/action bodies →
  `declaration_body`, report dataitem body → `report_body`, case branches →
  `case_body`, `var_section.body` → `var_body`. Any `named_children(x)` reading
  statements/properties/members directly must descend the wrapper.
  - Statements: use `node_util::block_statements` (flattens `statement_block`
    inline, preserving trailing trivia).
  - Object/field properties: `decl.child_by_field_name("body")` then iterate.
  - A member-trigger's parent is now a `*_body` wrapper, not the named member.
- **Owned grammar fixes (we maintain `tree-sitter-al`).** Two field-pollution fixes
  landed on grammar `main` and changed node shapes the lowerer relies on:
  - `trigger_declaration name:` is no longer only `(identifier)`/`(quoted_identifier)` —
    a scoped member trigger (`Object::Member`) is a single named `member_trigger_name`
    node (`object` / `member` fields). The `name` field is `multiple:false` (no `::`
    token in its type set). `lower::routine_name_text` joins it to `Object::Member`.
  - The case `pattern` field binds ONE `_single_pattern` value, never the `,`/`:`
    separators (rule `_case_pattern_item`); an `in`-as-case-pattern is the named
    `in_expression`, not an inline `seq` leaking `left`/`operator`/`right`. The lowerer
    keeps a defensive `is_named()` filter on `children_by_field("pattern")`.
  - Editor consumers: `queries/highlights.scm` + `queries/tags.scm` capture
    `member_trigger_name`. The engine does NOT use `.scm` queries (IR-only).
- **Validate a migration with the al-sem differential goldens** (`cargo test`):
  zero divergences = behaviour-preserving. The goldens are the al-sem **TS
  reference** output, not Rust's — they are the source of truth.
- Dump a real tree with `tree-sitter parse <file.al>` from `tree-sitter-al/`.

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

## Working Principle

**Always pursue the best solution — not the simplest, easiest, or quickest.** Time is not
a constraint and this project is not yet released, so refactoring is always on the table
and all downstream consumers are ours to change. Fix root causes, never symptoms: when a
golden or test disagrees with the code, find out WHY before rebaselining (a wrong golden
or half-finished feature gets fixed properly, not papered over). Prefer correct
architecture over a quick patch even when larger; verify by building/running/measuring,
not by assertion.

## Development Guidelines

- **CHANGELOG.md must be updated** after making any feature additions, bug fixes, or breaking changes
- Follow [Keep a Changelog](https://keepachangelog.com/) format
- Group changes under: Added, Changed, Deprecated, Removed, Fixed, Security
- **Format per-file with `rustfmt <file>`**, never `cargo fmt`. Stage only intended paths;
  never `git add -A`. Never push or merge to `master` without an explicit request.
