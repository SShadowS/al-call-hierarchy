//! Dual-run parity support (owned-syntax-IR migration). Exposes the LEGACY
//! tree-sitter extraction the engine relies on, reachable from integration tests,
//! so the IR lowerer can be diffed against it. Removed at the Phase 5 seal.

use crate::language;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Parser, Query, QueryCursor};

/// Run the REAL legacy L2 walk per routine and return `(routine_name, PFeatures)`
/// for every routine in the source. This is the Phase-2 dual-run gate: it exposes
/// the actual engine `PFeatures` (call sites, ops, record ops, CFN, branching, …)
/// so the IR re-expression can be diffed against it, not against query proxies.
/// Minimal identity context (feature extraction is body-structural).
pub fn legacy_l2_features(source: &str) -> Vec<(String, crate::engine::l2::features::PFeatures)> {
    use crate::engine::l2::node_util::Utf16Cols;
    use crate::engine::l2::IdentityCtx;

    let lang = language::language();
    let mut parser = Parser::new();
    if parser.set_language(&lang).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source, None) else {
        return Vec::new();
    };
    let cols = Utf16Cols::new(source);
    let id_ctx = IdentityCtx {
        app_guid: "dual",
        model_instance_id: "dual",
        source_unit_id: "dual",
    };
    let mut out = Vec::new();

    fn collect_routines<'t>(node: tree_sitter::Node<'t>, out: &mut Vec<tree_sitter::Node<'t>>) {
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            match child.kind() {
                "procedure" | "trigger_declaration" => out.push(child),
                _ => collect_routines(child, out),
            }
        }
    }
    fn walk_objects<'t>(
        node: tree_sitter::Node<'t>,
        source: &str,
        cols: &Utf16Cols,
        id_ctx: &crate::engine::l2::IdentityCtx,
        out: &mut Vec<(String, crate::engine::l2::features::PFeatures)>,
    ) {
        use crate::engine::l2::scope::{extract_object_globals, object_type_for};
        use crate::engine::l2::{extract_object_number, project_routine_features};
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            if let Some(object_type) = object_type_for(child.kind()) {
                let object_number = extract_object_number(child, source);
                let globals = extract_object_globals(child, id_ctx.source_unit_id, source);
                let mut routines = Vec::new();
                collect_routines(child, &mut routines);
                let mut proc_names = std::collections::HashSet::new();
                for r in &routines {
                    if let Some(nm) = r.child_by_field_name("name") {
                        let mut t = &source[nm.byte_range()];
                        t = t
                            .strip_prefix('"')
                            .and_then(|x| x.strip_suffix('"'))
                            .unwrap_or(t);
                        proc_names.insert(t.to_lowercase());
                    }
                }
                for r in routines {
                    // project_routine_features returns (routine_id_hash, PFeatures);
                    // key on the routine NAME instead (extracted from the node).
                    if let Some((_, feats)) = project_routine_features(
                        child,
                        r,
                        object_type,
                        object_number,
                        None,
                        &proc_names,
                        &globals,
                        id_ctx,
                        source,
                        cols,
                    ) {
                        let rname = r
                            .child_by_field_name("name")
                            .map(|nm| {
                                let t = &source[nm.byte_range()];
                                t.strip_prefix('"')
                                    .and_then(|x| x.strip_suffix('"'))
                                    .unwrap_or(t)
                                    .to_string()
                            })
                            .unwrap_or_default();
                        out.push((rname, feats));
                    }
                }
            } else {
                walk_objects(child, source, cols, id_ctx, out);
            }
        }
    }
    walk_objects(tree.root_node(), source, &cols, &id_ctx, &mut out);
    out
}

/// Legacy callee method/function names: the `CALLS` query (`Foo()` / `Rec.SetRange()`)
/// PLUS parenless call statements (`Modify;` / `Rec.SetRecFilter;`), which parse as a
/// bare identifier/member in statement position (not `call_expression`) but ARE calls.
pub fn legacy_call_methods(source: &str) -> Vec<String> {
    let mut v = capture_texts(
        source,
        language::queries::CALLS,
        &["call.simple", "call.method"],
    );
    let lang = language::language();
    let mut parser = Parser::new();
    if parser.set_language(&lang).is_ok() {
        if let Some(tree) = parser.parse(source, None) {
            walk_parenless_calls(tree.root_node(), source, &mut v);
        }
    }
    v
}

/// Collect parenless call method names. A bare identifier/member is a parenless
/// CALL when it's in STATEMENT position (per-field disambiguation, mirroring legacy):
/// a direct `statement_block` child, or the body/then/else field of a control
/// statement — NOT a condition/bound (expression position).
fn walk_parenless_calls(node: tree_sitter::Node, source: &str, out: &mut Vec<String>) {
    match node.kind() {
        "identifier" | "quoted_identifier" => {
            if is_statement_position(node) {
                out.push(source[node.byte_range()].to_string());
            }
        }
        "member_expression" => {
            if is_statement_position(node) {
                if let Some(m) = node.child_by_field_name("member") {
                    out.push(source[m.byte_range()].to_string());
                }
            }
        }
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        walk_parenless_calls(child, source, out);
    }
}

