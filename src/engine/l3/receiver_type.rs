//! Member-call receiver typing (the ReceiverType lattice) — the two-phase typed
//! resolver that replaced the string-keyed if/else ladder in
//! `call_resolver::resolve_call_site`'s `PCallee::Member` arm.
//!
//! # Phase A / Phase B
//!
//! Member-call resolution is split into a *type-inference* phase and a *typed
//! dispatch* phase:
//!
//!   * **Phase A — [`infer_receiver_type`]** maps a receiver expression (in the
//!     context of its caller routine + the workspace symbol table) onto a
//!     [`ReceiverType`] lattice value. It performs ONLY today's Phase-A logic:
//!     simple-name extraction → variable lookup → object-type-ref parse →
//!     builtin-catalog classification (incl. the Record table-object-id
//!     resolution). It never looks at the method name and never produces edges.
//!
//!   * **Phase B — [`dispatch`]** takes the inferred [`ReceiverType`] + the method
//!     name + a [`DispatchCtx`] and produces the exact `CallEdge`s the legacy
//!     ladder produced — one `match` arm per lattice variant.
//!
//! # Fail-closed invariant
//!
//! Every Phase-A path that cannot positively type a receiver yields
//! [`ReceiverType::Unknown`] carrying the attributed [`UnknownReason`]; Phase B
//! turns that (and the non-callable [`ReceiverType::Primitive`] /
//! [`ReceiverType::Enum`], and a non-builtin method on a table-less
//! [`ReceiverType::Record`]) into an honest `unknown` edge. The engine NEVER
//! panics and NEVER invents a resolution: an unrecognized shape is DATA (an
//! `unknown` edge with a reason), not a silent gap.
//!
//! Note the ONE place where typing succeeds but resolution may still fail in
//! Phase B: a [`ReceiverType::Record`] always types (a Record is a Record even
//! when its table is out-of-source), but the catalog-builtin check is FIRST and
//! table-independent, so only a NON-builtin method on a Record with no resolvable
//! table degrades to `Unknown { RecordTableProcedure }` — a Phase-B decision, not
//! a Phase-A typing failure. This split is load-bearing for behavior parity: a
//! `SetRange` on a dependency-app-typed Record stays `builtin`.
//!
//! This is a behavior-preserving refactor — it produces byte-identical edges to
//! the legacy ladder for every input.

use super::l3_workspace::L3Routine;
use super::member_builtins::{classify_receiver, member_builtin_disposition, ReceiverBuiltinKind};
use super::receiver::simple_receiver_name;
use super::symbol_table::SymbolTable;
use super::type_ref::{parse_object_type_ref, ObjectKind};

use crate::engine::l2::features::PCallSite;
use crate::engine::l3::call_resolver::{
    dynamic_method, mark_bindings_ambiguous, resolve_by_name_and_arity,
    resolve_by_name_and_arity_multi, resolve_interface_dispatch, sorted_ids, unknown_method,
    upgrade_bindings, ArityResolution, BindingState, CallEdge, Diagnostic, ExternalTypeRef,
    UnknownReason,
};
use crate::engine::l3::taxonomy::{DispatchKind, Resolution};

/// The inferred type of a member-call receiver — the lattice Phase B dispatches
/// on. Every variant maps 1:1 onto a Phase-B `match` arm.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReceiverType {
    /// A first-class object type the AL compiler exposes by `Kind "Name"`
    /// (Codeunit / Page / Report / Query / XmlPort) — the `parse_object_type_ref`
    /// shape, minus Interface/Enum which split out below.
    Object { kind: ObjectKind, name: String },
    /// An `Interface IFoo` receiver — Phase B fans out to every implementer.
    Interface { name: String },
    /// An `Enum ...` receiver — enum statics are not callable here.
    Enum { name: String },
    /// A `Record`-typed receiver. `table_object_id` is the workspace table OBJECT
    /// id (`{appGuid}/Table/{n}` — the `L3Routine.object_id` shape) when the
    /// declared table RESOLVED, else `None` (the table is out-of-source / a
    /// dependency object). Phase A performs the table-object-id resolution so Phase
    /// B can dispatch table procedures directly.
    ///
    /// CRITICAL (legacy parity): a Record receiver is ALWAYS `Record`, NEVER
    /// `Unknown` — even when the table did not resolve. The catalog-builtin check
    /// runs FIRST in Phase B and is independent of the table (a `SetRange` /
    /// `FindSet` on a Record whose table is in a dependency app is still a platform
    /// `builtin`). Only a NON-builtin method on a Record with no resolvable table
    /// becomes `Unknown { RecordTableProcedure }`, decided in Phase B.
    Record { table_object_id: Option<String> },
    /// A `RecordRef` receiver — catalog-only in Phase B.
    RecordRef,
    /// A `FieldRef` receiver — catalog-only in Phase B.
    FieldRef,
    /// A `KeyRef` receiver — catalog-only in Phase B.
    KeyRef,
    /// A framework data type (`Json*` / `Http*` / `In`/`OutStream` / `List` /
    /// `Dictionary` / `TextBuilder` / `Dialog` / `Xml*`) — catalog-only in Phase B.
    Framework { kind: ReceiverBuiltinKind },
    /// A primitive / unrecognized non-object, non-catalog type. Phase B
    /// turns it into `Unknown { NonObjectReceiverType }`.
    Primitive,
    /// A `Variant`-typed receiver — the held type (and therefore the method
    /// dispatch) is determined at RUNTIME. This is NOT a resolution failure; per the
    /// honest taxonomy (spec §6) it is genuinely `dynamic`. Phase B emits a
    /// dynamic-dispatch edge (classified `dynamic`, not real-`unknown`).
    Dynamic,
    /// Fail-closed sink — Phase A could not positively type the receiver. Carries
    /// the attributed reason for the `aldump --l3-unknown-breakdown` diagnostic.
    Unknown { reason: UnknownReason },
}

