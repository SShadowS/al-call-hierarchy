# Telemetry Schema Reference

This document describes the telemetry events that `al-call-hierarchy` may emit. It is intended for users who want to audit exactly what is sent, and for maintainers who query the resulting data in Azure Application Insights.

The implementation lives entirely under [`src/telemetry/`](../src/telemetry/) — the canonical event definitions are in [`src/telemetry/events.rs`](../src/telemetry/events.rs) (line numbers cited throughout). The serialization rules live in [`src/telemetry/events_attrs.rs`](../src/telemetry/events_attrs.rs) and [`src/telemetry/exporter.rs`](../src/telemetry/exporter.rs).

If anything in this document disagrees with the source, the source wins — please open an issue so this doc can be corrected.

## 1. Where events arrive

Events are sent over OTLP-compatible spans through the [`opentelemetry-application-insights`](https://crates.io/crates/opentelemetry-application-insights) exporter. They land in the **`dependencies`** table of the configured Azure Application Insights resource (not `customEvents` — this is a side effect of how `tracing-opentelemetry` translates `tracing::span!` into Azure's data model).

A typical KQL query to inspect them looks like:

```kusto
dependencies
| where timestamp > ago(1h)
| where customDimensions["telemetry.alch.schema_version"] == "1"
| project timestamp, name, customDimensions
```

All event-specific attributes use the `telemetry.alch.*` prefix to make them easy to filter. The event type is in the span `name` field (e.g. `resolution.miss`, `parser.error`).

## 2. Common envelope fields

Every event carries an envelope with the following dimensions, regardless of the event type. See [`EventEnvelope` at events.rs:268](../src/telemetry/events.rs#L268).

| Attribute | Type | Description |
|-----------|------|-------------|
| `telemetry.alch.schema_version` | integer | Wire-format version. Currently `1`. Increment on breaking changes; consumers should filter on this. |
| `telemetry.alch.install_id` | 16-char hex | Stable per-installation pseudonym derived from the local 32-byte salt (`blake3(salt)[..8]`). Lets the maintainer count distinct installations without learning who you are. |
| `telemetry.alch.workspace_id` | 16-char hex | Salted, domain-separated hash of the absolute workspace path. Lets misses from the same project cluster together without revealing the path. |
| `telemetry.alch.session_id` | u64 | Random 64-bit session identifier, regenerated on every process start. Lets one session's events be grouped without tying them to prior sessions. |
| `telemetry.alch.al_version` | string | The `al-call-hierarchy` crate version (e.g. `0.7.0`). |
| `telemetry.alch.grammar_version` | string | The bundled `tree-sitter-al` grammar revision. |
| `telemetry.alch.os` | string | One of `windows`, `macos`, `linux`, or the raw `std::env::consts::OS` value if unrecognised. |

The hashing scheme is described in [§7 Privacy](#7-privacy-notes).

## 3. Event types

### 3.1 `resolution.miss`

**Source:** [`ResolutionMiss` at events.rs:159](../src/telemetry/events.rs#L159).

Emitted when the resolver fails to bind a call site to a procedure definition. This is the highest-signal event: each miss represents a real-world AL pattern the maintainer has not yet handled. Five subkinds, distinguished by the `failure` attribute:

| Subkind | Meaning |
|---------|---------|
| `ObjectNotFound` | A qualified call `Object.Method` references an object the resolver cannot locate (not in workspace, not in `.alpackages`, not a built-in). Could indicate a missing dependency, a misspelled object name, or a construct the resolver doesn't recognise. |
| `ProcedureNotFound` | The object resolved, but the named procedure does not exist on it. Often points to a private/internal method the parser missed, or to dynamic dispatch the resolver cannot follow. |
| `UnresolvedUnqualified` | A bare `Method()` call that isn't a local procedure, trigger, or in-scope record method. Typically signals a missing `using` directive, a global helper, or a procedure declared in a way the indexer skipped. |
| `Ambiguous` | Multiple candidates matched and the resolver could not pick one. Suggests the workspace has overload-resolution rules the resolver doesn't model. |
| `UnsupportedConstruct` | A syntactic shape the resolver knows about but has not yet wired up (e.g. event-publisher invocations, complex member chains). |

#### Attributes

| Attribute | Type | Description |
|-----------|------|-------------|
| `telemetry.alch.failure` | enum | One of the five subkinds above. |
| `telemetry.alch.call_pattern` | enum | `Qualified`, `Unqualified`, or `MemberChain` (depth carried separately). |
| `telemetry.alch.member_chain_depth` | u8 | Present only when `call_pattern == MemberChain`. The number of `.` hops. |
| `telemetry.alch.callee_object_type` | enum or null | One of `Codeunit`, `Table`, `Page`, `Report`, `Query`, `XmlPort`, `Enum`, `Interface`, `PageExtension`, `TableExtension`, `EnumExtension`, `ControlAddIn`, or `Other`. Null when the object cannot be classified. |
| `telemetry.alch.callee_source` | enum | `Workspace`, `AppDependency`, `System`, or `Unknown` — where the resolver expected the callee to live. |
| `telemetry.alch.caller_object_type` | enum | The object type containing the failing call site (same enum as above). |
| `telemetry.alch.caller_context` | enum | `Procedure`, `Trigger`, `EventSubscriber`, or `Layout`. |
| `telemetry.alch.object_hash` | 32-char hex or null | Salted hash of the lowercased callee object name. Null when the call has no qualifier. |
| `telemetry.alch.procedure_hash` | 32-char hex | Salted hash of the lowercased procedure name. |
| `telemetry.alch.arg_count` | u8 | Number of arguments at the call site. |
| `telemetry.alch.name_len_object` | u16 or null | Byte length of the original object identifier. Helps detect long-name patterns without revealing them. |
| `telemetry.alch.name_len_procedure` | u16 | Byte length of the original procedure identifier. |
| `telemetry.alch.ts_node_path` | string | Tree-sitter node-kind path describing the syntactic shape (e.g. `member_expression>identifier`). Pure structural information — no identifier text. |
| `telemetry.alch.repeat_count` | u32 | How many times this exact pattern fired in the same dedup window before being flushed. `1` means "first occurrence in window". |

The combination of `failure`, `call_pattern`, `caller_object_type`, `callee_source`, and `ts_node_path` is the deduplication key — repeated misses with the same shape collapse into a single event with `repeat_count > 1`.

### 3.2 `parser.error`

**Source:** [`ParserError` at events.rs:176](../src/telemetry/events.rs#L176).

Emitted when tree-sitter reports a parse error or the parser encounters a node-kind it doesn't know how to handle. Three subkinds distinguished by `kind`:

| Subkind | Meaning |
|---------|---------|
| `TreeError` | The parsed tree contains one or more `ERROR` nodes — the source has a syntax problem tree-sitter recovered from. Useful as a corpus-quality signal. |
| `ParseFailed` | Tree-sitter returned no tree at all (rare; typically indicates an internal error or an OS-level read failure). |
| `UnknownNodeKind` | The parser visited a node kind that no case in `parser.rs` handles — usually a sign the grammar has been updated and the consumer hasn't caught up. |

#### Attributes

| Attribute | Type | Description |
|-----------|------|-------------|
| `telemetry.alch.kind` | enum | One of the three subkinds above. |
| `telemetry.alch.node_kind_hash` | 32-char hex or null | Salted hash of the unknown node-kind name (only set for `UnknownNodeKind`). The maintainer can correlate the same unknown kind across installations without learning the kind itself, then update the parser. |
| `telemetry.alch.file_hash` | 32-char hex | Salted hash of the file's path within the workspace. Lets the same file's repeated errors collapse, without leaking the path. |
| `telemetry.alch.file_extension` | string | Just the extension (`al`, `dal`, …). Useful for detecting non-AL files mistakenly indexed. |
| `telemetry.alch.file_size_bucket` | enum | `Sub1k`, `Sub10k`, `Sub100k`, or `Over100k` — coarse byte buckets. |
| `telemetry.alch.error_count` | u32 | Number of `ERROR` nodes in the tree (when `kind == TreeError`). |
| `telemetry.alch.repeat_count` | u32 | Same dedup-window semantics as `resolution.miss`. |

### 3.3 `handler.empty_result`

**Source:** [`HandlerEmpty` at events.rs:187](../src/telemetry/events.rs#L187).

Emitted when an LSP handler (`prepareCallHierarchy`, `incomingCalls`, `outgoingCalls`) returns an empty result for a request that pointed at a real definition. This is the only event that **may** be sampled at 10% — empty results are common for correctly-isolated procedures, so the unsampled volume would drown out actually-rare patterns. **An empty result is an outcome, not a failure**; the event exists so the maintainer can spot patterns where the call hierarchy *should* have shown something but didn't (e.g. a procedure with known callers showing zero incoming calls).

#### Attributes

| Attribute | Type | Description |
|-----------|------|-------------|
| `telemetry.alch.method` | string | The LSP method name (`prepareCallHierarchy`, `callHierarchy/incomingCalls`, `callHierarchy/outgoingCalls`). |
| `telemetry.alch.target_object_type` | enum | The `ObjectType` of the AL object containing the request target. |
| `telemetry.alch.target_kind` | enum | `Procedure`, `Trigger`, or `EventSubscriber`. |
| `telemetry.alch.object_hash` | 32-char hex | Salted hash of the target's object name. |
| `telemetry.alch.procedure_hash` | 32-char hex | Salted hash of the target's procedure name. |
| `telemetry.alch.repeat_count` | u32 | Same dedup-window semantics. |

### 3.4 `indexer.issue`

**Source:** [`IndexerIssue` at events.rs:197](../src/telemetry/events.rs#L197).

Emitted when the indexer encounters a file or dependency it can't process. Four subkinds:

| Subkind | Meaning |
|---------|---------|
| `MissingDependency` | An `app.json` declares a dependency that has no matching `.app` file in `.alpackages`. Common when a workspace was opened without first restoring symbols. |
| `AppParseFailed` | An `.app` file was found but the embedded `SymbolReference.json` could not be parsed (corrupt ZIP, unexpected schema, etc.). |
| `BrokenSymlink` | A path under the workspace points to nothing (often a bad symlink to an external deps folder). |
| `IoError` | A non-symlink IO error reading a file (permission denied, disk error, …). The specific `std::io::ErrorKind` is encoded in `detail_code`. |

#### Attributes

| Attribute | Type | Description |
|-----------|------|-------------|
| `telemetry.alch.kind` | enum | One of the four subkinds above. |
| `telemetry.alch.app_id_hash` | 32-char hex or null | Salted hash of the app GUID (only meaningful for `MissingDependency` / `AppParseFailed`). |
| `telemetry.alch.detail_code` | u16 | Subkind-specific numeric detail; for `IoError` this is a stable mapping of `std::io::ErrorKind`. Enumerated in `src/telemetry/events_attrs.rs`. |

### 3.5 `session.start`

**Source:** [`SessionStart` at events.rs:204](../src/telemetry/events.rs#L204).

Emitted exactly once at server startup, after the initial index completes. Captures workspace shape so the maintainer can reason about which kinds of projects produce which kinds of misses.

#### Attributes

| Attribute | Type | Description |
|-----------|------|-------------|
| `telemetry.alch.workspace_file_count` | u32 | Total number of files indexed. |
| `telemetry.alch.al_file_count_bucket` | enum | `Sub1k`, `Sub10k`, `Sub100k`, or `Over100k` — coarse bucket of how many AL files were indexed. (Not raw byte size — file *count*.) |
| `telemetry.alch.dependency_count` | u8 | Number of declared `app.json` dependencies. |
| `telemetry.alch.has_app_dependencies` | bool | Whether `.alpackages` contained any matching `.app` files. |
| `telemetry.alch.config_flags` | u32 | Bit-packed snapshot of which diagnostic features are enabled (complexity, parameters, line-count, fan-in, unused-procedures). The bit layout is in `src/telemetry/events_attrs.rs`. |
| `telemetry.alch.previous_session_unclean` | bool | True if the previous session terminated without writing a clean shutdown marker. Lets the maintainer correlate crash patterns with workspace shape — useful for triaging segfaults that never reach a panic handler. |

### 3.6 `session.summary`

**Source:** [`SessionSummary` at events.rs:214](../src/telemetry/events.rs#L214).

Emitted exactly once at clean shutdown. **Always unsampled** — this event is the maintainer's only window into how much was *observed* versus how much was *exported*. If your App Insights resource has ingestion-side sampling, configure a rule that exempts `name == "session.summary"`.

#### Attributes

| Attribute | Type | Description |
|-----------|------|-------------|
| `telemetry.alch.duration_secs` | u64 | Wall-clock seconds the session ran. |
| `telemetry.alch.unique_patterns` | u32 | Number of distinct dedup keys observed across the session. |
| `telemetry.alch.queue_full_drops` | u32 | Events dropped because the bounded MPSC channel between recorders and the exporter task was full. Healthy systems should see `0`; non-zero means the exporter is being out-paced. |
| `telemetry.alch.dedup_suppressed` | u32 | Total events suppressed by the LRU dedup cache (rolled into other events' `repeat_count`). |
| `telemetry.alch.export_attempts` | u32 | OTLP export calls the SDK attempted. |
| `telemetry.alch.export_failures` | u32 | Of those, how many returned an error from App Insights. Persistent non-zero values usually indicate a wrong connection string or a network egress block. |
| `telemetry.alch.observed_by_kind` | u32[14] | One slot per `LeafKind` (see [`ALL_LEAF_KINDS` at events.rs:29](../src/telemetry/events.rs#L29)) — total events observed before dedup. |
| `telemetry.alch.exported_by_kind` | u32[14] | Parallel array of events actually exported. The difference between `observed` and `exported` per slot is the dedup compression ratio. |

The 14 slots, in order, correspond to the leaf-kind enumeration:

```
0  resolution.object_not_found
1  resolution.procedure_not_found
2  resolution.unresolved_unqualified
3  resolution.ambiguous
4  resolution.unsupported_construct
5  parser.tree_error
6  parser.parse_failed
7  parser.unknown_node_kind
8  handler.empty_result
9  indexer.missing_dependency
10 indexer.app_parse_failed
11 indexer.broken_symlink
12 indexer.io_error
13 session.start
```

`session.summary` itself does not appear in either array — it's meta and not self-counted (see `EventKind::leaf` at events.rs:238 returning `None` for the summary variant).

## 4. Sampling

| Event | Sampling rate |
|-------|---------------|
| `resolution.miss` | 100% (deduplicated by structural key) |
| `parser.error` | 100% (deduplicated by structural key) |
| `handler.empty_result` | 10% (head sampling) |
| `indexer.issue` | 100% |
| `session.start` | 100% (one per session) |
| `session.summary` | 100% (one per clean shutdown — never sample this) |

Dedup runs in-process before sampling, in a fixed-size LRU keyed by the event's structural shape. The `repeat_count` field on each event records the suppression count from the same window.

## 5. Lifecycle

```
process start
  └─ load consent (env, CLI, config) ─→ if disabled, no telemetry runtime starts
       └─ load/create install salt at ~/.al-call-hierarchy/install-id
            └─ initialise OTLP exporter with baked connection string
                 └─ session.start emitted
                      └─ recorders fire as resolutions/parses/handlers run
                           └─ pipeline batches, dedups, exports
process stop (clean)
  └─ session.summary emitted
       └─ exporter flushed
            └─ session marker written cleanly
```

If the process exits without flushing (panic, SIGKILL, segfault), the next session sees `previous_session_unclean = true` on its `session.start`.

## 6. Privacy notes

### What is sent

- Salted, domain-separated, truncated BLAKE3 hashes of AL identifier names, file paths, app GUIDs, and tree-sitter node-kind names.
- Enumerated structural information: object types, failure categories, call patterns, file-size buckets, OS, version strings.
- Coarse counts: argument counts (u8), error counts (u32), file counts (u32).
- Identifier byte lengths (u16) for objects and procedures.
- The wall-clock timestamp of each event.

### What is never sent

- Raw AL identifier text (procedure names, object names, variable names, parameter names).
- Raw file paths, URIs, or workspace paths.
- File contents or any source code (no snippets, no quoted text, no error messages from AL itself).
- The 32-byte installation salt — it never leaves the local machine.
- Operating-system usernames, machine names, IP addresses (App Insights may capture client IP at the network layer; the application code does not include any).
- Environment variables.
- App.json contents beyond the structural counts above.

### Hashing scheme

All identifier-derived attributes pass through [`src/telemetry/hash.rs`](../src/telemetry/hash.rs):

- **Salt**: a 32-byte cryptographically random value generated once per installation and stored at `~/.al-call-hierarchy/install-id`. Never transmitted.
- **Algorithm**: `blake3::Hasher::new_keyed(salt)` with the salt as the keying material — equivalent to a salted MAC, not a plain hash. An attacker who intercepts hashes cannot brute-force them without the salt.
- **Domain separation**: each kind of input is prefixed with a fixed tag (`object:`, `procedure:`, `app_id:`, `file:`, `workspace:`, `node_kind:`) before hashing, so the same string in two contexts produces two unrelated hashes.
- **Truncation**: 128 bits (32-char hex) for queryable AL-identifier hashes; 64 bits (16-char hex) for `install_id` and `workspace_id` where collision risk between installations is the concern, not preimage resistance.
- **Normalisation**: identifiers are lowercased before hashing (AL is case-insensitive). Inputs are truncated to 4096 bytes to bound hashing cost.
- **Public install_id**: derived as `blake3(salt)[..8]` — a one-way derivative of the salt that lets the maintainer count installations without the maintainer being able to compute any other identifier hash for that installation.

### Threat model

The hashing scheme defends against a passive attacker who reads the App Insights data: even with the full corpus of one installation's events, identifier names cannot be recovered without the local salt, and identifiers cannot be cross-correlated across installations.

It does *not* defend against an attacker with both the data **and** access to your local `install-id` file. If you need stronger guarantees, disable telemetry entirely.

## 7. How to disable

Three independent off-switches; **any one of them disables the runtime entirely** — no salt is read, no exporter starts, no events are buffered.

1. **Environment variable** (precedence: highest):
   - `AL_CH_TELEMETRY=0` (project-specific)
   - `DO_NOT_TRACK=1` (cross-tool community standard)

2. **CLI flag**: launch with `--no-telemetry`.

3. **Config file** at `~/.al-call-hierarchy/config.json`:

   ```json
   { "telemetry": { "enabled": false } }
   ```

In addition, telemetry is **off by default** in:

- Debug builds (`cargo build` without `--release`).
- Test runs (any binary launched under `cargo test`).
- CI environments (`CI`, `GITHUB_ACTIONS`, `GITLAB_CI`, `BUILDKITE`, `CIRCLECI`, `JENKINS_URL`, etc. — full list in `src/telemetry/consent.rs`).

To inspect what's been sent in the current session, send the LSP request `al-call-hierarchy/telemetryStatus`. The response includes per-kind observed/exported counts and the consent decision path. The same information is logged at startup at the `info` level.

## 8. Schema versioning

The wire format is versioned via `telemetry.alch.schema_version` (currently `1`, defined in `events.rs:9`). Any backward-incompatible change — renamed attributes, removed enum variants, repurposed fields — bumps this number.

Maintainer queries should always filter on the version they know:

```kusto
dependencies
| where customDimensions["telemetry.alch.schema_version"] == "1"
```

## 9. Quick KQL reference

A few queries the maintainer uses regularly:

```kusto
// Top resolution-miss patterns this week
dependencies
| where timestamp > ago(7d)
| where name == "resolution.miss"
| extend
    failure = tostring(customDimensions["telemetry.alch.failure"]),
    pattern = tostring(customDimensions["telemetry.alch.call_pattern"]),
    callee_kind = tostring(customDimensions["telemetry.alch.callee_object_type"]),
    ts_path = tostring(customDimensions["telemetry.alch.ts_node_path"])
| summarize hits = sum(toint(customDimensions["telemetry.alch.repeat_count"]))
            by failure, pattern, callee_kind, ts_path
| order by hits desc
| take 50
```

```kusto
// Are we losing events to backpressure?
dependencies
| where timestamp > ago(1d)
| where name == "session.summary"
| extend drops = toint(customDimensions["telemetry.alch.queue_full_drops"])
| where drops > 0
| project timestamp, install = tostring(customDimensions["telemetry.alch.install_id"]), drops
```

```kusto
// Crash detection — sessions that didn't flush cleanly last time
dependencies
| where name == "session.start"
| where customDimensions["telemetry.alch.previous_session_unclean"] == "true"
| summarize unclean_sessions = count() by bin(timestamp, 1d)
| render timechart
```

---

If you spot a discrepancy between this document and the source, the source is canonical. Open an issue at <https://github.com/SShadowS/al-call-hierarchy/issues> and the doc will be corrected.
