//! Single-DFS body walker — Rust port of `intraprocedural-body.ts`.
//!
//! Reproduces al-sem's L2 enumeration EXACTLY: namedChildren document order,
//! `node.id`-keyed op/callsite maps (NOT start byte — nested call_expressions
//! share a start), two-phase op/callsite numbering (record-ops/commit/error get
//! op0..opN-1 during visit; callsites get op{N+i} POST-visit; callsiteId cs{i}
//! during visit), ExpressionInfo classification, L2 caller-side argument
//! bindings, result-use flags, underAsserterror, loop discovery + loopStack,
//! record ops, field accesses, record vars, scalar variables, varAssignments,
//! conditionReferences, identifierReferences (lc+dedup+sort), unreachable
//! statements, hasBranching, and the CFN skeleton (built post-visit in cfn.rs).
//!
//! The recordOperation tempState/recordVariableId backfill from recordVariables
//! (routine-indexer.ts) is applied in [`extract_body_features`] after the walk.

use super::classify::{
    callee_from_node, classify_object_run_result_consumed, classify_receiver,
    expression_info_from_node, is_record_receiver_text, object_run_boolean_return_used,
    simple_receiver_name, ReceiverClass,
};
use super::features::{
    PAnchor, PCallArgumentBinding, PCallSite, PCallee, PConditionReference, PExpressionInfo,
    PFieldAccess, PLoop, POperationSite, PRecordOperation, PTempState, PUnreachableStatement,
    PVarAssignment,
};
use super::node_util::{child_of_kind, named_children, node_text, Utf16Cols};
use super::record_op::{record_op_type, FIELD_ARGS_OPS};
use super::scope::{ParameterSymbol, RecordVariable};
use std::collections::{HashMap, HashSet};
use tree_sitter::Node;

/// An implicit-receiver frame (with-statement body / base-Rec seed).
#[derive(Clone)]
pub struct ImplicitReceiverFrame {
    pub text: String,
    /// "simple" | "unknown"
    pub kind: &'static str,
}

/// The result of the single body walk (flat feature lists; CFN built after).
pub struct ExtractBodyResult {
    pub loops: Vec<PLoop>,
    pub operation_sites: Vec<POperationSite>,
    pub record_operations: Vec<PRecordOperation>,
    pub call_sites: Vec<PCallSite>,
    pub field_accesses: Vec<PFieldAccess>,
    pub unreachable_statements: Vec<PUnreachableStatement>,
    pub has_branching: bool,
    pub identifier_references: Vec<String>,
    pub var_assignments: Vec<PVarAssignment>,
    pub condition_references: Vec<PConditionReference>,
    /// node.id → operationId, for the CFN builder (op leaves).
    pub op_id_by_node_id: HashMap<usize, String>,
    /// node.id → callsiteId, for the CFN builder (call/error leaves).
    pub cs_id_by_node_id: HashMap<usize, String>,
}

/// Walk context — immutable inputs + mutable emit state.
pub struct Ctx<'a> {
    pub source: &'a str,
    pub cols: &'a Utf16Cols<'a>,
    pub routine_id: &'a str,
    pub source_unit_id: &'a str,
    pub record_var_names: &'a HashSet<String>,
    pub enclosing_parameters: &'a [ParameterSymbol],
    pub enclosing_record_variables: &'a [RecordVariable],
    pub variable_types_by_name: &'a HashMap<String, String>,
    pub object_procedure_names: &'a HashSet<String>,

    // emit state
    pub loops: Vec<PLoop>,
    pub operation_sites: Vec<POperationSite>,
    pub record_operations: Vec<PRecordOperation>,
    pub call_sites: Vec<PCallSite>,
    pub field_accesses: Vec<PFieldAccess>,
    pub unreachable_statements: Vec<PUnreachableStatement>,
    pub identifier_ref_set: HashSet<String>,
    pub loop_stack: Vec<String>,
    pub implicit_receiver_stack: Vec<ImplicitReceiverFrame>,
    pub op_index: u32,
    pub cs_index: u32,
    pub unreachable_index: u32,
    pub has_branching: bool,
    pub op_id_by_node_id: HashMap<usize, String>,
    pub cs_id_by_node_id: HashMap<usize, String>,
}

impl<'a> Ctx<'a> {
    pub fn anchor(&self, node: Node) -> PAnchor {
        let sp = node.start_position();
        let ep = node.end_position();
        PAnchor {
            source_unit_id: self.source_unit_id.to_string(),
            start_line: sp.row as u32,
            start_column: self.cols.col(sp.row, sp.column),
            end_line: ep.row as u32,
            end_column: self.cols.col(ep.row, ep.column),
            syntax_kind: node.kind().to_string(),
        }
    }

    fn text(&self, node: Node) -> &'a str {
        node_text(node, self.source)
    }
}

