//! Dual-run parity support (owned-syntax-IR migration). Exposes the LEGACY
//! tree-sitter extraction the engine relies on, reachable from integration tests,
//! so the IR lowerer can be diffed against it. Removed at the Phase 5 seal.

use crate::language;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Parser, Query, QueryCursor};

/// Legacy callee method/function names in a source file, via the engine's `CALLS`
/// query: `@call.simple` (`Foo()`) + `@call.method` (`Rec.SetRange()`).
pub fn legacy_call_methods(source: &str) -> Vec<String> {
    capture_texts(source, language::queries::CALLS, &["call.simple", "call.method"])
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
    let in_routine =
        in_routine || matches!(node.kind(), "procedure" | "trigger_declaration");
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
    capture_texts(source, language::queries::DEFINITIONS, &["proc.name", "trigger.name"])
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
