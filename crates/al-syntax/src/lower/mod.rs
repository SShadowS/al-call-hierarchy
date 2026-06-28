//! CST → IR lowering — the ONLY grammar-aware logic above the raw layer.
//!
//! Phase 1a (this step): outer structure — objects → routines → params / return
//! type / locals / globals. Statement/expression bodies (`RoutineDecl.body`) and
//! `temporary` detection are filled by the next Phase-1 step and validated against
//! the legacy walk under dual-run (spec §5). Unmodelled-but-present nodes are never
//! silently dropped — they surface as `SyntaxIssue` / IR `Unknown`.

use crate::ir::{
    AlFile, BinaryOp, Block, BlockId, BlockItem, CaseBranch, Expr, ExprId, ExprKind, Ir, Literal,
    ObjectDecl, ObjectKind, Origin, Param, ParseStatus, Point, RoutineDecl, RoutineKind, Stmt,
    StmtId, StmtKind, SyntaxIssue, UnaryOp, VarDecl,
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
    AlFile {
        objects,
        ir,
        issues,
        parse_status,
    }
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
    // /action triggers nested in sections, and both #if/#else branches). A dataitem
    // trigger carries its enclosing dataitem's source table (implicit-`Rec` type).
    let mut routine_nodes = Vec::new();
    collect_routines(node, None, None, source, &mut routine_nodes);
    let routines = routine_nodes
        .into_iter()
        .map(|(r, attr_items, di_table, member)| {
            lower_routine(r, attr_items, di_table, member, source, ir, issues)
        })
        .collect();

    // Report dataitems (name, source-table) — a dataitem name is in scope as a record
    // var across all the report's routines. Reports only (empty otherwise).
    let mut report_dataitems = Vec::new();
    if matches!(kind, ObjectKind::Report | ObjectKind::ReportExtension) {
        collect_report_dataitems(node, source, &mut report_dataitems);
    }

    // Extension `extends` target (the grammar's `base_object` field) — None for
    // non-extension objects (no such field).
    let extends_target = node
        .field(FieldName::BaseObject)
        .map(|n| ident_text(n, source))
        .filter(|s| !s.is_empty());

    // `implements` interface names (Codeunit / Enum / Interface).
    let implements = if matches!(
        kind,
        ObjectKind::Codeunit | ObjectKind::Enum | ObjectKind::Interface
    ) {
        extract_implements(node, source)
    } else {
        Vec::new()
    };

    // Page controls (Page / PageExtension).
    let mut page_controls = Vec::new();
    if matches!(kind, ObjectKind::Page | ObjectKind::PageExtension) {
        collect_page_controls(node, source, &mut page_controls);
    }

    // Table fields + keys (Table / TableExtension).
    let mut fields = Vec::new();
    let mut keys = Vec::new();
    if matches!(kind, ObjectKind::Table | ObjectKind::TableExtension) {
        collect_table_fields_keys(node, source, &mut fields, &mut keys);
    }

    // Object globals: var_sections under the declaration_body (not inside routines).
    // Object-level properties (SourceTable / TableNo / PageType / …) are siblings.
    let mut globals = Vec::new();
    let mut properties = Vec::new();
    if let Some(body) = node.field(FieldName::Body) {
        for member in body.named_children() {
            collect_globals(member, source, &mut globals);
            if member.kind() == RawKind::Property {
                if let Some(p) = lower_property(member, source) {
                    properties.push(p);
                }
            }
        }
    }

    ObjectDecl {
        kind,
        id,
        name,
        routines,
        globals,
        properties,
        report_dataitems,
        extends_target,
        implements,
        page_controls,
        fields,
        keys,
        origin: origin_of(node),
    }
}

/// `implements` interface names (unquoted, document order). Mirrors the legacy
/// `extract_implements_interfaces`: names after the `implements` keyword, or the
/// members of an `implements_clause` wrapper.
fn extract_implements(node: RawNode, source: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut saw_implements = false;
    for child in node.named_children() {
        match child.kind() {
            RawKind::ImplementsKeyword => saw_implements = true,
            RawKind::ImplementsClause => {
                for sub in child.named_children() {
                    if matches!(sub.kind(), RawKind::Identifier | RawKind::QuotedIdentifier) {
                        out.push(ident_text(sub, source));
                    }
                }
            }
            RawKind::Identifier | RawKind::QuotedIdentifier if saw_implements => {
                out.push(ident_text(child, source));
            }
            // Stop at the object body / a routine body.
            RawKind::DeclarationBody | RawKind::CodeBlock if saw_implements => break,
            _ => {}
        }
    }
    out
}