/// Map of lc name → ParameterSymbol / RecordVariable for binding extraction.
fn extract_argument_bindings(ctx: &Ctx, arg_nodes: &[Node]) -> Vec<PCallArgumentBinding> {
    let rec_var_by_lc: HashMap<String, &RecordVariable> = ctx
        .enclosing_record_variables
        .iter()
        .map(|rv| (rv.name.to_lowercase(), rv))
        .collect();
    let param_by_lc: HashMap<String, &ParameterSymbol> = ctx
        .enclosing_parameters
        .iter()
        .map(|p| (p.name.to_lowercase(), p))
        .collect();

    arg_nodes
        .iter()
        .enumerate()
        .map(|(parameter_index, &arg_node)| {
            let parameter_index = parameter_index as u32;
            let argument_anchor = ctx.anchor(arg_node);
            // Only bare-identifier args bind to a record/parameter symbol.
            if arg_node.kind() != "identifier" {
                return PCallArgumentBinding {
                    parameter_index,
                    source_kind: "expression".to_string(),
                    source_variable_name: None,
                    source_record_variable_id: None,
                    source_parameter_index: None,
                    caller_source_parameter_is_var: None,
                    source_temp_state: None,
                    argument_anchor,
                };
            }
            let text = ctx.text(arg_node).trim().to_string();
            let lc_name = text.to_lowercase();
            let rec_var = rec_var_by_lc.get(&lc_name);
            let param = param_by_lc.get(&lc_name);
            let source_kind = if param.is_some() {
                "parameter"
            } else if rec_var.is_some() {
                "local"
            } else if lc_name == "rec" || lc_name == "xrec" {
                "implicit-rec"
            } else {
                "unknown"
            };
            let source_variable_name = if source_kind == "unknown" {
                None
            } else {
                Some(lc_name.clone())
            };
            PCallArgumentBinding {
                parameter_index,
                source_kind: source_kind.to_string(),
                source_variable_name,
                source_record_variable_id: rec_var.map(|rv| rv.id.clone()),
                source_parameter_index: param.map(|p| p.index),
                caller_source_parameter_is_var: param.map(|p| p.is_var),
                source_temp_state: rec_var.map(|rv| rv.temp_state.clone()),
                argument_anchor,
            }
        })
        .collect()
}

/// True when `node` has an `asserterror_statement` ancestor (bounded by the
/// procedure/trigger boundary).
fn is_under_asserterror(node: Node) -> bool {
    let mut cur = node.parent();
    while let Some(c) = cur {
        let t = c.kind();
        if t == "asserterror_statement" {
            return true;
        }
        if t == "procedure" || t == "trigger" {
            break;
        }
        cur = c.parent();
    }
    false
}

/// Active implicit receiver: innermost frame, else None.
fn resolve_implicit_receiver<'f>(ctx: &'f Ctx) -> Option<&'f ImplicitReceiverFrame> {
    ctx.implicit_receiver_stack.last()
}

/// Receiver-type-aware record-op gate (isRecordReceiver in the walker).
fn is_record_receiver(ctx: &Ctx, receiver_text: &str) -> bool {
    match classify_receiver(receiver_text, ctx.variable_types_by_name) {
        ReceiverClass::Record => true,
        ReceiverClass::CallableObject | ReceiverClass::Other => false,
        ReceiverClass::Unknown => ctx.record_var_names.contains(&receiver_text.to_lowercase()),
    }
}

/// Classify a statement node as an unconditional exit (exit/Error/CurrReport.Quit).
fn unconditional_exit_kind(node: Node, source: &str) -> Option<&'static str> {
    if node.kind() == "exit_statement" {
        return Some("exit");
    }
    if node.kind() != "call_expression" {
        return None;
    }
    let func_node = node
        .child_by_field_name("function")
        .or_else(|| node.named_child(0))?;
    if func_node.kind() == "identifier" && node_text(func_node, source).to_lowercase() == "error" {
        return Some("error");
    }
    if func_node.kind() == "member_expression" {
        let obj_node = func_node
            .child_by_field_name("object")
            .or_else(|| func_node.named_child(0));
        let member_node = func_node
            .child_by_field_name("member")
            .or_else(|| func_node.named_child(1));
        if let (Some(obj_node), Some(member_node)) = (obj_node, member_node) {
            if node_text(obj_node, source).to_lowercase() == "currreport"
                && node_text(member_node, source).to_lowercase() == "quit"
            {
                return Some("currreport-quit");
            }
        }
    }
    None
}

/// Pure-statement parents (a member_expression child here IS a statement).
fn is_pure_statement_parent(parent_type: &str) -> bool {
    parent_type == "code_block"
}

/// Statement-position field names per parent type.
fn statement_fields_by_parent(parent_type: &str) -> Option<&'static [&'static str]> {
    match parent_type {
        "if_statement" => Some(&["then_branch", "else_branch"]),
        "for_statement" => Some(&["body"]),
        "while_statement" => Some(&["body"]),
        "foreach_statement" => Some(&["body"]),
        "with_statement" => Some(&["body"]),
        "case_branch" => Some(&["body"]),
        _ => None,
    }
}

