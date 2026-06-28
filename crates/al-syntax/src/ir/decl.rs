//! IR declarations: objects, routines, parameters, variables.
//!
//! Types are kept as raw strings here (e.g. `"Record Customer"`, `"Code[20]"`);
//! the engine parses/resolves them (that is resolver work, not syntax). `globals`
//! and `locals` include declarations from BOTH `#if`/`#else` branches.

use super::{BlockId, Origin};

pub struct ObjectDecl {
    pub kind: ObjectKind,
    /// Object number where the grammar provides one (codeunit/table/page/...).
    pub id: Option<i64>,
    pub name: String,
    pub routines: Vec<RoutineDecl>,
    pub globals: Vec<VarDecl>,
    /// Object-level properties (`SourceTable`, `TableNo`, `PageType`, …) in source
    /// order. Needed by the engine to seed implicit-`Rec` table resolution and object
    /// classification; the value is the raw value text (trimmed).
    pub properties: Vec<ObjectProperty>,
    /// Report `dataitem(Name; "Source Table")` declarations (name, source-table), both
    /// unquoted, document order — Report/ReportExtension only (empty otherwise). A
    /// dataitem name is in scope as a record var across ALL the report's routines, so
    /// the engine seeds each as a record variable in every routine. Distinct from a
    /// dataitem trigger's per-dataitem implicit `Rec` (see
    /// [`RoutineDecl::dataitem_source_table`]).
    pub report_dataitems: Vec<(String, String)>,
    /// The `extends <Target>` target name (unquoted) for an extension object
    /// (Table/Page/Report/Enum/PermissionSet extension), else `None`.
    pub extends_target: Option<String>,
    /// Interface names from an `implements` clause (unquoted, document order) — for
    /// Codeunit / Enum / Interface objects; empty otherwise.
    pub implements: Vec<String>,
    /// Page controls (`part` / `systempart` / `usercontrol` sections) in document
    /// order — Page / PageExtension only. Resolves `CurrPage.<control>…` member calls.
    pub page_controls: Vec<PageControl>,
    /// Table fields (Table / TableExtension only), document order. The engine assigns
    /// the internal/stable ids; the IR carries the raw extracted shape.
    pub fields: Vec<FieldDecl>,
    /// Table keys (Table / TableExtension only) — each the member field NAMES (unquoted,
    /// lowercased) in declaration order; the engine resolves them to field ids.
    pub keys: Vec<Vec<String>>,
    pub origin: Origin,
}

/// A page control section (`part` / `systempart` / `usercontrol`).
pub struct PageControl {
    pub name: String,
    /// Raw kind: `"part"` / `"systempart"` / `"usercontrol"`.
    pub kind: String,
    pub target: String,
}

/// A table `field(<no>; <Name>; <Type>) { ... }` declaration. `data_type` is the raw
/// type text; `field_class` is `Normal` / `FlowField` / `FlowFilter` (from the
/// `FieldClass` property); `is_blob_like` flags Blob/Media/MediaSet.
pub struct FieldDecl {
    pub number: i64,
    pub name: String,
    pub data_type: String,
    pub field_class: String,
    pub is_blob_like: bool,
}

/// A single object-level `property` node (`name = value`). `name` is lowercased;
/// `value` is the raw value text (quotes preserved — the engine strips as needed).
pub struct ObjectProperty {
    pub name: String,
    pub value: String,
    pub origin: Origin,
}

/// A parsed routine attribute (`[EventSubscriber(ObjectType::Codeunit, …)]`).
/// `name` is the raw attribute name; `raw` the full `attribute_item` text; `args`
/// the lowered argument exprs (the engine projects each to kind/text/value/…).
pub struct AttributeIr {
    pub name: String,
    pub raw: String,
    pub args: Vec<super::ExprId>,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum ObjectKind {
    Codeunit,
    Table,
    TableExtension,
    Page,
    PageExtension,
    Report,
    ReportExtension,
    Query,
    XmlPort,
    Enum,
    EnumExtension,
    Interface,
    ControlAddIn,
    Entitlement,
    PermissionSet,
    PermissionSetExtension,
    Profile,
    /// Any other object construct, kind preserved via `Origin.kind_text`.
    Other,
}

pub struct RoutineDecl {
    pub kind: RoutineKind,
    pub name: String,
    /// Origin of the routine's NAME identifier node (not the whole routine). The LSP
    /// front-end uses this for a call-hierarchy item's `selection_range` (the range
    /// the editor highlights when you click the symbol) — e.g. an event publisher's
    /// procedure-name range. Falls back to the routine `origin` if the name is absent
    /// (a malformed/anonymous routine).
    pub name_origin: Origin,
    pub params: Vec<Param>,
    /// Return type text, if declared.
    pub return_type: Option<String>,
    pub locals: Vec<VarDecl>,
    /// Attribute names (lowercased) from the `attribute_item` siblings preceding the
    /// routine — e.g. "eventsubscriber", "tryfunction", "integrationevent". Drives
    /// routine-kind classification + control-context guards.
    pub attributes: Vec<String>,
    /// Full parsed attributes (name + raw text + lowered argument exprs), in source
    /// order — for the L2 `attributes` / `attributesParsed` envelope. The engine
    /// renders each arg via its expression-info projection.
    pub attributes_parsed: Vec<AttributeIr>,
    /// Access modifier keyword (lowercased: "local"/"internal"/"protected"), or None
    /// for a public procedure / a trigger. Mirrors the `modifier` field.
    pub access_modifier: Option<String>,
    /// `true` when this routine's subtree contains a parse error (tree-sitter
    /// `has_error`). Mirrors the legacy `parseIncomplete` / drives the IR-vs-legacy
    /// feature-extraction choice (malformed routines use legacy ERROR-recovery).
    pub parse_incomplete: bool,
    /// For a trigger nested inside a report `dataitem(Name; "Source Table")`, the
    /// enclosing (innermost) dataitem's SOURCE TABLE name (unquoted) — the type of the
    /// dataitem trigger's implicit `Rec`. `None` for any non-dataitem routine. Mirrors
    /// the legacy `report_dataitem_source_table`.
    pub dataitem_source_table: Option<String>,
    /// For a MEMBER-trigger (a trigger nested in a named member: table/page field,
    /// page part, action, report dataitem, query element, …), the enclosing member's
    /// name (outer-quote-stripped — the engine applies `unescape_al_identifier`) and the
    /// member wrapper's `Origin` (for `enclosingMemberRange` + `originatingObject`).
    /// `None` for a procedure or an object-level trigger (OnRun / OnOpenPage). Mirrors
    /// the legacy `enclosing_member_of` (E1 — additive, never serialized into a golden).
    pub enclosing_member: Option<(String, Origin)>,
    /// `None` for a forward/external declaration with no body.
    pub body: Option<BlockId>,
    pub origin: Origin,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum RoutineKind {
    Procedure,
    Trigger,
}

pub struct Param {
    pub name: String,
    /// `var` (by-reference) parameter.
    pub by_ref: bool,
    pub ty: Option<String>,
    pub origin: Origin,
}

pub struct VarDecl {
    pub name: String,
    pub ty: Option<String>,
    pub temporary: bool,
    pub origin: Origin,
}