/// Collect page `part` / `systempart` / `usercontrol` sections (name, kind, target),
/// document order. Recurses the layout but NOT into routine bodies. Mirrors the legacy
/// `extract_page_controls`.
fn collect_page_controls(node: RawNode, source: &str, out: &mut Vec<crate::ir::PageControl>) {
    let kind = match node.kind() {
        RawKind::PartSection => Some("part"),
        RawKind::SystempartSection => Some("systempart"),
        RawKind::UsercontrolSection => Some("usercontrol"),
        _ => None,
    };
    if let Some(kind) = kind {
        if let (Some(name), Some(src)) =
            (node.field(FieldName::Name), node.field(FieldName::Source))
        {
            out.push(crate::ir::PageControl {
                name: ident_text(name, source),
                kind: kind.to_string(),
                target: ident_text(src, source),
            });
        }
    }
    if node.kind() == RawKind::CodeBlock {
        return;
    }
    for c in node.named_children() {
        collect_page_controls(c, source, out);
    }
}

/// Collect table `field(...)` declarations + `key(...)` member-name lists (prune at
/// match, document order). Mirrors the legacy `index_table` / `classify_field`.
fn collect_table_fields_keys(
    node: RawNode,
    source: &str,
    fields: &mut Vec<crate::ir::FieldDecl>,
    keys: &mut Vec<Vec<String>>,
) {
    for child in node.named_children() {
        match child.kind() {
            RawKind::FieldDeclaration => fields.push(lower_field(child, source)),
            RawKind::KeyDeclaration => {
                let mut members = Vec::new();
                if let Some(list) = child.field(FieldName::Fields) {
                    for m in list.named_children() {
                        if matches!(m.kind(), RawKind::Identifier | RawKind::QuotedIdentifier) {
                            members.push(ident_text(m, source).to_ascii_lowercase());
                        }
                    }
                }
                keys.push(members);
            }
            _ => collect_table_fields_keys(child, source, fields, keys),
        }
    }
}

/// Lower a `field(<no>; <Name>; <Type>) { ... }` declaration.
fn lower_field(node: RawNode, source: &str) -> crate::ir::FieldDecl {
    let number = node
        .field(FieldName::Id)
        .and_then(|n| n.text(source).trim().parse::<i64>().ok())
        .unwrap_or(0);
    let name = node
        .field(FieldName::Name)
        .map(|n| ident_text(n, source))
        .unwrap_or_default();
    let data_type = node
        .field(FieldName::Type)
        .map(|n| n.text(source).trim().to_string())
        .unwrap_or_default();
    // FieldClass property (in the field's declaration_body): FlowField / FlowFilter /
    // Normal. Mirrors classify_field.
    let mut field_class = "Normal".to_string();
    if let Some(body) = node.field(FieldName::Body) {
        for member in body.named_children() {
            if member.kind() != RawKind::Property {
                continue;
            }
            let Some(pname) = member.field(FieldName::Name) else {
                continue;
            };
            if pname.text(source).trim().to_ascii_lowercase() != "fieldclass" {
                continue;
            }
            let v = member
                .field(FieldName::Value)
                .map(|n| n.text(source).to_ascii_lowercase())
                .unwrap_or_default();
            if v.contains("flowfield") {
                field_class = "FlowField".to_string();
            } else if v.contains("flowfilter") {
                field_class = "FlowFilter".to_string();
            }
        }
    }
    let dt_lc = data_type.to_ascii_lowercase();
    let is_blob_like = dt_lc == "blob" || dt_lc == "media" || dt_lc == "mediaset";
    crate::ir::FieldDecl {
        number,
        name,
        data_type,
        field_class,
        is_blob_like,
    }
}

/// Collect every report `dataitem(Name; "Source Table")` (incl. nested) as
/// `(name, source-table)`, both unquoted, document order. Mirrors the legacy
/// `report_dataitem_record_vars`.
fn collect_report_dataitems(node: RawNode, source: &str, out: &mut Vec<(String, String)>) {
    for child in node.named_children() {
        if child.kind() == RawKind::ReportDataitem {
            let name = child
                .field(FieldName::Name)
                .map(|n| ident_text(n, source))
                .unwrap_or_default();
            let table = dataitem_table_name(child, source).unwrap_or_default();
            if !name.is_empty() && !table.is_empty() {
                out.push((name, table));
            }
        }
        // Descend (nested dataitems live under a dataitem's body); routine bodies hold
        // no dataitems so the extra recursion is harmless.
        collect_report_dataitems(child, source, out);
    }
}

