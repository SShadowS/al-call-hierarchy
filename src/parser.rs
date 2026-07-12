//! AL "parsed file" projection over the owned `al-syntax` IR.
//!
//! `parse_file_ir` is the single entry point: it parses via `al_syntax::parse` and
//! projects a [`ParsedFile`] (definitions / calls / variables / events) for the LSP
//! call-graph indexer. No tree-sitter — `al-syntax` is the only crate that links it.

use lsp_types::{Position, Range};

use crate::graph::{DefinitionKind, ObjectType};
// Relocated to `crate::analysis` (T3 Task 12 fix-wave, see that module's
// doc): pure IR/attribute helpers `src/lsp/lens.rs`/`diagnostics.rs` (which
// outlive this module — see the Grammar/Architecture notes on `parser.rs`
// being a Task-17 deletion target) need without depending on a module
// scheduled for deletion. Re-exported here so this module's OWN call sites
// below keep compiling unchanged.
use crate::analysis::for_each_subexpr;
pub(crate) use crate::analysis::{is_framework_invocation_attribute, routine_complexity_ir};
// Relocated to `crate::analysis` (T3 Task 13 review fix-wave, same rationale
// as above): `src/lsp/custom.rs`'s `event_publishers_in_file` needs this SAME
// signature-rendering helper to stay byte-identical to this module's own
// `ParsedEventPublisher::signature`; sharing one definition means the two
// never drift, and `parser.rs`'s own deletion (Task 17) won't orphan it.
use crate::analysis::signature_ir;

/// Parsed definitions and calls from a single file
#[derive(Debug, Default)]
pub struct ParsedFile {
    /// Object type in this file
    pub object_type: Option<ObjectType>,
    /// Object name in this file
    pub object_name: Option<String>,
    /// All procedure/trigger definitions
    pub definitions: Vec<ParsedDefinition>,
    /// All call sites
    pub calls: Vec<ParsedCall>,
    /// All variable declarations
    pub variables: Vec<ParsedVariable>,
    /// All event subscribers
    pub event_subscribers: Vec<ParsedEventSubscriber>,
    /// All event publishers (procedures with [IntegrationEvent]/[BusinessEvent]/[InternalEvent])
    pub event_publishers: Vec<ParsedEventPublisher>,
    /// Names of procedures invoked implicitly by a framework rather than by a
    /// direct call: test methods ([Test]) and test handlers ([ConfirmHandler],
    /// [MessageHandler], ...). Used to suppress unused-procedure diagnostics.
    /// (Event publishers/subscribers are tracked in their own fields.)
    pub implicitly_invoked: Vec<String>,
}

/// A parsed procedure/trigger definition
#[derive(Debug)]
pub struct ParsedDefinition {
    pub name: String,
    pub range: Range,
    pub kind: DefinitionKind,
    /// Cyclomatic complexity (calculated from AST)
    pub complexity: u32,
    /// Parameter count
    pub parameter_count: u32,
}

/// A parsed call site
#[derive(Debug)]
pub struct ParsedCall {
    /// Object being called (None for local calls)
    pub object: Option<String>,
    /// Method/procedure being called
    pub method: String,
    /// Range of the call
    pub range: Range,
    /// Name of the containing procedure (if known)
    pub containing_procedure: Option<String>,
}

/// A parsed variable declaration
#[derive(Debug)]
pub struct ParsedVariable {
    /// Variable name
    pub name: String,
    /// Type name (e.g., "CDO E-Mail Template Line" for Record type)
    pub type_name: String,
    /// Type kind (Record, Codeunit, etc.)
    pub type_kind: Option<String>,
    /// Name of the containing procedure (None for global variables)
    pub containing_procedure: Option<String>,
}

/// A parsed event publisher — a procedure decorated with `[IntegrationEvent]`,
/// `[BusinessEvent]`, or `[InternalEvent]`.
#[derive(Debug, Clone)]
pub struct ParsedEventPublisher {
    /// Name of the published procedure
    pub name: String,
    /// Range of the published procedure (the procedure node, not the attribute)
    pub range: Range,
    /// Range of the procedure's identifier (for selection_range)
    pub selection_range: Range,
    /// Which attribute decorated this procedure
    pub kind: EventPublisherKind,
    /// True if marked `local procedure`
    pub is_local: bool,
    /// Pre-formatted signature, e.g.
    /// `procedure OnAfterPost(var Rec: Record "Sales Header"): Boolean`.
    /// Renders the textual form of the procedure header.
    pub signature: String,
}

/// Event publisher attribute kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// The `Event` suffix is the AL domain term (IntegrationEvent/BusinessEvent/…), not noise.
#[allow(clippy::enum_variant_names)]
pub enum EventPublisherKind {
    IntegrationEvent,
    BusinessEvent,
    InternalEvent,
}

impl EventPublisherKind {
    #[allow(dead_code)] // label helper kept for future consumers
    pub fn tag(&self) -> &'static str {
        match self {
            Self::IntegrationEvent => "[IntegrationEvent]",
            Self::BusinessEvent => "[BusinessEvent]",
            Self::InternalEvent => "[InternalEvent]",
        }
    }
}

