//! LSP server implementation for call hierarchy

use anyhow::{Context, Result};
use log::{error, info, warn};
use lsp_server::{Connection, Message, Notification, Response};
use lsp_types::{
    CodeLensOptions, Diagnostic, DiagnosticSeverity, InitializeParams, InitializeResult,
    PositionEncodingKind, PublishDiagnosticsParams, ServerCapabilities,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::Duration;

use crate::config::DiagnosticConfig;
use crate::graph::CallGraph;
use crate::handlers::{get_unused_procedure_diagnostics, handle_notification, handle_request};
use crate::indexer::Indexer;
use crate::lsp::encoding::{PositionEncoding, negotiate};
use crate::protocol::{path_to_uri, uri_to_path};
use crate::watcher::{AlFileWatcher, FileChange};

/// Run the LSP server
pub fn run_server(no_watcher: bool, no_telemetry: bool) -> Result<()> {
    info!("Starting AL Call Hierarchy LSP server");

    let (connection, io_threads) = Connection::stdio();

    // Initialize
    let (id, params) = connection.initialize_start()?;
    let init_params: InitializeParams = serde_json::from_value(params)?;

    let workspace_root = init_params
        .workspace_folders
        .as_ref()
        .and_then(|folders| folders.first())
        .and_then(|f| crate::protocol::uri_to_path(&f.uri));
    let init_option_telemetry = init_params
        .initialization_options
        .as_ref()
        .and_then(|v| v.get("telemetry"))
        .and_then(|t| t.get("enabled"))
        .and_then(|b| b.as_bool());
    let telemetry_handle = crate::telemetry::init(crate::telemetry::TelemetryInputs {
        cli_no_telemetry: no_telemetry,
        init_option: init_option_telemetry,
        workspace_root,
        connection_string: option_env!("AL_CH_TELEMETRY_CONNECTION_STRING").map(String::from),
    });

    // H-12: negotiate the position encoding from `general.positionEncodings`
    // (LSP 3.17). Legacy handlers still serve byte columns regardless of the
    // negotiated value THIS task — for a utf-8-negotiating client that is now
    // correct; utf-16 clients stay unchanged-broken until the Task-15 cutover
    // wires `LineTable` conversion into the handlers. See CHANGELOG.
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

    // Index the workspace and set up file watchers
    let indexer = Arc::new(RwLock::new(Indexer::new()));
    let workspace_roots = index_workspaces(&indexer, &init_params);

    #[cfg(feature = "telemetry")]
    {
        let workspace_file_count = indexer
            .read()
            .map(|idx| {
                let g = idx.graph();
                (g.definition_count() + g.call_site_count()) as u32
            })
            .unwrap_or(0);
        let has_app_dependencies = workspace_roots
            .iter()
            .any(|root| root.join("app.json").exists());
        // Best-effort: derive dependency_count from app.json files present.
        let dependency_count = workspace_roots
            .iter()
            .filter(|root| root.join("app.json").exists())
            .count()
            .min(u8::MAX as usize) as u8;
        crate::telemetry::record_session_start(
            workspace_file_count,
            dependency_count,
            has_app_dependencies,
        );
    }

    // Load config from first workspace root (or use defaults)
    let config = workspace_roots
        .first()
        .map(|root| DiagnosticConfig::load(root))
        .unwrap_or_default();

    // Publish diagnostics after initial indexing
    publish_all_diagnostics(&connection, &indexer, &config);

    // Start file watcher thread for incremental updates (unless disabled)
    if no_watcher {
        info!("File watcher disabled (--no-watcher). Using LSP notifications for changes.");
    } else {
        start_file_watcher(Arc::clone(&indexer), workspace_roots);
    }

    // Main loop
    main_loop(&connection, &indexer, &config)?;

    io_threads.join()?;
    info!("Server shut down");
    crate::telemetry::shutdown(telemetry_handle);
    Ok(())
}

/// Index all workspace folders
#[allow(deprecated)] // root_uri is deprecated but kept for backward compatibility with older LSP clients
fn index_workspaces(indexer: &Arc<RwLock<Indexer>>, params: &InitializeParams) -> Vec<PathBuf> {
    let mut workspace_roots = Vec::new();

    if let Some(folders) = &params.workspace_folders {
        for folder in folders {
            if let Some(path) = uri_to_path(&folder.uri) {
                info!("Indexing workspace folder: {}", path.display());
                let mut idx = indexer.write().expect("Indexer lock poisoned");

                // Index local AL files
                if let Err(e) = idx.index_directory(&path) {
                    error!("Failed to index {}: {}", path.display(), e);
                    continue;
                }

                // Index external dependencies from .app packages
                if path.join("app.json").exists()
                    && let Err(e) = idx.index_dependencies(&path)
                {
                    warn!("Failed to index dependencies for {}: {}", path.display(), e);
                }

                workspace_roots.push(path);
            }
        }
    // Fallback to deprecated root_uri for backward compatibility with older LSP clients
    } else if let Some(ref uri) = params.root_uri
        && let Some(path) = uri_to_path(uri)
    {
        info!("Indexing root: {}", path.display());
        let mut idx = indexer.write().expect("Indexer lock poisoned");

        // Index local AL files
        if let Err(e) = idx.index_directory(&path) {
            error!("Failed to index {}: {}", path.display(), e);
        } else {
            // Index external dependencies from .app packages
            if path.join("app.json").exists()
                && let Err(e) = idx.index_dependencies(&path)
            {
                warn!("Failed to index dependencies for {}: {}", path.display(), e);
            }

            workspace_roots.push(path);
        }
    }

    workspace_roots
}

/// Start the file watcher thread for incremental updates
fn start_file_watcher(indexer: Arc<RwLock<Indexer>>, workspace_roots: Vec<PathBuf>) {
    thread::spawn(move || {
        let watchers: Vec<_> = workspace_roots
            .iter()
            .filter_map(|root| match AlFileWatcher::new(root) {
                Ok(w) => Some(w),
                Err(e) => {
                    warn!("Failed to create watcher for {}: {}", root.display(), e);
                    None
                }
            })
            .collect();

        if watchers.is_empty() {
            warn!("No file watchers active");
            return;
        }

        info!(
            "File watcher thread started with {} watchers",
            watchers.len()
        );

        loop {
            for watcher in &watchers {
                if let Some(change) = watcher.recv_timeout(Duration::from_millis(100)) {
                    match change {
                        FileChange::Modified(path) => {
                            info!("Re-indexing modified file: {}", path.display());
                            if let Err(e) = indexer
                                .write()
                                .expect("Indexer lock poisoned")
                                .reindex_file(&path)
                            {
                                error!("Failed to re-index {}: {}", path.display(), e);
                            }
                        }
                        FileChange::Deleted(path) => {
                            info!("Removing deleted file from index: {}", path.display());
                            if let Err(e) = indexer
                                .write()
                                .expect("Indexer lock poisoned")
                                .reindex_file(&path)
                            {
                                error!("Failed to remove {}: {}", path.display(), e);
                            }
                        }
                    }
                }
            }
        }
    });
}

/// Main message processing loop
fn main_loop(
    connection: &Connection,
    indexer: &Arc<RwLock<Indexer>>,
    config: &DiagnosticConfig,
) -> Result<()> {
    for msg in &connection.receiver {
        match msg {
            Message::Request(req) => {
                if connection.handle_shutdown(&req)? {
                    break;
                }

                let result = handle_request(indexer, &req, config);
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
                handle_notification(indexer, &notif);
            }
        }
    }

    Ok(())
}

/// Publish all diagnostics (unused procedures + code quality)
fn publish_all_diagnostics(
    connection: &Connection,
    indexer: &Arc<RwLock<Indexer>>,
    config: &DiagnosticConfig,
) {
    let indexer = indexer.read().expect("Indexer lock poisoned");
    let graph = indexer.graph();

    // Collect all diagnostics by file
    let mut file_diagnostics: HashMap<String, Vec<Diagnostic>> = HashMap::new();

    // Get unused procedure diagnostics (if enabled)
    if config.unused_procedures {
        for (file_path, diagnostics) in get_unused_procedure_diagnostics(&graph) {
            file_diagnostics
                .entry(file_path)
                .or_default()
                .extend(diagnostics);
        }
    }

    // Get code quality diagnostics
    for (file_path, diagnostics) in get_code_quality_diagnostics(&graph, config) {
        file_diagnostics
            .entry(file_path)
            .or_default()
            .extend(diagnostics);
    }

    // Publish diagnostics for each file
    for (file_path, diagnostics) in file_diagnostics {
        let uri = path_to_uri(std::path::Path::new(&file_path));
        let params = PublishDiagnosticsParams {
            uri,
            diagnostics,
            version: None,
        };

        let notification = Notification::new(
            "textDocument/publishDiagnostics".to_string(),
            serde_json::to_value(params).unwrap(),
        );

        if let Err(e) = connection.sender.send(Message::Notification(notification)) {
            warn!("Failed to publish diagnostics for {}: {}", file_path, e);
        }
    }

    info!("Published diagnostics for all files");
}

/// Get code quality diagnostics from the call graph
fn get_code_quality_diagnostics(
    graph: &CallGraph,
    config: &DiagnosticConfig,
) -> Vec<(String, Vec<Diagnostic>)> {
    let mut file_diagnostics: HashMap<String, Vec<Diagnostic>> = HashMap::new();

    // Iterate over all definitions and check for quality issues
    for (qname, def) in graph.iter_definitions() {
        let proc_name = graph.resolve(def.name).unwrap_or("Unknown");
        let obj_name = graph.resolve(def.object_name).unwrap_or("Unknown");
        let file_path = def.file.to_string_lossy().to_string();

        // Get the number of lines (approximation from range)
        let line_count = def.range.end.line.saturating_sub(def.range.start.line) + 1;
        let incoming_count = graph.get_incoming_call_count(qname);

        // Cyclomatic complexity warnings
        if config.complexity_enabled && def.complexity >= config.complexity_critical {
            let diagnostic = Diagnostic {
                range: def.range,
                severity: Some(DiagnosticSeverity::WARNING),
                code: Some(lsp_types::NumberOrString::String(
                    "high-complexity".to_string(),
                )),
                source: Some("al-call-hierarchy".to_string()),
                message: format!(
                    "Procedure '{}.{}' has cyclomatic complexity {} (critical threshold: {}) - consider simplifying",
                    obj_name, proc_name, def.complexity, config.complexity_critical
                ),
                related_information: None,
                tags: None,
                code_description: None,
                data: None,
            };
            file_diagnostics
                .entry(file_path.clone())
                .or_default()
                .push(diagnostic);
        } else if config.complexity_enabled && def.complexity >= config.complexity_warning {
            let diagnostic = Diagnostic {
                range: def.range,
                severity: Some(DiagnosticSeverity::INFORMATION),
                code: Some(lsp_types::NumberOrString::String(
                    "high-complexity".to_string(),
                )),
                source: Some("al-call-hierarchy".to_string()),
                message: format!(
                    "Procedure '{}.{}' has cyclomatic complexity {} (warning threshold: {})",
                    obj_name, proc_name, def.complexity, config.complexity_warning
                ),
                related_information: None,
                tags: None,
                code_description: None,
                data: None,
            };
            file_diagnostics
                .entry(file_path.clone())
                .or_default()
                .push(diagnostic);
        }

        // Parameter count warnings
        if config.params_enabled && def.parameter_count >= config.params_critical {
            let diagnostic = Diagnostic {
                range: def.range,
                severity: Some(DiagnosticSeverity::WARNING),
                code: Some(lsp_types::NumberOrString::String(
                    "too-many-parameters".to_string(),
                )),
                source: Some("al-call-hierarchy".to_string()),
                message: format!(
                    "Procedure '{}.{}' has {} parameters (critical threshold: {}) - consider using a record or reducing parameters",
                    obj_name, proc_name, def.parameter_count, config.params_critical
                ),
                related_information: None,
                tags: None,
                code_description: None,
                data: None,
            };
            file_diagnostics
                .entry(file_path.clone())
                .or_default()
                .push(diagnostic);
        } else if config.params_enabled && def.parameter_count >= config.params_warning {
            let diagnostic = Diagnostic {
                range: def.range,
                severity: Some(DiagnosticSeverity::INFORMATION),
                code: Some(lsp_types::NumberOrString::String(
                    "too-many-parameters".to_string(),
                )),
                source: Some("al-call-hierarchy".to_string()),
                message: format!(
                    "Procedure '{}.{}' has {} parameters (warning threshold: {})",
                    obj_name, proc_name, def.parameter_count, config.params_warning
                ),
                related_information: None,
                tags: None,
                code_description: None,
                data: None,
            };
            file_diagnostics
                .entry(file_path.clone())
                .or_default()
                .push(diagnostic);
        }

        // High fan-in warning (many callers)
        if config.fan_in_enabled && incoming_count > config.fan_in_warning {
            let diagnostic = Diagnostic {
                range: def.range,
                severity: Some(DiagnosticSeverity::INFORMATION),
                code: Some(lsp_types::NumberOrString::String("high-fan-in".to_string())),
                source: Some("al-call-hierarchy".to_string()),
                message: format!(
                    "Procedure '{}.{}' has {} callers - consider if it's doing too much",
                    obj_name, proc_name, incoming_count
                ),
                related_information: None,
                tags: None,
                code_description: None,
                data: None,
            };
            file_diagnostics
                .entry(file_path.clone())
                .or_default()
                .push(diagnostic);
        }

        // Long method warning
        if config.length_enabled && line_count > config.length_critical {
            let diagnostic = Diagnostic {
                range: def.range,
                severity: Some(DiagnosticSeverity::INFORMATION),
                code: Some(lsp_types::NumberOrString::String("long-method".to_string())),
                source: Some("al-call-hierarchy".to_string()),
                message: format!(
                    "Procedure '{}.{}' spans {} lines - consider breaking it down",
                    obj_name, proc_name, line_count
                ),
                related_information: None,
                tags: None,
                code_description: None,
                data: None,
            };
            file_diagnostics
                .entry(file_path)
                .or_default()
                .push(diagnostic);
        }
    }

    file_diagnostics.into_iter().collect()
}
