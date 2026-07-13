//! LSP request and notification handlers

use anyhow::{Context, Result};
use log::{debug, error};
use lsp_server::Request;
use lsp_types::{
    CallHierarchyIncomingCall, CallHierarchyIncomingCallsParams, CallHierarchyItem,
    CallHierarchyOutgoingCall, CallHierarchyOutgoingCallsParams, CallHierarchyPrepareParams,
    CodeLens, CodeLensParams, Command, SymbolKind,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::{Arc, RwLock};

use crate::config::DiagnosticConfig;
use crate::graph::{
    CallGraph, DefinitionKind, DependencyMethodKind, DependencyObject, ObjectType, QualifiedName,
};
use crate::indexer::Indexer;
use crate::protocol::{path_to_uri, uri_to_path};

/// Handle an LSP request
pub fn handle_request(
    indexer: &Arc<RwLock<Indexer>>,
    req: &Request,
    config: &DiagnosticConfig,
) -> Result<Value> {
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
            let result = code_lens(indexer, params, config)?;
            Ok(serde_json::to_value(result)?)
        }
        "al-call-hierarchy/fieldProperties" => {
            let params: SymbolPropertiesParams = serde_json::from_value(req.params.clone())?;
            let result = field_properties(params)?;
            Ok(serde_json::to_value(result)?)
        }
        "al-call-hierarchy/actionProperties" => {
            let params: SymbolPropertiesParams = serde_json::from_value(req.params.clone())?;
            let result = action_properties(params)?;
            Ok(serde_json::to_value(result)?)
        }
        "al-call-hierarchy/telemetryStatus" => {
            let result = crate::telemetry::status();
            Ok(serde_json::to_value(result)?)
        }
        "al-call-hierarchy/dependencyDocumentSymbol" => {
            let params: DependencyDocumentSymbolParams =
                serde_json::from_value(req.params.clone())?;
            let result = dependency_document_symbol(indexer, params)?;
            Ok(serde_json::to_value(result)?)
        }
        "al-call-hierarchy/eventPublishersInFile" => {
            let params: EventPublishersInFileParams = serde_json::from_value(req.params.clone())?;
            let result = event_publishers_in_file(indexer, params)?;
            Ok(serde_json::to_value(result)?)
        }
        "al-call-hierarchy/eventReferenceAtPosition" => {
            let params: EventReferenceAtPositionParams =
                serde_json::from_value(req.params.clone())?;
            let result = event_reference_at_position(indexer, params)?;
            Ok(serde_json::to_value(result)?)
        }
        _ => {
            debug!("Unhandled method: {}", req.method);
            Ok(Value::Null)
        }
    }
}

/// Handle an LSP notification
pub fn handle_notification(indexer: &Arc<RwLock<Indexer>>, notif: &lsp_server::Notification) {
    debug!("Notification: {}", notif.method);

    match notif.method.as_str() {
        "textDocument/didSave" => {
            if let Ok(params) =
                serde_json::from_value::<lsp_types::DidSaveTextDocumentParams>(notif.params.clone())
                && let Some(path) = uri_to_path(&params.text_document.uri)
                && path
                    .extension()
                    .map(|e| e.eq_ignore_ascii_case("al"))
                    .unwrap_or(false)
            {
                debug!("Re-indexing saved file: {}", path.display());
                if let Err(e) = indexer
                    .write()
                    .expect("Indexer lock poisoned")
                    .reindex_file(&path)
                {
                    error!("Failed to re-index {}: {}", path.display(), e);
                }
            }
        }
        "textDocument/didClose" => {}
        "textDocument/didOpen" => {}
        "textDocument/didChange" => {}
        _ => {}
    }
}

/// Prepare call hierarchy - find the item at the given position
///
/// `pub` (T0.5): benches call this directly to measure the handler layer
/// in-process, without an LSP stdio loop.
pub fn prepare_call_hierarchy(
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
///
/// `pub` (T0.5): benches call this directly (see `prepare_call_hierarchy`).
pub fn incoming_calls(
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
                detail: Some(format!(
                    "{}.{} [EventSubscriber]",
                    subscriber_obj, subscriber_proc
                )),
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

        #[cfg(feature = "telemetry")]
        {
            if results.is_empty() {
                crate::telemetry::record_handler_empty(
                    "incomingCalls",
                    crate::telemetry::ObjectType::Other,
                    crate::telemetry::DefinitionKind::Procedure,
                    &object,
                    &procedure,
                );
            }
        }

        Ok(Some(results))
    } else {
        #[cfg(feature = "telemetry")]
        crate::telemetry::record_handler_empty(
            "incomingCalls",
            crate::telemetry::ObjectType::Other,
            crate::telemetry::DefinitionKind::Procedure,
            &object,
            &procedure,
        );
        Ok(None)
    }
}

/// Get outgoing calls - what does this procedure call
///
/// `pub` (T0.5): benches call this directly (see `prepare_call_hierarchy`).
pub fn outgoing_calls(
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

        #[cfg(feature = "telemetry")]
        {
            if results.is_empty() {
                crate::telemetry::record_handler_empty(
                    "outgoingCalls",
                    crate::telemetry::ObjectType::Other,
                    crate::telemetry::DefinitionKind::Procedure,
                    &object,
                    &procedure,
                );
            }
        }

        Ok(Some(results))
    } else {
        #[cfg(feature = "telemetry")]
        crate::telemetry::record_handler_empty(
            "outgoingCalls",
            crate::telemetry::ObjectType::Other,
            crate::telemetry::DefinitionKind::Procedure,
            &object,
            &procedure,
        );
        Ok(None)
    }
}

