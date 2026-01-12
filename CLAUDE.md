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
cargo fmt                      # Format code
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
- tree-sitter-al grammar at `../tree-sitter-al` (or set `TREE_SITTER_AL_PATH`)

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

## Development Guidelines

- **CHANGELOG.md must be updated** after making any feature additions, bug fixes, or breaking changes
- Follow [Keep a Changelog](https://keepachangelog.com/) format
- Group changes under: Added, Changed, Deprecated, Removed, Fixed, Security
