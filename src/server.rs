//! LSP server implementation for call hierarchy

use anyhow::{Context, Result};
use log::{error, info, warn};
use lsp_server::{Connection, Message, Response};
use lsp_types::{InitializeParams, InitializeResult, ServerCapabilities};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::Duration;

use crate::handlers::{handle_notification, handle_request};
use crate::indexer::Indexer;
use crate::protocol::uri_to_path;
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

    // Start file watcher thread for incremental updates
    start_file_watcher(Arc::clone(&indexer), workspace_roots);

    // Main loop
    main_loop(&connection, &indexer)?;

    io_threads.join()?;
    info!("Server shut down");
    Ok(())
}

/// Index all workspace folders
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
