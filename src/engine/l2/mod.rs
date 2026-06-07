//! L2 structural body-walk + feature projection (R1a Task 2).
//!
//! Public surface:
//!   - [`features::PFeatures`] / sibling serde types = the allowlisted L2 shape.
//!   - [`project_routine_features`] — given an object-decl node + routine node +
//!     identity context, produce the projected `PFeatures` (the parity surface).
//!
//! This is the Rust port of al-sem's single-DFS body walker
//! (`intraprocedural-body.ts`) + the L2-relevant parts of `routine-indexer.ts`,
//! `variable-indexer.ts`, `intraprocedural-refs.ts`. Control-context (R1b),
//! operation-order (R1c), and capability facts (R1d) are intentionally OUT.

pub mod body_walk;
pub mod cfn;
pub mod classify;
pub mod features;
pub mod l2_workspace;
pub mod node_util;
pub mod record_op;
pub mod scope;

use body_walk::ImplicitReceiverFrame;
use features::{PCFNNode, PFeatures};
use node_util::{named_children, node_text, strip_quotes, Utf16Cols};
use scope::{
    build_variable_type_index, compute_routine_id, extract_object_globals, extract_parameters,
    extract_record_variables, extract_variables, ParameterSymbol, RecordVariable,
};
use std::collections::{HashMap, HashSet};
use tree_sitter::Node;

/// Identity context needed to compute routine ids and source anchors.
pub struct IdentityCtx<'a> {
    pub app_guid: &'a str,
    pub model_instance_id: &'a str,
    pub source_unit_id: &'a str,
}

/// Find the code_block child of a routine node.
pub(crate) fn find_code_block(node: Node) -> Option<Node> {
    named_children(node)
        .into_iter()
        .find(|c| c.kind() == "code_block")
}

/// Read the return-type text — first direct `type_specification` named child.
fn get_return_type_text(node: Node, source: &str) -> Option<String> {
    named_children(node)
        .into_iter()
        .find(|c| c.kind() == "type_specification")
        .map(|c| node_text(c, source).to_string())
}

/// `extractObjectNumber` — first `integer` named child, else 0.
pub(crate) fn extract_object_number(decl: Node, source: &str) -> i64 {
    for child in named_children(decl) {
        if child.kind() == "integer" {
            return node_text(child, source).trim().parse::<i64>().unwrap_or(0);
        }
    }
    0
}

/// Classify routine kind from preceding `attribute_item` siblings (event attrs).
fn classify_kind(node: Node, source: &str) -> &'static str {
    let mut kind = if node.kind() == "trigger_declaration" {
        "trigger"
    } else {
        "procedure"
    };
    let mut sibling = node.prev_sibling();
    while let Some(sib) = sibling {
        if sib.kind() != "attribute_item" {
            break;
        }
        if let Some(content) = sib.child_by_field_name("attribute") {
            if let Some(name_node) = content.child_by_field_name("name") {
                let name_lc = node_text(name_node, source).to_lowercase();
                if name_lc == "eventsubscriber" {
                    kind = "event-subscriber";
                    break;
                } else if name_lc == "integrationevent" || name_lc == "businessevent" {
                    kind = "event-publisher";
                    break;
                }
            }
        }
        sibling = sib.prev_sibling();
    }
    kind
}

/// True if loop `outer`'s source range strictly contains loop `inner`'s.
fn loop_strictly_contains(outer: &features::PLoop, inner: &features::PLoop) -> bool {
    if outer.id == inner.id {
        return false;
    }
    let o = &outer.source_anchor;
    let i = &inner.source_anchor;
    let starts_before = o.start_line < i.start_line
        || (o.start_line == i.start_line && o.start_column <= i.start_column);
    let ends_after =
        o.end_line > i.end_line || (o.end_line == i.end_line && o.end_column >= i.end_column);
    starts_before && ends_after
}

fn compute_nesting_depth(loops: &[features::PLoop]) -> u32 {
    let mut max_depth = 0;
    for loop_ in loops {
        let enclosing = loops
            .iter()
            .filter(|other| loop_strictly_contains(other, loop_))
            .count() as u32;
        let depth = 1 + enclosing;
        if depth > max_depth {
            max_depth = depth;
        }
    }
    max_depth
}

/// Build the implicit base-Rec frame for trigger / page-SourceTable routines.
fn implicit_base_receiver(
    object_type: &str,
    kind: &str,
    source_table_name: Option<&str>,
) -> Option<ImplicitReceiverFrame> {
    let obj_type_lc = object_type.to_lowercase();
    let is_table_trigger =
        (obj_type_lc == "table" || obj_type_lc == "tableextension") && kind == "trigger";
    let is_page_with_source_table = obj_type_lc == "page" && source_table_name.is_some();
    let is_page_extension = obj_type_lc == "pageextension";
    if is_table_trigger || is_page_with_source_table || is_page_extension {
        Some(ImplicitReceiverFrame {
            text: "Rec".to_string(),
            kind: "simple",
        })
    } else {
        None
    }
}

