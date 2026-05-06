//! Snapshot of telemetry runtime state for the transparency LSP request.

use serde::Serialize;

#[cfg(feature = "telemetry")]
use crate::telemetry::runtime;

#[derive(Debug, Serialize)]
pub struct TelemetryStatus {
    pub enabled: bool,
    pub install_id: String,
    pub workspace_id: String,
    pub events_sent_session: u32,
    pub events_dropped_queue_full: u32,
    pub events_dropped_dedup: u32,
    pub export_failures: u32,
    pub schema_version: u8,
}

#[cfg(feature = "telemetry")]
pub fn snapshot() -> TelemetryStatus {
    let Some(rt) = runtime::get() else {
        return TelemetryStatus {
            enabled: false,
            install_id: String::new(),
            workspace_id: String::new(),
            events_sent_session: 0,
            events_dropped_queue_full: 0,
            events_dropped_dedup: 0,
            export_failures: 0,
            schema_version: crate::telemetry::events::SCHEMA_VERSION,
        };
    };
    let snap = rt.counters.snapshot();
    TelemetryStatus {
        enabled: true,
        install_id: rt.install_id.clone(),
        workspace_id: rt.workspace_id.clone(),
        events_sent_session: snap.exported_by_kind.iter().sum(),
        events_dropped_queue_full: snap.queue_full_drops,
        events_dropped_dedup: snap.dedup_suppressed,
        export_failures: snap.export_failures,
        schema_version: crate::telemetry::events::SCHEMA_VERSION,
    }
}

#[cfg(not(feature = "telemetry"))]
pub fn snapshot() -> TelemetryStatus {
    TelemetryStatus {
        enabled: false,
        install_id: String::new(),
        workspace_id: String::new(),
        events_sent_session: 0,
        events_dropped_queue_full: 0,
        events_dropped_dedup: 0,
        export_failures: 0,
        schema_version: 1,
    }
}
