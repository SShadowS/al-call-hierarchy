//! LSP server implementation for call hierarchy — serves the program-engine
//! backend (T3 Task 15 cutover).
//!
//! Server state is now `Arc<SharedSnapshot>` (the published, immutable
//! `LspSnapshot`) + an `mpsc::Sender<ChangeEvent>` feeding a background
//! [`spawn_updater`] thread, instead of the legacy `Arc<RwLock<Indexer>>`.
//! Every request handler only ever `Arc`-clones the current snapshot
//! (`SharedSnapshot::get`) — no parsing, no graph rebuild, ever happens under
//! a request-facing lock; the updater thread owns all of that work and
//! publishes a fresh snapshot by atomic swap (see `src/lsp/updater.rs`'s
//! module doc).
//!
//! Dispatch is direct: `textDocument/prepareCallHierarchy` /
//! `callHierarchy/{incoming,outgoing}Calls` / `textDocument/codeLens` /
//! the three engine-backed custom requests go straight to the Task 11-13
//! functions (`lsp::handlers`/`lsp::lens`/`lsp::custom`) with the negotiated
//! [`PositionEncoding`]. `al-call-hierarchy/{fieldProperties,actionProperties,
//! telemetryStatus}` are graph-independent (pure source-read / process
//! status) and stay on their existing `crate::handlers` implementations —
//! they survive Task 17's legacy deletion untouched.
//!
//! Diagnostics follow "recompute-diff-publish-clear": every snapshot swap
//! (including the very first, batch-built one) runs `lsp::diagnostics::
//! compute_all` and diffs it through one shared [`DiagnosticsState`],
//! publishing only what changed and clearing a uri whose findings dropped to
//! zero — the legacy publish-once-at-startup path never cleared a fixed
//! finding until an unrelated one happened to overwrite it; this closes that
//! gap permanently.
//!
//! # No valid workspace: fail-closed-to-empty, not a crash
//!
//! The program engine's snapshot model fundamentally requires a single
//! AL-app workspace root with a readable `app.json` (see `SnapshotBuilder`'s
//! own doc) — unlike the legacy `Indexer`, which happily ran with zero
//! indexed files. When no workspace folder is given at `initialize`, or the
//! given root fails to build a snapshot (missing/invalid `app.json`), the
//! server still completes the LSP handshake and answers every request with
//! an empty result (`None`/`[]`) rather than refusing to start — see
//! [`ServerState`]'s `Option` wrapping in [`run_server`].

use anyhow::{Context, Result};
use log::{debug, info, warn};
use lsp_server::{Connection, Message, Notification, Request, Response};
use lsp_types::{
    CallHierarchyIncomingCallsParams, CallHierarchyItem, CallHierarchyOutgoingCallsParams,
    CallHierarchyPrepareParams, CodeLensOptions, CodeLensParams, DidSaveTextDocumentParams,
    InitializeParams, InitializeResult, PositionEncodingKind, PublishDiagnosticsParams,
    ServerCapabilities, Uri,
};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::config::DiagnosticConfig;
use crate::handlers::{SymbolPropertiesParams, action_properties, field_properties};
use crate::lsp::custom::{
    DependencyDocumentSymbolParams, EventPublishersInFileParams, EventReferenceAtPositionParams,
    dependency_document_symbol, event_publishers_in_file, event_reference_at_position,
};
use crate::lsp::diagnostics::{DiagnosticsState, compute_all};
use crate::lsp::encoding::{PositionEncoding, negotiate};
use crate::lsp::handlers::{ItemData, incoming, outgoing, prepare};
use crate::lsp::lens::code_lenses;
use crate::lsp::snapshot::LspSnapshot;
use crate::lsp::updater::{ChangeEvent, SharedSnapshot, spawn_updater};
use crate::protocol::uri_to_path;
use crate::watcher::{AlFileWatcher, FileChange};

/// Everything the server needs once a valid workspace snapshot exists.
/// `None` at the `run_server` call site means "no usable workspace" — see
/// the module doc.
struct ServerState {
    shared: Arc<SharedSnapshot>,
    tx: mpsc::Sender<ChangeEvent>,
    /// Deliberately unread in production (see `run_server`'s shutdown-sequence
    /// doc — joining it would hang whenever the file watcher is also running).
    /// Kept so the in-module smoke test (`tests::
    /// dispatch_prepare_and_outgoing_then_didsave_bumps_generation_and_republishes_diagnostics`)
    /// can assert the updater thread actually stops once `tx` is its only
    /// live sender — the watcher-free case that test exercises.
    #[allow(dead_code)]
    updater_handle: JoinHandle<()>,
    encoding: PositionEncoding,
    config: DiagnosticConfig,
}