/// Collect identifier-uses from a subtree (callee subtree harvest).
fn collect_identifiers_from(ctx: &mut Ctx, root: Node) {
    let mut stack: Vec<(Node, Option<Node>)> = vec![(root, None)];
    while let Some((node, parent)) = stack.pop() {
        if node.kind() == "identifier" {
            if let Some(parent) = parent {
                let parent_type = parent.kind();
                let mut is_value_ref = true;
                if parent_type == "member_expression" {
                    if let Some(member_field) = parent.child_by_field_name("member") {
                        if member_field.start_byte() == node.start_byte() {
                            is_value_ref = false;
                        }
                    }
                } else if parent_type == "qualified_enum_value" {
                    let value_field = parent.child_by_field_name("value");
                    let enum_type_field = parent.child_by_field_name("enum_type");
                    if value_field.map(|v| v.start_byte()) == Some(node.start_byte()) {
                        is_value_ref = false;
                    } else if let Some(etf) = enum_type_field {
                        if etf.start_byte() == node.start_byte() && etf.kind() == "identifier" {
                            is_value_ref = false;
                        }
                    }
                }
                if is_value_ref {
                    ctx.identifier_ref_set
                        .insert(node_text(node, ctx.source).to_lowercase());
                }
            }
        }
        for child in named_children(node) {
            stack.push((child, Some(node)));
        }
    }
}

fn make_temp_state_unknown() -> PTempState {
    PTempState {
        kind: "unknown".to_string(),
        value: None,
        parameter_index: None,
    }
}

/// Push a record-op + paired operationSite (kind record-op or lock).
fn push_record_op(
    ctx: &mut Ctx,
    node: Node,
    op_type: &str,
    receiver: &str,
    field_arguments: Option<Vec<String>>,
    field_argument_infos: Option<Vec<PExpressionInfo>>,
) {
    let anchor = ctx.anchor(node);
    let op_id = format!("{}/op{}", ctx.routine_id, ctx.op_index);
    ctx.op_index += 1;
    ctx.op_id_by_node_id.insert(node.id(), op_id.clone());
    let snapshot_loop_stack = ctx.loop_stack.clone();
    ctx.record_operations.push(PRecordOperation {
        id: op_id.clone(),
        op: op_type.to_string(),
        record_variable_name: receiver.to_string(),
        record_variable_id: None,
        temp_state: make_temp_state_unknown(),
        field_arguments,
        field_argument_infos,
        loop_stack: snapshot_loop_stack.clone(),
        source_anchor: anchor.clone(),
    });
    ctx.operation_sites.push(POperationSite {
        id: op_id,
        kind: if op_type == "LockTable" {
            "lock".to_string()
        } else {
            "record-op".to_string()
        },
        loop_stack: snapshot_loop_stack,
        source_anchor: anchor,
        under_asserterror: None,
        control_context: None,
    });
}

/// Collect args (texts, infos, nodes) from a call's argument_list.
fn collect_args<'b>(
    ctx: &Ctx<'b>,
    node: Node<'b>,
) -> (Vec<String>, Vec<PExpressionInfo>, Vec<Node<'b>>) {
    let mut texts = Vec::new();
    let mut infos = Vec::new();
    let mut nodes = Vec::new();
    if let Some(arg_list) = child_of_kind(node, "argument_list") {
        for arg in named_children(arg_list) {
            texts.push(node_text(arg, ctx.source).to_string());
            infos.push(expression_info_from_node(arg, ctx.source));
            nodes.push(arg);
        }
    }
    (texts, infos, nodes)
}

