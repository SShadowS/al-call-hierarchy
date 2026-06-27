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
pub mod capability;
pub mod cfn;
pub mod classify;
pub mod control_context;
pub mod control_flow;
pub mod features;
pub mod ir_walk;
pub mod l2_workspace;
pub mod node_util;
pub mod operation_order;
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
pub fn get_return_type_text(node: Node, source: &str) -> Option<String> {
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

pub(crate) fn compute_nesting_depth(loops: &[features::PLoop]) -> u32 {
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
    // A table / tableextension method — trigger OR procedure — operates on the
    // implicit current record (`Rec`). AL exposes the table's fields and procedures
    // unqualified inside ANY of its methods, not just triggers, so seed `Rec` for
    // both (e.g. a table procedure doing `"File Blob".CreateInStream(...)` or
    // `Rec.<field>`). Field triggers (OnValidate) report `kind == "trigger"` too.
    let is_table_method = (obj_type_lc == "table" || obj_type_lc == "tableextension")
        && (kind == "trigger" || kind == "procedure");
    let is_page_with_source_table = obj_type_lc == "page" && source_table_name.is_some();
    let is_page_extension = obj_type_lc == "pageextension";
    if is_table_method || is_page_with_source_table || is_page_extension {
        Some(ImplicitReceiverFrame {
            text: "Rec".to_string(),
            kind: "simple",
        })
    } else {
        None
    }
}

/// For a routine nested inside a report `dataitem(Name; "Source Table")`, return
/// the dataitem's SOURCE TABLE name (the `table_name` field), already unquoted.
/// A report has MULTIPLE dataitems, each over a DIFFERENT table, so the implicit
/// `Rec` of a dataitem trigger is typed per-dataitem — not by a single object-level
/// own-table. We therefore read it directly from the ENCLOSING `report_dataitem`
/// (the innermost one, for nested dataitems) by walking up the parse tree.
///
/// Returns `None` when the routine is not inside a dataitem (e.g. a report-level
/// procedure or a `OnInitReport`/`OnPreReport` trigger on the report itself).
fn report_dataitem_source_table(routine: Node, source: &str) -> Option<String> {
    let mut node = routine.parent();
    while let Some(n) = node {
        if n.kind() == "report_dataitem" {
            let table_node = n.child_by_field_name("table_name")?;
            return Some(strip_quotes(node_text(table_node, source)).to_string());
        }
        node = n.parent();
    }
    None
}

/// Collect every report `dataitem(Name; "Source Table")` declared anywhere in the
/// object (including nested dataitems) as `(dataitem name, source table)` pairs,
/// both unquoted. AL lets you reference a dataitem BY NAME as a record variable
/// typed to its source table — e.g. report code doing `"Sales Header Filter".GetView()`
/// where `"Sales Header Filter"` is the NAME of `dataitem("Sales Header Filter";
/// "Sales Header")`. The dataitem name is in scope across ALL of the report's
/// routines (report-level procedures + sibling dataitem triggers), so we seed each
/// as a record variable in every routine and let `record_types` pass-1 resolve the
/// `table_id` from the source-table name. Distinct from the per-dataitem implicit
/// `Rec` of a dataitem trigger (which `report_dataitem_source_table` handles).
fn report_dataitem_record_vars(decl: Node, source: &str) -> Vec<(String, String)> {
    fn walk(n: Node, source: &str, out: &mut Vec<(String, String)>) {
        if n.kind() == "report_dataitem" {
            if let (Some(name_node), Some(table_node)) = (
                n.child_by_field_name("name"),
                n.child_by_field_name("table_name"),
            ) {
                let name = strip_quotes(node_text(name_node, source)).to_string();
                let table = strip_quotes(node_text(table_node, source)).to_string();
                if !name.is_empty() && !table.is_empty() {
                    out.push((name, table));
                }
            }
        }
        // Don't descend into routine bodies — dataitems live in the dataset section.
        if n.kind() == "code_block" {
            return;
        }
        for c in named_children(n) {
            walk(c, source, out);
        }
    }
    let mut out = Vec::new();
    walk(decl, source, &mut out);
    out
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
    let mut record_variables = extract_record_variables(routine, &routine_id, &parameters, source);
    // A report dataitem trigger (OnAfterGetRecord / OnPreDataItem / OnPostDataItem,
    // OnAfterImportRecord, …) operates on an implicit `Rec` typed to ITS dataitem's
    // SOURCE TABLE. Unlike the object-level own-table path below, a report has
    // MULTIPLE dataitems each over a DIFFERENT table, so there is no single
    // object-level own-table to backfill from in `record_types` pass-3. We instead
    // read the enclosing `report_dataitem`'s source table HERE and seed the `Rec`
    // var WITH that `table_name` — `record_types` pass-1 (declared-record-var
    // resolution) then backfills its `table_id` by name, exactly as for a declared
    // record var. Skipped when a declared `Rec` already exists (never shadow it).
    let dataitem_table = if object_type == "Report" || object_type == "ReportExtension" {
        report_dataitem_source_table(routine, source).filter(|s| !s.is_empty())
    } else {
        None
    };
    // A codeunit with a `TableNo` property runs against an implicit `Rec` of that
    // table (its `OnRun(var Rec)` parameter; AL exposes `Rec` unqualified inside the
    // codeunit). `TableNo` is a table NAME or NUMBER — set directly as the seeded
    // Rec's `table_name`; `record_types` pass-1 resolves either form.
    let codeunit_tableno = if object_type == "Codeunit" {
        read_object_property(decl, "TableNo", source)
            .map(|s| strip_quotes(&s).to_string())
            .filter(|s| !s.is_empty())
    } else {
        None
    };
    // The per-routine implicit-`Rec` table NAME/NUMBER, when it must be set directly
    // (report dataitem or codeunit TableNo); `None` for the object-level cases that
    // `record_types` pass-3 backfills from the own-table.
    let direct_rec_table = dataitem_table.clone().or_else(|| codeunit_tableno.clone());

    // d22 FN fix: register the IMPLICIT `Rec` of a table-trigger / page (SourceTable)
    // / page-extension routine as a record variable so `Rec.<field>` reads are
    // captured as field accesses (gated on `record_var_names`). For table/page/ext
    // the table is NOT resolved here — `table_name` is left None and L3
    // `record_types` pass-3 fills the var's `table_id` from the EFFECTIVE OWN TABLE
    // (Table → self, Page → SourceTable, TableExt → extends target, PageExt → base
    // page's SourceTable), the single source of truth that already resolves the
    // implicit-Rec OPS. Setting a name there (e.g. the object name) would wrongly
    // hijack a tableextension's Rec to the extension object instead of the extended
    // base table. For a report DATAITEM the table varies per dataitem, so we DO set
    // `table_name` (to the dataitem's source table) and let pass-1 resolve it.
    // Skipped when a declared `Rec` already exists (never shadow it).
    if (implicit_base_receiver(object_type, kind, source_table_name).is_some()
        || direct_rec_table.is_some())
        && !record_variables
            .iter()
            .any(|v| v.name.eq_ignore_ascii_case("Rec"))
    {
        record_variables.push(scope::RecordVariable {
            id: format!("{routine_id}/rv/rec"),
            name: "Rec".to_string(),
            table_name: direct_rec_table.clone(),
            temp_state: scope::ts_known(false),
            is_parameter: false,
            parameter_index: None,
        });
    }
    // Seed each report dataitem NAME as a record variable typed to its source
    // table, visible across ALL of the report's routines (the dataitem names are in
    // scope in report-level procedures + sibling dataitem triggers). AL lets you
    // reference a dataitem by name as a record — e.g. `"Sales Header Filter".GetView()`
    // for `dataitem("Sales Header Filter"; "Sales Header")`. `record_types` pass-1
    // resolves the `table_id` from `table_name`. Never shadow a declared var, the
    // implicit `Rec`, or a duplicate dataitem name already seeded.
    if object_type == "Report" || object_type == "ReportExtension" {
        for (di_name, di_table) in report_dataitem_record_vars(decl, source) {
            if record_variables
                .iter()
                .any(|v| v.name.eq_ignore_ascii_case(&di_name))
            {
                continue;
            }
            record_variables.push(scope::RecordVariable {
                id: format!("{routine_id}/rv/dataitem/{di_name}"),
                name: di_name,
                table_name: Some(di_table),
                temp_state: scope::ts_known(false),
                is_parameter: false,
                parameter_index: None,
            });
        }
    }
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

    let base_recv = implicit_base_receiver(object_type, kind, source_table_name).or_else(|| {
        // Report dataitem trigger / codeunit `TableNo` — the implicit receiver is the
        // routine's `Rec` (the dataitem's record / the codeunit's TableNo record).
        direct_rec_table.as_ref().map(|_| ImplicitReceiverFrame {
            text: "Rec".to_string(),
            kind: "simple",
        })
    });

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

/// Compute a routine's `normalizedSignatureHash` (the return-type-aware canonical
/// signature SHA-256) from its node. Mirrors the hash baked into the internal
/// routine id and the StableRoutineId (`${stableObjectId}#${normalizedSignatureHash}`).
/// L3's record-type projection needs the StableRoutineId, which is keyed by this
/// hash; computing it here reuses the same `extract_parameters` / `classify_kind` /
/// return-type extraction the routine-id path uses, so the two cannot drift.
pub fn routine_normalized_signature_hash(routine: Node, source: &str) -> Option<String> {
    let name_node = routine.child_by_field_name("name")?;
    let name = strip_quotes(node_text(name_node, source)).to_string();
    if name.is_empty() {
        return None;
    }
    let parameters = extract_parameters(routine, source);
    let return_type_text = get_return_type_text(routine, source);
    let param_specs: Vec<crate::engine::ids::ParamSpec> = parameters
        .iter()
        .map(|p| crate::engine::ids::ParamSpec {
            type_text: p.type_text.clone(),
            is_var: p.is_var,
        })
        .collect();
    Some(crate::engine::ids::normalized_signature_hash(
        &name,
        &param_specs,
        return_type_text.as_deref(),
    ))
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
        scope_frames: vec![],
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
            scope: None,
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
        cols,
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
        // Populated by the emitter (`l2_workspace.rs`) via `apply_operation_order`;
        // body_walk leaves it empty.
        scope_frames: vec![],
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
