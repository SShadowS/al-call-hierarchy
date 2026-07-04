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
///
/// **Union-read semantics, stated honestly (Task 3, preproc foundations
/// plan).** Every collector that calls this (objects via `collect_objects`,
/// routines via `collect_routines`, globals/locals via `collect_globals`,
/// statements via `lower_block_child`, object properties via
/// `collect_properties`, `implements` via `extract_implements`) treats `#if`
/// as TRANSPARENT: it unions the content of every branch — including a dead
/// `#if UNDEFINED_SYMBOL .. #endif` branch that would never compile in for
/// ANY real build — into one flat IR. This is a deliberate SUPERSET
/// over-approximation:
/// - **Sound for absence proofs** — if a member is absent from the union, it
///   is absent from every possible build, so "not found anywhere in the
///   union" is a valid non-existence witness.
/// - **NOT sound for resolution CONFIDENCE** — a resolved call route may
///   target dead-branch code that never actually compiles into the running
///   app. Nothing downstream should read "the engine resolved this call" as
///   proof the target is reachable in a specific build without also
///   accounting for `#if` conditionality.
/// - A singular per-branch VALUE (an object property like `SourceTable`) can
///   therefore disagree across branches after this union-read — the
///   consuming layer (`crate::program`) must degrade a genuine disagreement
///   rather than pick one (see `collect_properties`'s doc); a purely additive
///   list-valued union (`implements`) needs no such degrade (see
///   `extract_implements`'s doc).
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
    // `dataset_ctx` starts `false` (Task 1, dataitem-receivers plan) — it only ever
    // becomes `true` while descending a report/report-extension `dataset` section, and
    // is force-reset to `false` on entering `requestpage` (REQUESTPAGE ISOLATION).
    let mut routine_nodes = Vec::new();
    collect_routines(node, None, None, false, source, &mut routine_nodes);
    let routines = routine_nodes
        .into_iter()
        .map(
            |(r, attr_items, di_table, member, in_dataset_modify_context)| {
                lower_routine(
                    r,
                    attr_items,
                    di_table,
                    member,
                    in_dataset_modify_context,
                    source,
                    ir,
                    issues,
                )
            },
        )
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
    // Both collectors descend preproc wrappers (both `#if`/`#else` branches) —
    // `collect_properties` mirrors `collect_globals`'s established pattern (Task
    // 3, preproc foundations plan: a `#if`-wrapped property was previously
    // silently dropped by a flat `body.named_children()` scan; see that
    // function's doc for the union-read + program-layer-degrade contract).
    let mut globals = Vec::new();
    let mut properties = Vec::new();
    if let Some(body) = node.field(FieldName::Body) {
        for member in body.named_children() {
            collect_globals(member, source, &mut globals);
            collect_properties(member, source, &mut properties);
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
///
/// Descends preproc wrappers (Task 3, preproc foundations plan) — defensive
/// rather than fixing a live gap: the only grammar-reachable `#if`-conditional
/// `implements` shape today is `preproc_split_declaration` (`#if COND
/// codeunit 1 X implements A #else codeunit 1 X implements B #endif { .. }`),
/// which the grammar itself flattens — both branches' `_object_header`s
/// (hence both `implements_clause` nodes) are already direct siblings of
/// `node` with no wrapper in between, so the ORIGINAL flat loop already found
/// both unaided (verified: `implements_clause` is only ever reachable from
/// `_object_header`, which is never a `_body_element` — a generic
/// `preproc_conditional` cannot wrap it). The descend below keeps this walk
/// consistent with every other collector (`collect_globals`/
/// `collect_properties`/`lower_block_child`) and future-proofs against a
/// grammar evolution that wraps a body-scoped conditional interface clause.
///
/// A conflicting union (a different interface name per `#if` branch) is
/// captured as-is — BOTH names land in the result — and is intentionally
/// NEVER degraded at the program layer, unlike a singular property
/// (`SourceTable`/`TableNo`): every consumer of `ObjectDecl.implements`
/// (`ResolveIndex`'s interface-implementer index, `interface_route_
/// applicable`) only ever asks "does this object POSSIBLY implement
/// `iface`?" for ADDITIVE may-fire fan-out — never "pick the one interface
/// this object implements". Including an object under both of its
/// conditional interfaces over-approximates the implementer set (one branch
/// is always dead at compile time) but never fabricates a false SINGLE-
/// target confidence, so the union is sound without a degrade.
fn extract_implements(node: RawNode, source: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut saw_implements = false;
    extract_implements_walk(node, source, &mut saw_implements, &mut out);
    out
}

/// Recursive walk backing [`extract_implements`]. Returns `true` once a
/// declaration/code body boundary was crossed (propagated up so every
/// enclosing call also stops — mirrors the original flat loop's `break`).
fn extract_implements_walk(
    node: RawNode,
    source: &str,
    saw_implements: &mut bool,
    out: &mut Vec<String>,
) -> bool {
    for child in node.named_children() {
        match child.kind() {
            RawKind::ImplementsKeyword => *saw_implements = true,
            RawKind::ImplementsClause => {
                for sub in child.named_children() {
                    if matches!(sub.kind(), RawKind::Identifier | RawKind::QuotedIdentifier) {
                        out.push(ident_text(sub, source));
                    }
                }
            }
            RawKind::Identifier | RawKind::QuotedIdentifier if *saw_implements => {
                out.push(ident_text(child, source));
            }
            // Stop at the object body / a routine body.
            RawKind::DeclarationBody | RawKind::CodeBlock if *saw_implements => return true,
            _ if is_preproc_wrapper(child)
                && extract_implements_walk(child, source, saw_implements, out) =>
            {
                return true;
            }
            _ => {}
        }
    }
    false
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
    if let Some(kind) = kind
        && let (Some(name), Some(src)) =
            (node.field(FieldName::Name), node.field(FieldName::Source))
    {
        out.push(crate::ir::PageControl {
            name: ident_text(name, source),
            kind: kind.to_string(),
            target: ident_text(src, source),
        });
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
            if !pname.text(source).trim().eq_ignore_ascii_case("fieldclass") {
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

/// Collect object-level `property` declarations, descending preproc wrappers
/// (both `#if`/`#else` branches) — mirrors [`collect_globals`]'s established
/// pattern. Union-read: a `#if`-wrapped singular property (e.g. `SourceTable`,
/// `TableNo`) now surfaces EVERY syntactically-present value, one per branch —
/// this lowering layer never resolves the `#if` condition, it only reports
/// what is textually there (same superset semantics as objects/routines/
/// globals; see [`is_preproc_wrapper`]'s module-level doc). A singular
/// property with two DIFFERING branch values is therefore ambiguous at this
/// layer by construction; [`crate::program`] (the consumer) is responsible
/// for degrading such a conflict rather than silently picking one (first/
/// last-wins is the cardinal sin this fix exists to prevent) — see
/// `node_extract::singular_property_value`.
fn collect_properties(node: RawNode, source: &str, out: &mut Vec<crate::ir::ObjectProperty>) {
    match node.kind() {
        RawKind::Property => {
            if let Some(p) = lower_property(node, source) {
                out.push(p);
            }
        }
        _ if is_preproc_wrapper(node) => {
            for c in node.named_children() {
                collect_properties(c, source, out);
            }
        }
        _ => {}
    }
}

/// DFS collecting `(routine, attribute items, enclosing-dataitem-source-table,
/// enclosing-member, in-dataset-modify-context)` 5-tuples. AL has no nested routines, so
/// we do not descend into a routine once found. `attribute_item` nodes are SIBLINGS
/// preceding the routine (grammar v2+); accumulate them and attach to the next routine,
/// resetting on any other node. When the walk crosses into a report `dataitem(Name;
/// "Source Table")`, the (innermost) dataitem's source table is threaded down so a
/// dataitem trigger gets its implicit-`Rec` type.
///
/// `dataset_ctx` (Task 1, dataitem-receivers plan) tracks whether the walk is currently
/// inside a report/report-extension `dataset` section: `true` on entering
/// `dataset_section`/`report_dataitem`, force-reset to `false` on entering
/// `requestpage_section` (REQUESTPAGE ISOLATION — binding, never leaks a dataset
/// `modify()`'s context into a requestpage trigger), inherited unchanged otherwise. Used
/// only to compute `in_dataset_modify_context` at push time — see that tuple field's doc
/// and [`crate::ir::RoutineDecl::in_dataset_modify_context`].
#[allow(clippy::type_complexity)]
fn collect_routines<'t>(
    node: RawNode<'t>,
    dataitem_table: Option<&str>,
    member: Option<RawNode<'t>>,
    dataset_ctx: bool,
    source: &str,
    out: &mut Vec<(
        RawNode<'t>,
        Vec<RawNode<'t>>,
        Option<String>,
        Option<RawNode<'t>>,
        bool,
    )>,
) {
    let mut pending: Vec<RawNode<'t>> = Vec::new();
    for child in node.named_children() {
        match child.kind() {
            RawKind::AttributeItem => pending.push(child),
            RawKind::Procedure | RawKind::TriggerDeclaration | RawKind::InterfaceProcedure => {
                // `interface_procedure` (receiver-closure plan, Task 1): a SIGNATURE-ONLY
                // procedure declaration — no body, no access modifier — used by BOTH
                // `interface_body` and `controladdin_body` (the same grammar rule; a
                // controladdin's AL-callable procedures are declared this way, e.g.
                // `procedure InitEditor(x: Text)` with no trailing `begin/end` or even a
                // semicolon). Previously UNHANDLED here — silently invisible to
                // `RoutineDecl`/`RoutineNode` extraction (fell into the `_` catch-all
                // below, which never emits a routine) — meaning a controladdin's declared
                // procedure surface could never be checked at all. `lower_routine` already
                // degrades gracefully for the fields this node lacks: `FieldName::Modifier`
                // absent → `access_modifier: None`; `FieldName::Body` absent (or not a
                // `CodeBlock`) → `body: None`. An `event` declaration inside the SAME
                // body is a DISTINCT grammar node (`RawKind::EventDeclaration`) that this
                // match still does not handle — it falls to the `_` catch-all and stays
                // correctly unrepresented as a `RoutineDecl` (events are never
                // AL-callable, so the gate's "declared procedures" set must never
                // include them).
                //
                // `in_dataset_modify_context` is only ever meaningful when the enclosing
                // member is itself a `modify_modification` (an actual `dataitem(...)`
                // block already threads its own table via `dataitem_table` above — this
                // flag exists purely for the resolve-time fallback that case doesn't
                // need). See `RoutineDecl::in_dataset_modify_context`'s doc.
                let in_dataset_modify_context =
                    dataset_ctx && member.is_some_and(|m| m.kind() == RawKind::ModifyModification);
                out.push((
                    child,
                    std::mem::take(&mut pending),
                    dataitem_table.map(str::to_string),
                    member,
                    in_dataset_modify_context,
                ));
            }
            RawKind::ReportDataitem => {
                pending.clear();
                // The innermost enclosing dataitem wins — including when its own source
                // table is absent/unparseable (→ None, NOT the outer table). Mirrors the
                // legacy `report_dataitem_source_table`, which takes the first (innermost)
                // enclosing dataitem's `table_name?` and stops (never inherits an outer).
                // A dataitem is also a named member → its triggers' enclosing member.
                // Being inside an actual dataitem is inherently dataset context (`true`
                // unconditionally — defensive, in practice already `true` by construction:
                // `report_dataitem` only appears under `dataset_section`/`report_body`).
                let inner = dataitem_table_name(child, source);
                collect_routines(child, inner.as_deref(), Some(child), true, source, out);
            }
            RawKind::DatasetSection => {
                pending.clear();
                collect_routines(child, dataitem_table, member, true, source, out);
            }
            RawKind::RequestpageSection => {
                // REQUESTPAGE ISOLATION (binding, dataitem-receivers plan round-1
                // addendum): force dataset context OFF regardless of the ambient value —
                // a requestpage trigger (or a requestpage/layout `modify()`) must NEVER
                // be treated as report-dataset context, even under a pathological/
                // decompiled nesting the real AL compiler would never accept ("parse
                // structure, don't validate").
                pending.clear();
                collect_routines(child, dataitem_table, member, false, source, out);
            }
            RawKind::ModifyModification => {
                pending.clear();
                // `modify_modification` carries the modified member's name in its
                // `target` field, not `name` — always treated as a named member wrapper
                // (unlike the generic gate below, no `Name`-field probe needed). See
                // `lower_routine`'s enclosing-member fallback for the name extraction.
                collect_routines(child, dataitem_table, Some(child), dataset_ctx, source, out);
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
                collect_routines(
                    child,
                    dataitem_table,
                    child_member,
                    dataset_ctx,
                    source,
                    out,
                );
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

#[allow(clippy::too_many_arguments)] // 7 pre-existing params + `in_dataset_modify_context`
// (dataitem-receivers plan, Task 1); each is a distinct piece of context
// `collect_routines`'s DFS threads down — grouping would obscure the call
// site, mirrors `infer_receiver_type`'s identical precedent.
fn lower_routine<'t>(
    node: RawNode<'t>,
    attr_items: Vec<RawNode<'t>>,
    dataitem_source_table: Option<String>,
    member: Option<RawNode<'t>>,
    in_dataset_modify_context: bool,
    source: &str,
    ir: &mut Ir,
    issues: &mut Vec<SyntaxIssue>,
) -> RoutineDecl {
    // Enclosing-member capture (E1): a member wrapper with a `name` → its (stripped)
    // name + the wrapper's origin (range). The engine unescapes the name + anchors.
    //
    // `modify_modification` carries the modified member's name in its `target` field,
    // not `name` (`RawModifyModification` has no `name()` accessor at all — Task 1,
    // dataitem-receivers plan), so it needs a fallback. Deliberately scoped to THIS node
    // kind only: sibling `addafter`/`addbefore`/`moveafter`/`movebefore` modification
    // nodes also carry a `target` field, but theirs is an INSERTION ANCHOR (a reference
    // to a different, already-existing member), never the identity of the member being
    // declared — extending this fallback to them would be semantically wrong.
    let enclosing_member = member.and_then(|m| {
        let name_node = m.field(FieldName::Name).or_else(|| {
            if m.kind() == RawKind::ModifyModification {
                m.field(FieldName::Target)
            } else {
                None
            }
        });
        name_node.map(|n| (ident_text(n, source), origin_of(m)))
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
        .map(|n| routine_name_text(n, source))
        .unwrap_or_default();
    // Origin of the name identifier itself (for the LSP selection_range); fall back to
    // the whole-routine origin when the name node is absent.
    let name_origin = node
        .field(FieldName::Name)
        .map(origin_of)
        .unwrap_or_else(|| origin_of(node));

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

    // `interface_procedure` (receiver-closure plan, Task 1): its return-type spec
    // lives on a nested, NAMED `interface_procedure_suffix` child (grammar:
    // `optional($.interface_procedure_suffix)`, not inlined — `interface_procedure`
    // itself has no `return_type` field), unlike `_procedure_header`'s
    // `_procedure_return_specification`/`_procedure_named_return`, which ARE
    // hidden/inlined directly onto `node`. Fall back to that child when the direct
    // field is absent, so a controladdin/interface procedure's declared return
    // type is captured with the same fidelity as an ordinary procedure's.
    let return_type = node
        .field(FieldName::ReturnType)
        .or_else(|| {
            node.named_children()
                .into_iter()
                .find(|c| c.kind() == RawKind::InterfaceProcedureSuffix)
                .and_then(|suffix| suffix.field(FieldName::ReturnType))
        })
        .map(|n| n.text(source).trim().to_string());

    // Named-return-value binding (T3, receiver-closure-and-arg-increments plan):
    // `procedure X() Ret: Record Y` — grammar's `_procedure_named_return` sets BOTH
    // `return_value` (the binding name) and `return_type` (captured above) together;
    // an anonymous `: Type` return sets neither. Previously discarded entirely — the
    // binding name never made it past this function, so `Ret.Get(...)` mid-body had no
    // way to type `Ret`. `ident_text` strips the outer quotes for a QUOTED binding name
    // (`"My Result": Record Y`), matching `Param`/`VarDecl` name storage convention.
    // Direct `node.field` (not the `InterfaceProcedureSuffix` fallback above) —
    // `interface_procedure`/`controladdin` signature-only declarations have no body to
    // reference a named return in anyway, so the fallback's extra reach is unneeded
    // here (a `None` there is correct: no binding to synthesize a scoped symbol from).
    let return_name = node
        .field(FieldName::ReturnValue)
        .map(|n| ident_text(n, source))
        .filter(|s| !s.is_empty());

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
        name_origin,
        params,
        return_type,
        return_name,
        locals,
        attributes,
        attributes_parsed,
        access_modifier,
        parse_incomplete: node.has_error(),
        dataitem_source_table,
        enclosing_member,
        in_dataset_modify_context,
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
                // The `pattern` field now binds a single value node per branch value
                // (grammar rule `_case_pattern_item`), so the `,` separators are NOT
                // tagged `pattern`. The `is_named()` filter is kept as defense-in-depth:
                // an anonymous `,` token has no `RawKind` and would panic in `lower_expr`.
                let patterns = child
                    .children_by_field(FieldName::Pattern)
                    .into_iter()
                    .filter(|p| p.is_named())
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

/// A routine name node — either a plain `(quoted_)identifier` or a scoped
/// `member_trigger_name` (`Object::Member`). For the scoped form, join the two
/// (each unescaped) as `Object::Member` so the full qualified trigger name is kept
/// (`field("name")` on the old `multiple` shape returned only the object half).
fn routine_name_text(n: RawNode, source: &str) -> String {
    if n.kind() == RawKind::MemberTriggerName {
        let object = n
            .field(FieldName::Object)
            .map(|o| ident_text(o, source))
            .unwrap_or_default();
        let member = n
            .field(FieldName::Member)
            .map(|m| ident_text(m, source))
            .unwrap_or_default();
        format!("{object}::{member}")
    } else {
        ident_text(n, source)
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

#[cfg(test)]
mod tests {
    use crate::ir::{ExprKind, Literal, StmtKind};
    use crate::parse;

    /// Find the first `Case` statement in a parsed file.
    fn first_case(
        af: &crate::ir::AlFile,
    ) -> (&[crate::ir::CaseBranch], Option<crate::ir::BlockId>) {
        for s in af.ir.iter_stmts() {
            if let StmtKind::Case {
                branches,
                else_block,
                ..
            } = &s.kind
            {
                return (branches, *else_block);
            }
        }
        panic!("no Case statement lowered");
    }

    /// Regression for the case-pattern field-pollution grammar fix: `case 1, 2:` must
    /// yield TWO value patterns (the `,` separator is NOT a pattern), and lowering must
    /// not panic / emit Unknown on the comma. Pre-fix, `field('pattern', …)` spread over
    /// the inlined `,` tokens and `children_by_field("pattern")` returned anonymous commas.
    #[test]
    fn comma_case_pattern_yields_two_named_patterns_no_comma() {
        let src = r#"
codeunit 50000 T
{
    procedure P(I: Integer)
    begin
        case I of
            1, 2:
                Foo();
            3:
                Bar();
            else
                Baz();
        end;
    end;
}
"#;
        let af = parse(src);
        let (branches, else_block) = first_case(&af);
        assert_eq!(branches.len(), 2, "two value branches (1,2 and 3)");
        assert_eq!(branches[0].patterns.len(), 2, "`1, 2:` → two patterns");
        assert_eq!(branches[1].patterns.len(), 1, "`3:` → one pattern");
        assert!(else_block.is_some(), "else branch present");
        // Both patterns of branch 0 are integer literals — never a comma token.
        for &p in &branches[0].patterns {
            assert!(
                matches!(af.ir.expr(p).kind, ExprKind::Literal(Literal::Int(_))),
                "expected Int literal pattern"
            );
        }
        // No Unknown exprs anywhere in this clean snippet.
        assert!(
            af.ir
                .iter_exprs()
                .all(|e| !matches!(e.kind, ExprKind::Unknown)),
            "no Unknown exprs"
        );
    }

    /// `X in [..]` as a case pattern now binds the `pattern` field to a SINGLE named
    /// `in_expression` node, instead of the old inline `seq` that spread `left` /
    /// `operator` / `right` fields onto the branch. So the branch has exactly ONE
    /// pattern (not three), and lowering does not panic. (`in_expression` itself is an
    /// unmodelled container today → `ExprKind::Unknown` with its `[..]`/identifier
    /// children captured in the arena — a separate, pre-existing modeling gap, not a
    /// field-pollution regression.)
    #[test]
    fn in_expression_case_pattern_is_a_single_pattern() {
        let src = r#"
codeunit 50001 T
{
    procedure P(I: Integer): Boolean
    begin
        case true of
            I in [1, 2, 3]:
                exit(true);
            else
                exit(false);
        end;
    end;
}
"#;
        let af = parse(src);
        let (branches, _else) = first_case(&af);
        assert_eq!(branches.len(), 1, "one value branch (the `in` pattern)");
        assert_eq!(
            branches[0].patterns.len(),
            1,
            "`I in [..]:` is a single pattern, not left/op/right"
        );
    }

    /// A scoped member-trigger name (`Object::Member`) lowers to the FULL qualified
    /// name. Pre-fix, `_trigger_name` was an inlined `seq(id, '::', id)` so the `name`
    /// field was `multiple:true` over the `::` token, and `field("name")` returned only
    /// the object half (`UserTours`), silently dropping the member.
    #[test]
    fn member_trigger_name_lowers_to_qualified_name() {
        let src = r#"
codeunit 50000 T
{
    trigger UserTours::ShowTourWizard()
    begin
    end;
}
"#;
        let af = parse(src);
        let routine = af
            .objects
            .iter()
            .flat_map(|o| &o.routines)
            .find(|r| matches!(r.kind, crate::ir::RoutineKind::Trigger))
            .expect("a trigger routine");
        assert_eq!(routine.name, "UserTours::ShowTourWizard");
    }

    /// Dataitem-receivers plan, Task 1: `modify_modification` carries the
    /// modified member's name in its `target` field, not `name`
    /// (`RawModifyModification` has no `name()` accessor). Pre-fix, a
    /// trigger nested in `dataset { modify(X) { trigger .. } }` lost BOTH
    /// `enclosing_member` and any path to `dataitem_source_table` — the
    /// lowerer's generic Name-based member-wrapper gate never recognized the
    /// node at all. A report `dataset` `modify()` must ALSO set
    /// `in_dataset_modify_context = true` (the confirmed-dataset-context
    /// signal the resolver's fallback requires).
    #[test]
    fn modify_modification_target_becomes_enclosing_member_and_sets_dataset_context() {
        let src = r#"
report 50100 T
{
    dataset
    {
        modify(BaseDataitem)
        {
            trigger OnAfterGetRecord()
            begin
            end;
        }
    }
}
"#;
        let af = parse(src);
        let routine = af
            .objects
            .iter()
            .flat_map(|o| &o.routines)
            .find(|r| r.name.eq_ignore_ascii_case("OnAfterGetRecord"))
            .expect("the trigger nested in modify() must still be lowered");
        let (member_name, _origin) = routine
            .enclosing_member
            .as_ref()
            .expect("modify()'s Target must populate enclosing_member");
        assert_eq!(member_name, "BaseDataitem");
        assert!(
            routine.in_dataset_modify_context,
            "a dataset modify() must set the confirmed dataset-context flag"
        );
    }

    /// REQUESTPAGE ISOLATION (binding, dataitem-receivers plan round-1
    /// addendum): a `modify()` nested inside `requestpage { layout { .. } }`
    /// still gets its `enclosing_member` populated (the Target-field fix is
    /// general, not dataset-specific), but `in_dataset_modify_context` MUST
    /// stay `false` — this is NOT a report dataset `modify()`, and the
    /// resolver's dataitem-map fallback must never fire for it.
    #[test]
    fn modify_modification_inside_requestpage_never_sets_dataset_context() {
        let src = r#"
report 50101 T
{
    requestpage
    {
        layout
        {
            modify(SomeField)
            {
                trigger OnValidate()
                begin
                end;
            }
        }
    }
}
"#;
        let af = parse(src);
        let routine = af
            .objects
            .iter()
            .flat_map(|o| &o.routines)
            .find(|r| r.name.eq_ignore_ascii_case("OnValidate"))
            .expect("the trigger nested in the requestpage modify() must still be lowered");
        let (member_name, _origin) = routine
            .enclosing_member
            .as_ref()
            .expect("modify()'s Target must populate enclosing_member even outside dataset");
        assert_eq!(member_name, "SomeField");
        assert!(
            !routine.in_dataset_modify_context,
            "a requestpage/layout modify() must NEVER set dataset context"
        );
    }

    /// Task 1 review-fix pin (undescribed/untested finding): the
    /// `RawKind::ModifyModification` arm added to `collect_routines` is
    /// GLOBAL — it fires for ANY `modify()` block regardless of enclosing
    /// object kind, not only a report `dataset`/`requestpage`. A
    /// TableExtension's `fields { modify(Field) { trigger .. } }` is the
    /// most common real-world shape this touches. `enclosing_member` must
    /// populate (the Target-field fix is general), but
    /// `in_dataset_modify_context` must stay `false` — `dataset_ctx` is only
    /// ever forced `true` descending into a report `DatasetSection`/
    /// `ReportDataitem`, neither of which a TableExtension's `fields`
    /// section is — so the resolver's dataitem-map fallback correctly never
    /// fires here (inert on CDO, verified by the Task 1 CDO re-measure; this
    /// test pins the lowering behavior the resolver's inertness depends on).
    #[test]
    fn modify_modification_in_tableextension_fields_populates_member_not_dataset_context() {
        let src = r#"
tableextension 50100 "T Ext" extends Customer
{
    fields
    {
        modify(Name)
        {
            trigger OnBeforeValidate()
            begin
            end;
        }
    }
}
"#;
        let af = parse(src);
        let routine = af
            .objects
            .iter()
            .flat_map(|o| &o.routines)
            .find(|r| r.name.eq_ignore_ascii_case("OnBeforeValidate"))
            .expect(
                "the trigger nested in the TableExtension field modify() must still be lowered",
            );
        let (member_name, _origin) = routine
            .enclosing_member
            .as_ref()
            .expect("modify()'s Target must populate enclosing_member outside Report objects too");
        assert_eq!(member_name, "Name");
        assert!(
            !routine.in_dataset_modify_context,
            "a TableExtension field modify() must never set report dataset context"
        );
    }

    /// A dataitem trigger's `dataitem_source_table` (the direct, non-fallback
    /// path) is unaffected by the `modify()` fix — sanity guard against
    /// regressing the existing mechanism while adding the new one.
    #[test]
    fn report_dataitem_trigger_still_carries_dataitem_source_table_directly() {
        let src = r#"
report 50102 T
{
    dataset
    {
        dataitem(Cust; Customer)
        {
            trigger OnAfterGetRecord()
            begin
            end;
        }
    }
}
"#;
        let af = parse(src);
        let routine = af
            .objects
            .iter()
            .flat_map(|o| &o.routines)
            .find(|r| r.name.eq_ignore_ascii_case("OnAfterGetRecord"))
            .expect("the dataitem trigger must be lowered");
        assert_eq!(routine.dataitem_source_table.as_deref(), Some("Customer"));
        assert!(
            !routine.in_dataset_modify_context,
            "a real dataitem(...) trigger is not a modify() member — the flag stays false"
        );
    }

    // -----------------------------------------------------------------------
    // Task 3 (preprocessor foundations plan): union-read pins + the two
    // flat-loop fixes (properties, implements) + the ParseStatus::Recovered
    // diagnostic fixture.
    // -----------------------------------------------------------------------

    /// The base union-read pin: a procedure declared inside a `#if
    /// UNDEFINED_SYMBOL .. #endif` branch (never true for any real build) is
    /// STILL lowered into `ObjectDecl.routines` — `is_preproc_wrapper`'s
    /// descent is unconditional, it never evaluates the condition. Documents
    /// the honest superset semantics: sound for an absence proof ("not found
    /// anywhere in the union" implies "absent from every build"), but this
    /// specific routine is NOT proof the call is reachable in any actual
    /// compiled build (see `is_preproc_wrapper`'s doc).
    #[test]
    fn preproc_undefined_branch_procedure_still_lowered_union_read() {
        let src = r#"
codeunit 50200 "Union Read"
{
    procedure Caller()
    begin
        Foo();
    end;

#if NEVER_DEFINED_SYMBOL
    procedure Foo()
    begin
    end;
#endif
}
"#;
        let af = parse(src);
        let names: Vec<&str> = af
            .objects
            .iter()
            .flat_map(|o| &o.routines)
            .map(|r| r.name.as_str())
            .collect();
        assert!(
            names.iter().any(|n| n.eq_ignore_ascii_case("Foo")),
            "a #if-UNDEFINED-branch procedure must still surface in the union-read \
             (sound-for-absence, not-sound-for-reachability); got routines: {names:?}"
        );
        assert!(names.iter().any(|n| n.eq_ignore_ascii_case("Caller")));
    }

    /// Both-arms `#if`/`#else` with DIFFERING signatures yields TWO distinct
    /// `RoutineDecl`s at the al-syntax layer (no dedup happens here at all —
    /// that is a `crate::program` (build.rs) concern, driven by `sig_fp`; see
    /// `preproc_both_arms_distinct_signature_yield_two_unmarked_source_
    /// overloads` / `preproc_same_signature_arms_collapse_to_one_unmarked_
    /// survivor` in `src/program/build.rs`'s test module for that half of the
    /// dedup interplay).
    #[test]
    fn preproc_both_arms_distinct_signature_yield_two_routine_decls() {
        let src = r#"
codeunit 50201 "Both Arms"
{
#if SOME_SYMBOL
    procedure Foo(X: Integer)
    begin
    end;
#else
    procedure Foo(Y: Text)
    begin
    end;
#endif
}
"#;
        let af = parse(src);
        let foos: Vec<_> = af
            .objects
            .iter()
            .flat_map(|o| &o.routines)
            .filter(|r| r.name.eq_ignore_ascii_case("Foo"))
            .collect();
        assert_eq!(
            foos.len(),
            2,
            "both #if/#else arms must lower to distinct RoutineDecls, not one \
             overwriting the other"
        );
        let param_types: Vec<Option<&str>> = foos
            .iter()
            .map(|r| r.params.first().and_then(|p| p.ty.as_deref()))
            .collect();
        assert!(
            param_types.contains(&Some("Integer")),
            "got param types: {param_types:?}"
        );
        assert!(
            param_types.contains(&Some("Text")),
            "got param types: {param_types:?}"
        );
    }

    /// The properties flat-loop fix: a `#if`-wrapped `SourceTable` property
    /// (previously silently dropped by `lower_object`'s flat
    /// `body.named_children()` scan — the ORIGINAL, now-fixed gap) is
    /// captured from BOTH branches. The program layer
    /// (`node_extract::singular_property_value`) is responsible for the
    /// fail-closed conflict degrade this union-read now enables; this test
    /// only pins the lowering-layer union-read itself.
    #[test]
    fn preproc_wrapped_source_table_property_captured_both_branches() {
        let src = r#"
page 50202 "Preproc Prop"
{
#if FOO
    SourceTable = Customer;
#else
    SourceTable = Vendor;
#endif

    layout
    {
    }
}
"#;
        let af = parse(src);
        let props: Vec<&crate::ir::ObjectProperty> = af.objects[0]
            .properties
            .iter()
            .filter(|p| p.name == "sourcetable")
            .collect();
        assert_eq!(
            props.len(),
            2,
            "both #if/#else SourceTable branches must be captured (union-read), \
             not just the first — got: {:?}",
            props.iter().map(|p| &p.value).collect::<Vec<_>>()
        );
        let values: Vec<&str> = props.iter().map(|p| p.value.as_str()).collect();
        assert!(values.contains(&"Customer"));
        assert!(values.contains(&"Vendor"));
    }

    /// Control: a NON-conditional property is unaffected by the fix (sanity
    /// guard against a regression in the common, unconditional case).
    #[test]
    fn plain_source_table_property_still_captured_control() {
        let src = r#"
page 50203 "Plain Prop"
{
    SourceTable = Customer;
    layout
    {
    }
}
"#;
        let af = parse(src);
        let props: Vec<&crate::ir::ObjectProperty> = af.objects[0]
            .properties
            .iter()
            .filter(|p| p.name == "sourcetable")
            .collect();
        assert_eq!(props.len(), 1);
        assert_eq!(props[0].value, "Customer");
    }

    /// The implements flat-loop's defensive descend fix, pinned against the
    /// only grammar-reachable `#if`-conditional `implements` shape
    /// (`preproc_split_declaration` — a whole-object header split): both
    /// branches' interface names are captured (a UNION, never degraded — see
    /// `extract_implements`'s doc for why this is sound without a
    /// program-layer conflict-degrade, unlike `SourceTable`/`TableNo`).
    #[test]
    fn preproc_split_declaration_implements_captured_both_branches() {
        let src = r#"
#if FOO
codeunit 50204 "Preproc Impl" implements IThing
#else
codeunit 50204 "Preproc Impl" implements IOther
#endif
{
}
"#;
        let af = parse(src);
        assert_eq!(af.objects.len(), 1, "one (preproc-split) object");
        let implements = &af.objects[0].implements;
        assert!(
            implements.iter().any(|i| i.eq_ignore_ascii_case("IThing")),
            "got implements: {implements:?}"
        );
        assert!(
            implements.iter().any(|i| i.eq_ignore_ascii_case("IOther")),
            "got implements: {implements:?}"
        );
    }

    /// The `ParseStatus::Recovered` diagnostic fixture: an unbalanced `#if`
    /// (no matching `#endif`) forces tree-sitter error recovery — the whole
    /// file's `parse_status` must report `Recovered`, the signal
    /// `crate::program`'s Recovered-file diagnostic (count + paths) consumes.
    #[test]
    fn unbalanced_if_directive_yields_recovered_parse_status() {
        let src = r#"
codeunit 50205 "Unbalanced"
{
    procedure Foo()
    begin
#if NEVER_CLOSED
        Bar();
    end;
}
"#;
        let af = parse(src);
        assert_eq!(
            af.parse_status,
            crate::ir::ParseStatus::Recovered,
            "an unbalanced #if must force error recovery, never report Clean"
        );
    }

    // -------------------------------------------------------------------
    // `interface_procedure` lowering (receiver-closure plan, Task 1) —
    // controladdin/interface signature-only procedures were previously
    // INVISIBLE to `RoutineDecl` extraction (a distinct grammar node,
    // `interface_procedure`, that `collect_routines` never matched).
    // -------------------------------------------------------------------

    /// A `controladdin` object's signature-only `procedure` declarations
    /// (no body, no trailing `;`) must be captured as `RoutineDecl`s with
    /// the right name + arity — the "declared procedures" surface Task 1's
    /// closed-if-known `CurrPage.<usercontrol>` gate depends on.
    #[test]
    fn controladdin_procedures_are_lowered_as_routines() {
        let src = r#"
controladdin "CDO.Editor"
{
    RequestedHeight = 500;

    event StartupCompleted();

    procedure InitEditor(mergeItemsAsJson: Text; localizationAsJson: Text)
    procedure GetHTML()
}
"#;
        let af = parse(src);
        assert_eq!(af.objects.len(), 1);
        let obj = &af.objects[0];
        assert_eq!(obj.routines.len(), 2, "two procedures, zero events");
        let init = obj
            .routines
            .iter()
            .find(|r| r.name.eq_ignore_ascii_case("InitEditor"))
            .expect("InitEditor must be lowered");
        assert_eq!(init.params.len(), 2, "arity must be captured");
        assert!(init.body.is_none(), "a signature has no body");
        assert!(!init.parse_incomplete);
        let get_html = obj
            .routines
            .iter()
            .find(|r| r.name.eq_ignore_ascii_case("GetHTML"))
            .expect("GetHTML must be lowered");
        assert_eq!(get_html.params.len(), 0);
    }

    /// An `event` declaration inside a controladdin is NEVER lowered as a
    /// `RoutineDecl` — events are not AL-callable, and the gate's declared
    /// surface must never accidentally admit one.
    #[test]
    fn controladdin_events_are_never_lowered_as_routines() {
        let src = r#"
controladdin "CDO.Editor"
{
    event OnSaveHTML(htmlString: Text);
    procedure SetHTML(html: Text)
}
"#;
        let af = parse(src);
        let obj = &af.objects[0];
        assert!(
            obj.routines
                .iter()
                .all(|r| !r.name.eq_ignore_ascii_case("OnSaveHTML")),
            "an event must never surface as a RoutineDecl: {:?}",
            obj.routines.iter().map(|r| &r.name).collect::<Vec<_>>()
        );
        assert_eq!(obj.routines.len(), 1, "only the procedure lowers");
    }

    /// An `interface` object's signature-only procedures lower identically
    /// (same grammar node, `interface_procedure`) — a bonus fidelity fix,
    /// not itself consumed by any resolver yet (interface dispatch routes
    /// through the implementer's own routine, not the interface's).
    #[test]
    fn interface_procedures_are_lowered_as_routines() {
        let src = r#"
interface "IFoo"
{
    procedure DoThing(x: Integer): Boolean;
}
"#;
        let af = parse(src);
        let obj = &af.objects[0];
        assert_eq!(obj.routines.len(), 1);
        let do_thing = &obj.routines[0];
        assert_eq!(do_thing.name, "DoThing");
        assert_eq!(do_thing.params.len(), 1);
        assert_eq!(
            do_thing.return_type.as_deref(),
            Some("Boolean"),
            "return type must be recovered from the nested interface_procedure_suffix child"
        );
    }

    /// T3 (receiver-closure-and-arg-increments plan): a NAMED return value
    /// (`procedure X() Ret: Record Y`) must capture the binding NAME on
    /// `RoutineDecl.return_name` — previously silently discarded entirely.
    #[test]
    fn named_return_value_binding_is_captured() {
        let src = r#"
codeunit 50100 "My Codeunit"
{
    procedure GetItem() Ret: Record Item
    begin
    end;
}
"#;
        let af = parse(src);
        let obj = &af.objects[0];
        let r = &obj.routines[0];
        assert_eq!(r.return_name.as_deref(), Some("Ret"));
        assert_eq!(r.return_type.as_deref(), Some("Record Item"));
    }

    /// An anonymous `: Type` return (no binding name) must capture `None` —
    /// never fabricate a name.
    #[test]
    fn anonymous_return_type_has_no_return_name() {
        let src = r#"
codeunit 50100 "My Codeunit"
{
    procedure GetCount(): Integer
    begin
    end;
}
"#;
        let af = parse(src);
        let obj = &af.objects[0];
        let r = &obj.routines[0];
        assert_eq!(r.return_name, None);
        assert_eq!(r.return_type.as_deref(), Some("Integer"));
    }

    /// A QUOTED binding name must be captured UNQUOTED (mirrors `Param`/
    /// `VarDecl` name storage convention — `ident_text` strips the outer
    /// quotes at lowering time).
    #[test]
    fn quoted_named_return_value_binding_is_unquoted() {
        let src = r#"
codeunit 50100 "My Codeunit"
{
    procedure GetItem() "My Result": Record Item
    begin
    end;
}
"#;
        let af = parse(src);
        let obj = &af.objects[0];
        let r = &obj.routines[0];
        assert_eq!(r.return_name.as_deref(), Some("My Result"));
        assert_eq!(r.return_type.as_deref(), Some("Record Item"));
    }

    /// A procedure with NO return spec at all (no `:`) must carry `None` for
    /// both — the common majority case, guards against a regression that
    /// fabricates a name/type out of thin air.
    #[test]
    fn no_return_spec_at_all_has_no_return_name_or_type() {
        let src = r#"
codeunit 50100 "My Codeunit"
{
    procedure DoSomething()
    begin
    end;
}
"#;
        let af = parse(src);
        let obj = &af.objects[0];
        let r = &obj.routines[0];
        assert_eq!(r.return_name, None);
        assert_eq!(r.return_type, None);
    }
}
