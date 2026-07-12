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

## Telemetry

`al-call-hierarchy` ships with **anonymous, opt-out failure-diagnostics telemetry** so the maintainer can find resolution gaps that real-world AL projects hit. **No raw identifiers, file paths, or source code leave your machine.** All AL identifier names are hashed with a per-installation random 32-byte salt that stays on your machine; the maintainer sees only structural fingerprints (object types, failure categories, tree-sitter shapes) plus salted hashes.

**What's collected:** see [docs/telemetry.md](docs/telemetry.md). **Source code:** [src/telemetry/](src/telemetry/) — auditable in one directory.

**Telemetry is OFF by default in:**
- Debug builds (`cargo build` without `--release`)
- Test runs (`cargo test`)
- CI environments (CI, GITHUB_ACTIONS, GITLAB_CI, etc.)

**Three ways to disable** (any wins):

1. Environment variable: `AL_CH_TELEMETRY=0` or `DO_NOT_TRACK=1`
2. CLI flag: `al-call-hierarchy --no-telemetry`
3. Config file `~/.al-call-hierarchy/config.json`:
   ```json
   { "telemetry": { "enabled": false } }
   ```

To inspect what telemetry has been sent in the current session, send the LSP request `al-call-hierarchy/telemetryStatus` (also logged at startup).

## Building

Prerequisites:
- Rust 1.75+
- tree-sitter-al grammar: included as a git submodule at `tree-sitter-al/` — clone with
  `git clone --recurse-submodules`, or run `git submodule update --init` afterwards.
  Override the location with the `TREE_SITTER_AL_PATH` env var if it lives elsewhere.

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
al-call-hierarchy --project /path/to/al-project
```

LSP mode is the default when `--project` is omitted; passing `--project` alone switches
to CLI mode (index the project and report definition/call-site counts). Add `--analyze`
for a code-quality report (`--format text|json|csv`). There is no `--no-lsp` flag.

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

Enforced on every PR by a release-mode CI gate (3x tolerance); see CLAUDE.md for
currently-measured numbers and the bench command.

## Resolution Coverage

| Call Pattern | Resolvable |
|--------------|------------|
| Local procedures | Yes |
| Qualified calls (Object.Method) | Yes |
| Record methods | Yes |
| Event subscribers | Yes |
| External .app dependencies | Yes |

A genuinely dynamic (runtime-typed) call target is honestly reported as such rather than
guessed — it is never silently dropped or misclassified as resolved. See CLAUDE.md's
Resolution Coverage section for the full resolution taxonomy and current measured rates.

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

Diagnostic thresholds are configurable at two levels. Workspace config overrides global config per field (deep merge). All values are optional — missing values use defaults.

### Global Config

Set defaults for all projects in `~/.al-call-hierarchy/config.json`:

```json
{
  "diagnostics": {
    "complexity": { "warning": 8, "critical": 15 },
    "unusedProcedures": false
  }
}
```

### Workspace Config

Override per project in `{workspace}/.al-call-hierarchy.json`:

```json
{
  "diagnostics": {
    "complexity": { "enabled": true, "warning": 5, "critical": 10 },
    "parameters": { "enabled": true, "warning": 4, "critical": 7 },
    "lineCount": { "enabled": true, "warning": 20, "critical": 50 },
    "fanIn": { "enabled": true, "warning": 20 },
    "unusedProcedures": true
  }
}
```

### Resolution Order

1. Built-in defaults
2. Global config (`~/.al-call-hierarchy/config.json`)
3. Workspace config (`{workspace}/.al-call-hierarchy.json`)

Each field merges independently — a workspace config only needs to specify fields it wants to override.

Each category can be disabled entirely by setting `"enabled": false`.

| Setting | Default | Description |
|---------|---------|-------------|
| `complexity.enabled` | true | Enable/disable complexity diagnostics |
| `complexity.warning` | 5 | Cyclomatic complexity information threshold |
| `complexity.critical` | 10 | Cyclomatic complexity warning threshold |
| `parameters.enabled` | true | Enable/disable parameter count diagnostics |
| `parameters.warning` | 4 | Parameter count information threshold |
| `parameters.critical` | 7 | Parameter count warning threshold |
| `lineCount.enabled` | true | Enable/disable method length diagnostics |
| `lineCount.warning` | 20 | Method length information threshold |
| `lineCount.critical` | 50 | Method length warning threshold |
| `fanIn.enabled` | true | Enable/disable fan-in diagnostics |
| `fanIn.warning` | 20 | Incoming call count information threshold |
| `unusedProcedures` | true | Enable/disable unused procedure detection |

## License

This project is licensed under the GNU General Public License v3.0 - see the [LICENSE](LICENSE) file for details.
