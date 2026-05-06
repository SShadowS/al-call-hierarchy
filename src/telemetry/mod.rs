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
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while !join.is_finished() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        if join.is_finished() {
            let _ = join.join();
        } else {
            log::warn!(
                "telemetry: background thread did not finish within 3s shutdown budget; detaching"
            );
            // Drop the handle without joining — the OS reclaims the thread on process exit.
        }
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

#[cfg(feature = "telemetry")]
pub use events::{
    CallPattern, CalleeSource, CallerContext, ConfigFlags, DefinitionKind, IndexerIssueKind,
    ObjectType, ParserErrorKind, ResolutionFailure,
};

#[cfg(feature = "telemetry")]
use events::{
    EventEnvelope, EventKind, HandlerEmpty, IndexerIssue, ParserError, ResolutionMiss,
    SessionStart, SizeBucket,
};

#[cfg(feature = "telemetry")]
pub struct CallContext<'a> {
    pub failure: ResolutionFailure,
    pub call_pattern: CallPattern,
    pub callee_object_type: Option<ObjectType>,
    pub callee_source: CalleeSource,
    pub caller_object_type: ObjectType,
    pub caller_context: CallerContext,
    pub callee_object_name: Option<&'a str>,
    pub callee_procedure_name: &'a str,
    pub arg_count: u8,
    pub ts_node_path: &'a str,
}

#[cfg(feature = "telemetry")]
pub fn record_resolution_miss(ctx: &CallContext<'_>) {
    let Some(rt) = runtime::get() else { return };
    let leaf = match ctx.failure {
        ResolutionFailure::ObjectNotFound => events::LeafKind::ResolutionObjectNotFound,
        ResolutionFailure::ProcedureNotFound => events::LeafKind::ResolutionProcedureNotFound,
        ResolutionFailure::UnresolvedUnqualified => {
            events::LeafKind::ResolutionUnresolvedUnqualified
        }
        ResolutionFailure::Ambiguous => events::LeafKind::ResolutionAmbiguous,
        ResolutionFailure::UnsupportedConstruct => events::LeafKind::ResolutionUnsupportedConstruct,
    };
    rt.counters.observe(leaf);

    let object_hash = ctx
        .callee_object_name
        .map(|n| hash::hash_identifier(&rt.salt, hash::DOMAIN_OBJECT, n));
    let procedure_hash =
        hash::hash_identifier(&rt.salt, hash::DOMAIN_PROCEDURE, ctx.callee_procedure_name);

    let env = EventEnvelope {
        schema_version: events::SCHEMA_VERSION,
        timestamp: std::time::SystemTime::now(),
        install_id: rt.install_id.clone(),
        al_version: env!("CARGO_PKG_VERSION"),
        grammar_version: "v2",
        os: events::current_os(),
        session_id: rt.session_id,
        workspace_id: rt.workspace_id.clone(),
        event: EventKind::ResolutionMiss(ResolutionMiss {
            failure: ctx.failure,
            call_pattern: ctx.call_pattern,
            callee_object_type: ctx.callee_object_type,
            callee_source: ctx.callee_source,
            caller_object_type: ctx.caller_object_type,
            caller_context: ctx.caller_context,
            object_hash,
            procedure_hash,
            arg_count: ctx.arg_count,
            name_len_object: ctx.callee_object_name.map(|n| n.len() as u16),
            name_len_procedure: ctx.callee_procedure_name.len() as u16,
            ts_node_path: ctx.ts_node_path.into(),
            repeat_count: 0,
        }),
    };

    if let Ok(guard) = rt.pipeline.read() {
        if let Some(p) = guard.as_ref() {
            p.clone_sender().send(env);
        }
    }
}

#[cfg(feature = "telemetry")]
pub fn record_parser_error(kind: ParserErrorKind, file: &std::path::Path) {
    let Some(rt) = runtime::get() else { return };
    let leaf = match kind {
        ParserErrorKind::TreeError => events::LeafKind::ParserTreeError,
        ParserErrorKind::ParseFailed => events::LeafKind::ParserParseFailed,
        ParserErrorKind::UnknownNodeKind => events::LeafKind::ParserUnknownNodeKind,
    };
    rt.counters.observe(leaf);

    let path_str = file.to_string_lossy();
    let file_hash = hash::hash_short(&rt.salt, hash::DOMAIN_FILE, path_str.as_bytes());
    let file_extension = file
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let size = std::fs::metadata(file).map(|m| m.len()).unwrap_or(0);
    let file_size_bucket = size_bucket_for_bytes(size);

    let env = EventEnvelope {
        schema_version: events::SCHEMA_VERSION,
        timestamp: std::time::SystemTime::now(),
        install_id: rt.install_id.clone(),
        al_version: env!("CARGO_PKG_VERSION"),
        grammar_version: "v2",
        os: events::current_os(),
        session_id: rt.session_id,
        workspace_id: rt.workspace_id.clone(),
        event: EventKind::ParserError(ParserError {
            kind,
            node_kind_hash: None,
            file_hash,
            file_extension,
            file_size_bucket,
            error_count: 0,
            repeat_count: 0,
        }),
    };
    if let Ok(guard) = rt.pipeline.read() {
        if let Some(p) = guard.as_ref() {
            p.clone_sender().send(env);
        }
    }
}

#[cfg(feature = "telemetry")]
fn size_bucket_for_bytes(size: u64) -> SizeBucket {
    match size {
        0..=1024 => SizeBucket::Sub1k,
        1025..=10_240 => SizeBucket::Sub10k,
        10_241..=102_400 => SizeBucket::Sub100k,
        _ => SizeBucket::Over100k,
    }
}