/// The (declared-type-bearing) receiver value Phase A hands to Phase B. The
/// `declared_type` is carried alongside the lattice value because Object/Record
/// `Resolved` edges stamp the receiver's declared type onto the edge
/// (`receiver_type`), and the framework-catalog-miss edge needs the same string
/// for its `unknown_method_name` detail — neither is recoverable from the lattice
/// variant alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InferredReceiver {
    pub ty: ReceiverType,
    /// The receiver variable's declared type (verbatim), or empty when Phase A
    /// declined before a variable was found (compound / untracked receivers — the
    /// corresponding `Unknown` edges never read it).
    pub declared_type: String,
    /// DIAGNOSTIC-only sub-characterization for `UntrackedReceiver` /
    /// `CompoundReceiver` — used by Phase B's `Unknown` arm to stamp
    /// `CallEdge::receiver_shape` for the `--l3-unknown-breakdown` histogram.
    /// `None` on all other paths.
    pub receiver_shape: Option<String>,
}

/// Everything Phase B needs that is not the receiver type or the method name. A
/// thin bundle of the per-callsite resolution context so `dispatch` reads as one
/// typed `match` rather than a parameter swarm.
pub(crate) struct DispatchCtx<'a> {
    pub from: &'a str,
    pub callsite_id: &'a str,
    pub operation_id: &'a str,
    pub routine: &'a L3Routine,
    pub call_site: &'a PCallSite,
    pub symbols: &'a SymbolTable,
    pub state: &'a mut BindingState,
    pub diagnostics: &'a mut Vec<Diagnostic>,
    pub unfetched_declared_dependency: bool,
}

// ---------------------------------------------------------------------------
// Phase A — type inference.
// ---------------------------------------------------------------------------

