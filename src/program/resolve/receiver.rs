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
//! 0. **`CurrPage.<part>.Page` subpage-instance receivers** — a page control's
//!    (`part(<part>; <SubPage>)`) SUBPAGE INSTANCE, distinct from the CONTROL
//!    itself (`CurrPage.<part>` with no `.Page`, which addresses structural
//!    control methods like `.Update`/`.Visible` and is NOT resolved here).
//!    Only the exact `<part>.Page` shape (one control segment, one trailing
//!    `.Page` accessor) resolves, and only for a `Part` control whose target
//!    resolves unambiguously; a `SystemPart`/`UserControl`, a bare part, a
//!    deeper chain, or an unresolved/ambiguous target all fall through to
//!    `Unknown` (see [`infer_receiver_type`]'s Step 0).
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
//!    (Table self-id, TableExtension base, Page/PageExtension via `SourceTable`,
//!    Codeunit via `TableNo` — topology-aware, fail-closed through
//!    `ResolveIndex::resolve_object_ref`, see [`infer_implicit_rec`] — or
//!    `Record{None}` for Report/ReportExtension, whose implicit Rec is
//!    per-dataitem scoped rather than object-level and is not yet modeled).
//!    A Codeunit with no `TableNo` declared at all (including `Subtype =
//!    Test`/`TestRunner`, which never declares one) has no implicit-Rec
//!    entity to type and returns `Unknown`; every other object kind not
//!    listed above (Report/ReportExtension aside) also returns `Unknown`.
//! 4. **Static framework type name** — when the receiver name matches a framework
//!    type name (e.g. `XmlDocument`, `Text`, `File`, `Version`) and no variable was
//!    found, type it as `Framework(kind)` so Phase B dispatches the static method
//!    via the builtin catalog.
//! 5. **Compound call-result receiver (`Func().Method()`)** — beyond-1B.3b
//!    Task 3. Only engages when `receiver_expr` carries a structured
//!    `ExprKind::Call{function, args}` node whose `function` is a BARE
//!    identifier (never dotted/member — a `Obj.Method().X()` cross-object
//!    chain declines HERE, at Step 5, and falls through to Step 6's
//!    cross-object-chain arm instead, plan v2.1 Task 3). Fail-closed, in order:
//!    (a) a caller param/local/global named identically to `function` SHADOWS
//!    it in AL (`resolve_bare` cannot see variables) — any such shadow
//!    declines immediately; (b) otherwise `function` is typed by calling
//!    [`crate::program::resolve::resolver::resolve_bare`] as a TYPE QUERY
//!    (reusing its own-object/extension-base/implicit-Rec/builtin precedence,
//!    ambiguity declines, and with-guard) — anything other than exactly one
//!    route to a `RouteTarget::Routine` declines; (c) that routine's
//!    `graph.routines[..].return_type` must be `Some` and parse (via
//!    [`classify_type_text`]) to a non-`Primitive` shape — a `None` or scalar
//!    return declines; the parsed type is then resolved to a receiver exactly
//!    like Step 2's declared-variable path (via [`parsed_type_to_receiver`]),
//!    inheriting its fail-closed cross-app-ambiguous-object decline. Only
//!    engaged when the caller passes a `bare_ctx` (full end-to-end resolution
//!    via `resolve_full_program`); callers with no `BodyMap`/`WithState` in
//!    scope (tests, `semantic_golden.rs`) pass `None` and this step is a no-op
//!    — resolution-neutral for them, exactly like `receiver_expr` itself.
//! 6. **Compound framework property/method + `this.<rest>` receiver**
//!    (beyond-1B.3b Task 4). Only engages when `receiver_expr` (Task 2) is
//!    populated — unlike Step 5, this step does NOT need `bare_ctx` (it never
//!    calls `resolve_bare`), so it also fires for callers that supply
//!    `receiver_expr` but not `bare_ctx`. Two independent AST-based sub-cases,
//!    both operating on the STRUCTURED `Expr` node (never `receiver_text`
//!    string-splitting): (a) `<Framework>.<Prop>` / `<Framework>.<Method(..)>`
//!    — the receiver is `ExprKind::Member{object, member}` (property form) or
//!    `ExprKind::Call{function: Member{object, member}, args}` (method-call
//!    form); `object` is recursively typed via the AST-native
//!    [`infer_receiver_type_for_expr`] helper, and if it resolves to
//!    `Framework(kind)`, the versioned [`framework_return_kind`] table maps
//!    `(kind, member_lc, is_method, arity)` to the returned kind — a table
//!    miss (wrong member, wrong form, wrong arity) declines. (b) `this.<rest>`
//!    — when `object` is the bare `this` identifier, `member` is resolved
//!    against a SELF-ONLY scope (`object_globals` only — never
//!    `routine.params`/`routine.locals`, per AL's documented `this.` semantics
//!    of addressing only "methods and globals within the same object"); a
//!    `this.<method>(..)` CALL form (dispatching a same-object procedure's
//!    return type) is deliberately NOT handled here — declines — since typing
//!    it correctly needs `resolve_bare`-style routine lookup, out of this
//!    step's scope. See [`infer_receiver_type_for_expr`] for the full
//!    recursion.
//! 7. **Unknown** — no positive typing found.
//!
//! # Clean-room note
//!
//! This mirrors the logic of L3's `infer_receiver_type` in
//! `src/engine/l3/receiver_type.rs` but is written fresh over the IR
//! (`RoutineDecl`/`VarDecl`/`Param`) and `ProgramGraph`/`ResolveIndex`, carrying
//! `ObjectNodeId`s instead of L3 string IDs.

use al_syntax::ir::{AlFile, ExprId, ExprKind, ObjectKind, RoutineDecl, VarDecl};

