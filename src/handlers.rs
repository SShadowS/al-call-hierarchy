//! LSP request and notification handlers

use anyhow::{Context, Result};
use log::debug;
use lsp_server::Request;
use lsp_types::{
    CallHierarchyIncomingCall, CallHierarchyIncomingCallsParams, CallHierarchyItem,
    CallHierarchyOutgoingCall, CallHierarchyOutgoingCallsParams, CallHierarchyPrepareParams,
    CodeLens, CodeLensParams, Command, SymbolKind,
};
use serde_json::Value;
use std::sync::{Arc, RwLock};

use crate::graph::{CallGraph, DefinitionKind, QualifiedName};
use crate::indexer::Indexer;
use crate::protocol::{path_to_uri, uri_to_path};

/// Handle an LSP request
pub fn handle_request(indexer: &Arc<RwLock<Indexer>>, req: &Request) -> Result<Value> {
    debug!("Request: {} - {:?}", req.method, req.params);

    match req.method.as_str() {
        "textDocument/prepareCallHierarchy" => {
            let params: CallHierarchyPrepareParams = serde_json::from_value(req.params.clone())?;
            let result = prepare_call_hierarchy(indexer, params)?;
            Ok(serde_json::to_value(result)?)
        }
        "callHierarchy/incomingCalls" => {
            let params: CallHierarchyIncomingCallsParams =
                serde_json::from_value(req.params.clone())?;
            let result = incoming_calls(indexer, params)?;
            Ok(serde_json::to_value(result)?)
        }
        "callHierarchy/outgoingCalls" => {
            let params: CallHierarchyOutgoingCallsParams =
                serde_json::from_value(req.params.clone())?;
            let result = outgoing_calls(indexer, params)?;
            Ok(serde_json::to_value(result)?)
        }
        "textDocument/codeLens" => {
            let params: CodeLensParams = serde_json::from_value(req.params.clone())?;
            let result = code_lens(indexer, params)?;
            Ok(serde_json::to_value(result)?)
        }
        _ => {
            debug!("Unhandled method: {}", req.method);
            Ok(Value::Null)
        }
    }
}

/// Handle an LSP notification
pub fn handle_notification(_indexer: &Arc<RwLock<Indexer>>, notif: &lsp_server::Notification) {
    debug!("Notification: {}", notif.method);

    match notif.method.as_str() {
        "textDocument/didSave" | "textDocument/didChange" => {
            // Could trigger re-indexing here
        }
        _ => {}
    }
}

/// Prepare call hierarchy - find the item at the given position
fn prepare_call_hierarchy(
    indexer: &Arc<RwLock<Indexer>>,
    params: CallHierarchyPrepareParams,
) -> Result<Option<Vec<CallHierarchyItem>>> {
    let uri = &params.text_document_position_params.text_document.uri;
    let path = uri_to_path(uri).ok_or_else(|| anyhow::anyhow!("Invalid file URI"))?;

    let line = params.text_document_position_params.position.line;
    let character = params.text_document_position_params.position.character;

    let indexer = indexer.read().unwrap();
    let graph = indexer.graph();

    // Find definition at position
    if let Some(def) = graph.find_definition_at(&path, line, character) {
        let object_name = graph.resolve(def.object_name).unwrap_or("Unknown");
        let proc_name = graph.resolve(def.name).unwrap_or("Unknown");

        let item = CallHierarchyItem {
            name: proc_name.to_string(),
            kind: match def.kind {
                DefinitionKind::Procedure => SymbolKind::FUNCTION,
                DefinitionKind::Trigger => SymbolKind::EVENT,
                DefinitionKind::EventSubscriber => SymbolKind::EVENT,
            },
            tags: None,
            detail: Some(format!("{}.{}", object_name, proc_name)),
            uri: path_to_uri(&def.file),
            range: def.range,
            selection_range: def.range,
            data: Some(serde_json::json!({
                "object": object_name,
                "procedure": proc_name,
            })),
        };

        Ok(Some(vec![item]))
    } else {
        Ok(None)
    }
}

