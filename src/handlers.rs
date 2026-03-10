//! LSP request and notification handlers

use anyhow::{Context, Result};
use log::debug;
use lsp_server::Request;
use lsp_types::{
    CallHierarchyIncomingCall, CallHierarchyIncomingCallsParams, CallHierarchyItem,
    CallHierarchyOutgoingCall, CallHierarchyOutgoingCallsParams, CallHierarchyPrepareParams,
    CodeLens, CodeLensParams, Command, SymbolKind,
};
use serde::{Deserialize, Serialize};
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

// --- Symbol Properties (generic for fields, actions, and any AL declaration) ---

/// Parameters for al-call-hierarchy/fieldProperties and al-call-hierarchy/actionProperties
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SymbolPropertiesParams {
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
struct SymbolPropertiesResult {
    /// For fields: the field ID number
    #[serde(skip_serializing_if = "Option::is_none")]
    field_id: Option<u32>,
    /// All declared properties from source (key = property name, value = property value)
    properties: Vec<PropertyEntry>,
}

/// A single property entry preserving declaration order
#[derive(Debug, Serialize)]
struct PropertyEntry {
    name: String,
    value: String,
}

/// Parse a file with tree-sitter and extract all properties for a field
fn field_properties(params: SymbolPropertiesParams) -> Result<SymbolPropertiesResult> {
    let (tree, source) = parse_file_from_uri(&params.uri)?;
    let target = params.field_name.trim().trim_matches('"').to_lowercase();
    let mut cursor = tree.root_node().walk();
    Ok(find_node_properties(&mut cursor, &source, "field_declaration", &target, true)
        .unwrap_or_default())
}

/// Parse a file with tree-sitter and extract all properties for an action
fn action_properties(params: SymbolPropertiesParams) -> Result<SymbolPropertiesResult> {
    let (tree, source) = parse_file_from_uri(&params.uri)?;
    let target = params.action_name.trim().trim_matches('"').to_lowercase();
    let mut cursor = tree.root_node().walk();
    Ok(find_node_properties(&mut cursor, &source, "action_declaration", &target, false)
        .unwrap_or_default())
}

/// Parse an AL file from a URI and return the tree + source
fn parse_file_from_uri(uri_str: &str) -> Result<(tree_sitter::Tree, String)> {
    use crate::language;
    use tree_sitter::Parser;

    let uri: lsp_types::Uri = uri_str.parse().context("Invalid URI")?;
    let path = uri_to_path(&uri).ok_or_else(|| anyhow::anyhow!("Invalid file URI"))?;

    let source = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read {}", path.display()))?;

    let lang = language::language();
    let mut parser = Parser::new();
    parser.set_language(&lang).context("Failed to set language")?;

    let tree = parser.parse(&source, None).context("Failed to parse file")?;

    Ok((tree, source))
}

/// Recursively search for a declaration node matching the target name,
/// then extract all properties from it.
fn find_node_properties(
    cursor: &mut tree_sitter::TreeCursor,
    source: &str,
    node_kind: &str,
    target_name: &str,
    extract_field_id: bool,
) -> Option<SymbolPropertiesResult> {
    loop {
        let node = cursor.node();

        if node.kind() == node_kind {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = &source[name_node.byte_range()];
                let clean = name.trim().trim_matches('"').to_lowercase();
                if clean == target_name {
                    return Some(extract_all_properties(&node, source, extract_field_id));
                }
            }
        }

        if cursor.goto_first_child() {
            if let Some(result) = find_node_properties(cursor, source, node_kind, target_name, extract_field_id) {
                return Some(result);
            }
            cursor.goto_parent();
        }

        if !cursor.goto_next_sibling() {
            return None;
        }
    }
}

/// Extract ALL properties from a declaration node.
/// Any child node whose kind ends with "_property" is collected.
fn extract_all_properties(
    decl_node: &tree_sitter::Node,
    source: &str,
    extract_field_id: bool,
) -> SymbolPropertiesResult {
    let mut result = SymbolPropertiesResult::default();

    // Extract field ID if requested (for field_declaration nodes)
    if extract_field_id {
        if let Some(id_node) = decl_node.child_by_field_name("id") {
            if let Ok(id) = source[id_node.byte_range()].trim().parse::<u32>() {
                result.field_id = Some(id);
            }
        }
    }

    let mut cursor = decl_node.walk();
    if !cursor.goto_first_child() {
        return result;
    }

    loop {
        let child = cursor.node();
        let kind = child.kind();

        // Collect any node whose kind ends with "_property"
        if kind.ends_with("_property") {
            let prop_name = property_display_name(kind);
            let prop_value = extract_property_value(&child, source);
            result.properties.push(PropertyEntry {
                name: prop_name,
                value: prop_value,
            });
        }

        if !cursor.goto_next_sibling() {
            break;
        }
    }

    result
}

