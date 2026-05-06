//! Process-wide singleton holding pipeline + salt + identifiers, so
//! `record_*` functions can be called without threading a handle everywhere.
//!
//! Set during `init`. Read by `record_*` functions on the hot path.

use crate::telemetry::counters::Counters;
use crate::telemetry::hash::Salt;
use crate::telemetry::pipeline::Pipeline;
use std::sync::{Arc, OnceLock, RwLock};

pub(super) struct Runtime {
    pub pipeline: RwLock<Option<Pipeline>>,
    pub counters: Arc<Counters>,
    pub salt: Salt,
    pub workspace_id: String,
    pub install_id: String,
    pub session_id: u64,
    pub previous_session_unclean: bool,
}

static RUNTIME: OnceLock<Runtime> = OnceLock::new();

pub(super) fn install(
    pipeline: Pipeline,
    counters: Arc<Counters>,
    salt: Salt,
    workspace_id: String,
    install_id: String,
    session_id: u64,
    previous_session_unclean: bool,
) {
    let _ = RUNTIME.set(Runtime {
        pipeline: RwLock::new(Some(pipeline)),
        counters,
        salt,
        workspace_id,
        install_id,
        session_id,
        previous_session_unclean,
    });
}

pub(super) fn get() -> Option<&'static Runtime> {
    RUNTIME.get()
}

pub(super) fn close_pipeline() {
    if let Some(rt) = RUNTIME.get() {
        let mut guard = rt.pipeline.write().expect("runtime pipeline lock poisoned");
        if let Some(p) = guard.take() {
            p.close();
        }
    }
}

#[cfg(any(test, feature = "test-runtime"))]
pub mod testing {
    //! Test-only helpers for installing a runtime without spawning the
    //! background exporter. `record_*` calls increment counters but never
    //! block on a network/exporter, and the pipeline remains `None` so the
    //! sender path is a no-op.
    use super::{Runtime, RUNTIME};
    use crate::telemetry::counters::Counters;
    use std::sync::{Arc, RwLock};

    /// Install a counters-only runtime. Idempotent in the sense that it
    /// silently ignores re-installation (OnceLock semantics) — the first
    /// install in the process wins. Tests that need fresh state should
    /// either share the same `Counters` instance via `Arc` or read the
    /// snapshot delta.
    pub fn install_runtime_for_test(counters: Arc<Counters>) {
        let _ = RUNTIME.set(Runtime {
            pipeline: RwLock::new(None),
            counters,
            salt: [0u8; 32],
            workspace_id: "test_workspace".into(),
            install_id: "0000000000000000".into(),
            session_id: 0,
            previous_session_unclean: false,
        });
    }

    /// Read the currently-installed runtime's counters, if any. Useful for
    /// assertions when the same runtime is shared across tests.
    pub fn current_counters() -> Option<Arc<Counters>> {
        RUNTIME.get().map(|rt| rt.counters.clone())
    }
}
