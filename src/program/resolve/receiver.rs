//! Receiver-type lattice and Phase-A inference for member-call resolution.
//!
//! # Overview
//!
//! Member-call resolution (Phase B) dispatches on the STATIC TYPE of the receiver
//! expression. This module provides:
//! - [`ReceiverType`] — the lattice value Phase B dispatches on.
//! - [`FrameworkKind`] — the platform/framework data-type discriminant.
//! - [`ParsedType`] — intermediate result of [`classify_type_text`] (string→shape,
//!   no graph access).
//! - [`classify_type_text`] — pure string parse of a declared type string.
//! - [`infer_receiver_type`] — Phase A: infer the receiver type for a member call.
//!
//! # Phase A inference order
//!
//! Given a lowercased receiver name `receiver_lc`, inference proceeds:
//!
//! 1. **Singletons** — hardcoded platform names (`currpage`, `session`, `this`, …)
//!    that are never declared as AL variables; returns immediately.
//! 2. **Variable lookup** — searches `routine.params` then `routine.locals` then
//!    `object_globals` by lowercased name → calls [`classify_type_text`] on the
//!    declared type → resolves Record table names and Object names against the graph.
//!    When the receiver name is `rec`/`xrec`, a variable with that name shadows
//!    the implicit-Rec step (a Codeunit routine may declare `var Rec: Record
//!    Customer`; the declared type is used in that case).
//! 3. **Implicit Rec / xRec** — reached only when no variable named `rec`/`xrec`
//!    was found in step 2: resolves to the enclosing object's implicit record type
//!    (Table self-id, TableExtension base, or `Record{None}` for Page/Report where
//!    the source table is not on `ObjectNode`). Returns `Unknown` for object kinds
//!    that have no implicit record (e.g. Codeunit).
//! 4. **Static framework type name** — when the receiver name matches a framework
//!    type name (e.g. `XmlDocument`, `Text`, `File`, `Version`) and no variable was
//!    found, type it as `Framework(kind)` so Phase B dispatches the static method
//!    via the builtin catalog.
//! 5. **Unknown** — no positive typing found.
//!
//! # Clean-room note
//!
//! This mirrors the logic of L3's `infer_receiver_type` in
//! `src/engine/l3/receiver_type.rs` but is written fresh over the IR
//! (`RoutineDecl`/`VarDecl`/`Param`) and `ProgramGraph`/`ResolveIndex`, carrying
//! `ObjectNodeId`s instead of L3 string IDs.

use al_syntax::ir::{ObjectKind, RoutineDecl, VarDecl};

use crate::program::graph::ProgramGraph;
use crate::program::node::ObjectNodeId;
use crate::program::node_extract::ObjectNode;
use crate::program::resolve::index::ResolveIndex;

// ---------------------------------------------------------------------------
// FrameworkKind
// ---------------------------------------------------------------------------

/// Discriminant for AL platform framework / value types whose methods are
/// dispatched purely via the builtin catalog in Phase B.
///
/// Explicit variants cover the high-volume framework types; `Other` is the
/// catch-all for less-common types, carrying the lowercased first token of the
/// declared type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameworkKind {
    // JSON types
    JsonObject,
    JsonToken,
    JsonArray,
    JsonValue,
    // HTTP types
    HttpClient,
    HttpRequestMessage,
    HttpResponseMessage,
    HttpContent,
    HttpHeaders,
    // Stream types
    InStream,
    OutStream,
    // String / text types
    TextBuilder,
    Text,
    BigText,
    SecretText,
    // Collection types
    List,
    Dictionary,
    // XML types (all xml* tokens)
    Xml,
    // Date/time value types
    Date,
    DateTime,
    Time,
    Duration,
    // GUID
    Guid,
    // Media / blob
    Blob,
    Media,
    // Notification / error
    Notification,
    ErrorInfo,
    // Misc platform value types
    RecordId,
    ModuleInfo,
    DataTransfer,
    SessionSettings,
    FilterPageBuilder,
    File,
    FileUpload,
    NumberSequence,
    Version,
    // Dialog
    Dialog,
    // Page/Report singleton types (from receiver name, not declared type)
    PageInstance,
    ReportInstance,
    // Platform singletons (from receiver name)
    Session,
    NavApp,
    Database,
    IsolatedStorage,
    TaskScheduler,
    System,
    CompanyProperty,
    SessionInformation,
    // ControlAddIn — every method is a JS-side platform invocation → builtin
    ControlAddIn,
    // Enum — static enum type used as a receiver (FromInteger / Names / Ordinals)
    Enum,
    /// Programmatic-construction catch-all for less-common types encountered at
    /// Phase-B dispatch time.  Carries the lowercased first token of the declared
    /// type string.
    ///
    /// **Never emitted by [`classify_type_text`]** — all recognized type names map
    /// to explicit variants.  This variant exists for callers (Phase B, tests) that
    /// construct a [`FrameworkKind`] programmatically for unlisted types.
    Other(String),
}

// ---------------------------------------------------------------------------
// ReceiverType
// ---------------------------------------------------------------------------

/// The static type of a member-call receiver — the lattice Phase B dispatches on.
///
/// Every variant maps 1:1 onto a Phase-B `match` arm. The lattice is fail-closed:
/// any receiver that Phase A cannot positively type becomes `Unknown`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReceiverType {
    /// A first-class AL object type (Codeunit / Page / Report / Query / XmlPort)
    /// identified by kind and lowercased name.  Phase B resolves the method among
    /// the object's declared procedures via `graph.resolve_object`.
    Object { kind: ObjectKind, name_lc: String },
    /// An `Interface IFoo` receiver — Phase B fans out to every implementer.
    Interface { name_lc: String },
    /// An `Enum "Color"` receiver — enum statics (FromInteger/Names/Ordinals).
    EnumType { name_lc: String },
    /// A `Record`-typed receiver.  `table` is the resolved `ObjectNodeId` of the
    /// table when it is in the workspace closure, else `None` (out-of-source table).
    ///
    /// A Record receiver is ALWAYS `Record`, even with `None` — Phase B's builtin
    /// catalog check is table-independent (SetRange / FindSet etc. are `builtin`
    /// regardless), and only a non-builtin method on a table-less Record yields
    /// the honest `Unknown` (decided in Phase B, not here).
    Record { table: Option<ObjectNodeId> },
    /// The enclosing object instance (`this.OwnMethod()`).  Phase B resolves the
    /// method among the caller object's own procedures.
    SelfObject,
    /// `RecordRef` receiver — catalog-only in Phase B.
    RecordRef,
    /// `FieldRef` receiver — catalog-only in Phase B.
    FieldRef,
    /// `KeyRef` receiver — catalog-only in Phase B.
    KeyRef,
    /// A platform/framework type (`Json*` / `Http*` / `InStream` / … ) — catalog
    /// lookup in Phase B.
    Framework(FrameworkKind),
    /// A primitive or unrecognized non-object, non-catalog type.  Phase B turns
    /// this into an honest `Unknown` edge.
    Primitive,
    /// A `Variant`-typed receiver — the held type is determined at runtime.
    /// NOT a resolution failure: genuinely `dynamic` per the honest taxonomy.
    Dynamic,
    /// Phase A could not positively type the receiver.
    Unknown,
}