/// Run the LSP server
pub fn run_server(no_watcher: bool, no_telemetry: bool) -> Result<()> {
    info!("Starting AL Call Hierarchy LSP server (program-engine backend)");

    let (connection, io_threads) = Connection::stdio();

    // Initialize
    let (id, params) = connection.initialize_start()?;
    let init_params: InitializeParams = serde_json::from_value(params)?;

    let workspace_root = primary_workspace_root(&init_params);

    let init_option_telemetry = init_params
        .initialization_options
        .as_ref()
        .and_then(|v| v.get("telemetry"))
        .and_then(|t| t.get("enabled"))
        .and_then(|b| b.as_bool());
    let telemetry_handle = crate::telemetry::init(crate::telemetry::TelemetryInputs {
        cli_no_telemetry: no_telemetry,
        init_option: init_option_telemetry,
        workspace_root: workspace_root.clone(),
        connection_string: option_env!("AL_CH_TELEMETRY_CONNECTION_STRING").map(String::from),
    });

    // H-12: negotiate the position encoding from `general.positionEncodings`
    // (LSP 3.17). Every handler dispatched below is driven with this SAME
    // negotiated value (Task 15 cutover — legacy handlers served byte
    // columns regardless of negotiation; the new backend never does).
    let client_position_encodings: Option<Vec<String>> = init_params
        .capabilities
        .general
        .as_ref()
        .and_then(|g| g.position_encodings.as_ref())
        .map(|encs| encs.iter().map(|e| e.as_str().to_string()).collect());
    let position_encoding = negotiate(client_position_encodings.as_deref());
    info!("Negotiated position encoding: {:?}", position_encoding);

    let capabilities = ServerCapabilities {
        call_hierarchy_provider: Some(lsp_types::CallHierarchyServerCapability::Simple(true)),
        code_lens_provider: Some(CodeLensOptions {
            resolve_provider: Some(false),
        }),
        text_document_sync: Some(lsp_types::TextDocumentSyncCapability::Options(
            lsp_types::TextDocumentSyncOptions {
                open_close: Some(true),
                change: Some(lsp_types::TextDocumentSyncKind::NONE),
                will_save: None,
                will_save_wait_until: None,
                save: Some(lsp_types::TextDocumentSyncSaveOptions::SaveOptions(
                    lsp_types::SaveOptions {
                        include_text: Some(false),
                    },
                )),
            },
        )),
        position_encoding: Some(match position_encoding {
            PositionEncoding::Utf8 => PositionEncodingKind::UTF8,
            PositionEncoding::Utf16 => PositionEncodingKind::UTF16,
        }),
        ..Default::default()
    };

    let result = InitializeResult {
        capabilities,
        server_info: Some(lsp_types::ServerInfo {
            name: "al-call-hierarchy".to_string(),
            version: Some(env!("CARGO_PKG_VERSION").to_string()),
        }),
    };

    connection.initialize_finish(id, serde_json::to_value(result)?)?;
    info!("Server initialized");

    let config = workspace_root
        .as_deref()
        .map(DiagnosticConfig::load)
        .unwrap_or_default();

    let state = match &workspace_root {
        Some(root) => {
            let st = build_server_state(root, position_encoding, config.clone(), &connection);
            if st.is_none() {
                warn!(
                    "Failed to build the program snapshot for workspace root {} \
                     (missing/invalid app.json?) — every request will return an \
                     empty result until a valid single-app AL workspace is opened.",
                    root.display()
                );
            }
            st
        }
        None => {
            warn!(
                "No workspace folder given at initialize — every request will \
                 return an empty result until a workspace is opened."
            );
            None
        }
    };

    #[cfg(feature = "telemetry")]
    {
        let (workspace_file_count, dep_app_count) = state
            .as_ref()
            .map(|st| {
                let snap = st.shared.get();
                let definitions: usize = snap.decls_by_file.values().map(|v| v.len()).sum();
                let call_sites: usize = snap.edges_by_file.values().map(|v| v.len()).sum();
                let deps = snap.snap.apps.len().saturating_sub(1);
                ((definitions + call_sites) as u32, deps)
            })
            .unwrap_or((0, 0));
        let has_app_dependencies = dep_app_count > 0;
        let dependency_count = dep_app_count.min(u8::MAX as usize) as u8;
        crate::telemetry::record_session_start(
            workspace_file_count,
            dependency_count,
            has_app_dependencies,
        );
    }

    // Start file watcher thread for incremental updates (unless disabled).
    // `watcher_started` feeds the shutdown-sequence log decision below.
    let watcher_started = if no_watcher {
        info!("File watcher disabled (--no-watcher). Using LSP notifications for changes.");
        false
    } else {
        match (&workspace_root, &state) {
            (Some(root), Some(st)) => {
                start_file_watcher(st.tx.clone(), root.clone());
                true
            }
            _ => {
                info!("File watcher not started (no active workspace snapshot).");
                false
            }
        }
    };

    // Main loop. Takes OWNERSHIP of `connection` (not `&connection`) —
    // verified end to end (a real stdio round trip via a Python LSP client,
    // T3 Task 15's own manual smoke test) that a BORROWED connection hangs
    // the process forever after a clean shutdown/exit: `IoThreads::join`
    // waits for `lsp_server`'s writer thread, which only exits once every
    // `Sender<Message>` clone is dropped — including `connection.sender`
    // ITSELF, which never happens while `connection` is still a live local
    // in `run_server`'s own scope below `io_threads.join()?`. Moving
    // `connection` into `main_loop` means it (and its `sender`) drops at
    // `main_loop`'s own closing brace, BEFORE `io_threads.join()?` runs —
    // matching `lsp_server`'s own `examples/minimal_lsp.rs` pattern exactly
    // (`main_loop(connection, init_params)?; io_thread.join()?;`, connection
    // passed by value). This bug predates this cutover (the legacy
    // `main_loop(connection: &Connection, ...)` had the identical shape) but
    // was never exercised end to end until this task's own verification.
    main_loop(connection, state.as_ref())?;

    if let Some(st) = state {
        // Dropping `tx` is a best-effort shutdown signal to the updater
        // thread — but we deliberately do NOT `st.updater_handle.join()`
        // here: when the file watcher is running, it holds its OWN clone of
        // `tx` (`start_file_watcher`) for as long as ITS OWN infinite loop
        // runs — which has no stop signal of its own (matching legacy's
        // watcher thread, which also never exits early; out of Task 15's
        // "event forwarding only" scope for `watcher.rs` to add one). The
        // updater's `rx` therefore never sees every sender dropped, so it
        // never returns from `gather_batch`, so joining it would block this
        // function forever. Dropping `tx` here is still worthwhile — in the
        // `--no-watcher` case (`watcher_started == false`) it's the ONLY
        // signal the updater thread gets, and lets it (and the
        // `sender_bg`-held `Message` sender clone it carries — see below)
        // exit promptly.
        drop(st.tx);
    }

    // `io_threads.join()` blocks on `lsp_server`'s writer thread, which
    // itself blocks until EVERY `Sender<Message>` clone is dropped —
    // including one captured by the updater thread's diagnostics on_swap
    // closure (`sender_bg` in `build_server_state`), which (per the note
    // above) legitimately never drops while the watcher thread is also
    // alive. Verified end to end (a real stdio round trip, T3 Task 15's own
    // manual smoke test): an unconditional `io_threads.join()?` here hangs
    // the process forever whenever the watcher started. Bounded wait +
    // detach — the SAME idiom `telemetry::shutdown` already uses just below,
    // for the identical reason (a background thread that legitimately never
    // stops on its own must never be joined unconditionally at shutdown):
    // everything meaningful was already flushed by the time `main_loop`
    // returned (the `shutdown` response itself, most importantly), so
    // giving up after a short grace period loses nothing.
    let (io_done_tx, io_done_rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = io_done_tx.send(io_threads.join());
    });
    match io_done_rx.recv_timeout(Duration::from_millis(500)) {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return Err(e.into()),
        Err(_) if watcher_started => debug!(
            "io_threads did not finish within the shutdown budget — expected \
             while the file watcher/updater threads are still alive; exiting anyway"
        ),
        Err(_) => warn!(
            "io_threads did not finish within the shutdown budget (unexpected — no \
             watcher was started this session); exiting anyway"
        ),
    }

    info!("Server shut down");
    crate::telemetry::shutdown(telemetry_handle);
    Ok(())
}