/// Phase A: infer the [`ReceiverType`] of a member-call receiver expression.
///
/// Reproduces EXACTLY the legacy `PCallee::Member` Phase-A logic:
///   1. `simple_receiver_name` declines a compound receiver ⇒
///      `Unknown { CompoundReceiver }`.
///   2. the name is not a tracked variable (param/local/global) ⇒
///      `Unknown { UntrackedReceiver }`.
///   3. `parse_object_type_ref` recognizes the declared type ⇒ `Interface` /
///      `Enum` / `Object`.
///   4. otherwise `classify_receiver` against the builtin catalog ⇒ `Record`
///      (carrying its table object id, `None` when the table did not resolve —
///      the catalog-builtin-first decision in Phase B does not need it) /
///      `RecordRef` / `FieldRef` / `KeyRef` / `Framework`; an unrecognized type ⇒
///      `Primitive`.
pub fn infer_receiver_type(
    receiver_expr: &str,
    routine: &L3Routine,
    symbols: &SymbolTable,
) -> InferredReceiver {
    // Step 1 — simple receiver name.
    let Some(receiver_name) = simple_receiver_name(receiver_expr) else {
        // Member-of-member: `<recvar>.<field>` where the field is a Blob / Media /
        // MediaSet field of the record's table exposes the field intrinsics
        // (`DOTempBlob.Blob.CreateOutStream(...)`, `PDFDocument."File Blob".CreateInStream(...)`).
        // Resolve before declining to CompoundReceiver.
        if let Some(kind) = compound_blob_media_field_kind(receiver_expr, routine, symbols) {
            return InferredReceiver {
                ty: ReceiverType::Framework { kind },
                declared_type: String::new(),
                receiver_shape: None,
            };
        }
        return InferredReceiver {
            ty: ReceiverType::Unknown {
                reason: UnknownReason::CompoundReceiver,
            },
            declared_type: String::new(),
            receiver_shape: Some(compound_receiver_shape(receiver_expr)),
        };
    };

    // Step 2 — find the receiver variable (params → locals → globals).
    let Some(recv_var) = routine.variables.iter().find(|v| v.name == receiver_name) else {
        // Step 2b — a `record_variables`-backed receiver with no declared local/
        // param. The implicit `Rec`/`xRec` (and any object-global record var) is
        // seeded here by L2/record_types; its mere PRESENCE proves the receiver IS
        // a Record. It is a Record REGARDLESS of whether its table object id
        // resolves: a cross-app / dependency SourceTable (common in extension apps
        // like CDO) leaves `table_id` None, but Record intrinsics
        // (Insert/Modify/SetRange/…) still classify as `builtin` in Phase B, and a
        // genuine table procedure on an unresolved table becomes the honest
        // `RecordTableProcedure`. Neither is an `UntrackedReceiver` inference
        // failure — mirroring Step 4's table-id-independent decision for DECLARED
        // record vars. Gate on entry existence, pass best-effort `table_object_id`.
        let receiver_name_lc = receiver_name.to_lowercase();
        if let Some(rv) = routine
            .record_variables
            .iter()
            .find(|rv| rv.name.to_lowercase() == receiver_name_lc)
        {
            let table_object_id = rv
                .table_id
                .as_deref()
                .and_then(|tid| symbols.table_by_id(tid))
                .map(|t| t.name.clone())
                .and_then(|tname| symbols.object_by_type_name("Table", &tname))
                .map(|obj| obj.id.clone());
            let declared_type = format!(
                "Record {}",
                rv.table_name.as_deref().unwrap_or(&receiver_name)
            );
            return InferredReceiver {
                ty: ReceiverType::Record { table_object_id },
                declared_type,
                receiver_shape: None,
            };
        }

        // Step 2c — language singletons: CurrPage / CurrReport and the AL platform
        // static-API singleton type names (IsolatedStorage, Session, NavApp,
        // TaskScheduler, Database, Page, Report) are not declared variables but are
        // platform-provided receivers. Intercept them here before emitting
        // UntrackedReceiver. The variables-first check (Step 2 above) already ran,
        // so a user variable with the same name (e.g. `var Session: Codeunit X`)
        // correctly shadows these and reaches this point only for BARE names with
        // no matching variable declaration.
        let receiver_name_lc = receiver_name.to_lowercase();
        let singleton_kind = match receiver_name_lc.as_str() {
            "currpage" | "page" => Some(ReceiverBuiltinKind::PageInstance),
            "currreport" | "report" => Some(ReceiverBuiltinKind::ReportInstance),
            "isolatedstorage" => Some(ReceiverBuiltinKind::IsolatedStorage),
            "session" => Some(ReceiverBuiltinKind::Session),
            "navapp" => Some(ReceiverBuiltinKind::NavApp),
            "taskscheduler" => Some(ReceiverBuiltinKind::TaskScheduler),
            "database" => Some(ReceiverBuiltinKind::Database),
            _ => None,
        };
        if let Some(kind) = singleton_kind {
            return InferredReceiver {
                ty: ReceiverType::Framework { kind },
                declared_type: String::new(),
                receiver_shape: None,
            };
        }

        // Step 2d — a bare FIELD of the implicit `Rec` used as a member receiver
        // (`"File Blob".CreateInStream(...)` in table/page code). A Blob / Media /
        // MediaSet field exposes the stream + media intrinsics; resolve the implicit
        // Rec's table and, if `receiver_name` names a blob/media-typed field, treat
        // it as that framework receiver so Phase B classifies the intrinsic as
        // `builtin`. Non-media fields are not callable, so they stay untracked.
        if let Some(kind) = implicit_rec_field_builtin_kind(&receiver_name_lc, routine, symbols) {
            return InferredReceiver {
                ty: ReceiverType::Framework { kind },
                declared_type: String::new(),
                receiver_shape: None,
            };
        }

        return InferredReceiver {
            ty: ReceiverType::Unknown {
                reason: UnknownReason::UntrackedReceiver,
            },
            declared_type: String::new(),
            receiver_shape: Some(untracked_receiver_shape(&receiver_name)),
        };
    };
    let declared_type = recv_var.declared_type.clone();

    // Step 3 — parse the declared type into an object type reference.
    if let Some(type_ref) = parse_object_type_ref(&declared_type) {
        let ty = match type_ref.kind {
            ObjectKind::Interface => ReceiverType::Interface {
                name: type_ref.name,
            },
            ObjectKind::Enum => ReceiverType::Enum {
                name: type_ref.name,
            },
            _ => ReceiverType::Object {
                kind: type_ref.kind,
                name: type_ref.name,
            },
        };
        return InferredReceiver {
            ty,
            declared_type,
            receiver_shape: None,
        };
    }

    // Step 4 — builtin-catalog classification (Record / RecordRef / FieldRef /
    // KeyRef / framework), or a primitive / unrecognized type.
    let ty = match classify_receiver(&declared_type) {
        Some(ReceiverBuiltinKind::Record) => {
            // The table id may not resolve (out-of-source / dependency table) — that
            // is NOT an inference failure. A Record receiver is ALWAYS `Record`; the
            // catalog-builtin check in Phase B is table-independent, and only a
            // non-builtin method with no resolvable table becomes the honest
            // `Unknown { RecordTableProcedure }` (decided in Phase B).
            let table_object_id =
                resolve_record_table_object_id(&receiver_name, &declared_type, routine, symbols);
            ReceiverType::Record { table_object_id }
        }
        Some(ReceiverBuiltinKind::RecordRef) => ReceiverType::RecordRef,
        Some(ReceiverBuiltinKind::FieldRef) => ReceiverType::FieldRef,
        Some(ReceiverBuiltinKind::KeyRef) => ReceiverType::KeyRef,
        Some(kind) => ReceiverType::Framework { kind },
        // A `Variant` receiver is genuinely runtime-typed → honest `dynamic`. Any
        // other non-object / unrecognized type stays `Primitive` (→ unknown).
        None if declared_type_first_token_lc(&declared_type) == "variant" => ReceiverType::Dynamic,
        None => ReceiverType::Primitive,
    };
    InferredReceiver {
        ty,
        declared_type,
        receiver_shape: None,
    }
}

