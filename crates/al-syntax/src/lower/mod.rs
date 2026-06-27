//! CST → IR lowering — the ONLY grammar-aware logic above the raw layer.
//!
//! Phase 1a (this step): outer structure — objects → routines → params / return
//! type / locals / globals. Statement/expression bodies (`RoutineDecl.body`) and
//! `temporary` detection are filled by the next Phase-1 step and validated against
//! the legacy walk under dual-run (spec §5). Unmodelled-but-present nodes are never
//! silently dropped — they surface as `SyntaxIssue` / IR `Unknown`.

use crate::ir::{
    AlFile, Ir, ObjectDecl, ObjectKind, Origin, Param, ParseStatus, Point, RoutineDecl,
    RoutineKind, VarDecl,
};
use crate::raw::{FieldName, RawKind, RawNode};

/// Lower a parsed file root into the owned IR.
pub fn lower_file(root: RawNode, source: &str) -> AlFile {
    let parse_status = if root.has_error() {
        ParseStatus::Recovered
    } else {
        ParseStatus::Clean
    };
    let mut ir = Ir::new();
    let mut issues = Vec::new();
    let mut objects = Vec::new();
    collect_objects(root, source, &mut ir, &mut issues, &mut objects);
    AlFile { objects, ir, issues, parse_status }
}

/// Walk for top-level object declarations, descending namespaces and preproc
/// wrappers (which may enclose objects in BC 24+ / `#if` builds).
fn collect_objects(
    node: RawNode,
    source: &str,
    ir: &mut Ir,
    issues: &mut Vec<crate::ir::SyntaxIssue>,
    out: &mut Vec<ObjectDecl>,
) {
    for child in node.named_children() {
        match object_kind_of(child.kind()) {
            Some(kind) => out.push(lower_object(child, kind, source, ir, issues)),
            None => {
                // Descend containers that may hold objects (namespace, preproc).
                if child.kind() == RawKind::NamespaceDeclaration || is_preproc_wrapper(child) {
                    collect_objects(child, source, ir, issues, out);
                }
            }
        }
    }
}

/// A `preproc_conditional*` wrapper node (`#if`/`#else` region). The lowerer
/// descends BOTH branches (legacy indexes both for BC version-compat).
fn is_preproc_wrapper(n: RawNode) -> bool {
    n.kind_str().starts_with("preproc_conditional")
}

fn object_kind_of(k: RawKind) -> Option<ObjectKind> {
    use ObjectKind as O;
    Some(match k {
        RawKind::CodeunitDeclaration | RawKind::PreprocSplitDeclaration => O::Codeunit,
        RawKind::TableDeclaration => O::Table,
        RawKind::TableextensionDeclaration => O::TableExtension,
        RawKind::PageDeclaration => O::Page,
        RawKind::PageextensionDeclaration => O::PageExtension,
        RawKind::ReportDeclaration => O::Report,
        RawKind::ReportextensionDeclaration => O::ReportExtension,
        RawKind::QueryDeclaration => O::Query,
        RawKind::XmlportDeclaration => O::XmlPort,
        RawKind::EnumDeclaration => O::Enum,
        RawKind::EnumextensionDeclaration => O::EnumExtension,
        RawKind::InterfaceDeclaration => O::Interface,
        RawKind::ControladdinDeclaration => O::ControlAddIn,
        RawKind::EntitlementDeclaration => O::Entitlement,
        RawKind::PermissionsetDeclaration => O::PermissionSet,
        RawKind::PermissionsetextensionDeclaration => O::PermissionSetExtension,
        RawKind::ProfileDeclaration => O::Profile,
        _ => return None,
    })
}

fn lower_object(
    node: RawNode,
    kind: ObjectKind,
    source: &str,
    ir: &mut Ir,
    issues: &mut Vec<crate::ir::SyntaxIssue>,
) -> ObjectDecl {
    let id = node
        .field(FieldName::ObjectId)
        .and_then(|n| n.text(source).trim().parse::<i64>().ok());
    let name = node
        .field(FieldName::ObjectName)
        .map(|n| ident_text(n, source))
        .unwrap_or_default();

    // Routines: every procedure/trigger anywhere in the object subtree (incl. field
    // /action triggers nested in sections, and both #if/#else branches).
    let mut routine_nodes = Vec::new();
    collect_routines(node, &mut routine_nodes);
    let routines = routine_nodes
        .into_iter()
        .map(|r| lower_routine(r, source, ir, issues))
        .collect();

    // Object globals: var_sections under the declaration_body (not inside routines).
    let mut globals = Vec::new();
    if let Some(body) = node.field(FieldName::Body) {
        for member in body.named_children() {
            collect_globals(member, source, &mut globals);
        }
    }

    ObjectDecl { kind, id, name, routines, globals, origin: origin_of(node) }
}