#[cfg(feature = "telemetry")]
pub fn record_indexer_issue(kind: IndexerIssueKind, detail_code: u16, app_id: Option<&str>) {
    let Some(rt) = runtime::get() else { return };
    let leaf = match kind {
        IndexerIssueKind::MissingDependency => events::LeafKind::IndexerMissingDependency,
        IndexerIssueKind::AppParseFailed => events::LeafKind::IndexerAppParseFailed,
        IndexerIssueKind::BrokenSymlink => events::LeafKind::IndexerBrokenSymlink,
        IndexerIssueKind::IoError => events::LeafKind::IndexerIoError,
    };
    rt.counters.observe(leaf);

    let app_id_hash = app_id.map(|a| hash::hash_identifier(&rt.salt, hash::DOMAIN_APP_ID, a));
    let env = EventEnvelope {
        schema_version: events::SCHEMA_VERSION,
        timestamp: std::time::SystemTime::now(),
        install_id: rt.install_id.clone(),
        al_version: env!("CARGO_PKG_VERSION"),
        grammar_version: "v2",
        os: events::current_os(),
        session_id: rt.session_id,
        workspace_id: rt.workspace_id.clone(),
        event: EventKind::IndexerIssue(IndexerIssue {
            kind,
            app_id_hash,
            detail_code,
        }),
    };
    if let Ok(guard) = rt.pipeline.read() {
        if let Some(p) = guard.as_ref() {
            p.clone_sender().send(env);
        }
    }
}

#[cfg(feature = "telemetry")]
pub fn record_handler_empty(
    method: &'static str,
    target_object_type: ObjectType,
    target_kind: DefinitionKind,
    object_name: &str,
    procedure_name: &str,
) {
    use std::sync::atomic::{AtomicU32, Ordering};
    static SAMPLE_COUNTER: AtomicU32 = AtomicU32::new(0);
    if SAMPLE_COUNTER.fetch_add(1, Ordering::Relaxed) % 10 != 0 {
        return;
    }
    let Some(rt) = runtime::get() else { return };
    rt.counters.observe(events::LeafKind::HandlerEmpty);

    let object_hash = hash::hash_identifier(&rt.salt, hash::DOMAIN_OBJECT, object_name);
    let procedure_hash = hash::hash_identifier(&rt.salt, hash::DOMAIN_PROCEDURE, procedure_name);
    let env = EventEnvelope {
        schema_version: events::SCHEMA_VERSION,
        timestamp: std::time::SystemTime::now(),
        install_id: rt.install_id.clone(),
        al_version: env!("CARGO_PKG_VERSION"),
        grammar_version: "v2",
        os: events::current_os(),
        session_id: rt.session_id,
        workspace_id: rt.workspace_id.clone(),
        event: EventKind::HandlerEmpty(HandlerEmpty {
            method,
            target_object_type,
            target_kind,
            object_hash,
            procedure_hash,
            repeat_count: 0,
        }),
    };
    if let Ok(guard) = rt.pipeline.read() {
        if let Some(p) = guard.as_ref() {
            p.clone_sender().send(env);
        }
    }
}

#[cfg(feature = "telemetry")]
pub fn record_session_start(
    workspace_file_count: u32,
    dependency_count: u8,
    has_app_dependencies: bool,
) {
    let Some(rt) = runtime::get() else { return };
    rt.counters.observe(events::LeafKind::SessionStart);

    let al_file_count_bucket = match workspace_file_count {
        0..=99 => SizeBucket::Sub1k,
        100..=499 => SizeBucket::Sub10k,
        500..=1999 => SizeBucket::Sub100k,
        _ => SizeBucket::Over100k,
    };
    let env = EventEnvelope {
        schema_version: events::SCHEMA_VERSION,
        timestamp: std::time::SystemTime::now(),
        install_id: rt.install_id.clone(),
        al_version: env!("CARGO_PKG_VERSION"),
        grammar_version: "v2",
        os: events::current_os(),
        session_id: rt.session_id,
        workspace_id: rt.workspace_id.clone(),
        event: EventKind::SessionStart(SessionStart {
            workspace_file_count,
            al_file_count_bucket,
            dependency_count,
            has_app_dependencies,
            config_flags: ConfigFlags { bits: 0 },
            previous_session_unclean: rt.previous_session_unclean,
        }),
    };
    if let Ok(guard) = rt.pipeline.read() {
        if let Some(p) = guard.as_ref() {
            p.clone_sender().send(env);
        }
    }
}

// No-op stubs for the disabled-feature build. Note these use generic
// signatures so callers don't need cfg blocks to call them.
#[cfg(not(feature = "telemetry"))]
pub fn record_resolution_miss<T>(_ctx: T) {}
#[cfg(not(feature = "telemetry"))]
pub fn record_parser_error<K, P: AsRef<std::path::Path>>(_kind: K, _file: P) {}
#[cfg(not(feature = "telemetry"))]
pub fn record_indexer_issue<K>(_kind: K, _detail_code: u16, _app_id: Option<&str>) {}
#[cfg(not(feature = "telemetry"))]
pub fn record_handler_empty<O, K>(
    _method: &'static str,
    _target: O,
    _kind: K,
    _object: &str,
    _procedure: &str,
) {
}
#[cfg(not(feature = "telemetry"))]
pub fn record_session_start(_a: u32, _b: u8, _c: bool) {}

// status submodule must be available regardless of feature so handlers.rs can
// always call status() without #[cfg] noise. Internal feature-gating happens
// inside status.rs.
pub mod status;

pub fn status() -> status::TelemetryStatus {
    status::snapshot()
}

#[cfg(all(feature = "telemetry", any(test, feature = "test-runtime")))]
pub mod testing {
    pub use super::runtime::testing::{current_counters, install_runtime_for_test};
}