/// Lower a `property` node (`name = value`). Name lowercased; value is the raw text
/// of the value field (trimmed). None when the name is missing.
fn lower_property(node: RawNode, source: &str) -> Option<crate::ir::ObjectProperty> {
    let name = node
        .field(FieldName::Name)?
        .text(source)
        .trim()
        .to_ascii_lowercase();
    let value = node
        .field(FieldName::Value)
        .map(|v| v.text(source).trim().to_string())
        .unwrap_or_default();
    Some(crate::ir::ObjectProperty {
        name,
        value,
        origin: origin_of(node),
    })
}

/// DFS collecting `(routine, attribute items, enclosing-dataitem-source-table)`
/// triples. AL has no nested routines, so we do not descend into a routine once found.
/// `attribute_item` nodes are SIBLINGS preceding the routine (grammar v2+); accumulate
/// them and attach to the next routine, resetting on any other node. When the walk
/// crosses into a report `dataitem(Name; "Source Table")`, the (innermost) dataitem's
/// source table is threaded down so a dataitem trigger gets its implicit-`Rec` type.
#[allow(clippy::type_complexity)]
fn collect_routines<'t>(
    node: RawNode<'t>,
    dataitem_table: Option<&str>,
    member: Option<RawNode<'t>>,
    source: &str,
    out: &mut Vec<(
        RawNode<'t>,
        Vec<RawNode<'t>>,
        Option<String>,
        Option<RawNode<'t>>,
    )>,
) {
    let mut pending: Vec<RawNode<'t>> = Vec::new();
    for child in node.named_children() {
        match child.kind() {
            RawKind::AttributeItem => pending.push(child),
            RawKind::Procedure | RawKind::TriggerDeclaration => {
                out.push((
                    child,
                    std::mem::take(&mut pending),
                    dataitem_table.map(str::to_string),
                    member,
                ));
            }
            RawKind::ReportDataitem => {
                pending.clear();
                // The innermost enclosing dataitem wins — including when its own source
                // table is absent/unparseable (→ None, NOT the outer table). Mirrors the
                // legacy `report_dataitem_source_table`, which takes the first (innermost)
                // enclosing dataitem's `table_name?` and stops (never inherits an outer).
                // A dataitem is also a named member → its triggers' enclosing member.
                let inner = dataitem_table_name(child, source);
                collect_routines(child, inner.as_deref(), Some(child), source, out);
            }
            _ => {
                pending.clear();
                // A named, non-object, non-`_body` node becomes the enclosing MEMBER for
                // routines in its subtree (mirrors `enclosing_member_of`: a trigger's
                // parent — stepping up a `_body` wrapper — is the member, unless that is
                // the object). Sections (no `name`) and `_body` wrappers inherit.
                let child_member = if child.field(FieldName::Name).is_some()
                    && object_kind_of(child.kind()).is_none()
                    && !child.kind_str().ends_with("_body")
                {
                    Some(child)
                } else {
                    member
                };
                collect_routines(child, dataitem_table, child_member, source, out);
            }
        }
    }
}

