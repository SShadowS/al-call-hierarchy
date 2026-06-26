//! Expression / callee / receiver classification — ports of
//! `expression-from-node.ts`, `callee-from-node.ts`, `receiver-classification.ts`,
//! `object-run-result-consumed.ts`, `object-run-return-used.ts`.

use super::features::{PCallee, PExpressionInfo};
use super::node_util::{child_of_kind, named_children, node_text, strip_quote_chars};
use std::collections::HashMap;
use tree_sitter::Node;

/// Map a tree-sitter node type into the closed `ExpressionKind` enum.
fn kind_of(node_type: &str) -> &'static str {
    match node_type {
        "string_literal" => "string_literal",
        "integer" => "integer",
        "decimal" => "decimal",
        "boolean" => "boolean",
        "identifier" => "identifier",
        "quoted_identifier" => "quoted_identifier",
        "qualified_enum_value" => "qualified_enum_value",
        "database_reference" => "database_reference",
        "unary_expression" => "unary_expression",
        "member_expression" => "member_expression",
        "call_expression" => "call_expression",
        "parenthesized_expression" => "parenthesized_expression",
        _ => "other",
    }
}

/// For `unary_expression`, return the operand's value (the signed text) when the
/// FIRST named child is a numeric literal (`integer` / `decimal`).
fn unary_literal_value(node: Node, text: &str) -> Option<String> {
    // Mirror the TS loop: inspect ONLY the FIRST named child (it returns on the
    // first iteration regardless of match), so a `+5` / `-5.5` over a numeric
    // literal yields the signed text; anything else yields None.
    let first = named_children(node).into_iter().next()?;
    if first.kind() == "integer" || first.kind() == "decimal" {
        Some(text.to_string())
    } else {
        None
    }
}

/// Build an `ExpressionInfo` from an arbitrary expression node.
pub fn expression_info_from_node(node: Node, source: &str) -> PExpressionInfo {
    let kind = kind_of(node.kind());
    let text = node_text(node, source).to_string();
    let mut value: Option<String> = None;
    let mut qualifier: Option<String> = None;
    let mut member: Option<String> = None;

    match kind {
        "string_literal" | "quoted_identifier" => {
            value = Some(strip_quote_chars(&text).to_string());
        }
        "integer" | "decimal" | "boolean" | "identifier" => {
            value = Some(text.clone());
        }
        "qualified_enum_value" => {
            qualifier = node
                .child_by_field_name("enum_type")
                .map(|n| node_text(n, source).to_string());
            let member_raw = node
                .child_by_field_name("value")
                .map(|n| node_text(n, source).to_string());
            member = member_raw.map(|m| strip_quote_chars(&m).to_string());
            value = member.clone();
        }
        "database_reference" => {
            qualifier = node
                .child_by_field_name("keyword")
                .map(|n| node_text(n, source).to_string());
            let member_raw = node
                .child_by_field_name("table_name")
                .map(|n| node_text(n, source).to_string());
            member = member_raw.map(|m| strip_quote_chars(&m).to_string());
            value = member.clone();
        }
        "unary_expression" => {
            value = unary_literal_value(node, &text);
        }
        _ => {}
    }

    PExpressionInfo {
        kind: kind.to_string(),
        text,
        value,
        qualifier,
        member,
    }
}

/// Object-run object-kind for a keyword_identifier receiver.
fn object_run_kind_of_receiver(obj_node: Node) -> Option<&'static str> {
    if obj_node.kind() != "keyword_identifier" {
        return None;
    }
    for child in named_children(obj_node) {
        match child.kind() {
            "codeunit_keyword" => return Some("Codeunit"),
            "page_keyword" => return Some("Page"),
            "report_keyword" => return Some("Report"),
            _ => {}
        }
    }
    None
}

/// First positional argument node of a call_expression's argument_list.
fn first_argument_node<'a>(call_node: Node<'a>) -> Option<Node<'a>> {
    let arg_list = child_of_kind(call_node, "argument_list")?;
    named_children(arg_list).into_iter().next()
}

struct ObjectRunParts {
    target_type: String,
    target_ref: Option<String>,
    target_is_name: bool,
}

