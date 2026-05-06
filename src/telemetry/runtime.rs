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