/// A parsed event subscriber
#[derive(Debug)]
pub struct ParsedEventSubscriber {
    /// Name of the subscriber procedure
    pub subscriber_name: String,
    /// Range of the subscriber procedure
    pub range: Range,
    /// Publisher object type (e.g., "Codeunit")
    pub publisher_object_type: Option<String>,
    /// Publisher object name (e.g., "Sales-Post")
    pub publisher_object: String,
    /// Publisher event name (e.g., "OnBeforePostSalesDoc")
    pub publisher_event: String,
}

/// Parse EventSubscriber attribute arguments
/// Format: (ObjectType::Codeunit, Codeunit::"Sales-Post", 'OnBeforePostSalesDoc', '', false, false)
fn parse_event_subscriber_args(args: &str) -> Option<(Option<String>, String, String)> {
    // Remove parentheses and split by comma
    let trimmed = args.trim().trim_start_matches('(').trim_end_matches(')');
    let parts: Vec<&str> = trimmed.split(',').map(|s| s.trim()).collect();

    if parts.len() < 3 {
        return None;
    }

    // Parse object type (e.g., "ObjectType::Codeunit")
    let obj_type = if parts[0].contains("::") {
        parts[0].split("::").last().map(|s| s.to_string())
    } else {
        None
    };

    // Parse object name (e.g., "Codeunit::\"Sales-Post\"" or "Database::\"Customer\"")
    let obj_name = extract_object_name(parts[1]);

    // Parse event name (e.g., "'OnBeforePostSalesDoc'" or "\"OnBeforePostSalesDoc\"")
    let event_name = clean_name(parts[2]);

    if obj_name.is_empty() || event_name.is_empty() {
        return None;
    }

    Some((obj_type, obj_name, event_name))
}

/// Extract object name from expressions like "Codeunit::\"Sales-Post\"" or "Database::\"Customer\""
fn extract_object_name(expr: &str) -> String {
    let trimmed = expr.trim();

    // Handle "Type::Name" format
    if let Some(idx) = trimmed.find("::") {
        let after_colons = &trimmed[idx + 2..];
        clean_name(after_colons)
    } else {
        // Just a plain name
        clean_name(trimmed)
    }
}

/// Parse a type specification like "Record \"Customer\"" into (kind, name)
fn parse_type_specification(type_text: &str) -> (Option<String>, String) {
    let trimmed = type_text.trim();

    // Common type patterns: "Record \"Name\"", "Codeunit \"Name\"", etc.
    let type_patterns = [
        "Record",
        "Codeunit",
        "Page",
        "Report",
        "Query",
        "XmlPort",
        "Enum",
        "Interface",
    ];

    for pattern in type_patterns {
        if let Some(rest) = trimmed.strip_prefix(pattern) {
            // Extract the object name after the type keyword
            let rest = rest.trim();
            if let Some(name) = extract_quoted_name(rest) {
                return (Some(pattern.to_string()), name);
            }
        }
    }

    // Not a complex type, just return as-is
    (None, clean_name(trimmed))
}

/// Extract a quoted name like "\"Customer\"" -> "Customer"
fn extract_quoted_name(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if let Some(rest) = trimmed.strip_prefix('"') {
        // Find the closing quote
        if let Some(end) = rest.find('"') {
            return Some(rest[..end].to_string());
        }
    }
    // May not be quoted (e.g., Record Customer without quotes in some cases)
    if !trimmed.is_empty() {
        Some(clean_name(trimmed))
    } else {
        None
    }
}

/// Clean up a name (remove quotes, trim whitespace)
fn clean_name(name: &str) -> String {
    name.trim().trim_matches('"').trim_matches('\'').to_string()
}

// ===========================================================================
// Owned-IR projection (the LSP front-end `ParsedFile`)
//
// `parse_file_ir` builds the `ParsedFile` (definitions / calls / variables /
// events) from the owned `al-syntax` IR (`al_syntax::parse`) — no S-expr queries.
// It began as a byte-identical port of the legacy tree-sitter `AlParser` (proven
// over the r0-corpus, then the queries were deleted) and now carries the
// completeness improvements the IR enables that the old query set could not:
// PARENLESS statement calls (`Initialize;`, `Rec.Find;`) are real call edges, and
// EVERY name of a grouped `A, B: T` declaration becomes its own variable.
// ===========================================================================

use al_syntax::ir::{
    self, AlFile, BlockId, BlockItem, ExprId, ExprKind, ObjectKind, Origin, RoutineDecl,
    RoutineKind, StmtKind, VarDecl,
};

