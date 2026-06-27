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
    pub origin: Origin,
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