/// Read a simple object property value (e.g. SourceTable) for implicit-Rec seeding.
fn read_object_property(decl: Node, name: &str, source: &str) -> Option<String> {
    fn find<'a>(n: Node<'a>, name_lc: &str, source: &str) -> Option<Node<'a>> {
        if n.kind() == "property" {
            if let Some(name_node) = n.child_by_field_name("name") {
                if node_text(name_node, source).to_lowercase() == name_lc {
                    return Some(n);
                }
            }
        }
        // Don't descend into routine bodies.
        if n.kind() == "code_block" {
            return None;
        }
        for c in named_children(n) {
            if let Some(found) = find(c, name_lc, source) {
                return Some(found);
            }
        }
        None
    }
    let name_lc = name.to_lowercase();
    let prop = find(decl, &name_lc, source)?;
    let value_node = prop.child_by_field_name("value")?;
    Some(node_text(value_node, source).to_string())
}

/// Run the body walk + post-passes and produce projected `PFeatures` for one
/// routine. `decl` is the enclosing object declaration node (for object metadata
/// + global var scope); `routine` is the procedure/trigger node.
#[allow(clippy::too_many_arguments)]
pub fn project_routine_features(
    decl: Node,
    routine: Node,
    object_type: &str,
    object_number: i64,
    source_table_name: Option<&str>,
    object_procedure_names: &HashSet<String>,
    object_globals: &[features::PVariableSymbol],
    id_ctx: &IdentityCtx,
    source: &str,
    cols: &Utf16Cols,
) -> Option<(String, PFeatures)> {
    let name_node = routine.child_by_field_name("name")?;
    let name = strip_quotes(node_text(name_node, source)).to_string();
    if name.is_empty() {
        return None;
    }
    let kind = classify_kind(routine, source);
    let parameters = extract_parameters(routine, source);
    let return_type_text = get_return_type_text(routine, source);
    let routine_id = compute_routine_id(
        id_ctx.app_guid,
        object_type,
        object_number,
        kind,
        &name,
        &parameters,
        return_type_text.as_deref(),
        id_ctx.model_instance_id,
    );

    let body = find_code_block(routine);
    let record_variables = extract_record_variables(routine, &routine_id, &parameters, source);
    let record_var_names: HashSet<String> = record_variables
        .iter()
        .map(|v| v.name.to_lowercase())
        .collect();

    let variables = extract_variables(
        routine,
        id_ctx.source_unit_id,
        &parameters,
        object_globals,
        source,
        cols,
    );
    let variable_types_by_name = build_variable_type_index(&variables);

    let base_recv = implicit_base_receiver(object_type, kind, source_table_name);

    let features = if let Some(body) = body {
        extract_body_features(
            body,
            source,
            cols,
            &routine_id,
            id_ctx.source_unit_id,
            &record_var_names,
            &parameters,
            &record_variables,
            &variable_types_by_name,
            base_recv,
            object_procedure_names,
            &variables,
        )
    } else {
        empty_features(&variables, &record_variables)
    };
    let _ = (decl, object_number);

    Some((routine_id, features))
}

fn empty_features(
    variables: &[features::PVariableSymbol],
    record_variables: &[RecordVariable],
) -> PFeatures {
    PFeatures {
        loops: vec![],
        operation_sites: vec![],
        record_operations: vec![],
        call_sites: vec![],
        field_accesses: vec![],
        record_variables: project_record_variables(record_variables),
        nesting_depth: 0,
        has_branching: false,
        unreachable_statements: vec![],
        identifier_references: vec![],
        variables: variables.to_vec(),
        var_assignments: vec![],
        condition_references: vec![],
        statement_tree: None,
    }
}

fn project_record_variables(record_variables: &[RecordVariable]) -> Vec<features::PRecordVariable> {
    record_variables
        .iter()
        .map(|rv| features::PRecordVariable {
            id: rv.id.clone(),
            name: rv.name.clone(),
            table_name: rv.table_name.clone(),
            temp_state: rv.temp_state.clone(),
            is_parameter: rv.is_parameter,
            parameter_index: rv.parameter_index,
        })
        .collect()
}

