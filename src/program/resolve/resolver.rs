//! Bare-call resolution: resolve an unqualified `Foo()` call to its [`Route`]
//! target(s), topology-scoped, with evidence and witness.
//!
//! # Precedence (first hit wins)
//!
//! 1. **Own object** — a procedure named `name_lc` declared in `from_object`.
//! 2. **Extension base** — if `from_object` is a `*Extension`, search the base
//!    object (`TableExtension`→`Table`, `PageExtension`→`Page`, …).
//! 3. **Implicit-Rec** (beyond-1B.3b Task 3) — a bare call inside a `Table`/
//!    `Page`/`TableExtension`/`PageExtension` implicitly dispatches to `Rec` as
//!    a LAST-RESORT fallback, after Steps 1-2 have had first refusal. Every
//!    other object kind (Codeunit/Report/XmlPort/Query/…) structurally skips
//!    this step. Gated on `WithState::NoWithProven` (a bare call lexically
//!    inside a `with X do` is NEVER eligible — see [`crate::program::resolve::
//!    extract::WithState`]) and on [`resolve_in_table_scope`] (Task 2's
//!    visibility-scoped table∪extensions search). A table-scope candidate that
//!    collides in name+arity with a global builtin or a bare-callable
//!    page/instance intrinsic (`Update`/`Close`/…) is an UNPROVEN precedence —
//!    fail closed to `Unknown` rather than assume the table wins.
//! 4. **Global builtin** — `is_global_builtin(name_lc)` → `Catalog` route.
//! 5. **Unknown** — genuine resolution failure.
//!
//! # Arity matching (Phase 2 / Phase 3 Task 0; ambiguity guard beyond-1B.3b Task 2)
//!
//! `RoutineNodeId` now carries `params_count`, so each overload (same name,
//! different arity) is a distinct node in the graph and index.
//! `routines_in_object` returns one entry per distinct overload.  An overload
//! matches when `rid.params_count == arity`.  When EXACTLY ONE match is found
//! it is returned.  When the name is found but NO overload matches the arity,
//! OR more than one same-arity overload matches (a genuine SOURCE-overload
//! collision — source `sig_fp` is always 0, so two distinct same-arity
//! overloads are indistinguishable at the id level; full arg-type dispatch to
//! disambiguate is deferred), an `Unknown` route is emitted — no
//! false-confident edge to a wrong-arity OR ambiguously-picked target.  The
//! caller still stops at that precedence level (does NOT fall through to
//! extension-base / global-builtin), mirroring L3's MemberNotFound stop
//! semantics while surfacing the gap honestly.  Name-absent ⇒ `None` ⇒ fall
//! through.
//!
//! # Witness↔evidence contract
//!
//! `Evidence::Source` ⇒ `Witness::SourceSpan`
//! `Evidence::Catalog`⇒ `Witness::CatalogEntry`
//! `Evidence::Unknown`⇒ `Witness::None`

use al_syntax::ir::ObjectKind;

use crate::program::graph::ProgramGraph;
use crate::program::node::{AppRef, ObjKey, ObjectNodeId, RoutineNodeId};
use crate::program::node_extract::{Access, ObjectNode};
use crate::program::resolve::body_map::BodyMap;
use crate::program::resolve::builtins::{catalog_version, global_builtin_id};
use crate::program::resolve::edge::{
    AbiEventKind, AbiRoutineKey, AbiRoutineKind, BuiltinId, CanonicalSpan, DispatchShape, Edge,
    EdgeKind, Evidence, OpenWorldReason, Route, RouteTarget, SetCompleteness, SiteId, SourcePos,
    Witness, callee_fp,
};
use crate::program::resolve::extract::WithState;
use crate::program::resolve::index::ResolveIndex;
use crate::program::resolve::member_catalog::{
    MemberCatalogKind, member_builtin, member_builtin_id,
};
use crate::program::resolve::receiver::{
    FrameworkKind, ReceiverType, resolve_pageext_base_source_table, resolve_source_table_ref,
    resolve_tableext_base_table,
};
use crate::snapshot::TrustTier;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Map a trust tier to the resolution evidence level.
///
/// Source-bearing tiers (Workspace / EmbeddedSource / LocalSource{Verified,
/// Approximate}) produce `Evidence::Source`; symbol-only tables produce `Abi`.
fn tier_evidence(tier: TrustTier) -> Evidence {
    match tier {
        TrustTier::Workspace
        | TrustTier::EmbeddedSource
        | TrustTier::LocalSourceVerified
        | TrustTier::LocalSourceApproximate => Evidence::Source,
        TrustTier::SymbolOnly => Evidence::Abi,
    }
}

/// For an extension object kind, return the corresponding base object kind.
///
/// Returns `None` for non-extension kinds.
fn extension_base_kind(kind: ObjectKind) -> Option<ObjectKind> {
    match kind {
        ObjectKind::TableExtension => Some(ObjectKind::Table),
        ObjectKind::PageExtension => Some(ObjectKind::Page),
        ObjectKind::ReportExtension => Some(ObjectKind::Report),
        ObjectKind::EnumExtension => Some(ObjectKind::Enum),
        ObjectKind::PermissionSetExtension => Some(ObjectKind::PermissionSet),
        _ => None,
    }
}

/// Build a `Route` for a resolved routine.
///
/// - If the routine is in the `BodyMap` (source-bearing): `Evidence::Source` +
///   `Witness::SourceSpan { file: virtual_path, span: (start_byte, end_byte) }`.
/// - If the routine is `SymbolOnly` (dep boundary, body unavailable): `Evidence::Opaque` +
///   `Witness::AbiSymbol` — the symbol identity is retained as a boundary marker.
/// - If the routine is NOT in the `BodyMap` (integration gap on a source-bearing tier):
///   `Evidence::Unknown` + `Witness::None`.  Surface this as Unknown so the gap shows up
///   in `real_unknown_rate` rather than being silently hidden.
fn make_routine_route(
    rid: &RoutineNodeId,
    obj_tier: TrustTier,
    body_map: &BodyMap<'_>,
    graph: &ProgramGraph,
) -> Route {
    if let Some((decl, path)) = body_map.get_with_path(rid) {
        Route {
            target: RouteTarget::Routine(rid.clone()),
            evidence: tier_evidence(obj_tier),
            conditions: vec![],
            witness: Witness::SourceSpan {
                file: path.to_string(),
                span: (decl.origin.byte.start as u32, decl.origin.byte.end as u32),
            },
        }
    } else if obj_tier == TrustTier::SymbolOnly {
        // SymbolOnly routine: body unavailable (loaded from .app SymbolReference,
        // no source parse).  We know the symbol exists at this ABI boundary, so
        // return Opaque rather than Unknown.  This preserves identity across the
        // boundary and classifies the route as `Resolved` (not `Unknown`) in the
        // obligation metric, matching L3's External treatment of dep symbols.
        let (obj_num, obj_name_lc) = match &rid.object.key {
            ObjKey::Id(n) => (*n, String::new()),
            ObjKey::Name(s) => (0i64, s.clone()),
        };
        // Read the ABI-sourced routine/event kinds from the graph node.
        // `graph.routines` is sorted by RoutineNodeId (see build.rs), enabling
        // O(log n) lookup. Falls back to Procedure/None when the node is absent
        // (integration gap — should not happen for a valid SymbolOnly boundary).
        let opt_node = graph
            .routines
            .binary_search_by(|probe| probe.id.cmp(rid))
            .ok()
            .map(|i| &graph.routines[i]);
        let routine_kind = opt_node
            .and_then(|n| n.abi_routine_kind.clone())
            .unwrap_or(AbiRoutineKind::Procedure);
        let event_kind = opt_node
            .and_then(|n| n.abi_event_kind.clone())
            .unwrap_or(AbiEventKind::None);
        let key = AbiRoutineKey {
            app: rid.object.app,
            object_type: format!("{:?}", rid.object.kind).to_ascii_lowercase(),
            object_number: obj_num,
            object_name_lc: obj_name_lc,
            routine_name_lc: rid.name_lc.clone(),
            params_count: rid.params_count,
            param_type_fp: rid.sig_fp,
            routine_kind,
            event_kind,
        };
        Route {
            target: RouteTarget::AbiSymbol { key: key.clone() },
            evidence: Evidence::Opaque,
            conditions: vec![],
            witness: Witness::AbiSymbol { key },
        }
    } else {
        // Source-tier BodyMap miss: integration bug.  Surface it as Unknown so
        // the gap shows up in `real_unknown_rate` rather than being silently hidden.
        Route {
            target: RouteTarget::Unresolved,
            evidence: Evidence::Unknown,
            conditions: vec![],
            witness: Witness::None,
        }
    }
}

/// Try to resolve `name_lc` with `arity` arguments inside `obj_id`.
///
/// Returns the UNIQUE arity-matched overload as a `Source` route.  When the
/// name is found but NO overload matches the arity, OR more than one
/// same-arity overload matches (a genuine SOURCE-overload collision; see
/// module-level doc), returns an `Unknown` route — no false-confident edge to
/// a wrong-arity OR ambiguously-picked candidate (does NOT fall through to
/// the next precedence level; see module-level doc).  Returns `None` only
/// when the name is absent entirely in `obj_id`.
///
/// **SymbolOnly tier exception:** `params_count` is now populated from the ABI
/// (Task 1), but arity matching for SymbolOnly routines remains deferred
/// (caller-side type inference not yet implemented).  Any name match immediately
/// produces an `Opaque` boundary route (via [`make_routine_route`]) rather than
/// a false Unknown that would regress vs L3's External resolution.
fn resolve_in_object(
    obj_id: &ObjectNodeId,
    obj_tier: TrustTier,
    name_lc: &str,
    arity: usize,
    graph: &ProgramGraph,
    index: &ResolveIndex,
    body_map: &BodyMap<'_>,
) -> Option<Route> {
    let candidates = index.routines_in_object(obj_id, name_lc);
    if candidates.is_empty() {
        return None;
    }

    // SymbolOnly: `params_count` is now populated from the ABI (Task 1), but
    // arity matching remains deferred — caller-side type inference is not yet
    // implemented. Use the first candidate directly. `make_routine_route` returns
    // an Opaque boundary route for SymbolOnly BodyMap misses, carrying the
    // ABI-sourced routine_kind and event_kind from the graph node.
    if obj_tier == TrustTier::SymbolOnly {
        // SAFETY: candidates is non-empty (checked above).
        return Some(make_routine_route(
            candidates.first().unwrap(),
            obj_tier,
            body_map,
            graph,
        ));
    }

    // Arity-exact match: collect EVERY overload whose params_count == arity.
    // With params_count in RoutineNodeId, each overload is normally a distinct
    // node — but two DISTINCT SOURCE overloads sharing
    // (object, name_lc, params_count) collide onto one `RoutineNodeId` (source
    // `sig_fp` is always 0; see node.rs). `build_program_graph`'s dedup
    // (beyond-1B.3b Task 2) preserves every raw entry in that genuine
    // collision rather than silently dropping one, so >1 arity-matched
    // candidates here is REAL, unresolved ambiguity: no arg-type evidence
    // exists to pick between them (full arg-type dispatch is deferred — see
    // module doc). Exactly one candidate resolves normally; more than one
    // must fail closed — mirroring the interface-implementer fan-out's
    // `>1 candidates → Unresolved` rule (this module, `resolve_member`'s
    // `Interface` arm). Never pick-first.
    let matched: Vec<&RoutineNodeId> = candidates
        .iter()
        .filter(|rid| rid.params_count == arity)
        .collect();
    match matched.len() {
        0 => {}
        1 => return Some(make_routine_route(matched[0], obj_tier, body_map, graph)),
        _ => {
            return Some(Route {
                target: RouteTarget::Unresolved,
                evidence: Evidence::Unknown,
                conditions: vec![],
                witness: Witness::None,
            });
        }
    }

    // Name found but no arity-matched overload: emit Unknown rather than a
    // false-confident route to a wrong-arity candidate. Does NOT fall through
    // to extension-base / global-builtin — mirrors L3's MemberNotFound stop.
    Some(Route {
        target: RouteTarget::Unresolved,
        evidence: Evidence::Unknown,
        conditions: vec![],
        witness: Witness::None,
    })
}

/// Whether `obj_id` (at trust tier `obj_tier`) carries a visible source/ABI
/// candidate routine matching `method_lc`/`arity` — used by the Record-receiver
/// source-shadows-catalog precedence check (beyond-1B.3b Task 1) to determine
/// CARDINALITY across a multi-object scope (base table ∪ TableExtensions)
/// WITHOUT committing to a route.
///
/// Mirrors the matching rule [`resolve_in_object`] applies internally: a
/// SymbolOnly (ABI/dep) object counts ANY name match as a candidate (arity
/// matching is deferred for ABI routines — same exception `resolve_in_object`
/// documents); a source/ABI (non-SymbolOnly) tier object counts only an EXACT
/// arity match.
fn object_has_member_candidate(
    obj_id: &ObjectNodeId,
    obj_tier: TrustTier,
    method_lc: &str,
    arity: usize,
    index: &ResolveIndex,
) -> bool {
    let candidates = index.routines_in_object(obj_id, method_lc);
    if candidates.is_empty() {
        return false;
    }
    if obj_tier == TrustTier::SymbolOnly {
        return true;
    }
    candidates.iter().any(|rid| rid.params_count == arity)
}

/// Look up the declared [`Access`] of `rid` in `graph.routines` (already
/// sorted by `RoutineNodeId` — binary-searchable, mirroring
/// `make_routine_route`'s existing `graph.routines.binary_search_by` lookup
/// pattern). Returns `None` on a lookup miss — should never happen for a
/// `RoutineNodeId` sourced from `index.routines_in_object` (the index is
/// built directly from `graph.routines`), but if it ever does, the caller
/// ([`object_has_visible_member_candidate`]) fails closed rather than
/// assuming the routine is visible.
fn lookup_routine_access(graph: &ProgramGraph, rid: &RoutineNodeId) -> Option<Access> {
    graph
        .routines
        .binary_search_by(|probe| probe.id.cmp(rid))
        .ok()
        .map(|i| graph.routines[i].access)
}

/// Like [`object_has_member_candidate`], but additionally excludes a
/// candidate whose declared [`Access`] is not visible from the CALLING
/// object's identity `from_object` — the caller-identity-aware visibility
/// check (beyond-1B.3b Task 1, superseding the app-scoped Task 2 version).
///
/// # Why this is additive, not a behavior change, for SymbolOnly candidates
///
/// SymbolOnly (ABI-ingested `.app` dependency) objects already drop
/// `is_local`/`is_internal` routines entirely at ingestion time
/// (`abi_ingest::extract_abi_nodes`) and hardcode every surviving routine's
/// `Access` to `Public` (`abi_ingest.rs:283`) — so `object_has_member_
/// candidate`'s existing SymbolOnly short-circuit (any name match counts)
/// already never sees a non-Public ABI routine; this function's access check
/// is a no-op for that tier and is skipped entirely for it (avoids a lookup
/// against ABI routine data that was never populated with real modifiers).
///
/// # SOURCE-tier per-candidate `Access` rule (RESOLVED OBJECT IDENTITY, never
/// a lowercased-name comparison — every branch below compares
/// [`ObjectNodeId`]s or [`AppRef`]s, both derived from `graph`/`index`
/// identity, never from `Origin`/source text)
///
/// - [`Access::Public`] → always visible.
/// - [`Access::Local`] → visible ONLY when `obj_id == from_object` (the
///   candidate's declaring object IS the calling object itself — AL's
///   `local` is OBJECT-scoped, not app-scoped; this was the first latent
///   false-`Source` this task closes: the pre-fix code treated ANY same-app
///   candidate as visible, so a same-app but DIFFERENT object's `local`
///   procedure false-resolved to `Source`).
/// - [`Access::Internal`] → visible when `obj_id.app == from_object.app`
///   (app-scoped; unaffected by this task). Cross-app `internal` fails
///   closed to `Unknown` — AL's `InternalsVisibleTo`/friend-app exception is
///   OUT OF SCOPE here (a documented recall cost, not a soundness hole: a
///   false `Unknown` is never the cardinal sin a false `Source` is).
/// - [`Access::Protected`] → visible when `obj_id == from_object` (self) OR
///   `index.object_extends(graph, from_object, obj_id)` is `true` — `from_object`
///   is a DIRECT, kind-compatible extension of the candidate's declaring
///   object (see [`ResolveIndex::object_extends`] for the full DIRECT +
///   KIND-COMPATIBLE + never-reverse + never-peer contract). This closes the
///   second latent false-`Source`: the pre-fix code left `Protected`
///   completely unfiltered for any same-app candidate, including a
///   same-app-but-unrelated object AND a PEER extension of the same base
///   (the sibling-bleed case — the single biggest false-`Source` this task
///   closes).
/// - Lookup miss (`None`) → fails closed (excluded), never assumed visible.
fn object_has_visible_member_candidate(
    obj_id: &ObjectNodeId,
    obj_tier: TrustTier,
    method_lc: &str,
    arity: usize,
    from_object: &ObjectNodeId,
    graph: &ProgramGraph,
    index: &ResolveIndex,
) -> bool {
    if !object_has_member_candidate(obj_id, obj_tier, method_lc, arity, index) {
        return false;
    }
    if obj_tier == TrustTier::SymbolOnly {
        return true;
    }
    index
        .routines_in_object(obj_id, method_lc)
        .iter()
        .filter(|rid| rid.params_count == arity)
        .any(|rid| match lookup_routine_access(graph, rid) {
            Some(Access::Public) => true,
            Some(Access::Local) => obj_id == from_object,
            Some(Access::Internal) => obj_id.app == from_object.app,
            Some(Access::Protected) => {
                obj_id == from_object || index.object_extends(graph, from_object, obj_id)
            }
            None => false,
        })
}

/// Resolve `name_lc`/`arity` against the VISIBILITY-SCOPED table scope: the
/// base table `table_id` plus every `TableExtension` of it that is reachable
/// in `from_object`'s compile-time app dependency closure (beyond-1B.3b Task
/// 2; extracted from `resolve_member`'s `Record` arm so a future caller with
/// the same scope+cardinality need — e.g. `resolve_bare`'s implicit-Rec
/// lookup — can reuse the identical algorithm rather than re-deriving it).
///
/// # Visibility scoping (the Task 2 soundness fix)
///
/// Two INDEPENDENT fail-closed filters narrow the raw scope before
/// cardinality is counted — either one dropping a candidate can turn a
/// pre-Task-2 false `Source` into a correct decline:
///
/// 1. **Closure filter.** [`ResolveIndex::table_extensions_of`] is
///    whole-snapshot (`WorldMode::AnalyzedSnapshot` — Task 2 Step 1
///    investigation confirmed it has no app-scoping, unlike
///    `object_by_number`/`resolve_object_ref`). A `TableExtension` declared
///    in an app OUTSIDE `from_object`'s transitive dependency closure is a
///    symbol `from_object`'s own app never imported — the real AL compiler
///    could never have resolved a call to it. Such an extension is dropped
///    from `scope` entirely, not merely deprioritized. The base table
///    (`table_id`) is gated the same way, defense-in-depth (it is normally
///    already closure-validated by the receiver-inference stage that
///    produced it — see `receiver::resolve_source_table_ref` — but
///    re-checking here makes this helper safe to call independent of that
///    upstream guarantee).
/// 2. **Access filter.** A candidate procedure whose declared [`Access`] is
///    not visible from `from_object`'s identity is excluded from the
///    candidate count — `Local` requires `from_object` to BE the candidate's
///    declaring object, `Internal` requires the same app, `Protected`
///    requires self OR a direct kind-compatible extension relationship (see
///    [`object_has_visible_member_candidate`] for the full per-access
///    rationale, including why this is a no-op for SymbolOnly/ABI candidates
///    — beyond-1B.3b Task 1, superseding the earlier app-scoped-only Task 2
///    version of this filter).
///
/// # Cardinality (unchanged from the pre-extraction Record arm)
///
/// - 0 visible candidates (or `table_id` itself not visible) → `None` — fall
///   through to the caller's next precedence level (e.g. the Record builtin
///   catalog).
/// - Exactly 1 visible candidate → `Some((Exact, [route]))`, a single
///   `Source`/`Abi`/`Opaque` route via [`resolve_in_object`].
/// - `>1` visible candidates → `Some(member_unknown_route())` — honest
///   ambiguous `Unknown`; never pick-first, never fall through to the
///   catalog (source ambiguity still shadows a same-named intrinsic).
///
/// Deterministic: `scope` is explicitly sorted by `ObjectNodeId` before
/// cardinality is counted.
fn resolve_in_table_scope(
    from_object: &ObjectNode,
    table_id: ObjectNodeId,
    name_lc: &str,
    arity: usize,
    graph: &ProgramGraph,
    index: &ResolveIndex,
    body_map: &BodyMap<'_>,
) -> Option<(DispatchShape, Vec<Route>)> {
    let closure = graph.topology.closure(from_object.id.app);

    if !closure.contains(&table_id.app) {
        return None;
    }
    let (table_tier, table_name_lc) = graph
        .objects
        .iter()
        .find(|o| o.id == table_id)
        .map(|o| (o.tier, o.name.to_ascii_lowercase()))?;

    // Visible scope: the base table plus every TableExtension of it that is
    // reachable in `from_object`'s app dependency closure.
    let mut scope: Vec<(ObjectNodeId, TrustTier)> = vec![(table_id.clone(), table_tier)];
    for ext_id in index.table_extensions_of(&table_name_lc) {
        if !closure.contains(&ext_id.app) {
            // Outside from_object's dependency closure: invisible, not a
            // candidate (the Task 2 soundness fix).
            continue;
        }
        let Some(ext_tier) = graph
            .objects
            .iter()
            .find(|o| &o.id == ext_id)
            .map(|o| o.tier)
        else {
            continue;
        };
        scope.push((ext_id.clone(), ext_tier));
    }
    // Deterministic ordering (candidate vectors sorted by stable ObjectNodeId).
    scope.sort_by(|(a, _), (b, _)| a.cmp(b));

    let mut candidate_objects = scope.iter().filter(|(oid, tier)| {
        object_has_visible_member_candidate(
            oid,
            *tier,
            name_lc,
            arity,
            &from_object.id,
            graph,
            index,
        )
    });
    let first = candidate_objects.next();
    let second = candidate_objects.next();

    match (first, second) {
        (None, _) => None,
        (Some(_), Some(_)) => Some(member_unknown_route()),
        (Some((oid, tier)), None) => {
            resolve_in_object(oid, *tier, name_lc, arity, graph, index, body_map)
                .map(|route| (DispatchShape::Exact, vec![route]))
        }
    }
}

/// Compute the implicit-`Rec` table `ObjectNodeId` for `resolve_bare`'s Step 3
/// (bare unqualified calls implicitly dispatching to `Rec` — beyond-1B.3b
/// Task 3), by `from_object`'s kind. Reuses the SAME fail-closed per-kind
/// lookups `infer_implicit_rec` (`receiver.rs`) already established for the
/// EXPLICIT `Rec.Foo()` member-call case (Tasks 5-7) — a guessed table is the
/// cardinal sin either way, so there is exactly one correct answer per kind
/// and no reason to re-derive it.
///
/// Deliberately narrower than `infer_implicit_rec`: `Codeunit` (`TableNo`) is
/// NOT handled here. `resolve_bare`'s Step 3 caller already structurally
/// excludes every kind but `{Table, Page, TableExtension, PageExtension}`
/// before this is ever called (AL's bare-implicit-dispatch fallback is a
/// Page/Table source-record mechanism, not a Codeunit `TableNo` one), so this
/// function is never invoked for a Codeunit — its `_ => None` arm is
/// defense-in-depth, not a live path.
///
/// Returns `None` when there is no unique in-closure table (no declared
/// property, ambiguous cross-app name, out-of-closure, unresolved) — the
/// caller falls through to Step 4 rather than guess.
fn implicit_rec_table_id(
    from_object: &ObjectNode,
    graph: &ProgramGraph,
    index: &ResolveIndex,
) -> Option<ObjectNodeId> {
    match from_object.id.kind {
        ObjectKind::Table => Some(from_object.id.clone()),
        ObjectKind::Page => from_object
            .source_table
            .as_ref()
            .and_then(|r| resolve_source_table_ref(from_object.id.clone(), r, graph, index)),
        ObjectKind::TableExtension => resolve_tableext_base_table(from_object, graph, index),
        ObjectKind::PageExtension => resolve_pageext_base_source_table(from_object, graph, index),
        _ => None,
    }
}