fn is_statement_position(node: tree_sitter::Node) -> bool {
    let Some(parent) = node.parent() else {
        return false;
    };
    let field_is = |f: &str| {
        parent
            .child_by_field_name(f)
            .map(|n| n.id() == node.id())
            .unwrap_or(false)
    };
    match parent.kind() {
        "statement_block" => true,
        // A parenless no-arg call (`Initialize;`) is the `function` of a `call_statement`
        // (grammar node added for owned-IR debris discrimination). The bare identifier is
        // definitionally a parenless call in this position.
        "call_statement" => field_is("function"),
        "if_statement" => field_is("then_branch") || field_is("else_branch"),
        "while_statement" | "for_statement" | "foreach_statement" | "with_statement"
        | "case_branch" | "case_else_branch" => field_is("body"),
        _ => false,
    }
}

/// Legacy member names for `object.member` expressions **inside routine bodies**
/// (procedure/trigger). The IR models routine bodies, not page-field-source or
/// property expressions, so the comparison is scoped to match.
pub fn legacy_body_member_names(source: &str) -> Vec<String> {
    let lang = language::language();
    let mut parser = Parser::new();
    if parser.set_language(&lang).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source, None) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    walk_members(tree.root_node(), false, source, &mut out);
    out
}

/// The unambiguous statement kinds compared in the histogram stream (excludes
/// `call_expression`, which is ambiguous between statement and expression position).
const STMT_KINDS: &[&str] = &[
    "if_statement",
    "while_statement",
    "repeat_statement",
    "for_statement",
    "foreach_statement",
    "with_statement",
    "case_statement",
    "assignment_statement",
    "exit_statement",
    "break_statement",
    "continue_statement",
    "asserterror_statement",
];

/// Legacy statement-kind multiset inside routine bodies (one entry per statement
/// node). Mirrors the IR `Stmt` arena (which holds only body statements).
pub fn legacy_statement_kinds(source: &str) -> Vec<String> {
    let lang = language::language();
    let mut parser = Parser::new();
    if parser.set_language(&lang).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source, None) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    walk_stmt_kinds(tree.root_node(), false, &mut out);
    out
}

fn walk_stmt_kinds(node: tree_sitter::Node, in_routine: bool, out: &mut Vec<String>) {
    let in_routine = in_routine || matches!(node.kind(), "procedure" | "trigger_declaration");
    if in_routine && STMT_KINDS.contains(&node.kind()) {
        out.push(node.kind().to_string());
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        walk_stmt_kinds(child, in_routine, out);
    }
}

fn walk_members(node: tree_sitter::Node, in_routine: bool, source: &str, out: &mut Vec<String>) {
    let in_routine = in_routine || matches!(node.kind(), "procedure" | "trigger_declaration");
    if in_routine && node.kind() == "member_expression" {
        if let Some(m) = node.child_by_field_name("member") {
            out.push(source[m.byte_range()].to_string());
        }
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        walk_members(child, in_routine, source, out);
    }
}

/// Legacy variable-declaration names (locals + globals; NOT parameters), via a
/// direct query. `variable_declaration` has multiple `name` children (`A, B: T`).
pub fn legacy_variable_names(source: &str) -> Vec<String> {
    capture_texts(source, "(variable_declaration name: (_) @vn)", &["vn"])
}

/// Legacy TEMPORARY variable names: `variable_declaration`s whose `type` subtree
/// contains a `temporary_keyword`.
pub fn legacy_temporary_var_names(source: &str) -> Vec<String> {
    let lang = language::language();
    let mut parser = Parser::new();
    if parser.set_language(&lang).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source, None) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    walk_temp_vars(tree.root_node(), source, &mut out);
    out
}

fn subtree_contains(node: tree_sitter::Node, kind: &str) -> bool {
    if node.kind() == kind {
        return true;
    }
    let mut cursor = node.walk();
    for c in node.named_children(&mut cursor) {
        if subtree_contains(c, kind) {
            return true;
        }
    }
    false
}

fn walk_temp_vars(node: tree_sitter::Node, source: &str, out: &mut Vec<String>) {
    if node.kind() == "variable_declaration" {
        if let Some(t) = node.child_by_field_name("type") {
            if subtree_contains(t, "temporary_keyword") {
                let mut cursor = node.walk();
                for nm in node.children_by_field_name("name", &mut cursor) {
                    out.push(source[nm.byte_range()].to_string());
                }
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        walk_temp_vars(child, source, out);
    }
}

/// Legacy routine names in a source file: every `procedure` / `trigger`
/// definition, via the same `DEFINITIONS` query the engine uses.
pub fn legacy_routine_names(source: &str) -> Vec<String> {
    capture_texts(
        source,
        language::queries::DEFINITIONS,
        &["proc.name", "trigger.name"],
    )
}

/// Run a query and collect the source text of the named captures, in match order.
fn capture_texts(source: &str, query_src: &str, wanted: &[&str]) -> Vec<String> {
    let lang = language::language();
    let mut parser = Parser::new();
    if parser.set_language(&lang).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source, None) else {
        return Vec::new();
    };
    let Ok(query) = Query::new(&lang, query_src) else {
        return Vec::new();
    };
    let names = query.capture_names();
    let mut out = Vec::new();
    let mut cursor = QueryCursor::new();
    let mut it = cursor.matches(&query, tree.root_node(), source.as_bytes());
    while let Some(m) = it.next() {
        for cap in m.captures {
            if wanted.contains(&names[cap.index as usize]) {
                out.push(source[cap.node.byte_range()].to_string());
            }
        }
    }
    out
}
