# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Page `SourceTableTemporary = true` capture + implicit `Rec`/`xRec` `Known(true)`
  override (Task 5 / ts5, G4, RV-8):
  - `project_file` (`l3_workspace.rs`) now reads the `SourceTableTemporary` property
    for Page and PageExtension objects via `read_object_property`, setting
    `L3Object.source_table_temporary = Some(true)` on an exact case-insensitive match
    against `"true"` (trim + lowercase); `Some(false)` when present but not `"true"`;
    `None` when absent. Never `.contains()` / string-sniffing; engine never throws.
    `L3Object` is not serialised into any gate surface, so this never moves a golden.
  - Page-level override pass added to `resolve_routine_record_types` (`record_types.rs`),
    running after the table-level override: when the current object's
    `source_table_temporary == Some(true)`, every record op whose
    `record_variable_name` (lowercased) is `rec` or `xrec` is force-upgraded to
    `Known(true)`. Both `rec` AND `xrec` (RV-8: xRec alongside Rec). Purely ADDITIVE
    toward `Known(true)` — never downgrades; `SourceTableTemporary = true` is a
    structural page property that cannot be carried by physical-source pages, so the
    upgrade is sound (suppression-safe direction).
- Native `TableType = Temporary` capture + table-level override precedence
  (Task 4 / ts4, G3, RV-8):
  - `index_table` (`l3_workspace.rs`) now reads the object-level `TableType`
    property via `read_object_property` and sets `L3Table.is_temporary = true`
    on an EXACT case-insensitive match (trim + lowercase + `== "temporary"`;
    never `.contains()` / string-sniffing). A missing/other value → `false`
    (conservative). This is the only allowed temp signal — a structural property
    read. `L3Table` is not serialised into any gate surface, so this never moves
    a golden.
  - Final override pass in `resolve_routine_record_types` (`record_types.rs`),
    running AFTER all `table_id` resolution (declared vars, ops, lexical fallback,
    implicit Rec/xRec pass-3): for every record op whose resolved table is
    `is_temporary`, force `temp_state = Known(true)`, and likewise for the matching
    record VARIABLE. The "one precedence rule everywhere" — table-level temp WINS
    over keyword / no-keyword / by-value / by-var / `ParameterDependent(i)`. So a
    by-var PARAM of a temp table reports `Known(true)`, not the L2-stamped `PD(i)`
    (RV-8). Purely ADDITIVE toward `Known(true)`: only upgrades, never downgrades a
    `Known(true)` and never forces `Known(false)`. Table lookup uses the existing
    `SymbolTable::table_by_id`.
  - `TableView::is_temporary()` test-facing accessor.
- `extract_object_global_record_vars` in `scope.rs` (Task 2 / ts2, G1): captures
  the `temporary_keyword` on object-level `var_section` record variable declarations,
  producing `PRecordVariable` with `temp_state = Known(true/false)` and
  `scope = Some("global")`.  Non-record vars are skipped; `preproc_conditional_var_block`
  and dataitem-scoped var sections are conservative gaps (fall to Unknown, RV-8).
  Not yet wired into L3 projection (Task 3).
- Additive model fields for temp-state tracking epoch (Task 1 / ts1):
  - `PRecordVariable.scope: Option<String>` (`"local"` | `"parameter"` |
    `"global"`; `skip_serializing_if` keeps goldens stable; populated by later tasks).
  - `L3RecordVariable.scope: Option<String>` — forwarded from L2; field-allowlisted
    L3 projection never reaches goldens.
  - `L3Table.is_temporary: bool` (default `false`) — additive; L3Table is not
    serialised into any gate surface.
  - `L3Object.source_table_temporary: Option<bool>` (default `None`) — additive;
    L3Object is not serialised into any gate surface.
  - `AbiTable.is_temporary: bool` (default `false`) — slot for ABI temp capture
    (populated by Task 6).
  - `AbiParameter.is_temporary: bool` (default `false`) — slot for parameter
    `temporary` modifier (populated by Task 6).
  - `RawTypeDef.temporary: Option<bool>` (`#[serde(rename = "Temporary")]`) —
    deserialises the `Temporary` field from `SymbolReference.json`; consumed by
    Task 6.