fn classify_object_run_first_arg(
    first_arg: Option<Node>,
    object_kind: &str,
    source: &str,
) -> ObjectRunParts {
    let none = ObjectRunParts {
        target_type: object_kind.to_string(),
        target_ref: None,
        target_is_name: false,
    };
    let Some(first_arg) = first_arg else {
        return none;
    };
    if first_arg.kind() != "database_reference" {
        return none;
    }
    let Some(table_name_node) = first_arg.child_by_field_name("table_name") else {
        return none;
    };
    let text = node_text(table_name_node, source);
    match table_name_node.kind() {
        "integer" => ObjectRunParts {
            target_type: object_kind.to_string(),
            target_ref: Some(text.to_string()),
            target_is_name: false,
        },
        "quoted_identifier" => ObjectRunParts {
            target_type: object_kind.to_string(),
            target_ref: Some(strip_quote_chars(text).to_string()),
            target_is_name: true,
        },
        _ => ObjectRunParts {
            target_type: object_kind.to_string(),
            target_ref: Some(text.to_string()),
            target_is_name: true,
        },
    }
}

/// Build the member-shaped Callee for a member_expression, with object-run upgrade.
fn callee_from_member_expression(
    member_expr: Node,
    call_node: Option<Node>,
    source: &str,
) -> PCallee {
    let obj_node = member_expr
        .child_by_field_name("object")
        .or_else(|| member_expr.named_child(0));
    let member_node = member_expr
        .child_by_field_name("member")
        .or_else(|| member_expr.named_child(1));
    let (Some(obj_node), Some(member_node)) = (obj_node, member_node) else {
        return PCallee::Unknown;
    };
    let member_lc = node_text(member_node, source).to_lowercase();
    if let Some(object_kind) = object_run_kind_of_receiver(obj_node) {
        if member_lc == "run" {
            let first_arg = call_node.and_then(first_argument_node);
            let parts = classify_object_run_first_arg(first_arg, object_kind, source);
            return PCallee::ObjectRun {
                object_kind: object_kind.to_string(),
                target_type: parts.target_type,
                target_ref: parts.target_ref,
                target_is_name: parts.target_is_name,
            };
        }
    }
    PCallee::Member {
        receiver: node_text(obj_node, source).to_string(),
        method: strip_quote_chars(node_text(member_node, source)).to_string(),
    }
}

/// Classify a call_expression OR statement-position member_expression into a Callee.
pub fn callee_from_node(node: Node, source: &str) -> PCallee {
    if node.kind() == "member_expression" {
        return callee_from_member_expression(node, None, source);
    }
    if node.kind() != "call_expression" {
        return PCallee::Unknown;
    }
    let func_node = node
        .child_by_field_name("function")
        .or_else(|| node.named_child(0));
    let Some(func_node) = func_node else {
        return PCallee::Unknown;
    };
    match func_node.kind() {
        "identifier" | "quoted_identifier" => PCallee::Bare {
            name: strip_quote_chars(node_text(func_node, source)).to_string(),
        },
        "member_expression" => callee_from_member_expression(func_node, Some(node), source),
        _ => PCallee::Unknown,
    }
}

// --- Receiver classification (receiver-classification.ts) ---

const CALLABLE_OBJECT_KEYWORDS: &[&str] = &[
    "codeunit",
    "page",
    "report",
    "query",
    "xmlport",
    "interface",
    "enum",
    "controladdin",
    "testpage",
];

/// Returns the lowercased, quote-stripped receiver name iff the receiver text is
/// a simple identifier; None for compound expressions.
pub fn simple_receiver_name(receiver_text: &str) -> Option<String> {
    if receiver_text.is_empty() {
        return None;
    }
    let trimmed = receiver_text.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.starts_with('"') {
        if !trimmed.ends_with('"') || trimmed.chars().count() < 2 {
            return None;
        }
        let inner = &trimmed[1..trimmed.len() - 1];
        if inner.contains('(') || inner.contains('[') {
            return None;
        }
        return Some(inner.to_lowercase());
    }
    if trimmed.contains('.')
        || trimmed.contains('(')
        || trimmed.contains('[')
        || trimmed.contains(' ')
        || trimmed.contains('\t')
    {
        return None;
    }
    Some(trimmed.to_lowercase())
}

