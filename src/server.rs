//! LSP server implementation for call hierarchy

use anyhow::{Context, Result};
use log::{error, info, warn};
use lsp_server::{Connection, Message, Notification, Response};
use lsp_types::{
    CodeLensOptions, Diagnostic, DiagnosticSeverity, InitializeParams,
    InitializeResult, PublishDiagnosticsParams, ServerCapabilities,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::Duration;

use crate::graph::CallGraph;
use crate::handlers::{get_unused_procedure_diagnostics, handle_notification, handle_request};
use crate::indexer::Indexer;
use crate::protocol::{path_to_uri, uri_to_path};
use crate::watcher::{AlFileWatcher, FileChange};

/// Run the LSP server
pub fn run_server() -> Result<()> {
    info!("Starting AL Call Hierarchy LSP server");

    let (connection, io_threads) = Connection::stdio();

    // Initialize
    let (id, params) = connection.initialize_start()?;
    let init_params: InitializeParams = serde_json::from_value(params)?;

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

    // Publish diagnostics after initial indexing
    publish_all_diagnostics(&connection, &indexer);

    // Start file watcher thread for incremental updates
    start_file_watcher(Arc::clone(&indexer), workspace_roots);

    // Main loop
    main_loop(&connection, &indexer)?;

    io_threads.join()?;
    info!("Server shut down");
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
                if path.join("app.json").exists() {
                    if let Err(e) = idx.index_dependencies(&path) {
                        warn!("Failed to index dependencies for {}: {}", path.display(), e);
                    }
                }

                workspace_roots.push(path);
            }
        }
    // Fallback to deprecated root_uri for backward compatibility with older LSP clients
    } else if let Some(ref uri) = params.root_uri {
        if let Some(path) = uri_to_path(uri) {
            info!("Indexing root: {}", path.display());
            let mut idx = indexer.write().expect("Indexer lock poisoned");

            // Index local AL files
            if let Err(e) = idx.index_directory(&path) {
                error!("Failed to index {}: {}", path.display(), e);
            } else {
                // Index external dependencies from .app packages
                if path.join("app.json").exists() {
                    if let Err(e) = idx.index_dependencies(&path) {
                        warn!("Failed to index dependencies for {}: {}", path.display(), e);
                    }
                }

                workspace_roots.push(path);
            }
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
fn main_loop(connection: &Connection, indexer: &Arc<RwLock<Indexer>>) -> Result<()> {
    for msg in &connection.receiver {
        match msg {
            Message::Request(req) => {
                if connection.handle_shutdown(&req)? {
                    break;
                }

                let result = handle_request(indexer, &req);
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
fn publish_all_diagnostics(connection: &Connection, indexer: &Arc<RwLock<Indexer>>) {
    let indexer = indexer.read().expect("Indexer lock poisoned");
    let graph = indexer.graph();

    // Collect all diagnostics by file
    let mut file_diagnostics: HashMap<String, Vec<Diagnostic>> = HashMap::new();

    // Get unused procedure diagnostics
    for (file_path, diagnostics) in get_unused_procedure_diagnostics(&graph) {
        file_diagnostics
            .entry(file_path)
            .or_default()
            .extend(diagnostics);
    }

    // Get code quality diagnostics
    for (file_path, diagnostics) in get_code_quality_diagnostics(&graph) {
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

// Thresholds for code quality diagnostics (matching analysis.rs)
const COMPLEXITY_WARNING: u32 = 5;
const COMPLEXITY_CRITICAL: u32 = 10;
const LENGTH_CRITICAL: u32 = 50;
const PARAMS_WARNING: u32 = 4;
const PARAMS_CRITICAL: u32 = 7;
const FAN_IN_WARNING: usize = 20;

/// Get code quality diagnostics from the call graph
fn get_code_quality_diagnostics(graph: &CallGraph) -> Vec<(String, Vec<Diagnostic>)> {
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
        if def.complexity >= COMPLEXITY_CRITICAL {
            let diagnostic = Diagnostic {
                range: def.range,
                severity: Some(DiagnosticSeverity::WARNING),
                code: Some(lsp_types::NumberOrString::String("high-complexity".to_string())),
                source: Some("al-call-hierarchy".to_string()),
                message: format!(
                    "Procedure '{}.{}' has cyclomatic complexity {} (critical threshold: {}) - consider simplifying",
                    obj_name, proc_name, def.complexity, COMPLEXITY_CRITICAL
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
        } else if def.complexity >= COMPLEXITY_WARNING {
            let diagnostic = Diagnostic {
                range: def.range,
                severity: Some(DiagnosticSeverity::INFORMATION),
                code: Some(lsp_types::NumberOrString::String("high-complexity".to_string())),
                source: Some("al-call-hierarchy".to_string()),
                message: format!(
                    "Procedure '{}.{}' has cyclomatic complexity {} (warning threshold: {})",
                    obj_name, proc_name, def.complexity, COMPLEXITY_WARNING
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
        if def.parameter_count >= PARAMS_CRITICAL {
            let diagnostic = Diagnostic {
                range: def.range,
                severity: Some(DiagnosticSeverity::WARNING),
                code: Some(lsp_types::NumberOrString::String("too-many-parameters".to_string())),
                source: Some("al-call-hierarchy".to_string()),
                message: format!(
                    "Procedure '{}.{}' has {} parameters (critical threshold: {}) - consider using a record or reducing parameters",
                    obj_name, proc_name, def.parameter_count, PARAMS_CRITICAL
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
        } else if def.parameter_count >= PARAMS_WARNING {
            let diagnostic = Diagnostic {
                range: def.range,
                severity: Some(DiagnosticSeverity::INFORMATION),
                code: Some(lsp_types::NumberOrString::String("too-many-parameters".to_string())),
                source: Some("al-call-hierarchy".to_string()),
                message: format!(
                    "Procedure '{}.{}' has {} parameters (warning threshold: {})",
                    obj_name, proc_name, def.parameter_count, PARAMS_WARNING
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
        if incoming_count > FAN_IN_WARNING {
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
        if line_count > LENGTH_CRITICAL {
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