fn handle_call_expression(ctx: &mut Ctx, node: Node, parent: Option<Node>) {
    let func_node = node
        .child_by_field_name("function")
        .or_else(|| node.named_child(0));
    let Some(func_node) = func_node else {
        return;
    };

    collect_identifiers_from(ctx, func_node);

    if func_node.kind() == "member_expression" {
        let member_node = func_node
            .child_by_field_name("member")
            .or_else(|| func_node.named_child(1));
        let Some(member_node) = member_node else {
            return;
        };
        let method_lc = ctx.text(member_node).to_lowercase();
        let op_type = record_op_type(&method_lc);
        let obj_node = func_node
            .child_by_field_name("object")
            .or_else(|| func_node.named_child(0));
        let receiver = obj_node
            .map(|n| ctx.text(n).to_string())
            .unwrap_or_default();

        if let Some(op_type) = op_type {
            if is_record_receiver(ctx, &receiver) {
                let mut field_arguments: Option<Vec<String>> = None;
                let mut field_argument_infos: Option<Vec<PExpressionInfo>> = None;
                if FIELD_ARGS_OPS.contains(&op_type) {
                    if let Some(arg_list) = child_of_kind(node, "argument_list") {
                        let mut args = Vec::new();
                        let mut infos = Vec::new();
                        for arg in named_children(arg_list) {
                            args.push(node_text(arg, ctx.source).to_string());
                            infos.push(expression_info_from_node(arg, ctx.source));
                        }
                        field_arguments = Some(args);
                        field_argument_infos = Some(infos);
                    }
                }
                push_record_op(
                    ctx,
                    node,
                    op_type,
                    &receiver,
                    field_arguments,
                    field_argument_infos,
                );
                recurse_into_args(ctx, node);
                chained_receiver_descent(ctx, node, func_node);
                return;
            }
        }
        // Member call NOT a record DB op → CallSite.
        let (argument_texts, argument_infos, arg_nodes) = collect_args(ctx, node);
        let cs_id = format!("{}/cs{}", ctx.routine_id, ctx.cs_index);
        ctx.cs_index += 1;
        ctx.cs_id_by_node_id.insert(node.id(), cs_id.clone());
        let callee = callee_from_node(node, ctx.source);
        let is_object_run = matches!(callee, PCallee::ObjectRun { .. });
        let result_consumed = if is_object_run {
            Some(classify_object_run_result_consumed(node, parent))
        } else {
            None
        };
        let object_run_return_used = if is_object_run {
            Some(object_run_boolean_return_used(node, parent))
        } else {
            None
        };
        let bindings = extract_argument_bindings(ctx, &arg_nodes);
        let under = if is_under_asserterror(node) {
            Some(true)
        } else {
            None
        };
        ctx.call_sites.push(PCallSite {
            id: cs_id,
            operation_id: String::new(),
            callee_text: ctx.text(func_node).to_string(),
            callee,
            argument_texts,
            argument_infos,
            argument_bindings: bindings,
            loop_stack: ctx.loop_stack.clone(),
            source_anchor: ctx.anchor(node),
            result_consumed,
            object_run_return_used,
            under_asserterror: under,
            control_context: None,
        });
        recurse_into_args(ctx, node);
        chained_receiver_descent(ctx, node, func_node);
        return;
    } else if func_node.kind() == "identifier" {
        let method_text = ctx.text(func_node).to_string();
        if method_text.to_lowercase() == "commit" {
            let op_id = format!("{}/op{}", ctx.routine_id, ctx.op_index);
            ctx.op_index += 1;
            ctx.op_id_by_node_id.insert(node.id(), op_id.clone());
            ctx.operation_sites.push(POperationSite {
                id: op_id,
                kind: "commit".to_string(),
                loop_stack: ctx.loop_stack.clone(),
                source_anchor: ctx.anchor(node),
                under_asserterror: None,
                control_context: None,
            });
        } else {
            let method_lc = method_text.to_lowercase();
            let record_op = record_op_type(&method_lc);
            let frame = if record_op.is_some() {
                resolve_implicit_receiver(ctx).cloned()
            } else {
                None
            };
            let frame_is_record = frame.as_ref().is_some_and(|f| {
                f.kind == "simple"
                    && is_record_receiver_text(&f.text, ctx.variable_types_by_name)
                    && !ctx.object_procedure_names.contains(&method_lc)
            });
            if let (Some(record_op), Some(frame)) = (record_op, frame.as_ref()) {
                if frame_is_record {
                    push_record_op(ctx, node, record_op, &frame.text, None, None);
                } else {
                    // Non-record/unknown/collision implicit receiver → member-or-unknown CallSite.
                    let (argument_texts, argument_infos, arg_nodes) = collect_args(ctx, node);
                    let cs_id = format!("{}/cs{}", ctx.routine_id, ctx.cs_index);
                    ctx.cs_index += 1;
                    ctx.cs_id_by_node_id.insert(node.id(), cs_id.clone());
                    let member = if frame.kind == "simple"
                        && !ctx.object_procedure_names.contains(&method_lc)
                    {
                        PCallee::Member {
                            receiver: frame.text.clone(),
                            method: method_text.clone(),
                        }
                    } else {
                        PCallee::Unknown
                    };
                    let bindings = extract_argument_bindings(ctx, &arg_nodes);
                    let under = if is_under_asserterror(node) {
                        Some(true)
                    } else {
                        None
                    };
                    ctx.call_sites.push(PCallSite {
                        id: cs_id,
                        operation_id: String::new(),
                        callee_text: format!("{}.{}", frame.text, method_text),
                        callee: member,
                        argument_texts,
                        argument_infos,
                        argument_bindings: bindings,
                        loop_stack: ctx.loop_stack.clone(),
                        source_anchor: ctx.anchor(node),
                        result_consumed: None,
                        object_run_return_used: None,
                        under_asserterror: under,
                        control_context: None,
                    });
                }
            } else {
                // Plain bare call.
                let (argument_texts, argument_infos, arg_nodes) = collect_args(ctx, node);
                let cs_id = format!("{}/cs{}", ctx.routine_id, ctx.cs_index);
                ctx.cs_index += 1;
                ctx.cs_id_by_node_id.insert(node.id(), cs_id.clone());
                let bindings = extract_argument_bindings(ctx, &arg_nodes);
                let under = if is_under_asserterror(node) {
                    Some(true)
                } else {
                    None
                };
                ctx.call_sites.push(PCallSite {
                    id: cs_id,
                    operation_id: String::new(),
                    callee_text: method_text.clone(),
                    callee: callee_from_node(node, ctx.source),
                    argument_texts,
                    argument_infos,
                    argument_bindings: bindings,
                    loop_stack: ctx.loop_stack.clone(),
                    source_anchor: ctx.anchor(node),
                    result_consumed: None,
                    object_run_return_used: None,
                    under_asserterror: under,
                    control_context: None,
                });
                // Error() additional error-call OperationSite.
                if method_text.to_lowercase() == "error" {
                    let error_op_id = format!("{}/op{}", ctx.routine_id, ctx.op_index);
                    ctx.op_index += 1;
                    let under_ae = is_under_asserterror(node);
                    ctx.operation_sites.push(POperationSite {
                        id: error_op_id,
                        kind: "error-call".to_string(),
                        loop_stack: ctx.loop_stack.clone(),
                        source_anchor: ctx.anchor(node),
                        under_asserterror: if under_ae { Some(true) } else { None },
                        control_context: None,
                    });
                }
            }
        }
    }

    recurse_into_args(ctx, node);
    chained_receiver_descent(ctx, node, func_node);
}

