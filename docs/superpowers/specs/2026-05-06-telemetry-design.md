# Telemetry Design — Anonymous Failure Diagnostics

- **Date:** 2026-05-06
- **Status:** Revised after expert review (gpt-5.4-pro). Pending user re-approval.
- **Owner:** SShadowS
- **Target version:** 0.7.0
- **Schema version:** 1

## 1. Problem & Goal

`al-call-hierarchy` is an early-stage LSP server. Real-world AL workspaces hit resolution gaps the maintainer cannot reproduce locally — most notably calls into `.app` dependency packages and tree-sitter V2 grammar shapes the resolver doesn't handle yet.

**Goal:** Capture these gaps automatically from real installs, without exposing customer code, so the maintainer can prioritize fixes and write fixtures.

**Non-goals:**
- Full APM / performance monitoring
- User-facing analytics
- Crash reporting beyond what `session.summary` already covers
- Telemetry from VSCode extension internals (separate concern)

## 2. Constraints & Decisions

| Constraint | Decision |
|---|---|
| Must not block LSP request threads | Hot path is sync hash + LRU lookup + non-blocking `try_send`. ~5µs worst case. Network I/O lives on dedicated background thread. |
| Must not leak customer AL identifiers | All names hashed with per-installation random 32-byte salt. Salt stays local. |
| Must produce data the maintainer can debug from | Hashes cluster (same name in same install collides), structural fields (object types, sources, failure kinds, ts_node_path) reveal patterns. |
| Cost not a concern (Azure MVP) | Send on detected failures, opt-out, no aggressive sampling. |
| Vendor lock-in concern | OpenTelemetry SDK API for instrumentation. Exporter swap = change one crate dependency. Initial exporter is `opentelemetry-application-insights` (direct Azure ingest); swapping to OTLP collector for Honeycomb/Grafana is a single-file change. |
| App Insights ingestion compatibility | Use `opentelemetry-application-insights` crate, not generic `opentelemetry-otlp`. Generic OTLP/HTTP does not work against App Insights ingestion endpoints; the Azure-specific exporter speaks the breeze/track protocol with connection-string auth. |
| Trust on dev machines | Three opt-out mechanisms, prominent README disclosure, runtime status endpoint, telemetry source in one auditable directory. |

## 3. Architecture

```
LSP request thread(s)              Background telemetry thread
─────────────────────              ──────────────────────────
parser.rs / handlers.rs                Tokio current_thread runtime
    │                                       │
    │ tracing::event!()                     │
    ▼                                       │
┌─────────────────┐                         │
│ telemetry::emit │  (sync, non-blocking)   │
│   - check opt-out                         │
│   - hash names                            │
│   - dedup LRU lookup                      │
│   - try_send → mpsc                       │
└────────┬────────┘                         │
         │ try_send (drop on full)          │
         ▼                                  │
    bounded mpsc (cap 2048)  ───────────►   │
                                            ▼
                                   ┌──────────────────┐
                                   │ BatchLogProcessor│
                                   │ - 5s flush       │
                                   │ - 512 batch      │
                                   │ - exp backoff    │
                                   └────────┬─────────┘
                                            ▼
                                   opentelemetry-application-insights
                                   exporter (HTTPS POST to breeze
                                   endpoint with connection-string auth)
                                            │
                                            ▼
                                   Azure Application Insights
```

**Key invariant:** LSP request threads never await network I/O. All telemetry is best-effort; failures degrade silently.

**`session.summary` is NOT routed through the mpsc queue.** It is constructed by the background thread *after* the producer side closes and the queue drains. Counters live in atomics shared between producer and background thread. This guarantees the summary reflects reality even under shutdown pressure and is never dropped by queue-full or dedup logic.

## 4. Module Layout