### Fixed
- Object-global record vars are now promoted into EACH routine's
  `record_variables` during L3 assembly (Task 3 / ts3, G2), and member-var record
  operations re-derive their `temp_state` from the promoted set — the root-cause
  fix for the CDO false-critical class (a codeunit member
  `Files: Record "CDO File" temporary;` was never seen by the L2 body walk, so
  `Files.DeleteAll()` carried `tempState = Unknown`, fired a false critical, and
  d1 stamped "(temp state uncertain)"). Promotion honors AL shadowing: a routine's
  own param/local of the same name shadows the global (innermost wins). Shadowed
  globals are NOT promoted, keeping `record_variables` NAME-UNIQUE — which
  preserves the documented pass-1 `var_index_by_name` last-wins invariant in
  `record_types.rs` (a name-duplicated list would let the global clobber the
  local). The op `temp_state` backfill lives in `record_types.rs` pass-2a: when an
  op matches its declaring record var, `op.temp_state` is copied from that var
  (alongside the existing `table_id` / `record_variable_id` derivation).
- `record_types.rs` pass 2b `variable_decl_by_name` map changed from last-wins
  (unconditional `insert`) to first-wins (`entry().or_insert()`) so that a
  procedure-local declaration always shadows an object-global with the same name
  — the correct AL innermost-scope rule and a prerequisite for the tempState
  backfill epoch (RV-5).

## [0.7.0] - 2026-05-06

### Added
- Anonymous, opt-out failure-diagnostics telemetry (Azure App Insights).
  - Captures resolution misses, parser errors, indexer issues, and handler outcomes.
  - All AL identifier names hashed with a per-installation 32-byte salt that stays local.
  - Three disable mechanisms: `DO_NOT_TRACK=1`, `--no-telemetry`, `~/.al-call-hierarchy/config.json` `telemetry.enabled=false`.
  - Off by default in debug, test, and CI builds.
  - LSP request `al-call-hierarchy/telemetryStatus` for runtime introspection.
  - Schema documented in `docs/telemetry.md`.
  - Fire-and-forget export: `BatchSpanProcessor` on a dedicated tokio current-thread runtime; HTTP calls are non-blocking, individual export failures are silently dropped, and LSP request threads are never affected by network state. 10s/5s reqwest timeouts cap any single HTTP call; shutdown is bounded to a 3s budget.

## [0.5.0] - 2026-03-22

### Changed
- **BREAKING: Migrated to tree-sitter-al V2 grammar** — all tree-sitter queries and parsing logic updated for the rewritten grammar
  - `procedure name:` and `trigger_declaration name:` now hold `(identifier)`/`(quoted_identifier)` directly (no `(name)`/`(trigger_name)` wrapper nodes)
  - `member_expression` field renamed from `property:` to `member:`
  - `parameter` field renamed from `parameter_name:` to `name:`
  - Individual `*_property` nodes replaced by unified `property` node with `name:` and `value:` fields
  - `preproc_split_codeunit_declaration` renamed to `preproc_split_declaration`
- **tree-sitter-al is now a git submodule** instead of an external sibling directory — clone with `--recurse-submodules`
- `build.rs` defaults to `tree-sitter-al` (submodule) instead of `../tree-sitter-al`

### Removed
- `field_access` query pattern — merged into `member_expression` with `quoted_identifier` as member
- `named_trigger` / `onrun_trigger` handling — unified into `trigger_declaration`
- `extract_trigger_name()` helper — no longer needed with V2 grammar
- `property_display_name()` helper — replaced by reading `property_name` field directly

### Fixed
- EventSubscriber detection now correctly handles V2 attribute-as-sibling model (attributes are siblings of procedures, not children)

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