/// Recurse only into the argument_list children of a call_expression.
fn recurse_into_args(ctx: &mut Ctx, node: Node) {
    if let Some(arg_list) = child_of_kind(node, "argument_list") {
        for child in named_children(arg_list) {
            visit(ctx, child, Some(arg_list));
        }
    }
}

/// Chained-receiver descent: if callee is a member_expression whose object is a
/// call_expression (e.g. `Helper(C).FindSet()`), visit the inner call.
fn chained_receiver_descent(ctx: &mut Ctx, _node: Node, func_node: Node) {
    if func_node.kind() == "member_expression" {
        let obj_node = func_node
            .child_by_field_name("object")
            .or_else(|| func_node.named_child(0));
        if let Some(obj_node) = obj_node {
            if obj_node.kind() == "call_expression" {
                visit(ctx, obj_node, Some(func_node));
            }
        }
    }
}

pub fn visit(ctx: &mut Ctx, node: Node, parent: Option<Node>) {
    let node_type = node.kind();
    let parent_type = parent.map(|p| p.kind()).unwrap_or("");

    // Identifier-reference collection (value position).
    if let (Some(parent), true) = (parent, node_type == "identifier") {
        let mut is_value_ref = true;
        if parent_type == "member_expression" {
            if let Some(member_field) = parent.child_by_field_name("member") {
                if member_field.start_byte() == node.start_byte() {
                    is_value_ref = false;
                }
            }
        } else if parent_type == "qualified_enum_value" {
            let value_field = parent.child_by_field_name("value");
            let enum_type_field = parent.child_by_field_name("enum_type");
            if value_field.map(|v| v.start_byte()) == Some(node.start_byte()) {
                is_value_ref = false;
            } else if let Some(etf) = enum_type_field {
                if etf.start_byte() == node.start_byte() && etf.kind() == "identifier" {
                    is_value_ref = false;
                }
            }
        }
        if is_value_ref {
            ctx.identifier_ref_set.insert(ctx.text(node).to_lowercase());
        }
    }

    // Implicit-receiver routing shape 2: paren-less bare record-op identifier in
    // statement position.
    if let (Some(parent), true) = (parent, node_type == "identifier") {
        let method_lc = ctx.text(node).to_lowercase();
        if let Some(record_op) = record_op_type(&method_lc) {
            let mut is_statement = is_pure_statement_parent(parent_type);
            if !is_statement {
                if let Some(stmt_fields) = statement_fields_by_parent(parent_type) {
                    for f in stmt_fields {
                        if let Some(fc) = parent.child_by_field_name(f) {
                            if fc.start_byte() == node.start_byte() {
                                is_statement = true;
                                break;
                            }
                        }
                    }
                } else if parent_type == "with_statement" {
                    let body_node = parent
                        .child_by_field_name("body")
                        .or_else(|| named_children(parent).into_iter().last());
                    if let Some(bn) = body_node {
                        if bn.start_byte() == node.start_byte() {
                            is_statement = true;
                        }
                    }
                }
            }
            let frame = if is_statement {
                resolve_implicit_receiver(ctx).cloned()
            } else {
                None
            };
            if let Some(frame) = frame {
                let frame_is_record = frame.kind == "simple"
                    && is_record_receiver_text(&frame.text, ctx.variable_types_by_name)
                    && !ctx.object_procedure_names.contains(&method_lc);
                if frame_is_record {
                    push_record_op(ctx, node, record_op, &frame.text, None, None);
                } else {
                    let cs_id = format!("{}/cs{}", ctx.routine_id, ctx.cs_index);
                    ctx.cs_index += 1;
                    ctx.cs_id_by_node_id.insert(node.id(), cs_id.clone());
                    let member = if frame.kind == "simple"
                        && !ctx.object_procedure_names.contains(&method_lc)
                    {
                        PCallee::Member {
                            receiver: frame.text.clone(),
                            method: ctx.text(node).to_string(),
                        }
                    } else {
                        PCallee::Unknown
                    };
                    ctx.call_sites.push(PCallSite {
                        id: cs_id,
                        operation_id: String::new(),
                        callee_text: format!("{}.{}", frame.text, ctx.text(node)),
                        callee: member,
                        argument_texts: vec![],
                        argument_infos: vec![],
                        argument_bindings: vec![],
                        loop_stack: ctx.loop_stack.clone(),
                        source_anchor: ctx.anchor(node),
                        result_consumed: None,
                        object_run_return_used: None,
                        under_asserterror: None,
                        control_context: None,
                    });
                }
            }
        }
    }

    // Unreachable-after-exit scan when entering a code_block.
    if node_type == "code_block" {
        let stmts: Vec<Node> = named_children(node)
            .into_iter()
            .filter(|c| c.kind() != "begin_keyword" && c.kind() != "end_keyword")
            .collect();
        for i in 0..stmts.len().saturating_sub(1) {
            let s = stmts[i];
            if let Some(exit_kind) = unconditional_exit_kind(s, ctx.source) {
                let next = stmts[i + 1];
                ctx.unreachable_statements.push(PUnreachableStatement {
                    id: format!("{}/u{}", ctx.routine_id, ctx.unreachable_index),
                    exit_kind: exit_kind.to_string(),
                    exit_anchor: ctx.anchor(s),
                    unreachable_anchor: ctx.anchor(next),
                });
                ctx.unreachable_index += 1;
                break;
            }
        }
    }

    // Branching detection.
    if node_type == "if_statement"
        || node_type == "case_statement"
        || node_type == "case_branch"
        || node_type == "try_statement"
    {
        ctx.has_branching = true;
    }

    // Loop detection.
    let loop_type = match node_type {
        "repeat_statement" => Some("repeat"),
        "for_statement" => Some("for"),
        "foreach_statement" => Some("foreach"),
        "while_statement" => Some("while"),
        _ => None,
    };
    let mut pushed_loop = false;
    if let Some(loop_type) = loop_type {
        let id = format!("{}/loop{}", ctx.routine_id, ctx.loops.len());
        ctx.loops.push(PLoop {
            id: id.clone(),
            loop_type: loop_type.to_string(),
            source_anchor: ctx.anchor(node),
        });
        ctx.loop_stack.push(id);
        pushed_loop = true;
    }

    if node_type == "call_expression" {
        handle_call_expression(ctx, node, parent);
        if pushed_loop {
            ctx.loop_stack.pop();
        }
        return;
    }

    if node_type == "member_expression" {
        let obj_node = node
            .child_by_field_name("object")
            .or_else(|| node.named_child(0));
        let member_node = node
            .child_by_field_name("member")
            .or_else(|| node.named_child(1));
        if let (Some(obj_node), Some(member_node)) = (obj_node, member_node) {
            let mut is_statement_position = is_pure_statement_parent(parent_type);
            if !is_statement_position {
                if let Some(parent) = parent {
                    if let Some(stmt_fields) = statement_fields_by_parent(parent_type) {
                        for field_name in stmt_fields {
                            if let Some(fc) = parent.child_by_field_name(field_name) {
                                if fc.start_byte() == node.start_byte() {
                                    is_statement_position = true;
                                    break;
                                }
                            }
                        }
                    } else if parent_type == "repeat_statement" {
                        let cond = parent.child_by_field_name("condition");
                        if cond.map(|c| c.start_byte()) != Some(node.start_byte()) {
                            is_statement_position = true;
                        }
                    }
                }
            }
            if is_statement_position {
                let method_lc = ctx.text(member_node).to_lowercase();
                let op_type = record_op_type(&method_lc);
                if let Some(op_type) = op_type {
                    if is_record_receiver(ctx, ctx.text(obj_node)) {
                        push_record_op(ctx, node, op_type, ctx.text(obj_node), None, None);
                        if pushed_loop {
                            ctx.loop_stack.pop();
                        }
                        return;
                    }
                }
                // Parameterless method call → CallSite.
                let cs_id = format!("{}/cs{}", ctx.routine_id, ctx.cs_index);
                ctx.cs_index += 1;
                ctx.cs_id_by_node_id.insert(node.id(), cs_id.clone());
                ctx.call_sites.push(PCallSite {
                    id: cs_id,
                    operation_id: String::new(),
                    callee_text: ctx.text(node).to_string(),
                    callee: callee_from_node(node, ctx.source),
                    argument_texts: vec![],
                    argument_infos: vec![],
                    argument_bindings: vec![],
                    loop_stack: ctx.loop_stack.clone(),
                    source_anchor: ctx.anchor(node),
                    result_consumed: None,
                    object_run_return_used: None,
                    under_asserterror: None,
                    control_context: None,
                });
                if pushed_loop {
                    ctx.loop_stack.pop();
                }
                return;
            }
            // Expression-position field access.
            let is_enum_scope_ref = parent_type == "qualified_enum_value";
            let record_variable_name = ctx.text(obj_node).to_string();
            if !is_enum_scope_ref
                && ctx
                    .record_var_names
                    .contains(&record_variable_name.to_lowercase())
            {
                let member_text = ctx.text(member_node);
                let field_name = if member_node.kind() == "quoted_identifier"
                    && member_text.len() >= 2
                    && member_text.starts_with('"')
                    && member_text.ends_with('"')
                {
                    member_text[1..member_text.len() - 1].to_string()
                } else {
                    member_text.to_string()
                };
                ctx.field_accesses.push(PFieldAccess {
                    record_variable_name,
                    field_name,
                    source_anchor: ctx.anchor(node),
                });
            }
        }
        // Continue into children for chained accesses.
    }

    if node_type == "with_statement" {
        let receiver_node = named_children(node)
            .into_iter()
            .find(|c| c.kind() != "with_keyword" && c.kind() != "do_keyword");
        let body_node = node
            .child_by_field_name("body")
            .or_else(|| named_children(node).into_iter().last());
        if let Some(receiver_node) = receiver_node {
            if Some(receiver_node.id()) != body_node.map(|b| b.id()) {
                visit(ctx, receiver_node, Some(node));
            }
        }
        let receiver_text = receiver_node
            .map(|n| ctx.text(n).to_string())
            .unwrap_or_default();
        let simple_name = simple_receiver_name(&receiver_text);
        let frame = ImplicitReceiverFrame {
            text: receiver_text.trim().to_string(),
            kind: if simple_name.is_some() {
                "simple"
            } else {
                "unknown"
            },
        };
        ctx.implicit_receiver_stack.push(frame);
        if let Some(body_node) = body_node {
            visit(ctx, body_node, Some(node));
        }
        ctx.implicit_receiver_stack.pop();
        if pushed_loop {
            ctx.loop_stack.pop();
        }
        return;
    }

    for child in named_children(node) {
        visit(ctx, child, Some(node));
    }

    if pushed_loop {
        ctx.loop_stack.pop();
    }
}