// ---------------------------------------------------------------------------
// ParsedType — intermediate result of classify_type_text
// ---------------------------------------------------------------------------

/// Result of the pure string→shape parse in [`classify_type_text`].
///
/// Names (table name, object name, interface name, enum name) are preserved as
/// lowercased strings for subsequent graph-based resolution in
/// [`infer_receiver_type`].  No graph access is performed here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedType {
    /// `Record <TableName>` — table name is lowercased, stripped of quotes and
    /// a trailing ` temporary` modifier.
    Record { table_name: String },
    /// `Codeunit X` / `Page X` / `Report X` / `Query X` / `XmlPort X` — object
    /// kind and (possibly numeric) name as written.
    Object { kind: ObjectKind, name: String },
    /// `Interface <Name>` — lowercased interface name.
    Interface { name: String },
    /// `Enum <Name>` — lowercased enum name.
    EnumType { name: String },
    /// `RecordRef`
    RecordRef,
    /// `FieldRef`
    FieldRef,
    /// `KeyRef`
    KeyRef,
    /// A recognized platform/framework type.
    Framework(FrameworkKind),
    /// Primitive numeric/boolean type or an unrecognized keyword → Phase B unknown.
    Primitive,
    /// `Variant` — runtime-typed, genuinely dynamic dispatch.
    Dynamic,
}

// ---------------------------------------------------------------------------
// classify_type_text
// ---------------------------------------------------------------------------

/// Parse a declared type string into its [`ParsedType`] shape without any graph
/// access.
///
/// Logic mirrors L3's `classify_receiver` + `parse_object_type_ref` (clean-room):
/// - Strips a trailing `[N]` length suffix from the leading token (`Text[200]` →
///   `text`, `Code[20]` → `code`).
/// - Checks the first whitespace-delimited lowercased token against the full
///   catalog of keywords / framework types.
/// - Strips surrounding double-quotes from the name portion of compound types.
/// - `Record "Customer" temporary` → `Record { table_name: "customer" }`.
/// - `Variant` → `Dynamic`; unrecognized or primitive numeric types → `Primitive`.
pub fn classify_type_text(ty: &str) -> ParsedType {
    let trimmed = ty.trim();
    if trimmed.is_empty() {
        return ParsedType::Primitive;
    }

    // Split on the first whitespace character to get the leading token and the
    // remaining name portion (empty when the type has no name component).
    let (first_tok, rest) = match trimmed.find(char::is_whitespace) {
        Some(i) => (&trimmed[..i], trimmed[i + 1..].trim()),
        None => (trimmed, ""),
    };

    // Strip a trailing `[N]` length suffix so `Text[200]` normalises to `text`.
    let base = match first_tok.find('[') {
        Some(i) => &first_tok[..i],
        None => first_tok,
    };
    let lc = base.to_ascii_lowercase();

    match lc.as_str() {
        "record" => {
            // Parse the table name: strip trailing " temporary" then unquote.
            let stripped = strip_trailing_temporary(rest);
            let name = unquote_identifier(stripped.trim());
            ParsedType::Record {
                table_name: name.to_ascii_lowercase(),
            }
        }
        "codeunit" => parse_object_kind_type(ObjectKind::Codeunit, rest),
        "page" => parse_object_kind_type(ObjectKind::Page, rest),
        "report" => parse_object_kind_type(ObjectKind::Report, rest),
        "query" => parse_object_kind_type(ObjectKind::Query, rest),
        "xmlport" => parse_object_kind_type(ObjectKind::XmlPort, rest),
        "interface" => ParsedType::Interface {
            name: unquote_identifier(rest).to_ascii_lowercase(),
        },
        "enum" => ParsedType::EnumType {
            name: unquote_identifier(rest).to_ascii_lowercase(),
        },
        // Ref types
        "recordref" => ParsedType::RecordRef,
        "fieldref" => ParsedType::FieldRef,
        "keyref" => ParsedType::KeyRef,
        // JSON framework types
        "jsonobject" => ParsedType::Framework(FrameworkKind::JsonObject),
        "jsontoken" => ParsedType::Framework(FrameworkKind::JsonToken),
        "jsonarray" => ParsedType::Framework(FrameworkKind::JsonArray),
        "jsonvalue" => ParsedType::Framework(FrameworkKind::JsonValue),
        // HTTP framework types
        "httpclient" => ParsedType::Framework(FrameworkKind::HttpClient),
        "httprequestmessage" => ParsedType::Framework(FrameworkKind::HttpRequestMessage),
        "httpresponsemessage" => ParsedType::Framework(FrameworkKind::HttpResponseMessage),
        "httpheaders" => ParsedType::Framework(FrameworkKind::HttpHeaders),
        "httpcontent" => ParsedType::Framework(FrameworkKind::HttpContent),
        // Stream types
        "instream" => ParsedType::Framework(FrameworkKind::InStream),
        "outstream" => ParsedType::Framework(FrameworkKind::OutStream),
        // Text / string types
        "textbuilder" => ParsedType::Framework(FrameworkKind::TextBuilder),
        "text" | "code" | "label" => ParsedType::Framework(FrameworkKind::Text),
        "bigtext" => ParsedType::Framework(FrameworkKind::BigText),
        "secrettext" => ParsedType::Framework(FrameworkKind::SecretText),
        // Collection types
        "list" => ParsedType::Framework(FrameworkKind::List),
        "dictionary" => ParsedType::Framework(FrameworkKind::Dictionary),
        // XML types — all tokens starting with "xml" (XmlDocument, XmlElement, …)
        s if s.starts_with("xml") => ParsedType::Framework(FrameworkKind::Xml),
        // Media / blob
        "blob" => ParsedType::Framework(FrameworkKind::Blob),
        "media" | "mediaset" => ParsedType::Framework(FrameworkKind::Media),
        // Dialog
        "dialog" => ParsedType::Framework(FrameworkKind::Dialog),
        // Date / time value types (callable methods)
        "date" => ParsedType::Framework(FrameworkKind::Date),
        "datetime" => ParsedType::Framework(FrameworkKind::DateTime),
        "time" => ParsedType::Framework(FrameworkKind::Time),
        "duration" => ParsedType::Framework(FrameworkKind::Duration),
        // GUID
        "guid" => ParsedType::Framework(FrameworkKind::Guid),
        // Notification / error
        "notification" => ParsedType::Framework(FrameworkKind::Notification),
        "errorinfo" => ParsedType::Framework(FrameworkKind::ErrorInfo),
        // Misc platform value types
        "recordid" => ParsedType::Framework(FrameworkKind::RecordId),
        "moduleinfo" => ParsedType::Framework(FrameworkKind::ModuleInfo),
        "datatransfer" => ParsedType::Framework(FrameworkKind::DataTransfer),
        "sessionsettings" => ParsedType::Framework(FrameworkKind::SessionSettings),
        "filterpagebuilder" => ParsedType::Framework(FrameworkKind::FilterPageBuilder),
        "file" => ParsedType::Framework(FrameworkKind::File),
        "fileupload" => ParsedType::Framework(FrameworkKind::FileUpload),
        "numbersequence" => ParsedType::Framework(FrameworkKind::NumberSequence),
        "version" => ParsedType::Framework(FrameworkKind::Version),
        "controladdin" => ParsedType::Framework(FrameworkKind::ControlAddIn),
        // Variant — runtime-typed, genuinely dynamic
        "variant" => ParsedType::Dynamic,
        // Primitive numeric / boolean types and anything else unrecognized
        _ => ParsedType::Primitive,
    }
}