/// Single-DFS walk + post-passes (recordOp backfill, nestingDepth, CFN).
#[allow(clippy::too_many_arguments)]
fn extract_body_features(
    body: Node,
    source: &str,
    cols: &Utf16Cols,
    routine_id: &str,
    source_unit_id: &str,
    record_var_names: &HashSet<String>,
    parameters: &[ParameterSymbol],
    record_variables: &[RecordVariable],
    variable_types_by_name: &HashMap<String, String>,
    base_recv: Option<ImplicitReceiverFrame>,
    object_procedure_names: &HashSet<String>,
    variables: &[features::PVariableSymbol],
) -> PFeatures {
    let result = body_walk::run_walk(
        body,
        source,
        cols,
        routine_id,
        source_unit_id,
        record_var_names,
        parameters,
        record_variables,
        variable_types_by_name,
        base_recv,
        object_procedure_names,
    );

    // recordOperation backfill: copy tempState + recordVariableId from the
    // declaring RecordVariable (matched by lc name).
    let rec_var_by_lc: HashMap<String, &RecordVariable> = record_variables
        .iter()
        .map(|rv| (rv.name.to_lowercase(), rv))
        .collect();
    let mut record_operations = result.record_operations;
    for op in &mut record_operations {
        if let Some(rv) = rec_var_by_lc.get(&op.record_variable_name.to_lowercase()) {
            if op.record_variable_id.is_none() {
                op.record_variable_id = Some(rv.id.clone());
            }
            op.temp_state = rv.temp_state.clone();
        }
    }

    let nesting_depth = compute_nesting_depth(&result.loops);

    // Build the CFN skeleton.
    let cfn_ctx = cfn::CfnCtx {
        source,
        op_id_by_node_id: &result.op_id_by_node_id,
        cs_id_by_node_id: &result.cs_id_by_node_id,
    };
    let statement_tree: Option<PCFNNode> = Some(cfn_ctx.build_block(body));

    PFeatures {
        loops: result.loops,
        operation_sites: result.operation_sites,
        record_operations,
        call_sites: result.call_sites,
        field_accesses: result.field_accesses,
        record_variables: project_record_variables(record_variables),
        nesting_depth,
        has_branching: result.has_branching,
        unreachable_statements: result.unreachable_statements,
        identifier_references: result.identifier_references,
        variables: variables.to_vec(),
        var_assignments: result.var_assignments,
        condition_references: result.condition_references,
        statement_tree,
    }
}

/// Drive the L2 walk over an entire single-file source for the vector tests:
/// parse, find the named routine, project its features. Returns the projected
/// `PFeatures` (None if the routine isn't found).
pub fn features_for_named_routine(
    source: &str,
    routine_name: &str,
    app_guid: &str,
    model_instance_id: &str,
    source_unit_id: &str,
    tree: &tree_sitter::Tree,
) -> Option<PFeatures> {
    let root = tree.root_node();
    let cols = Utf16Cols::new(source);

    for decl in named_children(root) {
        let Some(object_type) = scope::object_type_for(decl.kind()) else {
            continue;
        };
        let object_number = extract_object_number(decl, source);

        // Object metadata for implicit-Rec seeding.
        let source_table_name = if object_type == "Page" || object_type == "PageExtension" {
            read_object_property(decl, "SourceTable", source).map(|s| strip_quotes(&s).to_string())
        } else {
            None
        };

        // Object globals.
        let object_globals = extract_object_globals(decl, source_unit_id, source);

        // Routine nodes (prune-at-match).
        let routine_nodes = collect_routine_nodes(decl);

        // Object procedure-name collision set.
        let mut object_procedure_names = HashSet::new();
        for n in &routine_nodes {
            if let Some(nm) = n.child_by_field_name("name") {
                object_procedure_names.insert(strip_quotes(node_text(nm, source)).to_lowercase());
            }
        }

        let id_ctx = IdentityCtx {
            app_guid,
            model_instance_id,
            source_unit_id,
        };

        for routine in routine_nodes {
            let Some(nm) = routine.child_by_field_name("name") else {
                continue;
            };
            let rname = strip_quotes(node_text(nm, source)).to_string();
            if rname != routine_name {
                continue;
            }
            return project_routine_features(
                decl,
                routine,
                object_type,
                object_number,
                source_table_name.as_deref(),
                &object_procedure_names,
                &object_globals,
                &id_ctx,
                source,
                &cols,
            )
            .map(|(_, f)| f);
        }
    }
    None
}

/// `collectDescendants(prune-at-match)` for procedure / trigger_declaration.
fn collect_routine_nodes(decl: Node) -> Vec<Node> {
    let mut out = Vec::new();
    let mut stack = vec![decl];
    while let Some(node) = stack.pop() {
        if node.kind() == "procedure" || node.kind() == "trigger_declaration" {
            out.push(node);
            continue;
        }
        for child in named_children(node) {
            stack.push(child);
        }
    }
    out
}