/// Collect every `<lhs> := <rhs>` assignment (sorted by source position).
fn collect_var_assignments(ctx: &Ctx, body_node: Node) -> Vec<PVarAssignment> {
    let mut out = Vec::new();
    let mut stack = vec![body_node];
    while let Some(n) = stack.pop() {
        if n.kind() == "assignment_statement" {
            let target = n
                .child_by_field_name("left")
                .or_else(|| n.child_by_field_name("target"))
                .or_else(|| n.named_child(0));
            let value = n
                .child_by_field_name("right")
                .or_else(|| n.child_by_field_name("value"))
                .or_else(|| {
                    let c = n.named_child_count();
                    if c > 0 {
                        n.named_child(c as u32 - 1)
                    } else {
                        None
                    }
                });
            if let (Some(target), Some(value)) = (target, value) {
                if let Some(lhs_name) = lhs_identifier_of(target, ctx.source) {
                    out.push(PVarAssignment {
                        lhs_name: lhs_name.to_lowercase(),
                        rhs_literal_value: literal_text_of(value, ctx.source),
                        source_anchor: ctx.anchor(n),
                    });
                }
            }
        }
        for c in named_children(n) {
            stack.push(c);
        }
    }
    out.sort_by(|a, b| {
        (a.source_anchor.start_line, a.source_anchor.start_column)
            .cmp(&(b.source_anchor.start_line, b.source_anchor.start_column))
    });
    out
}