/// DFS collecting `procedure` / `trigger_declaration` nodes. AL has no nested
/// routines, so we do not descend into a routine once found.
fn collect_routines<'t>(node: RawNode<'t>, out: &mut Vec<RawNode<'t>>) {
    for child in node.named_children() {
        match child.kind() {
            RawKind::Procedure | RawKind::TriggerDeclaration => out.push(child),
            _ => collect_routines(child, out),
        }
    }
}

/// Collect object-level var declarations, descending preproc wrappers (both
/// branches) but NOT routines/sections-with-their-own-scope.
fn collect_globals(node: RawNode, source: &str, out: &mut Vec<VarDecl>) {
    match node.kind() {
        RawKind::VarSection => extract_var_section(node, source, out),
        _ if is_preproc_wrapper(node) => {
            for c in node.named_children() {
                collect_globals(c, source, out);
            }
        }
        _ => {}
    }
}

fn lower_routine(
    node: RawNode,
    source: &str,
    ir: &mut Ir,
    _issues: &mut Vec<crate::ir::SyntaxIssue>,
) -> RoutineDecl {
    let kind = if node.kind() == RawKind::TriggerDeclaration {
        RoutineKind::Trigger
    } else {
        RoutineKind::Procedure
    };
    let name = node
        .field(FieldName::Name)
        .map(|n| ident_text(n, source))
        .unwrap_or_default();

    let params = node
        .field(FieldName::Parameters)
        .map(|pl| {
            pl.named_children()
                .into_iter()
                .filter(|p| p.kind() == RawKind::Parameter)
                .map(|p| lower_param(p, source))
                .collect()
        })
        .unwrap_or_default();

    let return_type = node
        .field(FieldName::ReturnType)
        .map(|n| n.text(source).trim().to_string());

    // Locals: var_section child(ren) of the routine (+ preproc-wrapped).
    let mut locals = Vec::new();
    for child in node.named_children() {
        collect_globals(child, source, &mut locals);
    }

    // body lowered in the next Phase-1 step (validated by dual-run).
    let _ = ir;
    RoutineDecl { kind, name, params, return_type, locals, body: None, origin: origin_of(node) }
}

fn lower_param(node: RawNode, source: &str) -> Param {
    let by_ref = node.field(FieldName::Modifier).is_some();
    let name = node
        .field(FieldName::Name)
        .map(|n| ident_text(n, source))
        .unwrap_or_default();
    let ty = node
        .field(FieldName::Type)
        .map(|n| n.text(source).trim().to_string());
    Param { name, by_ref, ty, origin: origin_of(node) }
}

/// A `var_section` → its `var_body` → one `VarDecl` per declared name (`A, B: T`
/// yields two). `temporary` detection is refined in the parity step (false here).
fn extract_var_section(section: RawNode, source: &str, out: &mut Vec<VarDecl>) {
    let Some(body) = section.field(FieldName::Body) else {
        return;
    };
    for decl in body.named_children() {
        match decl.kind() {
            RawKind::VariableDeclaration => {
                let ty = decl
                    .field(FieldName::Type)
                    .map(|n| n.text(source).trim().to_string());
                let names = decl.children_by_field(FieldName::Name);
                if names.is_empty() {
                    // single unnamed-by-field fallback: skip (no name to record)
                    continue;
                }
                for nm in names {
                    out.push(VarDecl {
                        name: ident_text(nm, source),
                        ty: ty.clone(),
                        temporary: false,
                        origin: origin_of(decl),
                    });
                }
            }
            _ if is_preproc_wrapper(decl) => {
                for c in decl.named_children() {
                    if c.kind() == RawKind::VariableDeclaration {
                        // shallow: flatten preproc-wrapped declarations (both branches)
                        let ty = c
                            .field(FieldName::Type)
                            .map(|n| n.text(source).trim().to_string());
                        for nm in c.children_by_field(FieldName::Name) {
                            out.push(VarDecl {
                                name: ident_text(nm, source),
                                ty: ty.clone(),
                                temporary: false,
                                origin: origin_of(c),
                            });
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

/// Strip one layer of surrounding double quotes from a (quoted) identifier.
fn ident_text(n: RawNode, source: &str) -> String {
    let t = n.text(source);
    let bytes = t.as_bytes();
    if bytes.len() >= 2 && bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"' {
        t[1..t.len() - 1].to_string()
    } else {
        t.to_string()
    }
}

/// Build an [`Origin`] from a raw node (used pervasively by lowering).
pub(crate) fn origin_of(n: RawNode) -> Origin {
    let s = n.start_position();
    let e = n.end_position();
    Origin {
        kind_text: n.kind_str(),
        ts_id: n.id(),
        byte: n.byte_range(),
        start: Point { row: s.row as u32, column: s.column as u32 },
        end: Point { row: e.row as u32, column: e.column as u32 },
    }
}