```
src/telemetry/
    mod.rs           — public API: record_*, init, shutdown, status
    config.rs        — TelemetryConfig: enabled, endpoint, flush_interval, sample_rate, install_id
    consent.rs       — opt-out resolution: env > CLI > config.json > DO_NOT_TRACK
    install_id.rs    — load/generate ~/.al-call-hierarchy/installation-id (32-byte salt)
    hash.rs          — blake3_keyed(salt, name)[..8] → 16-char hex
    events.rs        — event structs + EventKind enum (6 outer variants encoding 14 leaf event types + 1 session summary). Single source of truth for kind-count via const ALL_LEAF_KINDS: [LeafKind; 14]
    dedup.rs         — LRU cache, key = (kind, object_hash, procedure_hash, callee_object_type, workspace_id)
    pipeline.rs      — mpsc sender, try_send wrapper, atomic pipeline counters (observed, dedup_suppressed, queue_full_drops)
    exporter.rs      — tokio runtime + OTel SDK init, opentelemetry-application-insights exporter, atomic counters (export_attempts, export_failures, exported), shutdown drain
    session_marker.rs— ~/.al-call-hierarchy/session.lock created at startup, deleted on graceful shutdown. Presence at startup → previous session was unclean
    summary.rs       — pulls atomic counters at shutdown (after queue drain), constructs SessionSummary directly on background thread, exports unsampled
```

### Public API

```rust
pub fn init(cfg: &Config) -> Result<TelemetryHandle>;
pub fn shutdown(handle: TelemetryHandle);  // drains queue, 2s timeout
pub fn record_resolution_miss(kind: ResolutionMissKind, ctx: &CallContext);
pub fn record_parser_error(kind: ParserErrorKind, file: &Path);
pub fn record_indexer_issue(kind: IndexerIssueKind, detail: IndexerDetail);
pub fn record_handler_empty(method: &'static str, item: &CallHierarchyItem);
pub fn record_session_start(stats: SessionStats);
pub fn status() -> TelemetryStatus;
```

When telemetry disabled, `init` returns a no-op handle. All `record_*` fns return after a single atomic load + branch. **No salt file created, no marker file written, no background thread spawned, no atomic counters allocated beyond the `enabled` flag.** "Off" leaves zero filesystem and runtime trace.

### Cargo features

`telemetry` feature flag, on by default. `cargo build --no-default-features` strips the entire OTel dep tree. All `record_*` bodies wrapped in `#[cfg(feature = "telemetry")]` with stub no-op fallbacks.

### Call sites

| Module | Location | Event |
|---|---|---|
| `parser.rs` | call resolution path | `record_resolution_miss` |
| `parser.rs` | tree-sitter error walk after parse | `record_parser_error` |
| `indexer.rs` | dependency loading from `.alpackages` | `record_indexer_issue` |
| `handlers.rs` | end of `incoming_calls` / `outgoing_calls` | `record_handler_empty` (sampled 10%) |
| `server.rs` | startup | `record_session_start` |
| `server.rs` | shutdown | `shutdown(handle)` → emits `session.summary` |

## 5. Data Model

### Common envelope

```rust
pub struct EventEnvelope {
    schema_version: u8,             // start at 1; bump on breaking field changes
    timestamp: SystemTime,
    install_id: String,             // 16-char hex (8 bytes from BLAKE3 of salt)
    al_version: &'static str,       // env!("CARGO_PKG_VERSION")
    grammar_version: &'static str,  // "v2"
    os: &'static str,               // "windows" / "linux" / "macos"
    session_id: u64,                // random per process start
    workspace_id: String,           // 16-char hex per workspace root: blake3_keyed(salt, b"workspace:" || abs_path)[..8] — scopes dedup keys
    event: EventKind,
}
```

### Event kinds

Six outer `EventKind` variants encode 14 leaf event types plus a session summary:

- `ResolutionMiss` — 5 leaves: `ObjectNotFound`, `ProcedureNotFound`, `UnresolvedUnqualified`, `Ambiguous`, `UnsupportedConstruct`
- `ParserError` — 3 leaves: `TreeError`, `ParseFailed`, `UnknownNodeKind`
- `HandlerEmpty` — 1 leaf, modeled as **outcome `EmptyResult`** not failure (legitimate for leaf functions). Sampled 10%; method field distinguishes `incomingCalls` vs `outgoingCalls`. Dashboards must NOT roll this into failure rate without explicit filter.
- `IndexerIssue` — 4 leaves: `MissingDependency`, `AppParseFailed`, `BrokenSymlink`, `IoError`
- `SessionStart` — 1 leaf
- `SessionSummary` — meta event, emitted once per session at shutdown

