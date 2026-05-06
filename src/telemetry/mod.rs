//! Anonymous failure-diagnostics telemetry.
//!
//! See `docs/superpowers/specs/2026-05-06-telemetry-design.md` for the full design.
//!
//! When the `telemetry` feature is disabled, all public functions are no-ops
//! that compile to a single early return.

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
mod runtime;
#[cfg(feature = "telemetry")]
mod session_marker;

#[cfg(feature = "telemetry")]
use std::sync::Arc;
#[cfg(feature = "telemetry")]
use std::time::{Duration, Instant};

#[cfg(feature = "telemetry")]
pub struct TelemetryInputs {
    pub cli_no_telemetry: bool,
    pub init_option: Option<bool>,
    pub workspace_root: Option<std::path::PathBuf>,
    pub connection_string: Option<String>,
}

#[cfg(feature = "telemetry")]
pub struct TelemetryHandle {
    enabled: bool,
    join: Option<std::thread::JoinHandle<()>>,
}

#[cfg(feature = "telemetry")]
pub fn init(inputs: TelemetryInputs) -> TelemetryHandle {
    let env = consent::live_env();
    let config = inputs
        .workspace_root
        .as_ref()
        .map(|root| crate::config::TelemetryFileConfig::load_merged(root))
        .unwrap_or_default();
    let consent_inputs = consent::Inputs {
        cli_no_telemetry: inputs.cli_no_telemetry,
        init_option: inputs.init_option,
        config: config.enabled,
        env,
        is_debug: cfg!(debug_assertions),
        is_test: cfg!(test),
    };
    let decision = consent::decide(&consent_inputs);
    let connection_string = inputs.connection_string.or(config.connection_string);

    let enabled = matches!(decision, consent::Decision::Enabled) && connection_string.is_some();
    if !enabled {
        match &decision {
            consent::Decision::Disabled(reason) => {
                log::info!("telemetry: disabled ({:?})", reason);
            }
            consent::Decision::Enabled if connection_string.is_none() => {
                log::info!("telemetry: disabled (no connection string configured)");
            }
            _ => {}
        }
        return disabled_handle();
    }

    let (salt, _persisted) = install_id::load_or_create();
    let install_id = hash::install_id_from_salt(&salt);
    let workspace_id = inputs
        .workspace_root
        .as_ref()
        .map(|p| {
            hash::hash_short(
                &salt,
                hash::DOMAIN_WORKSPACE,
                p.to_string_lossy().as_bytes(),
            )
        })
        .unwrap_or_else(|| "0000000000000000".into());
    let marker = session_marker::record_session_start();
    let previous_session_unclean = marker.previous_session_unclean;
    let counters = Arc::new(counters::Counters::new());
    let started_at = Instant::now();
    let session_id: u64 = {
        let mut h = blake3::Hasher::new();
        h.update(&started_at.elapsed().as_nanos().to_le_bytes());
        h.update(&std::process::id().to_le_bytes());
        let d = h.finalize();
        u64::from_le_bytes(d.as_bytes()[..8].try_into().unwrap())
    };

    let (pipeline, rx) = pipeline::Pipeline::new(
        config.queue_capacity.unwrap_or(2048) as usize,
        counters.clone(),
    );
    let exporter_config = exporter::ExporterConfig {
        connection_string: connection_string.unwrap(),
        flush_interval: Duration::from_secs(config.flush_interval_secs.unwrap_or(5)),
        batch_size: config.batch_size.unwrap_or(512),
    };
    let join = exporter::spawn(exporter_config, rx, counters.clone(), started_at);
    runtime::install(
        pipeline,
        counters.clone(),
        salt,
        workspace_id.clone(),
        install_id.clone(),
        session_id,
        previous_session_unclean,
    );

    log::info!(
        "telemetry: enabled (anonymous, hashed). install_id={}. Disable: AL_CH_TELEMETRY=0 or telemetry.enabled=false in ~/.al-call-hierarchy/config.json",
        install_id
    );

    TelemetryHandle {
        enabled: true,
        join: Some(join),
    }
}

#[cfg(feature = "telemetry")]
fn disabled_handle() -> TelemetryHandle {
    TelemetryHandle {
        enabled: false,
        join: None,
    }
}

#[cfg(feature = "telemetry")]
pub fn shutdown(handle: TelemetryHandle) {
    if !handle.enabled {
        return;
    }
    runtime::close_pipeline();
    if let Some(join) = handle.join {
        let _ = join.join();
    }
    session_marker::record_clean_shutdown();
}

#[cfg(not(feature = "telemetry"))]
pub struct TelemetryInputs {
    pub cli_no_telemetry: bool,
    pub init_option: Option<bool>,
    pub workspace_root: Option<std::path::PathBuf>,
    pub connection_string: Option<String>,
}

#[cfg(not(feature = "telemetry"))]
pub struct TelemetryHandle;

#[cfg(not(feature = "telemetry"))]
pub fn init(_inputs: TelemetryInputs) -> TelemetryHandle {
    TelemetryHandle
}

#[cfg(not(feature = "telemetry"))]
pub fn shutdown(_h: TelemetryHandle) {}