use crate::program::graph::ProgramGraph;
use crate::program::node::{ObjectNodeId, RoutineNodeId};
use crate::program::node_extract::{
    ObjectNode, ObjectRef, PageControlKind, PageControlNode, RoutineNode,
};
use crate::program::resolve::body_map::BodyMap;
use crate::program::resolve::edge::RouteTarget;
use crate::program::resolve::extract::WithState;
use crate::program::resolve::framework_returns::framework_return_kind;
use crate::program::resolve::index::{ObjectRefResolution, ResolveIndex};
use crate::program::resolve::resolver::{
    resolve_bare, resolve_member, routine_node_for_type_query,
};

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
    Object {
        kind: ObjectKind,
        name_lc: String,
        /// The resolved target's `ObjectNodeId`, when Phase A already proved
        /// it MECHANICALLY (Task 7's `CurrPage.<part>.Page` subpage-instance
        /// Step 0, via the fail-closed `ResolveIndex::resolve_object_ref`) —
        /// carried through so `resolve_member`'s `Object` arm can
        /// short-circuit on it directly instead of re-resolving `name_lc`
        /// against the graph a second time (which could in principle land on
        /// a different object than the one Step 0 actually verified unique).
        /// `None` for every other `Object` receiver (declared-variable /
        /// param / global lookup via [`classify_type_text`]), which still
        /// resolves by name in `resolve_member` as before.
        id: Option<ObjectNodeId>,
    },
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
    /// `Record <TableName>` — the table reference, LOSSLESSLY shaped: a
    /// numeric AL object id (`Record 18`) is [`ObjectRef::Id`]; a name
    /// (quoted or not, `Record Customer` / `Record "Customer"`) is
    /// [`ObjectRef::Name`], stripped of quotes and a trailing ` temporary`
    /// modifier. Distinguishing the two shapes here (rather than collapsing
    /// both to a bare string) is the I1 Caller-A fix: `Record "18"` (a table
    /// literally NAMED "18") must never be silently coerced into numeric id
    /// 18 by a later stringly-typed re-parse.
    Record { table_ref: ObjectRef },
    /// `Codeunit X` / `Page X` / `Report X` / `Query X` / `XmlPort X` — object
    /// kind and the object reference, LOSSLESSLY shaped exactly like
    /// `Record`'s `table_ref` above (this is the I1 mirror for Caller-A's
    /// object-typed sibling): a numeric AL object id (`Codeunit 80`) is
    /// [`ObjectRef::Id`]; a name (quoted or not, `Codeunit "Sales-Post"` /
    /// `Codeunit MyMgt`) is [`ObjectRef::Name`]. Distinguishing the two shapes
    /// here — rather than collapsing both to a bare string — is required so
    /// `Codeunit 80` (numeric id 80) and `Codeunit "80"` (a codeunit literally
    /// NAMED "80") can never be conflated by a later stringly-typed re-parse.
    Object {
        kind: ObjectKind,
        object_ref: ObjectRef,
    },
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
            // Parse the table reference: strip trailing " temporary", then
            // shape-classify — a numeric string is an `Id`, ANYTHING else
            // (including a QUOTED numeric string, since the quote characters
            // make it fail the `i64` parse before unquoting) is a `Name`.
            // Mirrors `node_extract::parse_object_ref_value`'s identical
            // numeric-vs-quoted-name distinction for `SourceTable`/`TableNo`.
            let stripped = strip_trailing_temporary(rest);
            let stripped = stripped.trim();
            let table_ref = if let Ok(n) = stripped.parse::<i64>() {
                ObjectRef::Id(n)
            } else {
                let raw = unquote_identifier(stripped);
                let normalized_lc = raw.to_ascii_lowercase();
                ObjectRef::Name { raw, normalized_lc }
            };
            ParsedType::Record { table_ref }
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
/// `receiver_lc` is the lowercased receiver text: usually a simple identifier,
/// but Step 0 also recognizes the `currpage.<part>.page` compound form (a
/// subpage-instance receiver) — any other compound expression that reaches here
/// unrecognized falls through to `Unknown` (fail-closed).
///
/// Inference order:
/// 0. **`CurrPage.<part>.Page` subpage-instance receivers** — see the module
///    doc's Step 0. Checked first because it is a COMPOUND (dotted) receiver
///    text that none of steps 1-4 would otherwise positively type.
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
/// 5. **Compound call-result receiver (`Func().Method()`)** — see the module
///    doc's Step 5. Requires both `receiver_expr` (Task 2) and `bare_ctx`
///    (Task 3) to be populated; a no-op otherwise.
/// 6. **Compound framework property/method + `this.<rest>` receiver** — see
///    the module doc's Step 6. Requires only `receiver_expr` (Task 2); a
///    no-op otherwise.
/// 7. **Unknown** — no positive typing found.
///
/// # `receiver_expr` (Task 2 enabling primitive)
///
/// `receiver_expr` carries the PARSED receiver `Expr` — `Some((file, id))` when
/// the call site's [`CalleeShape::Member`] populated a `receiver` `ExprId`
/// (`file.ir.expr(id)` recovers the structured node: `ExprKind::Call{..}` /
/// `Member{..}` / …), `None` when the caller has no such node in scope (e.g.
/// the `RecordOp` shape, which does not carry one). Steps 0-4 are UNCHANGED by
/// this parameter and continue to dispatch purely on `receiver_lc`; Step 5
/// (Task 3) is the first consumer.
///
/// # `bare_ctx` (Task 3 enabling primitive)
///
/// `Some((body_map, with_state))` when the caller can supply the two extra
/// inputs Step 5 needs to run [`crate::program::resolve::resolver::resolve_bare`]
/// as a type query (`resolve_full_program`'s real `CalleeShape::Member`
/// resolution path); `None` for callers with no such context in scope (unit
/// tests, `semantic_golden.rs`, the `RecordOp` shape) — Step 5 is then a
/// no-op, resolution-neutral exactly like `receiver_expr` for those callers.
///
/// [`CalleeShape::Member`]: crate::program::resolve::extract::CalleeShape::Member
#[allow(clippy::too_many_arguments)] // 6 pre-existing params + `bare_ctx` (Task 3); each is a distinct identity/lookup input, grouping would obscure call sites (mirrors `resolve_in_object`'s precedent).
pub fn infer_receiver_type(
    receiver_lc: &str,
    routine: &RoutineDecl,
    object_globals: &[VarDecl],
    from_object: &ObjectNode,
    graph: &ProgramGraph,
    index: &ResolveIndex,
    receiver_expr: Option<(&AlFile, ExprId)>,
    bare_ctx: Option<(&BodyMap<'_>, WithState)>,
) -> ReceiverType {
    // -----------------------------------------------------------------------
    // Step 0 — `CurrPage.<part>.Page` subpage-instance receivers (Task 7).
    //
    // A page's `part(<part>; <SubPage>)` control's SUBPAGE INSTANCE is
    // accessed as `CurrPage.<part>.Page.<method>()`; resolving `<part>.Page`
    // to the target Page object lets `resolve_member`'s ordinary `Object` arm
    // dispatch the subpage's user procedures. This is DISTINCT from
    // `CurrPage.<part>.<method>()` (no `.Page`), which addresses the CONTROL
    // itself (structural methods like `.Update`/`.Visible`) — that shape
    // falls through to `Unknown` here, never fabricated as a subpage call.
    // `SystemPart`/`UserControl` controls and any chain deeper than one
    // `.Page` accessor also fall through: a wrong subpage is a false
    // `Source` edge, the cardinal sin, so anything short of an exact
    // single-segment `<part>.Page` shape resolving to exactly one in-closure
    // Page object declines rather than guesses.
    // -----------------------------------------------------------------------
    if let Some(rest) = receiver_lc.strip_prefix("currpage.")
        && let Some(part_name_lc) = parse_currpage_dot_page_segment(rest)
        && let Some(control) = find_page_control(&part_name_lc, from_object, graph, index)
        && control.kind == PageControlKind::Part
        && let ObjectRefResolution::Unique(page_id) = index.resolve_object_ref(
            graph,
            from_object.id.clone(),
            ObjectKind::Page,
            &control.target,
        )
        && let Some(page_obj) = graph.objects.iter().find(|o| o.id == page_id)
    {
        return ReceiverType::Object {
            kind: ObjectKind::Page,
            name_lc: page_obj.name.to_ascii_lowercase(),
            id: Some(page_id),
        };
    }

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
        return parsed_type_to_receiver(classify_type_text(ty), from_object, graph, index);
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
    // Step 5 — compound call-result receiver (`Func().Method()`, Task 3).
    //
    // Only engages when BOTH `receiver_expr` (the parsed receiver node, Task
    // 2) and `bare_ctx` (the `BodyMap`/`WithState` Step 5 needs to run
    // `resolve_bare` as a type query, Task 3) are populated — a no-op
    // otherwise, so callers that don't supply them (unit tests,
    // `semantic_golden.rs`, the `RecordOp` shape) are unaffected.
    // -----------------------------------------------------------------------

    if let Some((file, expr_id)) = receiver_expr
        && let Some((body_map, with_state)) = bare_ctx
        && let Some(recv) = infer_call_result_receiver(
            file,
            expr_id,
            routine,
            object_globals,
            from_object,
            graph,
            index,
            body_map,
            with_state,
        )
    {
        return recv;
    }

    // -----------------------------------------------------------------------
    // Step 6 — compound framework property/method + `this.<rest>` receiver
    // (beyond-1B.3b Task 4) + cross-object call-result chain receiver
    // (`Var.Method().X()`, plan v2.1 Task 3 — see [`infer_compound_member_receiver`]'s
    // new arm).
    //
    // The framework/`this.<rest>` sub-cases only need `receiver_expr` (Task
    // 2) — unlike Step 5, they never call `resolve_bare`, so they do NOT
    // gate on `bare_ctx`. The NEW cross-object-chain sub-case DOES need a
    // `BodyMap` (it calls `resolve_member` as a type-query, which needs one
    // to build routes) — threaded here as `Option<&BodyMap<'_>>` extracted
    // from `bare_ctx`, so it is a no-op for callers with no `bare_ctx` in
    // scope (unit tests that pass `None`, `semantic_golden.rs`, the
    // `RecordOp` shape), exactly like Step 5, while the framework/`this.`
    // sub-cases remain resolution-neutral either way.
    // -----------------------------------------------------------------------

    if let Some((file, expr_id)) = receiver_expr {
        let recv = infer_receiver_type_for_expr(
            file,
            expr_id,
            routine,
            object_globals,
            from_object,
            graph,
            index,
            bare_ctx.map(|(body_map, _)| body_map),
        );
        if !matches!(recv, ReceiverType::Unknown) {
            return recv;
        }
    }

    // -----------------------------------------------------------------------
    // Step 7 — Unknown.
    // -----------------------------------------------------------------------

    ReceiverType::Unknown
}

/// Step 6's AST-native entry point: type an arbitrary `Expr` node directly
/// from the IR — never by re-parsing source text — recursing through
/// `Member`/`Call` chains to find a `Framework`-typed base for the compound
/// framework-property/method step, or the bare `this` identifier for the
/// `this.<rest>` step (both in [`infer_compound_member_receiver`]).
///
/// Dispatch:
/// - `Identifier`/`QuotedIdentifier` — the base case: type it exactly like a
///   bare receiver name via [`infer_receiver_type`]'s Steps 0-4 (`receiver_expr`
///   and `bare_ctx` both `None` — this deliberately does NOT recurse into
///   Steps 5-6 again for a bare identifier; Step 4's `rec`/singleton/framework
///   lookup is Step 6's whole base case, so recursing further here would only
///   ever re-derive the same `Unknown` a second time, never additional
///   coverage. Terminates by construction — no cycle risk).
///
///   **Quote-parity guard (round-2 fix):** the IR's `QuotedIdentifier(name)`
///   stores `name` ALREADY UNQUOTED (the lowerer strips quotes — see
///   `extract.rs`'s `classify_call`), whereas the TOP-LEVEL `receiver_lc`
///   [`infer_receiver_type`] itself dispatches on is sliced from RAW SOURCE
///   TEXT and so ALWAYS retains any quote characters. Feeding the bare
///   unquoted name into a fresh `infer_receiver_type` call would therefore
///   run Steps 0-4 on a DIFFERENT string than the top-level call would have
///   seen for the same site — concretely, Step 4's naive first-whitespace-
///   token match (`classify_type_text`) can then spuriously match a quoted
///   FIELD name that merely STARTS WITH a framework keyword word (e.g. a
///   `Blob` field literally named `"File Blob"` unquotes to `"file blob"`,
///   whose first token `"file"` collides with the `File` framework type —
///   verified as a REAL CDO false-positive during this task's CDO gate: the
///   table's own implicit-Rec field `"File Blob"` was mis-typed
///   `Framework(File)` and `.CreateInStream`/`.CreateOutStream` false-
///   resolved to the `File` catalog instead of staying the honest
///   `Unknown` a Blob FIELD reference correctly is (field-type indexing is
///   the DEFERRED record-field mechanism, module doc Step 6a's sibling
///   note) — the cardinal sin this whole plan exists to prevent). So a
///   `QuotedIdentifier` is RE-QUOTED before the recursive call, exactly
///   reproducing what `receiver_text.to_ascii_lowercase()` would have
///   produced for the same source site — restoring BYTE-FOR-BYTE parity
///   with Steps 0-4's existing (conservative) quoted-name behavior, never
///   granting quoted names new resolving power Task 4 doesn't intend to add.
/// - `Member{object, member, ..}` — the property-access form (`<base>.<member>`,
///   no parens): delegate to [`infer_compound_member_receiver`] with
///   `is_method: false`, `arity: 0`.
/// - `Call{function, args}` whose `function` derefs to `Member{object, member,
///   ..}` — the method-call form (`<base>.<member>(args)`): delegate to
///   [`infer_compound_member_receiver`] with `is_method: true`,
///   `arity: args.len()`. A `Call` whose `function` is anything else (a bare
///   identifier call, i.e. the Step-5 shape already handled at the TOP level
///   only — not recursively here) declines.
/// - Anything else (`Index`, `Literal`, `Binary`, …) — declines. Fail-closed by
///   construction: every arm either delegates to more fail-closed logic or
///   returns `Unknown` directly.
///
/// `body_map` (plan v2.1 Task 3 enabling primitive): `Some` when the caller
/// can supply the `BodyMap` [`infer_compound_member_receiver`]'s new
/// cross-object call-result chain arm needs to run `resolve_member` as a
/// type-query; `None` for callers with no such context in scope — that arm
/// is then a no-op there, exactly like [`infer_receiver_type`]'s `bare_ctx`.
/// Threaded unchanged through every recursive call so a multi-hop chain's
/// BASE typing (itself possibly another compound receiver) can reach the new
/// arm too — a 3-level chain whose middle hop cannot be typed (no
/// `body_map`, or the middle hop itself declines) correctly propagates
/// `Unknown` rather than partially guessing.
#[allow(clippy::too_many_arguments)] // 7 pre-existing params + `body_map` (plan v2.1 Task 3); each is a distinct identity/lookup input, grouping would obscure the recursive call sites.
fn infer_receiver_type_for_expr(
    file: &AlFile,
    expr_id: ExprId,
    routine: &RoutineDecl,
    object_globals: &[VarDecl],
    from_object: &ObjectNode,
    graph: &ProgramGraph,
    index: &ResolveIndex,
    body_map: Option<&BodyMap<'_>>,
) -> ReceiverType {
    match &file.ir.expr(expr_id).kind {
        ExprKind::Identifier(name) => {
            let name_lc = name.to_ascii_lowercase();
            infer_receiver_type(
                &name_lc,
                routine,
                object_globals,
                from_object,
                graph,
                index,
                None,
                None,
            )
        }
        ExprKind::QuotedIdentifier(name) => {
            // Quote-parity guard — see this function's doc. Re-quote so
            // Steps 0-4 see EXACTLY the string the top-level `receiver_lc`
            // (sliced from raw source text) would have carried for the same
            // site, never a bare unquoted name a quoted field/var reference
            // never actually is.
            let requoted_lc = format!("\"{}\"", name.to_ascii_lowercase());
            infer_receiver_type(
                &requoted_lc,
                routine,
                object_globals,
                from_object,
                graph,
                index,
                None,
                None,
            )
        }
        ExprKind::Member { object, member, .. } => infer_compound_member_receiver(
            file,
            *object,
            member,
            false,
            0,
            routine,
            object_globals,
            from_object,
            graph,
            index,
            body_map,
        ),
        ExprKind::Call { function, args } => {
            if let ExprKind::Member { object, member, .. } = &file.ir.expr(*function).kind {
                infer_compound_member_receiver(
                    file,
                    *object,
                    member,
                    true,
                    args.len(),
                    routine,
                    object_globals,
                    from_object,
                    graph,
                    index,
                    body_map,
                )
            } else {
                // A bare-identifier call (`Func(...)`) reaching HERE (i.e. as
                // the BASE of a deeper chain, not the top-level receiver) is
                // the Step-5 shape recursed one level deeper than Step 5
                // handles — deliberately out of scope (single-hop
                // `<Framework>.<rest>`/`this.<rest>`/cross-object chains
                // target the OUTER receiver only, not nested bare-call
                // chains); decline rather than guess.
                ReceiverType::Unknown
            }
        }
        _ => ReceiverType::Unknown,
    }
}

/// Step 6's shared implementation for both sub-cases (framework-property/method
/// chain and `this.<rest>`) — dispatches on whether `object_expr_id` is
/// literally the bare `this` identifier.
///
/// - **`this.<rest>`**: when `object_expr_id` derefs to `Identifier`/
///   `QuotedIdentifier` matching `"this"` (case-insensitively — AL identifiers
///   are case-insensitive), `is_method: true` (a `this.Method(...)` CALL form)
///   declines immediately — deliberately DEFERRED (see the module doc's Step
///   6b): typing a same-object procedure's return type needs
///   `resolve_bare`-style routine lookup, out of this step's scope. The
///   property form (`is_method: false`) resolves `member` via
///   [`infer_this_member`] against the SELF-ONLY `object_globals` scope.
/// - **Framework chain**: recursively type `object_expr_id` via
///   [`infer_receiver_type_for_expr`]; if it resolves to `Framework(kind)`,
///   look up `(kind, member_lc, is_method, arity)` in the versioned
///   [`framework_return_kind`] table. A table miss falls through to the
///   cross-object-chain arm below (never declines early — a `Framework`
///   base has no source/ABI procedures to type-query, but falling through
///   costs nothing and keeps this dispatch a single funnel).
/// - **Cross-object call-result chain** (plan v2.1 Task 3): STRICTLY the
///   procedure-CALL form (`is_method`; a bare `Member` — a field/property
///   access — is never this arm, round-1 I7). When `base_ty` is `Object`/
///   `Record`/`SelfObject`/`Interface` (proven by the SAME recursive typing
///   above) and a `body_map` is available, types the base call's RETURN
///   TYPE via a PURE [`resolve_member`] type-query — see
///   [`infer_cross_object_chain_receiver`] for the full guard. Untyped/
///   `Unknown`/`Primitive`/`Dynamic`/`*Ref` bases, or any decline along the
///   way, fall through to `Unknown` — never a partial guess.
#[allow(clippy::too_many_arguments)] // mirrors infer_receiver_type_for_expr's identity/lookup inputs plus member/is_method/arity — grouping would obscure the dispatch.
fn infer_compound_member_receiver(
    file: &AlFile,
    object_expr_id: ExprId,
    member: &str,
    is_method: bool,
    arity: usize,
    routine: &RoutineDecl,
    object_globals: &[VarDecl],
    from_object: &ObjectNode,
    graph: &ProgramGraph,
    index: &ResolveIndex,
    body_map: Option<&BodyMap<'_>>,
) -> ReceiverType {
    // `member` (from `ExprKind::Member`/`Call{function: Member{..}}`) may
    // itself be RAW WITH QUOTES (mirrors `extract.rs::classify_call`'s own
    // `strip_quote_chars(member)` before use) — unquote before matching so a
    // quoted member name (`Response."Content"()`, however rare in practice)
    // normalizes the same way an unquoted one does, rather than silently
    // missing the table via a stray embedded quote character.
    let member_lc = unquote_identifier(member).to_ascii_lowercase();

    if is_this_identifier(file, object_expr_id) {
        if is_method {
            // `this.Method(...)` call-result chaining — deferred, decline.
            return ReceiverType::Unknown;
        }
        return infer_this_member(&member_lc, object_globals, from_object, graph, index);
    }

    let base_ty = infer_receiver_type_for_expr(
        file,
        object_expr_id,
        routine,
        object_globals,
        from_object,
        graph,
        index,
        body_map,
    );

    if let ReceiverType::Framework(kind) = &base_ty {
        if let Some(returned) = framework_return_kind(kind, &member_lc, is_method, arity) {
            return ReceiverType::Framework(returned);
        }
        return ReceiverType::Unknown;
    }

    // Cross-object call-result chain (plan v2.1 Task 3) — see this
    // function's doc. `is_method` gates the shape (procedure-CALL form
    // only); `body_map` gates on the caller having supplied one
    // (resolution-neutral otherwise, mirrors Step 5's `bare_ctx` gate).
    if is_method
        && let Some(bm) = body_map
        && matches!(
            base_ty,
            ReceiverType::Object { .. }
                | ReceiverType::Record { .. }
                | ReceiverType::SelfObject
                | ReceiverType::Interface { .. }
        )
        && let Some(recv) = infer_cross_object_chain_receiver(
            &base_ty,
            &member_lc,
            arity,
            from_object,
            graph,
            index,
            bm,
        )
    {
        return recv;
    }

    ReceiverType::Unknown
}

/// Cross-object call-result chain (plan v2.1 Task 3): type a `Var.Method()`
/// PREFIX's result by a PURE [`resolve_member`] type-query on `base_ty`,
/// converting the resolved procedure's declared return type to a
/// [`ReceiverType`] exactly like Step 2's declared-variable path
/// ([`parsed_type_to_receiver`]).
///
/// # Route-count guard
///
/// `resolve_member(base_ty, member_lc, arity, ..)` must yield EXACTLY ONE
/// [`crate::program::resolve::edge::Route`]. For `Object`/`Record`/
/// `SelfObject` bases this is `resolve_member`'s own unconditional contract
/// (every arm of its `match` returns a single-element `Vec`). For an
/// `Interface` base it fans out to every implementer — exactly one route
/// means exactly one implementer in the closed-world closure; more than one
/// (a genuinely polymorphic prefix) declines here, never a guessed pick.
///
/// A route whose target carries no routine identity at all
/// (`RouteTarget::Unresolved` — arity mismatch/ambiguous overload/access
/// excluded — or `RouteTarget::Builtin`, a platform-intrinsic method with no
/// modeled return type) also declines: there is nothing to read a
/// `return_type` from.
///
/// # Single-implementer interface prefix
///
/// Once the route-count guard already passed (exactly one implementer),
/// PREFERS reading the return type from the INTERFACE's own declared method
/// signature when the graph models one ([`interface_own_routine_node`]) —
/// AL requires every implementer's signature to match the interface's
/// exactly, so this can never be a looser answer than the implementer's, and
/// sidesteps needing to know the implementer's own tier/ABI-ness. Falls back
/// to the resolved implementer's own routine node
/// ([`routine_node_for_type_query`], which also applies the ABI-PREFIX
/// UNIQUENESS GUARD for an `AbiSymbol` target) when the interface's own
/// signature isn't modeled.
fn infer_cross_object_chain_receiver(
    base_ty: &ReceiverType,
    member_lc: &str,
    arity: usize,
    from_object: &ObjectNode,
    graph: &ProgramGraph,
    index: &ResolveIndex,
    body_map: &BodyMap<'_>,
) -> Option<ReceiverType> {
    let (_shape, routes) = resolve_member(
        base_ty,
        member_lc,
        arity,
        from_object,
        graph,
        index,
        body_map,
    );
    let [route] = routes.as_slice() else {
        return None;
    };
    if matches!(
        route.target,
        RouteTarget::Unresolved | RouteTarget::Builtin(_)
    ) {
        return None;
    }

    if let ReceiverType::Interface { name_lc } = base_ty
        && let Some(node) =
            interface_own_routine_node(name_lc, member_lc, arity, from_object, graph, index)
    {
        return receiver_from_routine_node(node, from_object, graph, index);
    }

    let node = routine_node_for_type_query(route, arity, from_object, graph, index)?;
    receiver_from_routine_node(node, from_object, graph, index)
}

/// The interface's OWN declared member signature (name+arity match), when
/// modeled — see [`infer_cross_object_chain_receiver`]'s doc. Interface
/// members carry no access modifier in AL (they are always the public
/// contract), so no visibility filtering applies here (unlike
/// `resolve_member`'s implementer dispatch). `None` when the interface
/// object itself is not resolvable from `from_object`'s app, or zero/more-
/// than-one same-arity candidate is declared (defensive — a single interface
/// declaration should never itself be arity-ambiguous, but this never
/// guesses).
fn interface_own_routine_node<'g>(
    name_lc: &str,
    member_lc: &str,
    arity: usize,
    from_object: &ObjectNode,
    graph: &'g ProgramGraph,
    index: &ResolveIndex,
) -> Option<&'g RoutineNode> {
    let iface = graph.resolve_object(from_object.id.app, ObjectKind::Interface, name_lc)?;
    let matched: Vec<&RoutineNodeId> = index
        .routines_in_object(&iface.id, member_lc)
        .iter()
        .filter(|rid| rid.params_count == arity)
        .collect();
    let [rid] = matched.as_slice() else {
        return None;
    };
    graph
        .routines
        .binary_search_by(|probe| probe.id.cmp(rid))
        .ok()
        .map(|i| &graph.routines[i])
}

/// Convert a resolved prefix routine's declared return type into a
/// [`ReceiverType`] — the shared tail of [`infer_cross_object_chain_receiver`]'s
/// two paths (interface's own signature, or the resolved implementer/routine).
///
/// Declines (`None`) on: no declared return type; a scalar/primitive return
/// (`classify_type_text` → `ParsedType::Primitive`); or — Task 2's structured
/// cross-validation — an ABI-sourced return type whose `Subtype` `(name, id)`
/// pair disagrees with the object the name resolves to (`node.return_type_id`
/// is `Some` only for an ABI/SymbolOnly-ingested routine whose declared
/// Subtype carried both fields; applies uniformly regardless of which
/// `RouteTarget` shape supplied `node`, per `AbiRoutine::return_type_id`'s
/// doc). Cross-validation only applies when the parsed return type resolved
/// to an `Object`/`Record` (the only shapes carrying a resolved
/// `ObjectNodeId` to check an id against); any other shape (`Interface`,
/// `EnumType`, `Framework`, …) has no identity to cross-check and passes
/// through unconditionally — those shapes carry no risk of a false `Source`
/// edge to a WRONG object.
fn receiver_from_routine_node(
    node: &RoutineNode,
    from_object: &ObjectNode,
    graph: &ProgramGraph,
    index: &ResolveIndex,
) -> Option<ReceiverType> {
    let return_type = node.return_type.as_deref()?;
    let parsed = classify_type_text(return_type);
    if matches!(parsed, ParsedType::Primitive) {
        return None;
    }
    let receiver = parsed_type_to_receiver(parsed, from_object, graph, index);

    if let Some((_name, id)) = &node.return_type_id {
        let resolved_obj = match &receiver {
            ReceiverType::Object { id: Some(oid), .. } => {
                graph.objects.iter().find(|o| &o.id == oid)
            }
            ReceiverType::Record {
                table: Some(table_id),
            } => graph.objects.iter().find(|o| &o.id == table_id),
            _ => None,
        };
        match resolved_obj {
            Some(obj) if obj.declared_id == Some(*id) => {}
            _ => return None,
        }
    }

    Some(receiver)
}

/// `true` when `expr_id` derefs to a bare `this` identifier (case-insensitive
/// — AL identifiers are case-insensitive), the ONLY shape the `this.<rest>`
/// step (module doc Step 6b) recognizes. A `"this"` `QuotedIdentifier` (i.e.
/// written `"this"` with quotes, which in AL would refer to a DIFFERENTLY
/// -named symbol, not the self-reference keyword) is deliberately EXCLUDED —
/// only the unquoted keyword form is the self-reference.
fn is_this_identifier(file: &AlFile, expr_id: ExprId) -> bool {
    matches!(
        &file.ir.expr(expr_id).kind,
        ExprKind::Identifier(name) if name.eq_ignore_ascii_case("this")
    )
}