/// Get incoming calls - who calls this procedure
fn incoming_calls(
    indexer: &Arc<RwLock<Indexer>>,
    params: CallHierarchyIncomingCallsParams,
) -> Result<Option<Vec<CallHierarchyIncomingCall>>> {
    let data = params
        .item
        .data
        .as_ref()
        .context("Missing call hierarchy item data")?;

    let object: String = serde_json::from_value(data.get("object").cloned().unwrap_or_default())?;
    let procedure: String =
        serde_json::from_value(data.get("procedure").cloned().unwrap_or_default())?;

    let indexer = indexer.read().unwrap();
    let graph = indexer.graph();

    // Get the symbols
    let obj_sym = graph.get_symbol(&object);
    let proc_sym = graph.get_symbol(&procedure);

    if let (Some(obj_sym), Some(proc_sym)) = (obj_sym, proc_sym) {
        let qname = QualifiedName {
            object: obj_sym,
            procedure: proc_sym,
        };

        let calls = graph.get_incoming_calls(&qname);
        let mut results = Vec::new();

        // Add direct call sites
        for call in calls {
            let caller_name = graph.resolve(call.caller).unwrap_or("Unknown");

            // Find the caller's definition
            // For now, create a synthetic item
            let from_item = CallHierarchyItem {
                name: caller_name.to_string(),
                kind: SymbolKind::FUNCTION,
                tags: None,
                detail: None,
                uri: path_to_uri(&call.file),
                range: call.range,
                selection_range: call.range,
                data: None,
            };

            results.push(CallHierarchyIncomingCall {
                from: from_item,
                from_ranges: vec![call.range],
            });
        }

        // Add event subscribers (if this is a trigger/event)
        let event_subscribers = graph.get_event_subscribers(&qname);
        for sub in event_subscribers {
            let subscriber_obj = graph.resolve(sub.subscriber.object).unwrap_or("Unknown");
            let subscriber_proc = graph.resolve(sub.subscriber.procedure).unwrap_or("Unknown");

            let from_item = CallHierarchyItem {
                name: subscriber_proc.to_string(),
                kind: SymbolKind::EVENT,
                tags: None,
                detail: Some(format!("{}.{} [EventSubscriber]", subscriber_obj, subscriber_proc)),
                uri: path_to_uri(&sub.file),
                range: sub.range,
                selection_range: sub.range,
                data: Some(serde_json::json!({
                    "object": subscriber_obj,
                    "procedure": subscriber_proc,
                })),
            };

            results.push(CallHierarchyIncomingCall {
                from: from_item,
                from_ranges: vec![sub.range],
            });
        }

        Ok(Some(results))
    } else {
        Ok(None)
    }
}

/// Get outgoing calls - what does this procedure call
fn outgoing_calls(
    indexer: &Arc<RwLock<Indexer>>,
    params: CallHierarchyOutgoingCallsParams,
) -> Result<Option<Vec<CallHierarchyOutgoingCall>>> {
    let data = params
        .item
        .data
        .as_ref()
        .context("Missing call hierarchy item data")?;

    let object: String = serde_json::from_value(data.get("object").cloned().unwrap_or_default())?;
    let procedure: String =
        serde_json::from_value(data.get("procedure").cloned().unwrap_or_default())?;

    let indexer = indexer.read().unwrap();
    let graph = indexer.graph();

    // Get the symbols
    let obj_sym = graph.get_symbol(&object);
    let proc_sym = graph.get_symbol(&procedure);

    if let (Some(obj_sym), Some(proc_sym)) = (obj_sym, proc_sym) {
        let qname = QualifiedName {
            object: obj_sym,
            procedure: proc_sym,
        };

        let calls = graph.get_outgoing_calls(&qname);
        let mut results = Vec::new();

        for call in calls {
            let callee_method = graph.resolve(call.callee_method).unwrap_or("Unknown");
            let callee_obj = call
                .callee_object
                .and_then(|s| graph.resolve(s))
                .unwrap_or("local");

            let detail = if call.callee_object.is_some() {
                format!("{}.{}", callee_obj, callee_method)
            } else {
                callee_method.to_string()
            };

            // Try to find the target definition
            let to_item = if let Some(callee_obj_sym) = call.callee_object {
                let target_qname = QualifiedName {
                    object: callee_obj_sym,
                    procedure: call.callee_method,
                };

                if let Some(target_def) = graph.get_definition(&target_qname) {
                    // Local definition found
                    CallHierarchyItem {
                        name: callee_method.to_string(),
                        kind: SymbolKind::FUNCTION,
                        tags: None,
                        detail: Some(detail),
                        uri: path_to_uri(&target_def.file),
                        range: target_def.range,
                        selection_range: target_def.range,
                        data: Some(serde_json::json!({
                            "object": callee_obj,
                            "procedure": callee_method,
                        })),
                    }
                } else if let Some(ext_def) = graph.get_external_definition(&target_qname) {
                    // External definition found (from .app package)
                    let app_name = graph.resolve(ext_def.source.app_name).unwrap_or("external");
                    CallHierarchyItem {
                        name: callee_method.to_string(),
                        kind: SymbolKind::FUNCTION,
                        tags: None,
                        detail: Some(format!("{} (from {})", detail, app_name)),
                        uri: path_to_uri(&call.file),
                        range: call.range,
                        selection_range: call.range,
                        data: Some(serde_json::json!({
                            "external": true,
                            "app": app_name,
                        })),
                    }
                } else {
                    // Unresolved external call
                    CallHierarchyItem {
                        name: callee_method.to_string(),
                        kind: SymbolKind::FUNCTION,
                        tags: None,
                        detail: Some(format!("{} (external)", detail)),
                        uri: path_to_uri(&call.file),
                        range: call.range,
                        selection_range: call.range,
                        data: None,
                    }
                }
            } else {
                // Local/unqualified call
                CallHierarchyItem {
                    name: callee_method.to_string(),
                    kind: SymbolKind::FUNCTION,
                    tags: None,
                    detail: Some("(local)".to_string()),
                    uri: path_to_uri(&call.file),
                    range: call.range,
                    selection_range: call.range,
                    data: None,
                }
            };

            results.push(CallHierarchyOutgoingCall {
                to: to_item,
                from_ranges: vec![call.range],
            });
        }

        Ok(Some(results))
    } else {
        Ok(None)
    }
}