/// The one workspace root this session serves. The program engine's
/// `SnapshotBuilder` models a workspace as exactly one AL app (see its own
/// doc) — a real structural departure from legacy's `Indexer`, which could
/// silently accumulate several `workspace_folders` into one graph. A client
/// offering more than one folder gets the FIRST, with a clear warning
/// (fail-loud, never silently drop work without saying so) rather than an
/// attempt to merge multiple app.json-rooted trees into one snapshot.
#[allow(deprecated)] // root_uri is deprecated but kept for older LSP clients
fn primary_workspace_root(params: &InitializeParams) -> Option<PathBuf> {
    if let Some(folders) = &params.workspace_folders {
        if folders.len() > 1 {
            warn!(
                "{} workspace folders given; the program-engine backend serves exactly \
                 one AL app per session — using {} and ignoring the rest",
                folders.len(),
                folders[0].uri.as_str()
            );
        }
        if let Some(first) = folders.first() {
            return uri_to_path(&first.uri);
        }
    }
    params.root_uri.as_ref().and_then(uri_to_path)
}

/// Build the initial snapshot, publish its diagnostics, and spawn the
/// background updater. Returns `None` when [`LspSnapshot::build_full_with_parsed`]
/// fails (see the module doc's "no valid workspace" section) — the caller
/// logs the reason.
fn build_server_state(
    workspace_root: &Path,
    encoding: PositionEncoding,
    config: DiagnosticConfig,
    connection: &Connection,
) -> Option<ServerState> {
    let (initial, parsed) = LspSnapshot::build_full_with_parsed(workspace_root)?;
    let initial = Arc::new(initial);
    let shared = Arc::new(SharedSnapshot::new(Arc::clone(&initial)));

    let diag_state = Arc::new(Mutex::new(DiagnosticsState::new()));
    {
        let sender = connection.sender.clone();
        publish_diagnostics_diff(
            move |m| {
                if let Err(e) = sender.send(m) {
                    warn!("Failed to publish initial diagnostics: {}", e);
                }
            },
            &diag_state,
            &initial,
            encoding,
            &config,
        );
    }

    let (tx, rx) = mpsc::channel::<ChangeEvent>();
    let diag_state_bg = Arc::clone(&diag_state);
    let sender_bg = connection.sender.clone();
    let config_bg = config.clone();
    let updater_handle = spawn_updater(
        Arc::clone(&shared),
        rx,
        workspace_root.to_path_buf(),
        parsed,
        move |_old, new| {
            let sender = sender_bg.clone();
            publish_diagnostics_diff(
                move |m| {
                    if let Err(e) = sender.send(m) {
                        warn!("Failed to publish diagnostics: {}", e);
                    }
                },
                &diag_state_bg,
                new,
                encoding,
                &config_bg,
            );
        },
    );

    Some(ServerState {
        shared,
        tx,
        updater_handle,
        encoding,
        config,
    })
}