/// If `field_name_lc` names a `Blob` / `Media` / `MediaSet` field on the implicit
/// `Rec`/`xRec`'s table, return the corresponding framework receiver kind — so a
/// bare field-as-receiver member call (`"File Blob".CreateInStream(...)`) dispatches
/// the field intrinsic as `builtin` instead of degrading to `UntrackedReceiver`. The
/// implicit Rec's table id is resolved by `record_types` pass 3 (Table self / Page
/// SourceTable / extension base). `None` when there is no implicit Rec, the table is
/// out-of-source, or the field is not a media-bearing type.
fn implicit_rec_field_builtin_kind(
    field_name_lc: &str,
    routine: &L3Routine,
    symbols: &SymbolTable,
) -> Option<ReceiverBuiltinKind> {
    let table = routine
        .record_variables
        .iter()
        .find(|rv| {
            let n = rv.name.to_lowercase();
            n == "rec" || n == "xrec"
        })
        .and_then(|rv| rv.table_id.as_deref())
        .and_then(|tid| symbols.table_by_id(tid))?;
    let field = table
        .fields
        .iter()
        .find(|f| f.name.to_lowercase() == field_name_lc)?;
    match field.data_type.to_lowercase().as_str() {
        "blob" => Some(ReceiverBuiltinKind::Blob),
        "media" | "mediaset" => Some(ReceiverBuiltinKind::Media),
        _ => None,
    }
}

/// Member-of-member blob/media resolution: for a receiver `<base>.<field>` where
/// `base` is a simple record receiver (a record var/param/global or the implicit
/// `Rec`/`xRec`) and `field` is a `Blob` / `Media` / `MediaSet` field of `base`'s
/// table, return the field-intrinsic framework kind. Splits on the LAST `.` so a
/// deeper chain (`CurrPage.Part.Page`) declines here (its `base` is itself compound
/// and `simple_receiver_name` rejects it). `None` when `base` is not a resolvable
/// record, the field is absent, or the field is not media-bearing.
fn compound_blob_media_field_kind(
    receiver_expr: &str,
    routine: &L3Routine,
    symbols: &SymbolTable,
) -> Option<ReceiverBuiltinKind> {
    let (base, member) = receiver_expr.rsplit_once('.')?;
    let base_name = simple_receiver_name(base)?;
    let member_name = simple_receiver_name(member)?;
    let table = routine
        .record_variables
        .iter()
        .find(|rv| rv.name.to_lowercase() == base_name)
        .and_then(|rv| rv.table_id.as_deref())
        .and_then(|tid| symbols.table_by_id(tid))?;
    let field = table
        .fields
        .iter()
        .find(|f| f.name.to_lowercase() == member_name)?;
    match field.data_type.to_lowercase().as_str() {
        "blob" => Some(ReceiverBuiltinKind::Blob),
        "media" | "mediaset" => Some(ReceiverBuiltinKind::Media),
        _ => None,
    }
}

/// Sub-characterize a compound receiver expression for the breakdown histogram.
/// A compound receiver is one that `simple_receiver_name` declined (contains `.`,
/// `(`, `[`, or similar). The returned string is the shape tag stored on the edge.
///
/// For `member-of-member` expressions the full expression (truncated to 120 chars)
/// is embedded as `"member-of-member::<expr>"` so `--l3-unknown-breakdown` can
/// surface concrete receiver expressions for targeting. Other well-known shapes
/// keep their short tag (no expression needed — they are self-explanatory).
fn compound_receiver_shape(receiver_expr: &str) -> String {
    if receiver_expr.contains('.') {
        // Embed the expression (capped) so the breakdown can show concrete samples.
        let expr = if receiver_expr.len() > 120 {
            &receiver_expr[..120]
        } else {
            receiver_expr
        };
        format!("member-of-member::{expr}")
    } else if receiver_expr.contains('(') {
        "call-result".to_string()
    } else if receiver_expr.contains('[') {
        "indexed".to_string()
    } else {
        "other".to_string()
    }
}

/// Sub-characterize an untracked receiver name for the breakdown histogram. The
/// name is the simple receiver name that could not be found in `routine.variables`.
///
/// For the `other` bucket the receiver name is embedded as `"other::<name>"` so
/// `--l3-unknown-breakdown` can surface concrete untracked receiver names for
/// targeting (object globals, `CurrPage`/`CurrReport` aliases, etc.).
fn untracked_receiver_shape(receiver_name: &str) -> String {
    match receiver_name.to_lowercase().as_str() {
        "rec" | "xrec" => "implicit-rec".to_string(),
        "currpage" => "currpage".to_string(),
        "currreport" => "currreport".to_string(),
        _ => format!("other::{receiver_name}"),
    }
}

/// Resolve a `Record`-typed receiver's declared table to its workspace table
/// OBJECT id (`{appGuid}/Table/{n}` — the `L3Routine.object_id` format, capital
/// `T` from `encode_object_id`). `L3Table.id` uses lowercase `table`, so the raw
/// table id cannot be passed to `resolve_by_name_and_arity`; we route through the
/// table NAME to the `Table` object id.
///
/// Resolution order (legacy parity):
///   1. match `receiver_name` in `routine.record_variables` → its resolved
///      `table_id` → `L3Table.name` → `object_by_type_name("Table", name)`.
///   2. fallback: parse the table name directly from `declared_type` via
///      `record_table_name_of` → `object_by_type_name("Table", name)`.
///
/// `None` when no table object resolves (Phase A then yields
/// `Unknown { RecordTableProcedure }`).
fn resolve_record_table_object_id(
    receiver_name: &str,
    declared_type: &str,
    routine: &L3Routine,
    symbols: &SymbolTable,
) -> Option<String> {
    let receiver_name_lc = receiver_name.to_lowercase();

    // Path 1: via the record variable's resolved table_id.
    let via_rv = routine
        .record_variables
        .iter()
        .find(|rv| rv.name.to_lowercase() == receiver_name_lc)
        .and_then(|rv| rv.table_id.as_deref())
        .and_then(|tid| symbols.table_by_id(tid))
        .map(|t| t.name.clone())
        .and_then(|tname| symbols.object_by_type_name("Table", &tname))
        .map(|obj| obj.id.clone());
    if via_rv.is_some() {
        return via_rv;
    }

    // Path 2: parse the table name from the declared type.
    super::record_types::record_table_name_of(declared_type)
        .and_then(|tname| symbols.object_by_type_name("Table", &tname))
        .map(|obj| obj.id.clone())
}

