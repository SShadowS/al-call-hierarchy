//! Anonymous failure-diagnostics telemetry.
//!
//! See `docs/superpowers/specs/2026-05-06-telemetry-design.md` for the full design.
//!
//! When the `telemetry` feature is disabled, all public functions are no-ops
//! that compile to a single early return.

#![allow(dead_code)] // Stubs come online in Phase 0/1.

// Submodules added in Tasks 0.4-0.8 of the plan:
// #[cfg(feature = "telemetry")] mod consent;
// #[cfg(feature = "telemetry")] mod hash;
// #[cfg(feature = "telemetry")] mod install_id;
// #[cfg(feature = "telemetry")] mod session_marker;
// #[cfg(feature = "telemetry")] pub mod events;

/// Opaque handle returned from `init` and passed to `shutdown`.
/// When telemetry is disabled, this is a zero-sized type.
#[cfg(feature = "telemetry")]
pub struct TelemetryHandle {
    _private: (),
}

#[cfg(not(feature = "telemetry"))]
pub struct TelemetryHandle;

/// Initialize the telemetry subsystem. Returns a no-op handle when disabled.
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
pub fn shutdown(_handle: TelemetryHandle) {
    // Phase 1 wires this up; Phase 0 stub is a no-op.
}