/// Get code lens - reference counts and quality metrics for procedures
///
/// `pub` (T3 Task 14): the adjudicated legacy-vs-new differential harness
/// (`tests/lsp_differential.rs`) is an external integration-test crate — it
/// needs this to drive legacy's `codeLens` surface exactly as `server.rs`
/// does, mirroring the same T0.5 precedent already applied to
/// `prepare_call_hierarchy`/`incoming_calls`/`outgoing_calls` above.
pub fn code_lens(
    indexer: &Arc<RwLock<Indexer>>,
    params: CodeLensParams,
    config: &DiagnosticConfig,
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
        let complexity_text = if def.complexity >= config.complexity_critical {
            format!(
                "complexity: {} ⚠️ (>{})",
                def.complexity, config.complexity_critical
            )
        } else if def.complexity >= config.complexity_warning {
            format!(
                "complexity: {} (>{})",
                def.complexity, config.complexity_warning
            )
        } else {
            format!("complexity: {}", def.complexity)
        };

        let lines_text = if line_count > config.length_critical {
            format!("lines: {} ⚠️ (>{})", line_count, config.length_critical)
        } else {
            format!("lines: {}", line_count)
        };

        let params_text = if def.parameter_count >= config.params_critical {
            format!(
                "params: {} ⚠️ (>{})",
                def.parameter_count, config.params_critical
            )
        } else if def.parameter_count >= config.params_warning {
            format!(
                "params: {} (>{})",
                def.parameter_count, config.params_warning
            )
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

// --- Symbol Properties (generic for fields, actions, and any AL declaration) ---

/// Parameters for al-call-hierarchy/fieldProperties and al-call-hierarchy/actionProperties
///
/// `pub` (T3 Task 15 cutover): these two request handlers touch no graph/
/// indexer state at all (pure source-read + al-syntax facade lookup), so the
/// cutover re-points the dispatcher straight at them rather than routing
/// through the now-unwired legacy `Indexer` — see `src/server.rs`'s dispatch.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SymbolPropertiesParams {
    uri: String,
    /// For fieldProperties
    #[serde(default)]
    field_name: String,
    /// For actionProperties
    #[serde(default)]
    action_name: String,
}

/// Generic response: all declared properties as key-value pairs.
/// Keys are human-readable property names (e.g., "Caption", "CalcFormula").
/// Only properties explicitly declared in source are included.
#[derive(Debug, Serialize, Default)]
pub struct SymbolPropertiesResult {
    /// For fields: the field ID number
    #[serde(skip_serializing_if = "Option::is_none")]
    field_id: Option<u32>,
    /// All declared properties from source (key = property name, value = property value)
    properties: Vec<PropertyEntry>,
}

/// A single property entry preserving declaration order
#[derive(Debug, Serialize)]
pub struct PropertyEntry {
    name: String,
    value: String,
}

/// Extract all properties for a table field, via the owned `al-syntax` facade.
pub fn field_properties(params: SymbolPropertiesParams) -> Result<SymbolPropertiesResult> {
    let source = read_source_from_uri(&params.uri)?;
    Ok(al_syntax::lookup_symbol_properties(
        &source,
        al_syntax::SymbolDeclKind::Field,
        &params.field_name,
    )
    .map(to_symbol_properties_result)
    .unwrap_or_default())
}

/// Extract all properties for a page action, via the owned `al-syntax` facade.
pub fn action_properties(params: SymbolPropertiesParams) -> Result<SymbolPropertiesResult> {
    let source = read_source_from_uri(&params.uri)?;
    Ok(al_syntax::lookup_symbol_properties(
        &source,
        al_syntax::SymbolDeclKind::Action,
        &params.action_name,
    )
    .map(to_symbol_properties_result)
    .unwrap_or_default())
}

/// Read an AL file's source from a `file:` URI (no parsing — al-syntax owns that).
fn read_source_from_uri(uri_str: &str) -> Result<String> {
    let uri: lsp_types::Uri = uri_str.parse().context("Invalid URI")?;
    let path = uri_to_path(&uri).ok_or_else(|| anyhow::anyhow!("Invalid file URI"))?;
    std::fs::read_to_string(&path).with_context(|| format!("Failed to read {}", path.display()))
}

/// Map the al-syntax facade result into the LSP response shape.
fn to_symbol_properties_result(p: al_syntax::SymbolProperties) -> SymbolPropertiesResult {
    SymbolPropertiesResult {
        field_id: p.field_id,
        properties: p
            .properties
            .into_iter()
            .map(|e| PropertyEntry {
                name: e.name,
                value: e.value,
            })
            .collect(),
    }
}

/// Helper function to get file diagnostics (used by server.rs for publishing)
pub fn get_unused_procedure_diagnostics(
    graph: &CallGraph,
) -> Vec<(String, Vec<lsp_types::Diagnostic>)> {
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
            code: Some(lsp_types::NumberOrString::String(
                "unused-procedure".to_string(),
            )),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_field_properties_extraction() {
        let source = r#"
table 50000 "TEST Customer"
{
    fields
    {
        field(1; "No."; Code[20])
        {
            Caption = 'No.';
            DataClassification = CustomerContent;
        }

        field(11; Balance; Decimal)
        {
            Caption = 'Balance';
            Editable = false;
            FieldClass = FlowField;
            CalcFormula = sum("Cust. Ledger Entry".Amount where("Customer No." = field("No.")));
        }

        field(20; "Payment Terms Code"; Code[10])
        {
            Caption = 'Payment Terms Code';
            DataClassification = CustomerContent;
            TableRelation = "Payment Terms";
        }
    }
}
"#;

        // Helper to find a property by name in the result
        fn prop(result: &SymbolPropertiesResult, name: &str) -> Option<String> {
            result
                .properties
                .iter()
                .find(|p| p.name == name)
                .map(|p| p.value.clone())
        }
        let lookup = |target: &str| {
            to_symbol_properties_result(
                al_syntax::lookup_symbol_properties(
                    source,
                    al_syntax::SymbolDeclKind::Field,
                    target,
                )
                .unwrap(),
            )
        };

        // Test Balance field (FlowField with CalcFormula)
        let result = lookup("balance");
        assert_eq!(result.field_id, Some(11));
        assert_eq!(prop(&result, "Caption").as_deref(), Some("'Balance'"));
        assert_eq!(prop(&result, "Editable").as_deref(), Some("false"));
        assert_eq!(prop(&result, "FieldClass").as_deref(), Some("FlowField"));
        assert!(prop(&result, "CalcFormula").is_some());
        assert!(
            prop(&result, "CalcFormula")
                .unwrap()
                .contains("Cust. Ledger Entry")
        );

        // Test Payment Terms Code field (with TableRelation)
        let result = lookup("payment terms code");
        assert_eq!(result.field_id, Some(20));
        assert!(prop(&result, "TableRelation").is_some());
        assert!(
            prop(&result, "TableRelation")
                .unwrap()
                .contains("Payment Terms")
        );

        // Test No. field (basic field)
        let result = lookup("no.");
        assert_eq!(result.field_id, Some(1));
        assert_eq!(prop(&result, "Caption").as_deref(), Some("'No.'"));
        assert_eq!(
            prop(&result, "DataClassification").as_deref(),
            Some("CustomerContent")
        );
        assert!(prop(&result, "FieldClass").is_none());
        assert!(prop(&result, "CalcFormula").is_none());
    }

    #[test]
    fn test_action_properties_extraction() {
        let source = r#"
page 50001 "TEST Customer Card"
{
    PageType = Card;
    SourceTable = "TEST Customer";

    actions
    {
        area(Navigation)
        {
            action(LedgerEntries)
            {
                ApplicationArea = All;
                Caption = 'Ledger E&ntries';
                Image = CustomerLedger;
                RunObject = page "Customer Ledger Entries";
                RunPageLink = "Customer No." = field("No.");
                RunPageView = sorting("Customer No.");
                ShortcutKey = 'Ctrl+F7';
                ToolTip = 'View the history of transactions for the customer.';
            }

            action(CheckCreditLimit)
            {
                ApplicationArea = All;
                Caption = 'Check Credit Limit';
                Image = Check;
                ToolTip = 'Check if the customer has exceeded their credit limit.';

                trigger OnAction()
                begin
                end;
            }
        }
    }
}
"#;

        // Helper to find a property by name
        fn prop(result: &SymbolPropertiesResult, name: &str) -> Option<String> {
            result
                .properties
                .iter()
                .find(|p| p.name == name)
                .map(|p| p.value.clone())
        }
        let lookup = |target: &str| {
            to_symbol_properties_result(
                al_syntax::lookup_symbol_properties(
                    source,
                    al_syntax::SymbolDeclKind::Action,
                    target,
                )
                .unwrap(),
            )
        };

        // Test LedgerEntries action (with RunObject)
        let result = lookup("ledgerentries");
        assert_eq!(
            prop(&result, "Caption").as_deref(),
            Some("'Ledger E&ntries'")
        );
        assert_eq!(prop(&result, "Image").as_deref(), Some("CustomerLedger"));
        assert!(prop(&result, "RunObject").is_some());
        assert!(
            prop(&result, "RunObject")
                .unwrap()
                .contains("Customer Ledger Entries")
        );
        assert!(prop(&result, "RunPageLink").is_some());
        assert!(prop(&result, "RunPageView").is_some());
        assert_eq!(prop(&result, "ShortcutKey").as_deref(), Some("'Ctrl+F7'"));
        assert!(prop(&result, "ToolTip").is_some());
        assert!(
            prop(&result, "ToolTip")
                .unwrap()
                .contains("history of transactions")
        );

        // Test CheckCreditLimit action (no RunObject, has trigger)
        let result = lookup("checkcreditlimit");
        assert_eq!(
            prop(&result, "Caption").as_deref(),
            Some("'Check Credit Limit'")
        );
        assert_eq!(prop(&result, "Image").as_deref(), Some("Check"));
        assert!(prop(&result, "RunObject").is_none());
        assert!(prop(&result, "ToolTip").is_some());
    }

    #[test]
    fn test_unused_procedure_diagnostics_finds_unused() {
        use crate::graph::*;

        let mut graph = CallGraph::new();
        let obj = graph.intern("TestCU");
        let used_proc = graph.intern("UsedProc");
        let unused_proc = graph.intern("UnusedProc");
        let caller = graph.intern("Caller");
        let file = graph.get_shared_path(std::path::Path::new("test.al"));

        graph.register_object(obj, ObjectType::Codeunit);

        graph.add_definition(Definition {
            file: file.clone(),
            range: lsp_types::Range {
                start: lsp_types::Position {
                    line: 10,
                    character: 4,
                },
                end: lsp_types::Position {
                    line: 20,
                    character: 8,
                },
            },
            object_type: ObjectType::Codeunit,
            object_name: obj,
            name: used_proc,
            kind: DefinitionKind::Procedure,
            complexity: 0,
            parameter_count: 0,
        });

        graph.add_definition(Definition {
            file: file.clone(),
            range: lsp_types::Range {
                start: lsp_types::Position {
                    line: 25,
                    character: 4,
                },
                end: lsp_types::Position {
                    line: 35,
                    character: 8,
                },
            },
            object_type: ObjectType::Codeunit,
            object_name: obj,
            name: unused_proc,
            kind: DefinitionKind::Procedure,
            complexity: 0,
            parameter_count: 0,
        });

        graph.add_definition(Definition {
            file: file.clone(),
            range: lsp_types::Range {
                start: lsp_types::Position {
                    line: 40,
                    character: 4,
                },
                end: lsp_types::Position {
                    line: 50,
                    character: 8,
                },
            },
            object_type: ObjectType::Codeunit,
            object_name: obj,
            name: caller,
            kind: DefinitionKind::Procedure,
            complexity: 0,
            parameter_count: 0,
        });

        // Caller calls UsedProc
        let caller_qname = QualifiedName {
            object: obj,
            procedure: caller,
        };
        graph.add_call_site(
            caller_qname,
            CallSite {
                file: file.clone(),
                range: lsp_types::Range {
                    start: lsp_types::Position {
                        line: 45,
                        character: 8,
                    },
                    end: lsp_types::Position {
                        line: 45,
                        character: 20,
                    },
                },
                caller,
                callee_object: None,
                callee_method: used_proc,
            },
        );

        let diagnostics = get_unused_procedure_diagnostics(&graph);
        // Should have diagnostics for the file
        assert!(!diagnostics.is_empty());
        // Find the file's diagnostics
        let file_diags: Vec<_> = diagnostics
            .iter()
            .flat_map(|(_, diags)| diags.iter())
            .collect();
        // unused_proc and caller should have diagnostics (caller is unused too since nobody calls it)
        assert!(!file_diags.is_empty());
        // Check that at least one diagnostic mentions "never called"
        assert!(
            file_diags
                .iter()
                .any(|d| d.message.contains("never called"))
        );
    }

    #[test]
    fn test_unused_procedure_diagnostics_excludes_triggers() {
        use crate::graph::*;

        let mut graph = CallGraph::new();
        let obj = graph.intern("TestCU");
        let trigger = graph.intern("OnRun");
        let file = graph.get_shared_path(std::path::Path::new("test.al"));

        graph.add_definition(Definition {
            file,
            range: lsp_types::Range {
                start: lsp_types::Position {
                    line: 10,
                    character: 4,
                },
                end: lsp_types::Position {
                    line: 20,
                    character: 8,
                },
            },
            object_type: ObjectType::Codeunit,
            object_name: obj,
            name: trigger,
            kind: DefinitionKind::Trigger,
            complexity: 0,
            parameter_count: 0,
        });

        let diagnostics = get_unused_procedure_diagnostics(&graph);
        // Triggers should not be reported as unused
        let all_diags: Vec<_> = diagnostics
            .iter()
            .flat_map(|(_, diags)| diags.iter())
            .collect();
        assert!(all_diags.is_empty());
    }

    #[test]
    fn test_unused_procedure_diagnostics_empty_graph() {
        use crate::graph::CallGraph;

        let graph = CallGraph::new();
        let diagnostics = get_unused_procedure_diagnostics(&graph);
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn test_prepare_call_hierarchy() {
        use crate::indexer::Indexer;
        use crate::protocol::path_to_uri;
        use lsp_types::CallHierarchyPrepareParams;
        use std::sync::{Arc, RwLock};

        let dir = tempfile::TempDir::new().unwrap();
        let file_path = dir.path().join("test.al");
        std::fs::write(
            &file_path,
            r#"codeunit 50100 "TestCU"
{
    procedure MyProcedure()
    begin
        Message('Hello');
    end;
}"#,
        )
        .unwrap();

        let mut indexer = Indexer::new();
        indexer.index_directory(dir.path()).unwrap();
        let indexer = Arc::new(RwLock::new(indexer));

        let uri = path_to_uri(&file_path);

        // Position inside MyProcedure (line 2 = the procedure declaration line)
        let params = CallHierarchyPrepareParams {
            text_document_position_params: lsp_types::TextDocumentPositionParams {
                text_document: lsp_types::TextDocumentIdentifier { uri: uri.clone() },
                position: lsp_types::Position {
                    line: 2,
                    character: 10,
                },
            },
            work_done_progress_params: Default::default(),
        };

        let result = prepare_call_hierarchy(&indexer, params).unwrap();
        assert!(
            result.is_some(),
            "Should find definition at procedure position"
        );
        let items = result.unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "MyProcedure");
        assert!(items[0].detail.as_ref().unwrap().contains("TestCU"));
    }

    #[test]
    fn test_prepare_call_hierarchy_no_match() {
        use crate::indexer::Indexer;
        use crate::protocol::path_to_uri;
        use lsp_types::CallHierarchyPrepareParams;
        use std::sync::{Arc, RwLock};

        let dir = tempfile::TempDir::new().unwrap();
        let file_path = dir.path().join("test.al");
        std::fs::write(
            &file_path,
            r#"codeunit 50100 "TestCU"
{
    procedure MyProcedure()
    begin
    end;
}"#,
        )
        .unwrap();

        let mut indexer = Indexer::new();
        indexer.index_directory(dir.path()).unwrap();
        let indexer = Arc::new(RwLock::new(indexer));

        let uri = path_to_uri(&file_path);

        // Position outside any procedure (line 0)
        let params = CallHierarchyPrepareParams {
            text_document_position_params: lsp_types::TextDocumentPositionParams {
                text_document: lsp_types::TextDocumentIdentifier { uri },
                position: lsp_types::Position {
                    line: 0,
                    character: 0,
                },
            },
            work_done_progress_params: Default::default(),
        };

        let result = prepare_call_hierarchy(&indexer, params).unwrap();
        assert!(
            result.is_none(),
            "Should not find definition outside procedure"
        );
    }

    #[test]
    fn test_code_lens_handler() {
        use crate::config::DiagnosticConfig;
        use crate::indexer::Indexer;
        use crate::protocol::path_to_uri;
        use lsp_types::CodeLensParams;
        use std::sync::{Arc, RwLock};

        let dir = tempfile::TempDir::new().unwrap();
        let file_path = dir.path().join("test.al");
        std::fs::write(
            &file_path,
            r#"codeunit 50100 "TestCU"
{
    procedure SimpleProc()
    begin
        Message('Hello');
    end;

    procedure CalledProc()
    begin
    end;

    procedure Caller1()
    begin
        CalledProc();
    end;

    procedure Caller2()
    begin
        CalledProc();
    end;
}"#,
        )
        .unwrap();

        let mut indexer = Indexer::new();
        indexer.index_directory(dir.path()).unwrap();
        let indexer = Arc::new(RwLock::new(indexer));

        let uri = path_to_uri(&file_path);
        let config = DiagnosticConfig::default();

        let params = CodeLensParams {
            text_document: lsp_types::TextDocumentIdentifier { uri },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let result = super::code_lens(&indexer, params, &config).unwrap();
        assert!(result.is_some());
        let lenses = result.unwrap();

        // Should have one CodeLens per procedure (4 procedures)
        assert_eq!(
            lenses.len(),
            4,
            "Should have CodeLens for each procedure. Got titles: {:?}",
            lenses
                .iter()
                .map(|l| l.command.as_ref().map(|c| &c.title))
                .collect::<Vec<_>>()
        );

        // Find CalledProc's lens - it should show "2 references"
        let called_lens = lenses.iter().find(|l| {
            l.command
                .as_ref()
                .map(|c| c.title.contains("2 references"))
                .unwrap_or(false)
        });
        assert!(
            called_lens.is_some(),
            "CalledProc should show '2 references'. Lens titles: {:?}",
            lenses
                .iter()
                .map(|l| l.command.as_ref().map(|c| &c.title))
                .collect::<Vec<_>>()
        );

        // All lenses should have complexity and line info
        for lens in &lenses {
            let title = &lens.command.as_ref().unwrap().title;
            assert!(
                title.contains("complexity:"),
                "Lens should show complexity: {}",
                title
            );
            assert!(
                title.contains("lines:"),
                "Lens should show lines: {}",
                title
            );
            assert!(
                title.contains("params:"),
                "Lens should show params: {}",
                title
            );
        }
    }

    #[test]
    fn test_code_lens_empty_file() {
        use crate::config::DiagnosticConfig;
        use crate::indexer::Indexer;
        use crate::protocol::path_to_uri;
        use lsp_types::CodeLensParams;
        use std::sync::{Arc, RwLock};

        let dir = tempfile::TempDir::new().unwrap();
        let file_path = dir.path().join("empty.al");
        std::fs::write(&file_path, "// no procedures here").unwrap();

        let mut indexer = Indexer::new();
        indexer.index_directory(dir.path()).unwrap();
        let indexer = Arc::new(RwLock::new(indexer));

        let uri = path_to_uri(&file_path);
        let config = DiagnosticConfig::default();

        let params = CodeLensParams {
            text_document: lsp_types::TextDocumentIdentifier { uri },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let result = super::code_lens(&indexer, params, &config).unwrap();
        assert!(result.is_some());
        assert!(
            result.unwrap().is_empty(),
            "Empty file should have no CodeLens"
        );
    }

    #[test]
    fn test_incoming_calls_handler() {
        use crate::indexer::Indexer;
        use crate::protocol::path_to_uri;
        use lsp_types::{CallHierarchyIncomingCallsParams, CallHierarchyItem, SymbolKind};
        use std::sync::{Arc, RwLock};

        let dir = tempfile::TempDir::new().unwrap();
        let file_path = dir.path().join("test.al");
        std::fs::write(
            &file_path,
            r#"codeunit 50100 "TestCU"
{
    procedure Caller1()
    begin
        TargetProc();
    end;

    procedure Caller2()
    begin
        TargetProc();
    end;

    procedure TargetProc()
    begin
    end;
}"#,
        )
        .unwrap();

        let mut indexer = Indexer::new();
        indexer.index_directory(dir.path()).unwrap();
        let indexer = Arc::new(RwLock::new(indexer));

        let uri = path_to_uri(&file_path);
        let params = CallHierarchyIncomingCallsParams {
            item: CallHierarchyItem {
                name: "TargetProc".to_string(),
                kind: SymbolKind::FUNCTION,
                tags: None,
                detail: None,
                uri,
                range: Default::default(),
                selection_range: Default::default(),
                data: Some(serde_json::json!({
                    "object": "TestCU",
                    "procedure": "TargetProc",
                })),
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let result = super::incoming_calls(&indexer, params).unwrap();
        assert!(result.is_some());
        let calls = result.unwrap();
        assert_eq!(calls.len(), 2, "TargetProc should have 2 callers");
    }

    #[test]
    fn test_outgoing_calls_handler() {
        use crate::indexer::Indexer;
        use crate::protocol::path_to_uri;
        use lsp_types::{CallHierarchyItem, CallHierarchyOutgoingCallsParams, SymbolKind};
        use std::sync::{Arc, RwLock};

        let dir = tempfile::TempDir::new().unwrap();
        let file_path = dir.path().join("caller.al");
        std::fs::write(
            &file_path,
            r#"codeunit 50100 "CallerCU"
{
    procedure DoWork()
    begin
        HelperA();
        HelperB();
    end;

    procedure HelperA()
    begin
    end;

    procedure HelperB()
    begin
    end;
}"#,
        )
        .unwrap();

        let mut indexer = Indexer::new();
        indexer.index_directory(dir.path()).unwrap();
        let indexer = Arc::new(RwLock::new(indexer));

        let uri = path_to_uri(&file_path);
        let params = CallHierarchyOutgoingCallsParams {
            item: CallHierarchyItem {
                name: "DoWork".to_string(),
                kind: SymbolKind::FUNCTION,
                tags: None,
                detail: None,
                uri,
                range: Default::default(),
                selection_range: Default::default(),
                data: Some(serde_json::json!({
                    "object": "CallerCU",
                    "procedure": "DoWork",
                })),
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let result = super::outgoing_calls(&indexer, params).unwrap();
        assert!(result.is_some());
        let calls = result.unwrap();
        assert_eq!(calls.len(), 2, "DoWork should call 2 procedures");
    }

    #[test]
    fn test_incoming_calls_unknown_symbol() {
        use crate::indexer::Indexer;
        use lsp_types::{CallHierarchyIncomingCallsParams, CallHierarchyItem, SymbolKind};
        use std::str::FromStr;
        use std::sync::{Arc, RwLock};

        let indexer = Arc::new(RwLock::new(Indexer::new()));

        let params = CallHierarchyIncomingCallsParams {
            item: CallHierarchyItem {
                name: "NonExistent".to_string(),
                kind: SymbolKind::FUNCTION,
                tags: None,
                detail: None,
                uri: lsp_types::Uri::from_str("file:///test.al").unwrap(),
                range: Default::default(),
                selection_range: Default::default(),
                data: Some(serde_json::json!({
                    "object": "NoCU",
                    "procedure": "NonExistent",
                })),
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let result = super::incoming_calls(&indexer, params).unwrap();
        assert!(result.is_none(), "Unknown symbol should return None");
    }
}

// =====================================================================
// dependencyDocumentSymbol
// =====================================================================
//
// Custom request that synthesizes an LSP `DocumentSymbol[]` response for a
// dependency object identified by an `al-preview:/allang/{App}/{Type}/{Id}/{Name}.dal`
// URI. The wrapper falls back to this when AL LSP's documentSymbol on a
// virtual URI returns empty (which happens in practice).

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DependencyDocumentSymbolParams {
    /// al-preview:/ URI, or a hint object specifying app/type/name directly.
    #[serde(default)]
    pub uri: Option<String>,
    #[serde(default)]
    pub app: Option<String>,
    #[serde(default)]
    pub object_type: Option<String>,
    #[serde(default)]
    pub object_name: Option<String>,
    #[serde(default)]
    #[allow(dead_code)] // parsed from request; not yet read (future design)
    pub object_id: Option<i64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DependencyDocumentSymbol {
    name: String,
    detail: String,
    /// LSP SymbolKind value (Event=24, Method=6, Field=8, etc.).
    kind: u32,
    /// LSP SymbolTag values, e.g. [1] for deprecated. Currently always [].
    tags: Vec<u32>,
    range: DependencyRange,
    selection_range: DependencyRange,
}

#[derive(Debug, Serialize, Clone, Copy)]
struct DependencyRange {
    start: DependencyPosition,
    end: DependencyPosition,
}

#[derive(Debug, Serialize, Clone, Copy)]
struct DependencyPosition {
    line: u32,
    character: u32,
}

const ZERO_RANGE: DependencyRange = DependencyRange {
    start: DependencyPosition {
        line: 0,
        character: 0,
    },
    end: DependencyPosition {
        line: 0,
        character: 0,
    },
};

/// Resolve a dependency object by parsing the URI hints or the explicit fields.
fn resolve_dependency_object<'a>(
    graph: &'a CallGraph,
    params: &DependencyDocumentSymbolParams,
) -> Option<&'a DependencyObject> {
    let parts = params.uri.as_deref().and_then(parse_al_preview_uri);

    let (app, otype, name) = match (parts, params) {
        (Some(uri_parts), _) => uri_parts,
        (None, p) => {
            let app = p.app.clone().unwrap_or_default();
            let otype_str = p.object_type.clone().unwrap_or_default();
            let otype: ObjectType = otype_str.as_str().try_into().ok()?;
            let name = p.object_name.clone().unwrap_or_default();
            (app, otype, name)
        }
    };

    // First try app+type+name (most specific). Then fall back to type+name across
    // all apps (in case the URI's App segment doesn't match the .app manifest name
    // exactly — Microsoft uses "Base Application" while VS Code may say "Base App").
    if !app.is_empty()
        && let Some(obj) = graph.get_dependency_object(&app, otype, &name)
    {
        return Some(obj);
    }
    graph.find_dependency_object_by_type_name(otype, &name)
}

/// Parse an `al-preview:/allang/{App}/{Type}/{Id}/{Name}.dal` URI into its parts.
/// Returns (app_name, object_type, object_name). Tolerates URL-encoded segments
/// and unusual scheme separators.
///
/// `pub(crate)` (T3 Task 11 review fix-wave): `src/lsp/handlers.rs`'s
/// `abi_symbol_uri` mints URIs in this SAME `al-preview://` object-level
/// layout for the fresh engine's `RouteTarget::AbiSymbol` fallback item —
/// its own conformance test calls this parser directly to prove the emitted
/// URI actually round-trips through the ONE real consumer this scheme has
/// today, rather than merely resembling it by eye.
pub(crate) fn parse_al_preview_uri(uri: &str) -> Option<(String, ObjectType, String)> {
    // Strip scheme and any number of leading slashes.
    let rest = uri.strip_prefix("al-preview:")?;
    let rest = rest.trim_start_matches('/');

    // Expect "allang/<App>/<Type>/<Id>/<Name>.dal" — but the App name and the
    // object Name can themselves contain '/', so a naive split is wrong.
    // Heuristic: locate the ".dal" suffix and walk segments from there.
    let trimmed = rest.strip_suffix(".dal").unwrap_or(rest);
    let segments: Vec<&str> = trimmed.split('/').collect();
    if segments.len() < 5 {
        return None;
    }

    // Layout: ["allang", <App pieces...>, <Type>, <Id>, <Name pieces...>]
    // Walk from the right: last segment(s) = Name, segment before Id = Type,
    // before that = Id, everything between "allang" and Type = App.
    //
    // The simplest robust approach: the *type* segment must match a known
    // ObjectType (case-insensitive). Scan segments from index 1 onward and
    // pick the first that parses as ObjectType — that anchors the layout.
    let mut type_idx = None;
    for (i, seg) in segments.iter().enumerate().skip(1) {
        let decoded = urldecode(seg);
        if ObjectType::try_from(decoded.as_str()).is_ok() {
            type_idx = Some(i);
            break;
        }
    }
    let type_idx = type_idx?;
    if type_idx + 2 > segments.len() - 1 {
        // need Id and at least one name segment after Type
        return None;
    }

    let object_type: ObjectType =
        ObjectType::try_from(urldecode(segments[type_idx]).as_str()).ok()?;
    let app_parts: Vec<String> = segments[1..type_idx].iter().map(|s| urldecode(s)).collect();
    let app = app_parts.join("/");

    // Skip Id segment, take rest as Name (may contain slashes if Microsoft ever does that).
    let name_parts: Vec<String> = segments[type_idx + 2..]
        .iter()
        .map(|s| urldecode(s))
        .collect();
    let mut name = name_parts.join("/");
    // The original Name segment may also have included the trailing ".dal";
    // we already stripped it once from the whole URI, but if it landed inside
    // the name segment due to splitting, strip again.
    if let Some(stripped) = name.strip_suffix(".dal") {
        name = stripped.to_string();
    }

    Some((app, object_type, name))
}

/// Minimal URL-decoder for the percent-encoded segments AL LSP may emit.
/// Avoids pulling in another crate.
fn urldecode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'%' && i + 2 < bytes.len() {
            let hi = (bytes[i + 1] as char).to_digit(16);
            let lo = (bytes[i + 2] as char).to_digit(16);
            if let (Some(h), Some(l)) = (hi, lo) {
                out.push(((h << 4) | l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

fn method_kind_to_lsp_kind(kind: DependencyMethodKind) -> u32 {
    // LSP SymbolKind values
    match kind {
        DependencyMethodKind::IntegrationEvent
        | DependencyMethodKind::BusinessEvent
        | DependencyMethodKind::InternalEvent => 24, // Event
        DependencyMethodKind::EventSubscriber => 6, // Method (still a method, just tagged)
        DependencyMethodKind::Procedure => 6,       // Method
    }
}

fn dependency_document_symbol(
    indexer: &Arc<RwLock<Indexer>>,
    params: DependencyDocumentSymbolParams,
) -> Result<Vec<DependencyDocumentSymbol>> {
    let indexer = indexer.read().expect("Indexer lock poisoned");
    let graph_guard = indexer.graph();
    let Some(obj) = resolve_dependency_object(&graph_guard, &params) else {
        debug!(
            "dependencyDocumentSymbol: no match for uri={:?} app={:?} type={:?} name={:?}",
            params.uri, params.app, params.object_type, params.object_name
        );
        return Ok(Vec::new());
    };

    let mut symbols = Vec::with_capacity(obj.methods.len());
    for m in &obj.methods {
        let kind = method_kind_to_lsp_kind(m.kind);
        let mut detail = m.signature.clone();
        if !m.kind.tag().is_empty() {
            // Prepend the attribute tag so it shows up at the start of the detail
            // string in editor symbol pickers.
            detail = format!("{} {}", m.kind.tag(), detail);
        }
        symbols.push(DependencyDocumentSymbol {
            name: m.name.clone(),
            detail,
            kind,
            tags: Vec::new(),
            range: ZERO_RANGE,
            selection_range: ZERO_RANGE,
        });
    }

    debug!(
        "dependencyDocumentSymbol: {} symbols for {} {}",
        symbols.len(),
        obj.object_type,
        obj.object_name
    );
    Ok(symbols)
}

#[cfg(test)]
mod dependency_doc_symbol_tests {
    use super::*;
    use crate::graph::ObjectType;

    #[test]
    fn parse_uri_basic() {
        let uri = "al-preview:/allang/Base Application/Codeunit/1535/Approvals Mgmt..dal";
        let (app, ty, name) = parse_al_preview_uri(uri).expect("parse");
        assert_eq!(app, "Base Application");
        assert_eq!(ty, ObjectType::Codeunit);
        assert_eq!(name, "Approvals Mgmt.");
    }

    #[test]
    fn parse_uri_with_percent_encoding() {
        let uri = "al-preview:/allang/Base%20Application/Codeunit/1535/Approvals%20Mgmt..dal";
        let (app, ty, name) = parse_al_preview_uri(uri).expect("parse");
        assert_eq!(app, "Base Application");
        assert_eq!(ty, ObjectType::Codeunit);
        assert_eq!(name, "Approvals Mgmt.");
    }

    #[test]
    fn parse_uri_multi_slash() {
        let uri = "al-preview:///allang/Base Application/Codeunit/1535/Approvals Mgmt..dal";
        let (_, ty, name) = parse_al_preview_uri(uri).expect("parse");
        assert_eq!(ty, ObjectType::Codeunit);
        assert_eq!(name, "Approvals Mgmt.");
    }

    /// End-to-end check that wires together: real .app parsing → indexer →
    /// CallGraph dependency_objects → dependencyDocumentSymbol RPC. Confirms
    /// that the synthesized DocumentSymbol[] contains the right counts and
    /// that IntegrationEvent publishers are tagged with SymbolKind::Event (24).
    #[test]
    fn rpc_on_approvals_mgmt() {
        use crate::indexer::Indexer;
        use std::path::Path;
        use std::sync::{Arc, RwLock};

        // Test/ has both a Base Application .app and an app.json that lists
        // dependencies, so the indexer can resolve them.
        let alpackages_root = Path::new(r"U:\Git\DO.Support-wi-75360\DocumentOutput\Test");
        if !alpackages_root.join(".alpackages").exists() {
            eprintln!("Skipping: test .alpackages not present");
            return;
        }

        let indexer = Indexer::new();
        indexer
            .index_dependencies(alpackages_root)
            .expect("index .alpackages");

        let indexer_arc = Arc::new(RwLock::new(indexer));
        let result = super::dependency_document_symbol(
            &indexer_arc,
            super::DependencyDocumentSymbolParams {
                uri: Some(
                    "al-preview:/allang/Base Application/Codeunit/1535/Approvals Mgmt..dal".into(),
                ),
                app: None,
                object_type: None,
                object_name: None,
                object_id: None,
            },
        )
        .expect("rpc");

        assert!(
            result.len() > 100,
            "Expected many methods; got {}",
            result.len()
        );

        let event_count = result.iter().filter(|s| s.kind == 24).count();
        assert!(
            event_count > 50,
            "Expected many event publishers; got {}",
            event_count
        );

        // Sanity: an event entry should have a tagged detail string and a real signature.
        let any_event = result.iter().find(|s| s.kind == 24).unwrap();
        assert!(
            any_event.detail.starts_with("[IntegrationEvent]")
                || any_event.detail.starts_with("[BusinessEvent]")
        );
        assert!(
            any_event.detail.contains('('),
            "Detail must contain signature"
        );

        eprintln!(
            "rpc_on_approvals_mgmt: total={} events={} sample-detail={}",
            result.len(),
            event_count,
            any_event.detail
        );
    }
}

// =====================================================================
// eventPublishersInFile
// =====================================================================
//
// Returns event publishers detected in a workspace .al file, shaped as
// DocumentSymbol entries. The wrapper overlays these on AL LSP's
// documentSymbol response to tag matching procedures with kind:Event +
// attribute-tagged detail strings.

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventPublishersInFileParams {
    pub uri: String,
}

fn event_publishers_in_file(
    indexer: &Arc<RwLock<Indexer>>,
    params: EventPublishersInFileParams,
) -> Result<Vec<DependencyDocumentSymbol>> {
    let uri = match params.uri.parse::<lsp_types::Uri>() {
        Ok(u) => u,
        Err(e) => {
            debug!("eventPublishersInFile: invalid uri {}: {}", params.uri, e);
            return Ok(Vec::new());
        }
    };
    let Some(path) = uri_to_path(&uri) else {
        return Ok(Vec::new());
    };

    let indexer = indexer.read().expect("Indexer lock poisoned");
    let graph = indexer.graph();
    let publishers = graph.get_local_event_publishers(&path);

    let symbols = publishers
        .iter()
        .map(|p| {
            let kind = 24u32; // SymbolKind::Event
            let tag = p.kind.tag();
            let detail = if tag.is_empty() {
                p.signature.clone()
            } else {
                format!("{} {}", tag, p.signature)
            };
            DependencyDocumentSymbol {
                name: p.name.clone(),
                detail,
                kind,
                tags: Vec::new(),
                range: lsp_range_to_dep_range(p.range),
                selection_range: lsp_range_to_dep_range(p.selection_range),
            }
        })
        .collect();

    Ok(symbols)
}

fn lsp_range_to_dep_range(r: lsp_types::Range) -> DependencyRange {
    DependencyRange {
        start: DependencyPosition {
            line: r.start.line,
            character: r.start.character,
        },
        end: DependencyPosition {
            line: r.end.line,
            character: r.end.character,
        },
    }
}

// =====================================================================
// eventReferenceAtPosition
// =====================================================================
//
// Given (uri, position), determine whether the cursor is on an event-name
// string literal inside an `[EventSubscriber(...)]` attribute. If yes,
// return publisher metadata (looked up in the dependency_objects index).
// Returns null otherwise. Used by the wrapper to enrich hover for the
// agent's most common discovery path: hover on `'OnAfterPost...'` literal.

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventReferenceAtPositionParams {
    pub uri: String,
    pub position: lsp_types::Position,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventReferenceMatch {
    pub publisher_object_type: String,
    pub publisher_object: String,
    pub event_name: String,
    /// Pre-formatted full signature of the publisher (or null if the
    /// publisher couldn't be resolved in the dependency index — e.g. it's
    /// in a local file the indexer hasn't reached).
    pub signature: Option<String>,
    pub attribute_kind: Option<String>,
    pub app_name: Option<String>,
    pub app_version: Option<String>,
}

fn event_reference_at_position(
    indexer: &Arc<RwLock<Indexer>>,
    params: EventReferenceAtPositionParams,
) -> Result<Option<EventReferenceMatch>> {
    let uri = match params.uri.parse::<lsp_types::Uri>() {
        Ok(u) => u,
        Err(e) => {
            debug!(
                "eventReferenceAtPosition: invalid uri {}: {}",
                params.uri, e
            );
            return Ok(None);
        }
    };
    let Some(path) = uri_to_path(&uri) else {
        return Ok(None);
    };
    let source = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return Ok(None),
    };

    let Some(parsed_ref) = find_event_subscriber_arg_at(&source, params.position) else {
        return Ok(None);
    };

    let indexer = indexer.read().expect("Indexer lock poisoned");
    let graph = indexer.graph();

    // Resolve publisher_object_type → ObjectType (best effort)
    let otype: Option<ObjectType> =
        ObjectType::try_from(parsed_ref.publisher_object_type.as_str()).ok();

    // Find a dependency object matching (type, name); search across apps.
    let dep = otype
        .and_then(|t| graph.find_dependency_object_by_type_name(t, &parsed_ref.publisher_object));

    let (signature, kind_tag, app_name, app_version) = match dep {
        Some(obj) => {
            let m = obj
                .methods
                .iter()
                .find(|m| m.name.eq_ignore_ascii_case(&parsed_ref.event_name));
            match m {
                Some(m) => {
                    let tag = m.kind.tag();
                    let tag_opt = if tag.is_empty() {
                        None
                    } else {
                        Some(tag.to_string())
                    };
                    (
                        Some(m.signature.clone()),
                        tag_opt,
                        Some(obj.app_name.clone()),
                        Some(obj.app_version.clone()),
                    )
                }
                None => (
                    None,
                    None,
                    Some(obj.app_name.clone()),
                    Some(obj.app_version.clone()),
                ),
            }
        }
        None => (None, None, None, None),
    };

    Ok(Some(EventReferenceMatch {
        publisher_object_type: parsed_ref.publisher_object_type,
        publisher_object: parsed_ref.publisher_object,
        event_name: parsed_ref.event_name,
        signature,
        attribute_kind: kind_tag,
        app_name,
        app_version,
    }))
}

/// Result of identifying that a cursor position sits on the event-name
/// argument of an `[EventSubscriber(...)]` attribute.
#[derive(Debug)]
struct EventSubscriberRef {
    publisher_object_type: String,
    publisher_object: String,
    event_name: String,
}

/// Pure-text scan that figures out whether a (line, char) sits on the
/// event-name string literal of an EventSubscriber attribute.
///
/// Algorithm:
///   1. Locate the line at `pos.line` and verify the cursor offset.
///   2. Determine whether the cursor is inside a `'...'` string literal.
///   3. Search backward (up to 4 lines, to handle multi-line attributes)
///      for `[EventSubscriber(`.
///   4. If found, parse the attribute arguments to extract the three known
///      pieces (object type, object name, event name).
///
/// This deliberately avoids a tree-sitter pass: hover requests happen on a
/// hot path and re-parsing each time is overkill when the attribute syntax
/// is so regular. Falls back to None on any malformed input.
fn find_event_subscriber_arg_at(
    source: &str,
    pos: lsp_types::Position,
) -> Option<EventSubscriberRef> {
    let lines: Vec<&str> = source.lines().collect();
    let line_idx = pos.line as usize;
    if line_idx >= lines.len() {
        return None;
    }
    let line = lines[line_idx];
    let col = pos.character as usize;
    if col > line.len() {
        return None;
    }

    // Build a window around the cursor large enough to contain the full
    // `[EventSubscriber(...)]` attribute. We scan backward for the `[`
    // opener and forward for the `)]` closer, both bounded by 32 lines /
    // 8 KB to keep this cheap.
    //
    // Multi-line attribute arguments (one-per-line formatting) are common
    // for verbose EventSubscriber attributes; the previous 4-line backward-
    // only window missed them.
    const MAX_SCAN_LINES: usize = 32;
    const MAX_SCAN_BYTES: usize = 8 * 1024;

    // Backward scan: find the line containing `[`.
    let mut start_back = line_idx;
    let mut bytes_seen = 0usize;
    while start_back > 0 && line_idx - start_back < MAX_SCAN_LINES {
        bytes_seen += lines[start_back].len();
        if bytes_seen > MAX_SCAN_BYTES {
            break;
        }
        if lines[start_back].contains('[') {
            break;
        }
        start_back -= 1;
    }

    // Forward scan: extend until we see a line containing `)]` (close of
    // the attribute). Stops at the same safety bounds.
    let mut end_forward = line_idx;
    let mut forward_bytes = 0usize;
    while end_forward + 1 < lines.len() && end_forward - line_idx < MAX_SCAN_LINES {
        if lines[end_forward].contains(')') && lines[end_forward].contains(']') {
            break;
        }
        end_forward += 1;
        forward_bytes += lines[end_forward].len();
        if forward_bytes > MAX_SCAN_BYTES {
            break;
        }
    }

    let combined = lines[start_back..=end_forward].join("\n");
    let lower = combined.to_lowercase();
    let attr_idx = lower.rfind("[eventsubscriber(")?;
    // Compute the cursor offset within `combined`.
    let mut cursor_offset = 0usize;
    for l in lines.iter().take(line_idx).skip(start_back) {
        cursor_offset += l.len() + 1; // +1 for '\n'
    }
    cursor_offset += col;

    // The attribute opens after `[EventSubscriber(` — locate the matching `)`
    // ignoring parens inside string literals.
    let after_open = attr_idx + "[EventSubscriber(".len();
    let close_idx = find_matching_close(&combined, after_open)?;

    if cursor_offset < after_open || cursor_offset > close_idx {
        return None;
    }

    let args = &combined[after_open..close_idx];
    parse_event_subscriber_args(args)
}

/// Find the position of the matching `)` for `(` at `start - 1`.
/// Honors AL's string-literal and comment syntax so things like
/// `[EventSubscriber(..., /* note */ ..., 'Event(name)', ...)]` parse
/// correctly. Specifically:
///   - skips `// ...\n` line comments
///   - skips `/* ... */` block comments
///   - tracks single- and double-quoted strings, treating doubled quotes
///     (`''` inside `'...'`, `""` inside `"..."`) as embedded literals
fn find_matching_close(s: &str, start: usize) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut depth = 1usize;
    let mut i = start;
    let mut quote: Option<u8> = None;
    while i < bytes.len() {
        let b = bytes[i];
        match quote {
            Some(q) => {
                // Inside a string literal. AL escapes the surrounding quote
                // by doubling it; skip the pair instead of closing.
                if b == q {
                    if i + 1 < bytes.len() && bytes[i + 1] == q {
                        i += 2;
                        continue;
                    }
                    quote = None;
                }
            }
            None => {
                // Top-level. Handle comments before regular tokens.
                if b == b'/' && i + 1 < bytes.len() {
                    match bytes[i + 1] {
                        b'/' => {
                            // line comment runs to next \n or end
                            i += 2;
                            while i < bytes.len() && bytes[i] != b'\n' {
                                i += 1;
                            }
                            continue;
                        }
                        b'*' => {
                            // block comment runs to */ or end
                            i += 2;
                            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/')
                            {
                                i += 1;
                            }
                            if i + 1 < bytes.len() {
                                i += 2; // consume */
                            } else {
                                i = bytes.len();
                            }
                            continue;
                        }
                        _ => {}
                    }
                }
                match b {
                    b'\'' | b'"' => quote = Some(b),
                    b'(' => depth += 1,
                    b')' => {
                        depth -= 1;
                        if depth == 0 {
                            return Some(i);
                        }
                    }
                    _ => {}
                }
            }
        }
        i += 1;
    }
    None
}

/// Parse the three known arguments out of an `[EventSubscriber(...)]` call.
/// Format: `ObjectType::X, ObjectType::"Name", 'EventName', '', false, false`.
/// We capture the object-type qualifier, the object name, and the event name.
///
/// Comments preceding each argument (`/* note */ 'OnFoo'`) are stripped
/// before parsing so split_top_level_commas's comment-aware tokenization
/// still yields clean values.
///
/// Returns None for malformed input.
fn parse_event_subscriber_args(args: &str) -> Option<EventSubscriberRef> {
    let parts = split_top_level_commas(args);
    if parts.len() < 3 {
        return None;
    }

    let p0 = strip_al_comments(parts[0]);
    let p1 = strip_al_comments(parts[1]);
    let p2 = strip_al_comments(parts[2]);

    // Parts[0]: `ObjectType::Codeunit` or `ObjectType::Database`
    let object_type = match p0.split("::").nth(1) {
        Some(t) => t.trim().to_string(),
        None => return None,
    };
    // The object-type word "Database" means a table reference — normalize.
    let object_type_norm = if object_type.eq_ignore_ascii_case("Database") {
        "Table".to_string()
    } else {
        object_type
    };

    // Parts[1]: `Codeunit::"Approvals Mgmt."` (the qualifier prefix is redundant
    // with parts[0] — strip it).
    let raw_obj = p1.trim();
    let object_name = raw_obj
        .split("::")
        .last()
        .unwrap_or(raw_obj)
        .trim()
        .trim_matches('"')
        .to_string();

    // Parts[2]: `'OnAfterPostApprovalEntries'` (single-quoted, possibly empty).
    let event_name = p2.trim().trim_matches('\'').to_string();
    if event_name.is_empty() {
        return None;
    }

    Some(EventSubscriberRef {
        publisher_object_type: object_type_norm,
        publisher_object: object_name,
        event_name,
    })
}

/// Remove top-level `// ...` and `/* ... */` comments from a string,
/// preserving string-literal contents. Used to clean each EventSubscriber
/// argument before quote-trimming.
fn strip_al_comments(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(bytes.len());
    let mut i = 0usize;
    let mut quote: Option<u8> = None;
    while i < bytes.len() {
        let b = bytes[i];
        match quote {
            Some(q) => {
                out.push(b as char);
                if b == q {
                    if i + 1 < bytes.len() && bytes[i + 1] == q {
                        out.push(bytes[i + 1] as char);
                        i += 2;
                        continue;
                    }
                    quote = None;
                }
                i += 1;
            }
            None => {
                if b == b'/' && i + 1 < bytes.len() {
                    match bytes[i + 1] {
                        b'/' => {
                            i += 2;
                            while i < bytes.len() && bytes[i] != b'\n' {
                                i += 1;
                            }
                            continue;
                        }
                        b'*' => {
                            i += 2;
                            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/')
                            {
                                i += 1;
                            }
                            if i + 1 < bytes.len() {
                                i += 2;
                            } else {
                                i = bytes.len();
                            }
                            continue;
                        }
                        _ => {}
                    }
                }
                if b == b'\'' || b == b'"' {
                    quote = Some(b);
                }
                out.push(b as char);
                i += 1;
            }
        }
    }
    out
}

/// Split `s` on top-level commas (commas not inside parens, brackets,
/// quotes, or comments). Mirrors find_matching_close's syntax model so
/// EventSubscriber arguments with comments and doubled-quote escapes
/// split correctly.
fn split_top_level_commas(s: &str) -> Vec<&str> {
    let bytes = s.as_bytes();
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut depth = 0i32;
    let mut quote: Option<u8> = None;
    let mut i = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        match quote {
            Some(q) => {
                if b == q {
                    if i + 1 < bytes.len() && bytes[i + 1] == q {
                        i += 2;
                        continue;
                    }
                    quote = None;
                }
            }
            None => {
                if b == b'/' && i + 1 < bytes.len() {
                    match bytes[i + 1] {
                        b'/' => {
                            i += 2;
                            while i < bytes.len() && bytes[i] != b'\n' {
                                i += 1;
                            }
                            continue;
                        }
                        b'*' => {
                            i += 2;
                            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/')
                            {
                                i += 1;
                            }
                            if i + 1 < bytes.len() {
                                i += 2;
                            } else {
                                i = bytes.len();
                            }
                            continue;
                        }
                        _ => {}
                    }
                }
                match b {
                    b'\'' | b'"' => quote = Some(b),
                    b'(' | b'[' => depth += 1,
                    b')' | b']' => depth -= 1,
                    b',' if depth == 0 => {
                        out.push(&s[start..i]);
                        start = i + 1;
                    }
                    _ => {}
                }
            }
        }
        i += 1;
    }
    out.push(&s[start..]);
    out
}

#[cfg(test)]
mod event_ref_tests {
    use super::*;

    const SUBSCRIBER_SRC: &str = r#"codeunit 50100 "Test Subs"
{
    [EventSubscriber(ObjectType::Codeunit, Codeunit::"Approvals Mgmt.", 'OnAfterPostApprovalEntries', '', true, false)]
    local procedure DoStuff()
    begin
    end;
}
"#;

    #[test]
    fn finds_event_name_on_cursor() {
        // Cursor in the middle of 'OnAfterPostApprovalEntries'.
        // The literal starts at col ~80; aim for col 95.
        let pos = lsp_types::Position {
            line: 2,
            character: 95,
        };
        let r = find_event_subscriber_arg_at(SUBSCRIBER_SRC, pos).expect("hit");
        assert_eq!(r.publisher_object_type, "Codeunit");
        assert_eq!(r.publisher_object, "Approvals Mgmt.");
        assert_eq!(r.event_name, "OnAfterPostApprovalEntries");
    }

    #[test]
    fn returns_none_outside_attribute() {
        let pos = lsp_types::Position {
            line: 4,
            character: 4,
        };
        assert!(find_event_subscriber_arg_at(SUBSCRIBER_SRC, pos).is_none());
    }

    #[test]
    fn handles_database_qualifier() {
        let src = r#"codeunit 1 X
{
    [EventSubscriber(ObjectType::Table, Database::"Sales Header", 'OnAfterValidate', 'Customer No.', true, false)]
    local procedure X() begin end;
}
"#;
        let pos = lsp_types::Position {
            line: 2,
            character: 80,
        };
        let r = find_event_subscriber_arg_at(src, pos).expect("hit");
        assert_eq!(r.publisher_object_type, "Table");
        assert_eq!(r.publisher_object, "Sales Header");
        assert_eq!(r.event_name, "OnAfterValidate");
    }

    /// Multi-line EventSubscriber with arguments wrapped one-per-line —
    /// previously failed because the lookback was capped at 4 lines.
    #[test]
    fn handles_multiline_attribute() {
        let src = r#"codeunit 1 X
{
    [EventSubscriber(
        ObjectType::Codeunit,
        Codeunit::"Approvals Mgmt.",
        'OnAfterPostApprovalEntries',
        '',
        true,
        false)]
    local procedure X() begin end;
}
"#;
        // Cursor on the event name (line 5, 0-based; the literal starts ~col 9).
        let pos = lsp_types::Position {
            line: 5,
            character: 20,
        };
        let r = find_event_subscriber_arg_at(src, pos).expect("hit");
        assert_eq!(r.event_name, "OnAfterPostApprovalEntries");
        assert_eq!(r.publisher_object, "Approvals Mgmt.");
    }

    /// Block comment inside the attribute argument list — previously this
    /// confused the comma-split state machine.
    #[test]
    fn handles_block_comment_in_attribute() {
        let src = r#"codeunit 1 X
{
    [EventSubscriber(ObjectType::Codeunit, /* TODO: rename */ Codeunit::"Approvals Mgmt.", 'OnAfterPost', '', false, false)]
    local procedure X() begin end;
}
"#;
        let pos = lsp_types::Position {
            line: 2,
            character: 102,
        }; // mid 'OnAfterPost'
        let r = find_event_subscriber_arg_at(src, pos).expect("hit");
        assert_eq!(r.publisher_object_type, "Codeunit");
        assert_eq!(r.publisher_object, "Approvals Mgmt.");
        assert_eq!(r.event_name, "OnAfterPost");
    }

    /// Block comment containing `)` — must not be interpreted as the closing
    /// paren of the attribute.
    #[test]
    fn block_comment_paren_does_not_close_attribute() {
        let src = r#"codeunit 1 X
{
    [EventSubscriber(ObjectType::Codeunit /* (early close attempt) */, Codeunit::"A", 'OnFoo', '', false, false)]
    local procedure X() begin end;
}
"#;
        // Cursor anywhere inside attribute should still find the event.
        let pos = lsp_types::Position {
            line: 2,
            character: 95,
        };
        let r = find_event_subscriber_arg_at(src, pos).expect("hit");
        assert_eq!(r.event_name, "OnFoo");
    }

    /// Line comment AFTER the attribute on the same line shouldn't break parsing.
    /// (The attribute syntax in AL ends with `)]`; trailing `//` is comment.)
    #[test]
    fn handles_trailing_line_comment() {
        let src = r#"codeunit 1 X
{
    [EventSubscriber(ObjectType::Codeunit, Codeunit::"A", 'OnFoo', '', false, false)]  // legacy
    local procedure X() begin end;
}
"#;
        let pos = lsp_types::Position {
            line: 2,
            character: 65,
        };
        let r = find_event_subscriber_arg_at(src, pos).expect("hit");
        assert_eq!(r.event_name, "OnFoo");
    }
}