/// The first whitespace-delimited token of a declared type, lowercased (e.g.
/// `"Variant"` from `"Variant"`, `"variant"` from `"Variant temporary"`-style
/// noise). Used to recognize a `Variant` receiver as runtime-typed (`dynamic`).
fn declared_type_first_token_lc(declared_type: &str) -> String {
    declared_type
        .trim()
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_lowercase()
}

// ---------------------------------------------------------------------------
// Phase B — typed dispatch.
// ---------------------------------------------------------------------------

/// Phase B: dispatch a typed receiver + method into `CallEdge`s, one `match` arm
/// per [`ReceiverType`] variant. Every arm mirrors the exact
/// dispatch-kind/resolution/candidates/external-type-ref/receiver-type/
/// dispatch-meta and the `upgrade_bindings` / `mark_bindings_ambiguous`
/// side-effects of the legacy ladder.
pub(crate) fn dispatch(
    receiver: &InferredReceiver,
    method: &str,
    ctx: &mut DispatchCtx,
) -> Vec<CallEdge> {
    match &receiver.ty {
        ReceiverType::Object { kind, name } => {
            dispatch_object(*kind, name, &receiver.declared_type, method, ctx)
        }
        ReceiverType::Interface { name } => resolve_interface_dispatch(
            ctx.from,
            ctx.callsite_id,
            ctx.operation_id,
            name,
            method,
            ctx.routine,
            ctx.call_site,
            ctx.symbols,
            ctx.state,
        ),
        ReceiverType::Enum { .. } => unknown_method(
            ctx.from,
            ctx.callsite_id,
            ctx.operation_id,
            UnknownReason::EnumStatic,
        ),
        ReceiverType::Record { table_object_id } => dispatch_record(
            table_object_id.as_deref(),
            &receiver.declared_type,
            method,
            ctx,
        ),
        ReceiverType::RecordRef => dispatch_framework(ReceiverBuiltinKind::RecordRef, method, ctx),
        ReceiverType::FieldRef => dispatch_framework(ReceiverBuiltinKind::FieldRef, method, ctx),
        ReceiverType::KeyRef => dispatch_framework(ReceiverBuiltinKind::KeyRef, method, ctx),
        ReceiverType::Framework { kind } => dispatch_framework(*kind, method, ctx),
        ReceiverType::Primitive => unknown_method(
            ctx.from,
            ctx.callsite_id,
            ctx.operation_id,
            UnknownReason::NonObjectReceiverType,
        ),
        ReceiverType::Dynamic => dynamic_method(ctx.from, ctx.callsite_id, ctx.operation_id),
        ReceiverType::Unknown { reason } => {
            let mut edges = unknown_method(ctx.from, ctx.callsite_id, ctx.operation_id, *reason);
            if let Some(shape) = receiver.receiver_shape.clone() {
                if let Some(e) = edges.first_mut() {
                    e.receiver_shape = Some(shape);
                }
            }
            edges
        }
    }
}

/// Object dispatch (Codeunit / Page / Report / Query / XmlPort).
///
/// `object_by_type_name` miss ⇒ external-target / opaque (gated by the
/// unfetched-declared-dependency boolean), carrying the `external_type_ref`.
/// Otherwise `resolve_by_name_and_arity` against the object id, with the Codeunit
/// `.Run` → `OnRun` special case on `NotFound`.
fn dispatch_object(
    kind: ObjectKind,
    name: &str,
    declared_type: &str,
    method: &str,
    ctx: &mut DispatchCtx,
) -> Vec<CallEdge> {
    let Some(obj) = ctx.symbols.object_by_type_name(kind.as_str(), name) else {
        // Object named but not in indexed source.
        let external = ExternalTypeRef {
            kind: kind.as_str().to_string(),
            name: name.to_string(),
        };
        let mut e = CallEdge::base(ctx.from, ctx.callsite_id, ctx.operation_id);
        e.dispatch_kind = DispatchKind::Method;
        e.external_type_ref = Some(external);
        e.resolution = if ctx.unfetched_declared_dependency {
            Resolution::Opaque
        } else {
            Resolution::ExternalTarget
        };
        return vec![e];
    };
    let obj_id = obj.id.clone();

    match resolve_by_name_and_arity(ctx.symbols, &obj_id, method, ctx.routine, ctx.call_site) {
        ArityResolution::Resolved(r) => {
            if let Some(d) = upgrade_bindings(ctx.state, r, ctx.callsite_id) {
                ctx.diagnostics.push(d);
            }
            let mut e = CallEdge::base(ctx.from, ctx.callsite_id, ctx.operation_id);
            e.to = Some(r.id.clone());
            e.dispatch_kind = DispatchKind::Method;
            e.resolution = Resolution::Resolved;
            e.receiver_type = Some(declared_type.to_string());
            vec![e]
        }
        ArityResolution::NotFound => {
            // Built-in instance `<codeunitVar>.Run([Rec])` → OnRun trigger, when
            // the codeunit has an OnRun and arity ≤ 1.
            if kind == ObjectKind::Codeunit
                && method.to_lowercase() == "run"
                && ctx.call_site.argument_bindings.len() <= 1
            {
                if let Some(on_run) = ctx.symbols.routine_in_object(&obj_id, "OnRun") {
                    if let Some(d) = upgrade_bindings(ctx.state, on_run, ctx.callsite_id) {
                        ctx.diagnostics.push(d);
                    }
                    let mut e = CallEdge::base(ctx.from, ctx.callsite_id, ctx.operation_id);
                    e.to = Some(on_run.id.clone());
                    e.dispatch_kind = DispatchKind::CodeunitRun;
                    e.resolution = Resolution::Resolved;
                    return vec![e];
                }
            }
            let mut e = CallEdge::base(ctx.from, ctx.callsite_id, ctx.operation_id);
            e.dispatch_kind = DispatchKind::Method;
            e.resolution = Resolution::MemberNotFound;
            vec![e]
        }
        ArityResolution::NoArityMatch(candidates) => {
            let ids = sorted_ids(&candidates);
            let mut e = CallEdge::base(ctx.from, ctx.callsite_id, ctx.operation_id);
            e.dispatch_kind = DispatchKind::Method;
            e.resolution = Resolution::MemberNotFound;
            if !ids.is_empty() {
                e.candidates = Some(ids);
            }
            vec![e]
        }
        ArityResolution::Ambiguous(candidates) => {
            mark_bindings_ambiguous(ctx.state);
            let mut e = CallEdge::base(ctx.from, ctx.callsite_id, ctx.operation_id);
            e.dispatch_kind = DispatchKind::Method;
            e.resolution = Resolution::Ambiguous;
            e.candidates = Some(sorted_ids(&candidates));
            vec![e]
        }
    }
}

