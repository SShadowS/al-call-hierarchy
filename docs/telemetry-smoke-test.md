# Telemetry Smoke Test (manual, pre-release)

Run before publishing each release.

## Prerequisites

- App Insights resource with connection string
- An AL workspace fixture: `tests/fixtures/telemetry/unresolved_app_dep/`
- Azure CLI logged in (`az account show` succeeds)

## Steps

1. Build a release binary with the connection string baked in:
   ```bash
   AL_CH_TELEMETRY_CONNECTION_STRING="InstrumentationKey=...;IngestionEndpoint=..." \
     cargo build --release
   ```

2. Run the binary against the fixture in CLI mode:
   ```bash
   target/release/al-call-hierarchy --project tests/fixtures/telemetry/unresolved_app_dep
   ```

3. Wait 60-120 seconds, then query App Insights via az CLI (replace `<APP_ID>` with your resource's Application ID):
   ```bash
   az monitor app-insights query --app <APP_ID> --analytics-query \
     "dependencies | where timestamp > ago(5m) | where customDimensions['telemetry.alch.schema_version'] == '1' | summarize count() by name"
   ```
   Or in the Azure Portal Log query UI:
   ```kusto
   dependencies
   | where timestamp > ago(5m)
   | where customDimensions["telemetry.alch.schema_version"] == "1"
   | summarize count() by name
   ```

   Expected: at least one `resolution.miss` and one `session.start` row.

4. Verify hashes look like 32-char hex (resolution events) or 16-char hex (install/workspace IDs):
   ```kusto
   dependencies
   | where name == "resolution.miss"
   | extend obj = tostring(customDimensions["telemetry.alch.object_hash"])
   | project obj, strlen(obj)
   ```

5. Verify no field contains an obvious AL identifier:
   ```kusto
   dependencies
   | where timestamp > ago(5m)
   | where customDimensions has "PostInvoice" or customDimensions has "Customer"
   | count
   ```
   Expected: 0 rows.

6. Verify `session.summary` event is unsampled (run the spike binary which emits 100 burst spans):
   ```bash
   AL_CH_SPIKE_CONNECTION_STRING="..." cargo run --bin telemetry-spike --features telemetry --release
   ```
   ```kusto
   dependencies
   | where timestamp > ago(5m)
   | where name == "session.summary.burst"
   | count
   ```
   Expected: 100. If less, ingestion sampling is active — configure resource-level sampling rules to exempt `name == "session.summary"` events.

## On failure

Do NOT release. File an issue, add the failing dimension to the privacy lint test, fix the leak, restart.