/// Parse + project a `ParsedFile` from the owned AL syntax IR.
pub fn parse_file_ir(source: &str) -> ParsedFile {
    let al: AlFile = al_syntax::parse(source);
    let mut result = ParsedFile::default();

    for obj in &al.objects {
        // object_type / object_name: last object whose kind the legacy query covered
        // wins (kinds the query omits — ReportExtension/Entitlement/Profile — leave the
        // prior value untouched, exactly as a non-matching query would).
        if let Some(ot) = map_object_type(obj.kind) {
            result.object_type = Some(ot);
            result.object_name = Some(clean_name(&obj.name));
        }

        // Object-level globals (containing_procedure = None).
        push_variables_ir(&mut result, &obj.globals, None);

        for r in &obj.routines {
            let rname = clean_name(&r.name);
            let def_kind = match r.kind {
                RoutineKind::Procedure => DefinitionKind::Procedure,
                RoutineKind::Trigger => DefinitionKind::Trigger,
            };
            result.definitions.push(ParsedDefinition {
                name: rname.clone(),
                range: origin_to_range(&r.origin),
                kind: def_kind,
                complexity: routine_complexity_ir(&al.ir, r),
                // Legacy hardcodes 0 parameters for triggers.
                parameter_count: match r.kind {
                    RoutineKind::Trigger => 0,
                    RoutineKind::Procedure => r.params.len() as u32,
                },
            });

            // Locals (containing_procedure = the routine name).
            push_variables_ir(&mut result, &r.locals, Some(rname.clone()));

            // Calls — every `call_expression` reachable in the body (matches the
            // legacy whole-subtree query), recursively through expressions + blocks.
            if let Some(body) = r.body {
                calls_in_block(&al.ir, source, body, &rname, &mut result.calls);
            }

            // Attributes → event subscribers / publishers / framework-invoked.
            project_routine_attributes(&al.ir, source, r, &mut result);
        }
    }

    result
}

/// Map an IR object kind to the front-end `ObjectType`, mirroring exactly which
/// object kinds the legacy DEFINITIONS query captured (no ReportExtension /
/// Entitlement / Profile — those have no query pattern and no `ObjectType` variant).
fn map_object_type(k: ObjectKind) -> Option<ObjectType> {
    use ObjectKind as K;
    Some(match k {
        K::Codeunit => ObjectType::Codeunit,
        K::Table => ObjectType::Table,
        K::Page => ObjectType::Page,
        K::Report => ObjectType::Report,
        K::Query => ObjectType::Query,
        K::XmlPort => ObjectType::XmlPort,
        K::Enum => ObjectType::Enum,
        K::Interface => ObjectType::Interface,
        K::ControlAddIn => ObjectType::ControlAddIn,
        K::PageExtension => ObjectType::PageExtension,
        K::TableExtension => ObjectType::TableExtension,
        K::EnumExtension => ObjectType::EnumExtension,
        K::PermissionSet => ObjectType::PermissionSet,
        K::PermissionSetExtension => ObjectType::PermissionSetExtension,
        K::ReportExtension | K::Entitlement | K::Profile | K::Other => return None,
    })
}

/// Convert an IR `Origin` to an LSP `Range`. `Origin` columns are UTF-8 byte
/// columns within the line — the same convention the legacy `node_range` used
/// (tree-sitter `Point.column`), so positions are byte-identical.
fn origin_to_range(o: &Origin) -> Range {
    Range {
        start: Position {
            line: o.start.row,
            character: o.start.column,
        },
        end: Position {
            line: o.end.row,
            character: o.end.column,
        },
    }
}

/// Project IR variable declarations into `ParsedVariable`s — ONE per declared name.
/// A grouped declaration `A, B: T` yields a `VarDecl` per name in the IR, and we
/// now emit ALL of them (the legacy query captured only the first, leaving `B` an
/// untracked receiver / false unknown — the completeness fast-follow).
fn push_variables_ir(result: &mut ParsedFile, vars: &[VarDecl], containing: Option<String>) {
    for v in vars {
        // A variable needs both a name and a type; untyped declarations are skipped.
        let Some(ty_text) = &v.ty else {
            continue;
        };
        let (type_kind, type_name) = parse_type_specification(ty_text);
        result.variables.push(ParsedVariable {
            name: clean_name(&v.name),
            type_name,
            type_kind,
            containing_procedure: containing.clone(),
        });
    }
}

fn calls_in_block(ir: &ir::Ir, source: &str, bid: BlockId, name: &str, out: &mut Vec<ParsedCall>) {
    for item in &ir.block(bid).items {
        match item {
            BlockItem::Stmt(sid) => calls_in_stmt(ir, source, *sid, name, out),
            BlockItem::Preproc(g) => {
                for b in &g.branches {
                    calls_in_block(ir, source, *b, name, out);
                }
            }
        }
    }
}