/// `this.<rest>` member resolution (module doc Step 6b): resolve `member_lc`
/// against the SELF-ONLY scope AL's `this` keyword actually permits — object
/// GLOBALS only (`object_globals`), never `routine.params`/`routine.locals`.
///
/// Per Microsoft's AL language documentation ("Use the `this` keyword for
/// codeunit self-reference"), `this` is a self-reference allowing a symbol
/// reference to be "a member of the object itself"; the System Application's
/// own adoption note describes it as "referencing methods and globals within
/// the same object". Locals and parameters are NOT members of the object —
/// they belong to the routine's own stack frame — so `this.` cannot address
/// them; a same-named local/param simply does not shadow a global reached via
/// `this.` (that is the entire point of the keyword: disambiguating from a
/// same-named local). This function only ever resolves `member_lc` against
/// `object_globals`, matching that documented scope exactly — never `routine`
/// at all. See `tests/r0-corpus/ws-compound-framework/PROOF.md` for the full
/// citation (no AL compiler was available in this task's execution
/// environment; the semantics above are spec-stated per Microsoft Learn, not
/// `alc`-verified).
///
/// `this.<method>(...)` (a CALL, dispatching a same-object PROCEDURE's return
/// type) is handled by the caller ([`infer_compound_member_receiver`]),
/// which declines before ever reaching here — this function is reached only
/// for the property form.
fn infer_this_member(
    member_lc: &str,
    object_globals: &[VarDecl],
    from_object: &ObjectNode,
    graph: &ProgramGraph,
    index: &ResolveIndex,
) -> ReceiverType {
    let Some(ty) = object_globals
        .iter()
        .find(|v| v.name.to_ascii_lowercase() == member_lc)
        .and_then(|v| v.ty.as_deref())
    else {
        return ReceiverType::Unknown;
    };
    parsed_type_to_receiver(classify_type_text(ty), from_object, graph, index)
}

/// Step 5's implementation: type a `Func().Method()` compound receiver by the
/// return type of the bare same-scope function `Func()`.
///
/// `expr_id` must dereference (via `file.ir.expr`) to a structured
/// `ExprKind::Call{function, args}` node — the receiver of the OUTER member
/// call (`.Method()`), i.e. the `Func(...)` sub-expression.  Every other
/// shape reaching here (a `Member` function — the `Obj.Method().X()`
/// cross-object chain — or anything else) declines to `None` (fail-closed;
/// Step 5 is not the shape's home). A `Member`-function shape specifically
/// then falls through to Step 6's cross-object-chain arm (plan v2.1 Task 3),
/// which may resolve it; anything else genuinely falls through to `Unknown`.
///
/// Fail-closed at every step (see the module doc's Step 5 for the full
/// rationale):
/// 1. **Bare-identifier guard** — `function` must be `Identifier`/
///    `QuotedIdentifier`; a dotted/member function chain declines.
/// 2. **Local-shadowing guard** (round-2 gemini critical, checked BEFORE
///    typing) — `resolve_bare` resolves ROUTINE calls and cannot see
///    locals/params/globals, but in AL a same-named variable SHADOWS a
///    same-named procedure. If `function_lc` matches ANY of
///    `routine.params`/`routine.locals`/`object_globals`, decline — this
///    plan does not type variable-backed receivers (e.g. a local ARRAY named
///    `GetCustomer` makes `GetCustomer(1)` an index access, not a call).
/// 3. **`resolve_bare` type query** — call `resolve_bare` with `function_lc`
///    and `args.len()` as the arity; require the SINGLE returned `Route` (its
///    contract: always exactly one) to target `RouteTarget::Routine` — this
///    reuses `resolve_bare`'s own-object/extension-base/implicit-Rec/builtin
///    precedence, its same-arity-overload-ambiguity decline, its
///    builtin/intrinsic PROBE-THEN-DECIDE collision guard, and its
///    `with`-guard, for free. A `Builtin`/`AbiSymbol`/`Unresolved` target
///    (name absent, arity mismatch, ambiguous overload, or an unproven
///    builtin/Rec-shadow precedence collision) declines.
/// 4. **Non-scalar return-type guard** — the resolved routine's
///    `return_type` must be `Some` and parse (via [`classify_type_text`]) to
///    a non-`Primitive` shape; `None` (no declared return type) or a scalar
///    primitive (`Integer`, `Boolean`, …) declines — nothing to dispatch a
///    member call on.
/// 5. **Type conversion** — the parsed return type is resolved to a
///    [`ReceiverType`] via [`parsed_type_to_receiver`], the SAME
///    graph/`ResolveIndex`-backed, fail-closed conversion Step 2's
///    declared-variable path uses: a cross-app-ambiguous `Record`/`Object`
///    return inherits that path's decline-to-`None` (never guess), and an
///    `Interface` return becomes `ReceiverType::Interface` (Phase B fans out
///    to every implementer — polymorphic, never a concrete guess).
#[allow(clippy::too_many_arguments)] // 9 distinct identity/lookup inputs mirror `resolve_in_object`'s precedent; grouping would obscure call sites.
fn infer_call_result_receiver(
    file: &AlFile,
    expr_id: ExprId,
    routine: &RoutineDecl,
    object_globals: &[VarDecl],
    from_object: &ObjectNode,
    graph: &ProgramGraph,
    index: &ResolveIndex,
    body_map: &BodyMap<'_>,
    with_state: WithState,
) -> Option<ReceiverType> {
    // 0. Must be a structured Call whose function is a BARE identifier — a
    //    Member function (`Obj.Method()`) is the cross-object-chain shape
    //    Step 6 handles instead (plan v2.1 Task 3) and declines here.
    let ExprKind::Call { function, args } = &file.ir.expr(expr_id).kind else {
        return None;
    };
    let function_lc = match &file.ir.expr(*function).kind {
        ExprKind::Identifier(name) | ExprKind::QuotedIdentifier(name) => name.to_ascii_lowercase(),
        _ => return None,
    };

    // 1. Local-shadowing guard FIRST — see the doc above.
    let shadowed = routine
        .params
        .iter()
        .any(|p| p.name.to_ascii_lowercase() == function_lc)
        || routine
            .locals
            .iter()
            .any(|v| v.name.to_ascii_lowercase() == function_lc)
        || object_globals
            .iter()
            .any(|v| v.name.to_ascii_lowercase() == function_lc);
    if shadowed {
        return None;
    }

    // 2. Type-query `function_lc` via `resolve_bare`. Contract: always
    //    exactly one `Route`; only a `Routine` target is usable here.
    let routes = resolve_bare(
        from_object,
        &function_lc,
        args.len(),
        graph,
        index,
        body_map,
        with_state,
    );
    let [route] = routes.as_slice() else {
        return None;
    };
    let RouteTarget::Routine(ref rid) = route.target else {
        return None;
    };

    // 3. Non-scalar return-type guard.
    //
    // Task-3 review finding (folded in by Task 4): `graph.routines` is kept
    // sorted by `RoutineNodeId` (the same invariant `resolver.rs`'s
    // `lookup_routine_access`/`make_routine_route` rely on) — an O(n) linear
    // `.find` here was a needless scan when a `binary_search_by` mirrors that
    // existing idiom exactly, for both consistency and scaling.
    let return_type = graph
        .routines
        .binary_search_by(|probe| probe.id.cmp(rid))
        .ok()
        .map(|i| &graph.routines[i])?
        .return_type
        .as_deref()?;
    let parsed = classify_type_text(return_type);
    if matches!(parsed, ParsedType::Primitive) {
        return None;
    }

    // 4. Convert the parsed return type to a receiver — same fail-closed
    //    conversion Step 2's declared-variable path uses.
    Some(parsed_type_to_receiver(parsed, from_object, graph, index))
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
    match from_object.id.kind {
        // A Table IS its own record.
        ObjectKind::Table => ReceiverType::Record {
            table: Some(from_object.id.clone()),
        },
        // A TableExtension's implicit record is the base table (Caller B).
        // Resolution goes through the same fail-closed
        // `ResolveIndex::resolve_object_ref` as Page's `SourceTable` below —
        // a guessed base table is the cardinal sin (I1), so anything short
        // of a single unambiguous in-closure match stays `Record{table:
        // None}`.
        ObjectKind::TableExtension => ReceiverType::Record {
            table: resolve_tableext_base_table(from_object, graph, index),
        },
        // A Page's implicit Rec is typed by its own `SourceTable` property
        // (Task 5). Resolution goes through the fail-closed
        // `ResolveIndex::resolve_object_ref`: a guessed table is the cardinal
        // sin (a wrong table produces a false `Source` edge), so anything
        // short of a single unambiguous in-closure match stays
        // `Record{table: None}` — builtins (SetRange/FindSet/…) still resolve
        // table-independently in Phase B; only a non-builtin method call on a
        // table-less Record becomes the honest `Unknown`.
        ObjectKind::Page => ReceiverType::Record {
            table: from_object
                .source_table
                .as_ref()
                .and_then(|r| resolve_source_table_ref(from_object.id.clone(), r, graph, index)),
        },
        // A PageExtension may declare its own `SourceTable`; when it does not,
        // its implicit Rec follows the BASE page's `SourceTable` instead — the
        // `extends` target is resolved to exactly one in-closure Page first
        // (same fail-closed rule), then that page's `source_table` is read and
        // resolved the same way. An own `SourceTable` that fails to resolve
        // does NOT fall through to the base page — it explicitly overrides the
        // base, so a failed override stays `None` rather than silently
        // reverting to inherited behavior.
        ObjectKind::PageExtension => {
            let table = if let Some(r) = &from_object.source_table {
                resolve_source_table_ref(from_object.id.clone(), r, graph, index)
            } else {
                resolve_pageext_base_source_table(from_object, graph, index)
            };
            ReceiverType::Record { table }
        }
        // A Codeunit's implicit Rec is typed by its own `TableNo` property
        // (Task 6 — the direct analog of Task 5's Page/`SourceTable` fix).
        // Unlike Page (which ALWAYS has an implicit Rec, typed or not), a
        // Codeunit only gets an implicit Rec when `TableNo` is declared at
        // all — `None` here means there is no implicit-Rec entity to type,
        // so this stays the honest `Unknown` (not `Record{table: None}`).
        // `Subtype = Test`/`TestRunner` codeunits fall into this same `None`
        // arm: they never declare `TableNo` (no statically-typed implicit
        // Rec — unhandled even in the legacy L3 engine), so nothing is
        // fabricated for them; `ObjectNode` does not track `Subtype` at all,
        // deliberately, since the `TableNo`-presence check alone already
        // produces the correct honest decline.
        //
        // When `TableNo` IS declared, resolution goes through the same
        // fail-closed `ResolveIndex::resolve_object_ref` as Page's
        // `SourceTable`, and mirrors its non-`Unique` treatment: a single
        // unambiguous in-closure match yields `Record{table: Some(id)}`;
        // anything else (cross-app ambiguity, out-of-closure, unresolved)
        // stays `Record{table: None}` rather than guessing — a wrong table
        // is a false `Source` edge, the cardinal sin. Builtins
        // (SetRange/FindSet/…) still resolve table-independently in Phase B
        // either way; only a non-builtin method call on a table-less Record
        // becomes the honest `Unknown`.
        ObjectKind::Codeunit => match &from_object.table_no {
            Some(r) => ReceiverType::Record {
                table: resolve_source_table_ref(from_object.id.clone(), r, graph, index),
            },
            None => ReceiverType::Unknown,
        },
        // Report / ReportExtension: EXCLUDED for now. A report's implicit Rec
        // is scoped PER-DATAITEM (each `dataitem(...)` block sources its own
        // table; a report can have several, nested), not a single object-level
        // `SourceTable` the way Page/PageExtension are. Resolving this
        // correctly needs dataitem-scope tracking (which dataitem encloses the
        // routine) that the graph does not carry yet — a future task. Until
        // then this stays the honest `Record{table: None}` rather than
        // guessing e.g. the outermost dataitem's table.
        ObjectKind::Report | ObjectKind::ReportExtension => ReceiverType::Record { table: None },
        // All other object kinds have no implicit Rec.
        _ => ReceiverType::Unknown,
    }
}

/// Resolve an object's `SourceTable` [`ObjectRef`] to a table `ObjectNodeId`,
/// scoped from `from`'s dependency closure via the fail-closed
/// [`ResolveIndex::resolve_object_ref`]. Only [`ObjectRefResolution::Unique`]
/// yields a table; `Ambiguous`/`OutOfClosure`/`Unresolved` all decline to
/// `None` rather than guess.
///
/// `pub(crate)`: also reused directly by `resolver.rs`'s `resolve_bare` Step 3
/// (beyond-1B.3b Task 3) for the Page implicit-Rec table lookup — the exact
/// same fail-closed rule the EXPLICIT `Rec.Foo()` receiver-inference path
/// (this module) already established for Tasks 5-7.
pub(crate) fn resolve_source_table_ref(
    from: ObjectNodeId,
    source_table: &ObjectRef,
    graph: &ProgramGraph,
    index: &ResolveIndex,
) -> Option<ObjectNodeId> {
    match index.resolve_object_ref(graph, from, ObjectKind::Table, source_table) {
        ObjectRefResolution::Unique(id) => Some(id),
        ObjectRefResolution::Ambiguous
        | ObjectRefResolution::OutOfClosure
        | ObjectRefResolution::Unresolved => None,
    }
}

/// Resolve a TableExtension's `extends_target` to the base Table's
/// `ObjectNodeId`, scoped from `from_object`'s own dependency closure via the
/// fail-closed [`ResolveIndex::resolve_object_ref`] (Caller B, I1). `None`
/// when there is no `extends_target`, or resolution is anything other than
/// `Unique` (ambiguous, out-of-closure, unresolved) — never guess. Mirrors
/// [`resolve_pageext_base_page`]'s template, `ObjectKind::Table` instead of
/// `ObjectKind::Page`. `extends_target` is always a NAME in AL grammar (a
/// TableExtension cannot `extends` by numeric id), so this always builds an
/// [`ObjectRef::Name`], unlike `SourceTable`/`TableNo` which may be numeric.
///
/// `pub(crate)`: also reused directly by `resolver.rs`'s `resolve_bare` Step 3
/// (beyond-1B.3b Task 3) for the TableExtension implicit-Rec table lookup —
/// literally "`resolve_object_ref(Table, extends_target)`" as the task brief
/// specifies, via this existing helper rather than re-deriving it.
pub(crate) fn resolve_tableext_base_table(
    from_object: &ObjectNode,
    graph: &ProgramGraph,
    index: &ResolveIndex,
) -> Option<ObjectNodeId> {
    let extends = from_object.extends_target.as_deref()?;
    let base_ref = ObjectRef::Name {
        raw: extends.to_string(),
        normalized_lc: extends.to_ascii_lowercase(),
    };
    resolve_source_table_ref(from_object.id.clone(), &base_ref, graph, index)
}

/// Resolve a PageExtension's `extends_target` to the base Page's
/// `ObjectNodeId`, scoped from `from_object`'s own dependency closure via the
/// fail-closed [`ResolveIndex::resolve_object_ref`]. `None` when there is no
/// `extends_target`, or resolution is anything other than `Unique`
/// (ambiguous, out-of-closure, unresolved) — never guess. Shared by
/// [`resolve_pageext_base_source_table`] (Task 5's implicit-`Rec` base-page
/// lookup) and [`find_page_control`] (Task 7's PageExtension control merge).
fn resolve_pageext_base_page(
    from_object: &ObjectNode,
    graph: &ProgramGraph,
    index: &ResolveIndex,
) -> Option<ObjectNodeId> {
    let extends = from_object.extends_target.as_deref()?;
    let base_ref = ObjectRef::Name {
        raw: extends.to_string(),
        normalized_lc: extends.to_ascii_lowercase(),
    };
    match index.resolve_object_ref(graph, from_object.id.clone(), ObjectKind::Page, &base_ref) {
        ObjectRefResolution::Unique(id) => Some(id),
        ObjectRefResolution::Ambiguous
        | ObjectRefResolution::OutOfClosure
        | ObjectRefResolution::Unresolved => None,
    }
}

/// Resolve a PageExtension's inherited `SourceTable`: follow `extends_target`
/// to exactly one in-closure base Page, then read and resolve THAT page's own
/// `source_table`. Any decline at either hop (ambiguous extends target,
/// extends target out of closure/unresolved, base page has no `SourceTable`,
/// or the base page's `SourceTable` itself fails to resolve) yields `None`.
///
/// Both hops are scoped from `from_object`'s own closure (the extension's),
/// consistent with every other lookup in this module keying off the CALLING
/// object's app — not the base page's.
///
/// `pub(crate)`: also reused directly by `resolver.rs`'s `resolve_bare` Step 3
/// (beyond-1B.3b Task 3) for the PageExtension implicit-Rec table lookup.
pub(crate) fn resolve_pageext_base_source_table(
    from_object: &ObjectNode,
    graph: &ProgramGraph,
    index: &ResolveIndex,
) -> Option<ObjectNodeId> {
    let base_id = resolve_pageext_base_page(from_object, graph, index)?;
    let base_page = graph.objects.iter().find(|o| o.id == base_id)?;
    resolve_source_table_ref(
        from_object.id.clone(),
        base_page.source_table.as_ref()?,
        graph,
        index,
    )
}