/// Whether `name_lc` is a global builtin OR a bare-callable page/instance
/// intrinsic (`member_catalog`'s `PageInstance` set: `Update`/`Close`/
/// `SetRecord`/…) — the collision set `resolve_bare`'s Step 3 PROBE-THEN-
/// DECIDE guard checks AFTER finding a table-scope candidate (never gates the
/// probe itself; see the Step 3 doc in this function's body). A bare call to
/// one of these names inside a Page/Table trigger is textually ambiguous
/// between "the implicit-Rec table's own procedure" and "the platform
/// intrinsic" — with no compiler-verified precedence rule captured here,
/// fail-closed to `Unknown` on any such collision rather than pick a side.
///
/// # Why this checks `Global`/`Framework(PageInstance)` only, never `Record`
/// (beyond-1B.3b Task 1, Item 4 — NOT a bug, do not "fix" by adding a
/// `Record` branch here)
///
/// `resolve_member`'s `Record` arm (see the comment there) deliberately lets
/// a visible source/ABI table candidate SHADOW a same-named `Record` catalog
/// builtin with NO collision guard — that is corpus-validated correct AL
/// precedence (42 real CDO `builtin-catalog-fp-collision` instances, e.g.
/// `Record::fieldno`, `Record::setrecfilter`; see
/// `resolve_member_record_source_proc_shadows_same_named_builtin`). This
/// function's collision set is exactly the TWO catalogs (`Global`,
/// `Framework(PageInstance)`) where the reverse holds — Step 3's own
/// table-scope candidate has NO compiler-verified precedence over those, so
/// it fails closed instead. Adding `Record` here would regress the
/// beyond-1B.3b Task 1 source-shadows-catalog fix back into a false
/// `Unknown`.
fn is_bare_builtin_or_page_intrinsic(name_lc: &str) -> bool {
    global_builtin_id(name_lc).is_some()
        || member_builtin(
            MemberCatalogKind::Framework(&FrameworkKind::PageInstance),
            name_lc,
        )
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Resolve a bare (unqualified) call to `name_lc` with `arity` arguments from
/// the context of `from_object`.
///
/// `with_state` is the call site's [`WithState`] (beyond-1B.3b Task 3): Step 3
/// (implicit-Rec) only runs when this is `NoWithProven` — see the module doc
/// and [`WithState`] itself for the two-signal fail-closed soundness
/// argument. Every OTHER precedence step is unaffected by `with_state` (a
/// `with` block does not change own-object/extension-base/builtin lookup).
///
/// Returns a `Vec<Route>` with exactly one entry (bare calls are
/// single-dispatch in AL; the vec wrapper aligns with the multi-route edge
/// model for future polymorphic cases).
pub fn resolve_bare(
    from_object: &ObjectNode,
    name_lc: &str,
    arity: usize,
    graph: &ProgramGraph,
    index: &ResolveIndex,
    body_map: &BodyMap<'_>,
    with_state: WithState,
) -> Vec<Route> {
    // 1. Own object.
    if let Some(route) = resolve_in_object(
        &from_object.id,
        from_object.tier,
        name_lc,
        arity,
        graph,
        index,
        body_map,
    ) {
        return vec![route];
    }

    // 2. Extension base (Task 1.5 — access-filtered). `resolve_in_object`
    // itself does ZERO access filtering, so gate it behind the SAME
    // caller-identity-aware visibility check Task 1 established
    // (`object_has_visible_member_candidate`): the calling object is the
    // extension (`from_object`), the candidate object is the resolved base
    // (`base_id`). Per the Task-1 rule: base `Local` is NEVER visible to a
    // bare call from an extension (base-self only, even though the caller IS
    // a direct extension); cross-app `Internal` requires the same app;
    // `Protected` is visible (the caller is by construction a direct,
    // kind-compatible extension of `base_id`, so the self-or-extends check
    // trivially holds — incidentally safe, not accidentally permissive);
    // `Public` is always visible. When the base member is not visible, Step 2
    // declines entirely (no `resolve_in_object` call) and falls through to
    // Step 3/4/5, exactly like the pre-existing "no candidate at all"
    // fallthrough shape.
    if let Some(base_kind) = extension_base_kind(from_object.id.kind)
        && let Some(extends_target) = from_object.extends_target.as_deref()
        && let Some(base_obj) = graph.resolve_object(from_object.id.app, base_kind, extends_target)
    {
        let base_id = base_obj.id.clone();
        let base_tier = base_obj.tier;
        if object_has_visible_member_candidate(
            &base_id,
            base_tier,
            name_lc,
            arity,
            &from_object.id,
            graph,
            index,
        ) && let Some(route) =
            resolve_in_object(&base_id, base_tier, name_lc, arity, graph, index, body_map)
        {
            return vec![route];
        }
    }

    // 3. Implicit-Rec (beyond-1B.3b Task 3). Every guard below is
    // independently fail-closed; any of them declining routes straight past
    // this step to Step 4/5 rather than guessing.
    //
    // (0) STRICT ObjectKind guard: bare-implicit-Rec dispatch is structurally
    // a Page/Table source-record mechanism in AL — ONLY these four kinds are
    // eligible. Every other kind (Codeunit/Report/XmlPort/Query/…) skips this
    // step entirely, no accidental leakage via `implicit_rec_table_id`'s own
    // (defense-in-depth) kind match.
    if matches!(
        from_object.id.kind,
        ObjectKind::Table
            | ObjectKind::Page
            | ObjectKind::TableExtension
            | ObjectKind::PageExtension
    ) {
        // (1) with-guard: Step 3 runs ONLY on a proven with-free call site.
        // `InsideWith`/`Unknown` (the AST places the site inside a `with`, or
        // the two with-detection signals disagree) skip Step 3 — a false
        // `Source` inside an unrepresented `with` is the fatal case this
        // guards against (see `WithState`'s doc).
        if with_state == WithState::NoWithProven
            // (2) Compute the implicit-Rec table id by kind; no unique
            // in-closure table → fall through (nothing to search).
            && let Some(table_id) = implicit_rec_table_id(from_object, graph, index)
            // (3) Visibility-scoped table ∪ extensions search (Task 2):
            // `None` (0 visible candidates) falls through to Step 4/5;
            // `Some` is either a clean Source/Abi/Opaque route or an honest
            // ambiguous Unknown (>1 visible candidate — never pick-first).
            && let Some((_, routes)) = resolve_in_table_scope(
                from_object,
                table_id,
                name_lc,
                arity,
                graph,
                index,
                body_map,
            )
        {
            // (4) Builtin/intrinsic PROBE-THEN-DECIDE: the probe (step 3)
            // already ran; a same-name+arity table-scope candidate exists
            // AND `name_lc` is also a global builtin or a bare-callable
            // page/instance intrinsic is an UNPROVEN precedence collision —
            // fail closed to `Unknown` rather than assume the table wins
            // (never emit `Catalog` here; Step 4 below is the only place
            // that does). No table-scope candidate at all means there is
            // nothing to collide with, so this arm is unreachable in that
            // case — the surrounding `if let` already required `Some`.
            if is_bare_builtin_or_page_intrinsic(name_lc) {
                return member_unknown_route().1;
            }
            return routes;
        }
    }

    // 4. Global builtin.
    if let Some(builtin_id) = global_builtin_id(name_lc) {
        return vec![Route {
            target: RouteTarget::Builtin(builtin_id.clone()),
            evidence: Evidence::Catalog,
            conditions: vec![],
            witness: Witness::CatalogEntry {
                id: builtin_id,
                catalog_version: catalog_version().to_string(),
            },
        }];
    }

    // 5. Unknown.
    vec![Route {
        target: RouteTarget::Unresolved,
        evidence: Evidence::Unknown,
        conditions: vec![],
        witness: Witness::None,
    }]
}

// ---------------------------------------------------------------------------
// ObjectRun resolution
// ---------------------------------------------------------------------------

/// Entry-trigger name for an object kind (Phase-2 correction over L3's always-"OnRun").
///
/// | Kind       | Trigger         |
/// |------------|-----------------|
/// | Codeunit   | `"onrun"`       |
/// | Page       | `"onopenpage"`  |
/// | Report     | `"onprereport"` |
/// | Query/Other| `"onrun"` (best-effort) |
fn entry_trigger_name(kind: ObjectKind) -> &'static str {
    match kind {
        ObjectKind::Page => "onopenpage",
        ObjectKind::Report => "onprereport",
        _ => "onrun",
    }
}

/// Build a single Opaque boundary route (target exists but is not in our source).
fn opaque_boundary_route(key: AbiRoutineKey) -> Route {
    Route {
        target: RouteTarget::AbiSymbol { key: key.clone() },
        evidence: Evidence::Opaque,
        conditions: vec![],
        witness: Witness::AbiSymbol { key },
    }
}

/// Resolve an `ObjectRun` dispatch (Codeunit.Run / Page.RunModal / Report.Run …)
/// to the entry trigger of the statically-named/numbered target object.
///
/// # Dispatch semantics
///
/// | Situation | Shape | Completeness | Routes |
/// |-----------|-------|-------------|--------|
/// | `target_ref` is `None` (runtime variable) | `DynamicOpen` | `Partial{RuntimeTypeUnbounded}` | `[{Unresolved, Unknown, None}]` |
/// | Target named/numbered but absent from graph | `Exact` | `Complete` | `[{AbiSymbol, Opaque, AbiSymbol}]` |
/// | Target found; entry trigger resolved | `Exact` | `Complete` | `[{Routine(trigger), Source/Abi, SourceSpan/…}]` |
/// | Target found; entry trigger absent from index | `Exact` | `Complete` | `[{AbiSymbol, Opaque, AbiSymbol}]` |
///
/// # Opaque-vs-Unknown choice (Phase 2 note)
///
/// When the target is not found in the graph or the entry trigger is not indexed,
/// we use `Evidence::Opaque` with `RouteTarget::AbiSymbol` because we know the
/// target *exists* somewhere — it just isn't in our source snapshot.
/// The `classify_obligation` metric counts a route with `AbiSymbol` target and
/// `Opaque` evidence as `Resolved` (not `Unknown`) because the symbol boundary
/// is known and retains its identity; this aligns with L3's External classification.
pub fn resolve_object_run(
    from: AppRef,
    object_kind: ObjectKind,
    target_ref: Option<&str>,
    target_is_name: bool,
    graph: &ProgramGraph,
    index: &ResolveIndex,
    body_map: &BodyMap<'_>,
) -> (DispatchShape, SetCompleteness, Vec<Route>) {
    let Some(target_ref) = target_ref else {
        // Dynamic target (a runtime variable) — known shape, open world.
        return (
            DispatchShape::DynamicOpen,
            SetCompleteness::Partial {
                reason: OpenWorldReason::RuntimeTypeUnbounded,
            },
            vec![Route {
                target: RouteTarget::Unresolved,
                evidence: Evidence::Unknown,
                conditions: vec![],
                witness: Witness::None,
            }],
        );
    };

    // Resolve the target object.
    let target_obj: Option<&ObjectNode> = if target_is_name {
        graph.resolve_object(from, object_kind, target_ref)
    } else {
        match target_ref.parse::<i64>() {
            Ok(n) => index
                .object_by_number(graph, from, object_kind, n)
                .and_then(|oid| graph.objects.iter().find(|o| o.id == oid)),
            Err(_) => None,
        }
    };

    let Some(target_obj) = target_obj else {
        // Target is named/numbered but absent from the entire graph (not in
        // workspace source, not in any dep's SymbolReference). We do NOT know
        // which app owns it, so creating an AbiSymbol with `app = from`
        // (caller's app) would be semantically wrong and would fail the
        // ABI ingestion integrity check. Emit Unknown/Unresolved: honest
        // failure — we cannot name the callee.
        return (
            DispatchShape::Exact,
            SetCompleteness::Complete,
            vec![Route {
                target: RouteTarget::Unresolved,
                evidence: Evidence::Unknown,
                conditions: vec![],
                witness: Witness::None,
            }],
        );
    };

    // Look up the entry trigger by kind-specific name.
    let trigger_name = entry_trigger_name(object_kind);
    let candidates = index.routines_in_object(&target_obj.id, trigger_name);

    // Object-level triggers have `enclosing_member_lc == None`.
    let entry_rid = candidates
        .iter()
        .find(|r| r.enclosing_member_lc.is_none())
        .or_else(|| candidates.first());

    let Some(entry_rid) = entry_rid else {
        // Trigger not found in index — Opaque (e.g. an object with no explicit trigger).
        let (obj_num, obj_name_lc) = match &target_obj.id.key {
            ObjKey::Id(n) => (*n, String::new()),
            ObjKey::Name(s) => (0i64, s.clone()),
        };
        let key = AbiRoutineKey {
            app: target_obj.id.app,
            object_type: format!("{:?}", target_obj.id.kind).to_ascii_lowercase(),
            object_number: obj_num,
            object_name_lc: obj_name_lc,
            routine_name_lc: trigger_name.to_string(),
            params_count: 0,
            param_type_fp: 0,
            routine_kind: AbiRoutineKind::Procedure,
            event_kind: AbiEventKind::None,
        };
        return (
            DispatchShape::Exact,
            SetCompleteness::Complete,
            vec![opaque_boundary_route(key)],
        );
    };

    let route = make_routine_route(entry_rid, target_obj.tier, body_map, graph);
    (DispatchShape::Exact, SetCompleteness::Complete, vec![route])
}

// ---------------------------------------------------------------------------
// Implicit-trigger resolution (data-is-control-flow edges)
// ---------------------------------------------------------------------------

/// Resolve an implicit record-operation trigger fan-out.
///
/// Maps the AL record operation name to the corresponding object/field trigger
/// and collects routes from both the base table and all `TableExtension`s visible
/// in the whole-program snapshot.
///
/// # Trigger mapping
///
/// | `op`        | Trigger         |
/// |-------------|-----------------|
/// | `"insert"`  | `"oninsert"`    |
/// | `"modify"`  | `"onmodify"`    |
/// | `"delete"`  | `"ondelete"`    |
/// | `"validate"`| `"onvalidate"`  |
/// | `"rename"`  | `"onrename"`    |
///
/// # Validate-field approximation (Phase 2)
///
/// `Rec.Validate(FieldName)` targets `OnValidate` on **one specific field**,
/// but the field argument may not be captured in the extraction layer.  For
/// Phase 2 this function fans out to **all** `onvalidate` triggers on the
/// table and its extensions (one route per `(object, field)` pair), using
/// [`DispatchShape::Multicast`] with
/// `SetCompleteness::Partial{ReverseDependentExtensions}`.  The Task-6
/// differential gate measures the residual versus L3's per-field resolution.
///
/// For `insert/modify/delete/rename` (object-level triggers,
/// `enclosing_member_lc == None`) the fan-out is clean.
pub fn resolve_implicit_trigger(
    op: &str,
    table_object: &ObjectNode,
    graph: &ProgramGraph,
    index: &ResolveIndex,
    body_map: &BodyMap<'_>,
) -> (DispatchShape, SetCompleteness, Vec<Route>) {
    let trigger_name: &str = match op.to_ascii_lowercase().as_str() {
        "insert" => "oninsert",
        "modify" => "onmodify",
        "delete" => "ondelete",
        "validate" => "onvalidate",
        "rename" => "onrename",
        _ => {
            // Unrecognised op: honest empty Multicast.
            return (
                DispatchShape::Multicast,
                SetCompleteness::Partial {
                    reason: OpenWorldReason::ReverseDependentExtensions,
                },
                vec![],
            );
        }
    };

    let mut routes: Vec<Route> = Vec::new();

    // Triggers on the base table itself.
    for rid in index.routines_in_object(&table_object.id, trigger_name) {
        routes.push(make_routine_route(rid, table_object.tier, body_map, graph));
    }

    // Triggers on every TableExtension of this table (reverse-dep; whole-snapshot).
    let table_name_lc = table_object.name.to_ascii_lowercase();
    for ext_id in index.table_extensions_of(&table_name_lc) {
        let ext_tier = graph
            .objects
            .iter()
            .find(|o| &o.id == ext_id)
            .map(|o| o.tier)
            .unwrap_or(TrustTier::Workspace);
        for rid in index.routines_in_object(ext_id, trigger_name) {
            routes.push(make_routine_route(rid, ext_tier, body_map, graph));
        }
    }

    (
        DispatchShape::Multicast,
        SetCompleteness::Partial {
            reason: OpenWorldReason::ReverseDependentExtensions,
        },
        routes,
    )
}

// ---------------------------------------------------------------------------
// Member-call resolution (Phase 3 Task 2)
// ---------------------------------------------------------------------------

/// Build a `(Exact, [Catalog route])` outcome for a recognized member builtin.
fn member_catalog_route(bid: BuiltinId) -> (DispatchShape, Vec<Route>) {
    (
        DispatchShape::Exact,
        vec![Route {
            target: RouteTarget::Builtin(bid.clone()),
            evidence: Evidence::Catalog,
            conditions: vec![],
            witness: Witness::CatalogEntry {
                id: bid,
                catalog_version: catalog_version().to_string(),
            },
        }],
    )
}

/// Build a `(Exact, [Unknown route])` outcome.
fn member_unknown_route() -> (DispatchShape, Vec<Route>) {
    (
        DispatchShape::Exact,
        vec![Route {
            target: RouteTarget::Unresolved,
            evidence: Evidence::Unknown,
            conditions: vec![],
            witness: Witness::None,
        }],
    )
}

/// Build a `(DynamicOpen, [Unknown blocker])` outcome for Dynamic receivers.
fn member_dynamic_open_route() -> (DispatchShape, Vec<Route>) {
    (
        DispatchShape::DynamicOpen,
        vec![Route {
            target: RouteTarget::Unresolved,
            evidence: Evidence::Unknown,
            conditions: vec![],
            witness: Witness::None,
        }],
    )
}

/// Map an object kind to its instance-builtin [`FrameworkKind`] catalog, if any.
///
/// Returns `None` for kinds that have no instance-builtin catalog — their member
/// methods are source-declared procedures, not platform-intrinsic instance builtins.
fn object_instance_framework_kind(kind: ObjectKind) -> Option<FrameworkKind> {
    match kind {
        ObjectKind::Page => Some(FrameworkKind::PageInstance),
        ObjectKind::Report => Some(FrameworkKind::ReportInstance),
        _ => None,
    }
}

/// Returns `true` when `method_lc` is an object-metadata-sensitive method for the
/// given object `kind`.
///
/// Metadata-sensitive methods are present in the instance-builtin catalog but their
/// argument or return type depends on the specific object's source table (e.g.
/// `Page.SetRecord` takes a `Record <SourceTable>` argument, not a generic record).
/// These are EXCLUDED from the Catalog fast-path in Phase 4 Task 1 and remain
/// `Unknown` until per-object source-table constraint modelling is in place.
///
/// Exclusion list:
/// - Page: `setrecord`, `settableview`, `setselectionfilter`, `getrecord`, `saverecord`
/// - Report: `settableview`
fn is_metadata_sensitive_instance_method(kind: ObjectKind, method_lc: &str) -> bool {
    match kind {
        ObjectKind::Page => matches!(
            method_lc,
            "setrecord" | "settableview" | "setselectionfilter" | "getrecord" | "saverecord"
        ),
        ObjectKind::Report => matches!(method_lc, "settableview"),
        _ => false,
    }
}

/// Resolve a member call (`receiver.method_lc(...)`) to its `(DispatchShape, Vec<Route>)`.
///
/// # Implemented arms (Phase 3 Task 2 + Task 3 + Phase 4 Task 2; precedence
/// fixed beyond-1B.3b Task 1)
/// - `RecordRef` / `FieldRef` / `KeyRef` / `Framework(_)` → catalog lookup (these
///   receivers have no source-declared procedures, so there is nothing to shadow).
/// - `Record{..}` → **source-before-catalog**: a visible source/ABI table method
///   (base table ∪ its TableExtensions) of matching name+arity SHADOWS a
///   same-named platform-intrinsic Record method (AL semantics). Cardinality:
///   exactly one visible candidate → `Source`/`Abi`/`Opaque`; more than one →
///   honest ambiguous `Unknown` (never pick-first, never fall through to the
///   catalog); zero candidates (or `table == None`) → consult the Record
///   builtin catalog; no catalog hit either → `Unknown`.
/// - `Object{kind, name_lc}` → resolve target object via `graph.resolve_object`, then
///   dispatch method via `resolve_in_object`.  Special case: `Codeunit.Run(arity≤1)` →
///   entry `OnRun` trigger (mirrors `resolve_object_run`).
/// - `SelfObject` → `resolve_in_object` on the calling object itself.
/// - `Interface{name_lc}` → `Polymorphic` fan-out to all known implementers via
///   `index.implementers_of`.  For each implementer: Source-tier → unique-arity-matched
///   `Routine` route, or `Unresolved` on name-absent / arity-mismatch / ambiguous
///   (Rule 1/2 — no reachability black hole, no guessed route).  SymbolOnly-tier (cross-app
///   `.app` dep) → `AbiSymbol` (Opaque boundary) via `resolve_in_object`.
///
/// # Deferred arms (TODO markers only)
/// - `Primitive` → Unknown; `Dynamic` → `DynamicOpen`; `Unknown` → Unknown.
pub fn resolve_member(
    receiver: &ReceiverType,
    method_lc: &str,
    arity: usize,
    from_object: &ObjectNode,
    graph: &ProgramGraph,
    index: &ResolveIndex,
    body_map: &BodyMap<'_>,
) -> (DispatchShape, Vec<Route>) {
    match receiver {
        ReceiverType::RecordRef => {
            // Catalog-only by construction (Item 4 — see the `Record` arm's
            // comment below): `RecordRef` carries no table identity (unlike
            // `Record { table }`), so there is no source candidate to shadow
            // the catalog and no collision guard is needed OR possible here.
            if let Some(bid) = member_builtin_id(MemberCatalogKind::RecordRef, method_lc) {
                member_catalog_route(bid)
            } else {
                member_unknown_route()
            }
        }
        ReceiverType::FieldRef => {
            if let Some(bid) = member_builtin_id(MemberCatalogKind::FieldRef, method_lc) {
                member_catalog_route(bid)
            } else {
                member_unknown_route()
            }
        }
        ReceiverType::KeyRef => {
            if let Some(bid) = member_builtin_id(MemberCatalogKind::KeyRef, method_lc) {
                member_catalog_route(bid)
            } else {
                member_unknown_route()
            }
        }
        ReceiverType::Framework(kind) => {
            if let Some(bid) = member_builtin_id(MemberCatalogKind::Framework(kind), method_lc) {
                member_catalog_route(bid)
            } else {
                member_unknown_route()
            }
        }
        ReceiverType::Record { table } => {
            // Source-before-catalog (beyond-1B.3b Task 1 / Item 4 — see
            // `is_bare_builtin_or_page_intrinsic`'s doc for the mirror-image
            // rationale): AL semantics say a visible source/ABI table method
            // of matching name+arity SHADOWS a same-named platform-intrinsic
            // Record method (Catalog), and this arm is DELIBERATELY NOT
            // collision-guarded the way Step 3's bare-call probe is —
            // same-receiver Source-shadows-Catalog is corpus-validated
            // correct AL precedence (42 real CDO `builtin-catalog-fp-
            // collision` instances; see
            // `resolve_member_record_source_proc_shadows_same_named_builtin`).
            // Adding a collision guard here would regress that fix back into
            // a false `Unknown` — do not "fix" this. Gather every visible
            // source/ABI candidate across the base table AND its
            // TableExtensions FIRST — visibility-scoped to `from_object`'s
            // app dependency closure, with `Local`/`Internal`/`Protected`
            // candidates excluded per `from_object`'s CALLER IDENTITY
            // (beyond-1B.3b Task 1, caller-identity-aware; see
            // `object_has_visible_member_candidate`) — only consult the
            // catalog when that scope has ZERO candidates (or the table
            // itself is unresolved/not visible — builtins still resolve
            // table-independently in that case).
            //
            // Cardinality (gpt round-2): exactly one candidate object → resolve
            // it (Source/Abi/Opaque); more than one → honest ambiguous Unknown
            // — source ambiguity STILL shadows the catalog, never fall through
            // to a false intrinsic; zero → consult the catalog.
            if let Some(table_id) = table
                && let Some((shape, routes)) = resolve_in_table_scope(
                    from_object,
                    table_id.clone(),
                    method_lc,
                    arity,
                    graph,
                    index,
                    body_map,
                )
            {
                return (shape, routes);
            }

            // Zero visible source/ABI candidates in scope (or `table`
            // unresolved/not visible): Record built-in methods (SetRange,
            // Find, Insert, ...) are platform-intrinsic and resolve
            // table-independently.
            if let Some(bid) = member_builtin_id(MemberCatalogKind::Record, method_lc) {
                return member_catalog_route(bid);
            }
            member_unknown_route()
        }
        ReceiverType::Object { kind, name_lc, id } => {
            // Resolve the target object (topology-scoped from the calling app).
            // Task 7: when Phase A already carried a resolved id MECHANICALLY
            // (Step 0's `CurrPage.<part>.Page` subpage receiver, proven via
            // the fail-closed `ResolveIndex::resolve_object_ref`), short-
            // circuit on it directly rather than re-resolving by name — a
            // second by-name lookup could in principle disagree with the id
            // Phase A already verified unique (e.g. a same-named object
            // resolving differently), silently substituting the WRONG
            // subpage for the one the control's `target` actually names.
            let target = match id {
                Some(id) => graph.objects.iter().find(|o| &o.id == id),
                None => graph.resolve_object(from_object.id.app, *kind, name_lc),
            };
            let Some(target) = target else {
                // Target not in the graph — honest Unknown (not Opaque: we have no
                // identity for an unresolvable typed receiver).
                return member_unknown_route();
            };
            let target_id = target.id.clone();
            let target_tier = target.tier;

            // Codeunit.Run(arity≤1) special case: dispatch to the OnRun entry
            // trigger, mirroring `resolve_object_run`'s entry-trigger semantics.
            if *kind == ObjectKind::Codeunit && method_lc == "run" && arity <= 1 {
                let candidates = index.routines_in_object(&target_id, "onrun");
                // Object-level triggers have `enclosing_member_lc == None`.
                let entry_rid = candidates
                    .iter()
                    .find(|r| r.enclosing_member_lc.is_none())
                    .or_else(|| candidates.first());
                return if let Some(entry_rid) = entry_rid {
                    (
                        DispatchShape::Exact,
                        vec![make_routine_route(entry_rid, target_tier, body_map, graph)],
                    )
                } else {
                    // OnRun not indexed — Opaque boundary (object exists, trigger absent).
                    let (obj_num, obj_name_lc) = match &target_id.key {
                        ObjKey::Id(n) => (*n, String::new()),
                        ObjKey::Name(s) => (0i64, s.clone()),
                    };
                    let key = AbiRoutineKey {
                        app: target_id.app,
                        object_type: format!("{:?}", target_id.kind).to_ascii_lowercase(),
                        object_number: obj_num,
                        object_name_lc: obj_name_lc,
                        routine_name_lc: "onrun".to_string(),
                        params_count: 0,
                        param_type_fp: 0,
                        routine_kind: AbiRoutineKind::Procedure,
                        event_kind: AbiEventKind::None,
                    };
                    (DispatchShape::Exact, vec![opaque_boundary_route(key)])
                };
            }

            // General dispatch: resolve the method among the target object's procedures.
            if let Some(route) = resolve_in_object(
                &target_id,
                target_tier,
                method_lc,
                arity,
                graph,
                index,
                body_map,
            ) {
                (DispatchShape::Exact, vec![route])
            } else {
                // Method name absent from target object's declared procedures.
                // Fall through to the instance-builtin catalog for kinds that have one
                // (Page→PageInstance, Report→ReportInstance), EXCLUDING metadata-sensitive
                // methods whose argument/return types depend on the object's source table
                // (SetRecord / SetTableView-class).
                if !is_metadata_sensitive_instance_method(*kind, method_lc)
                    && let Some(fk) = object_instance_framework_kind(*kind)
                    && let Some(bid) =
                        member_builtin_id(MemberCatalogKind::Framework(&fk), method_lc)
                {
                    return member_catalog_route(bid);
                }
                member_unknown_route()
            }
        }
        ReceiverType::SelfObject => {
            // Dispatch to the calling object's own declared procedures.
            if let Some(route) = resolve_in_object(
                &from_object.id,
                from_object.tier,
                method_lc,
                arity,
                graph,
                index,
                body_map,
            ) {
                (DispatchShape::Exact, vec![route])
            } else {
                // Method not found in own object.
                member_unknown_route()
            }
        }
        ReceiverType::Interface { name_lc } => {
            // Phase 4 Task 2: fan out to all known implementers.
            //
            // For each implementer:
            //   SymbolOnly tier  → `params_count` is populated from the ABI (Task 1),
            //                      but arity matching is deferred; delegate directly to
            //                      `resolve_in_object` which returns AbiSymbol or Unknown.
            //   Source tier      → count arity-matched overloads first:
            //                        0 candidates → Rule 1 Unresolved (method absent/arity mismatch)
            //                        1 candidate  → unique resolution via `resolve_in_object`
            //                        >1 candidates→ Rule 2 Unresolved (ambiguous, never guess)
            //
            // A known implementer that FAILS resolution MUST emit
            // `Route{Unresolved, Unknown}` and must NOT be dropped — silently
            // dropping it would create a reachability black hole where a
            // runtime-reachable target is invisible in the call graph.
            let implementers = index.implementers_of(name_lc);
            let mut routes: Vec<Route> = Vec::with_capacity(implementers.len());

            for impl_id in implementers {
                let impl_tier = graph
                    .objects
                    .iter()
                    .find(|o| &o.id == impl_id)
                    .map(|o| o.tier)
                    .unwrap_or(TrustTier::Workspace);

                if impl_tier == TrustTier::SymbolOnly {
                    // SymbolOnly: arity matching deferred; delegate.
                    let route = resolve_in_object(
                        impl_id, impl_tier, method_lc, arity, graph, index, body_map,
                    )
                    .unwrap_or(Route {
                        target: RouteTarget::Unresolved,
                        evidence: Evidence::Unknown,
                        conditions: vec![],
                        witness: Witness::None,
                    });
                    routes.push(route);
                } else {
                    let candidates = index.routines_in_object(impl_id, method_lc);
                    if candidates.is_empty() {
                        // Method name absent from this implementer — Rule 1 Unresolved.
                        routes.push(Route {
                            target: RouteTarget::Unresolved,
                            evidence: Evidence::Unknown,
                            conditions: vec![],
                            witness: Witness::None,
                        });
                    } else {
                        let matching = candidates
                            .iter()
                            .filter(|r| r.params_count == arity)
                            .count();
                        match matching {
                            1 => {
                                // Unique arity-matched overload: guaranteed to resolve.
                                let route = resolve_in_object(
                                    impl_id, impl_tier, method_lc, arity, graph, index, body_map,
                                )
                                .unwrap_or(Route {
                                    target: RouteTarget::Unresolved,
                                    evidence: Evidence::Unknown,
                                    conditions: vec![],
                                    witness: Witness::None,
                                });
                                routes.push(route);
                            }
                            _ => {
                                // 0 (arity mismatch) or >1 (ambiguous) — Rule 1+2 Unresolved.
                                // Never emit a guessed route to a wrong-arity or wrong-overload target.
                                routes.push(Route {
                                    target: RouteTarget::Unresolved,
                                    evidence: Evidence::Unknown,
                                    conditions: vec![],
                                    witness: Witness::None,
                                });
                            }
                        }
                    }
                }
            }

            (DispatchShape::Polymorphic, routes)
        }
        ReceiverType::EnumType { .. } => {
            // Enum instance statics: AsInteger / FromInteger / Names / Ordinals.
            if let Some(bid) = member_builtin_id(
                MemberCatalogKind::Framework(&FrameworkKind::Enum),
                method_lc,
            ) {
                member_catalog_route(bid)
            } else {
                member_unknown_route()
            }
        }
        ReceiverType::Primitive => {
            // Non-catalog type — honest Unknown (not a false resolution gap).
            member_unknown_route()
        }
        ReceiverType::Dynamic => {
            // Variant-typed receiver — genuinely dynamic, not a resolution hole.
            member_dynamic_open_route()
        }
        ReceiverType::Unknown => member_unknown_route(),
    }
}