fn calls_in_stmt(
    ir: &ir::Ir,
    source: &str,
    sid: ir::StmtId,
    name: &str,
    out: &mut Vec<ParsedCall>,
) {
    match &ir.stmt(sid).kind {
        StmtKind::Assignment { target, value } => {
            calls_in_expr(ir, source, *target, name, out);
            calls_in_expr(ir, source, *value, name, out);
        }
        StmtKind::Call(e) => calls_in_expr(ir, source, *e, name, out),
        StmtKind::If {
            cond,
            then_block,
            else_block,
        } => {
            calls_in_expr(ir, source, *cond, name, out);
            calls_in_block(ir, source, *then_block, name, out);
            if let Some(b) = else_block {
                calls_in_block(ir, source, *b, name, out);
            }
        }
        StmtKind::Case {
            scrutinee,
            branches,
            else_block,
        } => {
            calls_in_expr(ir, source, *scrutinee, name, out);
            for br in branches {
                for p in &br.patterns {
                    calls_in_expr(ir, source, *p, name, out);
                }
                calls_in_block(ir, source, br.body, name, out);
            }
            if let Some(b) = else_block {
                calls_in_block(ir, source, *b, name, out);
            }
        }
        StmtKind::While { cond, body } => {
            calls_in_expr(ir, source, *cond, name, out);
            calls_in_block(ir, source, *body, name, out);
        }
        StmtKind::Repeat { body, until } => {
            calls_in_block(ir, source, *body, name, out);
            calls_in_expr(ir, source, *until, name, out);
        }
        StmtKind::For {
            var,
            from,
            to,
            body,
            ..
        } => {
            calls_in_expr(ir, source, *var, name, out);
            calls_in_expr(ir, source, *from, name, out);
            calls_in_expr(ir, source, *to, name, out);
            calls_in_block(ir, source, *body, name, out);
        }
        StmtKind::Foreach {
            var,
            iterable,
            body,
        } => {
            calls_in_expr(ir, source, *var, name, out);
            calls_in_expr(ir, source, *iterable, name, out);
            calls_in_block(ir, source, *body, name, out);
        }
        StmtKind::With { receiver, body } => {
            calls_in_expr(ir, source, *receiver, name, out);
            calls_in_block(ir, source, *body, name, out);
        }
        StmtKind::Try { body, catch_block } => {
            calls_in_block(ir, source, *body, name, out);
            if let Some(b) = catch_block {
                calls_in_block(ir, source, *b, name, out);
            }
        }
        StmtKind::AssertError(b) => calls_in_block(ir, source, *b, name, out),
        StmtKind::Exit(Some(e)) => calls_in_expr(ir, source, *e, name, out),
        StmtKind::Block(b) => calls_in_block(ir, source, *b, name, out),
        _ => {}
    }
}

fn calls_in_expr(ir: &ir::Ir, source: &str, eid: ExprId, name: &str, out: &mut Vec<ParsedCall>) {
    let expr = ir.expr(eid);
    // Every `ExprKind::Call` is a call edge: a parenthesized `call_expression`
    // (`Foo()`, `Rec.Find('-')`) OR a PARENLESS statement call (`Initialize;`,
    // `Rec.Find;`, `Modify;`) — the lowerer only ever builds `ExprKind::Call` for a
    // genuine call (a `call_statement`/member/subscript in statement position, never
    // for ERROR-recovery debris). Capturing the parenless form is the completeness
    // fast-follow: a procedure invoked only as `MyProc;` is now a real call-hierarchy
    // edge (and no longer mis-flagged as unused). Parenless record builtins
    // (`Modify;`) simply don't resolve to a user procedure — harmless non-edges.
    if let ExprKind::Call { function, .. } = &expr.kind {
        record_call(ir, source, eid, *function, name, out);
    }
    for_each_subexpr(ir, eid, &mut |sub| {
        calls_in_expr(ir, source, sub, name, out)
    });
}

/// Record a call at `call_eid` whose function is `function`, mirroring the legacy
/// CALLS query: only a `function` that is a plain identifier (simple call) or a
/// member expression (`object.method`) is captured; any other function shape
/// (e.g. `Arr[i]()`) matches no query pattern and is skipped. Object/method text
/// is the raw source slice of the relevant node, cleaned — byte-identical to the
/// legacy `node_text(...)` + `clean_name(...)`.
fn record_call(
    ir: &ir::Ir,
    source: &str,
    call_eid: ExprId,
    function: ExprId,
    containing: &str,
    out: &mut Vec<ParsedCall>,
) {
    let fexpr = ir.expr(function);
    let (object, method) = match &fexpr.kind {
        ExprKind::Identifier(_) | ExprKind::QuotedIdentifier(_) => {
            (None, clean_name(&source[fexpr.origin.byte.clone()]))
        }
        ExprKind::Member {
            object,
            member_origin,
            ..
        } => {
            let obj_expr = ir.expr(*object);
            (
                Some(clean_name(&source[obj_expr.origin.byte.clone()])),
                clean_name(&source[member_origin.byte.clone()]),
            )
        }
        _ => return,
    };
    out.push(ParsedCall {
        object,
        method,
        range: origin_to_range(&ir.expr(call_eid).origin),
        containing_procedure: Some(containing.to_string()),
    });
}

/// Classify an attribute name into an event-publisher kind (case-insensitive;
/// real AL attribute names are case-insensitive).
fn publisher_kind_ir(name: &str) -> Option<EventPublisherKind> {
    if name.eq_ignore_ascii_case("IntegrationEvent") {
        Some(EventPublisherKind::IntegrationEvent)
    } else if name.eq_ignore_ascii_case("BusinessEvent") {
        Some(EventPublisherKind::BusinessEvent)
    } else if name.eq_ignore_ascii_case("InternalEvent") {
        Some(EventPublisherKind::InternalEvent)
    } else {
        None
    }
}