/// Find a `CurrPage.<part>` layout control by lowercased name, in the set
/// visible to `from_object`: its own `page_controls` first; for a
/// `PageExtension` with no matching control of its own, also the extended
/// BASE page's controls (merged — mirrors L3's `symbol_table::
/// page_controls_for`), resolved via the fail-closed
/// [`resolve_pageext_base_page`] rather than a raw name lookup. An own
/// PageExtension control of the same name always shadows the base page's
/// (checked first, short-circuits before the base-page hop).
///
/// Returns an owned clone — `PageControlNode` is small (`Vec`-backed) and
/// this sidesteps unifying the lifetime of a borrow from `from_object` with
/// one from `graph.objects` in a single return type.
fn find_page_control(
    name_lc: &str,
    from_object: &ObjectNode,
    graph: &ProgramGraph,
    index: &ResolveIndex,
) -> Option<PageControlNode> {
    if let Some(c) = from_object
        .page_controls
        .iter()
        .find(|c| c.name_lc == name_lc)
    {
        return Some(c.clone());
    }
    if from_object.id.kind != ObjectKind::PageExtension {
        return None;
    }
    let base_id = resolve_pageext_base_page(from_object, graph, index)?;
    let base_page = graph.objects.iter().find(|o| o.id == base_id)?;
    base_page
        .page_controls
        .iter()
        .find(|c| c.name_lc == name_lc)
        .cloned()
}

/// Parse the text following `"currpage."` (already lowercased by the caller)
/// for the `<part>.page` subpage-instance shape (Task 7): a single, possibly
/// quoted, control-name segment followed by EXACTLY one trailing `.page`
/// accessor and nothing else. Returns the control name, quotes stripped
/// (already lowercase since the input is).
///
/// Returns `None` — decline, honest `Unknown` — for: a bare part with no
/// `.page` accessor (`CurrPage.Lines` — the CONTROL, distinct from the
/// subpage INSTANCE); a chain deeper than one `.page` accessor
/// (`CurrPage.Lines.Page.Foo`); or any other shape.
fn parse_currpage_dot_page_segment(rest: &str) -> Option<String> {
    let (segment, remainder) = if let Some(after_quote) = rest.strip_prefix('"') {
        // Quoted control name: the segment runs to the next `"`. An escaped
        // `""` literal-quote inside the name is not handled here (matching
        // this module's existing `unquote_identifier`, which doesn't either)
        // — such a name simply fails the `page_controls` lookup and declines.
        let close = after_quote.find('"')?;
        (&after_quote[..close], &after_quote[close + 1..])
    } else {
        match rest.find('.') {
            Some(i) => (&rest[..i], &rest[i..]),
            None => (rest, ""),
        }
    };
    if segment.is_empty() || remainder != ".page" {
        return None;
    }
    Some(segment.to_string())
}