```rust
pub enum EventKind {
    ResolutionMiss(ResolutionMiss),
    ParserError(ParserError),
    HandlerEmpty(HandlerEmpty),
    IndexerIssue(IndexerIssue),
    SessionStart(SessionStart),
    SessionSummary(SessionSummary),
}

pub struct ResolutionMiss {
    failure: ResolutionFailure,
    // ObjectNotFound | ProcedureNotFound | UnresolvedUnqualified | Ambiguous | UnsupportedConstruct
    call_pattern: CallPattern,
    // Qualified | Unqualified | MemberChain { depth: u8 }
    callee_object_type: Option<ObjectType>,
    callee_source: CalleeSource,
    // Workspace | AppDependency | System | Unknown
    caller_object_type: ObjectType,
    caller_context: CallerContext,
    // Procedure | Trigger | EventSubscriber | Layout
    object_hash: Option<String>,    // 32-char hex (16 bytes), None for Unqualified
    procedure_hash: String,         // 32-char hex (16 bytes)
    arg_count: u8,
    name_len_object: Option<u16>,
    name_len_procedure: u16,
    ts_node_path: String,
    repeat_count: u32,              // dedup-merged
}

pub struct ParserError {
    kind: ParserErrorKind,
    // TreeError | ParseFailed | UnknownNodeKind { node_kind: String }
    file_hash: String,              // hash of relative path inside workspace root
    file_extension: String,         // "al" | "dal"
    file_size_bucket: SizeBucket,
    error_count: u32,
    repeat_count: u32,
}

pub struct HandlerEmpty {
    method: &'static str,           // "incomingCalls" | "outgoingCalls"
    target_object_type: ObjectType,
    target_kind: DefinitionKind,
    object_hash: String,
    procedure_hash: String,
    repeat_count: u32,
}

pub struct IndexerIssue {
    kind: IndexerIssueKind,
    // MissingDependency | AppParseFailed | BrokenSymlink | IoError
    app_id_hash: Option<String>,
    detail_code: u16,
}

pub struct SessionStart {
    workspace_file_count: u32,
    al_file_count_bucket: SizeBucket,
    dependency_count: u8,
    has_app_dependencies: bool,
    config_flags: ConfigFlags,
    previous_session_unclean: bool, // session_marker existed at startup → prior run was killed
}

pub struct SessionSummary {
    duration_secs: u64,
    unique_patterns: u32,
    // Pipeline integrity counters (telemetry-side, separate from app counts)
    queue_full_drops: u32,
    dedup_suppressed: u32,
    export_attempts: u32,
    export_failures: u32,
    // App-side observed counts per leaf event type (every record_* call increments,
    // before dedup or queue logic). Single source of truth for "what actually happened".
    observed_by_kind: [u32; 14],
    // Telemetry-side exported counts per leaf event type (what survived dedup + queue + export).
    // Compare to observed_by_kind to estimate signal fidelity.
    exported_by_kind: [u32; 14],
}
```

### Hashing rules

- **Domain separation:** prefix every hashed value with a domain tag, e.g. `blake3_keyed(salt, b"object:" || name)`. Domains: `object`, `procedure`, `app_id`, `file`, `workspace`, `node_kind`. Prevents cross-field collision and ambiguity.
- **128-bit truncation:** take first 16 bytes of digest → 32-char lowercase hex. 64-bit was tight against birthday attacks at thousands of installs × hundreds of identifiers each; 128-bit makes collision negligible.
- **Exception:** `install_id` and `workspace_id` are 8 bytes (16 hex chars) — published as coarse fingerprints. `install_id = blake3(salt)[..8]`. `workspace_id = blake3_keyed(salt, b"workspace:" || abs_path)[..8]`. Smaller than 128-bit identifier hashes since they're not used as queryable cardinality dimensions.
- AL identifiers normalized to lowercase before hashing (AL is case-insensitive)
- Salt = installation-id, generated via `OsRng`, stored at `~/.al-call-hierarchy/installation-id` mode 0600 on Unix
- Keyed mode (not `update()`) to avoid length-extension surprises
- Truncate input to 4KB before hashing (paranoia; AL identifiers are bounded ~120 chars)
- `node_kind` strings in `UnknownNodeKind` are hashed too — they're tree-sitter grammar node names (public schema) but go through the same hash path for consistency. Maintainer can map back via the public V2 grammar source.

