# AL Call Hierarchy

Blazing-fast call hierarchy server for AL (Business Central) using tree-sitter.

## Features

- **Sub-millisecond queries** - Pre-computed call graph with O(1) lookups
- **Parallel indexing** - Uses all CPU cores for initial index
- **Incremental updates** - Re-parse only changed files
- **External dependency support** - Resolves calls to .app packages in `.alpackages`
- **Event subscriber integration** - Shows `[EventSubscriber]` procedures in call hierarchy
- **Code Lens** - Reference counts above each procedure
- **Diagnostics** - Unused procedure detection and code quality warnings
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
- `textDocument/codeLens`
- `textDocument/publishDiagnostics` (server push)

### CLI Mode (testing)

```bash
al-call-hierarchy --project /path/to/al-project --no-lsp
```

## Integration

### With AL LSP Wrapper

The Go/Python wrapper spawns this server and routes requests. See [LSP.md](LSP.md) for detailed integration guide.

```go
case "textDocument/prepareCallHierarchy",
     "callHierarchy/incomingCalls",
     "callHierarchy/outgoingCalls",
     "textDocument/codeLens":
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
| External .app dependencies | Yes |

## External Dependencies

The server automatically resolves calls to procedures defined in external .app packages:

1. Reads `app.json` in the project root for declared dependencies
2. Finds matching .app files in the `.alpackages` folder
3. Extracts procedure definitions from `SymbolReference.json` inside each .app
4. Shows "(from AppName)" in call hierarchy for resolved external calls

### Version Matching

When multiple versions of the same app exist in `.alpackages`, the server selects the highest compatible version based on the dependency declaration in `app.json`.

### Supported .app Structure

The server parses .app files with the standard BC format:
- 40-byte NAVX header (skipped)
- ZIP archive containing:
  - `NavxManifest.xml` - App metadata
  - `SymbolReference.json` - Symbol definitions

## Configuration

Diagnostic thresholds are configurable via `.al-call-hierarchy.json` in the workspace root. All values are optional — missing values use defaults.

```json
{
  "diagnostics": {
    "complexity": { "warning": 5, "critical": 10 },
    "parameters": { "warning": 4, "critical": 7 },
    "lineCount": { "warning": 20, "critical": 50 },
    "fanIn": { "warning": 20 },
    "unusedProcedures": true
  }
}
```

| Setting | Default | Description |
|---------|---------|-------------|
| `complexity.warning` | 5 | Cyclomatic complexity information threshold |
| `complexity.critical` | 10 | Cyclomatic complexity warning threshold |
| `parameters.warning` | 4 | Parameter count information threshold |
| `parameters.critical` | 7 | Parameter count warning threshold |
| `lineCount.warning` | 20 | Method length information threshold |
| `lineCount.critical` | 50 | Method length warning threshold |
| `fanIn.warning` | 20 | Incoming call count information threshold |
| `unusedProcedures` | true | Enable/disable unused procedure detection |

## License

This project is licensed under the GNU General Public License v3.0 - see the [LICENSE](LICENSE) file for details.
