# Telemetry Ingestion Spike — Findings

**Date:** 2026-05-06
**Spec:** `docs/superpowers/specs/2026-05-06-telemetry-design.md`
**Status:** PASS — proceed to Phase 1 with the planned exporter (with corrections noted below).

## Setup

- Azure Application Insights resource: `allsp` in resource group `AL-LSP` (West Europe).
- Connection string format: `InstrumentationKey=...;IngestionEndpoint=https://westeurope-5.in.applicationinsights.azure.com/;LiveEndpoint=...;ApplicationId=...`.
- Spike binary: `tools/telemetry-spike/main.rs`.
- Crate under test: `opentelemetry-application-insights = "0.36"` with `reqwest-blocking-client` feature.
- Backing OTel SDK: `opentelemetry = "0.26"`, `opentelemetry_sdk = "0.26"` (see Finding 5).

## Test 1 — synthetic resolution-miss event

- Event arrived: yes.
- All custom dimensions present: yes (`telemetry.alch.failure`, `callee_object_type`, `callee_source`, `object_hash`, `procedure_hash`, `arg_count`, `schema_version`).
- Latency to Kusto: ~1-2 minutes.
- Verified via `dependencies | where name == "resolution.procedure_not_found"`.

## Test 2 — session.summary with full attribute set

- Event arrived: yes.
- All 14 `telemetry.alch.observed.{0..13}` attributes preserved.
- All 14 `telemetry.alch.exported.{0..13}` attributes preserved.
- All four pipeline counters preserved: `queue_full_drops`, `dedup_suppressed`, `export_attempts`, `export_failures`.
- `duration_secs` preserved.
- Verified via `dependencies | where name == "session.summary" | project customDimensions`.

## Test 3 — backend sampling on summary burst

- Burst sent: 100 events (`session.summary.burst`).
- Burst received: 100.
- Sampling rate inferred: 0% drops.
- Mitigation needed: none for default ingestion. If/when adaptive sampling is enabled at the resource level, summary events should be exempted via a fixed-rate sampling rule keyed on `name == "session.summary"`.

## Findings (corrections to the spec/plan)

### 1. Events land in `dependencies`, not `traces`

OTel `tracer.start("name")` creates internal spans. The `opentelemetry-application-insights` exporter maps internal spans to the App Insights `dependencies` table. The `traces` table is reserved for log records (OTel `LogRecord`, not `Span`).

**Implication for Phase 1:**
- Dashboard queries must use `dependencies`, not `traces`. Update spec §11 dashboards accordingly.
- If `traces` semantics are preferred (pure log records, no parent/child structure), switch the pipeline to use OTel `LogsBridge` instead of `Tracer`. Adds complexity for marginal benefit; not recommended.

### 2. Numeric attributes are stringified at ingest

Values written via `KeyValue::new("k", 42_i64)` arrive in App Insights `customDimensions` as `"42"` (string). Kusto needs `tolong(customDimensions["k"])` to aggregate.

**Implication for Phase 1:** dashboard query examples should explicitly cast numeric attributes. Trivial; no architectural change.

### 3. Plan/spec dependency version was wrong

- Plan §13 / Task 1.1: `opentelemetry = "0.27"`, `opentelemetry_sdk = "0.27"`.
- Reality: `opentelemetry-application-insights = "0.36"` requires `opentelemetry = "0.26"`. Pinning to `0.27` produces a trait-coherence error at `set_tracer_provider` and `tracer_provider.tracer(...)`.
- **Action for Phase 1 Task 1.1:** pin `opentelemetry = "0.26"` and `opentelemetry_sdk = "0.26"` to match the AI exporter's transitive dep. Revisit when an `application-insights` 0.37+ targeting opentelemetry 0.27 is published.

### 4. Plan placed spike deps in `[dev-dependencies]` — invalid

Cargo `[dev-dependencies]` apply to tests, examples, and benches — NOT to `[[bin]]` targets. Putting `opentelemetry`/`reqwest` there gives "unresolved module" errors when building the spike binary.

**Action taken in Task 0.5.1:** moved spike deps into `[dependencies]` as `optional = true`, gated behind a new `telemetry-spike` cargo feature. Default LSP build is unaffected (verified via `cargo tree`).

**Action for Phase 1 Task 1.1:** the runtime promotion can drop the `telemetry-spike` feature once Phase 1 lands; the deps move under the existing `telemetry` feature gate as `optional = true`. Hand-roll the gate in Task 1.1.

### 5. Span trait import required

The plan's spike snippet calls `span.set_attribute(...)` but does not import `opentelemetry::trace::Span`. The method lives on the `Span` trait and must be in scope.

**Action for Phase 1:** include `Span` in the use list everywhere `set_attribute` is called on a span.

### 6. SDK metadata leaks into customDimensions

Every event includes:
- `service.name = "unknown_service"`
- `telemetry.sdk.language = "rust"`
- `telemetry.sdk.name = "opentelemetry"`
- `telemetry.sdk.version = "0.26.0"`

These are added by the OTel SDK automatically and are harmless (no PII), but they inflate the per-event payload by ~120 bytes.

**Action for Phase 1:** call `Resource::builder().with_service_name("al-call-hierarchy").build()` (or set `OTEL_SERVICE_NAME` env var) so `service.name` becomes meaningful. Optionally suppress `telemetry.sdk.*` attributes via a custom resource if payload size matters.

## Decision

**PROCEED** with `opentelemetry-application-insights` for Phase 1.

The spike validated the full pipeline shape: connection-string auth, exporter→breeze ingest, attribute fidelity, no surprise sampling. The five corrections above are deltas to the plan, not architectural pivots.

## Open questions

- Should the LSP also send `session.summary` to the `customEvents` table (App Insights' "user-facing event" table) instead of / in addition to `dependencies`? `customEvents` is more appropriate for "this happened, here are the dimensions" — but requires a separate API path (`tracer.start` won't reach it; needs `client.track_event(...)` from the older `appinsights` crate). Defer to post-Phase-1 unless dashboards need it.
- Sampling configuration on the App Insights resource itself was not exercised. Phase 1 should add a smoke test step that verifies summary events survive whatever resource-level sampling is configured at release time.