/// Record dispatch — absorbs the legacy "surgical" Record-table-procedure block.
///
/// CATALOG-FIRST ordering (preserved exactly): a Record builtin (`FieldNo`,
/// `SetRange`, …) is a platform terminal and emits `builtin`; it is NEVER
/// table-dispatched, and the check is INDEPENDENT of whether the receiver's table
/// resolved (a `SetRange` on a Record typed against a dependency-app table is
/// still `builtin`). Only a NON-builtin method goes to the table:
///   * `table_object_id == Some` ⇒ `resolve_by_name_and_arity` against it
///     (Resolved / NoArityMatch / Ambiguous; `NotFound` ⇒ the honest unknown);
///   * `table_object_id == None` ⇒ no resolvable table, the honest
///     `Unknown { RecordTableProcedure }` signal (legacy "no table id → unknown").
fn dispatch_record(
    table_object_id: Option<&str>,
    declared_type: &str,
    method: &str,
    ctx: &mut DispatchCtx,
) -> Vec<CallEdge> {
    let method_lc = method.to_lowercase();
    // Catalog-builtin FIRST — a Record intrinsic stays `builtin`, never dispatched
    // to a table procedure, regardless of whether the table resolved.
    if member_builtin_disposition(ReceiverBuiltinKind::Record, &method_lc).is_some() {
        let mut e = CallEdge::base(ctx.from, ctx.callsite_id, ctx.operation_id);
        e.dispatch_kind = DispatchKind::Builtin;
        e.resolution = Resolution::Builtin;
        return vec![e];
    }

    // Non-builtin method: only dispatchable when the table object id resolved.
    let Some(table_object_id) = table_object_id else {
        // Sub-cause TABLE-UNRESOLVED: the receiver's table object id never resolved
        // (the table is absent from the symbol set even with deps loaded). Tag the
        // diagnostic so `--l3-unknown-breakdown[-cross-app]` can split this from the
        // PROC-NOT-FOUND sub-cause below — they need different fixes.
        let mut edges = unknown_method(
            ctx.from,
            ctx.callsite_id,
            ctx.operation_id,
            UnknownReason::RecordTableProcedure,
        );
        if let Some(e) = edges.first_mut() {
            e.receiver_shape = Some(format!("table-unresolved::{declared_type}::{method_lc}"));
        }
        return edges;
    };

    // Search the base table UNION every TableExtension extending it — a
    // TableExtension procedure (CDO's `CDOOpenEmail`, etc.) is globally callable on
    // the base record in AL but lives under the extension's own object id, so the
    // base table's routine set alone would miss it (NotFound → false unknown).
    let mut search_ids: Vec<&str> = vec![table_object_id];
    if let Some(base_obj) = ctx.symbols.object_by_id(table_object_id) {
        search_ids.extend(
            ctx.symbols
                .table_extension_object_ids(&base_obj.name, base_obj.object_number),
        );
    }
    match resolve_by_name_and_arity_multi(
        ctx.symbols,
        &search_ids,
        method,
        ctx.routine,
        ctx.call_site,
    ) {
        ArityResolution::Resolved(r) => {
            if let Some(d) = upgrade_bindings(ctx.state, r, ctx.callsite_id) {
                ctx.diagnostics.push(d);
            }
            let mut e = CallEdge::base(ctx.from, ctx.callsite_id, ctx.operation_id);
            e.to = Some(r.id.clone());
            e.dispatch_kind = DispatchKind::Method;
            e.resolution = Resolution::Resolved;
            e.receiver_type = Some(declared_type.to_string());
            vec![e]
        }
        ArityResolution::NoArityMatch(candidates) => {
            let ids = sorted_ids(&candidates);
            let mut e = CallEdge::base(ctx.from, ctx.callsite_id, ctx.operation_id);
            e.dispatch_kind = DispatchKind::Method;
            e.resolution = Resolution::MemberNotFound;
            if !ids.is_empty() {
                e.candidates = Some(ids);
            }
            vec![e]
        }
        ArityResolution::Ambiguous(candidates) => {
            mark_bindings_ambiguous(ctx.state);
            let mut e = CallEdge::base(ctx.from, ctx.callsite_id, ctx.operation_id);
            e.dispatch_kind = DispatchKind::Method;
            e.resolution = Resolution::Ambiguous;
            e.candidates = Some(sorted_ids(&candidates));
            vec![e]
        }
        ArityResolution::NotFound => {
            // Genuinely not a table procedure — a builtin we lack in the catalog or
            // a real hole. Keep the honest unknown signal. Sub-cause PROC-NOT-FOUND:
            // the table object resolved but no routine of this name/arity exists on
            // it (a missing TableExtension proc, an uncataloged builtin, or a true
            // hole). Tagged distinctly from TABLE-UNRESOLVED above.
            let mut edges = unknown_method(
                ctx.from,
                ctx.callsite_id,
                ctx.operation_id,
                UnknownReason::RecordTableProcedure,
            );
            if let Some(e) = edges.first_mut() {
                e.receiver_shape = Some(format!("proc-not-found::{declared_type}::{method_lc}"));
            }
            edges
        }
    }
}

