# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0] - 2025-02-03

### Added
- **Event Subscriber Integration**: Event subscribers are now shown in the call hierarchy
  - Parses `[EventSubscriber]` attributes to extract publisher object and event name
  - Event subscribers appear as "callers" in `incomingCalls` for the subscribed events
  - Shows `[EventSubscriber]` tag in the call hierarchy detail

- **Code Lens Support**: Reference counts and quality metrics displayed above procedures
  - Shows "N references | complexity: X, lines: Y, params: Z" lens above each procedure/trigger definition
  - Displays cyclomatic complexity, line count, and parameter count for each procedure
  - Highlights procedures with 0 references as potential dead code
  - Click to navigate to the references (via `al-call-hierarchy.showReferences` command)

- **Unused Procedure Detection**: Diagnostics for procedures with no callers
  - Publishes `HINT` severity diagnostics for unused procedures
  - Excludes triggers and event subscribers (they're called implicitly)
  - Tagged with `UNNECESSARY` for IDE-specific rendering (strikethrough, etc.)

- **Code Quality Diagnostics**: Warnings for potential code quality issues
  - High fan-in warning: procedures called by more than 20 other procedures
  - Long method warning: procedures spanning more than 50 lines
  - Diagnostics published at `INFORMATION` severity

- **External .app dependency support**: The server now resolves calls to procedures defined in compiled .app packages
  - Automatically parses `app.json` to discover declared dependencies
  - Finds matching .app files in the `.alpackages` folder with version matching
  - Extracts procedure definitions from `SymbolReference.json` inside .app files
  - Shows "(from AppName)" in call hierarchy for resolved external calls
  - Supports all standard BC object types: Codeunits, Tables, Pages, Reports, etc.

### Changed
- **Memory optimization**: `ExternalSource.app_version` now uses interned `Symbol` instead of `String`, reducing memory usage when loading large .app dependencies (~50-100MB savings for BC base apps)

### New capabilities
- `textDocument/codeLens` - Returns reference counts for all procedures in a file
- Diagnostics publishing via `textDocument/publishDiagnostics`

### New modules
- `app_package.rs` - Parser for .app files (ZIP with 40-byte NAVX header)
- `dependencies.rs` - Dependency discovery and resolution from app.json

### Dependencies
- Added `zip` crate for .app file extraction
- Added `roxmltree` crate for NavxManifest.xml parsing