/// Recompute-diff-publish: run [`compute_all`] over `snap`, diff it through
/// `diag_state`, and hand every changed `(uri, diagnostics)` pair to `send`
/// as a `textDocument/publishDiagnostics` notification. Shared by the
/// initial (pre-updater) publish and every subsequent `on_swap` call so the
/// two paths can never drift apart. `send` is generic rather than a concrete
/// `crossbeam_channel::Sender<Message>` purely to avoid naming that type
/// here (an indirect dependency, never a direct one of this crate) — both
/// call sites just pass a small closure wrapping `Sender::send`.
fn publish_diagnostics_diff(
    send: impl Fn(Message),
    diag_state: &Mutex<DiagnosticsState>,
    snap: &LspSnapshot,
    enc: PositionEncoding,
    cfg: &DiagnosticConfig,
) {
    let all = compute_all(snap, enc, cfg);
    let changed = diag_state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .diff(all);

    for (uri, diagnostics) in changed {
        let Ok(uri) = uri.parse::<Uri>() else {
            warn!("Skipping diagnostics publish for an unparsable uri: {uri}");
            continue;
        };
        let params = PublishDiagnosticsParams {
            uri,
            diagnostics,
            version: None,
        };
        let notification = Notification::new(
            "textDocument/publishDiagnostics".to_string(),
            serde_json::to_value(params).expect("PublishDiagnosticsParams always serializes"),
        );
        send(Message::Notification(notification));
    }
}