/// Project a routine's attributes into event subscribers / publishers and the
/// framework-invoked name list, mirroring the EVENT_SUBSCRIBERS / EVENT_PUBLISHERS
/// / ATTRIBUTED_PROCEDURES queries. The lowerer already attached each attribute to
/// the routine it decorates (the legacy sibling-walk), so no re-resolution is needed.
fn project_routine_attributes(ir: &ir::Ir, source: &str, r: &RoutineDecl, result: &mut ParsedFile) {
    let rname = clean_name(&r.name);
    if rname.is_empty() {
        return;
    }
    for attr in &r.attributes_parsed {
        let aname = attr.name.trim();
        if aname.eq_ignore_ascii_case("EventSubscriber") {
            // Reconstruct the argument text as the source span covering all args
            // and reuse the legacy comma-splitting parser (byte-identical behavior).
            if let (Some(first), Some(last)) = (attr.args.first(), attr.args.last()) {
                let lo = ir.expr(*first).origin.byte.start;
                let hi = ir.expr(*last).origin.byte.end;
                if lo <= hi
                    && hi <= source.len()
                    && let Some((obj_type, obj_name, event_name)) =
                        parse_event_subscriber_args(&source[lo..hi])
                {
                    result.event_subscribers.push(ParsedEventSubscriber {
                        subscriber_name: rname.clone(),
                        range: origin_to_range(&r.origin),
                        publisher_object_type: obj_type,
                        publisher_object: obj_name,
                        publisher_event: event_name,
                    });
                }
            }
        } else if let Some(kind) = publisher_kind_ir(aname) {
            result.event_publishers.push(ParsedEventPublisher {
                name: rname.clone(),
                range: origin_to_range(&r.origin),
                selection_range: origin_to_range(&r.name_origin),
                kind,
                is_local: r.access_modifier.as_deref() == Some("local"),
                signature: signature_ir(source, r),
            });
        }
        // Framework-invocation attributes are a disjoint set (test / *handler) — a
        // routine with N such attributes pushes its name N times, as legacy did.
        if is_framework_invocation_attribute(aname) {
            result.implicitly_invoked.push(rname.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    // Behaviour tests exercise the owned-IR projection (`parse_file_ir`). The legacy
    // tree-sitter `AlParser` it replaced was validated byte-for-byte by a differential
    // over the whole r0-corpus before deletion; the forward regression gate is now the
    // `projection_snapshot_over_r0_corpus` digest golden at the end of this module.

    /// Pure resolution core (Task T0.6): only the exact value `"1"` regenerates;
    /// `"0"`/empty/anything else takes the normal assert path. No env state, so
    /// this is unit-testable without racing `projection_snapshot_over_r0_corpus`
    /// (below), which reads the real env var under real `cargo test` parallelism.
    fn resolve_regen_mode(raw: Option<&str>) -> bool {
        raw == Some("1")
    }

    /// Value-gated `REGEN_TEMP_GOLDENS` check. This is the ONE in-library unit
    /// test that reads the env var, so it cannot share code with the
    /// integration-test helper at `tests/common/regen.rs` (a lib unit test
    /// compiles inside `src/`, which cannot reach across into `tests/` without
    /// an awkward relative `#[path]`); it mirrors that helper's value-semantics
    /// contract instead.
    fn regen_mode() -> bool {
        resolve_regen_mode(std::env::var("REGEN_TEMP_GOLDENS").ok().as_deref())
    }

    #[test]
    fn resolve_regen_mode_true_only_for_exact_one() {
        assert!(resolve_regen_mode(Some("1")));
        assert!(!resolve_regen_mode(Some("0")), "\"0\" must NOT regenerate");
        assert!(!resolve_regen_mode(Some("")));
        assert!(!resolve_regen_mode(None));
    }

    #[test]
    fn test_variable_extraction() {
        let source = r#"
codeunit 50000 "Test Codeunit"
{
    procedure TestProc()
    var
        Customer: Record Customer;
        EMailLine: Record "CDO E-Mail Template Line";
        SalesPost: Codeunit "Sales-Post";
        Counter: Integer;
    begin
        Customer.Get();
        EMailLine.FindTemplate();
        SalesPost.Run();
    end;
}
"#;
        let result = parse_file_ir(source);

        assert!(
            result.variables.len() >= 3,
            "Expected at least 3 variables, got {}",
            result.variables.len()
        );

        let record_vars: Vec<_> = result
            .variables
            .iter()
            .filter(|v| v.type_kind.as_deref() == Some("Record"))
            .collect();
        assert!(
            record_vars.len() >= 2,
            "Expected at least 2 Record variables"
        );

        let email_var = result.variables.iter().find(|v| v.name == "EMailLine");
        assert!(email_var.is_some(), "Should find EMailLine variable");
        let v = email_var.unwrap();
        assert_eq!(v.type_kind.as_deref(), Some("Record"));
        assert_eq!(v.type_name, "CDO E-Mail Template Line");
        assert_eq!(v.containing_procedure.as_deref(), Some("TestProc"));
    }

    #[test]
    fn test_type_specification_parsing() {
        let (kind, name) = parse_type_specification("Record \"Customer\"");
        assert_eq!(kind.as_deref(), Some("Record"));
        assert_eq!(name, "Customer");

        let (kind, name) = parse_type_specification("Codeunit \"Sales-Post\"");
        assert_eq!(kind.as_deref(), Some("Codeunit"));
        assert_eq!(name, "Sales-Post");

        let (kind, name) = parse_type_specification("Integer");
        assert!(kind.is_none());
        assert_eq!(name, "Integer");
    }

    #[test]
    fn test_parse_all_object_types() {
        let test_cases: Vec<(&str, &str)> = vec![
            (
                "table",
                r#"table 50100 "TestTable" { fields { field(1; "Name"; Text[100]) { } } }"#,
            ),
            ("page", r#"page 50100 "TestPage" { }"#),
            ("report", r#"report 50100 "TestReport" { }"#),
            ("query", r#"query 50100 "TestQuery" { }"#),
            ("xmlport", r#"xmlport 50100 "TestXmlPort" { }"#),
            ("enum", r#"enum 50100 "TestEnum" { }"#),
            ("interface", r#"interface "TestInterface" { }"#),
            ("controladdin", r#"controladdin "TestControlAddIn" { }"#),
            (
                "pageextension",
                r#"pageextension 50100 "TestPageExt" extends "Customer Card" { }"#,
            ),
            (
                "tableextension",
                r#"tableextension 50100 "TestTableExt" extends "Customer" { }"#,
            ),
            (
                "enumextension",
                r#"enumextension 50100 "TestEnumExt" extends "TestEnum" { }"#,
            ),
            ("permissionset", r#"permissionset 50100 "TestPermSet" { }"#),
            (
                "permissionsetextension",
                r#"permissionsetextension 50100 "TestPermSetExt" extends "TestPermSet" { }"#,
            ),
        ];

        for (expected_type, source) in test_cases {
            let result = parse_file_ir(source);
            assert!(
                result.object_type.is_some(),
                "Object type should be detected for {expected_type} source: {source}"
            );
            assert_eq!(
                result
                    .object_type
                    .as_ref()
                    .unwrap()
                    .to_string()
                    .to_lowercase(),
                expected_type,
                "Wrong object type for source: {source}"
            );
            assert!(
                result.object_name.is_some(),
                "Object name should be detected for {expected_type} source: {source}"
            );
        }
    }

    #[test]
    fn test_parse_event_subscriber() {
        let al_code = r#"codeunit 50100 "TestSubscriber"
{
    [EventSubscriber(ObjectType::Codeunit, Codeunit::"Sales-Post", 'OnBeforePostSalesDoc', '', false, false)]
    local procedure HandleOnBeforePost(var SalesHeader: Record "Sales Header")
    begin
    end;
}"#;
        let result = parse_file_ir(al_code);

        assert!(
            !result.event_subscribers.is_empty(),
            "Should find event subscriber. Definitions: {:?}",
            result
                .definitions
                .iter()
                .map(|d| &d.name)
                .collect::<Vec<_>>()
        );
        let sub = &result.event_subscribers[0];
        assert_eq!(sub.subscriber_name, "HandleOnBeforePost");
        assert_eq!(sub.publisher_object_type.as_deref(), Some("Codeunit"));
        assert_eq!(sub.publisher_event, "OnBeforePostSalesDoc");
    }

    #[test]
    fn test_parse_variable_types() {
        let al_code = r#"codeunit 50100 "TestVars"
{
    procedure VarProc()
    var
        CustomerRec: Record Customer;
        SalesPost: Codeunit "Sales-Post";
        Counter: Integer;
    begin
    end;
}"#;
        let result = parse_file_ir(al_code);

        assert!(
            result.variables.len() >= 2,
            "Should find at least 2 typed variables, got {}: {:?}",
            result.variables.len(),
            result.variables
        );
        assert!(
            result
                .variables
                .iter()
                .any(|v| v.type_kind.as_deref() == Some("Record")),
            "Should find Record variables. All vars: {:?}",
            result.variables
        );
        assert!(
            result
                .variables
                .iter()
                .any(|v| v.type_kind.as_deref() == Some("Codeunit")),
            "Should find Codeunit variables. All vars: {:?}",
            result.variables
        );
    }

    #[test]
    fn test_parse_calls_with_containing_procedure() {
        let al_code = r#"codeunit 50100 "TestCalls"
{
    procedure CallerProc()
    begin
        HelperProc();
    end;

    procedure HelperProc()
    begin
    end;
}"#;
        let result = parse_file_ir(al_code);

        let helper_calls: Vec<_> = result
            .calls
            .iter()
            .filter(|c| c.method == "HelperProc")
            .collect();
        assert!(
            !helper_calls.is_empty(),
            "Should find call to HelperProc. All calls: {:?}",
            result.calls
        );
        assert_eq!(
            helper_calls[0].containing_procedure.as_deref(),
            Some("CallerProc"),
            "Call should be inside CallerProc"
        );
    }

    #[test]
    fn test_parenless_statement_calls_are_captured() {
        // Parenless calls (`Initialize;`, `Rec.Find;`) are real call edges — the old
        // `call_expression`-only query missed them.
        let al_code = r#"codeunit 50100 "TestParenless"
{
    procedure CallerProc(var Cust: Record Customer)
    begin
        Initialize;
        Cust.Find;
        HelperProc();
    end;

    procedure Initialize()
    begin
    end;

    procedure HelperProc()
    begin
    end;
}"#;
        let result = parse_file_ir(al_code);
        let methods: Vec<&str> = result.calls.iter().map(|c| c.method.as_str()).collect();
        assert!(
            methods.contains(&"Initialize"),
            "parenless `Initialize;` should be a call. calls: {methods:?}"
        );
        assert!(
            methods.contains(&"Find"),
            "parenless `Cust.Find;` should be a call. calls: {methods:?}"
        );
        // The parenless member call carries its receiver object.
        let find = result.calls.iter().find(|c| c.method == "Find").unwrap();
        assert_eq!(find.object.as_deref(), Some("Cust"));
        assert_eq!(find.containing_procedure.as_deref(), Some("CallerProc"));
    }

    #[test]
    fn test_grouped_var_declaration_yields_all_names() {
        // `A, B: T` declares BOTH A and B — the old query captured only the first.
        let al_code = r#"codeunit 50100 "TestGroupedVars"
{
    procedure P()
    var
        Cust, Vend: Record Customer;
        "Quoted A", "Quoted B": Codeunit "Sales-Post";
    begin
    end;
}"#;
        let result = parse_file_ir(al_code);
        let names: Vec<&str> = result.variables.iter().map(|v| v.name.as_str()).collect();
        for want in ["Cust", "Vend", "Quoted A", "Quoted B"] {
            assert!(
                names.contains(&want),
                "grouped declaration should yield `{want}`. got: {names:?}"
            );
        }
        // Trailing grouped names keep the group's type.
        let vend = result.variables.iter().find(|v| v.name == "Vend").unwrap();
        assert_eq!(vend.type_kind.as_deref(), Some("Record"));
        assert_eq!(vend.type_name, "Customer");
    }

    #[test]
    fn test_parse_procedure_parameters() {
        let al_code = r#"codeunit 50100 "TestParams"
{
    procedure NoParams()
    begin
    end;

    procedure TwoParams(First: Integer; Second: Text)
    begin
    end;

    procedure FiveParams(A: Integer; B: Text; C: Boolean; D: Decimal; E: Code[20])
    begin
    end;
}"#;
        let result = parse_file_ir(al_code);

        assert_eq!(result.definitions.len(), 3, "Should find 3 procedures");
        let by = |n: &str| result.definitions.iter().find(|d| d.name == n).unwrap();
        assert_eq!(by("NoParams").parameter_count, 0);
        assert_eq!(by("TwoParams").parameter_count, 2);
        assert_eq!(by("FiveParams").parameter_count, 5);
    }

    #[test]
    fn test_event_publisher_extraction() {
        let source = r#"
codeunit 50100 "Sample Publisher"
{
    [IntegrationEvent(false, false)]
    procedure OnAfterDoSomething(var Rec: Record "Customer"; xRec: Record "Customer")
    begin
    end;

    [BusinessEvent(false)]
    local procedure OnBusinessThing(Amount: Decimal): Boolean
    begin
    end;

    procedure NormalProc()
    begin
    end;

    [Obsolete('Use OnAfterDoSomethingV2', '24.0')]
    [IntegrationEvent(false, false)]
    procedure OnLegacyThing()
    begin
    end;
}
"#;
        let result = parse_file_ir(source);

        assert_eq!(
            result.event_publishers.len(),
            3,
            "expected 3, got {:#?}",
            result.event_publishers
        );

        let p0 = &result.event_publishers[0];
        assert_eq!(p0.name, "OnAfterDoSomething");
        assert_eq!(p0.kind, EventPublisherKind::IntegrationEvent);
        assert!(!p0.is_local);
        assert!(p0.signature.contains("OnAfterDoSomething"));
        assert!(p0.signature.contains("Record"));

        let p1 = &result.event_publishers[1];
        assert_eq!(p1.name, "OnBusinessThing");
        assert_eq!(p1.kind, EventPublisherKind::BusinessEvent);
        assert!(p1.is_local, "OnBusinessThing should be detected as local");
        assert!(p1.signature.contains("Decimal"));

        let p2 = &result.event_publishers[2];
        assert_eq!(p2.name, "OnLegacyThing");
        assert_eq!(p2.kind, EventPublisherKind::IntegrationEvent);
    }

    fn parse_real_bc(path: &str) -> Option<ParsedFile> {
        let p = Path::new(path);
        if !p.exists() {
            eprintln!("Skipping test: BC.History not available ({path})");
            return None;
        }
        let source = std::fs::read_to_string(p).expect("Failed to read file");
        Some(parse_file_ir(&source))
    }

    #[test]
    fn test_parse_real_bc_codeunit() {
        let Some(result) = parse_real_bc(
            "U:/Git/BC.History/BaseApp/Source/Base Application/Sales/Posting/SalesPost.Codeunit.al",
        ) else {
            return;
        };
        assert!(result.object_type.is_some(), "Should detect object type");
        assert!(result.object_name.is_some(), "Should extract object name");
        assert!(
            !result.definitions.is_empty(),
            "Real codeunit should have procedures"
        );
        assert!(
            !result.calls.is_empty(),
            "Real codeunit should have call sites"
        );
        assert!(
            !result.variables.is_empty(),
            "Real codeunit should have variables"
        );
    }

    #[test]
    fn test_parse_real_bc_table() {
        let Some(result) = parse_real_bc(
            "U:/Git/BC.History/BaseApp/Source/Base Application/Sales/Customer/Customer.Table.al",
        ) else {
            return;
        };
        assert!(result.object_type.is_some());
        assert!(result.object_name.is_some());
        let triggers: Vec<_> = result
            .definitions
            .iter()
            .filter(|d| d.kind == DefinitionKind::Trigger)
            .collect();
        assert!(!triggers.is_empty(), "Table should have triggers");
    }

    // ----------------------------------------------------------------------
    // Forward regression gate: a digest snapshot of the owned-IR projection over
    // the whole in-repo r0-corpus. Replaces the AlParser differential (the legacy
    // oracle is deleted). Each line is `<relpath>\t<fnv1a-hex>` of the normalized
    // ParsedFile. Regenerate intentional changes with `REGEN_TEMP_GOLDENS=1`.
    // ----------------------------------------------------------------------

    fn collect_al_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                collect_al_files(&p, out);
            } else if p.extension().and_then(|s| s.to_str()) == Some("al") {
                out.push(p);
            }
        }
    }

    fn fnv1a(s: &str) -> u64 {
        let mut h: u64 = 0xcbf29ce484222325;
        for b in s.bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x00000100000001b3);
        }
        h
    }

    /// Stable, order-insensitive textual rendering of a ParsedFile for digesting.
    fn render(pf: &ParsedFile) -> String {
        let mut parts: Vec<String> = Vec::new();
        parts.push(format!(
            "object\t{:?}\t{:?}",
            pf.object_type, pf.object_name
        ));
        let mut push_sorted = |label: &str, mut v: Vec<String>| {
            v.sort();
            for x in v {
                parts.push(format!("{label}\t{x}"));
            }
        };
        push_sorted(
            "def",
            pf.definitions
                .iter()
                .map(|d| {
                    format!(
                        "{}|{:?}|{:?}|{}|{}",
                        d.name, d.range, d.kind, d.complexity, d.parameter_count
                    )
                })
                .collect(),
        );
        push_sorted(
            "call",
            pf.calls
                .iter()
                .map(|c| {
                    format!(
                        "{:?}|{}|{:?}|{:?}",
                        c.object, c.method, c.range, c.containing_procedure
                    )
                })
                .collect(),
        );
        push_sorted(
            "var",
            pf.variables
                .iter()
                .map(|v| {
                    format!(
                        "{}|{:?}|{}|{:?}",
                        v.name, v.type_kind, v.type_name, v.containing_procedure
                    )
                })
                .collect(),
        );
        push_sorted(
            "sub",
            pf.event_subscribers
                .iter()
                .map(|s| {
                    format!(
                        "{}|{:?}|{:?}|{}|{}",
                        s.subscriber_name,
                        s.range,
                        s.publisher_object_type,
                        s.publisher_object,
                        s.publisher_event
                    )
                })
                .collect(),
        );
        push_sorted(
            "pub",
            pf.event_publishers
                .iter()
                .map(|p| {
                    format!(
                        "{}|{:?}|{:?}|{:?}|{}|{}",
                        p.name, p.range, p.selection_range, p.kind, p.is_local, p.signature
                    )
                })
                .collect(),
        );
        push_sorted("impl", pf.implicitly_invoked.clone());
        parts.join("\n")
    }

    #[test]
    fn projection_snapshot_over_r0_corpus() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let corpus = root.join("tests").join("r0-corpus");
        let golden = root
            .join("tests")
            .join("parser-ir-goldens")
            .join("projection.snapshot");

        let mut files = Vec::new();
        collect_al_files(&corpus, &mut files);
        assert!(
            files.len() > 100,
            "expected the r0-corpus to have many .al files, found {}",
            files.len()
        );
        files.sort();

        let mut out = String::new();
        for path in &files {
            let Ok(source) = std::fs::read_to_string(path) else {
                continue;
            };
            let rel = path.strip_prefix(&corpus).unwrap_or(path);
            let digest = fnv1a(&render(&parse_file_ir(&source)));
            // Forward-slash the relpath so the golden is OS-independent.
            let rel = rel.to_string_lossy().replace('\\', "/");
            out.push_str(&format!("{rel}\t{digest:016x}\n"));
        }

        if regen_mode() {
            std::fs::create_dir_all(golden.parent().unwrap()).unwrap();
            std::fs::write(&golden, &out).unwrap();
            return;
        }

        let expected = std::fs::read_to_string(&golden).unwrap_or_else(|_| {
            panic!(
                "missing golden {}; regenerate with REGEN_TEMP_GOLDENS=1",
                golden.display()
            )
        });
        // Normalize EOLs so a CRLF checkout doesn't spuriously fail.
        if expected.replace("\r\n", "\n") != out.replace("\r\n", "\n") {
            let exp: Vec<&str> = expected.lines().collect();
            let act: Vec<&str> = out.lines().collect();
            let mut diffs = Vec::new();
            for (i, a) in act.iter().enumerate() {
                if exp.get(i) != Some(a) {
                    diffs.push(format!(
                        "  line {}: golden={:?} actual={:?}",
                        i + 1,
                        exp.get(i),
                        a
                    ));
                }
            }
            panic!(
                "parse_file_ir projection drifted from the golden on {} line(s):\n{}\n(regenerate with REGEN_TEMP_GOLDENS=1 if intended)",
                diffs.len(),
                diffs.into_iter().take(30).collect::<Vec<_>>().join("\n")
            );
        }
    }
}