/// Framework / RecordRef / FieldRef / KeyRef dispatch — catalog-only. A catalog
/// hit ⇒ `builtin`; a miss ⇒ `Unknown { FrameworkMethodNotInCatalog }` carrying
/// the `"Kind::method_lc"` detail the breakdown histogram reads.
fn dispatch_framework(
    kind: ReceiverBuiltinKind,
    method: &str,
    ctx: &mut DispatchCtx,
) -> Vec<CallEdge> {
    let method_lc = method.to_lowercase();
    if member_builtin_disposition(kind, &method_lc).is_some() {
        let mut e = CallEdge::base(ctx.from, ctx.callsite_id, ctx.operation_id);
        e.dispatch_kind = DispatchKind::Builtin;
        e.resolution = Resolution::Builtin;
        return vec![e];
    }
    let mut edges = unknown_method(
        ctx.from,
        ctx.callsite_id,
        ctx.operation_id,
        UnknownReason::FrameworkMethodNotInCatalog,
    );
    if let Some(e) = edges.first_mut() {
        e.unknown_method_name = Some(format!("{:?}::{}", kind, method_lc));
    }
    edges
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::l2::features::PTempState;
    use crate::engine::l3::l3_workspace::{L3Object, L3RecordVariable, L3Table, L3Variable};

    fn empty_symbols() -> SymbolTable {
        SymbolTable::build(&[], &[], &[])
    }

    /// A symbol table with a single Table object + its internal `L3Table` entry,
    /// so the Record table-object-id resolution (table_id → name → Table object id)
    /// has something to find.
    fn symbols_with_table(table_id: &str, table_name: &str, table_object_id: &str) -> SymbolTable {
        let table = L3Table {
            id: table_id.to_string(),
            app_guid: "app".to_string(),
            table_number: 18,
            name: table_name.to_string(),
            fields: Vec::new(),
            keys: Vec::new(),
            is_temporary: false,
            is_extension_stub: false,
        };
        let object = L3Object {
            id: table_object_id.to_string(),
            app_guid: "app".to_string(),
            object_type: "Table".to_string(),
            object_number: 18,
            name: table_name.to_string(),
            source_table_name: None,
            extends_target_name: None,
            implements_interfaces: Some(Vec::new()),
            object_subtype: None,
            page_type: None,
            inherent_commit_behavior: None,
            source_table_temporary: None,
            page_controls: Vec::new(),
        };
        SymbolTable::build(&[object], &[table], &[])
    }

    fn temp_unknown() -> PTempState {
        PTempState {
            kind: "unknown".to_string(),
            value: None,
            parameter_index: None,
        }
    }

    /// A bare routine carrying just the `variables` (and optionally
    /// `record_variables`) Phase A inspects. All other fields default.
    fn routine_with(vars: Vec<L3Variable>, rec_vars: Vec<L3RecordVariable>) -> L3Routine {
        L3Routine {
            id: "obj/r".to_string(),
            stable_routine_id: String::new(),
            object_id: "obj".to_string(),
            object_type: "Codeunit".to_string(),
            name: "R".to_string(),
            kind: "procedure".to_string(),
            attributes_parsed: Vec::new(),
            app_guid: String::new(),
            object_number: 0,
            normalized_signature_hash: String::new(),
            body_available: true,
            parse_incomplete: false,
            record_variables: rec_vars,
            record_operations: Vec::new(),
            field_accesses: Vec::new(),
            variables: vars,
            parameters: Vec::new(),
            access_modifier: None,
            return_type: None,
            call_sites: Vec::new(),
            operation_sites: Vec::new(),
            statement_tree: None,
            loops: Vec::new(),
            source_anchor: crate::engine::l2::features::PAnchor {
                source_unit_id: "ws:test.al".to_string(),
                start_line: 0,
                start_column: 0,
                end_line: 0,
                end_column: 0,
                syntax_kind: "procedure".to_string(),
            },
            identifier_references: Vec::new(),
            unreachable_statements: Vec::new(),
            has_branching: false,
            var_assignments: Vec::new(),
            condition_references: Vec::new(),
            enclosing_member: None,
            originating_object: None,
            enclosing_member_range: None,
            entry_temp_guard_receiver: None,
        }
    }

    fn var(name: &str, declared_type: &str) -> L3Variable {
        L3Variable {
            name: name.to_string(),
            declared_type: declared_type.to_string(),
            is_parameter: false,
            parameter_index: None,
            initializer: None,
        }
    }

    fn infer(receiver: &str, vars: Vec<L3Variable>) -> ReceiverType {
        let routine = routine_with(vars, Vec::new());
        let symbols = empty_symbols();
        infer_receiver_type(receiver, &routine, &symbols).ty
    }

    #[test]
    fn compound_receiver_is_unknown() {
        // A dotted / compound expression — `simple_receiver_name` declines.
        assert_eq!(
            infer("a.b", vec![]),
            ReceiverType::Unknown {
                reason: UnknownReason::CompoundReceiver,
            }
        );
    }

    #[test]
    fn untracked_receiver_is_unknown() {
        // A simple name not present in the routine's variables.
        assert_eq!(
            infer("nosuchvar", vec![]),
            ReceiverType::Unknown {
                reason: UnknownReason::UntrackedReceiver,
            }
        );
    }

    #[test]
    fn object_codeunit_receiver() {
        assert_eq!(
            infer("cu", vec![var("cu", "Codeunit \"Sales-Post\"")]),
            ReceiverType::Object {
                kind: ObjectKind::Codeunit,
                name: "Sales-Post".to_string(),
            }
        );
    }

    #[test]
    fn object_page_receiver() {
        assert_eq!(
            infer("pg", vec![var("pg", "Page \"Customer Card\"")]),
            ReceiverType::Object {
                kind: ObjectKind::Page,
                name: "Customer Card".to_string(),
            }
        );
    }

    #[test]
    fn interface_receiver() {
        assert_eq!(
            infer("i", vec![var("i", "Interface IFoo")]),
            ReceiverType::Interface {
                name: "IFoo".to_string(),
            }
        );
    }

    #[test]
    fn enum_receiver() {
        assert_eq!(
            infer("e", vec![var("e", "Enum \"My Enum\"")]),
            ReceiverType::Enum {
                name: "My Enum".to_string(),
            }
        );
    }

    #[test]
    fn framework_receivers() {
        assert_eq!(
            infer("j", vec![var("j", "JsonObject")]),
            ReceiverType::Framework {
                kind: ReceiverBuiltinKind::JsonObject,
            }
        );
        assert_eq!(
            infer("l", vec![var("l", "List of [Text]")]),
            ReceiverType::Framework {
                kind: ReceiverBuiltinKind::List,
            }
        );
        assert_eq!(
            infer("tb", vec![var("tb", "TextBuilder")]),
            ReceiverType::Framework {
                kind: ReceiverBuiltinKind::TextBuilder,
            }
        );
    }

    #[test]
    fn ref_receivers() {
        assert_eq!(
            infer("rr", vec![var("rr", "RecordRef")]),
            ReceiverType::RecordRef
        );
        assert_eq!(
            infer("fr", vec![var("fr", "FieldRef")]),
            ReceiverType::FieldRef
        );
        assert_eq!(infer("kr", vec![var("kr", "KeyRef")]), ReceiverType::KeyRef);
    }

    #[test]
    fn variant_receiver_is_dynamic() {
        // A Variant receiver is runtime-typed → honest `dynamic` (not unknown).
        assert_eq!(infer("v", vec![var("v", "Variant")]), ReceiverType::Dynamic);
    }

    #[test]
    fn primitive_receiver() {
        // A recognized primitive — no object, no catalog kind.
        assert_eq!(
            infer("n", vec![var("n", "Integer")]),
            ReceiverType::Primitive
        );
        assert_eq!(infer("t", vec![var("t", "Text")]), ReceiverType::Primitive);
    }

    #[test]
    fn record_receiver_with_no_resolvable_table_is_record_with_none() {
        // A Record receiver whose table does not resolve against an empty symbol
        // table is STILL `Record` (table_object_id None) — NEVER Unknown. The
        // catalog-builtin-first decision in Phase B is table-independent, so a
        // `SetRange` on this receiver must still classify `builtin`. Only a
        // non-builtin method on this (table-less) Record becomes
        // `Unknown { RecordTableProcedure }`, and that is a Phase-B decision.
        assert_eq!(
            infer("rec", vec![var("rec", "Record Customer")]),
            ReceiverType::Record {
                table_object_id: None,
            }
        );
    }

    #[test]
    fn record_receiver_resolves_table_object_id_via_record_variable() {
        // record_variables carries a resolved table_id; the symbol table maps it to
        // a Table object → Phase A yields Record { table_object_id: Some(..) }.
        let vars = vec![var("rec", "Record Customer")];
        let rec_vars = vec![L3RecordVariable {
            id: "rv".to_string(),
            name: "Rec".to_string(),
            table_name: Some("Customer".to_string()),
            table_id: Some("tbl-internal".to_string()),
            is_parameter: false,
            parameter_index: None,
            temp_state: temp_unknown(),
            scope: None,
        }];
        let routine = routine_with(vars, rec_vars);
        let symbols = symbols_with_table("tbl-internal", "Customer", "obj/Table/18");
        let ty = infer_receiver_type("rec", &routine, &symbols).ty;
        assert_eq!(
            ty,
            ReceiverType::Record {
                table_object_id: Some("obj/Table/18".to_string()),
            }
        );
    }
}
