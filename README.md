# AL Call Hierarchy

Blazing-fast call hierarchy server for AL (Business Central) using tree-sitter.

## Features

- **Sub-millisecond queries** - Pre-computed call graph with O(1) lookups
- **Parallel indexing** - Uses all CPU cores for initial index
- **Incremental updates** - Re-parse only changed files
- **LSP integration** - Works with any LSP-compatible client

## Building

Prerequisites:
- Rust 1.75+
- tree-sitter-al grammar at `../tree-sitter-al`

```bash
cargo build --release
```

## Usage

### LSP Mode (default)

```bash
al-call-hierarchy
```

Communicates via stdio using the LSP protocol. Handles:
- `textDocument/prepareCallHierarchy`
- `callHierarchy/incomingCalls`
- `callHierarchy/outgoingCalls`

### CLI Mode (testing)

```bash
al-call-hierarchy --project /path/to/al-project --no-lsp
```

## Integration

### With AL LSP Wrapper

The Go/Python wrapper spawns this server and routes call hierarchy requests:

```go
case "textDocument/prepareCallHierarchy",
     "callHierarchy/incomingCalls",
     "callHierarchy/outgoingCalls":
    return callHierarchyServer.Request(method, params)
```

## Performance Targets

| Operation | Target |
|-----------|--------|
| Initial index (100 files) | < 500ms |
| Initial index (1000 files) | < 2s |
| prepareCallHierarchy | < 1ms |
| incomingCalls | < 1ms |
| outgoingCalls | < 1ms |
| File change update | < 50ms |

## Resolution Coverage

| Call Pattern | Resolvable |
|--------------|------------|
| Local procedures | Yes |
| Qualified calls (Object.Method) | Yes |
| Record methods | Partial |
| Event subscribers | Yes |
| External .app dependencies | No |

## License

This project is licensed under the GNU General Public License v3.0 - see the [LICENSE](LICENSE) file for details.