#[derive(PartialEq, Eq)]
pub enum ReceiverClass {
    Record,
    CallableObject,
    Other,
    Unknown,
}

pub fn classify_receiver(
    receiver_text: &str,
    variable_types_by_name: &HashMap<String, String>,
) -> ReceiverClass {
    let Some(name_lc) = simple_receiver_name(receiver_text) else {
        return ReceiverClass::Unknown;
    };
    if name_lc == "rec" || name_lc == "xrec" {
        return ReceiverClass::Record;
    }
    let Some(declared_type) = variable_types_by_name.get(&name_lc) else {
        return ReceiverClass::Unknown;
    };
    let type_lc = declared_type.to_lowercase();
    if type_lc == "record" || type_lc.starts_with("record ") || type_lc == "recordref" {
        return ReceiverClass::Record;
    }
    let first_token = type_lc.split(' ').next().unwrap_or(&type_lc);
    if CALLABLE_OBJECT_KEYWORDS.contains(&first_token) {
        return ReceiverClass::CallableObject;
    }
    ReceiverClass::Other
}

pub fn is_record_receiver_text(
    receiver_text: &str,
    variable_types_by_name: &HashMap<String, String>,
) -> bool {
    classify_receiver(receiver_text, variable_types_by_name) == ReceiverClass::Record
}

// --- object-run result-consumed / return-used ---

/// Mirror `classifyObjectRunResultConsumed` (SUPPRESSION FLOOR → true).
pub fn classify_object_run_result_consumed(node: Node, parent: Option<Node>) -> bool {
    let Some(parent) = parent else {
        return true;
    };
    let pt = parent.kind();
    if pt == "parenthesized_expression" {
        return classify_object_run_result_consumed(parent, parent.parent());
    }
    if pt == "asserterror_statement" {
        return true;
    }
    // tree-sitter-al v3 nests a code_block's statements in a `statement_block`
    // (code_block.body), so a bare call statement's parent is the statement_block,
    // not the code_block. Mirror the code_block branch: a bare statement does not
    // consume the result. The asserterror nuance is one level higher now
    // (asserterror -> code_block -> statement_block).
    if pt == "statement_block" {
        let asserterror_wrapped = parent
            .parent()
            .and_then(|cb| cb.parent())
            .map(|p| p.kind())
            == Some("asserterror_statement");
        return asserterror_wrapped;
    }
    if pt == "code_block" && parent.parent().map(|p| p.kind()) == Some("asserterror_statement") {
        return true;
    }
    if pt == "code_block" {
        return false;
    }
    if pt == "repeat_statement" {
        let Some(cond) = parent.child_by_field_name("condition") else {
            return true;
        };
        return cond.start_byte() == node.start_byte();
    }
    for field in ["then_branch", "else_branch", "body"] {
        if let Some(fc) = parent.child_by_field_name(field) {
            if fc.start_byte() == node.start_byte() {
                return false;
            }
        }
    }
    true
}

/// Mirror `objectRunBooleanReturnUsed` (STRICT affirmative → false default).
pub fn object_run_boolean_return_used(node: Node, parent: Option<Node>) -> bool {
    let Some(parent) = parent else {
        return false;
    };
    let pt = parent.kind();
    if pt == "parenthesized_expression" {
        return object_run_boolean_return_used(parent, parent.parent());
    }
    if pt == "if_statement" || pt == "while_statement" || pt == "repeat_statement" {
        return parent
            .child_by_field_name("condition")
            .map(|c| c.start_byte() == node.start_byte())
            .unwrap_or(false);
    }
    if pt == "case_statement" {
        return parent
            .child_by_field_name("expression")
            .map(|e| e.start_byte() == node.start_byte())
            .unwrap_or(false);
    }
    if pt == "unary_expression" || pt == "binary_expression" || pt == "argument_list" {
        return true;
    }
    if pt == "assignment_statement" || pt == "exit_statement" {
        return true;
    }
    false
}