/// Start the file watcher thread for incremental updates: every raw
/// [`FileChange`] is mapped onto the updater's [`ChangeEvent`] vocabulary
/// (see [`to_change_event`]) and pushed onto the SAME channel `didSave`
/// notifications use — one coalesced queue, so a save followed immediately
/// by the watcher's own notification of that same write no longer causes two
/// separate rebuilds (legacy's didSave + watcher paths each reindexed
/// independently).
fn start_file_watcher(tx: mpsc::Sender<ChangeEvent>, workspace_root: PathBuf) {
    thread::spawn(move || {
        let watcher = match AlFileWatcher::new(&workspace_root) {
            Ok(w) => w,
            Err(e) => {
                warn!(
                    "Failed to create file watcher for {}: {}",
                    workspace_root.display(),
                    e
                );
                return;
            }
        };
        info!(
            "File watcher thread started for {}",
            workspace_root.display()
        );

        loop {
            let Some(change) = watcher.recv_timeout(Duration::from_millis(100)) else {
                continue;
            };
            if tx.send(to_change_event(change)).is_err() {
                info!("Updater channel closed; stopping file watcher thread");
                return;
            }
        }
    });
}

/// Map a raw filesystem [`FileChange`] to the updater's [`ChangeEvent`]
/// vocabulary: a change under `.alpackages` (a dependency add/update/remove
/// — legacy never watched these at all, freezing dependency resolution at
/// startup) escalates to `DepsChanged` regardless of which side of the
/// Modified/Deleted split it came from; a backend-reported overflow escalates
/// to `Overflow` (also a forced full rebuild — see `ChangeEvent`'s doc);
/// everything else is a workspace `.al` file (the only other kind
/// [`AlFileWatcher`] forwards — see its own widened filter) and maps 1:1 onto
/// `FileSaved`/`FileRemoved`.
fn to_change_event(change: FileChange) -> ChangeEvent {
    match change {
        FileChange::Overflow => ChangeEvent::Overflow,
        FileChange::Modified(path) => {
            if path_is_dependency_source(&path) {
                ChangeEvent::DepsChanged
            } else {
                ChangeEvent::FileSaved(path)
            }
        }
        FileChange::Deleted(path) => {
            if path_is_dependency_source(&path) {
                ChangeEvent::DepsChanged
            } else {
                ChangeEvent::FileRemoved(path)
            }
        }
    }
}

fn path_is_dependency_source(path: &Path) -> bool {
    path.components().any(|c| {
        c.as_os_str()
            .to_str()
            .is_some_and(|s| s.eq_ignore_ascii_case(".alpackages"))
    })
}

/// Main message processing loop. Takes `connection` BY VALUE — see
/// `run_server`'s call site for why that (not `&Connection`) is required for
/// a clean process exit.
fn main_loop(connection: Connection, state: Option<&ServerState>) -> Result<()> {
    for msg in &connection.receiver {
        match msg {
            Message::Request(req) => {
                if connection.handle_shutdown(&req)? {
                    break;
                }

                let result = dispatch_request(&req, state);
                let response = match result {
                    Ok(value) => Response::new_ok(req.id, value),
                    Err(e) => Response::new_err(
                        req.id,
                        lsp_server::ErrorCode::InternalError as i32,
                        e.to_string(),
                    ),
                };

                connection
                    .sender
                    .send(Message::Response(response))
                    .context("Failed to send response")?;
            }
            Message::Response(_) => {}
            Message::Notification(notif) => {
                handle_notification(state, &notif);
            }
        }
    }

    Ok(())
}

/// Extract a `CallHierarchyItem`'s `data` payload as an [`ItemData`] —
/// shared by `incomingCalls`/`outgoingCalls`, both of which need it to look
/// up `data.node` in the current snapshot.
fn item_data(item: &CallHierarchyItem) -> Result<ItemData> {
    let data = item
        .data
        .clone()
        .ok_or_else(|| anyhow::anyhow!("call hierarchy item is missing its data payload"))?;
    Ok(serde_json::from_value(data)?)
}