/// Get code lens - reference counts and quality metrics for procedures
fn code_lens(
    indexer: &Arc<RwLock<Indexer>>,
    params: CodeLensParams,
) -> Result<Option<Vec<CodeLens>>> {
    let uri = &params.text_document.uri;
    let path = uri_to_path(uri).ok_or_else(|| anyhow::anyhow!("Invalid file URI"))?;

    let indexer = indexer.read().unwrap();
    let graph = indexer.graph();

    let definitions = graph.get_definitions_in_file(&path);
    let mut results = Vec::new();

    for def in definitions {
        let qname = QualifiedName {
            object: def.object_name,
            procedure: def.name,
        };

        let ref_count = graph.get_incoming_call_count(&qname);
        let proc_name = graph.resolve(def.name).unwrap_or("Unknown");
        let obj_name = graph.resolve(def.object_name).unwrap_or("Unknown");

        // Calculate line count from range
        let line_count = def.range.end.line.saturating_sub(def.range.start.line) + 1;

        // Create a Code Lens showing reference count and quality metrics with threshold indicators
        let ref_text = if ref_count == 0 {
            "0 references".to_string()
        } else if ref_count == 1 {
            "1 reference".to_string()
        } else {
            format!("{} references", ref_count)
        };

        // Add threshold indicators for metrics
        // Complexity: ≥5 warning, ≥10 critical
        let complexity_text = if def.complexity >= 10 {
            format!("complexity: {} ⚠️ (>10)", def.complexity)
        } else if def.complexity >= 5 {
            format!("complexity: {} (>5)", def.complexity)
        } else {
            format!("complexity: {}", def.complexity)
        };

        // Lines: >50 is concerning
        let lines_text = if line_count > 50 {
            format!("lines: {} ⚠️ (>50)", line_count)
        } else {
            format!("lines: {}", line_count)
        };

        // Parameters: ≥4 warning, ≥7 critical
        let params_text = if def.parameter_count >= 7 {
            format!("params: {} ⚠️ (>7)", def.parameter_count)
        } else if def.parameter_count >= 4 {
            format!("params: {} (>4)", def.parameter_count)
        } else {
            format!("params: {}", def.parameter_count)
        };

        let title = format!(
            "{} | {}, {}, {}",
            ref_text, complexity_text, lines_text, params_text
        );

        results.push(CodeLens {
            range: def.range,
            command: Some(Command {
                title,
                command: "al-call-hierarchy.showReferences".to_string(),
                arguments: Some(vec![serde_json::json!({
                    "object": obj_name,
                    "procedure": proc_name,
                    "uri": uri.to_string(),
                })]),
            }),
            data: None,
        });
    }

    Ok(Some(results))
}

/// Helper function to get file diagnostics (used by server.rs for publishing)
pub fn get_unused_procedure_diagnostics(graph: &CallGraph) -> Vec<(String, Vec<lsp_types::Diagnostic>)> {
    use lsp_types::{Diagnostic, DiagnosticSeverity, DiagnosticTag};
    use std::collections::HashMap;

    let unused = graph.get_unused_procedures();
    let mut file_diagnostics: HashMap<String, Vec<Diagnostic>> = HashMap::new();

    for (_, def) in unused {
        let proc_name = graph.resolve(def.name).unwrap_or("Unknown");
        let obj_name = graph.resolve(def.object_name).unwrap_or("Unknown");

        let diagnostic = Diagnostic {
            range: def.range,
            severity: Some(DiagnosticSeverity::HINT),
            code: Some(lsp_types::NumberOrString::String("unused-procedure".to_string())),
            source: Some("al-call-hierarchy".to_string()),
            message: format!("Procedure '{}.{}' is never called", obj_name, proc_name),
            related_information: None,
            tags: Some(vec![DiagnosticTag::UNNECESSARY]),
            code_description: None,
            data: None,
        };

        let file_path = def.file.to_string_lossy().to_string();
        file_diagnostics
            .entry(file_path)
            .or_default()
            .push(diagnostic);
    }

    file_diagnostics.into_iter().collect()
}