/// Convert a [`ParsedType`] (pure string parse) to a [`ReceiverType`] by
/// resolving names against the graph.
///
/// `from_object` (rather than a bare `AppRef`) is required so the `Record`
/// arm can drive [`ResolveIndex::resolve_object_ref`] (needs the full
/// `ObjectNodeId`, not just the app) — the fail-closed, shape-preserving
/// resolution Caller A needs (I1).
fn parsed_type_to_receiver(
    pt: ParsedType,
    from_object: &ObjectNode,
    graph: &ProgramGraph,
    index: &ResolveIndex,
) -> ReceiverType {
    match pt {
        ParsedType::Record { table_ref } => {
            // Reuses the same fail-closed, shape-preserving helper Task 5's
            // Page `SourceTable` resolution uses: `resolve_object_ref`'s
            // `Id`/`Name` arms already dispatch on `table_ref`'s shape
            // (`ObjectRef::Id`/`Name` — losslessly carried from
            // `classify_type_text`), so `Record 18` and `Record "18"` can
            // never be conflated, and >1 in-closure dependency match DECLINES
            // to `None` rather than guessing (I1).
            let table = resolve_source_table_ref(from_object.id.clone(), &table_ref, graph, index);
            ReceiverType::Record { table }
        }
        ParsedType::Object { kind, object_ref } => {
            // Task 2 (mirrors I1): the SAME fail-closed, shape-preserving
            // `resolve_object_ref` the `Record` arm above uses — `object_ref`
            // is losslessly shaped (`ObjectRef::Id`/`Name`) by
            // `parse_object_kind_type`, so `Codeunit 80` and `Codeunit "80"`
            // can never be conflated here either. A `Unique` resolution
            // carries the resolved `id` UP FRONT, so `resolve_member`'s
            // `Object` arm short-circuits on it directly (mirrors Task 7's
            // `CurrPage.<part>.Page` carried-id short-circuit) instead of
            // re-deriving it from `name_lc` — no redundant second lookup for
            // the (common) resolved case.
            let (id, name_lc) =
                resolve_object_ref_lc(kind, &object_ref, from_object.id.clone(), graph, index);
            ReceiverType::Object { kind, name_lc, id }
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

/// Resolve a losslessly-shaped [`ObjectRef`] (Task 2, mirrors I1) to a target
/// `ObjectNodeId` and its canonical lowercased name, via the same fail-closed,
/// dependency-closure-scoped [`ResolveIndex::resolve_object_ref`] the `Record`
/// arm's `SourceTable`/`TableNo` resolution (Tasks 5/6) already uses.
///
/// A numeric AL object id (`Codeunit 80`) is never conflated with a codeunit
/// literally NAMED `"80"` (`Codeunit "80"`) the way the old
/// `name.trim().parse::<i64>()` re-parse of an ALREADY-unquoted string used
/// to (both collapsed to numeric id 80) — `object_ref`'s `Id`/`Name` shape is
/// dispatched directly, with no string re-parsing at all.
///
/// Only [`ObjectRefResolution::Unique`] returns a resolved id; `Ambiguous`/
/// `OutOfClosure`/`Unresolved` all decline to `None` — never guess (the
/// cardinal sin) — falling back to [`object_ref_fallback_lc`] for `name_lc`
/// so `resolve_member`'s `Object` arm can still attempt its own by-name
/// lookup for the (rare, dormant — digit-named AL objects are ~never seen in
/// real BC) unresolved case.
fn resolve_object_ref_lc(
    kind: ObjectKind,
    object_ref: &ObjectRef,
    from: ObjectNodeId,
    graph: &ProgramGraph,
    index: &ResolveIndex,
) -> (Option<ObjectNodeId>, String) {
    match index.resolve_object_ref(graph, from, kind, object_ref) {
        ObjectRefResolution::Unique(id) => {
            let name_lc = graph
                .objects
                .iter()
                .find(|o| o.id == id)
                .map(|o| o.name.to_ascii_lowercase())
                .unwrap_or_else(|| object_ref_fallback_lc(object_ref));
            (Some(id), name_lc)
        }
        ObjectRefResolution::Ambiguous
        | ObjectRefResolution::OutOfClosure
        | ObjectRefResolution::Unresolved => (None, object_ref_fallback_lc(object_ref)),
    }
}

/// The lowercased display text of an [`ObjectRef`], used only as the
/// `name_lc` fallback when [`resolve_object_ref_lc`]'s shape-aware resolution
/// did not find a unique target — a numeric id renders as its decimal text
/// (matching legacy `resolve_object_name_lc` fallback behavior), never
/// re-derived by parsing a string as `i64`.
fn object_ref_fallback_lc(object_ref: &ObjectRef) -> String {
    match object_ref {
        ObjectRef::Name { normalized_lc, .. } => normalized_lc.clone(),
        ObjectRef::Id(n) => n.to_string(),
    }
}

/// Build a [`ParsedType::Object`] for the given kind and raw name portion,
/// classifying quoted-vs-bare EXACTLY as `classify_type_text`'s `Record` arm
/// does (Task 2, mirrors I1): a bare numeric string is [`ObjectRef::Id`];
/// ANYTHING else — including a QUOTED numeric string, since the quote
/// characters make it fail the `i64` parse before unquoting — is
/// [`ObjectRef::Name`]. This decides shape BEFORE any unquoting happens, so
/// `Codeunit 80` (numeric id) and `Codeunit "80"` (a codeunit literally named
/// `"80"`) can never be conflated by a later re-parse of an already-unquoted
/// string.
fn parse_object_kind_type(kind: ObjectKind, name_rest: &str) -> ParsedType {
    let trimmed = name_rest.trim();
    let object_ref = if let Ok(n) = trimmed.parse::<i64>() {
        ObjectRef::Id(n)
    } else {
        let raw = unquote_identifier(trimmed);
        let normalized_lc = raw.to_ascii_lowercase();
        ObjectRef::Name { raw, normalized_lc }
    };
    ParsedType::Object { kind, object_ref }
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
            ..Default::default()
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
                table_ref: ObjectRef::Name {
                    raw: "Customer".into(),
                    normalized_lc: "customer".into()
                }
            }
        );
    }

    #[test]
    fn classify_record_unquoted() {
        assert_eq!(
            classify_type_text("Record Customer"),
            ParsedType::Record {
                table_ref: ObjectRef::Name {
                    raw: "Customer".into(),
                    normalized_lc: "customer".into()
                }
            }
        );
    }

    #[test]
    fn classify_record_temporary() {
        assert_eq!(
            classify_type_text("Record Customer temporary"),
            ParsedType::Record {
                table_ref: ObjectRef::Name {
                    raw: "Customer".into(),
                    normalized_lc: "customer".into()
                }
            }
        );
    }

    #[test]
    fn classify_record_quoted_temporary() {
        assert_eq!(
            classify_type_text("Record \"Customer\" temporary"),
            ParsedType::Record {
                table_ref: ObjectRef::Name {
                    raw: "Customer".into(),
                    normalized_lc: "customer".into()
                }
            }
        );
    }

    // -- I1 Caller-A shape-preservation: numeric id vs quoted numeric name --

    #[test]
    fn classify_record_numeric_id() {
        // `Record 18` (unquoted digits) is a NUMERIC id reference.
        assert_eq!(
            classify_type_text("Record 18"),
            ParsedType::Record {
                table_ref: ObjectRef::Id(18)
            }
        );
    }

    #[test]
    fn classify_record_quoted_numeric_name() {
        // `Record "18"` is a table literally NAMED "18" — must NOT be
        // confused with the numeric id reference `Record 18` (I1 shape bug:
        // both used to collapse to the same string "18" once quotes were
        // stripped, silently coercing a quoted name into a guessed id).
        assert_eq!(
            classify_type_text("Record \"18\""),
            ParsedType::Record {
                table_ref: ObjectRef::Name {
                    raw: "18".into(),
                    normalized_lc: "18".into()
                }
            }
        );
    }

    #[test]
    fn classify_record_numeric_id_temporary() {
        assert_eq!(
            classify_type_text("Record 18 temporary"),
            ParsedType::Record {
                table_ref: ObjectRef::Id(18)
            }
        );
    }

    #[test]
    fn classify_codeunit_numeric() {
        // `Codeunit 80` (unquoted digits) is a NUMERIC id reference.
        assert_eq!(
            classify_type_text("Codeunit 80"),
            ParsedType::Object {
                kind: ObjectKind::Codeunit,
                object_ref: ObjectRef::Id(80)
            }
        );
    }

    #[test]
    fn classify_codeunit_named() {
        assert_eq!(
            classify_type_text("Codeunit \"Sales-Post\""),
            ParsedType::Object {
                kind: ObjectKind::Codeunit,
                object_ref: ObjectRef::Name {
                    raw: "Sales-Post".into(),
                    normalized_lc: "sales-post".into()
                }
            }
        );
    }

    #[test]
    fn classify_codeunit_quoted_numeric_name() {
        // `Codeunit "80"` is a codeunit literally NAMED "80" — must NOT be
        // confused with the numeric id reference `Codeunit 80` (Task 2, the
        // I1 shape bug mirrored: both used to collapse to the same string
        // "80" once quotes were stripped, silently coercing a quoted name
        // into a guessed id).
        assert_eq!(
            classify_type_text("Codeunit \"80\""),
            ParsedType::Object {
                kind: ObjectKind::Codeunit,
                object_ref: ObjectRef::Name {
                    raw: "80".into(),
                    normalized_lc: "80".into()
                }
            }
        );
    }

    #[test]
    fn classify_page_numeric_and_quoted_numeric_name() {
        assert_eq!(
            classify_type_text("Page 80"),
            ParsedType::Object {
                kind: ObjectKind::Page,
                object_ref: ObjectRef::Id(80)
            }
        );
        assert_eq!(
            classify_type_text("Page \"80\""),
            ParsedType::Object {
                kind: ObjectKind::Page,
                object_ref: ObjectRef::Name {
                    raw: "80".into(),
                    normalized_lc: "80".into()
                }
            }
        );
    }

    #[test]
    fn classify_report_numeric_and_quoted_numeric_name() {
        assert_eq!(
            classify_type_text("Report 80"),
            ParsedType::Object {
                kind: ObjectKind::Report,
                object_ref: ObjectRef::Id(80)
            }
        );
        assert_eq!(
            classify_type_text("Report \"80\""),
            ParsedType::Object {
                kind: ObjectKind::Report,
                object_ref: ObjectRef::Name {
                    raw: "80".into(),
                    normalized_lc: "80".into()
                }
            }
        );
    }

    #[test]
    fn classify_query_numeric_and_quoted_numeric_name() {
        assert_eq!(
            classify_type_text("Query 80"),
            ParsedType::Object {
                kind: ObjectKind::Query,
                object_ref: ObjectRef::Id(80)
            }
        );
        assert_eq!(
            classify_type_text("Query \"80\""),
            ParsedType::Object {
                kind: ObjectKind::Query,
                object_ref: ObjectRef::Name {
                    raw: "80".into(),
                    normalized_lc: "80".into()
                }
            }
        );
    }

    #[test]
    fn classify_xmlport_numeric_and_quoted_numeric_name() {
        assert_eq!(
            classify_type_text("XmlPort 80"),
            ParsedType::Object {
                kind: ObjectKind::XmlPort,
                object_ref: ObjectRef::Id(80)
            }
        );
        assert_eq!(
            classify_type_text("XmlPort \"80\""),
            ParsedType::Object {
                kind: ObjectKind::XmlPort,
                object_ref: ObjectRef::Name {
                    raw: "80".into(),
                    normalized_lc: "80".into()
                }
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

        let result =
            infer_receiver_type("cust", &routine, &[], &from_obj, &graph, &index, None, None);
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
        let result = infer_receiver_type(
            "rectmp",
            &routine,
            &[],
            &from_obj,
            &graph,
            &index,
            None,
            None,
        );
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

        let result = infer_receiver_type("j", &routine, &[], &from_obj, &graph, &index, None, None);
        assert_eq!(result, ReceiverType::Framework(FrameworkKind::JsonObject));
    }

    // -----------------------------------------------------------------------
    // I1 Caller-A shape-preservation: `Record 18` (numeric id) vs
    // `Record "18"` (a table literally NAMED "18") must resolve to two
    // DIFFERENT tables, never conflated by a lossy string round-trip.
    // -----------------------------------------------------------------------

    /// Single-app fixture: Table id=18 "Customer" AND a separate table
    /// literally NAMED "18" (`declared_id: None` — its only identity is the
    /// digit-string name). Proves the two are distinguishable.
    fn build_numeric_name_shape_fixture() -> (ProgramGraph, AppRef) {
        let mut apps = crate::program::node::AppRegistry::default();
        let app_id = AppId {
            guid: String::new(),
            name: "ShapeApp".into(),
            publisher: "Test".into(),
            version: "1.0.0.0".into(),
        };
        let app = apps.intern(&app_id);
        let topology = DependencyGraph::default();

        let mut objects = vec![
            make_object_node(app, ObjectKind::Table, "Customer", Some(18), None),
            make_object_node(app, ObjectKind::Table, "18", None, None),
        ];
        objects.sort_by(|a, b| a.id.cmp(&b.id));

        let obj_index = ObjectIndex::build(&objects);
        let graph = ProgramGraph {
            apps,
            topology,
            objects,
            routines: vec![],
            obj_index,
            ..Default::default()
        };
        (graph, app)
    }

    /// Routine with `ById: Record 18` (numeric) and `ByQuotedName: Record
    /// "18"` (quoted digit-string name) params.
    fn build_numeric_name_shape_routine() -> RoutineDecl {
        let o = test_origin();
        RoutineDecl {
            kind: RoutineKind::Procedure,
            name: "TestProc".into(),
            name_origin: o.clone(),
            params: vec![
                Param {
                    name: "ById".into(),
                    by_ref: false,
                    ty: Some("Record 18".into()),
                    origin: o.clone(),
                },
                Param {
                    name: "ByQuotedName".into(),
                    by_ref: false,
                    ty: Some("Record \"18\"".into()),
                    origin: o.clone(),
                },
            ],
            return_type: None,
            locals: vec![],
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

    #[test]
    fn caller_a_record_numeric_id_resolves_by_id_not_name() {
        let (graph, app) = build_numeric_name_shape_fixture();
        let index = ResolveIndex::build(&graph);
        let routine = build_numeric_name_shape_routine();
        let from_obj = make_object_node(app, ObjectKind::Codeunit, "CallerCu", Some(999), None);

        let customer_id = graph
            .resolve_object(app, ObjectKind::Table, "Customer")
            .unwrap()
            .id
            .clone();

        // "byid" -> `ById: Record 18` (numeric) -> table id 18 ("Customer"),
        // NEVER the table literally named "18".
        let result =
            infer_receiver_type("byid", &routine, &[], &from_obj, &graph, &index, None, None);
        assert_eq!(
            result,
            ReceiverType::Record {
                table: Some(customer_id)
            }
        );
    }

    #[test]
    fn caller_a_record_quoted_numeric_name_resolves_by_name_not_id() {
        let (graph, app) = build_numeric_name_shape_fixture();
        let index = ResolveIndex::build(&graph);
        let routine = build_numeric_name_shape_routine();
        let from_obj = make_object_node(app, ObjectKind::Codeunit, "CallerCu", Some(999), None);

        let named_18_id = graph
            .resolve_object(app, ObjectKind::Table, "18")
            .unwrap()
            .id
            .clone();
        let customer_id = graph
            .resolve_object(app, ObjectKind::Table, "Customer")
            .unwrap()
            .id
            .clone();
        assert_ne!(
            named_18_id, customer_id,
            "fixture sanity: the two tables must be distinct"
        );

        // "byquotedname" -> `ByQuotedName: Record "18"` (quoted name) -> the
        // table literally NAMED "18", NEVER coerced into table id 18
        // ("Customer") — the I1 shape bug this test locks in the fix for.
        let result = infer_receiver_type(
            "byquotedname",
            &routine,
            &[],
            &from_obj,
            &graph,
            &index,
            None,
            None,
        );
        assert_eq!(
            result,
            ReceiverType::Record {
                table: Some(named_18_id)
            }
        );
    }

    // -----------------------------------------------------------------------
    // Task 2 (mirrors I1): `<Kind> 80` (numeric id) vs `<Kind> "80"` (an
    // object literally NAMED "80") must resolve to two DIFFERENT objects,
    // never conflated by a lossy string round-trip — the `ParsedType::Object`
    // sibling of the `ParsedType::Record` fix directly above. Covers every
    // kind `resolve_object_ref_lc`/`resolve_member`'s `Object` arm serves.
    // -----------------------------------------------------------------------

    /// Single-app fixture, parametrized by `kind`: an object DECLARED with
    /// id 80 ("RealById") AND a separate object of the SAME kind literally
    /// NAMED "80" (`declared_id: None` — its only identity is the
    /// digit-string name), plus a `CallerCu` Codeunit (id 999) to serve as
    /// `from_object`. Mirrors `build_numeric_name_shape_fixture` above,
    /// generalized across object kinds.
    fn build_object_numeric_name_shape_fixture(kind: ObjectKind) -> (ProgramGraph, AppRef) {
        let mut apps = crate::program::node::AppRegistry::default();
        let app_id = AppId {
            guid: String::new(),
            name: "ObjShapeApp".into(),
            publisher: "Test".into(),
            version: "1.0.0.0".into(),
        };
        let app = apps.intern(&app_id);
        let topology = DependencyGraph::default();

        let mut objects = vec![
            make_object_node(app, kind, "RealById", Some(80), None),
            make_object_node(app, kind, "80", None, None),
            make_object_node(app, ObjectKind::Codeunit, "CallerCu", Some(999), None),
        ];
        objects.sort_by(|a, b| a.id.cmp(&b.id));

        let obj_index = ObjectIndex::build(&objects);
        let graph = ProgramGraph {
            apps,
            topology,
            objects,
            routines: vec![],
            obj_index,
            ..Default::default()
        };
        (graph, app)
    }

    /// Routine with `ById: <keyword> 80` (numeric) and `ByQuotedName:
    /// <keyword> "80"` (quoted digit-string name) params, for the given AL
    /// object-kind keyword (`"Codeunit"`/`"Page"`/`"Report"`/`"Query"`/
    /// `"XmlPort"`).
    fn build_object_numeric_name_shape_routine(keyword: &str) -> RoutineDecl {
        let o = test_origin();
        RoutineDecl {
            kind: RoutineKind::Procedure,
            name: "TestProc".into(),
            name_origin: o.clone(),
            params: vec![
                Param {
                    name: "ById".into(),
                    by_ref: false,
                    ty: Some(format!("{keyword} 80")),
                    origin: o.clone(),
                },
                Param {
                    name: "ByQuotedName".into(),
                    by_ref: false,
                    ty: Some(format!("{keyword} \"80\"")),
                    origin: o.clone(),
                },
            ],
            return_type: None,
            locals: vec![],
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

    /// Shared assertion body for the per-kind Task 2 shape-preservation
    /// tests below: `<keyword> 80` must resolve to the numeric-id-80 object
    /// (`id: Some`, carried up front — Task 2's other half of the mirror);
    /// `<keyword> "80"` must resolve to the DIFFERENT object literally named
    /// "80", never the id-80 object — the exact pre-fix collapse bug.
    fn assert_object_kind_shape_preserved(kind: ObjectKind, keyword: &str) {
        let (graph, app) = build_object_numeric_name_shape_fixture(kind);
        let index = ResolveIndex::build(&graph);
        let routine = build_object_numeric_name_shape_routine(keyword);
        let from_obj = graph
            .objects
            .iter()
            .find(|o| o.name == "CallerCu")
            .unwrap()
            .clone();

        let by_id_id = graph
            .resolve_object(app, kind, "RealById")
            .unwrap()
            .id
            .clone();
        let by_name_id = graph.resolve_object(app, kind, "80").unwrap().id.clone();
        assert_ne!(
            by_id_id, by_name_id,
            "fixture sanity: the two {keyword} objects must be distinct"
        );

        let by_id =
            infer_receiver_type("byid", &routine, &[], &from_obj, &graph, &index, None, None);
        assert_eq!(
            by_id,
            ReceiverType::Object {
                kind,
                name_lc: "realbyid".into(),
                id: Some(by_id_id),
            },
            "{keyword} 80 (numeric) must resolve to the id-80 object"
        );

        let by_quoted_name = infer_receiver_type(
            "byquotedname",
            &routine,
            &[],
            &from_obj,
            &graph,
            &index,
            None,
            None,
        );
        assert_eq!(
            by_quoted_name,
            ReceiverType::Object {
                kind,
                name_lc: "80".into(),
                id: Some(by_name_id),
            },
            "{keyword} \"80\" (quoted name) must resolve to the object literally \
             named \"80\", never the numeric id-80 object (pre-fix collapse bug)"
        );
    }

    #[test]
    fn caller_a_mirror_object_codeunit_numeric_vs_quoted_name_shape_preserved() {
        assert_object_kind_shape_preserved(ObjectKind::Codeunit, "Codeunit");
    }

    #[test]
    fn caller_a_mirror_object_page_numeric_vs_quoted_name_shape_preserved() {
        assert_object_kind_shape_preserved(ObjectKind::Page, "Page");
    }

    #[test]
    fn caller_a_mirror_object_report_numeric_vs_quoted_name_shape_preserved() {
        assert_object_kind_shape_preserved(ObjectKind::Report, "Report");
    }

    #[test]
    fn caller_a_mirror_object_query_numeric_vs_quoted_name_shape_preserved() {
        assert_object_kind_shape_preserved(ObjectKind::Query, "Query");
    }

    #[test]
    fn caller_a_mirror_object_xmlport_numeric_vs_quoted_name_shape_preserved() {
        assert_object_kind_shape_preserved(ObjectKind::XmlPort, "XmlPort");
    }

    #[test]
    fn infer_param_codeunit_by_name() {
        let (graph, app) = build_test_graph();
        let index = ResolveIndex::build(&graph);
        let routine = build_test_routine();
        let from_obj = make_object_node(app, ObjectKind::Codeunit, "CallerCu", Some(999), None);
        let mycodeunit_id = graph
            .resolve_object(app, ObjectKind::Codeunit, "MyCodeunit")
            .unwrap()
            .id
            .clone();

        // "cuparam" → param `CuParam: Codeunit "MyCodeunit"` → Object{Codeunit,
        // "mycodeunit"}, `id` carried up front (Task 2: mirrors I1's `Record`
        // — a `Unique` `resolve_object_ref` match is resolved in Phase A, not
        // re-derived by a redundant Phase B by-name lookup).
        let result = infer_receiver_type(
            "cuparam",
            &routine,
            &[],
            &from_obj,
            &graph,
            &index,
            None,
            None,
        );
        assert_eq!(
            result,
            ReceiverType::Object {
                kind: ObjectKind::Codeunit,
                name_lc: "mycodeunit".into(),
                id: Some(mycodeunit_id)
            }
        );
    }

    #[test]
    fn infer_param_codeunit_by_number() {
        let (graph, app) = build_test_graph();
        let index = ResolveIndex::build(&graph);
        let routine = build_test_routine();
        let from_obj = make_object_node(app, ObjectKind::Codeunit, "CallerCu", Some(999), None);
        let mycodeunit_id = graph
            .resolve_object(app, ObjectKind::Codeunit, "MyCodeunit")
            .unwrap()
            .id
            .clone();

        // "cunumparam" → param `CuNumParam: Codeunit 50100` → resolves to
        // "mycodeunit", `id` carried up front (Task 2, mirrors I1).
        let result = infer_receiver_type(
            "cunumparam",
            &routine,
            &[],
            &from_obj,
            &graph,
            &index,
            None,
            None,
        );
        assert_eq!(
            result,
            ReceiverType::Object {
                kind: ObjectKind::Codeunit,
                name_lc: "mycodeunit".into(),
                id: Some(mycodeunit_id)
            }
        );
    }

    #[test]
    fn infer_singleton_currpage() {
        let (graph, app) = build_test_graph();
        let index = ResolveIndex::build(&graph);
        let routine = build_test_routine();
        let from_obj = make_object_node(app, ObjectKind::Page, "MyPage", Some(50200), None);

        let result = infer_receiver_type(
            "currpage",
            &routine,
            &[],
            &from_obj,
            &graph,
            &index,
            None,
            None,
        );
        assert_eq!(result, ReceiverType::Framework(FrameworkKind::PageInstance));
    }

    #[test]
    fn infer_singleton_page() {
        let (graph, app) = build_test_graph();
        let index = ResolveIndex::build(&graph);
        let routine = build_test_routine();
        let from_obj = make_object_node(app, ObjectKind::Codeunit, "CallerCu", Some(999), None);

        // bare "page" singleton
        let result =
            infer_receiver_type("page", &routine, &[], &from_obj, &graph, &index, None, None);
        assert_eq!(result, ReceiverType::Framework(FrameworkKind::PageInstance));
    }

    #[test]
    fn infer_singleton_currreport() {
        let (graph, app) = build_test_graph();
        let index = ResolveIndex::build(&graph);
        let routine = build_test_routine();
        let from_obj = make_object_node(app, ObjectKind::Report, "MyReport", Some(50300), None);

        let result = infer_receiver_type(
            "currreport",
            &routine,
            &[],
            &from_obj,
            &graph,
            &index,
            None,
            None,
        );
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

        let result = infer_receiver_type(
            "session",
            &routine,
            &[],
            &from_obj,
            &graph,
            &index,
            None,
            None,
        );
        assert_eq!(result, ReceiverType::Framework(FrameworkKind::Session));
    }

    #[test]
    fn infer_singleton_database() {
        let (graph, app) = build_test_graph();
        let index = ResolveIndex::build(&graph);
        let routine = build_test_routine();
        let from_obj = make_object_node(app, ObjectKind::Codeunit, "CallerCu", Some(999), None);

        let result = infer_receiver_type(
            "database",
            &routine,
            &[],
            &from_obj,
            &graph,
            &index,
            None,
            None,
        );
        assert_eq!(result, ReceiverType::Framework(FrameworkKind::Database));
    }

    #[test]
    fn infer_this_is_self_object() {
        let (graph, app) = build_test_graph();
        let index = ResolveIndex::build(&graph);
        let routine = build_test_routine();
        let from_obj = make_object_node(app, ObjectKind::Codeunit, "CallerCu", Some(999), None);

        let result =
            infer_receiver_type("this", &routine, &[], &from_obj, &graph, &index, None, None);
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

        let result = infer_receiver_type(
            "rec",
            &routine,
            &[],
            &customer_node,
            &graph,
            &index,
            None,
            None,
        );
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

        let result = infer_receiver_type(
            "xrec",
            &routine,
            &[],
            &customer_node,
            &graph,
            &index,
            None,
            None,
        );
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

        let result = infer_receiver_type("rec", &routine, &[], &te_obj, &graph, &index, None, None);
        assert_eq!(
            result,
            ReceiverType::Record {
                table: Some(expected_id)
            }
        );
    }

    #[test]
    fn infer_rec_in_table_extension_ambiguous_base_declines_to_none() {
        // Reuses `build_page_rec_fixture`'s "AmbTable" (declared in BOTH `a`
        // and `b`, neither is `w`) — Caller B (`infer_implicit_rec`'s
        // TableExtension arm) must DECLINE (`Record{table: None}`), never
        // silently pick the lowest `ObjectNodeId` (I1).
        let (graph, w) = build_page_rec_fixture();
        let index = ResolveIndex::build(&graph);
        let routine = build_test_routine();

        let te_obj = make_object_node(
            w,
            ObjectKind::TableExtension,
            "AmbExt",
            Some(50230),
            Some("AmbTable".into()),
        );

        let result = infer_receiver_type("rec", &routine, &[], &te_obj, &graph, &index, None, None);
        assert_eq!(result, ReceiverType::Record { table: None });
    }

    #[test]
    fn infer_rec_in_table_extension_out_of_closure_base_declines_to_none() {
        // Reuses `build_page_rec_fixture`'s "Orphan" table, declared in an
        // app `w` does not depend on.
        let (graph, w) = build_page_rec_fixture();
        let index = ResolveIndex::build(&graph);
        let routine = build_test_routine();

        let te_obj = make_object_node(
            w,
            ObjectKind::TableExtension,
            "OrphanExt",
            Some(50231),
            Some("Orphan".into()),
        );

        let result = infer_receiver_type("rec", &routine, &[], &te_obj, &graph, &index, None, None);
        assert_eq!(result, ReceiverType::Record { table: None });
    }

    #[test]
    fn infer_rec_in_page_is_record_none() {
        // No `SourceTable` property at all (`source_table: None` on ObjectNode)
        // — `make_object_node` never sets it, matching a Page with no property.
        let (graph, app) = build_test_graph();
        let index = ResolveIndex::build(&graph);
        let routine = build_test_routine();
        let page_obj = make_object_node(app, ObjectKind::Page, "CustomerCard", Some(21), None);

        let result =
            infer_receiver_type("rec", &routine, &[], &page_obj, &graph, &index, None, None);
        assert_eq!(result, ReceiverType::Record { table: None });
    }

    // -----------------------------------------------------------------------
    // infer_implicit_rec — Page/PageExtension SourceTable resolution (Task 5)
    // -----------------------------------------------------------------------

    /// Multi-app fixture for Page/PageExtension `SourceTable` resolution tests:
    /// - `w` (the `from`/workspace app): Table "Customer" (id 18, own
    ///   declaration) + Page "CustomerPage" (id 50200, `SourceTable = Customer`).
    ///   `w` depends on `a` and `b`.
    /// - `a`, `b`: BOTH declare Table "AmbTable" — a genuine cross-app name
    ///   collision, neither app is `w` itself, so it is `Ambiguous` from `w`'s
    ///   perspective.
    /// - `orphan`: Table "Orphan" (id 900), declared but NOT a dependency of
    ///   `w` — out of `w`'s closure.
    fn build_page_rec_fixture() -> (ProgramGraph, AppRef) {
        let mut apps = crate::program::node::AppRegistry::default();
        let mk_id = |name: &str| crate::snapshot::AppId {
            guid: String::new(),
            name: name.into(),
            publisher: "Test".into(),
            version: "1.0.0.0".into(),
        };
        let w = apps.intern(&mk_id("PageRecW"));
        let a = apps.intern(&mk_id("PageRecA"));
        let b = apps.intern(&mk_id("PageRecB"));
        let orphan = apps.intern(&mk_id("PageRecOrphan"));

        let mut topology = crate::program::topology::DependencyGraph::default();
        topology.add_dependency(w, a);
        topology.add_dependency(w, b);
        // `orphan` intentionally never wired in as a dependency of `w`.

        let mut customer_page =
            make_object_node(w, ObjectKind::Page, "CustomerPage", Some(50200), None);
        customer_page.source_table = Some(ObjectRef::Name {
            raw: "Customer".into(),
            normalized_lc: "customer".into(),
        });

        let mut objects = vec![
            make_object_node(w, ObjectKind::Table, "Customer", Some(18), None),
            customer_page,
            make_object_node(a, ObjectKind::Table, "AmbTable", Some(700), None),
            make_object_node(b, ObjectKind::Table, "AmbTable", Some(701), None),
            make_object_node(orphan, ObjectKind::Table, "Orphan", Some(900), None),
        ];
        objects.sort_by(|x, y| x.id.cmp(&y.id));

        let obj_index = ObjectIndex::build(&objects);
        let graph = ProgramGraph {
            apps,
            topology,
            objects,
            routines: vec![],
            obj_index,
            ..Default::default()
        };
        (graph, w)
    }

    fn amb_table_ref() -> ObjectRef {
        ObjectRef::Name {
            raw: "AmbTable".into(),
            normalized_lc: "ambtable".into(),
        }
    }

    fn orphan_table_ref() -> ObjectRef {
        ObjectRef::Name {
            raw: "Orphan".into(),
            normalized_lc: "orphan".into(),
        }
    }

    #[test]
    fn infer_rec_in_page_resolves_own_source_table_unique() {
        let (graph, w) = build_page_rec_fixture();
        let index = ResolveIndex::build(&graph);
        let routine = build_test_routine();

        let mut page = make_object_node(w, ObjectKind::Page, "CardPage", Some(50201), None);
        page.source_table = Some(ObjectRef::Name {
            raw: "Customer".into(),
            normalized_lc: "customer".into(),
        });

        let customer_id = graph
            .resolve_object(w, ObjectKind::Table, "Customer")
            .unwrap()
            .id
            .clone();

        let result = infer_receiver_type("rec", &routine, &[], &page, &graph, &index, None, None);
        assert_eq!(
            result,
            ReceiverType::Record {
                table: Some(customer_id)
            }
        );
    }

    #[test]
    fn infer_rec_in_page_ambiguous_source_table_declines_to_none() {
        let (graph, w) = build_page_rec_fixture();
        let index = ResolveIndex::build(&graph);
        let routine = build_test_routine();

        let mut page = make_object_node(w, ObjectKind::Page, "AmbPage", Some(50202), None);
        page.source_table = Some(amb_table_ref());

        // "AmbTable" is declared in BOTH `a` and `b` (neither is `w`) — must
        // DECLINE to None, never guess one of the two.
        let result = infer_receiver_type("rec", &routine, &[], &page, &graph, &index, None, None);
        assert_eq!(result, ReceiverType::Record { table: None });
    }

    #[test]
    fn infer_rec_in_page_out_of_closure_source_table_declines_to_none() {
        let (graph, w) = build_page_rec_fixture();
        let index = ResolveIndex::build(&graph);
        let routine = build_test_routine();

        let mut page = make_object_node(w, ObjectKind::Page, "OrphanPage", Some(50203), None);
        page.source_table = Some(orphan_table_ref());

        // "Orphan" is declared, but in an app `w` does not depend on.
        let result = infer_receiver_type("rec", &routine, &[], &page, &graph, &index, None, None);
        assert_eq!(result, ReceiverType::Record { table: None });
    }

    #[test]
    fn caller_a_e2e_two_dep_apps_same_name_table_declines_not_pick_first_source() {
        // Two DEPENDENCY apps (`a`/`b` in `build_page_rec_fixture`) both
        // declare `"AmbTable"` — an AL-illegal same-name collision WITHIN one
        // real compile closure, but a genuine cross-app collision in a merged
        // whole-program snapshot (I1). Neither is `w`'s own app, so Caller A
        // (`parsed_type_to_receiver`'s `Record` arm, reached via a declared
        // local `var R: Record "AmbTable"`) must DECLINE (`Record{table:
        // None}`) end to end through BOTH Phase A (receiver-type inference)
        // and Phase B (member-call resolution) — never silently pick the
        // lower `ObjectNodeId` as a confident (and possibly WRONG) `Source`
        // route, the cardinal sin.
        let (graph, w) = build_page_rec_fixture();
        let index = ResolveIndex::build(&graph);

        let o = test_origin();
        let routine = RoutineDecl {
            kind: RoutineKind::Procedure,
            name: "Test".into(),
            name_origin: o.clone(),
            params: vec![],
            return_type: None,
            locals: vec![VarDecl {
                name: "R".into(),
                ty: Some("Record \"AmbTable\"".into()),
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
        let from_obj = make_object_node(w, ObjectKind::Codeunit, "Caller", Some(50300), None);

        // Phase A: infer_receiver_type must decline, never resolve to either
        // dep's AmbTable.
        let receiver =
            infer_receiver_type("r", &routine, &[], &from_obj, &graph, &index, None, None);
        assert_eq!(receiver, ReceiverType::Record { table: None });

        // Phase B: a non-builtin method call on the declined receiver stays
        // the honest Unknown (not a fabricated Source route to either dep's
        // table) — closes the loop end to end (mirrors the already-covered
        // `resolve_member_record_table_none_emits_unknown` invariant in
        // `resolver.rs`, now driven by a genuine Phase-A ambiguity decline
        // rather than a hand-constructed `Record{table: None}`).
        let body_map = crate::program::resolve::body_map::BodyMap::build(&graph, &[]);
        let (shape, routes) = crate::program::resolve::resolver::resolve_member(
            &receiver,
            "nonbuiltinproc",
            0,
            &from_obj,
            &graph,
            &index,
            &body_map,
        );
        assert_eq!(shape, crate::program::resolve::edge::DispatchShape::Exact);
        assert_eq!(routes.len(), 1);
        assert!(matches!(
            routes[0].evidence,
            crate::program::resolve::edge::Evidence::Unknown(_)
        ));
        assert_eq!(
            routes[0].target,
            crate::program::resolve::edge::RouteTarget::Unresolved
        );
    }

    #[test]
    fn infer_rec_in_pageext_with_no_own_source_table_inherits_base_page() {
        let (graph, w) = build_page_rec_fixture();
        let index = ResolveIndex::build(&graph);
        let routine = build_test_routine();

        // Extends "CustomerPage" (SourceTable = Customer) but declares no
        // SourceTable of its own.
        let page_ext = make_object_node(
            w,
            ObjectKind::PageExtension,
            "CustomerPageExt",
            Some(50210),
            Some("CustomerPage".into()),
        );

        let customer_id = graph
            .resolve_object(w, ObjectKind::Table, "Customer")
            .unwrap()
            .id
            .clone();

        let result =
            infer_receiver_type("rec", &routine, &[], &page_ext, &graph, &index, None, None);
        assert_eq!(
            result,
            ReceiverType::Record {
                table: Some(customer_id)
            }
        );
    }

    #[test]
    fn infer_rec_in_pageext_own_source_table_overrides_base_even_when_it_declines() {
        let (graph, w) = build_page_rec_fixture();
        let index = ResolveIndex::build(&graph);
        let routine = build_test_routine();

        // Extends "CustomerPage" (a page with a perfectly good SourceTable),
        // but ALSO declares its own (ambiguous) SourceTable — the own
        // declaration must win and DECLINE, never silently fall back to the
        // base page's Customer.
        let mut page_ext = make_object_node(
            w,
            ObjectKind::PageExtension,
            "OverridePageExt",
            Some(50211),
            Some("CustomerPage".into()),
        );
        page_ext.source_table = Some(amb_table_ref());

        let result =
            infer_receiver_type("rec", &routine, &[], &page_ext, &graph, &index, None, None);
        assert_eq!(result, ReceiverType::Record { table: None });
    }

    #[test]
    fn infer_rec_in_pageext_unresolvable_extends_target_declines_to_none() {
        let (graph, w) = build_page_rec_fixture();
        let index = ResolveIndex::build(&graph);
        let routine = build_test_routine();

        // Extends a base page that does not exist anywhere in the snapshot.
        let page_ext = make_object_node(
            w,
            ObjectKind::PageExtension,
            "DanglingExt",
            Some(50212),
            Some("NoSuchBasePage".into()),
        );

        let result =
            infer_receiver_type("rec", &routine, &[], &page_ext, &graph, &index, None, None);
        assert_eq!(result, ReceiverType::Record { table: None });
    }

    #[test]
    fn infer_rec_in_report_stays_none_even_if_source_table_were_present() {
        // Defensive: Report/ReportExtension are EXCLUDED (Task 5 scope), so
        // even if a Report ObjectNode somehow carried a `source_table` (real
        // extraction never sets one from a per-dataitem source — this
        // constructs it directly to lock in the exclusion regardless of data
        // presence), the implicit Rec must stay honest `Record{table: None}`.
        let (graph, w) = build_page_rec_fixture();
        let index = ResolveIndex::build(&graph);
        let routine = build_test_routine();

        let mut report = make_object_node(w, ObjectKind::Report, "SomeReport", Some(50220), None);
        report.source_table = Some(ObjectRef::Name {
            raw: "Customer".into(),
            normalized_lc: "customer".into(),
        });

        let result = infer_receiver_type("rec", &routine, &[], &report, &graph, &index, None, None);
        assert_eq!(result, ReceiverType::Record { table: None });
    }

    #[test]
    fn infer_rec_in_codeunit_is_unknown() {
        let (graph, app) = build_test_graph();
        let index = ResolveIndex::build(&graph);
        let routine = build_test_routine();
        let cu_obj = make_object_node(app, ObjectKind::Codeunit, "CallerCu", Some(999), None);

        let result = infer_receiver_type("rec", &routine, &[], &cu_obj, &graph, &index, None, None);
        assert_eq!(result, ReceiverType::Unknown);
    }

    // -----------------------------------------------------------------------
    // infer_implicit_rec — Codeunit TableNo resolution (Task 6)
    //
    // Reuses `build_page_rec_fixture`'s Customer (in `w`)/AmbTable (cross-app
    // ambiguous, in `a` and `b`)/Orphan (out of `w`'s closure) tables — the
    // same topology shapes Task 5 exercised for Page's `SourceTable`, now
    // driving a Codeunit's `TableNo` instead.
    // -----------------------------------------------------------------------

    #[test]
    fn infer_rec_in_codeunit_resolves_table_no_unique() {
        let (graph, w) = build_page_rec_fixture();
        let index = ResolveIndex::build(&graph);
        let routine = build_test_routine();

        let mut cu = make_object_node(w, ObjectKind::Codeunit, "ItemCu", Some(50230), None);
        cu.table_no = Some(ObjectRef::Name {
            raw: "Customer".into(),
            normalized_lc: "customer".into(),
        });

        let customer_id = graph
            .resolve_object(w, ObjectKind::Table, "Customer")
            .unwrap()
            .id
            .clone();

        let result = infer_receiver_type("rec", &routine, &[], &cu, &graph, &index, None, None);
        assert_eq!(
            result,
            ReceiverType::Record {
                table: Some(customer_id)
            }
        );
    }

    #[test]
    fn infer_rec_in_codeunit_no_table_no_is_unknown() {
        let (graph, w) = build_page_rec_fixture();
        let index = ResolveIndex::build(&graph);
        let routine = build_test_routine();

        // No `TableNo` declared at all — this is also the shape of a
        // `Subtype = Test`/`TestRunner` codeunit (never declares `TableNo`):
        // no implicit-Rec entity exists at all, so this is the honest
        // `Unknown`, NOT `Record{table: None}` (that variant is reserved for
        // "a Record entity exists but its table failed to resolve", which
        // does not apply when there is no `TableNo` to resolve in the first
        // place).
        let cu = make_object_node(w, ObjectKind::Codeunit, "PlainCu", Some(50231), None);

        let result = infer_receiver_type("rec", &routine, &[], &cu, &graph, &index, None, None);
        assert_eq!(result, ReceiverType::Unknown);
    }

    #[test]
    fn infer_rec_in_codeunit_ambiguous_table_no_declines_to_record_none() {
        let (graph, w) = build_page_rec_fixture();
        let index = ResolveIndex::build(&graph);
        let routine = build_test_routine();

        let mut cu = make_object_node(w, ObjectKind::Codeunit, "AmbCu", Some(50232), None);
        cu.table_no = Some(amb_table_ref());

        // "AmbTable" is declared in BOTH `a` and `b` (neither is `w`) — must
        // DECLINE, never guess one of the two. `TableNo` IS present, so this
        // stays `Record{table: None}` (mirroring Page's non-`Unique`
        // treatment: builtins still resolve table-independently in Phase B),
        // not `Unknown`.
        let result = infer_receiver_type("rec", &routine, &[], &cu, &graph, &index, None, None);
        assert_eq!(result, ReceiverType::Record { table: None });
    }

    #[test]
    fn infer_rec_in_codeunit_out_of_closure_table_no_declines_to_record_none() {
        let (graph, w) = build_page_rec_fixture();
        let index = ResolveIndex::build(&graph);
        let routine = build_test_routine();

        let mut cu = make_object_node(w, ObjectKind::Codeunit, "OrphanCu", Some(50233), None);
        cu.table_no = Some(orphan_table_ref());

        // "Orphan" is declared, but in an app `w` does not depend on.
        let result = infer_receiver_type("rec", &routine, &[], &cu, &graph, &index, None, None);
        assert_eq!(result, ReceiverType::Record { table: None });
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
            None,
            None,
        );
        assert_eq!(result, ReceiverType::Unknown);
    }

    #[test]
    fn infer_object_globals_lookup() {
        let (graph, app) = build_test_graph();
        let index = ResolveIndex::build(&graph);
        let routine = build_test_routine();
        let from_obj = make_object_node(app, ObjectKind::Codeunit, "CallerCu", Some(999), None);
        let mycodeunit_id = graph
            .resolve_object(app, ObjectKind::Codeunit, "MyCodeunit")
            .unwrap()
            .id
            .clone();

        let o = test_origin();
        let globals = vec![VarDecl {
            name: "GlobalCu".into(),
            ty: Some("Codeunit \"MyCodeunit\"".into()),
            temporary: false,
            origin: o,
        }];

        let result = infer_receiver_type(
            "globalcu", &routine, &globals, &from_obj, &graph, &index, None, None,
        );
        assert_eq!(
            result,
            ReceiverType::Object {
                kind: ObjectKind::Codeunit,
                name_lc: "mycodeunit".into(),
                id: Some(mycodeunit_id)
            }
        );
    }

    #[test]
    fn infer_local_interface_type() {
        let (graph, app) = build_test_graph();
        let index = ResolveIndex::build(&graph);
        let routine = build_test_routine();
        let from_obj = make_object_node(app, ObjectKind::Codeunit, "CallerCu", Some(999), None);

        let result = infer_receiver_type(
            "iface",
            &routine,
            &[],
            &from_obj,
            &graph,
            &index,
            None,
            None,
        );
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

        let result = infer_receiver_type(
            "enumvar",
            &routine,
            &[],
            &from_obj,
            &graph,
            &index,
            None,
            None,
        );
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
        let mycodeunit_id = graph
            .resolve_object(app, ObjectKind::Codeunit, "MyCodeunit")
            .unwrap()
            .id
            .clone();

        let o = test_origin();
        // Global also named "CuParam" but with a different type
        let globals = vec![VarDecl {
            name: "CuParam".into(),
            ty: Some("JsonObject".into()),
            temporary: false,
            origin: o,
        }];

        // Should resolve via the PARAM (Codeunit "MyCodeunit"), not the global (JsonObject)
        let result = infer_receiver_type(
            "cuparam", &routine, &globals, &from_obj, &graph, &index, None, None,
        );
        assert_eq!(
            result,
            ReceiverType::Object {
                kind: ObjectKind::Codeunit,
                name_lc: "mycodeunit".into(),
                id: Some(mycodeunit_id)
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
            None,
            None,
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
        let result = infer_receiver_type(
            "rec",
            &routine_with_rec_local,
            &[],
            &cu_obj,
            &graph,
            &index,
            None,
            None,
        );
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

        let result = infer_receiver_type(
            "xmldocument",
            &routine,
            &[],
            &from_obj,
            &graph,
            &index,
            None,
            None,
        );
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

        let result =
            infer_receiver_type("text", &routine, &[], &from_obj, &graph, &index, None, None);
        assert_eq!(result, ReceiverType::Framework(FrameworkKind::Text));
    }

    // -----------------------------------------------------------------------
    // parse_currpage_dot_page_segment — low-level shape parse (Task 7)
    // -----------------------------------------------------------------------

    #[test]
    fn parse_currpage_segment_unquoted_part_dot_page() {
        assert_eq!(
            parse_currpage_dot_page_segment("lines.page"),
            Some("lines".to_string())
        );
    }

    #[test]
    fn parse_currpage_segment_bare_part_no_page_is_none() {
        // `CurrPage.Lines` (no `.Page`) — the CONTROL, not the subpage
        // instance.
        assert_eq!(parse_currpage_dot_page_segment("lines"), None);
    }

    #[test]
    fn parse_currpage_segment_deep_chain_is_none() {
        assert_eq!(parse_currpage_dot_page_segment("lines.page.foo"), None);
    }

    #[test]
    fn parse_currpage_segment_quoted_part_dot_page() {
        assert_eq!(
            parse_currpage_dot_page_segment("\"sub lines\".page"),
            Some("sub lines".to_string())
        );
    }

    #[test]
    fn parse_currpage_segment_malformed_unterminated_quote_is_none() {
        assert_eq!(parse_currpage_dot_page_segment("\"unterminated.page"), None);
    }

    #[test]
    fn parse_currpage_segment_empty_is_none() {
        assert_eq!(parse_currpage_dot_page_segment(""), None);
    }

    // -----------------------------------------------------------------------
    // infer_receiver_type — `CurrPage.<part>.Page` subpage-instance
    // receivers (Task 7)
    //
    // Fixture: workspace app `w` with:
    // - Page "SubPage" (id 50310) — the subpage instance target.
    // - Page "HostPage" (id 50311) with THREE controls: `Lines` (Part →
    //   SubPage), `"Sub Lines"` (Part → SubPage, quoted name), `Notes`
    //   (SystemPart), `MyAddIn` (UserControl).
    // - PageExtension "HostPageExt" (id 50312, extends HostPage) with NO
    //   controls of its own — must inherit HostPage's via the merge.
    // -----------------------------------------------------------------------

    fn build_currpage_fixture() -> (ProgramGraph, AppRef) {
        let mut apps = crate::program::node::AppRegistry::default();
        let mk_id = |name: &str| crate::snapshot::AppId {
            guid: String::new(),
            name: name.into(),
            publisher: "Test".into(),
            version: "1.0.0.0".into(),
        };
        let w = apps.intern(&mk_id("CurrPageW"));
        let topology = crate::program::topology::DependencyGraph::default();

        let subpage = make_object_node(w, ObjectKind::Page, "SubPage", Some(50310), None);

        let mut host = make_object_node(w, ObjectKind::Page, "HostPage", Some(50311), None);
        let subpage_target = ObjectRef::Name {
            raw: "SubPage".into(),
            normalized_lc: "subpage".into(),
        };
        host.page_controls = vec![
            PageControlNode {
                name_lc: "lines".into(),
                kind: PageControlKind::Part,
                target: subpage_target.clone(),
            },
            PageControlNode {
                name_lc: "sub lines".into(),
                kind: PageControlKind::Part,
                target: subpage_target,
            },
            PageControlNode {
                name_lc: "notes".into(),
                kind: PageControlKind::SystemPart,
                target: ObjectRef::Name {
                    raw: "Notes".into(),
                    normalized_lc: "notes".into(),
                },
            },
            PageControlNode {
                name_lc: "myaddin".into(),
                kind: PageControlKind::UserControl,
                target: ObjectRef::Name {
                    raw: "MyAddIn".into(),
                    normalized_lc: "myaddin".into(),
                },
            },
        ];

        let host_ext = make_object_node(
            w,
            ObjectKind::PageExtension,
            "HostPageExt",
            Some(50312),
            Some("HostPage".into()),
        );

        let mut objects = vec![subpage, host, host_ext];
        objects.sort_by(|a, b| a.id.cmp(&b.id));
        let obj_index = ObjectIndex::build(&objects);
        let graph = ProgramGraph {
            apps,
            topology,
            objects,
            routines: vec![],
            obj_index,
            ..Default::default()
        };
        (graph, w)
    }

    /// Test (a), POSITIVE: `CurrPage.Lines.Page` resolves to the SubPage
    /// object, carrying its id mechanically.
    #[test]
    fn infer_currpage_part_page_resolves_subpage_object_with_id() {
        let (graph, w) = build_currpage_fixture();
        let index = ResolveIndex::build(&graph);
        let routine = build_test_routine();
        let host = graph
            .objects
            .iter()
            .find(|o| o.name == "HostPage")
            .unwrap()
            .clone();
        let subpage_id = graph
            .resolve_object(w, ObjectKind::Page, "SubPage")
            .unwrap()
            .id
            .clone();

        let result = infer_receiver_type(
            "currpage.lines.page",
            &routine,
            &[],
            &host,
            &graph,
            &index,
            None,
            None,
        );
        assert_eq!(
            result,
            ReceiverType::Object {
                kind: ObjectKind::Page,
                name_lc: "subpage".into(),
                id: Some(subpage_id),
            }
        );
    }

    /// POSITIVE, quoted control name: `CurrPage."Sub Lines".Page` resolves
    /// identically — quotes must be stripped when matching `page_controls`.
    #[test]
    fn infer_currpage_quoted_part_page_resolves_subpage_object() {
        let (graph, w) = build_currpage_fixture();
        let index = ResolveIndex::build(&graph);
        let routine = build_test_routine();
        let host = graph
            .objects
            .iter()
            .find(|o| o.name == "HostPage")
            .unwrap()
            .clone();
        let subpage_id = graph
            .resolve_object(w, ObjectKind::Page, "SubPage")
            .unwrap()
            .id
            .clone();

        let result = infer_receiver_type(
            "currpage.\"sub lines\".page",
            &routine,
            &[],
            &host,
            &graph,
            &index,
            None,
            None,
        );
        assert_eq!(
            result,
            ReceiverType::Object {
                kind: ObjectKind::Page,
                name_lc: "subpage".into(),
                id: Some(subpage_id),
            }
        );
    }

    /// Test (b), NEGATIVE — control vs subpage: `CurrPage.Lines` (no
    /// `.Page`) is the CONTROL, not the subpage instance — must stay
    /// `Unknown`, never fabricated as `SubPage`.
    #[test]
    fn infer_currpage_bare_part_no_page_accessor_stays_unknown() {
        let (graph, _w) = build_currpage_fixture();
        let index = ResolveIndex::build(&graph);
        let routine = build_test_routine();
        let host = graph
            .objects
            .iter()
            .find(|o| o.name == "HostPage")
            .unwrap()
            .clone();

        let result = infer_receiver_type(
            "currpage.lines",
            &routine,
            &[],
            &host,
            &graph,
            &index,
            None,
            None,
        );
        assert_eq!(result, ReceiverType::Unknown);
    }

    /// Test (c), NEGATIVE — deep chain: `CurrPage.Lines.Page.Foo` (more than
    /// one remaining segment) stays `Unknown`.
    #[test]
    fn infer_currpage_deep_chain_beyond_dot_page_stays_unknown() {
        let (graph, _w) = build_currpage_fixture();
        let index = ResolveIndex::build(&graph);
        let routine = build_test_routine();
        let host = graph
            .objects
            .iter()
            .find(|o| o.name == "HostPage")
            .unwrap()
            .clone();

        let result = infer_receiver_type(
            "currpage.lines.page.foo",
            &routine,
            &[],
            &host,
            &graph,
            &index,
            None,
            None,
        );
        assert_eq!(result, ReceiverType::Unknown);
    }

    /// Test (d), NEGATIVE — unknown part: `CurrPage.Nope.Page` (no control
    /// named "Nope") stays `Unknown`.
    #[test]
    fn infer_currpage_unknown_part_stays_unknown() {
        let (graph, _w) = build_currpage_fixture();
        let index = ResolveIndex::build(&graph);
        let routine = build_test_routine();
        let host = graph
            .objects
            .iter()
            .find(|o| o.name == "HostPage")
            .unwrap()
            .clone();

        let result = infer_receiver_type(
            "currpage.nope.page",
            &routine,
            &[],
            &host,
            &graph,
            &index,
            None,
            None,
        );
        assert_eq!(result, ReceiverType::Unknown);
    }

    /// Test (e), NEGATIVE — SystemPart: even WITH a `.Page` accessor, a
    /// SystemPart control must NOT resolve to a fabricated Object/Framework
    /// route — Task 7 scope is `Part` only.
    #[test]
    fn infer_currpage_systempart_dot_page_stays_unknown_not_fabricated() {
        let (graph, _w) = build_currpage_fixture();
        let index = ResolveIndex::build(&graph);
        let routine = build_test_routine();
        let host = graph
            .objects
            .iter()
            .find(|o| o.name == "HostPage")
            .unwrap()
            .clone();

        let result = infer_receiver_type(
            "currpage.notes.page",
            &routine,
            &[],
            &host,
            &graph,
            &index,
            None,
            None,
        );
        assert_eq!(result, ReceiverType::Unknown);
    }

    /// Test (e), NEGATIVE — UserControl: same as SystemPart, `.Page` on a
    /// UserControl must decline, not fabricate a route.
    #[test]
    fn infer_currpage_usercontrol_dot_page_stays_unknown_not_fabricated() {
        let (graph, _w) = build_currpage_fixture();
        let index = ResolveIndex::build(&graph);
        let routine = build_test_routine();
        let host = graph
            .objects
            .iter()
            .find(|o| o.name == "HostPage")
            .unwrap()
            .clone();

        let result = infer_receiver_type(
            "currpage.myaddin.page",
            &routine,
            &[],
            &host,
            &graph,
            &index,
            None,
            None,
        );
        assert_eq!(result, ReceiverType::Unknown);
    }

    /// NEGATIVE — bare SystemPart/UserControl (no `.Page` at all) also stay
    /// `Unknown`, exercising the ordinary "no .page suffix" decline path for
    /// these control kinds too.
    #[test]
    fn infer_currpage_bare_systempart_and_usercontrol_stay_unknown() {
        let (graph, _w) = build_currpage_fixture();
        let index = ResolveIndex::build(&graph);
        let routine = build_test_routine();
        let host = graph
            .objects
            .iter()
            .find(|o| o.name == "HostPage")
            .unwrap()
            .clone();

        assert_eq!(
            infer_receiver_type(
                "currpage.notes",
                &routine,
                &[],
                &host,
                &graph,
                &index,
                None,
                None
            ),
            ReceiverType::Unknown
        );
        assert_eq!(
            infer_receiver_type(
                "currpage.myaddin",
                &routine,
                &[],
                &host,
                &graph,
                &index,
                None,
                None
            ),
            ReceiverType::Unknown
        );
    }

    /// PageExtension merge: `HostPageExt` (extends `HostPage`, no controls
    /// of its own) inherits `HostPage`'s `Lines` control via the fail-closed
    /// base-page lookup — mirrors L3's `page_controls_for` merge.
    #[test]
    fn infer_currpage_pageext_inherits_base_page_control() {
        let (graph, w) = build_currpage_fixture();
        let index = ResolveIndex::build(&graph);
        let routine = build_test_routine();
        let host_ext = graph
            .objects
            .iter()
            .find(|o| o.name == "HostPageExt")
            .unwrap()
            .clone();
        let subpage_id = graph
            .resolve_object(w, ObjectKind::Page, "SubPage")
            .unwrap()
            .id
            .clone();

        let result = infer_receiver_type(
            "currpage.lines.page",
            &routine,
            &[],
            &host_ext,
            &graph,
            &index,
            None,
            None,
        );
        assert_eq!(
            result,
            ReceiverType::Object {
                kind: ObjectKind::Page,
                name_lc: "subpage".into(),
                id: Some(subpage_id),
            }
        );
    }

    // -----------------------------------------------------------------------
    // infer_receiver_type — Task 2 enabling primitive: `receiver_expr` threading
    // -----------------------------------------------------------------------

    /// Task 2 invariant: `infer_receiver_type` ACCEPTS a real
    /// `Some((&AlFile, ExprId))` for a `Func().M()` call site (the structured
    /// receiver `ExprKind::Call{..}` a resolver could fetch via
    /// `file.ir.expr(id)`) and — since Steps 0-4 dispatch purely on
    /// `receiver_lc`, unchanged by this task, AND `bare_ctx` (Task 3's Step 5
    /// enabling primitive) is `None` here — still returns exactly what it
    /// returned before this parameter existed: `Unknown` (`"func()"` matches
    /// none of Steps 0-4, and Step 5 is a no-op without `bare_ctx`).
    /// Resolution-neutral by construction; see the `infer_call_result_*`
    /// tests below for Step 5's actual (Task 3) behavior with `bare_ctx`
    /// populated.
    #[test]
    fn infer_receiver_type_accepts_threaded_call_receiver_and_stays_neutral() {
        use crate::program::resolve::extract::{CalleeShape, extract_sites};

        let src = r#"
codeunit 50100 "C"
{
    procedure Run()
    begin
        Func().M();
    end;
    procedure Func(): Codeunit "C" begin end;
}
"#;
        let file = al_syntax::parse(src);
        let sites = extract_sites(&file, src, "C.al", &std::collections::HashSet::new());
        let member_site = sites
            .iter()
            .find(|s| matches!(&s.shape, CalleeShape::Member { method, .. } if method.eq_ignore_ascii_case("m")))
            .expect("Func().M() must classify as a Member call");
        let CalleeShape::Member {
            receiver_text,
            receiver,
            ..
        } = &member_site.shape
        else {
            unreachable!("filtered to Member above");
        };
        let receiver_id = receiver.expect("Member.receiver must be populated");

        let (graph, app) = build_test_graph();
        let index = ResolveIndex::build(&graph);
        let routine = build_test_routine();
        let from_obj = make_object_node(app, ObjectKind::Codeunit, "CallerCu", Some(999), None);

        let receiver_lc = receiver_text.to_ascii_lowercase();
        let result = infer_receiver_type(
            &receiver_lc,
            &routine,
            &[],
            &from_obj,
            &graph,
            &index,
            Some((&file, receiver_id)),
            None,
        );
        assert_eq!(
            result,
            ReceiverType::Unknown,
            "threading a real receiver_expr must not change Task 2's resolution outcome"
        );
    }

    // -----------------------------------------------------------------------
    // infer_receiver_type — Step 6 (beyond-1B.3b Task 4): compound framework
    // property/method chains + `this.<rest>`.
    // -----------------------------------------------------------------------

    fn routine_with_locals(locals: Vec<VarDecl>) -> RoutineDecl {
        let o = test_origin();
        RoutineDecl {
            kind: RoutineKind::Procedure,
            name: "Run".into(),
            name_origin: o.clone(),
            params: vec![],
            return_type: None,
            locals,
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

    fn var_decl(name: &str, ty: &str) -> VarDecl {
        VarDecl {
            name: name.into(),
            ty: Some(ty.into()),
            temporary: false,
            origin: test_origin(),
        }
    }

    /// Parse `src`, extract the sole `Member` call site whose method matches
    /// `method_lc`, and return `(AlFile, receiver_text, receiver ExprId)`.
    fn parse_member_site(src: &str, method_lc: &str) -> (al_syntax::ir::AlFile, String, ExprId) {
        use crate::program::resolve::extract::{CalleeShape, extract_sites};

        let file = al_syntax::parse(src);
        let sites = extract_sites(&file, src, "T.al", &std::collections::HashSet::new());
        let site = sites
            .iter()
            .find(|s| matches!(&s.shape, CalleeShape::Member { method, .. } if method.eq_ignore_ascii_case(method_lc)))
            .unwrap_or_else(|| panic!("no Member call site with method {method_lc:?} found"));
        let CalleeShape::Member {
            receiver_text,
            receiver,
            ..
        } = &site.shape
        else {
            unreachable!("filtered to Member above");
        };
        let receiver_id = receiver.expect("Member.receiver must be populated");
        (file, receiver_text.clone(), receiver_id)
    }

    /// POSITIVE: `Response.Content().ReadAs(Foo)` — `Response: HttpResponseMessage`
    /// → `Content()` (real AL zero-arg method, table-verified) → `HttpContent`,
    /// so the receiver of `.ReadAs(...)` types `Framework(HttpContent)`.
    #[test]
    fn framework_chain_http_response_content_resolves_to_http_content() {
        let src = r#"
codeunit 50100 "C"
{
    procedure Run()
    var
        Response: HttpResponseMessage;
        Foo: Text;
    begin
        Response.Content().ReadAs(Foo);
    end;
}
"#;
        let (file, receiver_text, receiver_id) = parse_member_site(src, "readas");
        assert_eq!(receiver_text.to_ascii_lowercase(), "response.content()");

        let (graph, app) = build_test_graph();
        let index = ResolveIndex::build(&graph);
        let routine = routine_with_locals(vec![
            var_decl("Response", "HttpResponseMessage"),
            var_decl("Foo", "Text"),
        ]);
        let from_obj = make_object_node(app, ObjectKind::Codeunit, "CallerCu", Some(999), None);

        let result = infer_receiver_type(
            &receiver_text.to_ascii_lowercase(),
            &routine,
            &[],
            &from_obj,
            &graph,
            &index,
            Some((&file, receiver_id)),
            None,
        );
        assert_eq!(result, ReceiverType::Framework(FrameworkKind::HttpContent));
    }

    /// POSITIVE: `JToken.AsObject().Get('key', X)` — `JToken: JsonToken` →
    /// `AsObject()` (table-verified) → `JsonObject`, so the receiver of
    /// `.Get(...)` types `Framework(JsonObject)`.
    #[test]
    fn framework_chain_jsontoken_asobject_resolves_to_json_object() {
        let src = r#"
codeunit 50100 "C"
{
    procedure Run()
    var
        JToken: JsonToken;
        X: JsonToken;
    begin
        JToken.AsObject().Get('key', X);
    end;
}
"#;
        let (file, receiver_text, receiver_id) = parse_member_site(src, "get");
        assert_eq!(receiver_text.to_ascii_lowercase(), "jtoken.asobject()");

        let (graph, app) = build_test_graph();
        let index = ResolveIndex::build(&graph);
        let routine = routine_with_locals(vec![
            var_decl("JToken", "JsonToken"),
            var_decl("X", "JsonToken"),
        ]);
        let from_obj = make_object_node(app, ObjectKind::Codeunit, "CallerCu", Some(999), None);

        let result = infer_receiver_type(
            &receiver_text.to_ascii_lowercase(),
            &routine,
            &[],
            &from_obj,
            &graph,
            &index,
            Some((&file, receiver_id)),
            None,
        );
        assert_eq!(result, ReceiverType::Framework(FrameworkKind::JsonObject));
    }

    /// POSITIVE: `this.DialogWindow.Open()` — `this`-strip resolves
    /// `DialogWindow` against the object-GLOBALS-only scope (`Dialog` global),
    /// so the receiver of `.Open()` types `Framework(Dialog)`.
    #[test]
    fn this_strip_dialogwindow_resolves_to_dialog() {
        let src = r#"
codeunit 50100 "C"
{
    procedure Run()
    begin
        this.DialogWindow.Open();
    end;
}
"#;
        let (file, receiver_text, receiver_id) = parse_member_site(src, "open");
        assert_eq!(receiver_text.to_ascii_lowercase(), "this.dialogwindow");

        let (graph, app) = build_test_graph();
        let index = ResolveIndex::build(&graph);
        let routine = routine_with_locals(vec![]);
        let object_globals = vec![var_decl("DialogWindow", "Dialog")];
        let from_obj = make_object_node(app, ObjectKind::Codeunit, "CallerCu", Some(999), None);

        let result = infer_receiver_type(
            &receiver_text.to_ascii_lowercase(),
            &routine,
            &object_globals,
            &from_obj,
            &graph,
            &index,
            Some((&file, receiver_id)),
            None,
        );
        assert_eq!(result, ReceiverType::Framework(FrameworkKind::Dialog));
    }

    /// NEGATIVE: `this.DialogWindow.Open()` where `DialogWindow` is a LOCAL
    /// variable (or param), never declared as an object global — `this.`
    /// deliberately does NOT see locals/params (only `object_globals`), so
    /// this must stay `Unknown`, never fall back to a local-shadow guess.
    #[test]
    fn this_strip_ignores_locals_and_params() {
        let src = r#"
codeunit 50100 "C"
{
    procedure Run()
    var
        DialogWindow: Dialog;
    begin
        this.DialogWindow.Open();
    end;
}
"#;
        let (file, receiver_text, receiver_id) = parse_member_site(src, "open");

        let (graph, app) = build_test_graph();
        let index = ResolveIndex::build(&graph);
        // `DialogWindow` declared as a LOCAL on `routine`, NOT in `object_globals`.
        let routine = routine_with_locals(vec![var_decl("DialogWindow", "Dialog")]);
        let from_obj = make_object_node(app, ObjectKind::Codeunit, "CallerCu", Some(999), None);

        let result = infer_receiver_type(
            &receiver_text.to_ascii_lowercase(),
            &routine,
            &[], // no object globals — DialogWindow is NOT a member of the object
            &from_obj,
            &graph,
            &index,
            Some((&file, receiver_id)),
            None,
        );
        assert_eq!(
            result,
            ReceiverType::Unknown,
            "this. must resolve only against object globals, never locals/params"
        );
    }

    /// NEGATIVE: `this.Method()` (a CALL form, not a property) — deliberately
    /// DEFERRED (module doc Step 6b); must decline even when `Method` IS a
    /// framework-typed global's zero-arg conversion, since this shape isn't
    /// distinguishing a global from a same-object PROCEDURE without
    /// `resolve_bare`-style lookup, which this step doesn't perform.
    #[test]
    fn this_strip_call_form_declines() {
        let src = r#"
codeunit 50100 "C"
{
    procedure Run()
    begin
        this.DialogWindow().Open();
    end;
}
"#;
        let (file, receiver_text, receiver_id) = parse_member_site(src, "open");

        let (graph, app) = build_test_graph();
        let index = ResolveIndex::build(&graph);
        let routine = routine_with_locals(vec![]);
        let object_globals = vec![var_decl("DialogWindow", "Dialog")];
        let from_obj = make_object_node(app, ObjectKind::Codeunit, "CallerCu", Some(999), None);

        let result = infer_receiver_type(
            &receiver_text.to_ascii_lowercase(),
            &routine,
            &object_globals,
            &from_obj,
            &graph,
            &index,
            Some((&file, receiver_id)),
            None,
        );
        assert_eq!(result, ReceiverType::Unknown);
    }

    /// NEGATIVE: base not a known framework type — `Foo.Content().ReadAs(X)`
    /// where `Foo` is not declared anywhere (unresolved identifier); the
    /// recursive base-typing declines, so the whole chain declines.
    #[test]
    fn framework_chain_unknown_base_declines() {
        let src = r#"
codeunit 50100 "C"
{
    procedure Run()
    var
        Foo2: Text;
    begin
        Foo.Content().ReadAs(Foo2);
    end;
}
"#;
        let (file, receiver_text, receiver_id) = parse_member_site(src, "readas");

        let (graph, app) = build_test_graph();
        let index = ResolveIndex::build(&graph);
        let routine = routine_with_locals(vec![var_decl("Foo2", "Text")]);
        let from_obj = make_object_node(app, ObjectKind::Codeunit, "CallerCu", Some(999), None);

        let result = infer_receiver_type(
            &receiver_text.to_ascii_lowercase(),
            &routine,
            &[],
            &from_obj,
            &graph,
            &index,
            Some((&file, receiver_id)),
            None,
        );
        assert_eq!(result, ReceiverType::Unknown);
    }

    /// NEGATIVE: prop/method not in the table (table-miss = fail-closed) —
    /// `Response.Foo().ReadAs(X)`: `Response` types `Framework(HttpResponseMessage)`
    /// but `"foo"` is not a table entry for that kind.
    #[test]
    fn framework_chain_table_miss_declines() {
        let src = r#"
codeunit 50100 "C"
{
    procedure Run()
    var
        Response: HttpResponseMessage;
        X: Text;
    begin
        Response.Foo().ReadAs(X);
    end;
}
"#;
        let (file, receiver_text, receiver_id) = parse_member_site(src, "readas");

        let (graph, app) = build_test_graph();
        let index = ResolveIndex::build(&graph);
        let routine = routine_with_locals(vec![
            var_decl("Response", "HttpResponseMessage"),
            var_decl("X", "Text"),
        ]);
        let from_obj = make_object_node(app, ObjectKind::Codeunit, "CallerCu", Some(999), None);

        let result = infer_receiver_type(
            &receiver_text.to_ascii_lowercase(),
            &routine,
            &[],
            &from_obj,
            &graph,
            &index,
            Some((&file, receiver_id)),
            None,
        );
        assert_eq!(result, ReceiverType::Unknown);
    }

    /// NEGATIVE: wrong FORM — a table method-entry invoked as a property (no
    /// parens): `Response.Content.ReadAs(X)` (property form, `is_method:
    /// false`) never matches the table's `(HttpResponseMessage, "content",
    /// true, 0)` method-form entry.
    #[test]
    fn framework_chain_wrong_form_property_instead_of_method_declines() {
        let src = r#"
codeunit 50100 "C"
{
    procedure Run()
    var
        Response: HttpResponseMessage;
        X: Text;
    begin
        Response.Content.ReadAs(X);
    end;
}
"#;
        let (file, receiver_text, receiver_id) = parse_member_site(src, "readas");
        assert_eq!(receiver_text.to_ascii_lowercase(), "response.content");

        let (graph, app) = build_test_graph();
        let index = ResolveIndex::build(&graph);
        let routine = routine_with_locals(vec![
            var_decl("Response", "HttpResponseMessage"),
            var_decl("X", "Text"),
        ]);
        let from_obj = make_object_node(app, ObjectKind::Codeunit, "CallerCu", Some(999), None);

        let result = infer_receiver_type(
            &receiver_text.to_ascii_lowercase(),
            &routine,
            &[],
            &from_obj,
            &graph,
            &index,
            Some((&file, receiver_id)),
            None,
        );
        assert_eq!(result, ReceiverType::Unknown);
    }

    /// NEGATIVE: wrong ARITY — `Response.Content(X).ReadAs(Y)` (1 arg) never
    /// matches the table's arity-0 entry.
    #[test]
    fn framework_chain_wrong_arity_declines() {
        let src = r#"
codeunit 50100 "C"
{
    procedure Run()
    var
        Response: HttpResponseMessage;
        X: HttpContent;
        Y: Text;
    begin
        Response.Content(X).ReadAs(Y);
    end;
}
"#;
        let (file, receiver_text, receiver_id) = parse_member_site(src, "readas");
        assert_eq!(receiver_text.to_ascii_lowercase(), "response.content(x)");

        let (graph, app) = build_test_graph();
        let index = ResolveIndex::build(&graph);
        let routine = routine_with_locals(vec![
            var_decl("Response", "HttpResponseMessage"),
            var_decl("X", "HttpContent"),
            var_decl("Y", "Text"),
        ]);
        let from_obj = make_object_node(app, ObjectKind::Codeunit, "CallerCu", Some(999), None);

        let result = infer_receiver_type(
            &receiver_text.to_ascii_lowercase(),
            &routine,
            &[],
            &from_obj,
            &graph,
            &index,
            Some((&file, receiver_id)),
            None,
        );
        assert_eq!(result, ReceiverType::Unknown);
    }

    /// NEGATIVE: a base whose recursion mis-types — `Response.Bar().Content().ReadAs(X)`:
    /// `Response.Bar()` itself is a table-miss (declines to `Unknown`), so the
    /// OUTER `.Content()` hop's base is `Unknown` (not `Framework`), and the
    /// whole chain declines — proving a mis-typed intermediate hop propagates
    /// rather than silently resetting to some other guess.
    #[test]
    fn framework_chain_recursion_mistype_declines() {
        let src = r#"
codeunit 50100 "C"
{
    procedure Run()
    var
        Response: HttpResponseMessage;
        X: Text;
    begin
        Response.Bar().Content().ReadAs(X);
    end;
}
"#;
        let (file, receiver_text, receiver_id) = parse_member_site(src, "readas");
        assert_eq!(
            receiver_text.to_ascii_lowercase(),
            "response.bar().content()"
        );

        let (graph, app) = build_test_graph();
        let index = ResolveIndex::build(&graph);
        let routine = routine_with_locals(vec![
            var_decl("Response", "HttpResponseMessage"),
            var_decl("X", "Text"),
        ]);
        let from_obj = make_object_node(app, ObjectKind::Codeunit, "CallerCu", Some(999), None);

        let result = infer_receiver_type(
            &receiver_text.to_ascii_lowercase(),
            &routine,
            &[],
            &from_obj,
            &graph,
            &index,
            Some((&file, receiver_id)),
            None,
        );
        assert_eq!(result, ReceiverType::Unknown);
    }

    /// NEGATIVE: a same-named member on a NON-framework type must NOT hit the
    /// table — `Cust.Content().ReadAs(X)` where `Cust: Record Customer` types
    /// `Record{..}`, not `Framework`, so the table lookup never even engages
    /// (short-circuited by the `Framework(kind)`-only guard), even though
    /// `"content"` IS a valid table member for `HttpResponseMessage`.
    #[test]
    fn framework_chain_non_framework_base_never_hits_table() {
        let src = r#"
codeunit 50100 "C"
{
    procedure Run()
    var
        Cust: Record Customer;
        X: Text;
    begin
        Cust.Content().ReadAs(X);
    end;
}
"#;
        let (file, receiver_text, receiver_id) = parse_member_site(src, "readas");

        let (graph, app) = build_test_graph();
        let index = ResolveIndex::build(&graph);
        let routine = routine_with_locals(vec![
            var_decl("Cust", "Record Customer"),
            var_decl("X", "Text"),
        ]);
        let from_obj = make_object_node(app, ObjectKind::Codeunit, "CallerCu", Some(999), None);

        let result = infer_receiver_type(
            &receiver_text.to_ascii_lowercase(),
            &routine,
            &[],
            &from_obj,
            &graph,
            &index,
            Some((&file, receiver_id)),
            None,
        );
        assert_eq!(result, ReceiverType::Unknown);
    }

    /// REGRESSION (round-2, found via the CDO gate's EXHAUSTIVE adjudication —
    /// see `.superpowers/sdd/task-4-report.md`): a QUOTED identifier whose
    /// UNQUOTED text merely STARTS WITH a framework keyword word must NOT
    /// collide with that framework type via Step 4's naive first-whitespace-
    /// token match. Real CDO: Table "CDO File"'s OWN `Blob` field
    /// `"File Blob"`, accessed bare (implicit Rec, inside the table's own
    /// procedure) as `"File Blob".CreateInStream(...)`, was FALSELY typed
    /// `Framework(File)` (unquoting "File Blob" → "file blob" → Step 4 matches
    /// the leading "file" token) and false-resolved `.CreateInStream`/
    /// `.CreateOutStream` to the `File` catalog instead of staying the honest
    /// `Unknown` a Blob FIELD reference correctly is — the quote-parity guard
    /// in `infer_receiver_type_for_expr` (re-quoting a `QuotedIdentifier`
    /// before recursing) fixes this by restoring byte-for-byte parity with
    /// the top-level `receiver_lc`'s (quoted, hence non-matching) string.
    #[test]
    fn quoted_identifier_never_collides_with_framework_keyword_via_recursion() {
        let src = r#"
codeunit 50100 "C"
{
    procedure Run()
    var
        Foo: InStream;
    begin
        "File Blob".CreateInStream(Foo);
    end;
}
"#;
        let (file, receiver_text, receiver_id) = parse_member_site(src, "createinstream");
        assert_eq!(receiver_text.to_ascii_lowercase(), "\"file blob\"");

        let (graph, app) = build_test_graph();
        let index = ResolveIndex::build(&graph);
        let routine = routine_with_locals(vec![var_decl("Foo", "InStream")]);
        let from_obj = make_object_node(app, ObjectKind::Codeunit, "CallerCu", Some(999), None);

        let result = infer_receiver_type(
            &receiver_text.to_ascii_lowercase(),
            &routine,
            &[], // "File Blob" is NOT a declared var/param/global (mirrors a table field)
            &from_obj,
            &graph,
            &index,
            Some((&file, receiver_id)),
            None,
        );
        assert_eq!(
            result,
            ReceiverType::Unknown,
            "a quoted name that merely unquotes to text starting with a framework \
             keyword (\"File Blob\" -> \"file blob\" -> \"File\") must never be \
             mis-typed as that framework type"
        );
    }

    /// DEFERRED-shape NEGATIVE: record-field member-of-member —
    /// `Rec.BlobField.CreateOutStream()` stays `Unknown`. `Rec` types
    /// `Record{..}` (not `Framework`), so — exactly like the non-framework-base
    /// case above — the table lookup never engages; field-type indexing
    /// (`BlobField`'s declared field TYPE) is a genuinely different, deferred
    /// mechanism (node-model-heavy, out of this task's scope per the plan's
    /// "Out of scope (next plan)" list) — this fixture pins that it stays
    /// honestly `Unknown` rather than silently mis-typed via this table.
    #[test]
    fn framework_chain_record_field_deferred_stays_unknown() {
        let src = r#"
codeunit 50100 "C"
{
    procedure Run()
    var
        Rec: Record Customer;
    begin
        Rec.BlobField.CreateOutStream();
    end;
}
"#;
        let (file, receiver_text, receiver_id) = parse_member_site(src, "createoutstream");
        assert_eq!(receiver_text.to_ascii_lowercase(), "rec.blobfield");

        let (graph, app) = build_test_graph();
        let index = ResolveIndex::build(&graph);
        let routine = routine_with_locals(vec![var_decl("Rec", "Record Customer")]);
        let from_obj = make_object_node(app, ObjectKind::Codeunit, "CallerCu", Some(999), None);

        let result = infer_receiver_type(
            &receiver_text.to_ascii_lowercase(),
            &routine,
            &[],
            &from_obj,
            &graph,
            &index,
            Some((&file, receiver_id)),
            None,
        );
        assert_eq!(result, ReceiverType::Unknown);
    }

    // NOTE: the Task-3 review finding folded into Task 4 (`infer_call_result_
    // receiver`'s return-type lookup switched from a linear `.find` to
    // `graph.routines.binary_search_by`, mirroring `lookup_routine_access`/
    // `make_routine_route`) is a behavior-preserving refactor over the SAME
    // sorted `graph.routines` data — it is exercised end-to-end by the
    // existing Task 3 fixture suite (`ws_compound_call_result_*` in
    // `tests/program_resolve_harness.rs`, built via the real
    // `resolve_full_program` pipeline that populates and sorts `graph.routines`
    // exactly as production code does), which all continue to pass unchanged.
    // A hand-built unit `RoutineNode`/`BodyMap`/`WithState` fixture here would
    // duplicate that coverage while risking drift from the real (much larger)
    // `RoutineNode` struct shape, so this is deliberately NOT re-tested with a
    // bespoke unit test.
}