/// Dispatch one LSP request to its handler. `state` is `None` exactly when
/// no valid workspace snapshot exists (see the module doc) — every
/// snapshot-backed method degrades to an empty result in that case;
/// `fieldProperties`/`actionProperties`/`telemetryStatus` are
/// graph-independent and answer unconditionally.
fn dispatch_request(req: &Request, state: Option<&ServerState>) -> Result<Value> {
    debug!("Request: {} - {:?}", req.method, req.params);

    match req.method.as_str() {
        "textDocument/prepareCallHierarchy" => {
            let params: CallHierarchyPrepareParams = serde_json::from_value(req.params.clone())?;
            let Some(state) = state else {
                return Ok(Value::Null);
            };
            let snap = state.shared.get();
            let pos = params.text_document_position_params.position;
            let result = prepare(
                &snap,
                state.encoding,
                params
                    .text_document_position_params
                    .text_document
                    .uri
                    .as_str(),
                pos.line,
                pos.character,
            );
            Ok(serde_json::to_value(result)?)
        }
        "callHierarchy/incomingCalls" => {
            let params: CallHierarchyIncomingCallsParams =
                serde_json::from_value(req.params.clone())?;
            let Some(state) = state else {
                return Ok(Value::Array(Vec::new()));
            };
            let snap = state.shared.get();
            let data = item_data(&params.item)?;
            let result = incoming(&snap, state.encoding, &data);
            Ok(serde_json::to_value(result)?)
        }
        "callHierarchy/outgoingCalls" => {
            let params: CallHierarchyOutgoingCallsParams =
                serde_json::from_value(req.params.clone())?;
            let Some(state) = state else {
                return Ok(Value::Array(Vec::new()));
            };
            let snap = state.shared.get();
            let data = item_data(&params.item)?;
            let result = outgoing(&snap, state.encoding, &data);
            Ok(serde_json::to_value(result)?)
        }
        "textDocument/codeLens" => {
            let params: CodeLensParams = serde_json::from_value(req.params.clone())?;
            let Some(state) = state else {
                return Ok(Value::Array(Vec::new()));
            };
            let snap = state.shared.get();
            let result = code_lenses(
                &snap,
                state.encoding,
                params.text_document.uri.as_str(),
                &state.config,
            );
            Ok(serde_json::to_value(result)?)
        }
        "al-call-hierarchy/fieldProperties" => {
            let params: SymbolPropertiesParams = serde_json::from_value(req.params.clone())?;
            Ok(serde_json::to_value(field_properties(params)?)?)
        }
        "al-call-hierarchy/actionProperties" => {
            let params: SymbolPropertiesParams = serde_json::from_value(req.params.clone())?;
            Ok(serde_json::to_value(action_properties(params)?)?)
        }
        "al-call-hierarchy/telemetryStatus" => {
            Ok(serde_json::to_value(crate::telemetry::status())?)
        }
        "al-call-hierarchy/dependencyDocumentSymbol" => {
            let params: DependencyDocumentSymbolParams =
                serde_json::from_value(req.params.clone())?;
            let Some(state) = state else {
                return Ok(Value::Array(Vec::new()));
            };
            let snap = state.shared.get();
            Ok(serde_json::to_value(dependency_document_symbol(
                &snap, params,
            ))?)
        }
        "al-call-hierarchy/eventPublishersInFile" => {
            let params: EventPublishersInFileParams = serde_json::from_value(req.params.clone())?;
            let Some(state) = state else {
                return Ok(Value::Array(Vec::new()));
            };
            let snap = state.shared.get();
            let result = event_publishers_in_file(&snap, state.encoding, &params.uri);
            Ok(serde_json::to_value(result)?)
        }
        "al-call-hierarchy/eventReferenceAtPosition" => {
            let params: EventReferenceAtPositionParams =
                serde_json::from_value(req.params.clone())?;
            let Some(state) = state else {
                return Ok(Value::Null);
            };
            let snap = state.shared.get();
            let result =
                event_reference_at_position(&snap, state.encoding, &params.uri, params.position);
            Ok(serde_json::to_value(result)?)
        }
        _ => {
            debug!("Unhandled method: {}", req.method);
            Ok(Value::Null)
        }
    }
}