/// Convert a tree-sitter node kind like "calc_formula_property" to a display name like "CalcFormula"
fn property_display_name(kind: &str) -> String {
    // Strip the "_property" suffix
    let base = kind.strip_suffix("_property").unwrap_or(kind);
    // Convert snake_case to PascalCase
    base.split('_')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                None => String::new(),
                Some(c) => {
                    let upper: String = c.to_uppercase().collect();
                    upper + chars.as_str()
                }
            }
        })
        .collect()
}

/// Extract the value portion of a property node (everything after the '=')
fn extract_property_value(node: &tree_sitter::Node, source: &str) -> String {
    let text = source[node.byte_range()].trim();
    // Properties follow the pattern: PropertyName = value;
    // Extract everything after '=' and before the trailing ';'
    if let Some(eq_pos) = text.find('=') {
        let value = text[eq_pos + 1..].trim();
        // Remove trailing semicolon
        let value = value.strip_suffix(';').unwrap_or(value).trim();
        value.to_string()
    } else {
        text.to_string()
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_field_properties_extraction() {
        use crate::language;
        use tree_sitter::Parser;

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

        let lang = language::language();
        let mut parser = Parser::new();
        parser.set_language(&lang).unwrap();
        let tree = parser.parse(source, None).unwrap();
        let root = tree.root_node();

        // Helper to find a property by name in the result
        fn prop(result: &SymbolPropertiesResult, name: &str) -> Option<String> {
            result.properties.iter().find(|p| p.name == name).map(|p| p.value.clone())
        }

        // Test Balance field (FlowField with CalcFormula)
        let mut cursor = root.walk();
        let result = find_node_properties(&mut cursor, source, "field_declaration", "balance", true).unwrap();
        assert_eq!(result.field_id, Some(11));
        assert_eq!(prop(&result, "Caption").as_deref(), Some("'Balance'"));
        assert_eq!(prop(&result, "Editable").as_deref(), Some("false"));
        assert_eq!(prop(&result, "FieldClass").as_deref(), Some("FlowField"));
        assert!(prop(&result, "CalcFormula").is_some());
        assert!(prop(&result, "CalcFormula").unwrap().contains("Cust. Ledger Entry"));

        // Test Payment Terms Code field (with TableRelation)
        let mut cursor = root.walk();
        let result = find_node_properties(&mut cursor, source, "field_declaration", "payment terms code", true).unwrap();
        assert_eq!(result.field_id, Some(20));
        assert!(prop(&result, "TableRelation").is_some());
        assert!(prop(&result, "TableRelation").unwrap().contains("Payment Terms"));

        // Test No. field (basic field)
        let mut cursor = root.walk();
        let result = find_node_properties(&mut cursor, source, "field_declaration", "no.", true).unwrap();
        assert_eq!(result.field_id, Some(1));
        assert_eq!(prop(&result, "Caption").as_deref(), Some("'No.'"));
        assert_eq!(prop(&result, "DataClassification").as_deref(), Some("CustomerContent"));
        assert!(prop(&result, "FieldClass").is_none());
        assert!(prop(&result, "CalcFormula").is_none());
    }

    #[test]
    fn test_action_properties_extraction() {
        use crate::language;
        use tree_sitter::Parser;

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

        let lang = language::language();
        let mut parser = Parser::new();
        parser.set_language(&lang).unwrap();
        let tree = parser.parse(source, None).unwrap();
        let root = tree.root_node();

        // Helper to find a property by name
        fn prop(result: &SymbolPropertiesResult, name: &str) -> Option<String> {
            result.properties.iter().find(|p| p.name == name).map(|p| p.value.clone())
        }

        // Test LedgerEntries action (with RunObject)
        let mut cursor = root.walk();
        let result = find_node_properties(&mut cursor, source, "action_declaration", "ledgerentries", false).unwrap();
        assert_eq!(prop(&result, "Caption").as_deref(), Some("'Ledger E&ntries'"));
        assert_eq!(prop(&result, "Image").as_deref(), Some("CustomerLedger"));
        assert!(prop(&result, "RunObject").is_some());
        assert!(prop(&result, "RunObject").unwrap().contains("Customer Ledger Entries"));
        assert!(prop(&result, "RunPageLink").is_some());
        assert!(prop(&result, "RunPageView").is_some());
        assert_eq!(prop(&result, "ShortcutKey").as_deref(), Some("'Ctrl+F7'"));
        assert!(prop(&result, "ToolTip").is_some());
        assert!(prop(&result, "ToolTip").unwrap().contains("history of transactions"));

        // Test CheckCreditLimit action (no RunObject, has trigger)
        let mut cursor = root.walk();
        let result = find_node_properties(&mut cursor, source, "action_declaration", "checkcreditlimit", false).unwrap();
        assert_eq!(prop(&result, "Caption").as_deref(), Some("'Check Credit Limit'"));
        assert_eq!(prop(&result, "Image").as_deref(), Some("Check"));
        assert!(prop(&result, "RunObject").is_none());
        assert!(prop(&result, "ToolTip").is_some());
    }
}