// ---------------------------------------------------------------------------
// infer_receiver_type
// ---------------------------------------------------------------------------

/// Phase A: infer the [`ReceiverType`] of a member-call receiver expression.
///
/// `receiver_lc` is the lowercased receiver name (simple identifier — compound
/// expressions are handled by the caller before this function is reached).
///
/// Inference order:
/// 1. **Singletons** — `this`, `currpage`/`page`, `currreport`/`report`, and
///    other platform-provided names that are never declared as AL variables.
/// 2. **Variable lookup** — `routine.params` → `routine.locals` →
///    `object_globals`, matched by lowercased name; the declared type is
///    classified via [`classify_type_text`] and names are resolved against the
///    graph.  A variable named `rec`/`xrec` (idiomatic in Codeunits) is found
///    here and classified by its declared type, shadowing the implicit-Rec step.
/// 3. **Implicit Rec / xRec** — only when no variable named `rec`/`xrec` was
///    found in step 2: resolves to the object's implicit record type; returns
///    `Unknown` for object kinds with no implicit record (e.g. Codeunit).
/// 4. **Static framework type name** — bare identifier matching a framework type
///    (`XmlDocument`, `Text`, `File`, `Version`, …) with no variable found;
///    returned as `Framework(kind)`.
/// 5. **Unknown** — no positive typing found.
pub fn infer_receiver_type(
    receiver_lc: &str,
    routine: &RoutineDecl,
    object_globals: &[VarDecl],
    from_object: &ObjectNode,
    graph: &ProgramGraph,
    index: &ResolveIndex,
) -> ReceiverType {
    let from_app = from_object.id.app;

    // -----------------------------------------------------------------------
    // Step 1 — platform singletons (never declared as AL variables).
    // -----------------------------------------------------------------------

    // `this` — the enclosing object instance.
    if receiver_lc == "this" {
        return ReceiverType::SelfObject;
    }

    // Named platform singletons → Framework kind.
    let singleton = match receiver_lc {
        "currpage" | "page" => Some(FrameworkKind::PageInstance),
        "currreport" | "report" => Some(FrameworkKind::ReportInstance),
        "session" => Some(FrameworkKind::Session),
        "navapp" => Some(FrameworkKind::NavApp),
        "database" => Some(FrameworkKind::Database),
        "isolatedstorage" => Some(FrameworkKind::IsolatedStorage),
        "taskscheduler" => Some(FrameworkKind::TaskScheduler),
        "system" => Some(FrameworkKind::System),
        "companyproperty" => Some(FrameworkKind::CompanyProperty),
        "sessioninformation" => Some(FrameworkKind::SessionInformation),
        _ => None,
    };
    if let Some(kind) = singleton {
        return ReceiverType::Framework(kind);
    }

    // -----------------------------------------------------------------------
    // Step 2 — variable lookup (params → locals → object globals).
    //
    // NOTE: `rec`/`xrec` are looked up here too.  A Codeunit routine that
    // declares `var Rec: Record Customer` must resolve to
    // `Record{Some(customer_id)}`, not to `infer_implicit_rec(Codeunit)`
    // which would return `Unknown`.  The implicit-Rec path fires only as a
    // fallback in Step 3 when NO variable named `rec`/`xrec` was found.
    // -----------------------------------------------------------------------

    let declared_ty: Option<&str> = routine
        .params
        .iter()
        .find(|p| p.name.to_ascii_lowercase() == receiver_lc)
        .and_then(|p| p.ty.as_deref())
        .or_else(|| {
            routine
                .locals
                .iter()
                .find(|v| v.name.to_ascii_lowercase() == receiver_lc)
                .and_then(|v| v.ty.as_deref())
        })
        .or_else(|| {
            object_globals
                .iter()
                .find(|v| v.name.to_ascii_lowercase() == receiver_lc)
                .and_then(|v| v.ty.as_deref())
        });

    if let Some(ty) = declared_ty {
        return parsed_type_to_receiver(classify_type_text(ty), from_app, graph, index);
    }

    // -----------------------------------------------------------------------
    // Step 3 — implicit Rec / xRec (fallback: no variable named rec/xrec).
    // -----------------------------------------------------------------------

    if receiver_lc == "rec" || receiver_lc == "xrec" {
        return infer_implicit_rec(from_object, graph, index);
    }

    // -----------------------------------------------------------------------
    // Step 4 — static framework type name used as a static receiver
    // (`XmlDocument.Create(...)`, `Text.CopyStr(...)`, `Version.Create(...)`).
    // A real variable of the same name would have been found in Step 2 and
    // would shadow this path.  Only framework value types classify here;
    // Record/Object/Interface/Enum type names fall through to Unknown.
    // -----------------------------------------------------------------------

    if let ParsedType::Framework(kind) = classify_type_text(receiver_lc) {
        return ReceiverType::Framework(kind);
    }

    // -----------------------------------------------------------------------
    // Step 5 — Unknown.
    // -----------------------------------------------------------------------

    ReceiverType::Unknown
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Resolve the implicit record type for `Rec`/`xRec` based on the enclosing
/// object's kind.
fn infer_implicit_rec(
    from_object: &ObjectNode,
    graph: &ProgramGraph,
    index: &ResolveIndex,
) -> ReceiverType {
    let from_app = from_object.id.app;
    match from_object.id.kind {
        // A Table IS its own record.
        ObjectKind::Table => ReceiverType::Record {
            table: Some(from_object.id.clone()),
        },
        // A TableExtension's implicit record is the base table.
        ObjectKind::TableExtension => {
            let table_id = from_object
                .extends_target
                .as_deref()
                .and_then(|target| resolve_table_id(target, from_app, graph, index));
            ReceiverType::Record { table: table_id }
        }
        // Page / PageExtension / Report / ReportExtension have an implicit Rec
        // but the source table is not on ObjectNode — carry None, note the gap.
        ObjectKind::Page
        | ObjectKind::PageExtension
        | ObjectKind::Report
        | ObjectKind::ReportExtension => ReceiverType::Record { table: None },
        // All other object kinds have no implicit Rec.
        _ => ReceiverType::Unknown,
    }
}

/// Convert a [`ParsedType`] (pure string parse) to a [`ReceiverType`] by
/// resolving names against the graph.
fn parsed_type_to_receiver(
    pt: ParsedType,
    from_app: crate::program::node::AppRef,
    graph: &ProgramGraph,
    index: &ResolveIndex,
) -> ReceiverType {
    match pt {
        ParsedType::Record { table_name } => {
            let table = resolve_table_id(&table_name, from_app, graph, index);
            ReceiverType::Record { table }
        }
        ParsedType::Object { kind, name } => {
            // Resolve the name to get the canonical lowercased name from the
            // graph (handles both name-based and numeric-id references).
            let name_lc = resolve_object_name_lc(kind, &name, from_app, graph, index);
            ReceiverType::Object { kind, name_lc }
        }
        ParsedType::Interface { name } => ReceiverType::Interface { name_lc: name },
        ParsedType::EnumType { name } => ReceiverType::EnumType { name_lc: name },
        ParsedType::RecordRef => ReceiverType::RecordRef,
        ParsedType::FieldRef => ReceiverType::FieldRef,
        ParsedType::KeyRef => ReceiverType::KeyRef,
        ParsedType::Framework(kind) => ReceiverType::Framework(kind),
        ParsedType::Primitive => ReceiverType::Primitive,
        ParsedType::Dynamic => ReceiverType::Dynamic,
    }
}

/// Resolve a table name (which may be a numeric AL object id string) to an
/// `ObjectNodeId`, topology-scoped to `from_app`.
fn resolve_table_id(
    table_name: &str,
    from_app: crate::program::node::AppRef,
    graph: &ProgramGraph,
    index: &ResolveIndex,
) -> Option<ObjectNodeId> {
    // Numeric reference (dependency-app symbol form: `Record 18`).
    if let Ok(n) = table_name.trim().parse::<i64>() {
        return index.object_by_number(graph, from_app, ObjectKind::Table, n);
    }
    // Name-based reference.
    graph
        .resolve_object(from_app, ObjectKind::Table, table_name)
        .map(|o| o.id.clone())
}

/// Resolve an object name (or numeric id string) to the canonical lowercased
/// name of the matching `ObjectNode`, falling back to the lowercased input when
/// no match is found.
fn resolve_object_name_lc(
    kind: ObjectKind,
    name: &str,
    from_app: crate::program::node::AppRef,
    graph: &ProgramGraph,
    index: &ResolveIndex,
) -> String {
    // Numeric reference.
    if let Ok(n) = name.trim().parse::<i64>() {
        if let Some(oid) = index.object_by_number(graph, from_app, kind, n)
            && let Some(obj) = graph.objects.iter().find(|o| o.id == oid)
        {
            return obj.name.to_ascii_lowercase();
        }
        return name.to_ascii_lowercase();
    }
    // Name-based reference.
    if let Some(obj) = graph.resolve_object(from_app, kind, name) {
        return obj.name.to_ascii_lowercase();
    }
    name.to_ascii_lowercase()
}

/// Build a [`ParsedType::Object`] for the given kind and raw name portion.
fn parse_object_kind_type(kind: ObjectKind, name_rest: &str) -> ParsedType {
    // For numeric references like `Codeunit 80`, the name_rest is "80" (no quotes).
    // For named references like `Codeunit "Sales-Post"`, unquote to "Sales-Post".
    let name = unquote_identifier(name_rest);
    ParsedType::Object { kind, name }
}

/// Strip surrounding double-quotes from an identifier token.  Returns the
/// token unchanged if not quoted; returns an empty string for an empty input.
///
/// Port of al-sem `unquoteName`.
pub(crate) fn unquote_identifier(s: &str) -> String {
    let trimmed = s.trim();
    if trimmed.len() >= 2 && trimmed.starts_with('"') && trimmed.ends_with('"') {
        trimmed[1..trimmed.len() - 1].to_string()
    } else {
        trimmed.to_string()
    }
}

/// Strip a trailing `\s+temporary\s*$` modifier (case-insensitive) from a
/// Record type's name portion.  Port of L3's `strip_trailing_temporary`.
fn strip_trailing_temporary(s: &str) -> String {
    let trimmed_end = s.trim_end();
    let lower = trimmed_end.to_lowercase();
    if let Some(prefix_len) = lower.strip_suffix("temporary").map(|p| p.len()) {
        let prefix = &trimmed_end[..prefix_len];
        if prefix.ends_with(char::is_whitespace) {
            return prefix.to_string();
        }
    }
    trimmed_end.to_string()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use al_syntax::ir::{ObjectKind, Origin, Param, Point, RoutineDecl, RoutineKind, VarDecl};

    use crate::program::graph::{ObjectIndex, ProgramGraph};
    use crate::program::node::{AppRef, ObjKey, ObjectNodeId};
    use crate::program::node_extract::ObjectNode;
    use crate::program::resolve::index::ResolveIndex;
    use crate::program::topology::DependencyGraph;
    use crate::snapshot::{AppId, TrustTier};

    // -----------------------------------------------------------------------
    // Test helpers
    // -----------------------------------------------------------------------

    fn test_origin() -> Origin {
        Origin {
            kind_text: "",
            ts_id: 0,
            byte: 0..0,
            start: Point { row: 0, column: 0 },
            end: Point { row: 0, column: 0 },
        }
    }

    /// Build a minimal `ProgramGraph` with one app containing:
    /// - Table "Customer" (declared_id = 18)
    /// - Codeunit "MyCodeunit" (declared_id = 50100)
    /// - A Table with no declared_id, named "SalesHeader" (for extension tests)
    fn build_test_graph() -> (ProgramGraph, AppRef) {
        let mut apps = crate::program::node::AppRegistry::default();
        let app_id = AppId {
            guid: String::new(),
            name: "TestApp".into(),
            publisher: "Test".into(),
            version: "1.0.0.0".into(),
        };
        let app = apps.intern(&app_id);

        let topology = DependencyGraph::default();

        let make_obj =
            |app: AppRef, kind: ObjectKind, name: &str, declared_id: Option<i64>| ObjectNode {
                id: ObjectNodeId {
                    app,
                    kind,
                    key: match declared_id {
                        Some(n) => ObjKey::Id(n),
                        None => ObjKey::Name(name.to_ascii_lowercase()),
                    },
                },
                name: name.to_string(),
                declared_id,
                extends_target: None,
                implements: vec![],
                tier: TrustTier::Workspace,
                source_table: None,
                table_no: None,
                source_table_temporary: false,
                page_controls: vec![],
            };

        let mut objects = vec![
            make_obj(app, ObjectKind::Table, "Customer", Some(18)),
            make_obj(app, ObjectKind::Codeunit, "MyCodeunit", Some(50100)),
            make_obj(app, ObjectKind::Table, "SalesHeader", None),
        ];
        objects.sort_by(|a, b| a.id.cmp(&b.id));

        let obj_index = ObjectIndex::build(&objects);

        let graph = ProgramGraph {
            apps,
            topology,
            objects,
            routines: vec![],
            obj_index,
        };
        (graph, app)
    }

    /// Build a `RoutineDecl` with:
    /// - param `CuParam: Codeunit "MyCodeunit"`
    /// - param `CuNumParam: Codeunit 50100` (numeric id reference)
    /// - local `Cust: Record Customer`
    /// - local `J: JsonObject`
    /// - local `RecTmp: Record Customer temporary`
    fn build_test_routine() -> RoutineDecl {
        let o = test_origin();
        RoutineDecl {
            kind: RoutineKind::Procedure,
            name: "TestProc".into(),
            name_origin: o.clone(),
            params: vec![
                Param {
                    name: "CuParam".into(),
                    by_ref: false,
                    ty: Some("Codeunit \"MyCodeunit\"".into()),
                    origin: o.clone(),
                },
                Param {
                    name: "CuNumParam".into(),
                    by_ref: false,
                    ty: Some("Codeunit 50100".into()),
                    origin: o.clone(),
                },
            ],
            return_type: None,
            locals: vec![
                VarDecl {
                    name: "Cust".into(),
                    ty: Some("Record Customer".into()),
                    temporary: false,
                    origin: o.clone(),
                },
                VarDecl {
                    name: "J".into(),
                    ty: Some("JsonObject".into()),
                    temporary: false,
                    origin: o.clone(),
                },
                VarDecl {
                    name: "RecTmp".into(),
                    ty: Some("Record Customer temporary".into()),
                    temporary: true,
                    origin: o.clone(),
                },
                VarDecl {
                    name: "Iface".into(),
                    ty: Some("Interface \"IMyInterface\"".into()),
                    temporary: false,
                    origin: o.clone(),
                },
                VarDecl {
                    name: "EnumVar".into(),
                    ty: Some("Enum \"Color\"".into()),
                    temporary: false,
                    origin: o.clone(),
                },
            ],
            attributes: vec![],
            attributes_parsed: vec![],
            access_modifier: None,
            parse_incomplete: false,
            dataitem_source_table: None,
            enclosing_member: None,
            body: None,
            origin: o,
        }
    }

    /// Build a `ObjectNode` of the given kind for the test app.
    fn make_object_node(
        app: AppRef,
        kind: ObjectKind,
        name: &str,
        declared_id: Option<i64>,
        extends_target: Option<String>,
    ) -> ObjectNode {
        ObjectNode {
            id: ObjectNodeId {
                app,
                kind,
                key: match declared_id {
                    Some(n) => ObjKey::Id(n),
                    None => ObjKey::Name(name.to_ascii_lowercase()),
                },
            },
            name: name.to_string(),
            declared_id,
            extends_target,
            implements: vec![],
            tier: TrustTier::Workspace,
            source_table: None,
            table_no: None,
            source_table_temporary: false,
            page_controls: vec![],
        }
    }

    // -----------------------------------------------------------------------
    // classify_type_text tests
    // -----------------------------------------------------------------------

    #[test]
    fn classify_record_quoted() {
        assert_eq!(
            classify_type_text("Record \"Customer\""),
            ParsedType::Record {
                table_name: "customer".into()
            }
        );
    }

    #[test]
    fn classify_record_unquoted() {
        assert_eq!(
            classify_type_text("Record Customer"),
            ParsedType::Record {
                table_name: "customer".into()
            }
        );
    }

    #[test]
    fn classify_record_temporary() {
        assert_eq!(
            classify_type_text("Record Customer temporary"),
            ParsedType::Record {
                table_name: "customer".into()
            }
        );
    }

    #[test]
    fn classify_record_quoted_temporary() {
        assert_eq!(
            classify_type_text("Record \"Customer\" temporary"),
            ParsedType::Record {
                table_name: "customer".into()
            }
        );
    }

    #[test]
    fn classify_codeunit_numeric() {
        assert_eq!(
            classify_type_text("Codeunit 80"),
            ParsedType::Object {
                kind: ObjectKind::Codeunit,
                name: "80".into()
            }
        );
    }

    #[test]
    fn classify_codeunit_named() {
        assert_eq!(
            classify_type_text("Codeunit \"Sales-Post\""),
            ParsedType::Object {
                kind: ObjectKind::Codeunit,
                name: "Sales-Post".into()
            }
        );
    }

    #[test]
    fn classify_json_object() {
        assert_eq!(
            classify_type_text("JsonObject"),
            ParsedType::Framework(FrameworkKind::JsonObject)
        );
    }

    #[test]
    fn classify_integer_is_primitive() {
        assert_eq!(classify_type_text("Integer"), ParsedType::Primitive);
    }

    #[test]
    fn classify_variant_is_dynamic() {
        assert_eq!(classify_type_text("Variant"), ParsedType::Dynamic);
    }

    #[test]
    fn classify_interface() {
        assert_eq!(
            classify_type_text("Interface \"IFoo\""),
            ParsedType::Interface {
                name: "ifoo".into()
            }
        );
    }

    #[test]
    fn classify_enum() {
        assert_eq!(
            classify_type_text("Enum \"Color\""),
            ParsedType::EnumType {
                name: "color".into()
            }
        );
    }

    #[test]
    fn classify_text_with_length() {
        assert_eq!(
            classify_type_text("Text[200]"),
            ParsedType::Framework(FrameworkKind::Text)
        );
    }

    #[test]
    fn classify_code_with_length() {
        assert_eq!(
            classify_type_text("Code[20]"),
            ParsedType::Framework(FrameworkKind::Text)
        );
    }

    #[test]
    fn classify_recordref() {
        assert_eq!(classify_type_text("RecordRef"), ParsedType::RecordRef);
    }

    #[test]
    fn classify_fieldref() {
        assert_eq!(classify_type_text("FieldRef"), ParsedType::FieldRef);
    }

    #[test]
    fn classify_keyref() {
        assert_eq!(classify_type_text("KeyRef"), ParsedType::KeyRef);
    }

    #[test]
    fn classify_http_client() {
        assert_eq!(
            classify_type_text("HttpClient"),
            ParsedType::Framework(FrameworkKind::HttpClient)
        );
    }

    #[test]
    fn classify_xml_document() {
        assert_eq!(
            classify_type_text("XmlDocument"),
            ParsedType::Framework(FrameworkKind::Xml)
        );
    }

    #[test]
    fn classify_list() {
        assert_eq!(
            classify_type_text("List of [Integer]"),
            ParsedType::Framework(FrameworkKind::List)
        );
    }

    // Fix 2 — FileUpload / NumberSequence / Version
    #[test]
    fn classify_fileupload() {
        assert_eq!(
            classify_type_text("FileUpload"),
            ParsedType::Framework(FrameworkKind::FileUpload)
        );
    }

    #[test]
    fn classify_numbersequence() {
        assert_eq!(
            classify_type_text("NumberSequence"),
            ParsedType::Framework(FrameworkKind::NumberSequence)
        );
    }

    #[test]
    fn classify_version() {
        assert_eq!(
            classify_type_text("Version"),
            ParsedType::Framework(FrameworkKind::Version)
        );
    }

    // -----------------------------------------------------------------------
    // infer_receiver_type tests
    // -----------------------------------------------------------------------

    #[test]
    fn infer_local_record_resolves_table_id() {
        let (graph, app) = build_test_graph();
        let index = ResolveIndex::build(&graph);
        let routine = build_test_routine();
        let from_obj = make_object_node(app, ObjectKind::Codeunit, "CallerCu", Some(999), None);

        // "cust" → local `Cust: Record Customer` → table Customer resolved
        let customer_node = graph
            .resolve_object(app, ObjectKind::Table, "Customer")
            .unwrap();
        let expected_id = customer_node.id.clone();

        let result = infer_receiver_type("cust", &routine, &[], &from_obj, &graph, &index);
        assert_eq!(
            result,
            ReceiverType::Record {
                table: Some(expected_id)
            }
        );
    }

    #[test]
    fn infer_local_record_temporary_resolves_table_id() {
        let (graph, app) = build_test_graph();
        let index = ResolveIndex::build(&graph);
        let routine = build_test_routine();
        let from_obj = make_object_node(app, ObjectKind::Codeunit, "CallerCu", Some(999), None);

        let customer_node = graph
            .resolve_object(app, ObjectKind::Table, "Customer")
            .unwrap();
        let expected_id = customer_node.id.clone();

        // "rectmp" → local `RecTmp: Record Customer temporary` → same resolution
        let result = infer_receiver_type("rectmp", &routine, &[], &from_obj, &graph, &index);
        assert_eq!(
            result,
            ReceiverType::Record {
                table: Some(expected_id)
            }
        );
    }

    #[test]
    fn infer_local_json_object() {
        let (graph, app) = build_test_graph();
        let index = ResolveIndex::build(&graph);
        let routine = build_test_routine();
        let from_obj = make_object_node(app, ObjectKind::Codeunit, "CallerCu", Some(999), None);

        let result = infer_receiver_type("j", &routine, &[], &from_obj, &graph, &index);
        assert_eq!(result, ReceiverType::Framework(FrameworkKind::JsonObject));
    }

    #[test]
    fn infer_param_codeunit_by_name() {
        let (graph, app) = build_test_graph();
        let index = ResolveIndex::build(&graph);
        let routine = build_test_routine();
        let from_obj = make_object_node(app, ObjectKind::Codeunit, "CallerCu", Some(999), None);

        // "cuparam" → param `CuParam: Codeunit "MyCodeunit"` → Object{Codeunit, "mycodunit"}
        let result = infer_receiver_type("cuparam", &routine, &[], &from_obj, &graph, &index);
        assert_eq!(
            result,
            ReceiverType::Object {
                kind: ObjectKind::Codeunit,
                name_lc: "mycodeunit".into()
            }
        );
    }

    #[test]
    fn infer_param_codeunit_by_number() {
        let (graph, app) = build_test_graph();
        let index = ResolveIndex::build(&graph);
        let routine = build_test_routine();
        let from_obj = make_object_node(app, ObjectKind::Codeunit, "CallerCu", Some(999), None);

        // "cunumparam" → param `CuNumParam: Codeunit 50100` → resolves to "mycodeunit"
        let result = infer_receiver_type("cunumparam", &routine, &[], &from_obj, &graph, &index);
        assert_eq!(
            result,
            ReceiverType::Object {
                kind: ObjectKind::Codeunit,
                name_lc: "mycodeunit".into()
            }
        );
    }

    #[test]
    fn infer_singleton_currpage() {
        let (graph, app) = build_test_graph();
        let index = ResolveIndex::build(&graph);
        let routine = build_test_routine();
        let from_obj = make_object_node(app, ObjectKind::Page, "MyPage", Some(50200), None);

        let result = infer_receiver_type("currpage", &routine, &[], &from_obj, &graph, &index);
        assert_eq!(result, ReceiverType::Framework(FrameworkKind::PageInstance));
    }

    #[test]
    fn infer_singleton_page() {
        let (graph, app) = build_test_graph();
        let index = ResolveIndex::build(&graph);
        let routine = build_test_routine();
        let from_obj = make_object_node(app, ObjectKind::Codeunit, "CallerCu", Some(999), None);

        // bare "page" singleton
        let result = infer_receiver_type("page", &routine, &[], &from_obj, &graph, &index);
        assert_eq!(result, ReceiverType::Framework(FrameworkKind::PageInstance));
    }

    #[test]
    fn infer_singleton_currreport() {
        let (graph, app) = build_test_graph();
        let index = ResolveIndex::build(&graph);
        let routine = build_test_routine();
        let from_obj = make_object_node(app, ObjectKind::Report, "MyReport", Some(50300), None);

        let result = infer_receiver_type("currreport", &routine, &[], &from_obj, &graph, &index);
        assert_eq!(
            result,
            ReceiverType::Framework(FrameworkKind::ReportInstance)
        );
    }

    #[test]
    fn infer_singleton_session() {
        let (graph, app) = build_test_graph();
        let index = ResolveIndex::build(&graph);
        let routine = build_test_routine();
        let from_obj = make_object_node(app, ObjectKind::Codeunit, "CallerCu", Some(999), None);

        let result = infer_receiver_type("session", &routine, &[], &from_obj, &graph, &index);
        assert_eq!(result, ReceiverType::Framework(FrameworkKind::Session));
    }

    #[test]
    fn infer_singleton_database() {
        let (graph, app) = build_test_graph();
        let index = ResolveIndex::build(&graph);
        let routine = build_test_routine();
        let from_obj = make_object_node(app, ObjectKind::Codeunit, "CallerCu", Some(999), None);

        let result = infer_receiver_type("database", &routine, &[], &from_obj, &graph, &index);
        assert_eq!(result, ReceiverType::Framework(FrameworkKind::Database));
    }

    #[test]
    fn infer_this_is_self_object() {
        let (graph, app) = build_test_graph();
        let index = ResolveIndex::build(&graph);
        let routine = build_test_routine();
        let from_obj = make_object_node(app, ObjectKind::Codeunit, "CallerCu", Some(999), None);

        let result = infer_receiver_type("this", &routine, &[], &from_obj, &graph, &index);
        assert_eq!(result, ReceiverType::SelfObject);
    }

    #[test]
    fn infer_rec_in_table_is_self() {
        let (graph, app) = build_test_graph();
        let index = ResolveIndex::build(&graph);
        let routine = build_test_routine();
        // from_object IS the Customer table
        let customer_node = graph
            .resolve_object(app, ObjectKind::Table, "Customer")
            .unwrap()
            .clone();

        let result = infer_receiver_type("rec", &routine, &[], &customer_node, &graph, &index);
        assert_eq!(
            result,
            ReceiverType::Record {
                table: Some(customer_node.id.clone())
            }
        );
    }

    #[test]
    fn infer_xrec_in_table_is_self() {
        let (graph, app) = build_test_graph();
        let index = ResolveIndex::build(&graph);
        let routine = build_test_routine();
        let customer_node = graph
            .resolve_object(app, ObjectKind::Table, "Customer")
            .unwrap()
            .clone();

        let result = infer_receiver_type("xrec", &routine, &[], &customer_node, &graph, &index);
        assert_eq!(
            result,
            ReceiverType::Record {
                table: Some(customer_node.id.clone())
            }
        );
    }

    #[test]
    fn infer_rec_in_table_extension_resolves_base() {
        let (graph, app) = build_test_graph();
        let index = ResolveIndex::build(&graph);
        let routine = build_test_routine();
        // A TableExtension extending "Customer"
        let te_obj = make_object_node(
            app,
            ObjectKind::TableExtension,
            "CustomerExt",
            Some(50400),
            Some("Customer".into()),
        );

        let customer_node = graph
            .resolve_object(app, ObjectKind::Table, "Customer")
            .unwrap();
        let expected_id = customer_node.id.clone();

        let result = infer_receiver_type("rec", &routine, &[], &te_obj, &graph, &index);
        assert_eq!(
            result,
            ReceiverType::Record {
                table: Some(expected_id)
            }
        );
    }

    #[test]
    fn infer_rec_in_page_is_record_none() {
        let (graph, app) = build_test_graph();
        let index = ResolveIndex::build(&graph);
        let routine = build_test_routine();
        let page_obj = make_object_node(app, ObjectKind::Page, "CustomerCard", Some(21), None);

        let result = infer_receiver_type("rec", &routine, &[], &page_obj, &graph, &index);
        assert_eq!(result, ReceiverType::Record { table: None });
    }

    #[test]
    fn infer_rec_in_codeunit_is_unknown() {
        let (graph, app) = build_test_graph();
        let index = ResolveIndex::build(&graph);
        let routine = build_test_routine();
        let cu_obj = make_object_node(app, ObjectKind::Codeunit, "CallerCu", Some(999), None);

        let result = infer_receiver_type("rec", &routine, &[], &cu_obj, &graph, &index);
        assert_eq!(result, ReceiverType::Unknown);
    }

    #[test]
    fn infer_unknown_name_is_unknown() {
        let (graph, app) = build_test_graph();
        let index = ResolveIndex::build(&graph);
        let routine = build_test_routine();
        let from_obj = make_object_node(app, ObjectKind::Codeunit, "CallerCu", Some(999), None);

        let result = infer_receiver_type(
            "notdeclaredanywhere",
            &routine,
            &[],
            &from_obj,
            &graph,
            &index,
        );
        assert_eq!(result, ReceiverType::Unknown);
    }

    #[test]
    fn infer_object_globals_lookup() {
        let (graph, app) = build_test_graph();
        let index = ResolveIndex::build(&graph);
        let routine = build_test_routine();
        let from_obj = make_object_node(app, ObjectKind::Codeunit, "CallerCu", Some(999), None);

        let o = test_origin();
        let globals = vec![VarDecl {
            name: "GlobalCu".into(),
            ty: Some("Codeunit \"MyCodeunit\"".into()),
            temporary: false,
            origin: o,
        }];

        let result = infer_receiver_type("globalcu", &routine, &globals, &from_obj, &graph, &index);
        assert_eq!(
            result,
            ReceiverType::Object {
                kind: ObjectKind::Codeunit,
                name_lc: "mycodeunit".into()
            }
        );
    }

    #[test]
    fn infer_local_interface_type() {
        let (graph, app) = build_test_graph();
        let index = ResolveIndex::build(&graph);
        let routine = build_test_routine();
        let from_obj = make_object_node(app, ObjectKind::Codeunit, "CallerCu", Some(999), None);

        let result = infer_receiver_type("iface", &routine, &[], &from_obj, &graph, &index);
        assert_eq!(
            result,
            ReceiverType::Interface {
                name_lc: "imyinterface".into()
            }
        );
    }

    #[test]
    fn infer_local_enum_type() {
        let (graph, app) = build_test_graph();
        let index = ResolveIndex::build(&graph);
        let routine = build_test_routine();
        let from_obj = make_object_node(app, ObjectKind::Codeunit, "CallerCu", Some(999), None);

        let result = infer_receiver_type("enumvar", &routine, &[], &from_obj, &graph, &index);
        assert_eq!(
            result,
            ReceiverType::EnumType {
                name_lc: "color".into()
            }
        );
    }

    #[test]
    fn infer_param_shadows_globals() {
        // A param and a global with the same lowercased name — param wins.
        let (graph, app) = build_test_graph();
        let index = ResolveIndex::build(&graph);
        let routine = build_test_routine();
        let from_obj = make_object_node(app, ObjectKind::Codeunit, "CallerCu", Some(999), None);

        let o = test_origin();
        // Global also named "CuParam" but with a different type
        let globals = vec![VarDecl {
            name: "CuParam".into(),
            ty: Some("JsonObject".into()),
            temporary: false,
            origin: o,
        }];

        // Should resolve via the PARAM (Codeunit "MyCodeunit"), not the global (JsonObject)
        let result = infer_receiver_type("cuparam", &routine, &globals, &from_obj, &graph, &index);
        assert_eq!(
            result,
            ReceiverType::Object {
                kind: ObjectKind::Codeunit,
                name_lc: "mycodeunit".into()
            }
        );
    }

    #[test]
    fn infer_record_unresolvable_table_is_record_none() {
        // A local `var R: Record "NonExistentTable"` — Record{None} not Unknown
        let (graph, app) = build_test_graph();
        let index = ResolveIndex::build(&graph);
        let o = test_origin();
        let routine_with_dep_record = RoutineDecl {
            kind: RoutineKind::Procedure,
            name: "P".into(),
            name_origin: o.clone(),
            params: vec![],
            return_type: None,
            locals: vec![VarDecl {
                name: "R".into(),
                ty: Some("Record \"NonExistentTable\"".into()),
                temporary: false,
                origin: o.clone(),
            }],
            attributes: vec![],
            attributes_parsed: vec![],
            access_modifier: None,
            parse_incomplete: false,
            dataitem_source_table: None,
            enclosing_member: None,
            body: None,
            origin: o,
        };

        let from_obj = make_object_node(app, ObjectKind::Codeunit, "CallerCu", Some(999), None);
        let result = infer_receiver_type(
            "r",
            &routine_with_dep_record,
            &[],
            &from_obj,
            &graph,
            &index,
        );
        // Record with unresolvable table → Record{None} (not Unknown)
        assert_eq!(result, ReceiverType::Record { table: None });
    }

    // Fix 1 — rec/xrec variable lookup before implicit-rec
    #[test]
    fn infer_rec_local_in_codeunit_resolves_via_variable() {
        // A Codeunit routine with `var Rec: Record Customer` — `Rec.SetRange(...)`
        // must resolve to Record{Some(customer_id)}, NOT Unknown (which was the
        // bug: the old code hit infer_implicit_rec(Codeunit) → Unknown before the
        // variable lookup).
        let (graph, app) = build_test_graph();
        let index = ResolveIndex::build(&graph);
        let o = test_origin();
        let routine_with_rec_local = RoutineDecl {
            kind: RoutineKind::Procedure,
            name: "TestRecLocal".into(),
            name_origin: o.clone(),
            params: vec![],
            return_type: None,
            locals: vec![VarDecl {
                name: "Rec".into(),
                ty: Some("Record Customer".into()),
                temporary: false,
                origin: o.clone(),
            }],
            attributes: vec![],
            attributes_parsed: vec![],
            access_modifier: None,
            parse_incomplete: false,
            dataitem_source_table: None,
            enclosing_member: None,
            body: None,
            origin: o,
        };
        let cu_obj = make_object_node(app, ObjectKind::Codeunit, "CallerCu", Some(999), None);

        let customer_node = graph
            .resolve_object(app, ObjectKind::Table, "Customer")
            .unwrap();
        let expected_id = customer_node.id.clone();

        // receiver "rec" (lc) → local variable `Rec: Record Customer` → Record{Some(customer_id)}
        let result =
            infer_receiver_type("rec", &routine_with_rec_local, &[], &cu_obj, &graph, &index);
        assert_eq!(
            result,
            ReceiverType::Record {
                table: Some(expected_id)
            }
        );
    }

    // Fix 3 — static framework type name as receiver
    #[test]
    fn infer_static_xml_document_receiver() {
        // `XmlDocument.Create(...)` — bare `XmlDocument` with no matching variable
        // must type as Framework(Xml), not Unknown.
        let (graph, app) = build_test_graph();
        let index = ResolveIndex::build(&graph);
        let routine = build_test_routine();
        let from_obj = make_object_node(app, ObjectKind::Codeunit, "CallerCu", Some(999), None);

        let result = infer_receiver_type("xmldocument", &routine, &[], &from_obj, &graph, &index);
        assert_eq!(result, ReceiverType::Framework(FrameworkKind::Xml));
    }

    #[test]
    fn infer_static_text_receiver() {
        // `Text.CopyStr(...)` — bare `Text` with no matching variable must type
        // as Framework(Text), not Unknown.
        let (graph, app) = build_test_graph();
        let index = ResolveIndex::build(&graph);
        let routine = build_test_routine();
        let from_obj = make_object_node(app, ObjectKind::Codeunit, "CallerCu", Some(999), None);

        let result = infer_receiver_type("text", &routine, &[], &from_obj, &graph, &index);
        assert_eq!(result, ReceiverType::Framework(FrameworkKind::Text));
    }
}