/// Handle an LSP notification. Only `didSave` does anything: it queues a
/// [`ChangeEvent::FileSaved`] onto the SAME channel the file watcher feeds
/// (see [`start_file_watcher`]'s doc). `didOpen`/`didClose`/`didChange` are
/// no-ops, mirroring legacy — `text_document_sync.change` is negotiated as
/// `NONE`, so the server never relies on editor-buffer content anyway.
fn handle_notification(state: Option<&ServerState>, notif: &lsp_server::Notification) {
    debug!("Notification: {}", notif.method);

    match notif.method.as_str() {
        "textDocument/didSave" => {
            let Some(state) = state else {
                return;
            };
            let Ok(params) =
                serde_json::from_value::<DidSaveTextDocumentParams>(notif.params.clone())
            else {
                return;
            };
            let Some(path) = uri_to_path(&params.text_document.uri) else {
                return;
            };
            if path
                .extension()
                .map(|e| e.eq_ignore_ascii_case("al"))
                .unwrap_or(false)
            {
                debug!("Queueing save event for {}", path.display());
                if state.tx.send(ChangeEvent::FileSaved(path)).is_err() {
                    warn!("Updater channel closed; dropping didSave event");
                }
            }
        }
        "textDocument/didClose" => {}
        "textDocument/didOpen" => {}
        "textDocument/didChange" => {}
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// T3 Task 15 Step 3 smoke test. `tests/` has no existing LSP stdio harness
/// (`lsp_differential.rs`'s own doc notes `src/server.rs` is binary-only and
/// structurally unreachable from an integration-test crate) — `ServerState`/
/// `dispatch_request`/`handle_notification`/`build_server_state` are all
/// private to this module, so this lives here as an in-binary unit test
/// (`cargo test` runs these too) rather than under `tests/`.
///
/// Drives the REAL wiring this module assembles — `Connection::memory()`
/// stands in for stdio, `dispatch_request`/`handle_notification` are called
/// exactly as `main_loop` calls them — proving the cutover's plumbing, not
/// just the underlying `lsp::handlers`/`lsp::updater` functions in isolation
/// (already covered by Tasks 9-13's own tests).
#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::path_to_uri;
    use lsp_server::RequestId;
    use lsp_types::{CallHierarchyItem, CallHierarchyOutgoingCall, Diagnostic};
    use std::time::Instant;

    fn write_fixture_workspace(dir: &Path) {
        std::fs::write(
            dir.join("app.json"),
            r#"{
    "id": "55555555-0000-0000-0000-000000000015",
    "name": "Task15 Server Cutover Fixture",
    "publisher": "probe",
    "version": "1.0.0.0"
}"#,
        )
        .expect("write app.json");

        // `Extra` starts with zero callers (unused-procedure HINT expected
        // on the initial publish); the didSave edit below adds a call to it
        // from `DoWork`, which must clear that HINT on republish.
        std::fs::write(
            dir.join("Alpha.al"),
            r#"codeunit 50100 "Alpha"
{
    procedure DoWork()
    var
        Beta: Codeunit "Beta";
    begin
        Beta.Process();
    end;

    procedure Extra()
    begin
    end;
}
"#,
        )
        .expect("write Alpha.al");

        std::fs::write(
            dir.join("Beta.al"),
            r#"codeunit 50101 "Beta"
{
    procedure Process()
    begin
    end;
}
"#,
        )
        .expect("write Beta.al");

        // Gives `DoWork` a real caller so Alpha.al's ONLY initial finding is
        // `Extra()`'s unused hint — the didSave edit below clears exactly
        // that one, leaving Alpha.al with zero diagnostics (a clean
        // "republish clears to empty" assertion, not a partial one).
        std::fs::write(
            dir.join("Gamma.al"),
            r#"codeunit 50102 "Gamma"
{
    procedure Standalone()
    var
        Alpha: Codeunit "Alpha";
    begin
        Alpha.DoWork();
    end;
}
"#,
        )
        .expect("write Gamma.al");
    }

    #[test]
    fn dispatch_prepare_and_outgoing_then_didsave_bumps_generation_and_republishes_diagnostics() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_fixture_workspace(dir.path());

        let (server_conn, client_conn) = Connection::memory();
        let state = build_server_state(
            dir.path(),
            PositionEncoding::Utf8,
            DiagnosticConfig::default(),
            &server_conn,
        )
        .expect("build_server_state must succeed for a valid fixture workspace");
        assert_eq!(
            state.shared.get().generation,
            0,
            "a fresh build_full_with_parsed is generation 0"
        );

        let alpha_path = dir.path().join("Alpha.al");
        // Built from `LspSnapshot::workspace_root` (case-normalized on
        // Windows — see its own doc), the SAME construction
        // `diagnostics::workspace_uri` uses — NOT `path_to_uri(&alpha_path)`
        // directly, which would keep `dir.path()`'s on-disk casing and could
        // silently mismatch the snapshot's normalized root on a
        // case-insensitive filesystem.
        let alpha_uri = path_to_uri(&state.shared.get().workspace_root.join("Alpha.al"));

        // ── The initial publish must already flag Extra() as unused ───────
        let mut initial_alpha: Option<Vec<Diagnostic>> = None;
        {
            let deadline = Instant::now() + Duration::from_millis(1000);
            while initial_alpha.is_none() {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    break;
                }
                let Ok(msg) = client_conn.receiver.recv_timeout(remaining) else {
                    break;
                };
                if let Message::Notification(n) = msg
                    && n.method == "textDocument/publishDiagnostics"
                {
                    let params: PublishDiagnosticsParams =
                        serde_json::from_value(n.params).expect("valid publishDiagnostics params");
                    if params.uri.as_str() == alpha_uri.as_str() {
                        initial_alpha = Some(params.diagnostics);
                    }
                }
            }
        }
        let initial_alpha =
            initial_alpha.expect("the initial batch build must publish diagnostics for Alpha.al");
        assert!(
            initial_alpha
                .iter()
                .any(|d| d.message.contains("Extra") && d.message.contains("never called")),
            "Extra() has zero callers and must be flagged unused on the initial publish; got {initial_alpha:#?}"
        );

        // ── dispatch prepareCallHierarchy at Alpha.DoWork's name token ─────
        let do_work = {
            let snap = state.shared.get();
            snap.decls_by_file["Alpha.al"]
                .iter()
                .find(|d| d.name == "DoWork")
                .expect("DoWork decl")
                .clone()
        };
        let prepare_req = Request::new(
            RequestId::from(1),
            "textDocument/prepareCallHierarchy".to_string(),
            serde_json::json!({
                "textDocument": {"uri": alpha_uri.as_str()},
                "position": {
                    "line": do_work.name_origin.start.row,
                    "character": do_work.name_origin.start.column,
                },
            }),
        );
        let prepare_result =
            dispatch_request(&prepare_req, Some(&state)).expect("prepare dispatch must not error");
        let items: Vec<CallHierarchyItem> =
            serde_json::from_value(prepare_result).expect("prepare result deserializes");
        assert_eq!(items.len(), 1, "must resolve exactly Alpha.DoWork");
        assert_eq!(items[0].name, "DoWork");

        // ── dispatch outgoingCalls: Alpha.DoWork calls Beta.Process ────────
        let outgoing_req = Request::new(
            RequestId::from(2),
            "callHierarchy/outgoingCalls".to_string(),
            serde_json::json!({ "item": items[0] }),
        );
        let outgoing_result = dispatch_request(&outgoing_req, Some(&state))
            .expect("outgoing dispatch must not error");
        let outgoing: Vec<CallHierarchyOutgoingCall> =
            serde_json::from_value(outgoing_result).expect("outgoing result deserializes");
        assert!(
            outgoing.iter().any(|c| c.to.name == "Process"),
            "Alpha.DoWork's outgoing calls must include Beta.Process; got {outgoing:#?}"
        );

        // ── didSave: add a call to Extra() (still fingerprint-equal — rung
        // 1) — must swap in a new generation AND clear Extra()'s unused hint
        std::fs::write(
            &alpha_path,
            r#"codeunit 50100 "Alpha"
{
    procedure DoWork()
    var
        Beta: Codeunit "Beta";
    begin
        Beta.Process();
        Extra();
    end;

    procedure Extra()
    begin
    end;
}
"#,
        )
        .expect("rewrite Alpha.al");

        let did_save = Notification::new(
            "textDocument/didSave".to_string(),
            serde_json::json!({ "textDocument": {"uri": alpha_uri.as_str()} }),
        );
        handle_notification(Some(&state), &did_save);

        let mut alpha_after: Option<Vec<Diagnostic>> = None;
        let deadline = Instant::now() + Duration::from_millis(1000);
        while alpha_after.is_none() {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            let Ok(msg) = client_conn.receiver.recv_timeout(remaining) else {
                break;
            };
            if let Message::Notification(n) = msg
                && n.method == "textDocument/publishDiagnostics"
            {
                let params: PublishDiagnosticsParams =
                    serde_json::from_value(n.params).expect("valid publishDiagnostics params");
                if params.uri.as_str() == alpha_uri.as_str() {
                    alpha_after = Some(params.diagnostics);
                }
            }
        }
        let alpha_after = alpha_after.expect(
            "a didSave must eventually republish diagnostics for Alpha.al (Extra() clears)",
        );
        assert!(
            alpha_after.is_empty(),
            "Extra() is now called from DoWork — the unused-procedure hint must clear; got {alpha_after:#?}"
        );

        assert!(
            state.shared.get().generation > 0,
            "a didSave must bump the published generation"
        );

        let ServerState {
            tx, updater_handle, ..
        } = state;
        drop(tx);
        updater_handle
            .join()
            .expect("updater thread must exit cleanly");
    }
}