fn lhs_identifier_of(target: Node, source: &str) -> Option<String> {
    if target.kind() == "identifier" {
        return Some(node_text(target, source).to_string());
    }
    if target.kind() == "member_expression" {
        let member = target.child_by_field_name("member").or_else(|| {
            let c = target.named_child_count();
            if c > 0 {
                target.named_child(c as u32 - 1)
            } else {
                None
            }
        });
        return member.map(|m| node_text(m, source).to_string());
    }
    None
}

fn literal_text_of(value: Node, source: &str) -> Option<String> {
    let text = node_text(value, source);
    match value.kind() {
        "boolean" => Some(text.to_lowercase()),
        "integer" => Some(text.to_string()),
        "string_literal" => {
            let stripped = if text.len() >= 2 && text.starts_with('\'') && text.ends_with('\'') {
                &text[1..text.len() - 1]
            } else {
                text
            };
            Some(stripped.to_lowercase())
        }
        _ => None,
    }
}

/// Collect condition references (if/while/repeat-until/case subject), sorted by
/// referenceAnchor.
fn collect_condition_references(ctx: &Ctx, body_node: Node) -> Vec<PConditionReference> {
    let mut out = Vec::new();

    fn collect_idents(
        ctx: &Ctx,
        out: &mut Vec<PConditionReference>,
        expr: Option<Node>,
        kind: &str,
        stmt: &PAnchor,
    ) {
        let Some(expr) = expr else {
            return;
        };
        if expr.kind() == "identifier" {
            out.push(PConditionReference {
                identifier: node_text(expr, ctx.source).to_lowercase(),
                condition_kind: kind.to_string(),
                statement_anchor: stmt.clone(),
                reference_anchor: ctx.anchor(expr),
            });
            return;
        }
        if expr.kind() == "member_expression" {
            let member = expr.child_by_field_name("member").or_else(|| {
                let c = expr.named_child_count();
                if c > 0 {
                    expr.named_child(c as u32 - 1)
                } else {
                    None
                }
            });
            if let Some(member) = member {
                if member.kind() == "identifier" {
                    out.push(PConditionReference {
                        identifier: node_text(member, ctx.source).to_lowercase(),
                        condition_kind: kind.to_string(),
                        statement_anchor: stmt.clone(),
                        reference_anchor: ctx.anchor(member),
                    });
                }
            }
            return;
        }
        for c in named_children(expr) {
            collect_idents(ctx, out, Some(c), kind, stmt);
        }
    }

    let mut stack = vec![body_node];
    while let Some(n) = stack.pop() {
        let kind_field = match n.kind() {
            "if_statement" => Some(("condition", "if")),
            "while_statement" => Some(("condition", "while")),
            "repeat_statement" => Some(("condition", "repeat-until")),
            "case_statement" => Some(("expression", "case")),
            _ => None,
        };
        if let Some((field, kind)) = kind_field {
            if let Some(cond) = n.child_by_field_name(field) {
                let stmt = ctx.anchor(n);
                collect_idents(ctx, &mut out, Some(cond), kind, &stmt);
            }
        }
        for c in named_children(n) {
            stack.push(c);
        }
    }

    out.sort_by(|a, b| {
        (
            a.reference_anchor.start_line,
            a.reference_anchor.start_column,
        )
            .cmp(&(
                b.reference_anchor.start_line,
                b.reference_anchor.start_column,
            ))
    });
    out
}