/// The unquoted source-table name of a `report_dataitem(Name; "Source Table")` node
/// (its `table_name` field). `None` if absent.
fn dataitem_table_name(node: RawNode, source: &str) -> Option<String> {
    node.field(FieldName::TableName)
        .map(|n| ident_text(n, source))
        .filter(|s| !s.is_empty())
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

fn lower_routine<'t>(
    node: RawNode<'t>,
    attr_items: Vec<RawNode<'t>>,
    dataitem_source_table: Option<String>,
    member: Option<RawNode<'t>>,
    source: &str,
    ir: &mut Ir,
    issues: &mut Vec<SyntaxIssue>,
) -> RoutineDecl {
    // Enclosing-member capture (E1): a member wrapper with a `name` → its (stripped)
    // name + the wrapper's origin (range). The engine unescapes the name + anchors.
    let enclosing_member = member.and_then(|m| {
        m.field(FieldName::Name)
            .map(|n| (ident_text(n, source), origin_of(m)))
    });
    let kind = if node.kind() == RawKind::TriggerDeclaration {
        RoutineKind::Trigger
    } else {
        RoutineKind::Procedure
    };

    // Attributes: lowercased names (for classify_kind / control-context guards) +
    // the full parsed form (name + raw text + lowered argument exprs).
    let mut attributes: Vec<String> = Vec::new();
    let mut attributes_parsed: Vec<crate::ir::AttributeIr> = Vec::new();
    for item in attr_items {
        let Some(content) = item.field(FieldName::Attribute) else {
            continue;
        };
        let Some(name_node) = content.field(FieldName::Name) else {
            continue;
        };
        let raw_name = name_node.text(source).trim().to_string();
        attributes.push(raw_name.to_ascii_lowercase());
        let mut args = Vec::new();
        if let Some(args_node) = content.field(FieldName::Arguments) {
            for list in args_node.named_children() {
                if list.kind() == RawKind::AttributeArgumentList {
                    for arg in list.named_children() {
                        args.push(lower_expr(arg, ir, issues, source));
                    }
                }
            }
        }
        attributes_parsed.push(crate::ir::AttributeIr {
            name: raw_name,
            raw: item.text(source).to_string(),
            args,
        });
    }
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

    // Access modifier (`local`/`internal`/`protected`); None = public / trigger.
    let access_modifier = node.field(FieldName::Modifier).and_then(|m| {
        match m.text(source).trim().to_ascii_lowercase().as_str() {
            "local" => Some("local".to_string()),
            "internal" => Some("internal".to_string()),
            "protected" => Some("protected".to_string()),
            _ => None,
        }
    });

    // Locals: var_section child(ren) of the routine (+ preproc-wrapped).
    let mut locals = Vec::new();
    for child in node.named_children() {
        collect_globals(child, source, &mut locals);
    }

    let body = node
        .field(FieldName::Body)
        .filter(|b| b.kind() == RawKind::CodeBlock)
        .map(|cb| lower_code_block(cb, ir, issues, source));

    RoutineDecl {
        kind,
        name,
        params,
        return_type,
        locals,
        attributes,
        attributes_parsed,
        access_modifier,
        parse_incomplete: node.has_error(),
        dataitem_source_table,
        enclosing_member,
        body,
        origin: origin_of(node),
    }
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
    Param {
        name,
        by_ref,
        ty,
        origin: origin_of(node),
    }
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
                let type_node = decl.field(FieldName::Type);
                let ty = type_node.map(|n| n.text(source).trim().to_string());
                let temporary = type_node
                    .map(|t| contains_kind(t, RawKind::TemporaryKeyword))
                    .unwrap_or(false);
                let names = decl.children_by_field(FieldName::Name);
                if names.is_empty() {
                    // single unnamed-by-field fallback: skip (no name to record)
                    continue;
                }
                for nm in names {
                    out.push(VarDecl {
                        name: ident_text(nm, source),
                        ty: ty.clone(),
                        temporary,
                        origin: origin_of(decl),
                    });
                }
            }
            _ if is_preproc_wrapper(decl) => {
                for c in decl.named_children() {
                    if c.kind() == RawKind::VariableDeclaration {
                        // shallow: flatten preproc-wrapped declarations (both branches)
                        let type_node = c.field(FieldName::Type);
                        let ty = type_node.map(|n| n.text(source).trim().to_string());
                        let temporary = type_node
                            .map(|t| contains_kind(t, RawKind::TemporaryKeyword))
                            .unwrap_or(false);
                        for nm in c.children_by_field(FieldName::Name) {
                            out.push(VarDecl {
                                name: ident_text(nm, source),
                                ty: ty.clone(),
                                temporary,
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

// ---- body lowering (statements + expressions) ----
//
// First cut: preproc-wrapped statements are FLATTENED in document order (legacy
// recursively descends; the structured-vs-flat choice is settled by Phase 1
// dual-run). Unmodelled nodes become `Unknown` + a `SyntaxIssue` — never dropped.

/// `code_block` → its `statement_block` (v3) → a `Block`.
fn lower_code_block(
    cb: RawNode,
    ir: &mut Ir,
    issues: &mut Vec<SyntaxIssue>,
    source: &str,
) -> BlockId {
    let inner = cb.field(FieldName::Body).unwrap_or(cb);
    lower_stmt_seq(inner, origin_of(cb), ir, issues, source)
}

/// A branch position (then/else/loop body): a `code_block`, a bare `statement_block`,
/// or a single statement. Always normalized to a `Block`.
fn lower_branch(
    node: RawNode,
    ir: &mut Ir,
    issues: &mut Vec<SyntaxIssue>,
    source: &str,
) -> BlockId {
    match node.kind() {
        RawKind::CodeBlock => lower_code_block(node, ir, issues, source),
        RawKind::StatementBlock => lower_stmt_seq(node, origin_of(node), ir, issues, source),
        _ => {
            let mut items = Vec::new();
            lower_block_child(node, ir, issues, source, &mut items);
            ir.add_block(Block {
                items,
                origin: origin_of(node),
            })
        }
    }
}

fn lower_stmt_seq(
    container: RawNode,
    origin: Origin,
    ir: &mut Ir,
    issues: &mut Vec<SyntaxIssue>,
    source: &str,
) -> BlockId {
    let mut items = Vec::new();
    for child in container.named_children() {
        lower_block_child(child, ir, issues, source, &mut items);
    }
    ir.add_block(Block { items, origin })
}

fn lower_block_child(
    node: RawNode,
    ir: &mut Ir,
    issues: &mut Vec<SyntaxIssue>,
    source: &str,
    items: &mut Vec<BlockItem>,
) {
    if is_preproc_wrapper(node) {
        for c in node.named_children() {
            lower_block_child(c, ir, issues, source, items);
        }
        return;
    }
    // Statement-position keyword tokens are named children that leak from certain
    // wrappers: `begin`/`end` from an EMPTY `code_block` (v3 emits no `statement_block`
    // body, so `lower_code_block` falls back to the code_block itself), `else` from a
    // `case_else_branch` (lowered by iterating its direct children), etc. None are
    // statements — skip them, exactly as legacy `block_statements`/CFN `build_block` do.
    // Trivia (`comment` / `multiline_comment` / `pragma`) are named children of a
    // block but are NOT statements — skip them (legacy CFN `build_block` does too).
    // Lowering them as `Unknown` produced phantom "other" CFN nodes.
    if matches!(
        node.kind(),
        RawKind::EmptyStatement
            | RawKind::BeginKeyword
            | RawKind::EndKeyword
            | RawKind::IfKeyword
            | RawKind::ThenKeyword
            | RawKind::ElseKeyword
            | RawKind::CaseKeyword
            | RawKind::OfKeyword
            | RawKind::RepeatKeyword
            | RawKind::UntilKeyword
            | RawKind::WhileKeyword
            | RawKind::ForKeyword
            | RawKind::DoKeyword
            | RawKind::ForeachKeyword
            | RawKind::InKeyword
            | RawKind::Comment
            | RawKind::MultilineComment
            | RawKind::Pragma
    ) {
        return;
    }
    // A bare identifier in statement position is NOT a confident call: it is either
    // ERROR-recovery debris (a token rescued after a syntax error) or a parenless call
    // that omitted its `;` (a semicolon-less final statement). A real parenless call
    // owns its `;` and reduces to `call_statement` (handled in lower_stmt). We do not
    // fabricate a call edge from a bare identifier — that would manufacture spurious
    // `unknown` edges from parse-error debris (the moat-polluting case the grammar's
    // `call_statement` node was added to prevent). The residual (a genuine parenless
    // call written without a trailing `;`) is rare, never produces a FALSE edge, and is
    // still no worse than the legacy walk, which captured no parenless calls at all.
    if matches!(node.kind(), RawKind::Identifier | RawKind::QuotedIdentifier) {
        return;
    }
    // A `call_statement` whose subtree carries a parse error (e.g. tree-sitter
    // synthesized a MISSING `;`) is not a confident call either — skip it.
    if node.kind() == RawKind::CallStatement && node.has_error() {
        return;
    }
    let sid = lower_stmt(node, ir, issues, source);
    items.push(BlockItem::Stmt(sid));
}

fn lower_stmt(node: RawNode, ir: &mut Ir, issues: &mut Vec<SyntaxIssue>, source: &str) -> StmtId {
    // A parenless no-arg call statement (`Initialize;`) — the grammar wraps a bare
    // identifier that owns its `;` in a `call_statement` (distinct from ERROR-recovery
    // debris, which lacks a terminator and stays a raw identifier — never reaching here,
    // see `lower_block_child`). Lower its `function` to a Call with empty args, ANCHORED
    // on the callee identifier (NOT the wrapper) so the call's source anchor
    // (endColumn/syntaxKind) is byte-identical to a parenless call's bare identifier —
    // preserving golden parity with the pre-grammar form.
    if node.kind() == RawKind::CallStatement {
        let callee = node.field(FieldName::Function).unwrap_or(node);
        let function = lower_expr(callee, ir, issues, source);
        let call = ir.add_expr(Expr {
            kind: ExprKind::Call {
                function,
                args: Vec::new(),
            },
            origin: origin_of(callee),
        });
        return ir.add_stmt(Stmt {
            kind: StmtKind::Call(call),
            origin: origin_of(callee),
        });
    }
    let origin = origin_of(node);
    let kind = match node.kind() {
        RawKind::AssignmentStatement => {
            let target = lower_opt_field(node, FieldName::Left, ir, issues, source);
            let value = lower_opt_field(node, FieldName::Right, ir, issues, source);
            StmtKind::Assignment { target, value }
        }
        RawKind::CallExpression => StmtKind::Call(lower_expr(node, ir, issues, source)),
        // Parenless member / subscript call statements (`Rec.Find;`, `X[1];`) parse as a
        // bare member/subscript in statement position (the grammar leaves these UNCHANGED
        // — only a bare identifier became `call_statement`). AL has no field-access
        // statement, so these ARE calls — normalize to a Call with empty args.
        RawKind::MemberExpression | RawKind::SubscriptExpression => {
            let function = lower_expr(node, ir, issues, source);
            let call = ir.add_expr(Expr {
                kind: ExprKind::Call {
                    function,
                    args: Vec::new(),
                },
                origin: origin_of(node),
            });
            StmtKind::Call(call)
        }
        RawKind::IfStatement => StmtKind::If {
            cond: lower_opt_field(node, FieldName::Condition, ir, issues, source),
            then_block: lower_branch_field(node, FieldName::ThenBranch, ir, issues, source),
            else_block: node
                .field(FieldName::ElseBranch)
                .map(|b| lower_branch(b, ir, issues, source)),
        },
        RawKind::WhileStatement => StmtKind::While {
            cond: lower_opt_field(node, FieldName::Condition, ir, issues, source),
            body: lower_branch_field(node, FieldName::Body, ir, issues, source),
        },
        RawKind::RepeatStatement => StmtKind::Repeat {
            body: lower_branch_field(node, FieldName::Body, ir, issues, source),
            until: lower_opt_field(node, FieldName::Condition, ir, issues, source),
        },
        RawKind::ForStatement => {
            let down = node
                .field(FieldName::Direction)
                .map(|d| d.text(source).eq_ignore_ascii_case("downto"))
                .unwrap_or(false);
            StmtKind::For {
                var: lower_opt_field(node, FieldName::Variable, ir, issues, source),
                from: lower_opt_field(node, FieldName::Start, ir, issues, source),
                to: lower_opt_field(node, FieldName::End, ir, issues, source),
                down,
                body: lower_branch_field(node, FieldName::Body, ir, issues, source),
            }
        }
        RawKind::ForeachStatement => StmtKind::Foreach {
            var: lower_opt_field(node, FieldName::Variable, ir, issues, source),
            iterable: lower_opt_field(node, FieldName::Iterable, ir, issues, source),
            body: lower_branch_field(node, FieldName::Body, ir, issues, source),
        },
        RawKind::WithStatement => StmtKind::With {
            receiver: lower_opt_field(node, FieldName::Record, ir, issues, source),
            body: lower_branch_field(node, FieldName::Body, ir, issues, source),
        },
        RawKind::CaseStatement => {
            let scrutinee = lower_opt_field(node, FieldName::Expression, ir, issues, source);
            let (branches, else_block) = lower_case_body(node, ir, issues, source);
            StmtKind::Case {
                scrutinee,
                branches,
                else_block,
            }
        }
        RawKind::AsserterrorStatement => StmtKind::AssertError(lower_branch_field(
            node,
            FieldName::Body,
            ir,
            issues,
            source,
        )),
        RawKind::ExitStatement => StmtKind::Exit(
            node.field(FieldName::ReturnValue)
                .map(|e| lower_expr(e, ir, issues, source)),
        ),
        RawKind::BreakStatement => StmtKind::Break,
        RawKind::ContinueStatement => StmtKind::Continue,
        RawKind::CodeBlock => StmtKind::Block(lower_code_block(node, ir, issues, source)),
        _ => {
            issues.push(SyntaxIssue {
                message: format!("unlowered statement `{}`", node.kind_str()),
                origin: origin.clone(),
            });
            StmtKind::Unknown
        }
    };
    ir.add_stmt(Stmt { kind, origin })
}

/// Lower `case_body` → (branches, else block).
fn lower_case_body(
    case_node: RawNode,
    ir: &mut Ir,
    issues: &mut Vec<SyntaxIssue>,
    source: &str,
) -> (Vec<CaseBranch>, Option<BlockId>) {
    let mut branches = Vec::new();
    let mut else_block = None;
    // Branches live under the `case_body`; each `case_branch` has a `body` field.
    if let Some(body) = case_node.field(FieldName::Body) {
        for child in body.named_children() {
            if child.kind() == RawKind::CaseBranch {
                let patterns = child
                    .children_by_field(FieldName::Pattern)
                    .into_iter()
                    .map(|p| lower_expr(p, ir, issues, source))
                    .collect();
                let body = lower_branch_field(child, FieldName::Body, ir, issues, source);
                branches.push(CaseBranch {
                    patterns,
                    body,
                    origin: origin_of(child),
                });
            }
        }
    }
    // The `case_else_branch` is a DIRECT child of the case_statement (not under
    // case_body) and holds its content as direct children (the `else` keyword + a
    // `code_block` or bare statements; no `body` field). Lower it like a branch body:
    // a SOLE `code_block`/`statement_block` is unwrapped (`lower_branch`) so we don't
    // double-nest a block inside the else block — matching then/loop branches and the
    // legacy CFN, which builds the code_block child directly.
    if let Some(else_node) = case_node
        .named_children()
        .into_iter()
        .find(|c| c.kind() == RawKind::CaseElseBranch)
    {
        let content: Vec<RawNode> = else_node
            .named_children()
            .into_iter()
            .filter(|c| c.kind() != RawKind::ElseKeyword)
            .collect();
        else_block = Some(match content.as_slice() {
            [only] => lower_branch(*only, ir, issues, source),
            _ => lower_stmt_seq(else_node, origin_of(else_node), ir, issues, source),
        });
    }
    (branches, else_block)
}

/// Lower a required-expression field; missing → `Unknown` placeholder (recorded).
fn lower_opt_field(
    node: RawNode,
    f: FieldName,
    ir: &mut Ir,
    issues: &mut Vec<SyntaxIssue>,
    source: &str,
) -> ExprId {
    match node.field(f) {
        Some(e) => lower_expr(e, ir, issues, source),
        None => {
            let origin = origin_of(node);
            issues.push(SyntaxIssue {
                message: format!("missing `{:?}` on `{}`", f, node.kind_str()),
                origin: origin.clone(),
            });
            ir.add_expr(Expr {
                kind: ExprKind::Unknown,
                origin,
            })
        }
    }
}

fn lower_branch_field(
    node: RawNode,
    f: FieldName,
    ir: &mut Ir,
    issues: &mut Vec<SyntaxIssue>,
    source: &str,
) -> BlockId {
    match node.field(f) {
        Some(b) => lower_branch(b, ir, issues, source),
        None => ir.add_block(Block {
            items: Vec::new(),
            origin: origin_of(node),
        }),
    }
}

fn lower_expr(node: RawNode, ir: &mut Ir, issues: &mut Vec<SyntaxIssue>, source: &str) -> ExprId {
    let origin = origin_of(node);
    let kind = match node.kind() {
        RawKind::Identifier | RawKind::KeywordIdentifier => {
            ExprKind::Identifier(node.text(source).to_string())
        }
        RawKind::QuotedIdentifier => ExprKind::QuotedIdentifier(ident_text(node, source)),
        RawKind::MemberExpression => {
            let object = lower_opt_field(node, FieldName::Object, ir, issues, source);
            // RAW member text (quotes preserved) — source-faithful; consumers strip
            // when needed. Legacy keeps quotes for var-assignment lhs names.
            let member_node = node.field(FieldName::Member);
            let member = member_node
                .map(|m| m.text(source).to_string())
                .unwrap_or_default();
            let member_origin = member_node
                .map(origin_of)
                .unwrap_or_else(|| origin_of(node));
            ExprKind::Member {
                object,
                member,
                member_origin,
            }
        }
        RawKind::CallExpression => {
            let function = lower_opt_field(node, FieldName::Function, ir, issues, source);
            let args = node
                .field(FieldName::Arguments)
                .map(|al| {
                    al.named_children()
                        .into_iter()
                        .map(|a| lower_expr(a, ir, issues, source))
                        .collect()
                })
                .unwrap_or_default();
            ExprKind::Call { function, args }
        }
        RawKind::SubscriptExpression => ExprKind::Index {
            base: lower_opt_field(node, FieldName::Object, ir, issues, source),
            index: lower_opt_field(node, FieldName::Index, ir, issues, source),
        },
        RawKind::ParenthesizedExpression => match node.named_children().into_iter().next() {
            Some(inner) => ExprKind::Parenthesized(lower_expr(inner, ir, issues, source)),
            None => ExprKind::Unknown,
        },
        RawKind::UnaryExpression => ExprKind::Unary {
            op: unary_op(node, source),
            operand: lower_opt_field(node, FieldName::Operand, ir, issues, source),
        },
        RawKind::AdditiveExpression
        | RawKind::MultiplicativeExpression
        | RawKind::ComparisonExpression
        | RawKind::LogicalExpression => ExprKind::Binary {
            op: binary_op(node, source),
            lhs: lower_opt_field(node, FieldName::Left, ir, issues, source),
            rhs: lower_opt_field(node, FieldName::Right, ir, issues, source),
        },
        RawKind::RangeExpression => ExprKind::RangeExpr {
            start: lower_opt_field(node, FieldName::Left, ir, issues, source),
            end: lower_opt_field(node, FieldName::Right, ir, issues, source),
        },
        RawKind::QualifiedEnumValue => ExprKind::QualifiedEnum {
            enum_type: lower_opt_field(node, FieldName::EnumType, ir, issues, source),
            value: node
                .field(FieldName::Value)
                .map(|v| ident_text(v, source))
                .unwrap_or_default(),
        },
        RawKind::DatabaseReference => ExprKind::DatabaseReference(node.text(source).to_string()),
        RawKind::Boolean => ExprKind::Literal(Literal::Bool(
            node.text(source).eq_ignore_ascii_case("true"),
        )),
        RawKind::Integer => ExprKind::Literal(Literal::Int(node.text(source).to_string())),
        RawKind::Decimal => ExprKind::Literal(Literal::Decimal(node.text(source).to_string())),
        RawKind::StringLiteral | RawKind::VerbatimString => {
            ExprKind::Literal(Literal::Text(node.text(source).to_string()))
        }
        _ => {
            // Unmodelled expression container (in/is/as expression, list_literal,
            // ternary, …): lower its non-trivia children so nested calls/members are
            // still captured in the arena (completeness). The node itself is Unknown.
            for c in node.named_children() {
                if crate::schema::class_of(c.kind()) != crate::schema::Class::Trivia {
                    lower_expr(c, ir, issues, source);
                }
            }
            ExprKind::Unknown
        }
    };
    ir.add_expr(Expr { kind, origin })
}

fn binary_op(node: RawNode, source: &str) -> BinaryOp {
    let t = node
        .field(FieldName::Operator)
        .map(|o| o.text(source).to_string())
        .unwrap_or_default();
    match t.to_ascii_lowercase().as_str() {
        "+" => BinaryOp::Add,
        "-" => BinaryOp::Sub,
        "*" => BinaryOp::Mul,
        "/" => BinaryOp::Div,
        "div" => BinaryOp::IntDiv,
        "mod" => BinaryOp::Mod,
        "=" => BinaryOp::Eq,
        "<>" => BinaryOp::Ne,
        "<" => BinaryOp::Lt,
        "<=" => BinaryOp::Le,
        ">" => BinaryOp::Gt,
        ">=" => BinaryOp::Ge,
        "and" => BinaryOp::And,
        "or" => BinaryOp::Or,
        "xor" => BinaryOp::Xor,
        "in" => BinaryOp::In,
        _ => BinaryOp::Other,
    }
}

fn unary_op(node: RawNode, source: &str) -> UnaryOp {
    let t = node
        .field(FieldName::Operator)
        .map(|o| o.text(source).to_string())
        .unwrap_or_default();
    match t.to_ascii_lowercase().as_str() {
        "not" => UnaryOp::Not,
        "-" => UnaryOp::Neg,
        _ => UnaryOp::Plus,
    }
}

/// True if `node`'s subtree contains a node of kind `k` (used for `temporary`
/// detection: `temporary_keyword` nested in the variable's `record_type`).
fn contains_kind(node: RawNode, k: RawKind) -> bool {
    node.kind() == k || node.named_children().iter().any(|c| contains_kind(*c, k))
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
        start: Point {
            row: s.row as u32,
            column: s.column as u32,
        },
        end: Point {
            row: e.row as u32,
            column: e.column as u32,
        },
    }
}