// ---------------------------------------------------------------------------
// Publisher-anchored EventFlow edge emission (Phase 4b Task 3)
// ---------------------------------------------------------------------------

/// Emit one `EventFlow` `Multicast` edge per publisher event routine, with
/// routes to all its resolved subscribers (from [`ResolveIndex::subscribers_of`]).
///
/// # Edge contract
///
/// - `from` = the publisher `RoutineNodeId`.
/// - `site` = `SiteId` anchored at the publisher routine's name-origin span
///   (from the `body_map`; synthetic zero-span when the publisher is not in the
///   `body_map`, e.g. a SymbolOnly dep or an integration gap).
/// - `kind` = `EdgeKind::EventFlow`.
/// - `shape` = `DispatchShape::Multicast`.
/// - `completeness` = `SetCompleteness::Partial { ReverseDependentSubscribers }`.
/// - `routes` = one `Route` per subscriber entry, carrying its dispatch
///   `conditions` and the subscriber's source span as `Witness::SourceSpan`.
///   For SymbolOnly subscribers: `RouteTarget::AbiSymbol` + `Evidence::Opaque`
///   (mirrors [`make_routine_route`]'s SymbolOnly path).
///
/// A publisher with **zero** subscribers emits an edge with **empty routes** —
/// this is an honest "published, no subscribers in snapshot" state, classified
/// as `ObligationOutcome::HonestEmpty` by `classify_obligation`.
///
/// # Determinism
/// Publishers are iterated in `graph.routines` order (already sorted by
/// `RoutineNodeId`); subscriber routes within each edge are already sorted by
/// subscriber `RoutineNodeId` by [`ResolveIndex::build`].
pub fn emit_event_flow_edges(
    graph: &ProgramGraph,
    index: &ResolveIndex,
    body_map: &BodyMap<'_>,
) -> Vec<Edge> {
    let mut edges = Vec::new();

    for pub_routine in &graph.routines {
        if pub_routine.publisher_kind.is_none() {
            continue;
        }

        let subs = index.subscribers_of(&pub_routine.id);

        // Build one Route per subscriber (sorted by subscriber RoutineNodeId — already
        // guaranteed by ResolveIndex::build).
        let routes: Vec<Route> = subs
            .iter()
            .map(|se| {
                // Subscriber tier: look up from graph.routines (linear scan; subscriber
                // lists are small in practice).
                let sub_tier = graph
                    .routines
                    .iter()
                    .find(|r| r.id == se.subscriber)
                    .map(|r| r.tier)
                    .unwrap_or(TrustTier::Workspace);

                // Build base route using make_routine_route (handles Source/Opaque/Unknown
                // tiers via body_map), then inject the subscriber's conditions.
                let mut route = make_routine_route(&se.subscriber, sub_tier, body_map, graph);
                route.conditions = se.conditions.clone();
                route
            })
            .collect();

        // SiteId: anchored at the publisher routine's name-origin span.
        let site = if let Some((decl, path)) = body_map.get_with_path(&pub_routine.id) {
            SiteId {
                caller: pub_routine.id.clone(),
                span: CanonicalSpan {
                    unit: path.to_string(),
                    start: SourcePos {
                        line: decl.name_origin.start.row,
                        col: decl.name_origin.start.column,
                    },
                    end: SourcePos {
                        line: decl.name_origin.end.row,
                        col: decl.name_origin.end.column,
                    },
                },
                callee_fingerprint: callee_fp(&pub_routine.id.name_lc),
            }
        } else {
            // Publisher not in body_map (SymbolOnly dep or integration gap):
            // use a synthetic zero-span site.
            SiteId {
                caller: pub_routine.id.clone(),
                span: CanonicalSpan {
                    unit: String::new(),
                    start: SourcePos { line: 0, col: 0 },
                    end: SourcePos { line: 0, col: 0 },
                },
                callee_fingerprint: callee_fp(&pub_routine.id.name_lc),
            }
        };

        edges.push(Edge {
            from: pub_routine.id.clone(),
            site,
            kind: EdgeKind::EventFlow,
            shape: DispatchShape::Multicast,
            completeness: SetCompleteness::Partial {
                reason: OpenWorldReason::ReverseDependentSubscribers,
            },
            routes,
        });
    }

    edges
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    use crate::program::graph::{ObjectIndex, ProgramGraph};
    use crate::program::node::AppRegistry;
    use crate::program::node_extract::{Access, ObjectNode, RoutineNode, extract_nodes};
    use crate::program::resolve::body_map::BodyMap;
    use crate::program::resolve::edge::{
        Condition, DispatchShape, Edge, EdgeKind, Evidence, ObligationOutcome, OpenWorldReason,
        RouteTarget, SetCompleteness, Witness, classify_obligation,
    };
    use crate::program::resolve::index::ResolveIndex;
    use crate::program::topology::DependencyGraph;
    use crate::snapshot::{AppId, ParsedFile, ParsedUnit, Provenance, TrustTier};
    use al_syntax::ir::ObjectKind;

    // -----------------------------------------------------------------------
    // Test helpers
    // -----------------------------------------------------------------------

    fn make_app_id(name: &str) -> AppId {
        AppId {
            guid: String::new(),
            name: name.into(),
            publisher: "Test".into(),
            version: "1.0.0.0".into(),
        }
    }

    /// Parse AL source into a `ParsedUnit` with the given virtual path.
    fn make_unit(app_id: AppId, virtual_path: &str, src: &'static str) -> ParsedUnit {
        let provenance = Provenance {
            app: app_id.clone(),
            tier: TrustTier::Workspace,
            content_hash: String::new(),
        };
        ParsedUnit {
            app: app_id,
            files: vec![ParsedFile {
                virtual_path: virtual_path.to_string(),
                file: al_syntax::parse(src),
                provenance,
                text: src.to_string(),
            }],
        }
    }

    /// Build a `ProgramGraph` from one or more `ParsedUnit`s and an optional
    /// dependency edge `(from_app_name, to_app_name)`.
    ///
    /// All units must be fully registered in the graph before returning.
    fn build_graph(units: &[ParsedUnit], dep_edge: Option<(&str, &str)>) -> ProgramGraph {
        let mut apps = AppRegistry::default();
        let mut objects: Vec<ObjectNode> = Vec::new();
        let mut routines: Vec<RoutineNode> = Vec::new();

        for unit in units {
            let app_ref = apps.intern(&unit.app);
            for pf in &unit.files {
                extract_nodes(
                    app_ref,
                    &pf.file,
                    pf.provenance.tier,
                    &mut objects,
                    &mut routines,
                );
            }
        }

        objects.sort_by(|a, b| a.id.cmp(&b.id));
        routines.sort_by(|a, b| a.id.cmp(&b.id));

        let mut topology = DependencyGraph::default();
        if let Some((from_name, to_name)) = dep_edge {
            let from_ref = apps.find_by_name(from_name).expect("from app");
            let to_ref = apps.find_by_name(to_name).expect("to app");
            topology.add_dependency(from_ref, to_ref);
        }

        let obj_index = ObjectIndex::build(&objects);
        ProgramGraph {
            apps,
            topology,
            objects,
            routines,
            obj_index,
        }
    }

    /// Build a `ProgramGraph` from one or more `ParsedUnit`s and any number of
    /// dependency edges `(from_app_name, to_app_name)` — the multi-edge
    /// counterpart to [`build_graph`], needed for Task 2's cross-app
    /// visibility-scoping tests (e.g. App A depends on App B but NOT App C).
    fn build_graph_multi_dep(units: &[ParsedUnit], deps: &[(&str, &str)]) -> ProgramGraph {
        let mut apps = AppRegistry::default();
        let mut objects: Vec<ObjectNode> = Vec::new();
        let mut routines: Vec<RoutineNode> = Vec::new();

        for unit in units {
            let app_ref = apps.intern(&unit.app);
            for pf in &unit.files {
                extract_nodes(
                    app_ref,
                    &pf.file,
                    pf.provenance.tier,
                    &mut objects,
                    &mut routines,
                );
            }
        }

        objects.sort_by(|a, b| a.id.cmp(&b.id));
        routines.sort_by(|a, b| a.id.cmp(&b.id));

        let mut topology = DependencyGraph::default();
        for (from_name, to_name) in deps {
            let from_ref = apps.find_by_name(from_name).expect("from app");
            let to_ref = apps.find_by_name(to_name).expect("to app");
            topology.add_dependency(from_ref, to_ref);
        }

        let obj_index = ObjectIndex::build(&objects);
        ProgramGraph {
            apps,
            topology,
            objects,
            routines,
            obj_index,
        }
    }

    /// Find the `ObjectNode` with the given lowercase name in `graph.objects`.
    fn find_obj<'g>(graph: &'g ProgramGraph, name_lc: &str) -> &'g ObjectNode {
        graph
            .objects
            .iter()
            .find(|o| o.name.eq_ignore_ascii_case(name_lc))
            .unwrap_or_else(|| panic!("object {name_lc} not found in graph"))
    }

    // -----------------------------------------------------------------------
    // (a) bare call to an own-object procedure → Source evidence + SourceSpan
    // -----------------------------------------------------------------------

    #[test]
    fn bare_own_object_resolves_to_source_route() {
        let src: &'static str = r#"
codeunit 50100 "MyCU"
{
    procedure DoFoo()
    begin
    end;
}
"#;
        let app_id = make_app_id("TestApp");
        let unit = make_unit(app_id, "MyCU.al", src);
        let units = [unit];
        let graph = build_graph(&units, None);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let from_obj = find_obj(&graph, "MyCU");
        let routes = resolve_bare(
            from_obj,
            "dofoo",
            0,
            &graph,
            &index,
            &body_map,
            WithState::NoWithProven,
        );

        assert_eq!(routes.len(), 1, "expected exactly one route");
        let r = &routes[0];
        assert!(
            matches!(r.target, RouteTarget::Routine(_)),
            "target must be Routine"
        );
        assert_eq!(r.evidence, Evidence::Source, "evidence must be Source");
        assert!(
            matches!(r.witness, Witness::SourceSpan { .. }),
            "witness must be SourceSpan; got {:?}",
            r.witness
        );
        // The file coordinate must point to the virtual path we supplied.
        let Witness::SourceSpan { ref file, span } = r.witness else {
            unreachable!()
        };
        assert_eq!(file, "MyCU.al");
        // Span must be a non-empty range.
        assert!(span.1 > span.0, "byte span must be non-empty");
    }

    // -----------------------------------------------------------------------
    // (b) bare `Message` → Builtin evidence + CatalogEntry witness
    // -----------------------------------------------------------------------

    #[test]
    fn bare_global_builtin_message_emits_catalog_route() {
        let src: &'static str = r#"
codeunit 50101 "CallerCU"
{
    procedure Run()
    begin
    end;
}
"#;
        let app_id = make_app_id("TestApp");
        let unit = make_unit(app_id, "CallerCU.al", src);
        let units = [unit];
        let graph = build_graph(&units, None);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let from_obj = find_obj(&graph, "CallerCU");
        // "message" (1 arg) is a recognized global builtin.
        let routes = resolve_bare(
            from_obj,
            "message",
            1,
            &graph,
            &index,
            &body_map,
            WithState::NoWithProven,
        );

        assert_eq!(routes.len(), 1);
        let r = &routes[0];
        assert!(
            matches!(r.target, RouteTarget::Builtin(_)),
            "target must be Builtin"
        );
        assert_eq!(r.evidence, Evidence::Catalog);
        assert!(
            matches!(r.witness, Witness::CatalogEntry { .. }),
            "witness must be CatalogEntry"
        );
        // Builtin id must be the lowercased name.
        let RouteTarget::Builtin(ref bid) = r.target else {
            unreachable!()
        };
        assert_eq!(bid.0, "message");
        // CatalogEntry id must match.
        let Witness::CatalogEntry {
            ref id,
            ref catalog_version,
        } = r.witness
        else {
            unreachable!()
        };
        assert_eq!(id.0, "message");
        assert!(!catalog_version.is_empty());
    }

    // -----------------------------------------------------------------------
    // (c) bare nonexistent non-builtin → Unknown evidence + None witness
    // -----------------------------------------------------------------------

    #[test]
    fn bare_nonexistent_non_builtin_emits_unknown_route() {
        let src: &'static str = r#"
codeunit 50102 "AnotherCU"
{
    procedure Caller()
    begin
    end;
}
"#;
        let app_id = make_app_id("TestApp");
        let unit = make_unit(app_id, "AnotherCU.al", src);
        let units = [unit];
        let graph = build_graph(&units, None);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let from_obj = find_obj(&graph, "AnotherCU");
        let routes = resolve_bare(
            from_obj,
            "xyz_this_does_not_exist_at_all",
            0,
            &graph,
            &index,
            &body_map,
            WithState::NoWithProven,
        );

        assert_eq!(routes.len(), 1);
        let r = &routes[0];
        assert_eq!(r.target, RouteTarget::Unresolved);
        assert_eq!(r.evidence, Evidence::Unknown);
        assert_eq!(r.witness, Witness::None);
    }

    // -----------------------------------------------------------------------
    // (d) extension calling a base-object proc → resolved via the base
    // -----------------------------------------------------------------------

    #[test]
    fn bare_extension_base_object_proc_is_resolved() {
        // App A: has TableExtension 50100 "CustomerExt" extending "Customer".
        // App B: has Table 50000 "Customer" with procedure "Init".
        // App A depends on App B.
        // resolve_bare from CustomerExt for "init" → resolves to Table Customer's Init.
        let src_a: &'static str = r#"
tableextension 50100 "CustomerExt" extends Customer
{
    procedure ExtProc()
    begin
    end;
}
"#;
        let src_b: &'static str = r#"
table 50000 Customer
{
    procedure Init()
    begin
    end;
}
"#;
        let app_a_id = make_app_id("AppA");
        let app_b_id = make_app_id("AppB");

        let unit_a = make_unit(app_a_id.clone(), "CustomerExt.al", src_a);
        let unit_b = make_unit(app_b_id, "Customer.al", src_b);
        let units = [unit_a, unit_b];

        let graph = build_graph(&units, Some(("AppA", "AppB")));
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        // from_object = the TableExtension in App A.
        let from_obj = find_obj(&graph, "CustomerExt");
        assert_eq!(from_obj.id.kind, ObjectKind::TableExtension);
        assert_eq!(
            from_obj.extends_target.as_deref(),
            Some("Customer"),
            "extends_target must be populated from IR"
        );

        // "init" is not in the extension itself — only in the base Table.
        let routes = resolve_bare(
            from_obj,
            "init",
            0,
            &graph,
            &index,
            &body_map,
            WithState::NoWithProven,
        );

        assert_eq!(routes.len(), 1, "must resolve to exactly one route");
        let r = &routes[0];
        assert!(
            matches!(r.target, RouteTarget::Routine(_)),
            "target must be Routine (not Builtin/Unresolved)"
        );
        assert_eq!(r.evidence, Evidence::Source);
        assert!(
            matches!(r.witness, Witness::SourceSpan { .. }),
            "witness must be SourceSpan"
        );

        // Sanity: target routine is in the base table (App B), not the extension.
        let RouteTarget::Routine(ref rid) = r.target else {
            unreachable!()
        };
        let app_b_ref = graph.app_ref_by_name("AppB");
        assert_eq!(
            rid.object.app, app_b_ref,
            "resolved routine must live in AppB (the base table's app)"
        );
        assert_eq!(rid.name_lc, "init");
    }

    // -----------------------------------------------------------------------
    // Task 1.5 — resolve_bare Step 2 ("extension base") access filtering.
    //
    // Step 2 resolves a bare call against the caller's extended BASE object
    // via `resolve_in_object`, which does ZERO access filtering (unlike
    // `resolve_in_table_scope`, Task 1's caller-identity-aware path). Pre-fix,
    // ANY base member — regardless of declared `Access` — false-resolved to
    // `Source`. These fixtures pin the exact pre-fix wrong route (Source to
    // the inaccessible base member) and the post-fix honest `Unknown`, plus
    // two controls proving Step 2 still works for `Public`/`Protected` (the
    // extension trivially extends its own base, so `Protected` is visible).
    // -----------------------------------------------------------------------

    // (a) TableExtension `ExtA extends Base` bare-calls a `local procedure
    // L()` declared on `Base`. AL `local` is OBJECT-scoped — visible only to
    // `Base` itself, never to ANY of its extensions (even a direct one) — so
    // this must decline to `Unknown`. Pre-fix: false `Source` to `Base.L`.
    #[test]
    fn bare_extension_base_local_method_excluded() {
        let src_base: &'static str = r#"
table 52900 "Base"
{
    local procedure L()
    begin
    end;
}
"#;
        let src_ext: &'static str = r#"
tableextension 52901 "ExtA" extends Base
{
}
"#;
        let app_id = make_app_id("TestApp");
        let unit_base = make_unit(app_id.clone(), "Base.al", src_base);
        let unit_ext = make_unit(app_id, "ExtA.al", src_ext);
        let units = [unit_base, unit_ext];
        let graph = build_graph(&units, None);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let from_obj = find_obj(&graph, "ExtA");
        assert_eq!(from_obj.id.kind, ObjectKind::TableExtension);

        let routes = resolve_bare(
            from_obj,
            "l",
            0,
            &graph,
            &index,
            &body_map,
            WithState::NoWithProven,
        );

        assert_eq!(routes.len(), 1);
        assert_eq!(
            routes[0].target,
            RouteTarget::Unresolved,
            "a base table's `local` procedure must NOT be visible to a bare \
             call from ANY of its extensions (base-self only); got {:?}",
            routes[0].target
        );
        assert_eq!(routes[0].evidence, Evidence::Unknown);
    }

    // (b) CONTROL: same shape as (a), but `Base` declares a `Public`
    // (default-visibility) `procedure Pub()` — Step 2 must still resolve this
    // to `Source`, unchanged by the access filter.
    #[test]
    fn bare_extension_base_public_method_control_resolves_to_source() {
        let src_base: &'static str = r#"
table 52902 "Base"
{
    procedure Pub()
    begin
    end;
}
"#;
        let src_ext: &'static str = r#"
tableextension 52903 "ExtA" extends Base
{
}
"#;
        let app_id = make_app_id("TestApp");
        let unit_base = make_unit(app_id.clone(), "Base.al", src_base);
        let unit_ext = make_unit(app_id, "ExtA.al", src_ext);
        let units = [unit_base, unit_ext];
        let graph = build_graph(&units, None);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let from_obj = find_obj(&graph, "ExtA");
        let routes = resolve_bare(
            from_obj,
            "pub",
            0,
            &graph,
            &index,
            &body_map,
            WithState::NoWithProven,
        );

        assert_eq!(routes.len(), 1);
        assert!(
            matches!(routes[0].target, RouteTarget::Routine(_)),
            "a base table's `Public` procedure must still resolve via Step \
             2; got {:?}",
            routes[0].target
        );
        assert_eq!(routes[0].evidence, Evidence::Source);
    }

    // (c) cross-app `internal`: `ExtA` (App A) bare-calls an `internal
    // procedure I()` declared on `Base` (App B, a dependency of App A —
    // App A extends a base object it does not own). `internal` is app-scoped;
    // cross-app must decline to `Unknown`. Pre-fix: false `Source` to `Base.I`.
    #[test]
    fn bare_extension_base_cross_app_internal_method_excluded() {
        let src_base: &'static str = r#"
table 52904 "Base"
{
    internal procedure I()
    begin
    end;
}
"#;
        let src_ext: &'static str = r#"
tableextension 52905 "ExtA" extends Base
{
}
"#;
        let app_a = make_app_id("AppA");
        let app_b = make_app_id("AppB");
        let unit_base = make_unit(app_b, "Base.al", src_base);
        let unit_ext = make_unit(app_a, "ExtA.al", src_ext);
        let units = [unit_base, unit_ext];
        let graph = build_graph(&units, Some(("AppA", "AppB")));
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let from_obj = find_obj(&graph, "ExtA");
        let routes = resolve_bare(
            from_obj,
            "i",
            0,
            &graph,
            &index,
            &body_map,
            WithState::NoWithProven,
        );

        assert_eq!(routes.len(), 1);
        assert_eq!(
            routes[0].target,
            RouteTarget::Unresolved,
            "a cross-app `internal` base method must NOT be visible to a \
             bare call from an extension in a DIFFERENT app; got {:?}",
            routes[0].target
        );
        assert_eq!(routes[0].evidence, Evidence::Unknown);
    }

    // (d) CONTROL: `ExtA` bare-calls a `protected procedure P()` on `Base` —
    // the extension DOES see the base's `protected` member (Step 2's caller
    // is by construction a direct, kind-compatible extension of the base, so
    // the self-or-extends check always holds — confirms this incidentally-
    // safe path stays correct after the access filter is added).
    #[test]
    fn bare_extension_base_protected_method_control_resolves_to_source() {
        let src_base: &'static str = r#"
table 52906 "Base"
{
    protected procedure P()
    begin
    end;
}
"#;
        let src_ext: &'static str = r#"
tableextension 52907 "ExtA" extends Base
{
}
"#;
        let app_id = make_app_id("TestApp");
        let unit_base = make_unit(app_id.clone(), "Base.al", src_base);
        let unit_ext = make_unit(app_id, "ExtA.al", src_ext);
        let units = [unit_base, unit_ext];
        let graph = build_graph(&units, None);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let from_obj = find_obj(&graph, "ExtA");
        let routes = resolve_bare(
            from_obj,
            "p",
            0,
            &graph,
            &index,
            &body_map,
            WithState::NoWithProven,
        );

        assert_eq!(routes.len(), 1);
        assert!(
            matches!(routes[0].target, RouteTarget::Routine(_)),
            "a base table's `protected` procedure must be visible to a bare \
             call from its own extension; got {:?}",
            routes[0].target
        );
        assert_eq!(routes[0].evidence, Evidence::Source);
    }

    // (e) PageExtension→base-Page variant of (a): same `local`-excluded rule
    // generalized to a non-Table extension kind.
    #[test]
    fn bare_pageextension_base_local_method_excluded() {
        let src_page: &'static str = r#"
page 52908 "BasePage"
{
    local procedure L()
    begin
    end;
}
"#;
        let src_ext: &'static str = r#"
pageextension 52909 "ExtA" extends BasePage
{
}
"#;
        let app_id = make_app_id("TestApp");
        let unit_page = make_unit(app_id.clone(), "BasePage.al", src_page);
        let unit_ext = make_unit(app_id, "ExtA.al", src_ext);
        let units = [unit_page, unit_ext];
        let graph = build_graph(&units, None);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let from_obj = find_obj(&graph, "ExtA");
        assert_eq!(from_obj.id.kind, ObjectKind::PageExtension);

        let routes = resolve_bare(
            from_obj,
            "l",
            0,
            &graph,
            &index,
            &body_map,
            WithState::NoWithProven,
        );

        assert_eq!(routes.len(), 1);
        assert_eq!(
            routes[0].target,
            RouteTarget::Unresolved,
            "a base Page's `local` procedure must NOT be visible to a bare \
             call from ANY of its PageExtensions; got {:?}",
            routes[0].target
        );
        assert_eq!(routes[0].evidence, Evidence::Unknown);
    }

    // (f) PageExtension→base-Page variant of (b) CONTROL: `Public` still
    // resolves via Step 2.
    #[test]
    fn bare_pageextension_base_public_method_control_resolves_to_source() {
        let src_page: &'static str = r#"
page 52910 "BasePage"
{
    procedure Pub()
    begin
    end;
}
"#;
        let src_ext: &'static str = r#"
pageextension 52911 "ExtA" extends BasePage
{
}
"#;
        let app_id = make_app_id("TestApp");
        let unit_page = make_unit(app_id.clone(), "BasePage.al", src_page);
        let unit_ext = make_unit(app_id, "ExtA.al", src_ext);
        let units = [unit_page, unit_ext];
        let graph = build_graph(&units, None);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let from_obj = find_obj(&graph, "ExtA");
        let routes = resolve_bare(
            from_obj,
            "pub",
            0,
            &graph,
            &index,
            &body_map,
            WithState::NoWithProven,
        );

        assert_eq!(routes.len(), 1);
        assert!(
            matches!(routes[0].target, RouteTarget::Routine(_)),
            "a base Page's `Public` procedure must still resolve via Step 2; \
             got {:?}",
            routes[0].target
        );
        assert_eq!(routes[0].evidence, Evidence::Source);
    }

    // -----------------------------------------------------------------------
    // Witness↔evidence contract test
    // -----------------------------------------------------------------------

    #[test]
    fn witness_evidence_contract_holds_for_all_route_variants() {
        let src: &'static str = r#"
codeunit 50200 "ContractCU"
{
    procedure MyProc()
    begin
    end;
}
"#;
        let app_id = make_app_id("TestApp");
        let unit = make_unit(app_id, "ContractCU.al", src);
        let units = [unit];
        let graph = build_graph(&units, None);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);
        let from_obj = find_obj(&graph, "ContractCU");

        // Source route: resolve to own procedure.
        let src_routes = resolve_bare(
            from_obj,
            "myproc",
            0,
            &graph,
            &index,
            &body_map,
            WithState::NoWithProven,
        );
        assert_eq!(src_routes.len(), 1);
        let src_route = &src_routes[0];
        assert_eq!(src_route.evidence, Evidence::Source, "Source evidence");
        assert!(
            matches!(src_route.witness, Witness::SourceSpan { .. }),
            "Source evidence must pair with SourceSpan witness"
        );

        // Catalog route: global builtin.
        let cat_routes = resolve_bare(
            from_obj,
            "error",
            1,
            &graph,
            &index,
            &body_map,
            WithState::NoWithProven,
        );
        assert_eq!(cat_routes.len(), 1);
        let cat_route = &cat_routes[0];
        assert_eq!(cat_route.evidence, Evidence::Catalog, "Catalog evidence");
        assert!(
            matches!(cat_route.witness, Witness::CatalogEntry { .. }),
            "Catalog evidence must pair with CatalogEntry witness"
        );

        // Unknown route: no match anywhere.
        let unk_routes = resolve_bare(
            from_obj,
            "zz_absolutely_no_match_xyz",
            0,
            &graph,
            &index,
            &body_map,
            WithState::NoWithProven,
        );
        assert_eq!(unk_routes.len(), 1);
        let unk_route = &unk_routes[0];
        assert_eq!(unk_route.evidence, Evidence::Unknown, "Unknown evidence");
        assert_eq!(
            unk_route.witness,
            Witness::None,
            "Unknown evidence must pair with None witness"
        );
    }

    // -----------------------------------------------------------------------
    // (e) name-found-but-no-arity-match → Unknown, NOT a wrong-arity Source
    // -----------------------------------------------------------------------

    #[test]
    fn bare_name_match_arity_mismatch_emits_unknown_route() {
        // DoFoo takes 0 params.  Calling with arity 2 → name found, no overload
        // matches → Unknown route (not a false-confident Source to DoFoo).
        let src: &'static str = r#"
codeunit 50103 "ArityMismatchCU"
{
    procedure DoFoo()
    begin
    end;
}
"#;
        let app_id = make_app_id("TestApp");
        let unit = make_unit(app_id, "ArityMismatchCU.al", src);
        let units = [unit];
        let graph = build_graph(&units, None);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let from_obj = find_obj(&graph, "ArityMismatchCU");
        // "dofoo" exists with arity 0; we request arity 2 → no match.
        let routes = resolve_bare(
            from_obj,
            "dofoo",
            2,
            &graph,
            &index,
            &body_map,
            WithState::NoWithProven,
        );

        assert_eq!(routes.len(), 1);
        let r = &routes[0];
        assert_eq!(
            r.target,
            RouteTarget::Unresolved,
            "arity-mismatch must yield Unresolved target, not a Source route to the wrong-arity proc"
        );
        assert_eq!(
            r.evidence,
            Evidence::Unknown,
            "arity-mismatch must yield Unknown evidence"
        );
        assert_eq!(
            r.witness,
            Witness::None,
            "arity-mismatch must yield None witness"
        );
    }

    // -----------------------------------------------------------------------
    // Task-5 helper: find the AppRef for the first (and only) app in graph
    // -----------------------------------------------------------------------

    fn sole_app_ref(graph: &ProgramGraph) -> AppRef {
        graph
            .apps
            .find_by_name("TestApp")
            .expect("TestApp must be registered")
    }

    // -----------------------------------------------------------------------
    // Task-5 (a): Codeunit.Run to a known codeunit → Exact, OnRun, Source
    // -----------------------------------------------------------------------

    #[test]
    fn object_run_codeunit_to_known_codeunit_resolves_to_onrun() {
        let src: &'static str = r#"
codeunit 50200 "TargetCU"
{
    trigger OnRun()
    begin
    end;
}
"#;
        let app_id = make_app_id("TestApp");
        let unit = make_unit(app_id, "TargetCU.al", src);
        let units = [unit];
        let graph = build_graph(&units, None);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let from = sole_app_ref(&graph);
        let (shape, completeness, routes) = resolve_object_run(
            from,
            ObjectKind::Codeunit,
            Some("TargetCU"),
            true,
            &graph,
            &index,
            &body_map,
        );

        assert_eq!(
            shape,
            DispatchShape::Exact,
            "shape must be Exact for resolved target"
        );
        assert_eq!(
            completeness,
            SetCompleteness::Complete,
            "completeness must be Complete"
        );
        assert_eq!(routes.len(), 1, "expected exactly one route");
        let r = &routes[0];
        assert!(
            matches!(r.target, RouteTarget::Routine(_)),
            "target must be Routine(onrun), got {:?}",
            r.target
        );
        assert_eq!(r.evidence, Evidence::Source);
        assert!(
            matches!(r.witness, Witness::SourceSpan { .. }),
            "witness must be SourceSpan"
        );
        let RouteTarget::Routine(ref rid) = r.target else {
            unreachable!()
        };
        assert_eq!(rid.name_lc, "onrun", "must resolve to the onrun trigger");
    }

    // -----------------------------------------------------------------------
    // Task-5 (b): Page.RunModal to a page → Exact, OnOpenPage (NOT OnRun)
    // -----------------------------------------------------------------------

    #[test]
    fn object_run_page_resolves_to_onopenpage_not_onrun() {
        let src: &'static str = r#"
page 50300 "SomePage"
{
    trigger OnOpenPage()
    begin
    end;
}
"#;
        let app_id = make_app_id("TestApp");
        let unit = make_unit(app_id, "SomePage.al", src);
        let units = [unit];
        let graph = build_graph(&units, None);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let from = sole_app_ref(&graph);
        let (shape, completeness, routes) = resolve_object_run(
            from,
            ObjectKind::Page,
            Some("SomePage"),
            true,
            &graph,
            &index,
            &body_map,
        );

        assert_eq!(shape, DispatchShape::Exact);
        assert_eq!(completeness, SetCompleteness::Complete);
        assert_eq!(routes.len(), 1);
        let RouteTarget::Routine(ref rid) = routes[0].target else {
            panic!("target must be Routine, got {:?}", routes[0].target)
        };
        assert_eq!(
            rid.name_lc, "onopenpage",
            "Page must resolve to onopenpage, NOT onrun"
        );
        assert_eq!(routes[0].evidence, Evidence::Source);
    }

    // -----------------------------------------------------------------------
    // Task-5 (c): No static target → DynamicOpen + Unknown blocker
    // -----------------------------------------------------------------------

    #[test]
    fn object_run_no_target_emits_dynamic_open_with_unknown_blocker() {
        let src: &'static str = r#"
codeunit 50201 "CallerCU"
{
    procedure Run()
    begin
    end;
}
"#;
        let app_id = make_app_id("TestApp");
        let unit = make_unit(app_id, "CallerCU.al", src);
        let units = [unit];
        let graph = build_graph(&units, None);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let from = sole_app_ref(&graph);
        let (shape, completeness, routes) = resolve_object_run(
            from,
            ObjectKind::Codeunit,
            None, // no static target
            true,
            &graph,
            &index,
            &body_map,
        );

        assert_eq!(
            shape,
            DispatchShape::DynamicOpen,
            "no-target must be DynamicOpen"
        );
        assert_eq!(
            completeness,
            SetCompleteness::Partial {
                reason: OpenWorldReason::RuntimeTypeUnbounded
            },
        );
        // Must include a blocker route — not an empty list (open world, not false resolved).
        assert!(!routes.is_empty(), "must include a blocker route");
        let r = &routes[0];
        assert_eq!(r.target, RouteTarget::Unresolved);
        assert_eq!(r.evidence, Evidence::Unknown);
        assert_eq!(r.witness, Witness::None);
    }

    // -----------------------------------------------------------------------
    // Task-5 (d): Target named but not in graph → Unknown (honest failure)
    // Updated by Task-3: opaque-boundary arm replaced with Unresolved/Unknown
    // to avoid creating AbiSymbol keys with the wrong (caller) app ref.
    // -----------------------------------------------------------------------

    #[test]
    fn object_run_target_not_in_graph_emits_unknown() {
        let src: &'static str = r#"
codeunit 50202 "AnotherCaller"
{
    procedure Run()
    begin
    end;
}
"#;
        let app_id = make_app_id("TestApp");
        let unit = make_unit(app_id, "AnotherCaller.al", src);
        let units = [unit];
        let graph = build_graph(&units, None);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let from = sole_app_ref(&graph);
        let (shape, completeness, routes) = resolve_object_run(
            from,
            ObjectKind::Codeunit,
            Some("NonExistentCU"), // not in graph
            true,
            &graph,
            &index,
            &body_map,
        );

        assert_eq!(
            shape,
            DispatchShape::Exact,
            "not-in-graph is still an exact dispatch"
        );
        assert_eq!(
            completeness,
            SetCompleteness::Complete,
            "not-in-graph target is a known boundary (Complete, not RuntimeTypeUnbounded)"
        );
        assert_eq!(routes.len(), 1);
        let r = &routes[0];
        assert_eq!(
            r.target,
            RouteTarget::Unresolved,
            "target not in any indexed app must yield Unresolved (not AbiSymbol); got {:?}",
            r.target
        );
        assert_eq!(
            r.evidence,
            Evidence::Unknown,
            "not-found target must use Unknown evidence (honest failure)"
        );
        assert_eq!(r.witness, Witness::None);
    }

    // -----------------------------------------------------------------------
    // Task-5 (d-ii): Target in graph but entry trigger not found → AbiSymbol Opaque
    // -----------------------------------------------------------------------

    #[test]
    fn object_run_entry_trigger_not_found_emits_opaque() {
        let src: &'static str = r#"
codeunit 50203 "NoTriggerCU"
{
    // Note: no OnRun trigger defined
    procedure SomeProc()
    begin
    end;
}
"#;
        let app_id = make_app_id("TestApp");
        let unit = make_unit(app_id, "NoTriggerCU.al", src);
        let units = [unit];
        let graph = build_graph(&units, None);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let from = sole_app_ref(&graph);
        let (shape, completeness, routes) = resolve_object_run(
            from,
            ObjectKind::Codeunit,
            Some("NoTriggerCU"), // in graph but no OnRun trigger
            true,
            &graph,
            &index,
            &body_map,
        );

        assert_eq!(shape, DispatchShape::Exact);
        assert_eq!(
            completeness,
            SetCompleteness::Complete,
            "object exists; trigger-not-found is a known boundary (Complete)"
        );
        assert_eq!(routes.len(), 1);
        let r = &routes[0];
        assert!(
            matches!(r.target, RouteTarget::AbiSymbol { .. }),
            "target must be AbiSymbol; got {:?}",
            r.target
        );
        assert_eq!(r.evidence, Evidence::Opaque);
        assert!(
            matches!(r.witness, Witness::AbiSymbol { .. }),
            "AbiSymbol target must pair with AbiSymbol witness"
        );
    }

    // -----------------------------------------------------------------------
    // Task-5 (e): insert implicit-trigger on table with OnInsert → Multicast
    // -----------------------------------------------------------------------

    #[test]
    fn implicit_trigger_insert_resolves_to_oninsert_multicast() {
        let src: &'static str = r#"
table 50400 "SomeTable"
{
    trigger OnInsert()
    begin
    end;
}
"#;
        let app_id = make_app_id("TestApp");
        let unit = make_unit(app_id, "SomeTable.al", src);
        let units = [unit];
        let graph = build_graph(&units, None);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let table_obj = find_obj(&graph, "SomeTable");
        let (shape, completeness, routes) =
            resolve_implicit_trigger("insert", table_obj, &graph, &index, &body_map);

        assert_eq!(
            shape,
            DispatchShape::Multicast,
            "implicit trigger must be Multicast"
        );
        assert_eq!(
            completeness,
            SetCompleteness::Partial {
                reason: OpenWorldReason::ReverseDependentExtensions
            },
        );
        assert_eq!(routes.len(), 1, "one oninsert trigger from the base table");
        let r = &routes[0];
        let RouteTarget::Routine(ref rid) = r.target else {
            panic!("target must be Routine, got {:?}", r.target)
        };
        assert_eq!(rid.name_lc, "oninsert");
        assert_eq!(r.evidence, Evidence::Source);
        assert!(matches!(r.witness, Witness::SourceSpan { .. }));
    }

    // -----------------------------------------------------------------------
    // (f) BodyMap-miss on a source-tier graph routine → Unknown, NOT silent Abi
    // -----------------------------------------------------------------------

    #[test]
    fn bare_bodymap_miss_emits_unknown_route() {
        // Build graph with a codeunit containing MyProc, but supply an EMPTY
        // slice to BodyMap::build so the BodyMap has no entries.  MyProc is in
        // the graph (ResolveIndex sees it) but absent from the BodyMap — a real
        // integration gap that must surface as Unknown, not silent Abi degradation.
        let src: &'static str = r#"
codeunit 50104 "BodyMissCU"
{
    procedure MyProc()
    begin
    end;
}
"#;
        let app_id = make_app_id("TestApp");
        let unit = make_unit(app_id, "BodyMissCU.al", src);
        let units = [unit];
        let graph = build_graph(&units, None);
        let index = ResolveIndex::build(&graph);
        // Empty parsed slice: every graph routine is absent from the BodyMap.
        let body_map = BodyMap::build(&graph, &[]);

        let from_obj = find_obj(&graph, "BodyMissCU");
        let routes = resolve_bare(
            from_obj,
            "myproc",
            0,
            &graph,
            &index,
            &body_map,
            WithState::NoWithProven,
        );

        assert_eq!(routes.len(), 1);
        let r = &routes[0];
        assert_eq!(
            r.target,
            RouteTarget::Unresolved,
            "BodyMap-miss must yield Unresolved (not Routine+Abi)"
        );
        assert_eq!(
            r.evidence,
            Evidence::Unknown,
            "BodyMap-miss must yield Unknown evidence"
        );
        assert_eq!(
            r.witness,
            Witness::None,
            "BodyMap-miss must yield None witness"
        );
    }

    // -----------------------------------------------------------------------
    // Task 0 (Phase 3): overloads with distinct params_count → distinct
    // RoutineNodeIds, resolved by arity
    // -----------------------------------------------------------------------

    #[test]
    fn overloads_distinct_by_arity_and_resolved_by_arity() {
        // A codeunit with Post() (0 params) and Post(x: Integer) (1 param).
        // After the arity discriminant, both must produce DISTINCT RoutineNodeIds
        // and resolve_bare must pick the arity-matched overload.
        let src: &'static str = r#"
codeunit 50300 "OverloadCU"
{
    procedure Post()
    begin
    end;
    procedure Post(x: Integer)
    begin
    end;
}
"#;
        let app_id = make_app_id("TestApp");
        let unit = make_unit(app_id, "OverloadCU.al", src);
        let units = [unit];
        let graph = build_graph(&units, None);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        // Collect the RoutineNodeIds for "post" from the graph.
        let post_rids: Vec<_> = graph
            .routines
            .iter()
            .filter(|r| r.id.name_lc == "post")
            .map(|r| r.id.clone())
            .collect();
        assert_eq!(post_rids.len(), 2, "IR must expose two Post overloads");

        // Their params_counts must be 0 and 1.
        let mut counts: Vec<usize> = post_rids.iter().map(|r| r.params_count).collect();
        counts.sort();
        assert_eq!(
            counts,
            vec![0, 1],
            "Post() and Post(x: Integer) must have params_count 0 and 1"
        );

        // The two RoutineNodeIds must be DISTINCT (params_count is part of the key).
        assert_ne!(
            post_rids[0], post_rids[1],
            "overloads must have distinct RoutineNodeIds after adding params_count"
        );

        let from_obj = find_obj(&graph, "OverloadCU");

        // Resolve with arity=1 → must get the Post(x: Integer) overload (params_count=1).
        let routes1 = resolve_bare(
            from_obj,
            "post",
            1,
            &graph,
            &index,
            &body_map,
            WithState::NoWithProven,
        );
        assert_eq!(routes1.len(), 1);
        let r1 = &routes1[0];
        assert!(
            matches!(r1.target, RouteTarget::Routine(_)),
            "arity=1 call must resolve to a Routine, not Unknown; got {:?}",
            r1.target
        );
        let RouteTarget::Routine(ref rid1) = r1.target else {
            unreachable!()
        };
        assert_eq!(
            rid1.params_count, 1,
            "arity=1 call must resolve to the 1-param overload"
        );

        // Resolve with arity=0 → must get the Post() overload (params_count=0).
        let routes0 = resolve_bare(
            from_obj,
            "post",
            0,
            &graph,
            &index,
            &body_map,
            WithState::NoWithProven,
        );
        assert_eq!(routes0.len(), 1);
        let r0 = &routes0[0];
        assert!(
            matches!(r0.target, RouteTarget::Routine(_)),
            "arity=0 call must resolve to a Routine; got {:?}",
            r0.target
        );
        let RouteTarget::Routine(ref rid0) = r0.target else {
            unreachable!()
        };
        assert_eq!(
            rid0.params_count, 0,
            "arity=0 call must resolve to the 0-param overload"
        );
    }

    // -----------------------------------------------------------------------
    // Phase 3 Task 2 — resolve_member tests
    // -----------------------------------------------------------------------

    /// Build a minimal test graph, index and body_map for resolve_member tests.
    fn minimal_resolve_member_fixtures() -> (
        ProgramGraph,
        ResolveIndex,
        crate::program::resolve::body_map::BodyMap<'static>,
        crate::program::node_extract::ObjectNode,
    ) {
        use crate::program::node::{AppRegistry, ObjKey, ObjectNodeId};
        use crate::program::node_extract::ObjectNode;
        use crate::snapshot::{AppId, TrustTier};
        use al_syntax::ir::ObjectKind;

        let mut apps = AppRegistry::default();
        let app_id = AppId {
            guid: String::new(),
            name: "T".into(),
            publisher: "T".into(),
            version: "1.0.0.0".into(),
        };
        let app = apps.intern(&app_id);
        let graph = ProgramGraph {
            apps,
            topology: crate::program::topology::DependencyGraph::default(),
            objects: vec![],
            routines: vec![],
            obj_index: crate::program::graph::ObjectIndex::build(&[]),
        };
        let index = ResolveIndex::build(&graph);
        let body_map = crate::program::resolve::body_map::BodyMap::build(&graph, &[]);
        let from_obj = ObjectNode {
            id: ObjectNodeId {
                app,
                kind: ObjectKind::Codeunit,
                key: ObjKey::Id(1),
            },
            name: "T".into(),
            declared_id: Some(1),
            extends_target: None,
            implements: vec![],
            tier: TrustTier::Workspace,
            source_table: None,
            table_no: None,
            source_table_temporary: false,
            page_controls: vec![],
        };
        (graph, index, body_map, from_obj)
    }

    #[test]
    fn resolve_member_framework_json_object_catalog_route() {
        use crate::program::resolve::receiver::{FrameworkKind, ReceiverType};
        let (graph, index, body_map, from_obj) = minimal_resolve_member_fixtures();

        let receiver = ReceiverType::Framework(FrameworkKind::JsonObject);
        let (shape, routes) =
            resolve_member(&receiver, "add", 1, &from_obj, &graph, &index, &body_map);
        assert_eq!(shape, DispatchShape::Exact);
        assert_eq!(routes.len(), 1);
        assert!(
            matches!(routes[0].target, RouteTarget::Builtin(_)),
            "must be Builtin"
        );
        assert_eq!(routes[0].evidence, Evidence::Catalog);
        assert!(matches!(routes[0].witness, Witness::CatalogEntry { .. }));
        if let RouteTarget::Builtin(ref bid) = routes[0].target {
            assert_eq!(bid.0, "JsonObject::add");
        }
    }

    #[test]
    fn resolve_member_fieldref_value_catalog_route() {
        use crate::program::resolve::receiver::ReceiverType;
        let (graph, index, body_map, from_obj) = minimal_resolve_member_fixtures();

        let receiver = ReceiverType::FieldRef;
        let (shape, routes) =
            resolve_member(&receiver, "value", 0, &from_obj, &graph, &index, &body_map);
        assert_eq!(shape, DispatchShape::Exact);
        assert!(matches!(routes[0].target, RouteTarget::Builtin(_)));
        assert_eq!(routes[0].evidence, Evidence::Catalog);
    }

    #[test]
    fn resolve_member_record_builtin_catalog_route() {
        use crate::program::resolve::receiver::ReceiverType;
        let (graph, index, body_map, from_obj) = minimal_resolve_member_fixtures();

        // Record builtin → Catalog route
        let receiver = ReceiverType::Record { table: None };
        let (shape, routes) = resolve_member(
            &receiver, "setrange", 2, &from_obj, &graph, &index, &body_map,
        );
        assert_eq!(shape, DispatchShape::Exact);
        assert!(matches!(routes[0].target, RouteTarget::Builtin(_)));
        assert_eq!(routes[0].evidence, Evidence::Catalog);

        // Non-builtin Record method → Unknown
        let (_, routes2) = resolve_member(
            &receiver,
            "calculatediscount",
            0,
            &from_obj,
            &graph,
            &index,
            &body_map,
        );
        assert_eq!(routes2[0].target, RouteTarget::Unresolved);
        assert_eq!(routes2[0].evidence, Evidence::Unknown);
    }

    #[test]
    fn resolve_member_primitive_is_unknown() {
        use crate::program::resolve::receiver::ReceiverType;
        let (graph, index, body_map, from_obj) = minimal_resolve_member_fixtures();

        let (shape, routes) = resolve_member(
            &ReceiverType::Primitive,
            "totext",
            0,
            &from_obj,
            &graph,
            &index,
            &body_map,
        );
        assert_eq!(shape, DispatchShape::Exact);
        assert_eq!(routes[0].evidence, Evidence::Unknown);
    }

    #[test]
    fn resolve_member_dynamic_is_dynamic_open() {
        use crate::program::resolve::receiver::ReceiverType;
        let (graph, index, body_map, from_obj) = minimal_resolve_member_fixtures();

        let (shape, _routes) = resolve_member(
            &ReceiverType::Dynamic,
            "whatever",
            0,
            &from_obj,
            &graph,
            &index,
            &body_map,
        );
        assert_eq!(shape, DispatchShape::DynamicOpen);
    }

    // -----------------------------------------------------------------------
    // Phase 3 Task 3 — Object dispatch + SelfObject tests
    // -----------------------------------------------------------------------

    // (a) Object receiver (Codeunit "MyTarget") + known method "dowork"
    //     → Exact, Routine, Source evidence, SourceSpan witness
    #[test]
    fn resolve_member_object_known_method_resolves_to_source_route() {
        use crate::program::resolve::receiver::ReceiverType;

        let src_target: &'static str = r#"
codeunit 50500 "MyTarget"
{
    procedure DoWork()
    begin
    end;
}
"#;
        let src_caller: &'static str = r#"
codeunit 50501 "Caller"
{
    procedure Trigger()
    begin
    end;
}
"#;
        let app_id = make_app_id("TestApp");
        let unit_target = make_unit(app_id.clone(), "MyTarget.al", src_target);
        let unit_caller = make_unit(app_id, "Caller.al", src_caller);
        let units = [unit_target, unit_caller];
        let graph = build_graph(&units, None);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let from_obj = find_obj(&graph, "Caller");
        let receiver = ReceiverType::Object {
            kind: ObjectKind::Codeunit,
            name_lc: "mytarget".into(),
            id: None,
        };
        let (shape, routes) =
            resolve_member(&receiver, "dowork", 0, from_obj, &graph, &index, &body_map);

        assert_eq!(shape, DispatchShape::Exact);
        assert_eq!(routes.len(), 1);
        assert!(
            matches!(routes[0].target, RouteTarget::Routine(_)),
            "target must be Routine; got {:?}",
            routes[0].target
        );
        assert_eq!(routes[0].evidence, Evidence::Source);
        assert!(
            matches!(routes[0].witness, Witness::SourceSpan { .. }),
            "witness must be SourceSpan; got {:?}",
            routes[0].witness
        );
        let RouteTarget::Routine(ref rid) = routes[0].target else {
            unreachable!()
        };
        assert_eq!(rid.name_lc, "dowork");
    }

    // Task 7: `ReceiverType::Object`'s carried `id` (mechanically resolved by
    // Phase A — Step 0's `CurrPage.<part>.Page` subpage receiver) must make
    // `resolve_member` short-circuit on the id DIRECTLY rather than
    // re-resolving `name_lc` against the graph. Proven by giving a `name_lc`
    // that resolves to nothing (`"doesnotexist"`) alongside a valid `id`: if
    // the short-circuit were absent, this would fall back to the by-name
    // lookup and emit an honest Unknown; with it, the call resolves via the
    // carried id regardless of `name_lc`.
    #[test]
    fn resolve_member_object_carried_id_short_circuits_name_lookup() {
        use crate::program::resolve::receiver::ReceiverType;

        let src_target: &'static str = r#"
codeunit 50502 "RealTarget"
{
    procedure DoWork()
    begin
    end;
}
"#;
        let src_caller: &'static str = r#"
codeunit 50503 "Caller2"
{
    procedure Trigger()
    begin
    end;
}
"#;
        let app_id = make_app_id("TestApp");
        let unit_target = make_unit(app_id.clone(), "RealTarget.al", src_target);
        let unit_caller = make_unit(app_id, "Caller2.al", src_caller);
        let units = [unit_target, unit_caller];
        let graph = build_graph(&units, None);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let from_obj = find_obj(&graph, "caller2");
        let real_target_id = find_obj(&graph, "realtarget").id.clone();

        // `name_lc` is deliberately WRONG — a name that resolves to nothing —
        // to prove the carried `id` is what actually drives resolution.
        let receiver = ReceiverType::Object {
            kind: ObjectKind::Codeunit,
            name_lc: "doesnotexist".into(),
            id: Some(real_target_id.clone()),
        };
        let (shape, routes) =
            resolve_member(&receiver, "dowork", 0, from_obj, &graph, &index, &body_map);

        assert_eq!(shape, DispatchShape::Exact);
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].evidence, Evidence::Source);
        let RouteTarget::Routine(ref rid) = routes[0].target else {
            panic!(
                "expected RouteTarget::Routine via the carried id, got {:?}",
                routes[0].target
            );
        };
        assert_eq!(rid.name_lc, "dowork");
        assert_eq!(
            rid.object, real_target_id,
            "must dispatch against the CARRIED id, not a (failed) by-name lookup"
        );
    }

    // (b) Codeunit.Run() (arity 0) → Exact, Routine(onrun), Source, SourceSpan
    #[test]
    fn resolve_member_codeunit_run_dispatches_to_onrun_trigger() {
        use crate::program::resolve::receiver::ReceiverType;

        let src_target: &'static str = r#"
codeunit 50502 "RunTarget"
{
    trigger OnRun()
    begin
    end;
}
"#;
        let src_caller: &'static str = r#"
codeunit 50503 "RunCaller"
{
    procedure CallRun()
    begin
    end;
}
"#;
        let app_id = make_app_id("TestApp");
        let unit_target = make_unit(app_id.clone(), "RunTarget.al", src_target);
        let unit_caller = make_unit(app_id, "RunCaller.al", src_caller);
        let units = [unit_target, unit_caller];
        let graph = build_graph(&units, None);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let from_obj = find_obj(&graph, "RunCaller");
        let receiver = ReceiverType::Object {
            kind: ObjectKind::Codeunit,
            name_lc: "runtarget".into(),
            id: None,
        };
        // arity=0 is Cu.Run() with no record argument
        let (shape, routes) =
            resolve_member(&receiver, "run", 0, from_obj, &graph, &index, &body_map);

        assert_eq!(shape, DispatchShape::Exact);
        assert_eq!(routes.len(), 1);
        assert!(
            matches!(routes[0].target, RouteTarget::Routine(_)),
            "Codeunit.Run must resolve to the OnRun trigger; got {:?}",
            routes[0].target
        );
        assert_eq!(routes[0].evidence, Evidence::Source);
        assert!(matches!(routes[0].witness, Witness::SourceSpan { .. }));
        let RouteTarget::Routine(ref rid) = routes[0].target else {
            unreachable!()
        };
        assert_eq!(rid.name_lc, "onrun", "must target the OnRun trigger");
    }

    // (c) SelfObject + "helper" proc on the from_object → Exact, Routine, Source
    #[test]
    fn resolve_member_self_object_resolves_own_procedure() {
        use crate::program::resolve::receiver::ReceiverType;

        let src_self: &'static str = r#"
codeunit 50504 "SelfCU"
{
    procedure Helper()
    begin
    end;
    procedure Caller()
    begin
    end;
}
"#;
        let app_id = make_app_id("TestApp");
        let unit_self = make_unit(app_id, "SelfCU.al", src_self);
        let units = [unit_self];
        let graph = build_graph(&units, None);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let from_obj = find_obj(&graph, "SelfCU");
        let (shape, routes) = resolve_member(
            &ReceiverType::SelfObject,
            "helper",
            0,
            from_obj,
            &graph,
            &index,
            &body_map,
        );

        assert_eq!(shape, DispatchShape::Exact);
        assert_eq!(routes.len(), 1);
        assert!(
            matches!(routes[0].target, RouteTarget::Routine(_)),
            "SelfObject must resolve to own-object Routine; got {:?}",
            routes[0].target
        );
        assert_eq!(routes[0].evidence, Evidence::Source);
        assert!(matches!(routes[0].witness, Witness::SourceSpan { .. }));
        let RouteTarget::Routine(ref rid) = routes[0].target else {
            unreachable!()
        };
        assert_eq!(rid.name_lc, "helper");
    }

    // (d) Object receiver + nonexistent method → Exact, Unresolved, Unknown, None
    #[test]
    fn resolve_member_object_nonexistent_method_emits_unknown() {
        use crate::program::resolve::receiver::ReceiverType;

        let src_target: &'static str = r#"
codeunit 50505 "AnotherTarget"
{
    procedure RealProc()
    begin
    end;
}
"#;
        let src_caller: &'static str = r#"
codeunit 50506 "AnotherCaller"
{
    procedure Go()
    begin
    end;
}
"#;
        let app_id = make_app_id("TestApp");
        let unit_target = make_unit(app_id.clone(), "AnotherTarget.al", src_target);
        let unit_caller = make_unit(app_id, "AnotherCaller.al", src_caller);
        let units = [unit_target, unit_caller];
        let graph = build_graph(&units, None);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let from_obj = find_obj(&graph, "AnotherCaller");
        let receiver = ReceiverType::Object {
            kind: ObjectKind::Codeunit,
            name_lc: "anothertarget".into(),
            id: None,
        };
        let (shape, routes) = resolve_member(
            &receiver,
            "doesnotexistatall",
            0,
            from_obj,
            &graph,
            &index,
            &body_map,
        );

        assert_eq!(shape, DispatchShape::Exact);
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].target, RouteTarget::Unresolved);
        assert_eq!(routes[0].evidence, Evidence::Unknown);
        assert_eq!(routes[0].witness, Witness::None);
    }

    // (e) Object receiver where the target object isn't in the graph → Unknown
    #[test]
    fn resolve_member_object_target_not_in_graph_emits_unknown() {
        use crate::program::resolve::receiver::ReceiverType;

        let src_caller: &'static str = r#"
codeunit 50507 "OrphanCaller"
{
    procedure Go()
    begin
    end;
}
"#;
        let app_id = make_app_id("TestApp");
        let unit_caller = make_unit(app_id, "OrphanCaller.al", src_caller);
        let units = [unit_caller];
        let graph = build_graph(&units, None);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let from_obj = find_obj(&graph, "OrphanCaller");
        // "ghosttarget" does not exist in the graph at all
        let receiver = ReceiverType::Object {
            kind: ObjectKind::Codeunit,
            name_lc: "ghosttarget".into(),
            id: None,
        };
        let (shape, routes) = resolve_member(
            &receiver,
            "anymethod",
            0,
            from_obj,
            &graph,
            &index,
            &body_map,
        );

        assert_eq!(shape, DispatchShape::Exact);
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].target, RouteTarget::Unresolved);
        assert_eq!(routes[0].evidence, Evidence::Unknown);
        assert_eq!(routes[0].witness, Witness::None);
    }

    // -----------------------------------------------------------------------
    // Phase 3 Task 4 — Record table-procedure dispatch tests
    // -----------------------------------------------------------------------

    // (a) Record receiver + proc on base table → Exact, Source, SourceSpan
    #[test]
    fn resolve_member_record_table_proc_on_base_resolves_to_source() {
        use crate::program::resolve::receiver::ReceiverType;

        let src_table: &'static str = r#"
table 50700 Customer
{
    procedure GetBalance()
    begin
    end;
}
"#;
        let src_caller: &'static str = r#"
codeunit 50701 "TableProcCaller"
{
    procedure Test()
    begin
    end;
}
"#;
        let app_id = make_app_id("TestApp");
        let unit_table = make_unit(app_id.clone(), "Customer.al", src_table);
        let unit_caller = make_unit(app_id, "TableProcCaller.al", src_caller);
        let units = [unit_table, unit_caller];
        let graph = build_graph(&units, None);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let table_obj = find_obj(&graph, "Customer");
        let receiver = ReceiverType::Record {
            table: Some(table_obj.id.clone()),
        };
        let from_obj = find_obj(&graph, "TableProcCaller");
        let (shape, routes) = resolve_member(
            &receiver,
            "getbalance",
            0,
            from_obj,
            &graph,
            &index,
            &body_map,
        );

        assert_eq!(shape, DispatchShape::Exact);
        assert_eq!(routes.len(), 1);
        assert!(
            matches!(routes[0].target, RouteTarget::Routine(_)),
            "must resolve to Routine on the base table; got {:?}",
            routes[0].target
        );
        assert_eq!(routes[0].evidence, Evidence::Source);
        assert!(
            matches!(routes[0].witness, Witness::SourceSpan { .. }),
            "witness must be SourceSpan; got {:?}",
            routes[0].witness
        );
        let RouteTarget::Routine(ref rid) = routes[0].target else {
            unreachable!()
        };
        assert_eq!(rid.name_lc, "getbalance");
    }

    // (b) Record receiver + proc only on a TableExtension → resolves via extension
    #[test]
    fn resolve_member_record_table_proc_on_extension_resolves_to_source() {
        use crate::program::resolve::receiver::ReceiverType;

        // Base table has ExistingProc but NOT GetBalance.
        let src_table: &'static str = r#"
table 50800 Vendor
{
    procedure ExistingProc()
    begin
    end;
}
"#;
        // Extension adds GetBalance.
        let src_ext: &'static str = r#"
tableextension 50801 "VendorExt" extends Vendor
{
    procedure GetBalance()
    begin
    end;
}
"#;
        let src_caller: &'static str = r#"
codeunit 50802 "ExtCaller"
{
    procedure Test()
    begin
    end;
}
"#;
        let app_id = make_app_id("TestApp");
        let unit_table = make_unit(app_id.clone(), "Vendor.al", src_table);
        let unit_ext = make_unit(app_id.clone(), "VendorExt.al", src_ext);
        let unit_caller = make_unit(app_id, "ExtCaller.al", src_caller);
        let units = [unit_table, unit_ext, unit_caller];
        let graph = build_graph(&units, None);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let table_obj = find_obj(&graph, "Vendor");
        let receiver = ReceiverType::Record {
            table: Some(table_obj.id.clone()),
        };
        let from_obj = find_obj(&graph, "ExtCaller");
        let (shape, routes) = resolve_member(
            &receiver,
            "getbalance",
            0,
            from_obj,
            &graph,
            &index,
            &body_map,
        );

        assert_eq!(shape, DispatchShape::Exact);
        assert_eq!(routes.len(), 1);
        assert!(
            matches!(routes[0].target, RouteTarget::Routine(_)),
            "must resolve to Routine via extension; got {:?}",
            routes[0].target
        );
        assert_eq!(routes[0].evidence, Evidence::Source);
        assert!(
            matches!(routes[0].witness, Witness::SourceSpan { .. }),
            "witness must be SourceSpan; got {:?}",
            routes[0].witness
        );
        let RouteTarget::Routine(ref rid) = routes[0].target else {
            unreachable!()
        };
        assert_eq!(
            rid.object.kind,
            ObjectKind::TableExtension,
            "proc must resolve to the TableExtension, not the base table"
        );
        assert_eq!(rid.name_lc, "getbalance");
    }

    // (c) Record builtin method, NO same-name source competitor → Catalog.
    // (beyond-1B.3b Task 1: renamed from `..._wins_catalog_first` — the table
    // here declares `GetBalance`, not `SetView`, so this was never actually
    // testing catalog-vs-source precedence; it tests that a genuine builtin
    // with zero visible source candidates still resolves Catalog.  The real
    // shadowing case is covered by the `ws-builtin-shadow` r0-corpus fixture
    // in `tests/program_resolve_harness.rs`.)
    #[test]
    fn resolve_member_record_builtin_with_no_source_competitor_resolves_catalog() {
        use crate::program::resolve::receiver::ReceiverType;

        let src_table: &'static str = r#"
table 50900 "SomeTable"
{
    procedure GetBalance()
    begin
    end;
}
"#;
        let src_caller: &'static str = r#"
codeunit 50901 "CatalogFirstCaller"
{
    procedure Test()
    begin
    end;
}
"#;
        let app_id = make_app_id("TestApp");
        let unit_table = make_unit(app_id.clone(), "SomeTable.al", src_table);
        let unit_caller = make_unit(app_id, "CatalogFirstCaller.al", src_caller);
        let units = [unit_table, unit_caller];
        let graph = build_graph(&units, None);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let table_obj = find_obj(&graph, "SomeTable");
        let receiver = ReceiverType::Record {
            table: Some(table_obj.id.clone()),
        };
        let from_obj = find_obj(&graph, "CatalogFirstCaller");

        // "setview" is a Record catalog builtin; "SomeTable" declares no
        // "setview" procedure (base table or extension) — zero source
        // candidates, so resolution correctly falls through to the catalog.
        let (shape, routes) =
            resolve_member(&receiver, "setview", 0, from_obj, &graph, &index, &body_map);

        assert_eq!(shape, DispatchShape::Exact);
        assert_eq!(routes.len(), 1);
        assert!(
            matches!(routes[0].target, RouteTarget::Builtin(_)),
            "setview is a Record builtin — Catalog must win; got {:?}",
            routes[0].target
        );
        assert_eq!(routes[0].evidence, Evidence::Catalog);
        assert!(matches!(routes[0].witness, Witness::CatalogEntry { .. }));
    }

    // (d) table == None → Exact, Unknown (table unresolved, e.g. implicit Rec on Page)
    #[test]
    fn resolve_member_record_table_none_emits_unknown() {
        use crate::program::resolve::receiver::ReceiverType;

        let src_caller: &'static str = r#"
codeunit 51000 "NoneTableCaller"
{
    procedure Test()
    begin
    end;
}
"#;
        let app_id = make_app_id("TestApp");
        let unit_caller = make_unit(app_id, "NoneTableCaller.al", src_caller);
        let units = [unit_caller];
        let graph = build_graph(&units, None);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let from_obj = find_obj(&graph, "NoneTableCaller");
        let receiver = ReceiverType::Record { table: None };
        let (shape, routes) = resolve_member(
            &receiver,
            "getbalance",
            0,
            from_obj,
            &graph,
            &index,
            &body_map,
        );

        assert_eq!(shape, DispatchShape::Exact);
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].target, RouteTarget::Unresolved);
        assert_eq!(routes[0].evidence, Evidence::Unknown);
        assert_eq!(routes[0].witness, Witness::None);
    }

    // (e) Method exists on neither base table nor any extension → Exact, Unknown
    #[test]
    fn resolve_member_record_proc_not_found_emits_unknown() {
        use crate::program::resolve::receiver::ReceiverType;

        let src_table: &'static str = r#"
table 51100 "Invoice"
{
    procedure ValidateTotal()
    begin
    end;
}
"#;
        let src_caller: &'static str = r#"
codeunit 51101 "NotFoundCaller"
{
    procedure Test()
    begin
    end;
}
"#;
        let app_id = make_app_id("TestApp");
        let unit_table = make_unit(app_id.clone(), "Invoice.al", src_table);
        let unit_caller = make_unit(app_id, "NotFoundCaller.al", src_caller);
        let units = [unit_table, unit_caller];
        let graph = build_graph(&units, None);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let table_obj = find_obj(&graph, "Invoice");
        let receiver = ReceiverType::Record {
            table: Some(table_obj.id.clone()),
        };
        let from_obj = find_obj(&graph, "NotFoundCaller");
        let (shape, routes) = resolve_member(
            &receiver,
            "nonexistentmethod",
            0,
            from_obj,
            &graph,
            &index,
            &body_map,
        );

        assert_eq!(shape, DispatchShape::Exact);
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].target, RouteTarget::Unresolved);
        assert_eq!(routes[0].evidence, Evidence::Unknown);
        assert_eq!(routes[0].witness, Witness::None);
    }

    // -----------------------------------------------------------------------
    // beyond-1B.3b Task 1 — source shadows builtin (lookup precedence)
    // -----------------------------------------------------------------------

    // (f) A user table procedure whose NAME+ARITY matches a genuine Record
    // builtin (`FieldNo`) must SHADOW the catalog: resolves to the local
    // Source, not `builtin`. This is the exact shape of the 42 real CDO
    // `builtin-catalog-fp-collision` divergences (e.g. `Record::fieldno`,
    // `Record::setrecfilter`).
    #[test]
    fn resolve_member_record_source_proc_shadows_same_named_builtin() {
        use crate::program::resolve::receiver::ReceiverType;

        let src_table: &'static str = r#"
table 50950 "Acme"
{
    procedure FieldNo(FieldName: Text): Integer
    begin
        exit(0);
    end;
}
"#;
        let src_caller: &'static str = r#"
codeunit 50951 "ShadowCaller"
{
    procedure Test()
    begin
    end;
}
"#;
        let app_id = make_app_id("TestApp");
        let unit_table = make_unit(app_id.clone(), "Acme.al", src_table);
        let unit_caller = make_unit(app_id, "ShadowCaller.al", src_caller);
        let units = [unit_table, unit_caller];
        let graph = build_graph(&units, None);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let table_obj = find_obj(&graph, "Acme");
        let receiver = ReceiverType::Record {
            table: Some(table_obj.id.clone()),
        };
        let from_obj = find_obj(&graph, "ShadowCaller");

        // "fieldno" IS a genuine Record catalog builtin (arity 1) — but the
        // table declares its OWN FieldNo(FieldName: Text), matching arity 1.
        let (shape, routes) =
            resolve_member(&receiver, "fieldno", 1, from_obj, &graph, &index, &body_map);

        assert_eq!(shape, DispatchShape::Exact);
        assert_eq!(routes.len(), 1);
        assert!(
            matches!(routes[0].target, RouteTarget::Routine(_)),
            "local FieldNo must SHADOW the catalog; got {:?}",
            routes[0].target
        );
        assert_eq!(
            routes[0].evidence,
            Evidence::Source,
            "shadowed call must be Source, not Catalog"
        );
        assert!(matches!(routes[0].witness, Witness::SourceSpan { .. }));
        let RouteTarget::Routine(ref rid) = routes[0].target else {
            unreachable!()
        };
        assert_eq!(rid.name_lc, "fieldno");
        assert_eq!(rid.object.kind, ObjectKind::Table);
    }

    // (g) Two sibling TableExtensions of the same base table both declare a
    // same-name/arity method that ALSO happens to be a Record builtin name:
    // ambiguous source competition must shadow the catalog with an honest
    // Unknown — never pick-first, never fall through to a false `builtin`.
    #[test]
    fn resolve_member_record_ambiguous_extension_competitors_shadow_catalog_as_unknown() {
        use crate::program::resolve::receiver::ReceiverType;

        let src_table: &'static str = r#"
table 50960 "Widget"
{
}
"#;
        // Two independent extensions of the SAME base table both add a
        // `Rename` procedure (arity 1) — `rename` IS a Record catalog builtin.
        let src_ext1: &'static str = r#"
tableextension 50961 "WidgetExt1" extends Widget
{
    procedure Rename(NewName: Text)
    begin
    end;
}
"#;
        let src_ext2: &'static str = r#"
tableextension 50962 "WidgetExt2" extends Widget
{
    procedure Rename(NewName: Text)
    begin
    end;
}
"#;
        let src_caller: &'static str = r#"
codeunit 50963 "AmbiguousCaller"
{
    procedure Test()
    begin
    end;
}
"#;
        let app_id = make_app_id("TestApp");
        let unit_table = make_unit(app_id.clone(), "Widget.al", src_table);
        let unit_ext1 = make_unit(app_id.clone(), "WidgetExt1.al", src_ext1);
        let unit_ext2 = make_unit(app_id.clone(), "WidgetExt2.al", src_ext2);
        let unit_caller = make_unit(app_id, "AmbiguousCaller.al", src_caller);
        let units = [unit_table, unit_ext1, unit_ext2, unit_caller];
        let graph = build_graph(&units, None);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let table_obj = find_obj(&graph, "Widget");
        let receiver = ReceiverType::Record {
            table: Some(table_obj.id.clone()),
        };
        let from_obj = find_obj(&graph, "AmbiguousCaller");

        let (shape, routes) =
            resolve_member(&receiver, "rename", 1, from_obj, &graph, &index, &body_map);

        assert_eq!(shape, DispatchShape::Exact);
        assert_eq!(routes.len(), 1);
        assert_eq!(
            routes[0].target,
            RouteTarget::Unresolved,
            "ambiguous source competitors must NOT pick-first AND must NOT \
             fall through to a false Catalog hit; got {:?}",
            routes[0].target
        );
        assert_eq!(
            routes[0].evidence,
            Evidence::Unknown,
            "source ambiguity must shadow the catalog as honest Unknown"
        );
        assert_eq!(routes[0].witness, Witness::None);
    }

    // -----------------------------------------------------------------------
    // beyond-1B.3b Task 2 — `resolve_in_table_scope` visibility-scoping
    // characterization (closure filter + Local/Internal access filter).
    // -----------------------------------------------------------------------

    // (h) The BASE TABLE and one of its TableExtensions both declare the SAME
    // name+arity method (not just two sibling extensions, as in (g) above):
    // honest ambiguous Unknown — never pick-first, never fall through to a
    // false Catalog hit.
    #[test]
    fn resolve_member_record_base_and_extension_same_name_collision_is_unknown() {
        use crate::program::resolve::receiver::ReceiverType;

        let src_table: &'static str = r#"
table 52200 "CollBase"
{
    procedure DoWork()
    begin
    end;
}
"#;
        let src_ext: &'static str = r#"
tableextension 52201 "CollBaseExt" extends CollBase
{
    procedure DoWork()
    begin
    end;
}
"#;
        let src_caller: &'static str = r#"
codeunit 52202 "CollCaller"
{
    procedure Test()
    begin
    end;
}
"#;
        let app_id = make_app_id("TestApp");
        let unit_table = make_unit(app_id.clone(), "CollBase.al", src_table);
        let unit_ext = make_unit(app_id.clone(), "CollBaseExt.al", src_ext);
        let unit_caller = make_unit(app_id, "CollCaller.al", src_caller);
        let units = [unit_table, unit_ext, unit_caller];
        let graph = build_graph(&units, None);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let table_obj = find_obj(&graph, "CollBase");
        let receiver = ReceiverType::Record {
            table: Some(table_obj.id.clone()),
        };
        let from_obj = find_obj(&graph, "CollCaller");
        let (shape, routes) =
            resolve_member(&receiver, "dowork", 0, from_obj, &graph, &index, &body_map);

        assert_eq!(shape, DispatchShape::Exact);
        assert_eq!(routes.len(), 1);
        assert_eq!(
            routes[0].target,
            RouteTarget::Unresolved,
            "base+extension same-name collision must NOT pick-first; got {:?}",
            routes[0].target
        );
        assert_eq!(routes[0].evidence, Evidence::Unknown);
        assert_eq!(routes[0].witness, Witness::None);
    }

    // (i) THE core Task 2 soundness fix: a TableExtension declared in an app
    // that is NOT in `from_object`'s dependency closure must NOT be counted
    // as a visible candidate. Pre-Task-2, `table_extensions_of` was
    // whole-snapshot (no closure filter) and this resolved to a false
    // `Source` route pointing at a symbol `from_object`'s own app never
    // imported — a real AL compile could never have produced that edge.
    #[test]
    fn resolve_member_record_extension_outside_dependency_closure_declines() {
        use crate::program::resolve::receiver::ReceiverType;

        // AppB: the base table, no procedures of its own.
        let src_table: &'static str = r#"
table 52300 "VisFoo"
{
}
"#;
        // AppC: a TableExtension of VisFoo declaring DoWork — AppC is NEVER
        // wired as a dependency of AppA below.
        let src_ext: &'static str = r#"
tableextension 52301 "VisFooExtC" extends VisFoo
{
    procedure DoWork()
    begin
    end;
}
"#;
        // AppA: the caller, depends on AppB only (NOT AppC).
        let src_caller: &'static str = r#"
codeunit 52302 "OutClosureCaller"
{
    procedure Test()
    begin
    end;
}
"#;
        let app_a = make_app_id("AppA");
        let app_b = make_app_id("AppB");
        let app_c = make_app_id("AppC");
        let unit_table = make_unit(app_b, "VisFoo.al", src_table);
        let unit_ext = make_unit(app_c, "VisFooExtC.al", src_ext);
        let unit_caller = make_unit(app_a, "OutClosureCaller.al", src_caller);
        let units = [unit_table, unit_ext, unit_caller];
        // AppA -> AppB only; AppC is never a dependency of AppA.
        let graph = build_graph_multi_dep(&units, &[("AppA", "AppB")]);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let table_obj = find_obj(&graph, "VisFoo");
        let receiver = ReceiverType::Record {
            table: Some(table_obj.id.clone()),
        };
        let from_obj = find_obj(&graph, "OutClosureCaller");
        let (shape, routes) =
            resolve_member(&receiver, "dowork", 0, from_obj, &graph, &index, &body_map);

        assert_eq!(shape, DispatchShape::Exact);
        assert_eq!(routes.len(), 1);
        assert_eq!(
            routes[0].target,
            RouteTarget::Unresolved,
            "an extension declared in an app OUTSIDE from_object's dependency \
             closure must NOT resolve — from_object's AL compiler could never \
             have imported that symbol; got {:?}",
            routes[0].target
        );
        assert_eq!(
            routes[0].evidence,
            Evidence::Unknown,
            "must decline honestly, not fabricate a false Source"
        );
        assert_eq!(routes[0].witness, Witness::None);
    }

    // (j) The base table itself, declared in a DEPENDENCY app (cross-app
    // relative to `from_object`), with the candidate method marked
    // `internal`: not visible outside its declaring app — must decline, even
    // though the table's app IS in from_object's closure (the closure filter
    // alone is not sufficient; the access filter is independent).
    #[test]
    fn resolve_member_record_cross_app_base_table_internal_method_excluded() {
        use crate::program::resolve::receiver::ReceiverType;

        let src_table: &'static str = r#"
table 52500 "BaseIntFoo"
{
    internal procedure DoWork()
    begin
    end;
}
"#;
        let src_caller: &'static str = r#"
codeunit 52501 "BaseIntCallerA"
{
    procedure Test()
    begin
    end;
}
"#;
        let app_a = make_app_id("AppA");
        let app_b = make_app_id("AppB");
        let unit_table = make_unit(app_b, "BaseIntFoo.al", src_table);
        let unit_caller = make_unit(app_a, "BaseIntCallerA.al", src_caller);
        let units = [unit_table, unit_caller];
        let graph = build_graph_multi_dep(&units, &[("AppA", "AppB")]);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let table_obj = find_obj(&graph, "BaseIntFoo");
        let receiver = ReceiverType::Record {
            table: Some(table_obj.id.clone()),
        };
        let from_obj = find_obj(&graph, "BaseIntCallerA");
        let (shape, routes) =
            resolve_member(&receiver, "dowork", 0, from_obj, &graph, &index, &body_map);

        assert_eq!(shape, DispatchShape::Exact);
        assert_eq!(routes.len(), 1);
        assert_eq!(
            routes[0].target,
            RouteTarget::Unresolved,
            "a cross-app `internal` method on the BASE TABLE itself must be \
             excluded, not just on extensions; got {:?}",
            routes[0].target
        );
        assert_eq!(routes[0].evidence, Evidence::Unknown);
    }

    // (k) A TableExtension declared in a DEPENDENCY app (in-closure) whose
    // candidate method is marked `internal`: excluded — not visible outside
    // its declaring app even though the app itself is reachable.
    #[test]
    fn resolve_member_record_cross_app_extension_internal_method_excluded() {
        use crate::program::resolve::receiver::ReceiverType;

        let src_table: &'static str = r#"
table 52400 "IntFoo"
{
}
"#;
        let src_ext: &'static str = r#"
tableextension 52401 "IntFooExtB" extends IntFoo
{
    internal procedure DoWork()
    begin
    end;
}
"#;
        let src_caller: &'static str = r#"
codeunit 52402 "IntCallerA"
{
    procedure Test()
    begin
    end;
}
"#;
        let app_a = make_app_id("AppA");
        let app_b = make_app_id("AppB");
        // Both the table AND the extension live in AppB (a dependency of
        // AppA) — only the extension's app differing from from_object's app
        // matters, not which app it extends.
        let unit_table = make_unit(app_b.clone(), "IntFoo.al", src_table);
        let unit_ext = make_unit(app_b, "IntFooExtB.al", src_ext);
        let unit_caller = make_unit(app_a, "IntCallerA.al", src_caller);
        let units = [unit_table, unit_ext, unit_caller];
        let graph = build_graph_multi_dep(&units, &[("AppA", "AppB")]);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let table_obj = find_obj(&graph, "IntFoo");
        let receiver = ReceiverType::Record {
            table: Some(table_obj.id.clone()),
        };
        let from_obj = find_obj(&graph, "IntCallerA");
        let (shape, routes) =
            resolve_member(&receiver, "dowork", 0, from_obj, &graph, &index, &body_map);

        assert_eq!(shape, DispatchShape::Exact);
        assert_eq!(routes.len(), 1);
        assert_eq!(
            routes[0].target,
            RouteTarget::Unresolved,
            "a cross-app `internal` TableExtension method must be excluded; \
             got {:?}",
            routes[0].target
        );
        assert_eq!(routes[0].evidence, Evidence::Unknown);
    }

    // (l) Same shape as (k) but with `local` instead of `internal` — `local`
    // is even MORE restrictive (not even visible to other objects in its OWN
    // app), so it must a fortiori be excluded cross-app.
    #[test]
    fn resolve_member_record_cross_app_extension_local_method_excluded() {
        use crate::program::resolve::receiver::ReceiverType;

        let src_table: &'static str = r#"
table 52410 "LocFoo"
{
}
"#;
        let src_ext: &'static str = r#"
tableextension 52411 "LocFooExtB" extends LocFoo
{
    local procedure DoWork()
    begin
    end;
}
"#;
        let src_caller: &'static str = r#"
codeunit 52412 "LocCallerA"
{
    procedure Test()
    begin
    end;
}
"#;
        let app_a = make_app_id("AppA");
        let app_b = make_app_id("AppB");
        let unit_table = make_unit(app_b.clone(), "LocFoo.al", src_table);
        let unit_ext = make_unit(app_b, "LocFooExtB.al", src_ext);
        let unit_caller = make_unit(app_a, "LocCallerA.al", src_caller);
        let units = [unit_table, unit_ext, unit_caller];
        let graph = build_graph_multi_dep(&units, &[("AppA", "AppB")]);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let table_obj = find_obj(&graph, "LocFoo");
        let receiver = ReceiverType::Record {
            table: Some(table_obj.id.clone()),
        };
        let from_obj = find_obj(&graph, "LocCallerA");
        let (shape, routes) =
            resolve_member(&receiver, "dowork", 0, from_obj, &graph, &index, &body_map);

        assert_eq!(shape, DispatchShape::Exact);
        assert_eq!(routes.len(), 1);
        assert_eq!(
            routes[0].target,
            RouteTarget::Unresolved,
            "a cross-app `local` TableExtension method must be excluded; got {:?}",
            routes[0].target
        );
        assert_eq!(routes[0].evidence, Evidence::Unknown);
    }

    // (m) Regression guard: a cross-app TableExtension method with NO access
    // modifier (Public, the default) must still resolve to Source — the
    // access filter must not over-exclude ordinary public cross-app methods.
    #[test]
    fn resolve_member_record_cross_app_extension_public_method_still_resolves() {
        use crate::program::resolve::receiver::ReceiverType;

        let src_table: &'static str = r#"
table 52420 "PubFoo"
{
}
"#;
        let src_ext: &'static str = r#"
tableextension 52421 "PubFooExtB" extends PubFoo
{
    procedure DoWork()
    begin
    end;
}
"#;
        let src_caller: &'static str = r#"
codeunit 52422 "PubCallerA"
{
    procedure Test()
    begin
    end;
}
"#;
        let app_a = make_app_id("AppA");
        let app_b = make_app_id("AppB");
        let unit_table = make_unit(app_b.clone(), "PubFoo.al", src_table);
        let unit_ext = make_unit(app_b, "PubFooExtB.al", src_ext);
        let unit_caller = make_unit(app_a, "PubCallerA.al", src_caller);
        let units = [unit_table, unit_ext, unit_caller];
        let graph = build_graph_multi_dep(&units, &[("AppA", "AppB")]);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let table_obj = find_obj(&graph, "PubFoo");
        let receiver = ReceiverType::Record {
            table: Some(table_obj.id.clone()),
        };
        let from_obj = find_obj(&graph, "PubCallerA");
        let (shape, routes) =
            resolve_member(&receiver, "dowork", 0, from_obj, &graph, &index, &body_map);

        assert_eq!(shape, DispatchShape::Exact);
        assert_eq!(routes.len(), 1);
        assert!(
            matches!(routes[0].target, RouteTarget::Routine(_)),
            "a Public cross-app extension method must still resolve to \
             Source — the access filter must not over-exclude; got {:?}",
            routes[0].target
        );
        assert_eq!(routes[0].evidence, Evidence::Source);
        let RouteTarget::Routine(ref rid) = routes[0].target else {
            unreachable!()
        };
        assert_eq!(rid.object.kind, ObjectKind::TableExtension);
    }

    // -----------------------------------------------------------------------
    // beyond-1B.3b Task 1 — caller-identity-aware visibility: same-app
    // `local` is OBJECT-scoped (not app-scoped), cross-app `Protected` is
    // filtered via identity `object_extends`. The full access matrix from
    // the task brief, lettered (a)-(l); (e) and (l) (cross-app `local`/
    // `internal` exclusion) were ALREADY covered by the beyond-1B.3b Task 2
    // tests above (`resolve_member_record_cross_app_extension_local_method_
    // excluded`, `resolve_member_record_cross_app_base_table_internal_
    // method_excluded`, `resolve_member_record_cross_app_extension_internal_
    // method_excluded`) and are re-asserted green by this task's refactor,
    // not re-duplicated here. See `tests/r0-corpus/ws-visibility-local-
    // protected/COMPILER_PROOF.md` for the AL-compiler semantics backing
    // every lettered case.
    // -----------------------------------------------------------------------

    // (a) SELF: an object's OWN `local procedure`, called via a `Record`
    // variable of the object's OWN type (`Rec.DoWork()` from inside the same
    // table) — `Access::Local` visible to self.
    #[test]
    fn resolve_member_record_local_self_call_resolves_to_source() {
        use crate::program::resolve::receiver::ReceiverType;

        let src_table: &'static str = r#"
table 52600 "LocSelfFoo"
{
    procedure Wrapper()
    var
        R: Record LocSelfFoo;
    begin
        R.DoWork();
    end;

    local procedure DoWork()
    begin
    end;
}
"#;
        let app_id = make_app_id("TestApp");
        let unit_table = make_unit(app_id, "LocSelfFoo.al", src_table);
        let units = [unit_table];
        let graph = build_graph(&units, None);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let table_obj = find_obj(&graph, "LocSelfFoo");
        let receiver = ReceiverType::Record {
            table: Some(table_obj.id.clone()),
        };
        // The CALLING object IS the table itself — the self case.
        let (shape, routes) =
            resolve_member(&receiver, "dowork", 0, table_obj, &graph, &index, &body_map);

        assert_eq!(shape, DispatchShape::Exact);
        assert_eq!(routes.len(), 1);
        assert!(
            matches!(routes[0].target, RouteTarget::Routine(_)),
            "an object's own `local` procedure must be visible to ITSELF; got {:?}",
            routes[0].target
        );
        assert_eq!(routes[0].evidence, Evidence::Source);
    }

    // (b) same-app DIFFERENT object: `Table Foo` (no `DoWork`) + a
    // `TableExtension FooExtB` (SAME app) declaring `local procedure
    // DoWork()`; a same-app but DIFFERENT `Codeunit CallerA` calls
    // `R.DoWork()` — AL `local` is OBJECT-scoped, so this must decline. THE
    // pre-fix bug: `object_has_visible_member_candidate`'s same-app branch
    // used to return `true` unconditionally, false-resolving this to
    // `Source` targeting `FooExtB.DoWork` (verified against unfixed code
    // during this task's TDD Step 2 — see the task report).
    #[test]
    fn resolve_member_record_same_app_extension_local_method_excluded() {
        use crate::program::resolve::receiver::ReceiverType;

        let src_table: &'static str = r#"
table 52610 "Foo"
{
}
"#;
        let src_ext: &'static str = r#"
tableextension 52611 "FooExtB" extends Foo
{
    local procedure DoWork()
    begin
    end;
}
"#;
        let src_caller: &'static str = r#"
codeunit 52612 "CallerA"
{
    procedure Test()
    var
        R: Record Foo;
    begin
        R.DoWork();
    end;
}
"#;
        let app_id = make_app_id("TestApp");
        let unit_table = make_unit(app_id.clone(), "Foo.al", src_table);
        let unit_ext = make_unit(app_id.clone(), "FooExtB.al", src_ext);
        let unit_caller = make_unit(app_id, "CallerA.al", src_caller);
        let units = [unit_table, unit_ext, unit_caller];
        let graph = build_graph(&units, None);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let table_obj = find_obj(&graph, "Foo");
        let receiver = ReceiverType::Record {
            table: Some(table_obj.id.clone()),
        };
        let from_obj = find_obj(&graph, "CallerA");
        let (shape, routes) =
            resolve_member(&receiver, "dowork", 0, from_obj, &graph, &index, &body_map);

        assert_eq!(shape, DispatchShape::Exact);
        assert_eq!(routes.len(), 1);
        assert_eq!(
            routes[0].target,
            RouteTarget::Unresolved,
            "a same-app but DIFFERENT object's `local` TableExtension method \
             must be excluded (AL `local` is OBJECT-scoped, not app-scoped); \
             got {:?}",
            routes[0].target
        );
        assert_eq!(routes[0].evidence, Evidence::Unknown);
    }

    // (c) TableExtension `local` SELF-call: the extension declares its own
    // `local procedure` and calls it via `Rec.DoWork()` where `Rec` is typed
    // to the BASE table (the only way to reference a TableExtension's own
    // members) — the calling object (the extension) equals the candidate's
    // declaring object, so this is self, not the app-scoped case.
    #[test]
    fn resolve_member_record_tableext_local_self_call_resolves_to_source() {
        use crate::program::resolve::receiver::ReceiverType;

        let src_table: &'static str = r#"
table 52620 "Foo"
{
}
"#;
        let src_ext: &'static str = r#"
tableextension 52621 "FooExtC" extends Foo
{
    procedure Wrapper()
    var
        R: Record Foo;
    begin
        R.DoWork();
    end;

    local procedure DoWork()
    begin
    end;
}
"#;
        let app_id = make_app_id("TestApp");
        let unit_table = make_unit(app_id.clone(), "Foo.al", src_table);
        let unit_ext = make_unit(app_id, "FooExtC.al", src_ext);
        let units = [unit_table, unit_ext];
        let graph = build_graph(&units, None);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let table_obj = find_obj(&graph, "Foo");
        let receiver = ReceiverType::Record {
            table: Some(table_obj.id.clone()),
        };
        let from_obj = find_obj(&graph, "FooExtC");
        let (shape, routes) =
            resolve_member(&receiver, "dowork", 0, from_obj, &graph, &index, &body_map);

        assert_eq!(shape, DispatchShape::Exact);
        assert_eq!(routes.len(), 1);
        assert!(
            matches!(routes[0].target, RouteTarget::Routine(_)),
            "a TableExtension's own `local` procedure must be visible to \
             ITSELF; got {:?}",
            routes[0].target
        );
        assert_eq!(routes[0].evidence, Evidence::Source);
        let RouteTarget::Routine(ref rid) = routes[0].target else {
            unreachable!()
        };
        assert_eq!(rid.object.kind, ObjectKind::TableExtension);
    }

    // (d) PEER-extension: `FooExtA` declares `local procedure DoWork()`;
    // sibling `FooExtB` (same base, same app) calls `R.DoWork()` — NOT self
    // (the calling object is FooExtB, the declaring object is FooExtA) — must
    // decline even though both are same-app AND both extend the same base.
    #[test]
    fn resolve_member_record_peer_extension_local_method_excluded() {
        use crate::program::resolve::receiver::ReceiverType;

        let src_table: &'static str = r#"
table 52630 "Foo"
{
}
"#;
        let src_ext_a: &'static str = r#"
tableextension 52631 "FooExtA" extends Foo
{
    local procedure DoWork()
    begin
    end;
}
"#;
        let src_ext_b: &'static str = r#"
tableextension 52632 "FooExtB" extends Foo
{
    procedure Wrapper()
    var
        R: Record Foo;
    begin
        R.DoWork();
    end;
}
"#;
        let app_id = make_app_id("TestApp");
        let unit_table = make_unit(app_id.clone(), "Foo.al", src_table);
        let unit_ext_a = make_unit(app_id.clone(), "FooExtA.al", src_ext_a);
        let unit_ext_b = make_unit(app_id, "FooExtB.al", src_ext_b);
        let units = [unit_table, unit_ext_a, unit_ext_b];
        let graph = build_graph(&units, None);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let table_obj = find_obj(&graph, "Foo");
        let receiver = ReceiverType::Record {
            table: Some(table_obj.id.clone()),
        };
        let from_obj = find_obj(&graph, "FooExtB");
        let (shape, routes) =
            resolve_member(&receiver, "dowork", 0, from_obj, &graph, &index, &body_map);

        assert_eq!(shape, DispatchShape::Exact);
        assert_eq!(routes.len(), 1);
        assert_eq!(
            routes[0].target,
            RouteTarget::Unresolved,
            "a PEER extension's `local` method must never be visible to a \
             sibling extension; got {:?}",
            routes[0].target
        );
        assert_eq!(routes[0].evidence, Evidence::Unknown);
    }

    // (f) SELF: the declaring TABLE calls its OWN `protected procedure` via
    // `Rec.P()` — trivially visible (self), symmetric with (a).
    #[test]
    fn resolve_member_record_protected_self_call_resolves_to_source() {
        use crate::program::resolve::receiver::ReceiverType;

        let src_table: &'static str = r#"
table 52640 "Bar"
{
    procedure Wrapper()
    var
        R: Record Bar;
    begin
        R.P();
    end;

    protected procedure P()
    begin
    end;
}
"#;
        let app_id = make_app_id("TestApp");
        let unit_table = make_unit(app_id, "Bar.al", src_table);
        let units = [unit_table];
        let graph = build_graph(&units, None);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let table_obj = find_obj(&graph, "Bar");
        let receiver = ReceiverType::Record {
            table: Some(table_obj.id.clone()),
        };
        let (shape, routes) =
            resolve_member(&receiver, "p", 0, table_obj, &graph, &index, &body_map);

        assert_eq!(shape, DispatchShape::Exact);
        assert_eq!(routes.len(), 1);
        assert!(
            matches!(routes[0].target, RouteTarget::Routine(_)),
            "an object's own `protected` procedure must be visible to \
             ITSELF; got {:?}",
            routes[0].target
        );
        assert_eq!(routes[0].evidence, Evidence::Source);
    }

    // (g) same-app NON-extension: `Table Bar` with `protected procedure P()`;
    // a same-app `Codeunit` — NOT an extension of Bar — calls `R.P()`. THE
    // pre-fix bug: `Access::Protected` was completely unfiltered for any
    // same-app candidate, false-resolving this to `Source` targeting
    // `Bar.P` (verified against unfixed code — see the task report).
    #[test]
    fn resolve_member_record_same_app_non_extension_protected_excluded() {
        use crate::program::resolve::receiver::ReceiverType;

        let src_table: &'static str = r#"
table 52650 "Bar"
{
    protected procedure P()
    begin
    end;
}
"#;
        let src_caller: &'static str = r#"
codeunit 52651 "CallerG"
{
    procedure Test()
    var
        R: Record Bar;
    begin
        R.P();
    end;
}
"#;
        let app_id = make_app_id("TestApp");
        let unit_table = make_unit(app_id.clone(), "Bar.al", src_table);
        let unit_caller = make_unit(app_id, "CallerG.al", src_caller);
        let units = [unit_table, unit_caller];
        let graph = build_graph(&units, None);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let table_obj = find_obj(&graph, "Bar");
        let receiver = ReceiverType::Record {
            table: Some(table_obj.id.clone()),
        };
        let from_obj = find_obj(&graph, "CallerG");
        let (shape, routes) =
            resolve_member(&receiver, "p", 0, from_obj, &graph, &index, &body_map);

        assert_eq!(shape, DispatchShape::Exact);
        assert_eq!(routes.len(), 1);
        assert_eq!(
            routes[0].target,
            RouteTarget::Unresolved,
            "a same-app NON-extension object must NOT see a table's \
             `protected` procedure (not an extension of Bar → invisible); \
             got {:?}",
            routes[0].target
        );
        assert_eq!(routes[0].evidence, Evidence::Unknown);
    }

    // (g) supplemental: same shape as above, but the same-app caller is a
    // PAGE (SourceTable = Bar) rather than a Codeunit — the brief's example
    // shape. `object_has_visible_member_candidate` gates on the CALLING
    // object's identity/kind, never the receiver's own kind, so a Page
    // caller must decline identically to a Codeunit caller.
    #[test]
    fn resolve_member_record_same_app_page_non_extension_protected_excluded() {
        use crate::program::resolve::receiver::ReceiverType;

        let src_table: &'static str = r#"
table 52652 "Bar"
{
    protected procedure P()
    begin
    end;
}
"#;
        let src_page: &'static str = r#"
page 52653 "CallerGPage"
{
    SourceTable = Bar;

    procedure Test()
    var
        R: Record Bar;
    begin
        R.P();
    end;
}
"#;
        let app_id = make_app_id("TestApp");
        let unit_table = make_unit(app_id.clone(), "Bar.al", src_table);
        let unit_page = make_unit(app_id, "CallerGPage.al", src_page);
        let units = [unit_table, unit_page];
        let graph = build_graph(&units, None);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let table_obj = find_obj(&graph, "Bar");
        let receiver = ReceiverType::Record {
            table: Some(table_obj.id.clone()),
        };
        let from_obj = find_obj(&graph, "CallerGPage");
        let (shape, routes) =
            resolve_member(&receiver, "p", 0, from_obj, &graph, &index, &body_map);

        assert_eq!(shape, DispatchShape::Exact);
        assert_eq!(routes.len(), 1);
        assert_eq!(
            routes[0].target,
            RouteTarget::Unresolved,
            "a same-app Page whose SourceTable is Bar, but which does NOT \
             extend Bar, must NOT see Bar's `protected` procedure; got {:?}",
            routes[0].target
        );
        assert_eq!(routes[0].evidence, Evidence::Unknown);
    }

    // (h) cross-app NON-extension: same shape as (g), but the caller is in a
    // DIFFERENT app (a dependency relationship, not an extension
    // relationship) — must decline a fortiori.
    #[test]
    fn resolve_member_record_cross_app_non_extension_protected_excluded() {
        use crate::program::resolve::receiver::ReceiverType;

        let src_table: &'static str = r#"
table 52660 "Bar"
{
    protected procedure P()
    begin
    end;
}
"#;
        let src_caller: &'static str = r#"
codeunit 52661 "CallerH"
{
    procedure Test()
    var
        R: Record Bar;
    begin
        R.P();
    end;
}
"#;
        let app_a = make_app_id("AppA");
        let app_b = make_app_id("AppB");
        let unit_table = make_unit(app_b, "Bar.al", src_table);
        let unit_caller = make_unit(app_a, "CallerH.al", src_caller);
        let units = [unit_table, unit_caller];
        let graph = build_graph_multi_dep(&units, &[("AppA", "AppB")]);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let table_obj = find_obj(&graph, "Bar");
        let receiver = ReceiverType::Record {
            table: Some(table_obj.id.clone()),
        };
        let from_obj = find_obj(&graph, "CallerH");
        let (shape, routes) =
            resolve_member(&receiver, "p", 0, from_obj, &graph, &index, &body_map);

        assert_eq!(shape, DispatchShape::Exact);
        assert_eq!(routes.len(), 1);
        assert_eq!(
            routes[0].target,
            RouteTarget::Unresolved,
            "a cross-app NON-extension object must NOT see a dependency \
             table's `protected` procedure; got {:?}",
            routes[0].target
        );
        assert_eq!(routes[0].evidence, Evidence::Unknown);
    }

    // (i) valid extension → base protected: a `TableExtension` on `Bar`
    // calling `Bar`'s `protected P()` — `from_object` DIRECTLY extends the
    // declaring object, kind-compatible (TableExtension→Table).
    #[test]
    fn resolve_member_record_tableext_protected_base_resolves_to_source() {
        use crate::program::resolve::receiver::ReceiverType;

        let src_table: &'static str = r#"
table 52670 "Bar"
{
    protected procedure P()
    begin
    end;
}
"#;
        let src_ext: &'static str = r#"
tableextension 52671 "BarExtI" extends Bar
{
    procedure Wrapper()
    var
        R: Record Bar;
    begin
        R.P();
    end;
}
"#;
        let app_id = make_app_id("TestApp");
        let unit_table = make_unit(app_id.clone(), "Bar.al", src_table);
        let unit_ext = make_unit(app_id, "BarExtI.al", src_ext);
        let units = [unit_table, unit_ext];
        let graph = build_graph(&units, None);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let table_obj = find_obj(&graph, "Bar");
        let receiver = ReceiverType::Record {
            table: Some(table_obj.id.clone()),
        };
        let from_obj = find_obj(&graph, "BarExtI");
        let (shape, routes) =
            resolve_member(&receiver, "p", 0, from_obj, &graph, &index, &body_map);

        assert_eq!(shape, DispatchShape::Exact);
        assert_eq!(routes.len(), 1);
        assert!(
            matches!(routes[0].target, RouteTarget::Routine(_)),
            "a TableExtension must see its BASE table's `protected` \
             procedure; got {:?}",
            routes[0].target
        );
        assert_eq!(routes[0].evidence, Evidence::Source);
        let RouteTarget::Routine(ref rid) = routes[0].target else {
            unreachable!()
        };
        assert_eq!(rid.object.kind, ObjectKind::Table);
    }

    // (i) PageExtension→base Page generalization: `ResolveIndex::object_extends`
    // is DIRECTLY exercised (not via `resolve_member`/`object_has_visible_
    // member_candidate` — Page member calls never route through
    // `resolve_in_table_scope`, which is Table/TableExtension-only; see
    // `ResolveIndex::table_extensions_of`) to prove the identity check is
    // GENERALIZED across extension kinds, not hardcoded to TableExtension —
    // the gpt/gemini round-1 convergent fix. A PageExtension of a base Page
    // is a DIRECT, kind-compatible extension relationship exactly like
    // TableExtension→Table.
    #[test]
    fn object_extends_generalizes_to_pageextension_base_page() {
        let src_page: &'static str = r#"
page 52680 "BasePage"
{
}
"#;
        let src_ext: &'static str = r#"
pageextension 52681 "BasePageExtI" extends BasePage
{
}
"#;
        let app_id = make_app_id("TestApp");
        let unit_page = make_unit(app_id.clone(), "BasePage.al", src_page);
        let unit_ext = make_unit(app_id, "BasePageExtI.al", src_ext);
        let units = [unit_page, unit_ext];
        let graph = build_graph(&units, None);
        let index = ResolveIndex::build(&graph);

        let base_page = find_obj(&graph, "BasePage");
        let page_ext = find_obj(&graph, "BasePageExtI");

        assert!(
            index.object_extends(&graph, &page_ext.id, &base_page.id),
            "a PageExtension must be recognized as DIRECTLY extending its \
             base Page — the kind-generalized object_extends contract"
        );
        // Kind-compat guard, exercised from the SAME fixture: the base Page
        // does not "extend" anything (not an extension kind at all), so the
        // reverse direction is trivially false — see the dedicated reverse
        // test below for the full base/extension asymmetry.
        assert!(!index.object_extends(&graph, &base_page.id, &page_ext.id));
    }

    // `object_extends` must be kind-compatible: a TableExtension's
    // `extends_target` resolves to a Table, never to a same-named object of
    // the WRONG kind. This fixture makes a `Table` and a `Page` share the
    // literal name `"Shared"` on purpose — a TableExtension naming `Shared`
    // in its `extends` clause must resolve against the Table, and
    // `object_extends` against the Page identity must be `false` even though
    // the raw name matches, proving the kind filter (not just identity) does
    // the work.
    #[test]
    fn object_extends_is_kind_compatible_not_name_only() {
        let src_table: &'static str = r#"
table 52690 "Shared"
{
}
"#;
        let src_page: &'static str = r#"
page 52691 "Shared"
{
}
"#;
        let src_ext: &'static str = r#"
tableextension 52692 "SharedExt" extends Shared
{
}
"#;
        let app_id = make_app_id("TestApp");
        let unit_table = make_unit(app_id.clone(), "SharedTable.al", src_table);
        let unit_page = make_unit(app_id.clone(), "SharedPage.al", src_page);
        let unit_ext = make_unit(app_id, "SharedExt.al", src_ext);
        let units = [unit_table, unit_page, unit_ext];
        let graph = build_graph(&units, None);
        let index = ResolveIndex::build(&graph);

        let table_obj = graph
            .objects
            .iter()
            .find(|o| o.id.kind == ObjectKind::Table && o.name.eq_ignore_ascii_case("Shared"))
            .expect("table Shared");
        let page_obj = graph
            .objects
            .iter()
            .find(|o| o.id.kind == ObjectKind::Page && o.name.eq_ignore_ascii_case("Shared"))
            .expect("page Shared");
        let ext_obj = find_obj(&graph, "SharedExt");

        assert!(
            index.object_extends(&graph, &ext_obj.id, &table_obj.id),
            "a TableExtension must extend the SAME-KIND (Table) object"
        );
        assert!(
            !index.object_extends(&graph, &ext_obj.id, &page_obj.id),
            "a TableExtension must NEVER be considered to extend a \
             same-NAMED but WRONG-KIND (Page) object"
        );
    }

    // `object_extends` must never be reverse: a base object does not
    // "extend" its own extension, even when the extension's `extends_target`
    // correctly names the base.
    #[test]
    fn object_extends_never_reverse() {
        let src_table: &'static str = r#"
table 52693 "RevBase"
{
}
"#;
        let src_ext: &'static str = r#"
tableextension 52694 "RevExt" extends RevBase
{
}
"#;
        let app_id = make_app_id("TestApp");
        let unit_table = make_unit(app_id.clone(), "RevBase.al", src_table);
        let unit_ext = make_unit(app_id, "RevExt.al", src_ext);
        let units = [unit_table, unit_ext];
        let graph = build_graph(&units, None);
        let index = ResolveIndex::build(&graph);

        let base_obj = find_obj(&graph, "RevBase");
        let ext_obj = find_obj(&graph, "RevExt");

        assert!(index.object_extends(&graph, &ext_obj.id, &base_obj.id));
        assert!(
            !index.object_extends(&graph, &base_obj.id, &ext_obj.id),
            "a base object must NEVER be considered to extend its own \
             extension (the relationship is not symmetric)"
        );
    }

    // `object_extends` must never treat sibling extensions of the same base
    // as extending EACH OTHER — the direct unit-level counterpart of (j)'s
    // end-to-end peer-bleed regression below.
    #[test]
    fn object_extends_never_peer() {
        let src_table: &'static str = r#"
table 52695 "PeerBase"
{
}
"#;
        let src_ext_a: &'static str = r#"
tableextension 52696 "PeerExtA" extends PeerBase
{
}
"#;
        let src_ext_b: &'static str = r#"
tableextension 52697 "PeerExtB" extends PeerBase
{
}
"#;
        let app_id = make_app_id("TestApp");
        let unit_table = make_unit(app_id.clone(), "PeerBase.al", src_table);
        let unit_ext_a = make_unit(app_id.clone(), "PeerExtA.al", src_ext_a);
        let unit_ext_b = make_unit(app_id, "PeerExtB.al", src_ext_b);
        let units = [unit_table, unit_ext_a, unit_ext_b];
        let graph = build_graph(&units, None);
        let index = ResolveIndex::build(&graph);

        let ext_a = find_obj(&graph, "PeerExtA");
        let ext_b = find_obj(&graph, "PeerExtB");

        assert!(
            !index.object_extends(&graph, &ext_b.id, &ext_a.id),
            "sibling extensions of the same base must NEVER be considered \
             to extend each other"
        );
        assert!(!index.object_extends(&graph, &ext_a.id, &ext_b.id));
    }

    // (j) PEER-extension `Protected` BLEED — the biggest latent false-`Source`
    // this task closes: `TableExtension ExtA` declares `protected P()`;
    // `TableExtension ExtB` (sibling, extends the SAME base) calls `R.P()`.
    // ExtB extends Bar, NOT ExtA — must decline. THE pre-fix bug: same-app
    // blanket-true made this false-resolve to `Source` targeting `ExtA.P`
    // (verified against unfixed code — see the task report).
    #[test]
    fn resolve_member_record_peer_extension_protected_bleed_excluded() {
        use crate::program::resolve::receiver::ReceiverType;

        let src_table: &'static str = r#"
table 52698 "Bar"
{
}
"#;
        let src_ext_a: &'static str = r#"
tableextension 52699 "BarExtA" extends Bar
{
    protected procedure P()
    begin
    end;
}
"#;
        let src_ext_b: &'static str = r#"
tableextension 52700 "BarExtB" extends Bar
{
    procedure Wrapper()
    var
        R: Record Bar;
    begin
        R.P();
    end;
}
"#;
        let app_id = make_app_id("TestApp");
        let unit_table = make_unit(app_id.clone(), "Bar.al", src_table);
        let unit_ext_a = make_unit(app_id.clone(), "BarExtA.al", src_ext_a);
        let unit_ext_b = make_unit(app_id, "BarExtB.al", src_ext_b);
        let units = [unit_table, unit_ext_a, unit_ext_b];
        let graph = build_graph(&units, None);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let table_obj = find_obj(&graph, "Bar");
        let receiver = ReceiverType::Record {
            table: Some(table_obj.id.clone()),
        };
        let from_obj = find_obj(&graph, "BarExtB");
        let (shape, routes) =
            resolve_member(&receiver, "p", 0, from_obj, &graph, &index, &body_map);

        assert_eq!(shape, DispatchShape::Exact);
        assert_eq!(routes.len(), 1);
        assert_eq!(
            routes[0].target,
            RouteTarget::Unresolved,
            "a PEER extension's `protected` method must NEVER be visible to \
             a sibling extension of the same base (ExtB extends Bar, NOT \
             ExtA) — the sibling-bleed guard; got {:?}",
            routes[0].target
        );
        assert_eq!(routes[0].evidence, Evidence::Unknown);
    }

    // (k) same-app `internal`: `Table Foo` with `internal procedure P()`; a
    // same-app but DIFFERENT `Codeunit` calls `R.P()` — `Access::Internal` is
    // APP-scoped (unaffected by this task's `local`/`protected` fixes), so
    // this must resolve to `Source` regardless of self/extension status.
    #[test]
    fn resolve_member_record_same_app_internal_method_resolves_to_source() {
        use crate::program::resolve::receiver::ReceiverType;

        let src_table: &'static str = r#"
table 52701 "Foo"
{
    internal procedure P()
    begin
    end;
}
"#;
        let src_caller: &'static str = r#"
codeunit 52702 "CallerK"
{
    procedure Test()
    var
        R: Record Foo;
    begin
        R.P();
    end;
}
"#;
        let app_id = make_app_id("TestApp");
        let unit_table = make_unit(app_id.clone(), "Foo.al", src_table);
        let unit_caller = make_unit(app_id, "CallerK.al", src_caller);
        let units = [unit_table, unit_caller];
        let graph = build_graph(&units, None);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let table_obj = find_obj(&graph, "Foo");
        let receiver = ReceiverType::Record {
            table: Some(table_obj.id.clone()),
        };
        let from_obj = find_obj(&graph, "CallerK");
        let (shape, routes) =
            resolve_member(&receiver, "p", 0, from_obj, &graph, &index, &body_map);

        assert_eq!(shape, DispatchShape::Exact);
        assert_eq!(routes.len(), 1);
        assert!(
            matches!(routes[0].target, RouteTarget::Routine(_)),
            "a same-app `internal` method must resolve to Source regardless \
             of self/extension status (Internal is app-scoped); got {:?}",
            routes[0].target
        );
        assert_eq!(routes[0].evidence, Evidence::Source);
    }

    // -----------------------------------------------------------------------
    // Phase 4 Task 1 — instance-builtin resolution for Object/Enum receivers
    // -----------------------------------------------------------------------

    // Test 1: Page Object receiver + `runmodal` → Catalog route PageInstance::runmodal
    #[test]
    fn resolve_member_page_runmodal_emits_catalog_route() {
        use crate::program::resolve::receiver::ReceiverType;

        let src_page: &'static str = r#"
page 50610 "MyPage"
{
}
"#;
        let src_caller: &'static str = r#"
codeunit 50611 "PageCaller"
{
    procedure Go()
    begin
    end;
}
"#;
        let app_id = make_app_id("TestApp");
        let unit_page = make_unit(app_id.clone(), "MyPage.al", src_page);
        let unit_caller = make_unit(app_id, "PageCaller.al", src_caller);
        let units = [unit_page, unit_caller];
        let graph = build_graph(&units, None);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let from_obj = find_obj(&graph, "PageCaller");
        let receiver = ReceiverType::Object {
            kind: ObjectKind::Page,
            name_lc: "mypage".into(),
            id: None,
        };
        let (shape, routes) = resolve_member(
            &receiver, "runmodal", 0, from_obj, &graph, &index, &body_map,
        );

        assert_eq!(shape, DispatchShape::Exact);
        assert_eq!(routes.len(), 1);
        assert!(
            matches!(routes[0].target, RouteTarget::Builtin(_)),
            "target must be Builtin; got {:?}",
            routes[0].target
        );
        assert_eq!(routes[0].evidence, Evidence::Catalog);
        assert!(
            matches!(routes[0].witness, Witness::CatalogEntry { .. }),
            "witness must be CatalogEntry; got {:?}",
            routes[0].witness
        );
        let RouteTarget::Builtin(ref bid) = routes[0].target else {
            unreachable!()
        };
        assert_eq!(bid.0, "PageInstance::runmodal");
    }

    // Test 2: Report Object receiver + `saveaspdf` → Catalog route ReportInstance::saveaspdf
    #[test]
    fn resolve_member_report_saveaspdf_emits_catalog_route() {
        use crate::program::resolve::receiver::ReceiverType;

        let src_report: &'static str = r#"
report 50612 "MyReport"
{
    dataset
    {
    }
}
"#;
        let src_caller: &'static str = r#"
codeunit 50613 "ReportCaller"
{
    procedure Go()
    begin
    end;
}
"#;
        let app_id = make_app_id("TestApp");
        let unit_report = make_unit(app_id.clone(), "MyReport.al", src_report);
        let unit_caller = make_unit(app_id, "ReportCaller.al", src_caller);
        let units = [unit_report, unit_caller];
        let graph = build_graph(&units, None);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let from_obj = find_obj(&graph, "ReportCaller");
        let receiver = ReceiverType::Object {
            kind: ObjectKind::Report,
            name_lc: "myreport".into(),
            id: None,
        };
        let (shape, routes) = resolve_member(
            &receiver,
            "saveaspdf",
            0,
            from_obj,
            &graph,
            &index,
            &body_map,
        );

        assert_eq!(shape, DispatchShape::Exact);
        assert_eq!(routes.len(), 1);
        assert!(
            matches!(routes[0].target, RouteTarget::Builtin(_)),
            "target must be Builtin; got {:?}",
            routes[0].target
        );
        assert_eq!(routes[0].evidence, Evidence::Catalog);
        assert!(
            matches!(routes[0].witness, Witness::CatalogEntry { .. }),
            "witness must be CatalogEntry; got {:?}",
            routes[0].witness
        );
        let RouteTarget::Builtin(ref bid) = routes[0].target else {
            unreachable!()
        };
        assert_eq!(bid.0, "ReportInstance::saveaspdf");
    }

    // Test 3: EnumType receiver + `asinteger` → Catalog route Enum::asinteger
    #[test]
    fn resolve_member_enum_asinteger_emits_catalog_route() {
        use crate::program::resolve::receiver::ReceiverType;

        let (graph, index, body_map, from_obj) = minimal_resolve_member_fixtures();

        let receiver = ReceiverType::EnumType {
            name_lc: "myenum".into(),
        };
        let (shape, routes) = resolve_member(
            &receiver,
            "asinteger",
            0,
            &from_obj,
            &graph,
            &index,
            &body_map,
        );

        assert_eq!(shape, DispatchShape::Exact);
        assert_eq!(routes.len(), 1);
        assert!(
            matches!(routes[0].target, RouteTarget::Builtin(_)),
            "target must be Builtin; got {:?}",
            routes[0].target
        );
        assert_eq!(routes[0].evidence, Evidence::Catalog);
        assert!(
            matches!(routes[0].witness, Witness::CatalogEntry { .. }),
            "witness must be CatalogEntry; got {:?}",
            routes[0].witness
        );
        let RouteTarget::Builtin(ref bid) = routes[0].target else {
            unreachable!()
        };
        assert_eq!(bid.0, "Enum::asinteger");
    }

    // Test 4: EnumType receiver + `frominteger` → Catalog route Enum::frominteger
    #[test]
    fn resolve_member_enum_frominteger_emits_catalog_route() {
        use crate::program::resolve::receiver::ReceiverType;

        let (graph, index, body_map, from_obj) = minimal_resolve_member_fixtures();

        let receiver = ReceiverType::EnumType {
            name_lc: "myenum".into(),
        };
        let (shape, routes) = resolve_member(
            &receiver,
            "frominteger",
            0,
            &from_obj,
            &graph,
            &index,
            &body_map,
        );

        assert_eq!(shape, DispatchShape::Exact);
        assert_eq!(routes.len(), 1);
        assert!(
            matches!(routes[0].target, RouteTarget::Builtin(_)),
            "target must be Builtin; got {:?}",
            routes[0].target
        );
        assert_eq!(routes[0].evidence, Evidence::Catalog);
        let RouteTarget::Builtin(ref bid) = routes[0].target else {
            unreachable!()
        };
        assert_eq!(bid.0, "Enum::frominteger");
    }

    // Test 5: EnumType receiver + unknown method → Unknown
    #[test]
    fn resolve_member_enum_unknown_method_emits_unknown() {
        use crate::program::resolve::receiver::ReceiverType;

        let (graph, index, body_map, from_obj) = minimal_resolve_member_fixtures();

        let receiver = ReceiverType::EnumType {
            name_lc: "myenum".into(),
        };
        let (shape, routes) = resolve_member(
            &receiver,
            "nosuchmethod",
            0,
            &from_obj,
            &graph,
            &index,
            &body_map,
        );

        assert_eq!(shape, DispatchShape::Exact);
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].target, RouteTarget::Unresolved);
        assert_eq!(routes[0].evidence, Evidence::Unknown);
        assert_eq!(routes[0].witness, Witness::None);
    }

    // Test 6: Page Object receiver + `setrecord` → Unknown (metadata-sensitive exclusion)
    #[test]
    fn resolve_member_page_setrecord_emits_unknown_metadata_sensitive() {
        use crate::program::resolve::receiver::ReceiverType;

        let src_page: &'static str = r#"
page 50614 "AnotherPage"
{
}
"#;
        let src_caller: &'static str = r#"
codeunit 50615 "AnotherPageCaller"
{
    procedure Go()
    begin
    end;
}
"#;
        let app_id = make_app_id("TestApp");
        let unit_page = make_unit(app_id.clone(), "AnotherPage.al", src_page);
        let unit_caller = make_unit(app_id, "AnotherPageCaller.al", src_caller);
        let units = [unit_page, unit_caller];
        let graph = build_graph(&units, None);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let from_obj = find_obj(&graph, "AnotherPageCaller");
        let receiver = ReceiverType::Object {
            kind: ObjectKind::Page,
            name_lc: "anotherpage".into(),
            id: None,
        };
        let (shape, routes) = resolve_member(
            &receiver,
            "setrecord",
            1,
            from_obj,
            &graph,
            &index,
            &body_map,
        );

        assert_eq!(shape, DispatchShape::Exact);
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].target, RouteTarget::Unresolved);
        assert_eq!(routes[0].evidence, Evidence::Unknown);
        assert_eq!(routes[0].witness, Witness::None);
    }

    // Test 7: Page Object receiver + declared proc (shadows catalog) → Source route
    #[test]
    fn resolve_member_page_declared_proc_shadows_catalog() {
        use crate::program::resolve::receiver::ReceiverType;

        // This page declares its own RunModal — should shadow the PageInstance catalog entry.
        let src_page: &'static str = r#"
page 50616 "PageWithRunModal"
{
    procedure RunModal()
    begin
    end;
}
"#;
        let src_caller: &'static str = r#"
codeunit 50617 "ShadowCaller"
{
    procedure Go()
    begin
    end;
}
"#;
        let app_id = make_app_id("TestApp");
        let unit_page = make_unit(app_id.clone(), "PageWithRunModal.al", src_page);
        let unit_caller = make_unit(app_id, "ShadowCaller.al", src_caller);
        let units = [unit_page, unit_caller];
        let graph = build_graph(&units, None);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let from_obj = find_obj(&graph, "ShadowCaller");
        let receiver = ReceiverType::Object {
            kind: ObjectKind::Page,
            name_lc: "pagewithrunmodal".into(),
            id: None,
        };
        let (shape, routes) = resolve_member(
            &receiver, "runmodal", 0, from_obj, &graph, &index, &body_map,
        );

        assert_eq!(shape, DispatchShape::Exact);
        assert_eq!(routes.len(), 1);
        assert!(
            matches!(routes[0].target, RouteTarget::Routine(_)),
            "declared proc must shadow catalog; target must be Routine, got {:?}",
            routes[0].target
        );
        assert_eq!(routes[0].evidence, Evidence::Source);
        assert!(matches!(routes[0].witness, Witness::SourceSpan { .. }));
    }

    // -----------------------------------------------------------------------
    // Phase 4 Task 2 — Interface Polymorphic fan-out tests
    // -----------------------------------------------------------------------

    /// Two codeunits both implementing IFoo, each with a unique Bar() (arity 0).
    /// Expect: Polymorphic shape, two Routine routes (Source evidence), each
    /// passing `interface_route_applicable`.
    #[test]
    fn resolve_member_interface_two_implementers_emits_two_routine_routes() {
        use crate::program::resolve::applicability::interface_route_applicable;
        use crate::program::resolve::receiver::ReceiverType;

        // Two implementers + a caller; all in one file for simplicity.
        let src: &'static str = r#"
codeunit 51300 "IFooImpl1" implements IFoo
{
    procedure Bar()
    begin
    end;
}

codeunit 51301 "IFooImpl2" implements IFoo
{
    procedure Bar()
    begin
    end;
}

codeunit 51399 "IfaceCaller1"
{
    procedure Trigger()
    begin
    end;
}
"#;
        let app_id = make_app_id("TestApp");
        let unit = make_unit(app_id, "IfaceTwo.al", src);
        let units = [unit];
        let graph = build_graph(&units, None);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let from_obj = find_obj(&graph, "IfaceCaller1");
        let receiver = ReceiverType::Interface {
            name_lc: "ifoo".into(),
        };
        let (shape, routes) =
            resolve_member(&receiver, "bar", 0, from_obj, &graph, &index, &body_map);

        assert_eq!(
            shape,
            DispatchShape::Polymorphic,
            "shape must be Polymorphic"
        );
        assert_eq!(
            routes.len(),
            2,
            "two implementers → two routes; got {:?}",
            routes
        );

        // Both routes must be Routine (unique Bar() in each implementer).
        for r in &routes {
            assert!(
                matches!(r.target, RouteTarget::Routine(_)),
                "each implementer's Bar() route must be Routine; got {:?}",
                r.target
            );
            assert_eq!(r.evidence, Evidence::Source, "evidence must be Source");
        }

        // Each Routine must pass interface_route_applicable.
        for r in &routes {
            let RouteTarget::Routine(ref rid) = r.target else {
                unreachable!()
            };
            assert!(
                interface_route_applicable("ifoo", "bar", 0, rid, &graph, &index),
                "Routine route must pass interface_route_applicable; rid={:?}",
                rid
            );
        }
    }

    /// ONE implementer → exactly 1 Routine route.
    #[test]
    fn resolve_member_interface_one_implementer_emits_one_route() {
        use crate::program::resolve::receiver::ReceiverType;

        let src: &'static str = r#"
codeunit 51400 "OnlyImpl" implements IFoo
{
    procedure Bar()
    begin
    end;
}

codeunit 51499 "IfaceCaller2"
{
    procedure Trigger()
    begin
    end;
}
"#;
        let app_id = make_app_id("TestApp");
        let unit = make_unit(app_id, "IfaceOne.al", src);
        let units = [unit];
        let graph = build_graph(&units, None);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let from_obj = find_obj(&graph, "IfaceCaller2");
        let receiver = ReceiverType::Interface {
            name_lc: "ifoo".into(),
        };
        let (shape, routes) =
            resolve_member(&receiver, "bar", 0, from_obj, &graph, &index, &body_map);

        assert_eq!(shape, DispatchShape::Polymorphic);
        assert_eq!(
            routes.len(),
            1,
            "one implementer → one route; got {:?}",
            routes
        );
        assert!(
            matches!(routes[0].target, RouteTarget::Routine(_)),
            "must be Routine route; got {:?}",
            routes[0].target
        );
        assert_eq!(routes[0].evidence, Evidence::Source);
    }

    /// ZERO implementers → empty route vec (HonestEmpty).
    #[test]
    fn resolve_member_interface_zero_implementers_emits_empty_routes() {
        use crate::program::resolve::receiver::ReceiverType;

        // NobodyImpl does NOT implement IFoo.
        let src: &'static str = r#"
codeunit 51500 "NobodyImpl"
{
    procedure Bar()
    begin
    end;
}

codeunit 51599 "IfaceCaller3"
{
    procedure Trigger()
    begin
    end;
}
"#;
        let app_id = make_app_id("TestApp");
        let unit = make_unit(app_id, "IfaceZero.al", src);
        let units = [unit];
        let graph = build_graph(&units, None);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let from_obj = find_obj(&graph, "IfaceCaller3");
        let receiver = ReceiverType::Interface {
            name_lc: "ifoo".into(),
        };
        let (shape, routes) =
            resolve_member(&receiver, "bar", 0, from_obj, &graph, &index, &body_map);

        assert_eq!(
            shape,
            DispatchShape::Polymorphic,
            "shape must be Polymorphic even with zero implementers"
        );
        assert!(
            routes.is_empty(),
            "zero implementers → empty routes (HonestEmpty); got {:?}",
            routes
        );
    }

    /// A resolves Bar(), B has Bar(x: Integer) (arity mismatch for arity-0 call) →
    /// `[Routine(A), Unresolved(B)]`.  B MUST emit Unresolved and NOT be dropped
    /// (Rule 1: no reachability black hole).
    #[test]
    fn resolve_member_interface_failing_implementer_emits_unresolved_not_dropped() {
        use crate::program::resolve::receiver::ReceiverType;

        // FooImpl1 has Bar() (0 params) — unique, resolves OK.
        // FooImpl2 has Bar(x: Integer) (1 param) — arity mismatch for Bar(0) → Unresolved.
        let src: &'static str = r#"
codeunit 51600 "FooImpl1" implements IFoo
{
    procedure Bar()
    begin
    end;
}

codeunit 51601 "FooImpl2" implements IFoo
{
    procedure Bar(x: Integer)
    begin
    end;
}

codeunit 51699 "IfaceCaller4"
{
    procedure Trigger()
    begin
    end;
}
"#;
        let app_id = make_app_id("TestApp");
        let unit = make_unit(app_id, "IfaceFail.al", src);
        let units = [unit];
        let graph = build_graph(&units, None);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let from_obj = find_obj(&graph, "IfaceCaller4");
        let receiver = ReceiverType::Interface {
            name_lc: "ifoo".into(),
        };
        let (shape, routes) =
            resolve_member(&receiver, "bar", 0, from_obj, &graph, &index, &body_map);

        assert_eq!(shape, DispatchShape::Polymorphic);
        assert_eq!(
            routes.len(),
            2,
            "two implementers → two routes (B must NOT be silently dropped); got {:?}",
            routes
        );

        // One route must be Routine (A), one must be Unresolved (B).
        let routine_count = routes
            .iter()
            .filter(|r| matches!(r.target, RouteTarget::Routine(_)))
            .count();
        let unresolved_count = routes
            .iter()
            .filter(|r| r.target == RouteTarget::Unresolved)
            .count();
        assert_eq!(
            routine_count, 1,
            "exactly one Routine route (A); got {:?}",
            routes
        );
        assert_eq!(
            unresolved_count, 1,
            "exactly one Unresolved route (B, NOT dropped); got {:?}",
            routes
        );

        // The Unresolved route must have Unknown evidence and None witness.
        let unresolved_route = routes
            .iter()
            .find(|r| r.target == RouteTarget::Unresolved)
            .unwrap();
        assert_eq!(unresolved_route.evidence, Evidence::Unknown);
        assert_eq!(unresolved_route.witness, Witness::None);
    }

    /// SymbolOnly interface implementer → `AbiSymbol` route (Opaque evidence, not Unresolved).
    ///
    /// Validates the Phase-4 Task-2 fix: when a cross-app `.app` dependency implements an
    /// interface (SymbolOnly tier, no source body available), `resolve_member` must emit
    /// `RouteTarget::AbiSymbol` (Opaque boundary), not `Unresolved`.
    ///
    /// This test also directly validates the per-route gate predicate used in
    /// `run_member_resolution_harness` FreshOnly branch: `RouteTarget::AbiSymbol { .. }`
    /// must be classified as PASS (not `unverified_extra`).  The gate-gap fix adds
    /// `AbiSymbol { .. } => true` alongside the existing `Unresolved => true` arm.
    #[test]
    fn resolve_member_interface_symbol_only_implementer_emits_abi_symbol_route() {
        use crate::program::resolve::receiver::ReceiverType;

        // AppBSym provides a SymbolOnly codeunit "DepImpl" implementing IFoo.
        // Parsed from source to populate graph+index nodes, but the BodyMap is built
        // WITHOUT this unit (empty parsed slice) — mirrors real SymbolOnly loading from
        // .app SymbolReference where bodies are unavailable.
        let src_caller: &'static str = r#"
codeunit 51800 "IfaceCallerSym"
{
    procedure Trigger()
    begin
    end;
}
"#;
        let src_dep: &'static str = r#"
codeunit 51801 "DepImpl" implements IFoo
{
    procedure Bar()
    begin
    end;
}
"#;
        let app_a_id = make_app_id("AppA");
        let app_b_id = make_app_id("AppBSym");

        let unit_caller = make_unit(app_a_id.clone(), "IfaceCallerSym.al", src_caller);
        // SymbolOnly dep unit: parsed to extract graph/index nodes with SymbolOnly tier.
        let unit_dep = ParsedUnit {
            app: app_b_id.clone(),
            files: vec![ParsedFile {
                virtual_path: "DepImpl.al".to_string(),
                file: al_syntax::parse(src_dep),
                provenance: Provenance {
                    app: app_b_id,
                    tier: TrustTier::SymbolOnly,
                    content_hash: String::new(),
                },
                text: src_dep.to_string(),
            }],
        };

        let all_units = [unit_caller, unit_dep];
        let graph = build_graph(&all_units, Some(("AppA", "AppBSym")));
        let index = ResolveIndex::build(&graph);
        // BodyMap: empty — SymbolOnly routines have no parsed body in production.
        // A BodyMap miss on a SymbolOnly-tier routine triggers the AbiSymbol path
        // in `make_routine_route`.
        let body_map = BodyMap::build(&graph, &[]);

        let from_obj = find_obj(&graph, "IfaceCallerSym");
        let receiver = ReceiverType::Interface {
            name_lc: "ifoo".into(),
        };
        let (shape, routes) =
            resolve_member(&receiver, "bar", 0, from_obj, &graph, &index, &body_map);

        assert_eq!(shape, DispatchShape::Polymorphic);
        assert_eq!(
            routes.len(),
            1,
            "one SymbolOnly implementer → one route; got {:?}",
            routes
        );

        // Route must be AbiSymbol (Opaque boundary), NOT Unresolved or Routine.
        assert!(
            matches!(routes[0].target, RouteTarget::AbiSymbol { .. }),
            "SymbolOnly implementer must emit AbiSymbol (Opaque boundary); got {:?}",
            routes[0].target
        );
        assert_eq!(
            routes[0].evidence,
            Evidence::Opaque,
            "AbiSymbol route must carry Opaque evidence"
        );
        assert!(
            matches!(routes[0].witness, Witness::AbiSymbol { .. }),
            "AbiSymbol route must carry AbiSymbol witness; got {:?}",
            routes[0].witness
        );

        // Validate the per-route gate predicate (mirror of the fix in differential.rs).
        // The FreshOnly `is_interface_route` branch in `run_member_resolution_harness`
        // classifies each route as: Unresolved → PASS, Routine → applicability check,
        // AbiSymbol → PASS (the fix), _ → FAIL.
        let gate_pass = routes.iter().all(|r| match &r.target {
            RouteTarget::Unresolved => true,
            RouteTarget::AbiSymbol { .. } => true, // gate fix: was `_ => false` before
            _ => false,
        });
        assert!(
            gate_pass,
            "AbiSymbol route must pass the interface FreshOnly gate (not unverified_extra)"
        );
    }

    // Test 8: Page Object receiver + method not in catalog and not declared → Unknown
    #[test]
    fn resolve_member_page_method_not_in_catalog_emits_unknown() {
        use crate::program::resolve::receiver::ReceiverType;

        let src_page: &'static str = r#"
page 50618 "EmptyPage"
{
}
"#;
        let src_caller: &'static str = r#"
codeunit 50619 "EmptyPageCaller"
{
    procedure Go()
    begin
    end;
}
"#;
        let app_id = make_app_id("TestApp");
        let unit_page = make_unit(app_id.clone(), "EmptyPage.al", src_page);
        let unit_caller = make_unit(app_id, "EmptyPageCaller.al", src_caller);
        let units = [unit_page, unit_caller];
        let graph = build_graph(&units, None);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let from_obj = find_obj(&graph, "EmptyPageCaller");
        let receiver = ReceiverType::Object {
            kind: ObjectKind::Page,
            name_lc: "emptypage".into(),
            id: None,
        };
        let (shape, routes) = resolve_member(
            &receiver,
            "nosuchpagemethod",
            0,
            from_obj,
            &graph,
            &index,
            &body_map,
        );

        assert_eq!(shape, DispatchShape::Exact);
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].target, RouteTarget::Unresolved);
        assert_eq!(routes[0].evidence, Evidence::Unknown);
        assert_eq!(routes[0].witness, Witness::None);
    }

    // -----------------------------------------------------------------------
    // Phase 4b Task 3 — emit_event_flow_edges tests
    // -----------------------------------------------------------------------

    /// Build a publisher + Manual subscriber graph from real AL source.
    /// Returns `(graph, units)` with both objects in one `"TestApp"` app.
    fn build_event_flow_fixture_manual() -> (ProgramGraph, Vec<ParsedUnit>) {
        let pub_src: &'static str = r#"
codeunit 50700 "EvtPub"
{
    [IntegrationEvent(false, false)]
    procedure OnAfterX()
    begin
    end;
}
"#;
        let sub_src: &'static str = r#"
codeunit 50701 "EvtManualSub"
{
    EventSubscriberInstance = Manual;

    [EventSubscriber(ObjectType::Codeunit, Codeunit::"EvtPub", 'OnAfterX', '', false, false)]
    local procedure OnAfterXHandler()
    begin
    end;
}
"#;
        let app_id = make_app_id("TestApp");
        let unit_pub = make_unit(app_id.clone(), "EvtPub.al", pub_src);
        let unit_sub = make_unit(app_id, "EvtManualSub.al", sub_src);
        let units = vec![unit_pub, unit_sub];
        let graph = build_graph(&units, None);
        (graph, units)
    }

    // (a) publisher OnAfterX + Manual subscriber → ONE EventFlow Edge with ManualBinding
    #[test]
    fn event_flow_manual_subscriber_emits_correct_edge() {
        let (graph, units) = build_event_flow_fixture_manual();
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let edges = emit_event_flow_edges(&graph, &index, &body_map);

        // Must produce exactly ONE EventFlow edge (for OnAfterX publisher).
        let event_edges: Vec<&Edge> = edges
            .iter()
            .filter(|e| e.kind == EdgeKind::EventFlow)
            .collect();
        assert_eq!(event_edges.len(), 1, "expected exactly one EventFlow edge");

        let e = event_edges[0];

        // Publisher is the `from`.
        let pub_obj = graph
            .objects
            .iter()
            .find(|o| o.name == "EvtPub")
            .expect("EvtPub object");
        let expected_pub_rid = graph
            .routines
            .iter()
            .find(|r| r.id.object == pub_obj.id && r.id.name_lc == "onafterx")
            .expect("OnAfterX publisher routine")
            .id
            .clone();
        assert_eq!(e.from, expected_pub_rid, "edge must be from the publisher");

        // Edge shape.
        assert_eq!(e.kind, EdgeKind::EventFlow);
        assert_eq!(e.shape, DispatchShape::Multicast);
        assert_eq!(
            e.completeness,
            SetCompleteness::Partial {
                reason: OpenWorldReason::ReverseDependentSubscribers
            }
        );

        // Exactly one route — the Manual subscriber.
        assert_eq!(e.routes.len(), 1, "one subscriber → one route");
        let r = &e.routes[0];

        // Route target is the subscriber Routine.
        let sub_obj = graph
            .objects
            .iter()
            .find(|o| o.name == "EvtManualSub")
            .expect("EvtManualSub object");
        let expected_sub_rid = graph
            .routines
            .iter()
            .find(|r| r.id.object == sub_obj.id && r.id.name_lc == "onafterxhandler")
            .expect("OnAfterXHandler subscriber routine")
            .id
            .clone();
        assert_eq!(
            r.target,
            RouteTarget::Routine(expected_sub_rid),
            "route target must be the subscriber"
        );

        // Route has ManualBinding condition.
        assert!(
            r.conditions.contains(&Condition::ManualBinding),
            "Manual subscriber must carry ManualBinding condition; got {:?}",
            r.conditions
        );

        // Subscriber is in the body_map → Source evidence + SourceSpan witness.
        assert_eq!(r.evidence, Evidence::Source);
        assert!(
            matches!(r.witness, Witness::SourceSpan { .. }),
            "witness must be SourceSpan (subscriber in body_map); got {:?}",
            r.witness
        );
    }

    // (b) Manual route NOT in default_reachable_routes, IS in may_reachable_routes
    #[test]
    fn event_flow_manual_route_excluded_from_default_reachable() {
        let (graph, units) = build_event_flow_fixture_manual();
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let edges = emit_event_flow_edges(&graph, &index, &body_map);
        let e = edges
            .iter()
            .find(|e| e.kind == EdgeKind::EventFlow)
            .expect("EventFlow edge");

        // (b) Manual route must NOT appear in default_reachable_routes (reachability contract).
        let default_routes: Vec<_> = e.default_reachable_routes().collect();
        assert!(
            default_routes.is_empty(),
            "ManualBinding route must NOT be in default_reachable_routes; got {:?}",
            default_routes
        );

        // But it MUST appear in may_reachable_routes.
        let may_routes: Vec<_> = e.may_reachable_routes().collect();
        assert_eq!(
            may_routes.len(),
            1,
            "ManualBinding route MUST be in may_reachable_routes"
        );
    }

    // (c) publisher with ZERO subscribers → empty routes → HonestEmpty
    #[test]
    fn event_flow_zero_subscribers_honest_empty() {
        let pub_src: &'static str = r#"
codeunit 50702 "NoSubPub"
{
    [IntegrationEvent(false, false)]
    procedure OnAfterY()
    begin
    end;
}
"#;
        let app_id = make_app_id("TestApp");
        let unit_pub = make_unit(app_id, "NoSubPub.al", pub_src);
        let units = vec![unit_pub];
        let graph = build_graph(&units, None);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let edges = emit_event_flow_edges(&graph, &index, &body_map);

        let event_edges: Vec<&Edge> = edges
            .iter()
            .filter(|e| e.kind == EdgeKind::EventFlow)
            .collect();
        assert_eq!(
            event_edges.len(),
            1,
            "publisher must always produce an EventFlow edge"
        );

        let e = event_edges[0];
        assert!(
            e.routes.is_empty(),
            "zero subscribers → empty routes; got {:?}",
            e.routes
        );
        assert_eq!(
            classify_obligation(e),
            ObligationOutcome::HonestEmpty,
            "empty Multicast + Partial → HonestEmpty"
        );
    }

    // (d) non-Manual subscriber → route IS in default_reachable_routes
    #[test]
    fn event_flow_non_manual_subscriber_default_reachable() {
        let pub_src: &'static str = r#"
codeunit 50703 "DefaultPub"
{
    [IntegrationEvent(false, false)]
    procedure OnAfterZ()
    begin
    end;
}
"#;
        let sub_src: &'static str = r#"
codeunit 50704 "DefaultSub"
{
    [EventSubscriber(ObjectType::Codeunit, Codeunit::"DefaultPub", 'OnAfterZ', '', false, false)]
    local procedure OnAfterZHandler()
    begin
    end;
}
"#;
        let app_id = make_app_id("TestApp");
        let unit_pub = make_unit(app_id.clone(), "DefaultPub.al", pub_src);
        let unit_sub = make_unit(app_id, "DefaultSub.al", sub_src);
        let units = vec![unit_pub, unit_sub];
        let graph = build_graph(&units, None);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let edges = emit_event_flow_edges(&graph, &index, &body_map);
        let e = edges
            .iter()
            .find(|e| e.kind == EdgeKind::EventFlow)
            .expect("EventFlow edge");

        // Non-Manual subscriber: route must be in default_reachable_routes.
        let default_routes: Vec<_> = e.default_reachable_routes().collect();
        assert_eq!(
            default_routes.len(),
            1,
            "non-Manual route must be in default_reachable_routes"
        );
        assert!(
            !default_routes[0]
                .conditions
                .contains(&Condition::ManualBinding),
            "non-Manual route must not have ManualBinding"
        );
        assert_eq!(default_routes[0].evidence, Evidence::Source);
    }

    // (e) determinism: two calls with same inputs produce identical output
    #[test]
    fn event_flow_emission_is_deterministic() {
        let (graph, units) = build_event_flow_fixture_manual();
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let edges1 = emit_event_flow_edges(&graph, &index, &body_map);
        let edges2 = emit_event_flow_edges(&graph, &index, &body_map);

        assert_eq!(
            edges1, edges2,
            "emit_event_flow_edges must be deterministic across calls"
        );
    }

    // -----------------------------------------------------------------------
    // ABI event-kind threading: Task 1B.3a fix
    //
    // Verifies that `make_routine_route` threads `abi_routine_kind`/`abi_event_kind`
    // from the `RoutineNode` into the `AbiRoutineKey` instead of hardcoding
    // `Procedure`/`None` for every SymbolOnly dep routine.
    // -----------------------------------------------------------------------

    /// Build a graph with a workspace codeunit + a SymbolOnly dep codeunit that
    /// has one event-publisher routine and one regular procedure.
    fn build_abi_kind_fixture() -> (ProgramGraph, Vec<ParsedUnit>) {
        let ws_id = make_app_id("WS");
        let dep_id = make_app_id("DepApp");

        let src: &'static str = r#"
codeunit 50000 "Caller"
{
    procedure Run()
    begin
    end;
}
"#;
        let unit = make_unit(ws_id.clone(), "Caller.al", src);
        let units = vec![unit];

        let mut apps = AppRegistry::default();
        let ws_ref = apps.intern(&ws_id);
        let dep_ref = apps.intern(&dep_id);

        let mut objects: Vec<ObjectNode> = Vec::new();
        let mut routines: Vec<RoutineNode> = Vec::new();

        // Extract workspace nodes from source.
        for pf in &units[0].files {
            extract_nodes(
                ws_ref,
                &pf.file,
                pf.provenance.tier,
                &mut objects,
                &mut routines,
            );
        }

        // SymbolOnly dep codeunit 50100 "DepCU".
        let dep_obj_id = ObjectNodeId {
            app: dep_ref,
            kind: ObjectKind::Codeunit,
            key: ObjKey::Id(50100),
        };
        objects.push(ObjectNode {
            id: dep_obj_id.clone(),
            name: "DepCU".into(),
            declared_id: Some(50100),
            extends_target: None,
            implements: vec![],
            tier: TrustTier::SymbolOnly,
            source_table: None,
            table_no: None,
            source_table_temporary: false,
            page_controls: vec![],
        });

        // Event-publisher routine: abi_routine_kind=EventPublisher, abi_event_kind=Integration.
        routines.push(RoutineNode {
            id: RoutineNodeId {
                object: dep_obj_id.clone(),
                name_lc: "ondepevent".into(),
                enclosing_member_lc: None,
                params_count: 1,
                sig_fp: 0,
            },
            name: "OnDepEvent".into(),
            is_trigger: false,
            access: Access::Public,
            tier: TrustTier::SymbolOnly,
            event_subscribers: vec![],
            subscriber_instance_manual: false,
            publisher_kind: None,
            abi_routine_kind: Some(AbiRoutineKind::EventPublisher),
            abi_event_kind: Some(AbiEventKind::Integration),
            param_sig_key: String::new(),
        });

        // Regular procedure: abi_routine_kind=Procedure, abi_event_kind=None.
        routines.push(RoutineNode {
            id: RoutineNodeId {
                object: dep_obj_id.clone(),
                name_lc: "dowork".into(),
                enclosing_member_lc: None,
                params_count: 0,
                sig_fp: 0,
            },
            name: "DoWork".into(),
            is_trigger: false,
            access: Access::Public,
            tier: TrustTier::SymbolOnly,
            event_subscribers: vec![],
            subscriber_instance_manual: false,
            publisher_kind: None,
            abi_routine_kind: Some(AbiRoutineKind::Procedure),
            abi_event_kind: Some(AbiEventKind::None),
            param_sig_key: String::new(),
        });

        objects.sort_by(|a, b| a.id.cmp(&b.id));
        routines.sort_by(|a, b| a.id.cmp(&b.id));

        let mut topology = DependencyGraph::default();
        topology.add_dependency(ws_ref, dep_ref);

        let obj_index = ObjectIndex::build(&objects);
        let graph = ProgramGraph {
            apps,
            topology,
            objects,
            routines,
            obj_index,
        };
        (graph, units)
    }

    /// A SymbolOnly dep event-publisher resolved via `resolve_member` (Object
    /// receiver) must carry `AbiRoutineKey.routine_kind == EventPublisher` and
    /// `event_kind == Integration` — NOT the hardcoded `Procedure/None` that
    /// existed before Task 1B.3a.
    ///
    /// A SymbolOnly dep regular procedure must carry `Procedure/None` (unchanged).
    #[test]
    fn symbolonly_event_publisher_route_carries_correct_abi_kind() {
        let (graph, units) = build_abi_kind_fixture();
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);
        let from_obj = find_obj(&graph, "Caller");

        // --- event publisher: must be EventPublisher / Integration ---
        let (shape, routes) = resolve_member(
            &ReceiverType::Object {
                kind: ObjectKind::Codeunit,
                name_lc: "depcu".into(),
                id: None,
            },
            "ondepevent",
            1,
            from_obj,
            &graph,
            &index,
            &body_map,
        );
        assert_eq!(shape, DispatchShape::Exact);
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].evidence, Evidence::Opaque);
        let RouteTarget::AbiSymbol { ref key } = routes[0].target else {
            panic!(
                "expected AbiSymbol target for event publisher, got {:?}",
                routes[0].target
            );
        };
        assert_eq!(
            key.routine_kind,
            AbiRoutineKind::EventPublisher,
            "dep event-publisher must carry EventPublisher kind in AbiRoutineKey"
        );
        assert_eq!(
            key.event_kind,
            AbiEventKind::Integration,
            "integration event must carry Integration event_kind in AbiRoutineKey"
        );

        // --- regular procedure: must be Procedure / None (unchanged) ---
        let (shape2, routes2) = resolve_member(
            &ReceiverType::Object {
                kind: ObjectKind::Codeunit,
                name_lc: "depcu".into(),
                id: None,
            },
            "dowork",
            0,
            from_obj,
            &graph,
            &index,
            &body_map,
        );
        assert_eq!(shape2, DispatchShape::Exact);
        assert_eq!(routes2.len(), 1);
        assert_eq!(routes2[0].evidence, Evidence::Opaque);
        let RouteTarget::AbiSymbol { key: ref key2 } = routes2[0].target else {
            panic!(
                "expected AbiSymbol target for procedure, got {:?}",
                routes2[0].target
            );
        };
        assert_eq!(
            key2.routine_kind,
            AbiRoutineKind::Procedure,
            "regular dep procedure must carry Procedure kind"
        );
        assert_eq!(
            key2.event_kind,
            AbiEventKind::None,
            "regular dep procedure must carry None event_kind"
        );
    }
}
