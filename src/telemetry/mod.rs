//! Anonymous failure-diagnostics telemetry.
//!
//! See `docs/superpowers/specs/2026-05-06-telemetry-design.md` for the full design.
//!
//! When the `telemetry` feature is disabled, all public functions are no-ops
//! that compile to a single early return.

// Submodules added in Tasks 0.4-0.8 of the plan:
#[cfg(feature = "telemetry")]
mod consent;
#[cfg(feature = "telemetry")]
pub mod counters;
#[cfg(feature = "telemetry")]
mod dedup;
#[cfg(feature = "telemetry")]
pub mod events;
#[cfg(feature = "telemetry")]
mod events_attrs;
#[cfg(feature = "telemetry")]
pub mod exporter;
#[cfg(feature = "telemetry")]
mod hash;
#[cfg(feature = "telemetry")]
mod install_id;
#[cfg(feature = "telemetry")]
pub mod pipeline;
#[cfg(feature = "telemetry")]
mod session_marker;

/// Opaque handle returned from `init` and passed to `shutdown`.
/// When telemetry is disabled, this is a zero-sized type.
#[cfg(feature = "telemetry")]
#[allow(dead_code)] // Wired up in Phase 1 (server.rs init/shutdown).
pub struct TelemetryHandle {
    _private: (),
}

#[cfg(not(feature = "telemetry"))]
#[allow(dead_code)] // Wired up in Phase 1 (server.rs init/shutdown).
pub struct TelemetryHandle;

/// Initialize the telemetry subsystem. Returns a no-op handle when disabled.
#[allow(dead_code)] // Wired up in Phase 1 (server.rs init/shutdown).
pub fn init() -> TelemetryHandle {
    #[cfg(feature = "telemetry")]
    {
        TelemetryHandle { _private: () }
    }
    #[cfg(not(feature = "telemetry"))]
    {
        TelemetryHandle
    }
}

/// Shut down telemetry. Drains the queue and emits the session summary.
#[allow(dead_code)] // Wired up in Phase 1 (server.rs init/shutdown).
pub fn shutdown(_handle: TelemetryHandle) {
    // Phase 1 wires this up; Phase 0 stub is a no-op.
}