/// Run the body walk + collect the post-walk streams. Does NOT build the CFN or
/// the recordOperation backfill — see `mod.rs::extract_body_features`.
#[allow(clippy::too_many_arguments)]
pub fn run_walk<'a>(
    body_node: Node<'a>,
    source: &'a str,
    cols: &'a Utf16Cols<'a>,
    routine_id: &'a str,
    source_unit_id: &'a str,
    record_var_names: &'a HashSet<String>,
    enclosing_parameters: &'a [ParameterSymbol],
    enclosing_record_variables: &'a [RecordVariable],
    variable_types_by_name: &'a HashMap<String, String>,
    implicit_base_receiver: Option<ImplicitReceiverFrame>,
    object_procedure_names: &'a HashSet<String>,
) -> ExtractBodyResult {
    let mut implicit_receiver_stack = Vec::new();
    if let Some(base) = implicit_base_receiver {
        implicit_receiver_stack.push(base);
    }
    let mut ctx = Ctx {
        source,
        cols,
        routine_id,
        source_unit_id,
        record_var_names,
        enclosing_parameters,
        enclosing_record_variables,
        variable_types_by_name,
        object_procedure_names,
        loops: Vec::new(),
        operation_sites: Vec::new(),
        record_operations: Vec::new(),
        call_sites: Vec::new(),
        field_accesses: Vec::new(),
        unreachable_statements: Vec::new(),
        identifier_ref_set: HashSet::new(),
        loop_stack: Vec::new(),
        implicit_receiver_stack,
        op_index: 0,
        cs_index: 0,
        unreachable_index: 0,
        has_branching: false,
        op_id_by_node_id: HashMap::new(),
        cs_id_by_node_id: HashMap::new(),
    };

    visit(&mut ctx, body_node, None);

    // Two-phase numbering: assign callsite operationIds = op{opIndex + i}.
    let op_index = ctx.op_index;
    for (i, cs) in ctx.call_sites.iter_mut().enumerate() {
        cs.operation_id = format!("{}/op{}", routine_id, op_index + i as u32);
    }

    let mut identifier_references: Vec<String> = ctx.identifier_ref_set.iter().cloned().collect();
    identifier_references.sort();

    let var_assignments = collect_var_assignments(&ctx, body_node);
    let condition_references = collect_condition_references(&ctx, body_node);

    ExtractBodyResult {
        loops: ctx.loops,
        operation_sites: ctx.operation_sites,
        record_operations: ctx.record_operations,
        call_sites: ctx.call_sites,
        field_accesses: ctx.field_accesses,
        unreachable_statements: ctx.unreachable_statements,
        has_branching: ctx.has_branching,
        identifier_references,
        var_assignments,
        condition_references,
        op_id_by_node_id: ctx.op_id_by_node_id,
        cs_id_by_node_id: ctx.cs_id_by_node_id,
    }
}