### OTLP mapping

Each event → one OTLP log record. `severity_text = "INFO"`, `body = event_kind.as_str()`, all fields as attributes namespaced `telemetry.alch.*`.

### Explicitly NOT in payload

- AL identifier names in clear (only hashes)
- File paths or workspace names (only hashed file path + extension + size bucket)
- Hostname, machine name, username, IP address
- Source code, comments, string literals
- App.json contents beyond hashed dependency GUIDs
- Environment variable values

## 6. Pipeline

### Hot path (every `record_*` call)

1. Atomic load `TELEMETRY_ENABLED.load(Relaxed)` → false short-circuits in 1ns.
2. `OBSERVED_BY_KIND[kind].fetch_add(1, Relaxed)` — app-side count, always recorded regardless of downstream fate.
3. Build event struct on stack.
4. Hash names via `blake3_keyed` with domain tag (~200ns per identifier).
5. Build dedup key (includes `workspace_id` so different projects in same session don't cross-suppress).
6. `dedup.check(key)`:
   - First seen → emit full event.
   - Repeat within TTL → `DEDUP_SUPPRESSED.fetch_add(1, Relaxed)`, no send.
   - TTL expired → flush accumulated `repeat_count` as one event, restart.
7. `tx.try_send(event)`:
   - Ok → done (background thread will increment `EXPORTED_BY_KIND` on successful export).
   - Full → `QUEUE_FULL_DROPS.fetch_add(1, Relaxed)`, return.

Worst-case budget: ~5µs.

### Counter dimensions (atomic, shared producer↔background)

```
OBSERVED_BY_KIND[14]      — incremented at hot-path entry, before any filtering
DEDUP_SUPPRESSED          — incremented when dedup says "repeat"
QUEUE_FULL_DROPS          — incremented on try_send failure
EXPORT_ATTEMPTS           — incremented per batch attempt by background thread
EXPORT_FAILURES           — incremented per failed batch (after retry exhaustion)
EXPORTED_BY_KIND[14]      — incremented per successful export
```

These power `SessionSummary` and let the maintainer distinguish app behavior from telemetry pipeline state.

### Background thread

- Tokio current-thread runtime on dedicated `std::thread::spawn`.
- BatchLogProcessor: 5s timer OR 512 events.
- OTLP HTTP exporter: POST to `OTEL_EXPORTER_OTLP_ENDPOINT/v1/logs` with Azure Monitor instrumentation key header.
- Retry: exponential backoff 1s/2s/4s, max 3 attempts.
- Failure after 3: drop batch, increment `EXPORT_FAILURES`.

### Dedup

- `lru::LruCache<DedupKey, DedupEntry>`, capacity 1024.
- TTL 5 min. Periodic sweep every 30s evicts TTL'd entries and emits summary events with `repeat_count`.
- Mutex held only for LRU operation; hashing happens outside.
- **Workspace-scoped:** `DedupKey` includes `workspace_id`. Switching projects mid-session cannot suppress same-shape patterns from a different workspace. New workspace gets a clean slate.
- Dedup is consumer-side (after hot-path emit). Bursts of identical failures still hit the queue; that's intentional — `OBSERVED_BY_KIND` records the burst, `DEDUP_SUPPRESSED` records what dedup absorbed, `QUEUE_FULL_DROPS` records what backpressure ate. All three are reported in summary so signal fidelity is auditable.

### Sampling

- `handler.empty_result` only: `if counter.fetch_add(1, Relaxed) % 10 == 0 { record }`. Cheap, no RNG.

### Backpressure

- Queue full → drop, increment counter (reported in `session.summary`).
- Export failure 3× → drop batch, increment counter.
- Hot path never blocks.

### Shutdown

1. Server receives `exit` notification.
2. Calls `telemetry::shutdown(handle)`.
3. Producer side: flips `TELEMETRY_ENABLED` to false (no new events accepted), closes mpsc sender.
4. Background thread: drains rx until disconnect (1s budget).
5. Background thread: snapshots all atomic counters, constructs `SessionSummary` directly (NOT via mpsc — guaranteed delivery, never deduped, never sampled). Pushes summary to exporter.
6. Background thread: force-flushes exporter (2s budget).
7. Background thread: deletes `~/.al-call-hierarchy/session.lock` (clean-shutdown marker).
8. Background thread exits.
9. Total budget ≤ 3s; if exceeded, OS reclaims on process exit. Session marker remains → next session detects unclean exit.

### Memory bound

- mpsc queue: 2048 × ~256 bytes = ~512KB max
- LRU dedup: 1024 × ~80 bytes = ~80KB
- Tokio runtime: ~200KB baseline
- Atomic counter array: 14 × 2 × 8 bytes = 224 bytes
- Total: ~1MB when active.

### Crash detection (session marker)

- Startup: if telemetry enabled, write `~/.al-call-hierarchy/session.lock` (empty file). If file already existed, set `previous_session_unclean = true` on next `SessionStart` event.
- Graceful shutdown: background thread deletes the marker after summary export.
- SIGKILL / OS crash / power loss: marker survives → next launch detects.
- Cost: one file create + one delete per session. Negligible.
- Marker is never written when telemetry is off.

## 7. Configuration

### Config file

`~/.al-call-hierarchy/config.json`, auto-created on first run **only when telemetry is enabled** (off → no file written):

```json
{
  "telemetry": {
    "enabled": true,
    "connection_string": "<from-azure-portal>",
    "flush_interval_secs": 5,
    "batch_size": 512,
    "queue_capacity": 2048,
    "dedup_ttl_secs": 300,
    "handler_empty_sample_rate": 10
  }
}
```

### Resolution order for `enabled` (first match wins)

**Hard-off tier (cannot be overridden):**
1. `DO_NOT_TRACK=1` env → off
2. `--no-telemetry` CLI → off
3. `AL_CH_TELEMETRY=0` env → off

**Hard-on tier (overrides defaults):**
4. `AL_CH_TELEMETRY=1` env → on
5. LSP `initializationOptions.telemetry.enabled = true` → on
6. `config.json` → `telemetry.enabled = true` → on

**Default heuristics tier (applied when none of the above set):**
7. `cfg(debug_assertions)` build → off (developer's local dev/test cycles never ship telemetry)
8. `cfg(test)` → off
9. CI environment detection → off when any of these env vars present: `CI=true`, `GITHUB_ACTIONS`, `GITLAB_CI`, `BUILDKITE`, `CIRCLECI`, `TRAVIS`, `JENKINS_URL`, `TEAMCITY_VERSION`, `TF_BUILD` (Azure Pipelines)
10. Otherwise: on (production release builds, interactive use)

Rationale for off-by-default in dev/CI: avoids polluting telemetry with maintainer's own development noise and CI runs of language tests, which would dwarf real user signal. Real users on release builds remain on by default per consent model in Section 8.

### Connection string (App Insights)

App Insights authenticates with a **connection string** (not raw instrumentation key) of the form:
```
InstrumentationKey=<guid>;IngestionEndpoint=https://<region>.in.applicationinsights.azure.com/;LiveEndpoint=...
```

- Burned into release binaries via `build.rs` reading `AL_CH_TELEMETRY_CONNECTION_STRING` env at build time.
- Users can override in `config.json` (`telemetry.connection_string`) to ship to their own App Insights resource.
- Dev/test builds without the connection string compile telemetry in but log `telemetry: no connection string configured, disabled` and return a no-op handle.
- Connection strings are write-only ingestion credentials, not subscription credentials. Public exposure (e.g., in a forked build) lets a fork ship its own data to your resource — annoying but not a security incident. Mitigation: monitor for anomalous volume; rotate by issuing a new App Insights resource if abuse occurs (old key still works until you decommission the resource).
- **Forks should rebuild without the env var** — they get a no-op telemetry layer, which is the correct default.

### Transparency endpoint

LSP request `al-call-hierarchy/telemetryStatus`:

```json
{
  "enabled": true,
  "install_id": "a1b2c3d4e5f67890",
  "endpoint": "https://...azure.com",
  "events_sent_session": 142,
  "events_dropped_queue_full": 0,
  "events_dropped_export_failed": 0,
  "unique_patterns_session": 17,
  "last_flush_secs_ago": 3,
  "telemetry_disabled_reason": null
}
```

Same data emitted as a single log line on startup for non-VSCode users.

### README disclosure (mandatory)

Top of README, "Telemetry" section above "Installation". Plain English: ON by default, what's collected, link to schema, how to disable (3 ways), link to source of `telemetry/` module.

## 8. Privacy

### Sent
- AL object types (Codeunit, Page, Table — public schema vocabulary)
- Failure categories
- Tree-sitter node paths (grammar shape, public)
- 16-char salted hashes of identifiers
- Length of names (bucketed where possible)
- Counts, durations, version strings
- Per-installation 16-char hex ID (8 bytes derived from local salt)
- Per-workspace 16-char hex ID (scopes dedup; cannot be reversed without local salt)

### Never sent
- AL identifier names in clear
- File paths, workspace names
- Hostnames, usernames, IP addresses
- Source code, comments, string literals
- App.json contents beyond hashed dependency GUIDs
- Environment variable values

### Why hashes are safe
- 32-byte salt per install, generated locally, never transmitted
- Without salt, hashes are preimage-resistant
- Different installs produce different hashes for same name → no cross-user joining at Azure
- User can `cat ~/.al-call-hierarchy/installation-id` and verify it's random bytes

### Consent surface
- README "Telemetry" section above "Installation"
- Startup log line every run while enabled
- LSP `al-call-hierarchy/telemetryStatus` runtime introspection
- Config file with comments explaining each field

### Three off-switches (any wins)
1. Env: `AL_CH_TELEMETRY=0` or `DO_NOT_TRACK=1`
2. CLI: `--no-telemetry`
3. Config file: `telemetry.enabled: false`

Once disabled, stays disabled. No re-prompting.

### Auditability
- All telemetry source in one directory: `src/telemetry/`
- README links to event source: `src/telemetry/events.rs`
- Hash function is a two-line file: `src/telemetry/hash.rs`

## 9. Error Handling

| Failure | Where | Response |
|---|---|---|
| `installation-id` file unreadable | Startup | In-memory salt, WARN once, continue |
| `installation-id` directory unwritable | First run | Same |
| `session.lock` create fails | Startup | WARN once, continue without crash detection (don't block telemetry over a marker file) |
| `session.lock` already exists at startup | Startup | Set `previous_session_unclean = true` on `SessionStart`, then overwrite |
| Config file parse error | Startup | WARN with line, defaults, telemetry stays on |
| `connection_string` empty / malformed | Startup | INFO log, no-op handle |
| OTel SDK init failure | Startup | WARN, no-op handle, LSP starts normally |
| App Insights exporter init failure (bad endpoint) | Startup | WARN, no-op handle |
| mpsc disconnected | Hot path | Atomic flag flips off, ERROR once, no restart |
| App Insights HTTP 4xx | Background | Drop batch, WARN once per session, no retry |
| App Insights HTTP 5xx / network | Background | Exp backoff 1s/2s/4s, max 3, then drop |
| Azure throttling 429 | Background | Honor `Retry-After` ≤30s, then drop |
| Tree-sitter panic in instrumentation walk | Hot path | `catch_unwind` wrapper, telemetry catches own panics |
| Hash on extreme input | Hot path | Truncate to 4KB |
| Clock skew | Dedup TTL | `saturating_duration_since` |
| Shutdown timeout | Server exit | Detached thread, OS reclaims; session.lock survives → next session detects |

### No-spam log policy
Each warn/error category fires at most once per session via `std::sync::Once`. Counts surface in `session.summary`.

### Recovery vs. fail-closed
Telemetry never auto-recovers. Once disabled mid-session, stays off until restart.

### Verification
- `cargo clippy -- -D clippy::unwrap_used -D clippy::expect_used` scoped to `src/telemetry/`
- Outer `catch_unwind` at every `record_*` entry point

## 10. Testing

| Layer | Coverage |
|---|---|
| Unit | Hash determinism, dedup LRU eviction, config resolution order, consent precedence |
| Unit | Event serialization to OTLP attributes (snapshot tests) |
| Unit | Panic boundary — induced panic in `record_*`, telemetry disables, LSP unaffected |
| Integration | Hot-path latency budget — 10k `record_*` calls, ≤5µs avg via `criterion` |
| Integration | Queue overflow — fill 2048+100 events, assert drop counter |
| Integration | OTLP exporter end-to-end via `wiremock` (request shape, retries, batching) |
| Integration | Shutdown drains within 3s, summary emitted |
| Integration | Resolution-failure fixtures from `tests/fixtures/telemetry/` |
| Privacy lint | Scan `events.rs` source — fail if any `String` field not behind a hash function |
| Privacy regression | 10k AL identifier corpus, no collisions in 16-char hex |
| Manual smoke | Real Azure Monitor with `AL_CH_TELEMETRY_KEY` set, documented in `docs/telemetry-smoke-test.md` |

### New fixtures
- `tests/fixtures/telemetry/unresolved_app_dep/`
- `tests/fixtures/telemetry/parser_error/`
- `tests/fixtures/telemetry/missing_dep/`

### Coverage gates
- Telemetry module ≥ 85% line coverage (`cargo llvm-cov`)
- Every `EventKind` variant exercised
- Every error-handling row has a test

### Concurrency
- 16 threads × 1000 events, Miri pass, all events accounted for
- Background thread crash simulation, atomic flag flips, no panic

### CI
- All telemetry tests on every PR
- Privacy lint test = hard gate
- Smoke test = manual / nightly with real key
- Hot-path bench tracked post-merge

## 11. Rollout

### Phase 0 — Foundation (PR #1)
- Add `telemetry` feature flag (default-on)
- Skeleton `src/telemetry/` with stubbed `record_*` bodies
- Hash module with domain-separation + 128-bit truncation
- Session marker module
- Tests: hash determinism, hash domain isolation, install_id, session marker round-trip, consent precedence (incl. CI env detection)
- Mergeable when: builds with and without `--no-default-features`, no LSP behavior change

### Phase 0.5 — App Insights ingestion spike (gate before Phase 1)

**Blocking prerequisite.** Validate end-to-end ingestion before building pipeline.

- Standalone binary: emit one synthetic `ResolutionMiss` event using `opentelemetry-application-insights` exporter against real App Insights resource
- Verify: event arrives, all attributes intact (no truncation, no transformation), Kusto queryable by `customDimensions.telemetry_alch_failure`
- Verify: `session.summary` (large attribute set) survives ingestion
- Verify: connection string parsing handles all official format variants
- Verify: backend sampling does NOT drop summary events (test 100 summary emits in succession; assert all 100 land)
- Document findings in `docs/telemetry-ingestion-spike.md`
- **Outcome decides Phase 1 architecture:** if exporter works → proceed as designed. If not → fall back options documented in spike doc (direct breeze HTTP, OTel collector sidecar, drop OTel)

### Phase 1 — Pipeline (PR #2)
- Implement events, dedup (workspace-scoped), pipeline, exporter, summary
- Add deps: `opentelemetry`, `opentelemetry_sdk`, `opentelemetry-application-insights`, `tokio` (rt feature), `tracing`, `tracing-opentelemetry`, `lru`, `blake3`
- Atomic counter array for OBSERVED_BY_KIND / EXPORTED_BY_KIND / DEDUP_SUPPRESSED / QUEUE_FULL_DROPS / EXPORT_*
- `session.summary` constructed by background thread post-drain, exported unsampled
- `init` / `shutdown` wired into `server.rs` (no-op when disabled)
- Tests: queue overflow distinguished from dedup-suppressed in counters, dedup workspace isolation, exporter mock, shutdown drain emits summary
- Mergeable when: end-to-end via mock, hot-path bench passes 5µs

### Phase 2 — Instrumentation (PR #3)
- Add `record_*` calls at all sites
- Resolution-failure fixtures
- LSP `telemetryStatus` request
- Mergeable when: fixture tests pass, privacy lint passes

### Phase 3 — Disclosure & release (PR #4)
- README "Telemetry" section
- `docs/telemetry.md` schema reference
- CHANGELOG entry under `Added`
- Release `0.7.0` with `AL_CH_TELEMETRY_CONNECTION_STRING` set, smoke test against Azure

### Post-release monitoring (first 2 weeks)
- Watch `queue_full_drops` / `dedup_suppressed` / `export_failures` ratios across `session.summary` events. Healthy: `queue_full_drops` near zero, `dedup_suppressed` ≫ `exported` (deduplication earning its keep), `export_failures` near zero.
- Watch `previous_session_unclean` rate — high rate signals LSP crashes worth investigating.
- App Insights dashboard:
  - Top 20 unresolved `(callee_object_type, callee_source)` pairs (filter on `failure != EmptyResult`)
  - Top 10 `unsupported_construct` ts_node_paths
  - Top `parser.tree_error` files by size bucket
  - `indexer.missing_dependency` rate
  - `EmptyResult` outcomes shown on a separate panel, never rolled into "failure rate"
- App Insights backend-side sampling: confirm disabled for `session.summary` events (set `samplingPercentage: 100` for events where `event_kind == "SessionSummary"`).

### Backout
- User-reported issue → ship `0.7.1` with default `enabled: false`. Code stays in.
- Catastrophic → users already have 3 disable mechanisms; pin GitHub issue, no emergency revert.

### Out of scope
- VSCode extension consent UI
- User-facing share-back dashboard
- Custom telemetry backends beyond OTLP endpoint swap
- Per-event severity / dynamic config

## 12. Open Decisions

None blocking. One **gated** decision:

- **App Insights exporter choice** (Phase 0.5 spike). Default plan: `opentelemetry-application-insights` crate. Fallbacks if spike fails: (a) direct `ureq` blocking POST to breeze endpoint with hand-rolled batching, dropping OTel SDK + Tokio entirely; (b) OTel collector sidecar (rejected as ops-heavy unless required). Spike outcome documented before Phase 1 begins.

## 13. Dependencies Added

```toml
opentelemetry = "0.27"
opentelemetry_sdk = { version = "0.27", features = ["rt-tokio-current-thread"] }
opentelemetry-application-insights = { version = "0.36", features = ["reqwest-client"] }
tokio = { version = "1", features = ["rt", "macros", "time"] }
tracing = "0.1"
tracing-opentelemetry = "0.28"
lru = "0.12"
blake3 = "1"
```

Notes:
- `opentelemetry-application-insights` replaces `opentelemetry-otlp`. Generic OTLP/HTTP does not match Application Insights ingestion protocol; this crate speaks the Azure-specific breeze format and accepts the connection-string format directly.
- Vendor swap (e.g., to OTLP collector for Honeycomb/Grafana) = swap `opentelemetry-application-insights` for `opentelemetry-otlp` and update endpoint config. Single-file change in `exporter.rs`.
- Versions pin to current stable as of 2026-05-06; bump during implementation if newer compatible releases exist.
- Phase 0.5 spike validates the `opentelemetry-application-insights` choice. If the spike fails, the dependency list updates per spike findings (e.g., direct `ureq` + `serde_json` if rolling our own breeze client).
