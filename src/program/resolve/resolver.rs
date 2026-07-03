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

use crate::program::abi_ingest::object_kind_from_abi_type;
use crate::program::graph::ProgramGraph;
use crate::program::node::{AppRef, ObjKey, ObjectNodeId, RoutineNodeId};
use crate::program::node_extract::{Access, ObjectNode, RoutineNode};
use crate::program::resolve::body_map::BodyMap;
use crate::program::resolve::builtins::{catalog_version, global_builtin_id};
use crate::program::resolve::edge::{
    AbiEventKind, AbiRoutineKey, AbiRoutineKind, BuiltinId, CanonicalSpan, DispatchShape, Edge,
    EdgeKind, Evidence, OpenWorldReason, Route, RouteTarget, SetCompleteness, SiteId, SourcePos,
    UnknownReason, Witness, callee_fp,
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
        unresolved_route(UnknownReason::IndexIntegrationGap)
    }
}

/// Build an `Unresolved`/`Unknown(reason)` route — the shared constructor for
/// every genuine resolution-failure route (Task 3: the `reason` argument is
/// REQUIRED, so every call site is forced to supply a diagnostic
/// [`UnknownReason`]).
fn unresolved_route(reason: UnknownReason) -> Route {
    Route {
        target: RouteTarget::Unresolved,
        evidence: Evidence::Unknown(reason),
        conditions: vec![],
        witness: Witness::None,
    }
}

/// Whether `rid` is currently marked [`RoutineNode::abi_overload_collapsed`]
/// on `graph` — the COLLAPSE-MARKER GUARD shared by every `make_routine_
/// route` call site (Task 2 review fix). Before this fix, only TWO of the
/// SIX sites that ultimately build a route for a specific `RoutineNodeId`
/// consulted the marker: [`resolve_in_object`]'s single-visible-candidate
/// arm (the PLAIN-DISPATCH MARKER GUARD, Task 2 round-2) and
/// [`routine_node_for_type_query`] (the CHAIN-type-query guard, Task 3
/// review fix). The other FOUR — [`resolve_object_run`], `resolve_member`'s
/// own inline `Codeunit.Run(arity<=1)` special case, [`resolve_implicit_
/// trigger`]'s trigger fan-out, and [`emit_event_flow_edges`]'s subscriber
/// fan-out — look up a routine directly by ROLE (entry trigger / trigger
/// name / subscriber match) rather than through either selection boundary,
/// so a collapse-marked survivor could reach a confident `Opaque`/`Source`
/// route through any of them unguarded. Centralizing the lookup here means
/// a future call site gets the same protection by construction rather than
/// by remembering to copy the check.
fn routine_is_collapse_marked(rid: &RoutineNodeId, graph: &ProgramGraph) -> bool {
    graph
        .routines
        .binary_search_by(|probe| probe.id.cmp(rid))
        .ok()
        .is_some_and(|i| graph.routines[i].abi_overload_collapsed)
}

/// Try to resolve `name_lc` with `arity` arguments inside `obj_id`, as called
/// from the identity `from_object` (Task 1 — beyond-1B.3b-follow-up:
/// PER-CANDIDATE access filtering; see [`routine_candidate_is_visible`]).
///
/// Returns the UNIQUE arity-matched, VISIBLE-from-`from_object` overload as a
/// `Source` route. Returns `None` only when the name is absent entirely in
/// `obj_id` (genuine absence — callers may fall through to a further
/// precedence level). Every other outcome (arity mismatch, access exclusion,
/// or an unresolved overload ambiguity) is `Some(Unresolved{Unknown(reason)})`
/// — a decline that STOPS at this precedence level rather than falling
/// through (mirrors L3's MemberNotFound stop semantics; see module doc).
///
/// # Selection rule (the overload-narrowing guard — do NOT weaken)
///
/// 1. Zero arity-matched candidates (`pre_filter_count == 0`): name found but
///    no overload matches the arity → `Unknown(OverloadAmbiguous)`.
/// 2. `pre_filter_count >= 1`: partition the arity-matched set by
///    [`routine_candidate_is_visible`].
///    - **0 visible** → access excluded every arity-matched candidate →
///      `Unknown(access_exclusion_reason(..))` (falls back to
///      `IndexIntegrationGap` only in the defensive case where a candidate
///      caused the exclusion but the reason-finder couldn't re-derive why —
///      should never happen, since the two functions apply the identical
///      per-`Access` rule).
///    - **Exactly 1 visible AND `pre_filter_count == 1`** → the visible
///      candidate WAS the only overload to begin with; access filtering
///      changed nothing about cardinality → resolve it.
///    - **Exactly 1 visible BUT `pre_filter_count > 1`** → access narrowed an
///      originally-AMBIGUOUS same-arity set down to one. This is NOT a safe
///      selection: the pre-filter set was ambiguous (no arg-type evidence to
///      pick between overloads — full arg-type dispatch is deferred), so
///      access removing the OTHER sibling(s) doesn't prove the call meant
///      THIS one. Selecting the lone survivor would MANUFACTURE a false
///      `Source` route from what is actually still an unproven overload
///      choice → `Unknown(OverloadAmbiguous)`, exactly like the >1-visible
///      case below.
///    - **>1 visible** → genuine unresolved ambiguity (mirrors the
///      interface-implementer fan-out's `>1 candidates → Unresolved` rule) →
///      `Unknown(OverloadAmbiguous)`. Never pick-first.
///
/// **SymbolOnly tier (Task 1 — FULL source-tier selection discipline, no
/// exception):** `params_count` is populated from the ABI (real `Parameters[]
/// .len()`, or the `UNKNOWN_ARITY` sentinel when the field was absent/
/// unparseable — see `abi_ingest::UNKNOWN_ARITY`'s tri-state contract), and
/// `access` is populated from `IsProtected`/the local/internal drop at
/// ingestion (`abi_ingest::ingest_abi`) — so a SymbolOnly candidate now flows
/// through EXACTLY the same arity + per-candidate-visibility selection below
/// as a source-tier one. There is no `candidates.first()` short-circuit: an
/// order-dependent pick on a multi-candidate ABI set is a false-`Source`/
/// `Opaque` vector the pre-Task-1 code was exposed to (a `protected` ABI
/// sibling could shadow a `public` one, or vice versa, purely by JSON array
/// order). An `UNKNOWN_ARITY`-sentinel candidate structurally never matches a
/// real call's `arity` (see the sentinel's doc), so it silently drops out of
/// `matched` below — never emitting, exactly like a genuine arity mismatch.
#[allow(clippy::too_many_arguments)] // 7 pre-existing params + `from_object` (Task 1); each is a distinct identity/lookup input, grouping would obscure call sites.
fn resolve_in_object(
    obj_id: &ObjectNodeId,
    obj_tier: TrustTier,
    name_lc: &str,
    arity: usize,
    from_object: &ObjectNodeId,
    graph: &ProgramGraph,
    index: &ResolveIndex,
    body_map: &BodyMap<'_>,
) -> Option<Route> {
    let candidates = index.routines_in_object(obj_id, name_lc);
    if candidates.is_empty() {
        return None;
    }

    // Arity-exact match: collect EVERY overload whose params_count == arity.
    // With params_count in RoutineNodeId, each overload is normally a distinct
    // node — but two DISTINCT overloads sharing (object, name_lc, params_count)
    // collide onto one `RoutineNodeId` when their `sig_fp` also matches: source
    // `sig_fp` is always 0 (see node.rs), so two textually distinct SOURCE
    // declarations never collide there (their real content lives in
    // `param_sig_key` instead — see `build_program_graph`'s dedup,
    // `dedup_routines_preserving_genuine_overloads`, beyond-1B.3b Task 2). An
    // ABI `sig_fp` (`abi_ingest::param_type_fp`) now folds a length-delimited
    // canonical tuple of every parameter's outer kind + Subtype id + raw
    // Subtype name + a degradation tag (Task 2 round-2 addendum — previously:
    // only the OUTER type keyword, never a `Subtype`, so two genuinely
    // DIFFERENT overloads differing only by an object-typed parameter's
    // Subtype silently collided). Two ABI entries now collide onto one
    // `RoutineNodeId` ONLY when their ENTIRE canonical tuple matches — a true
    // re-parse duplicate, or a residual fingerprint collision this engine
    // cannot further distinguish (either way, `dedup_routines_preserving_
    // genuine_overloads` collapses that run to ONE survivor and flags it
    // `RoutineNode::abi_overload_collapsed`, since an ABI routine's
    // `param_sig_key` is hardcoded empty — no independent content signature
    // beyond the tuple already folded into `sig_fp`). So >1 arity-matched
    // candidates HERE always means genuinely DISTINCT `RoutineNodeId`s
    // (different `sig_fp`) survived that collapse — REAL, unresolved overload
    // ambiguity this engine cannot break by parameter count alone, absent
    // further evidence — an `UNKNOWN_ARITY`-sentinel candidate (Task 1
    // tri-state arity) never lands in `matched` at all, since it can never
    // equal a real call's `arity`.
    let matched: Vec<&RoutineNodeId> = candidates
        .iter()
        .filter(|rid| rid.params_count == arity)
        .collect();
    let pre_filter_count = matched.len();
    if pre_filter_count == 0 {
        // Name found but no arity-matched overload: emit Unknown rather than
        // a false-confident route to a wrong-arity candidate. Does NOT fall
        // through to extension-base / global-builtin — mirrors L3's
        // MemberNotFound stop.
        return Some(unresolved_route(UnknownReason::OverloadAmbiguous));
    }

    // Per-candidate visibility filter (Task 1): partition the arity-matched
    // set by whether it is visible from `from_object`'s identity.
    let visible: Vec<&RoutineNodeId> = matched
        .iter()
        .copied()
        .filter(|rid| routine_candidate_is_visible(rid, from_object, graph, index))
        .collect();

    match visible.len() {
        0 => {
            let reason = access_exclusion_reason(obj_id, name_lc, arity, from_object, graph, index)
                .unwrap_or(UnknownReason::IndexIntegrationGap);
            Some(unresolved_route(reason))
        }
        // Overload-narrowing guard: only select the lone survivor when it was
        // ALSO the lone candidate before visibility filtering. If access
        // narrowed an originally-ambiguous (`pre_filter_count > 1`) set down
        // to one, that is NOT a safe selection — fall through to the `_` arm.
        1 if pre_filter_count == 1 => {
            let rid0 = visible[0];
            // PLAIN-DISPATCH MARKER GUARD (Task 2 round-2, the round-1
            // critical fold-in): before this fix, `abi_overload_collapsed`
            // was consulted ONLY by the chain-type-query boundary
            // (`routine_node_for_type_query`) — a marked survivor could
            // still resolve CONFIDENTLY right here via ordinary PLAIN
            // dispatch (a qualified call like `DepCollapse.Get(X)`, never
            // chained onward), an unguarded false-`Source`/`Opaque` vector.
            // `resolve_in_object` is the choke point for NAME+ARITY overload
            // SELECTION — every dispatch path that narrows a candidate SET
            // down to one by name+arity+visibility (bare-call Step 1/2/3,
            // `resolve_member`'s Object/SelfObject/Interface arms) funnels
            // through here, so placing the guard HERE closes all of them at
            // once rather than one branch at a time. It is NOT, however, the
            // only route-construction site in this module (corrected — the
            // former claim that it was "the SINGLE choke point every
            // plain-call AND qualified-member dispatch path funnels through"
            // was factually wrong, Task 2 review fix): entry-trigger dispatch
            // ([`resolve_object_run`], and `resolve_member`'s own inline
            // `Codeunit.Run(arity<=1)` special case) and multicast fan-out
            // ([`resolve_implicit_trigger`]'s trigger routes,
            // [`emit_event_flow_edges`]'s subscriber routes) each look up a
            // routine directly by ROLE rather than through this candidate-set
            // selection, so each of those four sites carries its OWN
            // [`routine_is_collapse_marked`] guard rather than inheriting
            // this one — see that helper's doc for the full enumeration. A
            // collapse-marked node is the arbitrary (or genuinely
            // indistinguishable) survivor of ≥2 raw ABI entries that
            // fingerprint-collided (see `build::dedup_routines_preserving_
            // genuine_overloads`'s doc) — its `return_type`/identity may not
            // even be the one the caller meant, so it must decline exactly
            // like a genuine >1-candidate ambiguity, never silently resolve,
            // no matter which of the five sites reached it.
            if routine_is_collapse_marked(rid0, graph) {
                return Some(unresolved_route(UnknownReason::OverloadAmbiguous));
            }
            Some(make_routine_route(rid0, obj_tier, body_map, graph))
        }
        _ => Some(unresolved_route(UnknownReason::OverloadAmbiguous)),
    }
}

/// Whether a single, CONCRETE arity-matched routine candidate `rid` is
/// visible from the calling object's identity `from_object` — the
/// PER-CANDIDATE counterpart to [`object_has_visible_member_candidate`]
/// (which only answers EXISTENTIALLY: "does SOME visible candidate exist").
/// [`resolve_in_object`] needs to know WHICH SPECIFIC candidate(s) survive so
/// it can apply the overload-narrowing guard rather than blindly picking the
/// sole visible survivor of an originally-ambiguous set.
///
/// Mirrors the per-`Access` rule [`object_has_visible_member_candidate`]
/// already established (see that function's doc for the full soundness
/// rationale) — RESOLVED OBJECT IDENTITY, never a lowercased-name comparison:
///
/// - [`Access::Public`] → always visible.
/// - [`Access::Local`] → visible ONLY when `rid.object == *from_object` (the
///   candidate's declaring object IS the calling object itself).
/// - [`Access::Internal`] → visible when `rid.object.app == from_object.app`
///   (app-scoped), OR when `rid.object.app`'s manifest declares
///   `from_object.app` a friend via `<InternalsVisibleTo>` (Task 1.5 —
///   see [`internal_visible_across`]). Cross-app `internal` from a
///   true-stranger app still fails closed.
/// - [`Access::Protected`] → visible when `rid.object == *from_object` (self)
///   OR `index.object_extends(graph, from_object, &rid.object)` is `true`
///   (`from_object` is a DIRECT, kind-compatible extension of the
///   candidate's declaring object).
/// - Lookup miss (`None`) → fails closed (excluded), never assumed visible.
///
/// A pure DELEGATE to [`object_access_visible_from`] (Task 2 fold-in, review
/// finding from Task 1): the two functions applied the IDENTICAL per-`Access`
/// rule as two independently-maintained copies — this one keyed by
/// `RoutineNodeId`, that one by `ObjectNodeId` + an already-looked-up
/// `Access`. Two copies of the same rule is exactly the kind of latent drift
/// vector this plan closes elsewhere (see [`object_access_visible_from`]'s
/// own doc) — collapsing to one predicate means a future rule change can
/// never update one copy and silently miss the other.
fn routine_candidate_is_visible(
    rid: &RoutineNodeId,
    from_object: &ObjectNodeId,
    graph: &ProgramGraph,
    index: &ResolveIndex,
) -> bool {
    object_access_visible_from(
        &rid.object,
        lookup_routine_access(graph, rid),
        from_object,
        graph,
        index,
    )
}

/// Whether `caller_app` may see `exposing_app`'s `internal` members —
/// same-app (AL's default `internal` scoping), OR `exposing_app`'s own
/// manifest declares `caller_app` a friend via
/// `<InternalsVisibleTo><Module .../></InternalsVisibleTo>` (Task 1.5).
///
/// Friendship is declared BY the app EXPOSING the internals, never inferred
/// from the reverse direction: `graph.friends` is keyed by the exposing
/// app's [`AppRef`], so `friends[A].contains(B)` means "A trusts B", and
/// does NOT imply `friends[B].contains(A)` — a caller B that itself grants A
/// friend access does not thereby gain access to B's own internals from A's
/// side. See [`crate::program::build::build_program_graph`] Step 3b for how
/// `graph.friends` is populated (GUID-first, name+publisher-fallback
/// resolution of each `<Module>` entry against the snapshot; entries whose
/// app is outside the closure are silently skipped, open-world).
fn internal_visible_across(exposing_app: AppRef, caller_app: AppRef, graph: &ProgramGraph) -> bool {
    exposing_app == caller_app
        || graph
            .friends
            .get(&exposing_app)
            .is_some_and(|f| f.contains(&caller_app))
}

/// Whether `obj_id` (at trust tier `obj_tier`) carries a visible source/ABI
/// candidate routine matching `method_lc`/`arity` — used by the Record-receiver
/// source-shadows-catalog precedence check (beyond-1B.3b Task 1) to determine
/// CARDINALITY across a multi-object scope (base table ∪ TableExtensions)
/// WITHOUT committing to a route.
///
/// A SymbolOnly (ABI/dep) object counts ANY name match as a candidate —
/// EXISTENCE only, deliberately arity-deferred (Task 1: a same-name ABI
/// sibling might carry the `UNKNOWN_ARITY` sentinel, or simply a different
/// arity than the one being probed here, so a name-only scan is the correct,
/// conservative existence signal; [`resolve_in_object`] is where real
/// arity-EXACT matching happens for what actually gets to emit). A source
/// tier object counts only an EXACT arity match.
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

/// The shared per-`Access` visibility predicate: is a candidate DECLARED IN
/// `obj_id` with declared access `access` visible from the CALLING object's
/// identity `from_object`? Factored out (Task 1) so [`object_has_visible_
/// member_candidate`]'s source/ABI arity-filtered scan and its SymbolOnly
/// NAME-ONLY scan share EXACTLY one rule — two independent copies could
/// silently drift and reopen the exact soundness gap this function closes.
/// [`routine_candidate_is_visible`] (the PER-CANDIDATE selection rule
/// `resolve_in_object` uses) DELEGATES to this same predicate too (Task 2
/// fold-in) — every per-candidate/per-object visibility check in this module
/// now traces back to this ONE function.
///
/// RESOLVED OBJECT IDENTITY, never a lowercased-name comparison — every
/// branch compares [`ObjectNodeId`]s or [`AppRef`]s, both derived from
/// `graph`/`index` identity, never from `Origin`/source text.
///
/// - [`Access::Public`] → always visible.
/// - [`Access::Local`] → visible ONLY when `obj_id == from_object` (the
///   candidate's declaring object IS the calling object itself — AL's
///   `local` is OBJECT-scoped, not app-scoped; this was the first latent
///   false-`Source` beyond-1B.3b Task 1 closed: the pre-fix code treated ANY
///   same-app candidate as visible, so a same-app but DIFFERENT object's
///   `local` procedure false-resolved to `Source`).
/// - [`Access::Internal`] → visible when `obj_id.app == from_object.app`
///   (app-scoped), OR when `obj_id.app`'s manifest declares
///   `from_object.app` a friend via `<InternalsVisibleTo>` — AL's
///   friend-app exception, modeled by [`internal_visible_across`] (Task
///   1.5, closing the over-decline the app-scoped-only version of this rule
///   left: measurement proved 100% of the resulting `InternalNotVisible`
///   bucket was AL-LEGAL friend calls, not genuine access violations).
///   Cross-app `internal` from a true stranger (no friend declaration in
///   either direction) still fails closed to `Unknown`.
/// - [`Access::Protected`] → visible when `obj_id == from_object` (self) OR
///   `index.object_extends(graph, from_object, obj_id)` is `true` — `from_object`
///   is a DIRECT, kind-compatible extension of the candidate's declaring
///   object (see [`ResolveIndex::object_extends`] for the full DIRECT +
///   KIND-COMPATIBLE + never-reverse + never-peer contract). This closes the
///   second latent false-`Source` beyond-1B.3b Task 1 closed: the pre-fix
///   code left `Protected` completely unfiltered for any same-app candidate,
///   including a same-app-but-unrelated object AND a PEER extension of the
///   same base (the sibling-bleed case). SAME rule for a SymbolOnly base
///   (Task 1): AL lets a workspace extension of a dep object call the dep's
///   `protected` members, and `object_extends` is already tier-agnostic.
/// - Lookup miss (`None`) → fails closed (excluded), never assumed visible.
fn object_access_visible_from(
    obj_id: &ObjectNodeId,
    access: Option<Access>,
    from_object: &ObjectNodeId,
    graph: &ProgramGraph,
    index: &ResolveIndex,
) -> bool {
    match access {
        Some(Access::Public) => true,
        Some(Access::Local) => obj_id == from_object,
        Some(Access::Internal) => internal_visible_across(obj_id.app, from_object.app, graph),
        Some(Access::Protected) => {
            obj_id == from_object || index.object_extends(graph, from_object, obj_id)
        }
        None => false,
    }
}

/// Like [`object_has_member_candidate`], but additionally excludes a
/// candidate whose declared [`Access`] is not visible from `from_object` —
/// the caller-identity-aware visibility check (beyond-1B.3b Task 1,
/// superseding the app-scoped Task 2 version). See [`object_access_visible_
/// from`] for the full per-`Access` rule (shared by both branches below).
///
/// # SymbolOnly (Task 1 — NAME-ONLY `.any()` scan, no shortcut)
///
/// `access` is now populated from the real ABI (`IsProtected`/local/internal
/// drop at ingestion — `abi_ingest::ingest_abi`), so a `protected` ABI
/// routine is a genuine, non-trivial visibility check, not a hardcoded
/// `Public` no-op. The scan is deliberately NAME-ONLY (the UN-arity-filtered
/// candidate list), mirroring [`object_has_member_candidate`]'s own
/// arity-deferred SymbolOnly existence check: a protected first sibling must
/// never hide a visible public one purely by JSON-array order, and a real
/// arity mismatch on ONE same-name candidate must not suppress a DIFFERENT
/// same-name candidate that IS both visible and arity-correct. This function
/// answers EXISTENTIALLY ("is SOME visible candidate reachable") — it is an
/// existence/diagnostics gate consumed by callers to decide whether to even
/// ATTEMPT `resolve_in_object`, never edge evidence by itself:
/// `resolve_in_object` alone performs the arity-EXACT, per-candidate
/// selection that decides what (if anything) actually emits.
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
        return index
            .routines_in_object(obj_id, method_lc)
            .iter()
            .any(|rid| {
                object_access_visible_from(
                    obj_id,
                    lookup_routine_access(graph, rid),
                    from_object,
                    graph,
                    index,
                )
            });
    }
    index
        .routines_in_object(obj_id, method_lc)
        .iter()
        .filter(|rid| rid.params_count == arity)
        .any(|rid| {
            object_access_visible_from(
                obj_id,
                lookup_routine_access(graph, rid),
                from_object,
                graph,
                index,
            )
        })
}

/// Diagnostic companion to [`object_has_visible_member_candidate`] (Task 3):
/// when NO visible candidate exists for `method_lc`/`arity` in `obj_id`,
/// determine WHY the most specific excluded candidate (if any) was excluded
/// — `Local`/`Internal`/`Protected` access not visible from `from_object`'s
/// identity. Returns `None` when no same-name/arity candidate exists in
/// `obj_id` at all (genuine absence, not a visibility exclusion) — mirrors
/// [`object_has_visible_member_candidate`]'s own per-access rule exactly, so
/// the two never disagree on WHETHER a candidate is visible, only on WHY not.
///
/// Tier-agnostic (Task 1): SymbolOnly `access` is now populated from the real
/// ABI (`IsProtected`/local+internal drop at ingestion), so a SymbolOnly
/// candidate can be genuinely access-excluded (most commonly `protected`,
/// from a non-extension caller) — the previous hardcoded "SymbolOnly is
/// always Public, never access-excluded" short-circuit is gone; the
/// arity-filtered scan + per-`Access` reason derivation below apply
/// identically to both tiers (every call site here is already arity-KNOWN,
/// so no separate name-only variant is needed, unlike [`object_has_visible_
/// member_candidate`]'s existence-only scan).
fn access_exclusion_reason(
    obj_id: &ObjectNodeId,
    method_lc: &str,
    arity: usize,
    from_object: &ObjectNodeId,
    graph: &ProgramGraph,
    index: &ResolveIndex,
) -> Option<UnknownReason> {
    index
        .routines_in_object(obj_id, method_lc)
        .iter()
        .filter(|rid| rid.params_count == arity)
        .find_map(|rid| match lookup_routine_access(graph, rid) {
            Some(Access::Local) if obj_id != from_object => Some(UnknownReason::LocalNotVisible),
            Some(Access::Internal)
                if !internal_visible_across(obj_id.app, from_object.app, graph) =>
            {
                Some(UnknownReason::InternalNotVisible)
            }
            Some(Access::Protected)
                if obj_id != from_object && !index.object_extends(graph, from_object, obj_id) =>
            {
                Some(UnknownReason::ProtectedNotVisible)
            }
            _ => None,
        })
}

/// The outcome of a [`resolve_in_table_scope`] search — sufficient for the
/// caller to know not just WHETHER it resolved, but on decline, WHY (Task 3's
/// diagnostic [`UnknownReason`] payload).
enum TableScopeOutcome {
    /// Exactly one visible candidate — resolved.
    Resolved(DispatchShape, Vec<Route>),
    /// `>1` visible candidates — honest ambiguous `Unknown`. Callers MUST
    /// return this immediately (never fall through to the catalog — source
    /// ambiguity still shadows a same-named intrinsic).
    Ambiguous,
    /// Zero visible candidates. `access_excluded` is `Some(reason)` when a
    /// same-name/arity candidate existed in scope but was excluded by the
    /// caller-identity access filter — the most specific decline reason
    /// available; `None` when no candidate existed in scope at all (name
    /// genuinely absent — the caller should fall through / use its own
    /// default reason).
    NotVisible {
        access_excluded: Option<UnknownReason>,
    },
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
///    rationale — beyond-1B.3b Task 1, superseded by Task 1's protected-ABI
///    soundness fix: SymbolOnly candidates are NO LONGER a `Public`-only
///    no-op here, since `access` now carries the real ABI `IsProtected`
///    modifier, so this filter can genuinely exclude a SymbolOnly candidate
///    too).
///
/// # Cardinality (unchanged from the pre-extraction Record arm; Task 3 wraps
/// the same three outcomes in [`TableScopeOutcome`] so a decline also
/// carries WHY)
///
/// - 0 visible candidates (or `table_id` itself not visible) →
///   [`TableScopeOutcome::NotVisible`] — fall through to the caller's next
///   precedence level (e.g. the Record builtin catalog).
/// - Exactly 1 visible candidate → [`TableScopeOutcome::Resolved`], a single
///   `Source`/`Abi`/`Opaque` route via [`resolve_in_object`].
/// - `>1` visible candidates → [`TableScopeOutcome::Ambiguous`] — honest
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
) -> TableScopeOutcome {
    let closure = graph.topology.closure(from_object.id.app);

    if !closure.contains(&table_id.app) {
        return TableScopeOutcome::NotVisible {
            access_excluded: None,
        };
    }
    let Some((table_tier, table_name_lc)) = graph
        .objects
        .iter()
        .find(|o| o.id == table_id)
        .map(|o| (o.tier, o.name.to_ascii_lowercase()))
    else {
        return TableScopeOutcome::NotVisible {
            access_excluded: None,
        };
    };

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
        (None, _) => {
            // Zero visible candidates in the whole scope — diagnose WHY, in
            // scope order (deterministic): the first same-name/arity
            // candidate that exists but is access-excluded, if any.
            let access_excluded = scope.iter().find_map(|(oid, _tier)| {
                access_exclusion_reason(oid, name_lc, arity, &from_object.id, graph, index)
            });
            TableScopeOutcome::NotVisible { access_excluded }
        }
        (Some(_), Some(_)) => TableScopeOutcome::Ambiguous,
        (Some((oid, tier)), None) => {
            match resolve_in_object(
                oid,
                *tier,
                name_lc,
                arity,
                &from_object.id,
                graph,
                index,
                body_map,
            ) {
                Some(route) => TableScopeOutcome::Resolved(DispatchShape::Exact, vec![route]),
                // Defensive: `object_has_visible_member_candidate` already
                // confirmed a visible arity match exists, so `resolve_in_object`
                // should always return `Some` here.
                None => TableScopeOutcome::NotVisible {
                    access_excluded: Some(UnknownReason::IndexIntegrationGap),
                },
            }
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
        &from_object.id,
        graph,
        index,
        body_map,
    ) {
        return vec![route];
    }

    // Task 3: running diagnostic reason for the eventual Step 5 fallback, in
    // case no earlier step resolves. Steps 2/3 below OVERWRITE this with a
    // more specific finding as they run; the DEFAULT (`MemberNotFound`)
    // survives when nothing more specific was found — genuine absence.
    let mut reason = UnknownReason::MemberNotFound;

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
        ) {
            if let Some(route) = resolve_in_object(
                &base_id,
                base_tier,
                name_lc,
                arity,
                &from_object.id,
                graph,
                index,
                body_map,
            ) {
                return vec![route];
            }
        } else if let Some(r) =
            access_exclusion_reason(&base_id, name_lc, arity, &from_object.id, graph, index)
        {
            reason = r;
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
    // (defense-in-depth) kind match. Task 3: tag WHY it's skipped for the two
    // named, high-volume excluded kinds (Codeunit/Report(Extension)) so the
    // eventual Step 5 Unknown carries that context rather than the generic
    // `MemberNotFound` default.
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
        if with_state == WithState::NoWithProven {
            // (2) Compute the implicit-Rec table id by kind; no unique
            // in-closure table → fall through (nothing to search).
            if let Some(table_id) = implicit_rec_table_id(from_object, graph, index) {
                // (3) Visibility-scoped table ∪ extensions search (Task 2):
                // `NotVisible` falls through to Step 4/5 (tagging WHY when a
                // candidate existed but was access-excluded); `Resolved` is a
                // clean Source/Abi/Opaque route; `Ambiguous` is an honest
                // ambiguous Unknown (>1 visible candidate — never pick-first,
                // never falls through to the catalog).
                match resolve_in_table_scope(
                    from_object,
                    table_id,
                    name_lc,
                    arity,
                    graph,
                    index,
                    body_map,
                ) {
                    TableScopeOutcome::Resolved(_, routes) => {
                        // (4) Builtin/intrinsic PROBE-THEN-DECIDE: the probe
                        // (step 3) already ran; a same-name+arity table-scope
                        // candidate exists AND `name_lc` is also a global
                        // builtin or a bare-callable page/instance intrinsic
                        // is an UNPROVEN precedence collision — fail closed
                        // to `Unknown` rather than assume the table wins
                        // (never emit `Catalog` here; Step 4 below is the
                        // only place that does).
                        if is_bare_builtin_or_page_intrinsic(name_lc) {
                            return vec![unresolved_route(
                                UnknownReason::BuiltinPrecedenceCollision,
                            )];
                        }
                        return routes;
                    }
                    TableScopeOutcome::Ambiguous => {
                        return vec![unresolved_route(UnknownReason::OverloadAmbiguous)];
                    }
                    TableScopeOutcome::NotVisible { access_excluded } => {
                        if let Some(r) = access_excluded {
                            reason = r;
                        }
                    }
                }
            } else {
                // No unique in-closure implicit-Rec table (ambiguous
                // cross-app name, out-of-closure, or unresolved).
                reason = UnknownReason::ReceiverOutOfClosure;
            }
        } else {
            // Lexically inside a `with` block (or with-freedom unproven).
            reason = UnknownReason::WithScopeGuard;
        }
    } else if matches!(from_object.id.kind, ObjectKind::Codeunit) {
        reason = UnknownReason::CodeunitTableNoExcluded;
    } else if matches!(
        from_object.id.kind,
        ObjectKind::Report | ObjectKind::ReportExtension
    ) {
        reason = UnknownReason::ReportRecExcluded;
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
    vec![unresolved_route(reason)]
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
            vec![unresolved_route(UnknownReason::UntrackedReceiver)],
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
            vec![unresolved_route(UnknownReason::MemberNotFound)],
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

    // COLLAPSE-MARKER GUARD (Task 2 review fix): this entry-trigger lookup
    // bypasses `resolve_in_object`'s name+arity selection entirely (it picks
    // by ROLE — the object-level trigger — never by counting candidates), so
    // it must consult the marker itself; see `routine_is_collapse_marked`'s
    // doc for the full enumeration of guarded sites.
    if routine_is_collapse_marked(entry_rid, graph) {
        return (
            DispatchShape::Exact,
            SetCompleteness::Complete,
            vec![unresolved_route(UnknownReason::OverloadAmbiguous)],
        );
    }

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

    // COLLAPSE-MARKER GUARD (Task 2 review fix): this trigger fan-out looks
    // up each candidate by ROLE (fixed trigger name), never through
    // `resolve_in_object`'s name+arity selection, so both loops below must
    // consult the marker themselves — see `routine_is_collapse_marked`'s
    // doc. A marked trigger declines to an honest Unknown route rather than
    // silently vanishing from the Multicast set (which would understate its
    // real cardinality) or resolving confidently to a possibly-wrong
    // identity.

    // Triggers on the base table itself.
    for rid in index.routines_in_object(&table_object.id, trigger_name) {
        if routine_is_collapse_marked(rid, graph) {
            routes.push(unresolved_route(UnknownReason::OverloadAmbiguous));
            continue;
        }
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
            if routine_is_collapse_marked(rid, graph) {
                routes.push(unresolved_route(UnknownReason::OverloadAmbiguous));
                continue;
            }
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

/// Build a `(Exact, [Unknown route])` outcome (Task 3: `reason` is REQUIRED —
/// every caller supplies a diagnostic [`UnknownReason`]).
fn member_unknown_route(reason: UnknownReason) -> (DispatchShape, Vec<Route>) {
    (DispatchShape::Exact, vec![unresolved_route(reason)])
}

/// Build a `(DynamicOpen, [Unknown blocker])` outcome for Dynamic receivers —
/// the receiver's static type is genuinely untracked (a runtime Variant), so
/// the single fixed reason is always [`UnknownReason::UntrackedReceiver`].
fn member_dynamic_open_route() -> (DispatchShape, Vec<Route>) {
    (
        DispatchShape::DynamicOpen,
        vec![unresolved_route(UnknownReason::UntrackedReceiver)],
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
                member_unknown_route(UnknownReason::CatalogMiss)
            }
        }
        ReceiverType::FieldRef => {
            if let Some(bid) = member_builtin_id(MemberCatalogKind::FieldRef, method_lc) {
                member_catalog_route(bid)
            } else {
                member_unknown_route(UnknownReason::CatalogMiss)
            }
        }
        ReceiverType::KeyRef => {
            if let Some(bid) = member_builtin_id(MemberCatalogKind::KeyRef, method_lc) {
                member_catalog_route(bid)
            } else {
                member_unknown_route(UnknownReason::CatalogMiss)
            }
        }
        ReceiverType::Framework(kind) => {
            if let Some(bid) = member_builtin_id(MemberCatalogKind::Framework(kind), method_lc) {
                member_catalog_route(bid)
            } else {
                member_unknown_route(UnknownReason::CatalogMiss)
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
            //
            // Task 3: `reason` defaults to `CatalogMiss` (the catalog-fallback
            // outcome below); `table: None` means Phase A already declined to
            // pin a unique receiver table (ambiguous/out-of-closure/
            // unresolved) — `ReceiverOutOfClosure`. A `NotVisible` table-scope
            // outcome with an access-excluded candidate overrides the default.
            let mut reason = UnknownReason::CatalogMiss;
            if let Some(table_id) = table {
                match resolve_in_table_scope(
                    from_object,
                    table_id.clone(),
                    method_lc,
                    arity,
                    graph,
                    index,
                    body_map,
                ) {
                    TableScopeOutcome::Resolved(shape, routes) => return (shape, routes),
                    TableScopeOutcome::Ambiguous => {
                        return (
                            DispatchShape::Exact,
                            vec![unresolved_route(UnknownReason::OverloadAmbiguous)],
                        );
                    }
                    TableScopeOutcome::NotVisible { access_excluded } => {
                        if let Some(r) = access_excluded {
                            reason = r;
                        }
                    }
                }
            } else {
                reason = UnknownReason::ReceiverOutOfClosure;
            }

            // Zero visible source/ABI candidates in scope (or `table`
            // unresolved/not visible): Record built-in methods (SetRange,
            // Find, Insert, ...) are platform-intrinsic and resolve
            // table-independently.
            if let Some(bid) = member_builtin_id(MemberCatalogKind::Record, method_lc) {
                return member_catalog_route(bid);
            }
            member_unknown_route(reason)
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
                return member_unknown_route(UnknownReason::MemberNotFound);
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
                    // COLLAPSE-MARKER GUARD (Task 2 review fix): mirrors
                    // `resolve_object_run`'s own guard — this arm looks up
                    // the entry trigger by ROLE, never through
                    // `resolve_in_object`'s name+arity selection, so it must
                    // consult the marker itself; see `routine_is_collapse_
                    // marked`'s doc.
                    if routine_is_collapse_marked(entry_rid, graph) {
                        (
                            DispatchShape::Exact,
                            vec![unresolved_route(UnknownReason::OverloadAmbiguous)],
                        )
                    } else {
                        (
                            DispatchShape::Exact,
                            vec![make_routine_route(entry_rid, target_tier, body_map, graph)],
                        )
                    }
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
                &from_object.id,
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
                member_unknown_route(UnknownReason::MemberNotFound)
            }
        }
        ReceiverType::SelfObject => {
            // Dispatch to the calling object's own declared procedures.
            if let Some(route) = resolve_in_object(
                &from_object.id,
                from_object.tier,
                method_lc,
                arity,
                &from_object.id,
                graph,
                index,
                body_map,
            ) {
                (DispatchShape::Exact, vec![route])
            } else {
                // Method not found in own object.
                member_unknown_route(UnknownReason::MemberNotFound)
            }
        }
        ReceiverType::Interface { name_lc } => {
            // Phase 4 Task 2: fan out to all known implementers.
            //
            // For each implementer:
            //   SymbolOnly tier  → delegate directly to `resolve_in_object`, which
            //                      (Task 1) applies the SAME arity-exact +
            //                      per-candidate-visibility selection as source —
            //                      returns AbiSymbol (unique visible arity match),
            //                      or Unknown (arity mismatch / access-excluded /
            //                      ambiguous), never an order-dependent pick.
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
                    // SymbolOnly: delegate to `resolve_in_object`'s full
                    // arity+visibility discipline (Task 1) — no pre-check
                    // needed here (unlike the source-tier `else` branch below)
                    // since `resolve_in_object` itself now returns Some(Unknown)
                    // on arity mismatch/access exclusion/ambiguity. The
                    // `unwrap_or` fires only when this implementer does not
                    // declare `method_lc` at all (`resolve_in_object` returns
                    // `None` only on a name-absent `candidates.is_empty()`).
                    let route = resolve_in_object(
                        impl_id,
                        impl_tier,
                        method_lc,
                        arity,
                        &from_object.id,
                        graph,
                        index,
                        body_map,
                    )
                    .unwrap_or_else(|| unresolved_route(UnknownReason::MemberNotFound));
                    routes.push(route);
                } else {
                    let candidates = index.routines_in_object(impl_id, method_lc);
                    if candidates.is_empty() {
                        // Method name absent from this implementer — Rule 1 Unresolved.
                        routes.push(unresolved_route(UnknownReason::MemberNotFound));
                    } else {
                        let matching = candidates
                            .iter()
                            .filter(|r| r.params_count == arity)
                            .count();
                        match matching {
                            1 => {
                                // Unique arity-matched overload: guaranteed to
                                // resolve — the `unwrap_or` is defensive
                                // (should never fire; `resolve_in_object`
                                // itself finds `matched.len() == 1`).
                                let route = resolve_in_object(
                                    impl_id,
                                    impl_tier,
                                    method_lc,
                                    arity,
                                    &from_object.id,
                                    graph,
                                    index,
                                    body_map,
                                )
                                .unwrap_or_else(|| {
                                    unresolved_route(UnknownReason::IndexIntegrationGap)
                                });
                                routes.push(route);
                            }
                            _ => {
                                // 0 (arity mismatch) or >1 (ambiguous) — Rule 1+2 Unresolved.
                                // Never emit a guessed route to a wrong-arity or wrong-overload target.
                                routes.push(unresolved_route(UnknownReason::OverloadAmbiguous));
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
                member_unknown_route(UnknownReason::CatalogMiss)
            }
        }
        ReceiverType::Primitive => {
            // Non-catalog type — honest Unknown (not a false resolution gap).
            member_unknown_route(UnknownReason::CatalogMiss)
        }
        ReceiverType::Dynamic => {
            // Variant-typed receiver — genuinely dynamic, not a resolution hole.
            member_dynamic_open_route()
        }
        ReceiverType::Unknown => member_unknown_route(UnknownReason::UntrackedReceiver),
    }
}

// ---------------------------------------------------------------------------
// Cross-object call-result chain support (plan v2.1 Task 3)
// ---------------------------------------------------------------------------

/// Resolve the [`RoutineNode`] a `resolve_member` type-query's SINGLE route
/// identifies, so `receiver.rs`'s cross-object call-result chain step (`Var.
/// Method().X()`, plan v2.1 Task 3) can read its declared `return_type`/
/// `return_type_id` — the shared lookup needed regardless of which
/// [`RouteTarget`] shape the route carries.
///
/// - [`RouteTarget::Routine`] — direct: `graph.routines` is sorted by
///   `RoutineNodeId` (see `build.rs`), so a `binary_search_by` finds the
///   exact node the route already resolved to (whatever its tier — a
///   source-bodied dependency reached via `Routine` carries a real
///   `return_type` from source parsing, never a `return_type_id`; see
///   `node_extract::extract_nodes`).
/// - [`RouteTarget::AbiSymbol`] — **the ABI-PREFIX UNIQUENESS GUARD**
///   (round-1 C1+C2, round-2 arity-PROOF). The route carries no routine id.
///   Prior to Task 2, ABI parameter types were DEGRADED — `abi_ingest::
///   param_type_fp` fingerprinted only the OUTER type keyword
///   (`AbiParameter::type_text` never carried a param's `Subtype`), so two
///   genuinely different same-arity overloads differing only in an
///   object-typed parameter's Subtype (`Get(X: Codeunit A)` vs `Get(X:
///   Codeunit B)`) could hash-collide onto the identical `RoutineNodeId`.
///   Task 2 closed that: `param_type_fp` now folds a length-delimited
///   canonical tuple (outer kind + Subtype id + raw Subtype name + a
///   degradation tag), so two overloads collide ONLY when their ENTIRE tuple
///   matches — a true duplicate, or a residual collision this engine cannot
///   further distinguish (either way, correctly collapse-marked, see
///   `build::dedup_routines_preserving_genuine_overloads`'s doc). This
///   function's uniqueness proof remains necessary regardless — a naive
///   `(object, name, arity)` re-lookup still CANNOT be trusted to reproduce
///   the exact candidate `resolve_member` selected when >1 same-name/
///   same-arity siblings exist (now genuinely distinct `RoutineNodeId`s,
///   not a fp collision) — this function instead requires ALL of:
///   - the declaring `ObjectNodeId` reconstructed from `key` (the same
///     `object_number != 0 ⇒ Id` / `else ⇒ Name` convention
///     `make_routine_route`/`abi_check::RawAbiIndex` already use, and the
///     same `object_kind_from_abi_type` reversal of the key's `{:?}`-derived
///     `object_type` string);
///   - the SAME arity matcher [`resolve_in_object`] uses
///     (`rid.params_count == dispatch_arity` — tri-state-safe by
///     construction: the `UNKNOWN_ARITY` sentinel can never equal a real
///     arity);
///   - EXACTLY ONE candidate surviving BOTH that arity filter AND
///     per-candidate visibility from `from_object`
///     ([`routine_candidate_is_visible`]) — same-name/same-arity siblings
///     (the exact degraded-collision case above), or a candidate excluded by
///     access, both decline. A unique surviving candidate is, by
///     construction, the one `resolve_member` itself selected (identical
///     filters, identical order).
/// - [`RouteTarget::Builtin`] / [`RouteTarget::Unresolved`] — no routine
///   identity at all; decline.
///
/// `dispatch_arity` MUST be the exact arity `resolve_member` was called
/// with to produce `route` (the same value, never re-derived) — see
/// `receiver.rs`'s round-1 M1 note.
///
/// **Collapsed-ABI-overload guard (Task 3 review fix; sibling landed in
/// plain dispatch by Task 2 round-2 — see [`resolve_in_object`]'s own
/// PLAIN-DISPATCH MARKER GUARD).** Whichever branch resolves a node, the
/// result is rejected when [`RoutineNode::abi_overload_collapsed`] is `true`
/// — such a node is the arbitrary (or genuinely indistinguishable) survivor
/// of ≥2 raw ABI overload entries that fingerprint-collided onto one
/// `RoutineNodeId` (a true duplicate, or — post Task 2 — a residual
/// canonical-tuple collision; see `abi_ingest::param_type_fp`'s doc), so its
/// `return_type`/`return_type_id` may belong to the WRONG declaration. This
/// is checked here — the single choke point both `RouteTarget` arms funnel
/// through for the CHAIN type-query path — rather than solely inside
/// [`resolve_abi_prefix_routine`], so a future `RouteTarget::Routine`
/// producer can never accidentally bypass it (today only source-tier
/// routines reach `RouteTarget::Routine`, and `abi_overload_collapsed` is
/// unconditionally `false` for those — see
/// `build::dedup_routines_preserving_genuine_overloads` — so this is a
/// no-op on the current call graph, purely defensive).
pub(crate) fn routine_node_for_type_query<'g>(
    route: &Route,
    dispatch_arity: usize,
    from_object: &ObjectNode,
    graph: &'g ProgramGraph,
    index: &ResolveIndex,
) -> Option<&'g RoutineNode> {
    let node = match &route.target {
        RouteTarget::Routine(rid) => graph
            .routines
            .binary_search_by(|probe| probe.id.cmp(rid))
            .ok()
            .map(|i| &graph.routines[i]),
        RouteTarget::AbiSymbol { key } => {
            resolve_abi_prefix_routine(key, dispatch_arity, from_object, graph, index)
        }
        RouteTarget::Builtin(_) | RouteTarget::Unresolved => None,
    }?;
    if node.abi_overload_collapsed {
        return None;
    }
    Some(node)
}

/// The ABI-PREFIX UNIQUENESS GUARD's implementation — see
/// [`routine_node_for_type_query`]'s doc for the full rationale. Note this
/// function only proves EXACTLY ONE `RoutineNodeId` is arity+visibility
/// selectable at the ABI boundary; it does NOT by itself prove that id was
/// never a dedup collapse of ≥2 raw ABI overloads — the caller,
/// [`routine_node_for_type_query`], applies that additional
/// `abi_overload_collapsed` check uniformly to whatever node this returns.
fn resolve_abi_prefix_routine<'g>(
    key: &AbiRoutineKey,
    dispatch_arity: usize,
    from_object: &ObjectNode,
    graph: &'g ProgramGraph,
    index: &ResolveIndex,
) -> Option<&'g RoutineNode> {
    let kind = object_kind_from_abi_type(&key.object_type);
    let obj_key = if key.object_number != 0 {
        ObjKey::Id(key.object_number)
    } else {
        ObjKey::Name(key.object_name_lc.clone())
    };
    let obj_id = ObjectNodeId {
        app: key.app,
        kind,
        key: obj_key,
    };

    let visible: Vec<&RoutineNodeId> = index
        .routines_in_object(&obj_id, &key.routine_name_lc)
        .iter()
        .filter(|rid| rid.params_count == dispatch_arity)
        .filter(|rid| routine_candidate_is_visible(rid, &from_object.id, graph, index))
        .collect();
    let [rid] = visible.as_slice() else {
        return None;
    };
    graph
        .routines
        .binary_search_by(|probe| probe.id.cmp(rid))
        .ok()
        .map(|i| &graph.routines[i])
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
///   (mirrors [`make_routine_route`]'s SymbolOnly path) — EXCEPT a subscriber
///   marked [`RoutineNode::abi_overload_collapsed`] (Task 2 review fix), whose
///   route is SKIPPED entirely rather than emitted `Opaque` to a possibly-wrong
///   identity; see `routine_is_collapse_marked`'s doc.
///
/// A publisher with **zero** subscribers emits an edge with **empty routes** —
/// this is an honest "published, no subscribers in snapshot" state, classified
/// as `ObligationOutcome::HonestEmpty` by `classify_obligation`. The SAME empty-
/// routes shape also results when every subscriber that WOULD have matched was
/// collapse-marked — indistinguishable from "no subscribers" at this edge's
/// granularity, and correctly so: neither case has a trustworthy target.
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
        // COLLAPSE-MARKER GUARD (Task 2 review fix): this subscriber fan-out
        // looks up each candidate by ROLE (an already-matched subscriber
        // entry), never through `resolve_in_object`'s name+arity selection,
        // so it must consult the marker itself — see `routine_is_collapse_
        // marked`'s doc. Unlike the OTHER three guarded sites (which
        // substitute an honest Unknown route in place of the marked
        // candidate), a marked subscriber's route is SKIPPED entirely here:
        // `SetCompleteness::Partial{ReverseDependentSubscribers}` already
        // documents this Multicast set as open-world, so dropping one
        // untrustworthy candidate doesn't understate a otherwise-closed
        // cardinality the way it would in `resolve_implicit_trigger`'s
        // fan-out.
        let routes: Vec<Route> = subs
            .iter()
            .filter(|se| !routine_is_collapse_marked(&se.subscriber, graph))
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
            ..Default::default()
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
            ..Default::default()
        }
    }

    /// Build a `ProgramGraph` from `ParsedUnit`s, dependency edges, and
    /// `internalsVisibleTo` friend-app authorizations — the Task 1.5
    /// counterpart to [`build_graph_multi_dep`]. `friends` is
    /// `(exposing_app_name, friend_app_name)`: the FIRST name's app is the
    /// one whose manifest declares the SECOND name's app a friend (mirrors
    /// `graph.friends`'s one-directional, exposing-app-keyed semantics —
    /// see `internal_visible_across`'s doc).
    fn build_graph_multi_dep_friends(
        units: &[ParsedUnit],
        deps: &[(&str, &str)],
        friends: &[(&str, &str)],
    ) -> ProgramGraph {
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

        let mut friends_map: std::collections::HashMap<AppRef, std::collections::BTreeSet<AppRef>> =
            std::collections::HashMap::new();
        for (exposing_name, friend_name) in friends {
            let exposing_ref = apps.find_by_name(exposing_name).expect("exposing app");
            let friend_ref = apps.find_by_name(friend_name).expect("friend app");
            friends_map
                .entry(exposing_ref)
                .or_default()
                .insert(friend_ref);
        }

        let obj_index = ObjectIndex::build(&objects);
        ProgramGraph {
            apps,
            topology,
            objects,
            routines,
            obj_index,
            friends: friends_map,
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
        assert!(matches!(r.evidence, Evidence::Unknown(_)));
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
        assert!(matches!(routes[0].evidence, Evidence::Unknown(_)));
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
        assert!(matches!(routes[0].evidence, Evidence::Unknown(_)));
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
        assert!(matches!(routes[0].evidence, Evidence::Unknown(_)));
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
    // Task 2 fold-in (cross-object-chains-and-protected-abi plan v2.1): the
    // genuine boundary fixture Task 1's fixture (g) missed.
    //
    // Task 1's (g)/(i) (`ws_protected_abi_wrong_arity_single_overload_no_emit`
    // in `tests/program_resolve_harness.rs`) proved "existence ≠ emission" for
    // a wrong-arity SymbolOnly candidate, but it drove the call through an
    // OBJECT-receiver dispatch (`resolve_member`'s Object arm →
    // `resolve_in_object` directly) — a path that never consults
    // `object_has_visible_member_candidate`'s existence boolean at all. Task
    // 1's OWN "caller audit" section identified the boolean's REAL callers as
    // exactly two: `resolve_bare` Step 2 (the extension-base gate below) and
    // `resolve_in_table_scope`'s cardinality filter. Neither was exercised by
    // (g). This fixture closes that gap: a SymbolOnly base object exposes a
    // SINGLE same-name candidate at the WRONG arity. `object_has_member_
    // candidate`'s SymbolOnly branch is deliberately arity-DEFERRED (an
    // `.any()` name-only scan — existence only), so Step 2's gate reports
    // "exists" and proceeds to call `resolve_in_object`; that function's own
    // arity-exact selection must then be the one and only place that decides,
    // correctly declining rather than leaking the existence boolean's
    // arity-blindness into a false emission.
    // -----------------------------------------------------------------------

    #[test]
    fn bare_extension_base_symbolonly_wrong_arity_existence_never_leaks_into_emission() {
        // App WS: ReportExtension 50100 "MyRptExt" extends Report "BaseRpt" (a
        // SymbolOnly dep object in App Dep, declaring the ONLY overload of
        // "DoFoo" at arity 0). App WS depends on App Dep.
        let ws_id = make_app_id("WS");
        let dep_id = make_app_id("DepApp");

        let mut apps = AppRegistry::default();
        let ws_ref = apps.intern(&ws_id);
        let dep_ref = apps.intern(&dep_id);

        let ext_obj_id = ObjectNodeId {
            app: ws_ref,
            kind: ObjectKind::ReportExtension,
            key: ObjKey::Id(50100),
        };
        let base_obj_id = ObjectNodeId {
            app: dep_ref,
            kind: ObjectKind::Report,
            key: ObjKey::Id(60000),
        };

        let objects = vec![
            ObjectNode {
                id: ext_obj_id.clone(),
                name: "MyRptExt".into(),
                declared_id: Some(50100),
                extends_target: Some("BaseRpt".into()),
                implements: vec![],
                tier: TrustTier::Workspace,
                source_table: None,
                table_no: None,
                source_table_temporary: false,
                page_controls: vec![],
                fields: vec![],
                dataitems: vec![],
            },
            ObjectNode {
                id: base_obj_id.clone(),
                name: "BaseRpt".into(),
                declared_id: Some(60000),
                extends_target: None,
                implements: vec![],
                tier: TrustTier::SymbolOnly,
                source_table: None,
                table_no: None,
                source_table_temporary: false,
                page_controls: vec![],
                fields: vec![],
                dataitems: vec![],
            },
        ];

        // The ONLY "DoFoo" overload on the SymbolOnly base is arity 0 (public).
        let routines = vec![RoutineNode {
            id: RoutineNodeId {
                object: base_obj_id.clone(),
                name_lc: "dofoo".into(),
                enclosing_member_lc: None,
                params_count: 0,
                sig_fp: 0,
            },
            name: "DoFoo".into(),
            is_trigger: false,
            access: Access::Public,
            tier: TrustTier::SymbolOnly,
            event_subscribers: vec![],
            subscriber_instance_manual: false,
            publisher_kind: None,
            include_sender: None,
            abi_routine_kind: Some(AbiRoutineKind::Procedure),
            abi_event_kind: Some(AbiEventKind::None),
            param_sig_key: String::new(),
            return_type: None,
            return_type_id: None,
            abi_overload_collapsed: false,
        }];

        let mut topology = DependencyGraph::default();
        topology.add_dependency(ws_ref, dep_ref);

        let obj_index = ObjectIndex::build(&objects);
        let graph = ProgramGraph {
            apps,
            topology,
            objects,
            routines,
            obj_index,
            ..Default::default()
        };
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &[]);

        let from_obj = graph
            .objects
            .iter()
            .find(|o| o.id == ext_obj_id)
            .expect("MyRptExt must exist");

        // Sanity: the arity-DEFERRED existence scan reports "exists" even
        // though the sole candidate is arity 0 and we are about to probe
        // arity 2 — proving the leak vector this test guards against is real
        // at the boolean layer, not merely hypothetical.
        assert!(
            object_has_visible_member_candidate(
                &base_obj_id,
                TrustTier::SymbolOnly,
                "dofoo",
                2,
                &from_obj.id,
                &graph,
                &index,
            ),
            "SymbolOnly existence scan is deliberately arity-deferred — it must \
             report 'exists' regardless of the requested arity"
        );

        // The REAL resolution call (Step 2 of resolve_bare) must NOT emit a
        // route to the wrong-arity candidate despite that existence signal.
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
            "existence-only must never leak into a false emission at the wrong \
             arity; got {r:?}"
        );
        assert!(
            matches!(
                r.evidence,
                Evidence::Unknown(UnknownReason::OverloadAmbiguous)
            ),
            "name found, no arity-matched overload → Unknown(OverloadAmbiguous); \
             got {r:?}"
        );
        assert_eq!(r.witness, Witness::None);
    }

    // -----------------------------------------------------------------------
    // Task 2 round-2 addendum: PLAIN-DISPATCH MARKER GUARD (round-1
    // critical). Before this fix, `abi_overload_collapsed` gated ONLY the
    // chain-type-query boundary (`routine_node_for_type_query`) — a marked
    // survivor could still resolve CONFIDENTLY via ordinary PLAIN dispatch
    // (`resolve_in_object`'s single-visible-candidate arm). These two
    // fixtures build a minimal graph with the marker ALREADY set (bypassing
    // ingestion/dedup entirely — this targets the SELECTION guard in
    // isolation) and prove: (f) a MARKED candidate declines even though it
    // is the sole arity-matched, visible candidate; (control) an UNMARKED
    // candidate in the identical shape still resolves normally — the guard
    // must not over-decline.
    // -----------------------------------------------------------------------

    fn plain_dispatch_marker_guard_fixture(
        collapsed: bool,
    ) -> (ProgramGraph, ResolveIndex, BodyMap<'static>, ObjectNodeId) {
        let ws_id = make_app_id("WS");
        let dep_id = make_app_id("DepApp");

        let mut apps = AppRegistry::default();
        let ws_ref = apps.intern(&ws_id);
        let dep_ref = apps.intern(&dep_id);

        let caller_obj_id = ObjectNodeId {
            app: ws_ref,
            kind: ObjectKind::Codeunit,
            key: ObjKey::Id(50600),
        };
        let dep_obj_id = ObjectNodeId {
            app: dep_ref,
            kind: ObjectKind::Codeunit,
            key: ObjKey::Id(60104),
        };

        let objects = vec![
            ObjectNode {
                id: caller_obj_id.clone(),
                name: "Caller".into(),
                declared_id: Some(50600),
                extends_target: None,
                implements: vec![],
                tier: TrustTier::Workspace,
                source_table: None,
                table_no: None,
                source_table_temporary: false,
                page_controls: vec![],
                fields: vec![],
                dataitems: vec![],
            },
            ObjectNode {
                id: dep_obj_id.clone(),
                name: "Dep Collapse".into(),
                declared_id: Some(60104),
                extends_target: None,
                implements: vec![],
                tier: TrustTier::SymbolOnly,
                source_table: None,
                table_no: None,
                source_table_temporary: false,
                page_controls: vec![],
                fields: vec![],
                dataitems: vec![],
            },
        ];

        // The ONLY "Get" overload visible at arity 1 — carries `abi_overload_
        // collapsed` directly (simulating dedup's post-collapse output)
        // rather than exercising real ABI ingestion, to isolate the
        // SELECTION guard from the fp/dedup mechanics Task 2's other tests
        // already cover.
        let routines = vec![RoutineNode {
            id: RoutineNodeId {
                object: dep_obj_id.clone(),
                name_lc: "get".into(),
                enclosing_member_lc: None,
                params_count: 1,
                sig_fp: 777,
            },
            name: "Get".into(),
            is_trigger: false,
            access: Access::Public,
            tier: TrustTier::SymbolOnly,
            event_subscribers: vec![],
            subscriber_instance_manual: false,
            publisher_kind: None,
            include_sender: None,
            abi_routine_kind: Some(AbiRoutineKind::Procedure),
            abi_event_kind: Some(AbiEventKind::None),
            param_sig_key: String::new(),
            return_type: Some("Codeunit \"Dep Http Content\"".into()),
            return_type_id: Some(("Dep Http Content".into(), 60101)),
            abi_overload_collapsed: collapsed,
        }];

        let mut topology = DependencyGraph::default();
        topology.add_dependency(ws_ref, dep_ref);

        let obj_index = ObjectIndex::build(&objects);
        let graph = ProgramGraph {
            apps,
            topology,
            objects,
            routines,
            obj_index,
            ..Default::default()
        };
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &[]);
        (graph, index, body_map, caller_obj_id)
    }

    #[test]
    fn plain_dispatch_declines_on_collapse_marked_candidate() {
        use crate::program::resolve::receiver::ReceiverType;

        let (graph, index, body_map, caller_obj_id) = plain_dispatch_marker_guard_fixture(true);
        let from_obj = graph
            .objects
            .iter()
            .find(|o| o.id == caller_obj_id)
            .expect("Caller must exist");

        let receiver = ReceiverType::Object {
            kind: ObjectKind::Codeunit,
            name_lc: "dep collapse".into(),
            id: None,
        };
        let (_shape, routes) =
            resolve_member(&receiver, "get", 1, from_obj, &graph, &index, &body_map);

        assert_eq!(routes.len(), 1);
        let r = &routes[0];
        assert_eq!(
            r.target,
            RouteTarget::Unresolved,
            "a collapse-MARKED candidate must never resolve confidently via \
             PLAIN dispatch either — only the chain-type-query boundary was \
             guarded before this fix; got {r:?}"
        );
        assert!(
            matches!(
                r.evidence,
                Evidence::Unknown(UnknownReason::OverloadAmbiguous)
            ),
            "expected Unknown(OverloadAmbiguous); got {r:?}"
        );
        assert_eq!(r.witness, Witness::None);
    }

    /// Control: the IDENTICAL fixture shape, but UNMARKED — proves the new
    /// guard does not over-decline a genuinely trustworthy sole ABI
    /// candidate (the CDO-neutrality-critical property: 0 marked routines
    /// on CDO means this guard is dormant there by construction).
    #[test]
    fn plain_dispatch_resolves_unmarked_candidate_normally() {
        use crate::program::resolve::receiver::ReceiverType;

        let (graph, index, body_map, caller_obj_id) = plain_dispatch_marker_guard_fixture(false);
        let from_obj = graph
            .objects
            .iter()
            .find(|o| o.id == caller_obj_id)
            .expect("Caller must exist");

        let receiver = ReceiverType::Object {
            kind: ObjectKind::Codeunit,
            name_lc: "dep collapse".into(),
            id: None,
        };
        let (_shape, routes) =
            resolve_member(&receiver, "get", 1, from_obj, &graph, &index, &body_map);

        assert_eq!(routes.len(), 1);
        let r = &routes[0];
        assert_eq!(r.evidence, Evidence::Opaque);
        assert!(
            matches!(r.target, RouteTarget::AbiSymbol { .. }),
            "an UNMARKED sole ABI candidate must still resolve normally; got {r:?}"
        );
    }

    // -----------------------------------------------------------------------
    // Task 2 review fix: collapse-marker guard at every `make_routine_route`
    // call site. `resolve_in_object`'s PLAIN-DISPATCH MARKER GUARD above (and
    // `routine_node_for_type_query`'s CHAIN-type-query guard) were the only
    // two `abi_overload_collapsed` consultation points before this fix — but
    // FOUR other call sites look up a routine directly by ROLE (entry
    // trigger / trigger fan-out / event subscriber) rather than through
    // either of those name+arity selection boundaries, so a collapse-marked
    // survivor could still reach a confident `Opaque`/`Source` route through
    // any of them:
    //   1. `resolve_object_run` (Codeunit.Run/Page.RunModal/Report.Run's
    //      entry-trigger lookup).
    //   2. `resolve_member`'s own inline `Codeunit.Run(arity<=1)` special
    //      case (the member-call-shaped mirror of (1)).
    //   3. `resolve_implicit_trigger`'s base-table + TableExtension trigger
    //      fan-out (data-is-control-flow Multicast routes).
    //   4. `emit_event_flow_edges`'s subscriber fan-out.
    // Each pair below builds the identical marked-vs-unmarked minimal-graph
    // shape `plain_dispatch_marker_guard_fixture` established (the marker
    // set directly, bypassing ingestion/dedup — isolates the SELECTION
    // guard) and proves: (f) a MARKED candidate declines at THIS specific
    // site; (control) an UNMARKED candidate in the identical shape still
    // resolves normally — the guard must neither miss a site nor
    // over-decline an honest one.
    // -----------------------------------------------------------------------

    /// Shared fixture for the two entry-trigger bypass sites
    /// (`resolve_object_run`; `resolve_member`'s inline `Codeunit.Run
    /// (arity<=1)` arm): a SymbolOnly dep Codeunit whose SOLE `onrun` entry
    /// trigger candidate (0-arg — `sig_fp` folds to the fixed `0` for an
    /// empty `Parameters[]`, see `abi_ingest::param_type_fp`) carries
    /// `abi_overload_collapsed` directly, simulating dedup's post-collapse
    /// output for a literal duplicate raw `OnRun` JSON entry.
    fn entry_trigger_marker_guard_fixture(
        collapsed: bool,
    ) -> (
        ProgramGraph,
        ResolveIndex,
        BodyMap<'static>,
        ObjectNodeId,
        ObjectNodeId,
    ) {
        let ws_id = make_app_id("WS");
        let dep_id = make_app_id("DepApp");

        let mut apps = AppRegistry::default();
        let ws_ref = apps.intern(&ws_id);
        let dep_ref = apps.intern(&dep_id);

        let caller_obj_id = ObjectNodeId {
            app: ws_ref,
            kind: ObjectKind::Codeunit,
            key: ObjKey::Id(50610),
        };
        let dep_obj_id = ObjectNodeId {
            app: dep_ref,
            kind: ObjectKind::Codeunit,
            key: ObjKey::Id(60150),
        };

        let objects = vec![
            ObjectNode {
                id: caller_obj_id.clone(),
                name: "Caller".into(),
                declared_id: Some(50610),
                extends_target: None,
                implements: vec![],
                tier: TrustTier::Workspace,
                source_table: None,
                table_no: None,
                source_table_temporary: false,
                page_controls: vec![],
                fields: vec![],
                dataitems: vec![],
            },
            ObjectNode {
                id: dep_obj_id.clone(),
                name: "Dep Trigger Collapse".into(),
                declared_id: Some(60150),
                extends_target: None,
                implements: vec![],
                tier: TrustTier::SymbolOnly,
                source_table: None,
                table_no: None,
                source_table_temporary: false,
                page_controls: vec![],
                fields: vec![],
                dataitems: vec![],
            },
        ];

        let routines = vec![RoutineNode {
            id: RoutineNodeId {
                object: dep_obj_id.clone(),
                name_lc: "onrun".into(),
                enclosing_member_lc: None,
                params_count: 0,
                sig_fp: 0,
            },
            name: "OnRun".into(),
            is_trigger: false,
            access: Access::Public,
            tier: TrustTier::SymbolOnly,
            event_subscribers: vec![],
            subscriber_instance_manual: false,
            publisher_kind: None,
            include_sender: None,
            abi_routine_kind: Some(AbiRoutineKind::Procedure),
            abi_event_kind: Some(AbiEventKind::None),
            param_sig_key: String::new(),
            return_type: None,
            return_type_id: None,
            abi_overload_collapsed: collapsed,
        }];

        let mut topology = DependencyGraph::default();
        topology.add_dependency(ws_ref, dep_ref);

        let obj_index = ObjectIndex::build(&objects);
        let graph = ProgramGraph {
            apps,
            topology,
            objects,
            routines,
            obj_index,
            ..Default::default()
        };
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &[]);
        (graph, index, body_map, caller_obj_id, dep_obj_id)
    }

    // --- Site 1: `resolve_object_run` ---------------------------------------

    #[test]
    fn object_run_declines_on_collapse_marked_entry_trigger() {
        let (graph, index, body_map, _caller_obj_id, _dep_obj_id) =
            entry_trigger_marker_guard_fixture(true);
        let from = graph.apps.find_by_name("WS").expect("WS app");

        let (shape, completeness, routes) = resolve_object_run(
            from,
            ObjectKind::Codeunit,
            Some("Dep Trigger Collapse"),
            true,
            &graph,
            &index,
            &body_map,
        );

        assert_eq!(shape, DispatchShape::Exact);
        assert_eq!(completeness, SetCompleteness::Complete);
        assert_eq!(routes.len(), 1);
        let r = &routes[0];
        assert_eq!(
            r.target,
            RouteTarget::Unresolved,
            "a collapse-MARKED entry trigger must never resolve confidently via \
             Codeunit.Run's resolve_object_run path either — this site bypasses \
             resolve_in_object entirely (Task 2 review fix); got {r:?}"
        );
        assert!(
            matches!(
                r.evidence,
                Evidence::Unknown(UnknownReason::OverloadAmbiguous)
            ),
            "expected Unknown(OverloadAmbiguous); got {r:?}"
        );
    }

    #[test]
    fn object_run_resolves_unmarked_entry_trigger_normally() {
        let (graph, index, body_map, _caller_obj_id, _dep_obj_id) =
            entry_trigger_marker_guard_fixture(false);
        let from = graph.apps.find_by_name("WS").expect("WS app");

        let (_shape, _completeness, routes) = resolve_object_run(
            from,
            ObjectKind::Codeunit,
            Some("Dep Trigger Collapse"),
            true,
            &graph,
            &index,
            &body_map,
        );

        assert_eq!(routes.len(), 1);
        let r = &routes[0];
        assert_eq!(r.evidence, Evidence::Opaque);
        assert!(
            matches!(r.target, RouteTarget::AbiSymbol { .. }),
            "an UNMARKED sole entry trigger must still resolve normally; got {r:?}"
        );
    }

    // --- Site 2: `resolve_member`'s inline Codeunit.Run(arity<=1) arm ------

    #[test]
    fn resolve_member_object_run_arm_declines_on_collapse_marked_entry_trigger() {
        use crate::program::resolve::receiver::ReceiverType;

        let (graph, index, body_map, caller_obj_id, _dep_obj_id) =
            entry_trigger_marker_guard_fixture(true);
        let from_obj = graph
            .objects
            .iter()
            .find(|o| o.id == caller_obj_id)
            .expect("Caller must exist");

        let receiver = ReceiverType::Object {
            kind: ObjectKind::Codeunit,
            name_lc: "dep trigger collapse".into(),
            id: None,
        };
        let (shape, routes) =
            resolve_member(&receiver, "run", 0, from_obj, &graph, &index, &body_map);

        assert_eq!(shape, DispatchShape::Exact);
        assert_eq!(routes.len(), 1);
        let r = &routes[0];
        assert_eq!(
            r.target,
            RouteTarget::Unresolved,
            "a collapse-MARKED entry trigger must never resolve confidently via \
             resolve_member's inline Codeunit.Run(arity<=1) arm either — this \
             site bypasses resolve_in_object entirely (Task 2 review fix); \
             got {r:?}"
        );
        assert!(
            matches!(
                r.evidence,
                Evidence::Unknown(UnknownReason::OverloadAmbiguous)
            ),
            "expected Unknown(OverloadAmbiguous); got {r:?}"
        );
    }

    #[test]
    fn resolve_member_object_run_arm_resolves_unmarked_entry_trigger_normally() {
        use crate::program::resolve::receiver::ReceiverType;

        let (graph, index, body_map, caller_obj_id, _dep_obj_id) =
            entry_trigger_marker_guard_fixture(false);
        let from_obj = graph
            .objects
            .iter()
            .find(|o| o.id == caller_obj_id)
            .expect("Caller must exist");

        let receiver = ReceiverType::Object {
            kind: ObjectKind::Codeunit,
            name_lc: "dep trigger collapse".into(),
            id: None,
        };
        let (_shape, routes) =
            resolve_member(&receiver, "run", 0, from_obj, &graph, &index, &body_map);

        assert_eq!(routes.len(), 1);
        let r = &routes[0];
        assert_eq!(r.evidence, Evidence::Opaque);
        assert!(
            matches!(r.target, RouteTarget::AbiSymbol { .. }),
            "an UNMARKED sole entry trigger must still resolve normally; got {r:?}"
        );
    }

    // --- Site 3: `resolve_implicit_trigger` ---------------------------------

    /// Fixture for the trigger-fan-out bypass site (`resolve_implicit_
    /// trigger`): a SymbolOnly dep Table whose SOLE `oninsert` object-level
    /// trigger candidate carries `abi_overload_collapsed` directly.
    fn implicit_trigger_marker_guard_fixture(
        collapsed: bool,
    ) -> (ProgramGraph, ResolveIndex, BodyMap<'static>, ObjectNodeId) {
        let dep_id = make_app_id("DepApp");
        let mut apps = AppRegistry::default();
        let dep_ref = apps.intern(&dep_id);

        let table_obj_id = ObjectNodeId {
            app: dep_ref,
            kind: ObjectKind::Table,
            key: ObjKey::Id(60160),
        };

        let objects = vec![ObjectNode {
            id: table_obj_id.clone(),
            name: "Dep Trigger Table".into(),
            declared_id: Some(60160),
            extends_target: None,
            implements: vec![],
            tier: TrustTier::SymbolOnly,
            source_table: None,
            table_no: None,
            source_table_temporary: false,
            page_controls: vec![],
            fields: vec![],
            dataitems: vec![],
        }];

        let routines = vec![RoutineNode {
            id: RoutineNodeId {
                object: table_obj_id.clone(),
                name_lc: "oninsert".into(),
                enclosing_member_lc: None,
                params_count: 0,
                sig_fp: 0,
            },
            name: "OnInsert".into(),
            is_trigger: false,
            access: Access::Public,
            tier: TrustTier::SymbolOnly,
            event_subscribers: vec![],
            subscriber_instance_manual: false,
            publisher_kind: None,
            include_sender: None,
            abi_routine_kind: Some(AbiRoutineKind::Procedure),
            abi_event_kind: Some(AbiEventKind::None),
            param_sig_key: String::new(),
            return_type: None,
            return_type_id: None,
            abi_overload_collapsed: collapsed,
        }];

        let obj_index = ObjectIndex::build(&objects);
        let graph = ProgramGraph {
            apps,
            topology: DependencyGraph::default(),
            objects,
            routines,
            obj_index,
            ..Default::default()
        };
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &[]);
        (graph, index, body_map, table_obj_id)
    }

    #[test]
    fn implicit_trigger_declines_route_for_collapse_marked_trigger() {
        let (graph, index, body_map, table_obj_id) = implicit_trigger_marker_guard_fixture(true);
        let table_obj = graph
            .objects
            .iter()
            .find(|o| o.id == table_obj_id)
            .expect("table");

        let (shape, completeness, routes) =
            resolve_implicit_trigger("insert", table_obj, &graph, &index, &body_map);

        assert_eq!(shape, DispatchShape::Multicast);
        assert_eq!(
            completeness,
            SetCompleteness::Partial {
                reason: OpenWorldReason::ReverseDependentExtensions
            }
        );
        assert_eq!(
            routes.len(),
            1,
            "a marked trigger must still contribute ONE honest Unknown route, \
             never silently vanish; got {routes:?}"
        );
        let r = &routes[0];
        assert_eq!(
            r.target,
            RouteTarget::Unresolved,
            "a collapse-MARKED table trigger must never resolve confidently via \
             resolve_implicit_trigger's fan-out either — this site bypasses \
             resolve_in_object entirely (Task 2 review fix); got {r:?}"
        );
        assert!(
            matches!(
                r.evidence,
                Evidence::Unknown(UnknownReason::OverloadAmbiguous)
            ),
            "expected Unknown(OverloadAmbiguous); got {r:?}"
        );
    }

    #[test]
    fn implicit_trigger_resolves_unmarked_trigger_normally() {
        let (graph, index, body_map, table_obj_id) = implicit_trigger_marker_guard_fixture(false);
        let table_obj = graph
            .objects
            .iter()
            .find(|o| o.id == table_obj_id)
            .expect("table");

        let (_shape, _completeness, routes) =
            resolve_implicit_trigger("insert", table_obj, &graph, &index, &body_map);

        assert_eq!(routes.len(), 1);
        let r = &routes[0];
        assert_eq!(r.evidence, Evidence::Opaque);
        assert!(
            matches!(r.target, RouteTarget::AbiSymbol { .. }),
            "an UNMARKED trigger must still resolve normally; got {r:?}"
        );
    }

    // --- Site 4: `emit_event_flow_edges` ------------------------------------

    /// Fixture for the event-subscriber-fan-out bypass site
    /// (`emit_event_flow_edges`): a SymbolOnly dep publisher/subscriber pair
    /// where the SOLE matching subscriber candidate carries `abi_overload_
    /// collapsed` directly.
    fn event_flow_marker_guard_fixture(
        collapsed: bool,
    ) -> (ProgramGraph, ResolveIndex, BodyMap<'static>) {
        let dep_id = make_app_id("DepApp");
        let mut apps = AppRegistry::default();
        let dep_ref = apps.intern(&dep_id);

        let pub_obj_id = ObjectNodeId {
            app: dep_ref,
            kind: ObjectKind::Codeunit,
            key: ObjKey::Id(60170),
        };
        let sub_obj_id = ObjectNodeId {
            app: dep_ref,
            kind: ObjectKind::Codeunit,
            key: ObjKey::Id(60171),
        };

        let objects = vec![
            ObjectNode {
                id: pub_obj_id.clone(),
                name: "Dep Evt Pub".into(),
                declared_id: Some(60170),
                extends_target: None,
                implements: vec![],
                tier: TrustTier::SymbolOnly,
                source_table: None,
                table_no: None,
                source_table_temporary: false,
                page_controls: vec![],
                fields: vec![],
                dataitems: vec![],
            },
            ObjectNode {
                id: sub_obj_id.clone(),
                name: "Dep Evt Sub".into(),
                declared_id: Some(60171),
                extends_target: None,
                implements: vec![],
                tier: TrustTier::SymbolOnly,
                source_table: None,
                table_no: None,
                source_table_temporary: false,
                page_controls: vec![],
                fields: vec![],
                dataitems: vec![],
            },
        ];

        let publisher = RoutineNode {
            id: RoutineNodeId {
                object: pub_obj_id.clone(),
                name_lc: "onafterx".into(),
                enclosing_member_lc: None,
                params_count: 0,
                sig_fp: 0,
            },
            name: "OnAfterX".into(),
            is_trigger: false,
            access: Access::Public,
            tier: TrustTier::SymbolOnly,
            event_subscribers: vec![],
            subscriber_instance_manual: false,
            publisher_kind: Some(crate::program::resolve::event::PublisherKind::Integration),
            include_sender: Some(false),
            abi_routine_kind: Some(AbiRoutineKind::EventPublisher),
            abi_event_kind: Some(AbiEventKind::Integration),
            param_sig_key: String::new(),
            return_type: None,
            return_type_id: None,
            abi_overload_collapsed: false,
        };

        let subscriber = RoutineNode {
            id: RoutineNodeId {
                object: sub_obj_id.clone(),
                name_lc: "onafterxhandler".into(),
                enclosing_member_lc: None,
                params_count: 0,
                sig_fp: 0,
            },
            name: "OnAfterXHandler".into(),
            is_trigger: false,
            access: Access::Public,
            tier: TrustTier::SymbolOnly,
            event_subscribers: vec![crate::program::resolve::event::ParsedSubscriberArgs {
                publisher_object_type: "codeunit".into(),
                publisher_name: "dep evt pub".into(),
                event_name: "onafterx".into(),
                element: None,
                skip_on_missing_license: false,
                skip_on_missing_permission: false,
            }],
            subscriber_instance_manual: false,
            publisher_kind: None,
            include_sender: None,
            abi_routine_kind: Some(AbiRoutineKind::EventSubscriber),
            abi_event_kind: Some(AbiEventKind::None),
            param_sig_key: String::new(),
            return_type: None,
            return_type_id: None,
            abi_overload_collapsed: collapsed,
        };

        let mut routines = vec![publisher, subscriber];
        routines.sort_by(|a, b| a.id.cmp(&b.id));

        let obj_index = ObjectIndex::build(&objects);
        let graph = ProgramGraph {
            apps,
            topology: DependencyGraph::default(),
            objects,
            routines,
            obj_index,
            ..Default::default()
        };
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &[]);
        (graph, index, body_map)
    }

    #[test]
    fn event_flow_skips_route_for_collapse_marked_subscriber() {
        let (graph, index, body_map) = event_flow_marker_guard_fixture(true);
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
            "a collapse-MARKED subscriber must never resolve confidently via \
             emit_event_flow_edges's subscriber fan-out either — this site \
             bypasses resolve_in_object entirely (Task 2 review fix): the \
             marked subscriber's route must be SKIPPED, not silently emitted \
             Opaque; got {:?}",
            e.routes
        );
    }

    #[test]
    fn event_flow_includes_route_for_unmarked_subscriber_normally() {
        let (graph, index, body_map) = event_flow_marker_guard_fixture(false);
        let edges = emit_event_flow_edges(&graph, &index, &body_map);
        let event_edges: Vec<&Edge> = edges
            .iter()
            .filter(|e| e.kind == EdgeKind::EventFlow)
            .collect();
        assert_eq!(event_edges.len(), 1);
        let e = event_edges[0];
        assert_eq!(
            e.routes.len(),
            1,
            "an UNMARKED subscriber must still be included normally"
        );
        let r = &e.routes[0];
        assert_eq!(r.evidence, Evidence::Opaque);
        assert!(matches!(r.target, RouteTarget::AbiSymbol { .. }));
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
        assert!(
            matches!(unk_route.evidence, Evidence::Unknown(_)),
            "Unknown evidence"
        );
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
        assert!(
            matches!(r.evidence, Evidence::Unknown(_)),
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
        assert!(matches!(r.evidence, Evidence::Unknown(_)));
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
        assert!(
            matches!(r.evidence, Evidence::Unknown(_)),
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
        assert!(
            matches!(r.evidence, Evidence::Unknown(_)),
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
            ..Default::default()
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
            fields: vec![],
            dataitems: vec![],
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
        assert!(matches!(routes2[0].evidence, Evidence::Unknown(_)));
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
        assert!(matches!(routes[0].evidence, Evidence::Unknown(_)));
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
        assert!(matches!(routes[0].evidence, Evidence::Unknown(_)));
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
        assert!(matches!(routes[0].evidence, Evidence::Unknown(_)));
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
        assert!(matches!(routes[0].evidence, Evidence::Unknown(_)));
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
        assert!(matches!(routes[0].evidence, Evidence::Unknown(_)));
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
        assert!(
            matches!(routes[0].evidence, Evidence::Unknown(_)),
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
        assert!(matches!(routes[0].evidence, Evidence::Unknown(_)));
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
        assert!(
            matches!(routes[0].evidence, Evidence::Unknown(_)),
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
        assert!(matches!(routes[0].evidence, Evidence::Unknown(_)));
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
        assert!(matches!(routes[0].evidence, Evidence::Unknown(_)));
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
        assert!(matches!(routes[0].evidence, Evidence::Unknown(_)));
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
        assert!(matches!(routes[0].evidence, Evidence::Unknown(_)));
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
        assert!(matches!(routes[0].evidence, Evidence::Unknown(_)));
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
        assert!(matches!(routes[0].evidence, Evidence::Unknown(_)));
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
        assert!(matches!(routes[0].evidence, Evidence::Unknown(_)));
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
        assert!(matches!(routes[0].evidence, Evidence::Unknown(_)));
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
        assert!(matches!(routes[0].evidence, Evidence::Unknown(_)));
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
        assert!(matches!(routes[0].evidence, Evidence::Unknown(_)));
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
        assert!(matches!(routes[0].evidence, Evidence::Unknown(_)));
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
        assert!(matches!(unresolved_route.evidence, Evidence::Unknown(_)));
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
        assert!(matches!(routes[0].evidence, Evidence::Unknown(_)));
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

    // (c2) subscriber to a platform TABLE event (OnAfterDeleteEvent) wires via a
    // synthetic `PublisherKind::Platform` publisher injected on the table — the
    // fix for orphaned auto-event subscribers (data-is-control-flow wiring).
    #[test]
    fn platform_table_event_subscriber_wires_via_synthetic_publisher() {
        let tbl_src: &'static str = r#"
table 18 Customer
{
    fields { field(1; "No."; Code[20]) { } }
}
"#;
        let sub_src: &'static str = r#"
codeunit 50710 "CustDeleteSub"
{
    [EventSubscriber(ObjectType::Table, Database::Customer, 'OnAfterDeleteEvent', '', true, false)]
    local procedure OnDeleteCustomer(var Rec: Record Customer; RunTrigger: Boolean)
    begin
    end;
}
"#;
        let app_id = make_app_id("TestApp");
        let units = vec![
            make_unit(app_id.clone(), "Customer.al", tbl_src),
            make_unit(app_id, "CustDeleteSub.al", sub_src),
        ];
        // build_graph is the local test helper (extract only); run the injection
        // that build_program_graph applies in production.
        let mut graph = build_graph(&units, None);
        crate::program::build::inject_platform_event_publishers(&mut graph);

        // A synthetic Platform publisher for OnAfterDeleteEvent now sits on Customer.
        let cust = graph
            .objects
            .iter()
            .find(|o| o.name == "Customer")
            .expect("Customer table");
        let synth = graph
            .routines
            .iter()
            .find(|r| {
                r.id.object == cust.id
                    && r.id.name_lc == "onafterdeleteevent"
                    && r.publisher_kind
                        == Some(crate::program::resolve::event::PublisherKind::Platform)
            })
            .expect("synthetic platform publisher injected on Customer");

        // The subscriber binds to it → exactly one EventFlow edge, one route, Resolved.
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);
        let edges = emit_event_flow_edges(&graph, &index, &body_map);
        let e = edges
            .iter()
            .find(|e| e.from == synth.id)
            .expect("EventFlow edge from the synthetic platform publisher");
        assert_eq!(
            e.routes.len(),
            1,
            "subscriber must bind as the single route"
        );
        assert_eq!(
            classify_obligation(e),
            ObligationOutcome::Resolved,
            "a bound subscriber makes the platform event Resolved, not orphaned"
        );
    }

    // (c3) subscriber to a platform PAGE event (OnOpenPageEvent) wires via a
    // synthetic Platform publisher on the page.
    #[test]
    fn platform_page_event_subscriber_wires_via_synthetic_publisher() {
        let pg_src: &'static str = r#"
page 21 "Customer Card"
{
    layout { area(Content) { } }
}
"#;
        let sub_src: &'static str = r#"
codeunit 50711 "CustCardOpenSub"
{
    [EventSubscriber(ObjectType::Page, Page::"Customer Card", 'OnOpenPageEvent', '', false, false)]
    local procedure OnOpenCustCard(var Rec: Record Customer)
    begin
    end;
}
"#;
        let app_id = make_app_id("TestApp");
        let units = vec![
            make_unit(app_id.clone(), "CustomerCard.al", pg_src),
            make_unit(app_id, "CustCardOpenSub.al", sub_src),
        ];
        let mut graph = build_graph(&units, None);
        crate::program::build::inject_platform_event_publishers(&mut graph);

        let pg = graph
            .objects
            .iter()
            .find(|o| o.name == "Customer Card")
            .expect("Customer Card page");
        let synth = graph
            .routines
            .iter()
            .find(|r| {
                r.id.object == pg.id
                    && r.id.name_lc == "onopenpageevent"
                    && r.publisher_kind
                        == Some(crate::program::resolve::event::PublisherKind::Platform)
            })
            .expect("synthetic platform publisher injected on the page");

        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);
        let edges = emit_event_flow_edges(&graph, &index, &body_map);
        let e = edges
            .iter()
            .find(|e| e.from == synth.id)
            .expect("EventFlow edge from the synthetic page publisher");
        assert_eq!(
            e.routes.len(),
            1,
            "subscriber must bind as the single route"
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
            fields: vec![],
            dataitems: vec![],
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
            include_sender: None,
            abi_routine_kind: Some(AbiRoutineKind::EventPublisher),
            abi_event_kind: Some(AbiEventKind::Integration),
            param_sig_key: String::new(),
            return_type: None,
            return_type_id: None,
            abi_overload_collapsed: false,
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
            include_sender: None,
            abi_routine_kind: Some(AbiRoutineKind::Procedure),
            abi_event_kind: Some(AbiEventKind::None),
            param_sig_key: String::new(),
            return_type: None,
            return_type_id: None,
            abi_overload_collapsed: false,
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
            ..Default::default()
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

    // -----------------------------------------------------------------------
    // Task 1 (feat/resolve-access-uniform-and-compound-receiver): per-candidate
    // access filter in `resolve_in_object` — closes the `ReceiverType::Object`
    // arm (gap D) + both `Interface`-impl delegates (gaps F/G), which
    // previously did ZERO access filtering. `Codeunit.Run`/`resolve_object_run`
    // and event-subscriber dispatch are UNTOUCHED (they bypass
    // `resolve_in_object` entirely) — the negative/control fixtures below pin
    // that boundary too. See `.superpowers/sdd/task-1-report.md` and
    // `tests/r0-corpus/ws-object-interface-visibility/` for the full matrix +
    // compiler-semantics writeup.
    // -----------------------------------------------------------------------

    // --- POSITIVE controls: must still resolve to Source -------------------

    // (D-pos-1) Object receiver, cross-app, `public` method.
    #[test]
    fn resolve_member_object_cross_app_public_method_resolves_to_source() {
        use crate::program::resolve::receiver::ReceiverType;

        let src_target: &'static str = r#"
codeunit 53900 "PubXTarget"
{
    procedure DoWork()
    begin
    end;
}
"#;
        let src_caller: &'static str = r#"
codeunit 53901 "PubXCaller"
{
    procedure Trigger()
    begin
    end;
}
"#;
        let app_a = make_app_id("PrimaryApp1");
        let app_b = make_app_id("DepApp1");
        let unit_target = make_unit(app_b, "PubXTarget.al", src_target);
        let unit_caller = make_unit(app_a, "PubXCaller.al", src_caller);
        let units = [unit_target, unit_caller];
        let graph = build_graph_multi_dep(&units, &[("PrimaryApp1", "DepApp1")]);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let from_obj = find_obj(&graph, "PubXCaller");
        let receiver = ReceiverType::Object {
            kind: ObjectKind::Codeunit,
            name_lc: "pubxtarget".into(),
            id: None,
        };
        let (shape, routes) =
            resolve_member(&receiver, "dowork", 0, from_obj, &graph, &index, &body_map);

        assert_eq!(shape, DispatchShape::Exact);
        assert_eq!(routes.len(), 1);
        assert!(
            matches!(routes[0].target, RouteTarget::Routine(_)),
            "a `public` cross-app Object-receiver method must still resolve \
             to Source (gap D positive control); got {:?}",
            routes[0].target
        );
        assert_eq!(routes[0].evidence, Evidence::Source);
    }

    // (D-pos-2) Object receiver, same-app, `internal` method.
    #[test]
    fn resolve_member_object_same_app_internal_method_resolves_to_source() {
        use crate::program::resolve::receiver::ReceiverType;

        let src_target: &'static str = r#"
codeunit 53910 "IntXTarget"
{
    internal procedure DoWork()
    begin
    end;
}
"#;
        let src_caller: &'static str = r#"
codeunit 53911 "IntXCaller"
{
    procedure Trigger()
    begin
    end;
}
"#;
        let app_id = make_app_id("TestApp");
        let unit_target = make_unit(app_id.clone(), "IntXTarget.al", src_target);
        let unit_caller = make_unit(app_id, "IntXCaller.al", src_caller);
        let units = [unit_target, unit_caller];
        let graph = build_graph(&units, None);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let from_obj = find_obj(&graph, "IntXCaller");
        let receiver = ReceiverType::Object {
            kind: ObjectKind::Codeunit,
            name_lc: "intxtarget".into(),
            id: None,
        };
        let (shape, routes) =
            resolve_member(&receiver, "dowork", 0, from_obj, &graph, &index, &body_map);

        assert_eq!(shape, DispatchShape::Exact);
        assert_eq!(routes.len(), 1);
        assert!(
            matches!(routes[0].target, RouteTarget::Routine(_)),
            "a same-app `internal` Object-receiver method must resolve to \
             Source (gap D positive control — Internal is app-scoped); got {:?}",
            routes[0].target
        );
        assert_eq!(routes[0].evidence, Evidence::Source);
    }

    // (D-pos-3) Object receiver, direct PageExtension → base Page `protected` method.
    #[test]
    fn resolve_member_object_direct_extension_protected_method_resolves_to_source() {
        use crate::program::resolve::receiver::ReceiverType;

        let src_page: &'static str = r#"
page 53920 "ProtXBase"
{
    protected procedure Prot()
    begin
    end;
}
"#;
        let src_ext: &'static str = r#"
pageextension 53921 "ProtXBaseExt" extends ProtXBase
{
    procedure Trigger()
    begin
    end;
}
"#;
        let app_id = make_app_id("TestApp");
        let unit_page = make_unit(app_id.clone(), "ProtXBase.al", src_page);
        let unit_ext = make_unit(app_id, "ProtXBaseExt.al", src_ext);
        let units = [unit_page, unit_ext];
        let graph = build_graph(&units, None);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let from_obj = find_obj(&graph, "ProtXBaseExt");
        assert_eq!(from_obj.id.kind, ObjectKind::PageExtension);
        let receiver = ReceiverType::Object {
            kind: ObjectKind::Page,
            name_lc: "protxbase".into(),
            id: None,
        };
        let (shape, routes) =
            resolve_member(&receiver, "prot", 0, from_obj, &graph, &index, &body_map);

        assert_eq!(shape, DispatchShape::Exact);
        assert_eq!(routes.len(), 1);
        assert!(
            matches!(routes[0].target, RouteTarget::Routine(_)),
            "a direct PageExtension calling its base's `protected` method \
             via an Object receiver must resolve to Source (gap D positive \
             control); got {:?}",
            routes[0].target
        );
        assert_eq!(routes[0].evidence, Evidence::Source);
    }

    // (B-pos) bare call to the caller's own `local` procedure (gap B is
    // pre-gated/self-no-op — this pins that threading `from_object` through
    // Step 1 changed nothing observable).
    #[test]
    fn bare_own_local_procedure_resolves_to_source() {
        let src: &'static str = r#"
codeunit 53930 "BareLocalSelf"
{
    local procedure DoFoo()
    begin
    end;
}
"#;
        let app_id = make_app_id("TestApp");
        let unit = make_unit(app_id, "BareLocalSelf.al", src);
        let units = [unit];
        let graph = build_graph(&units, None);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let from_obj = find_obj(&graph, "BareLocalSelf");
        let routes = resolve_bare(
            from_obj,
            "dofoo",
            0,
            &graph,
            &index,
            &body_map,
            WithState::NoWithProven,
        );

        assert_eq!(routes.len(), 1);
        assert!(
            matches!(routes[0].target, RouteTarget::Routine(_)),
            "a bare call to the caller's OWN `local` procedure must resolve \
             to Source (gap B self-no-op control); got {:?}",
            routes[0].target
        );
        assert_eq!(routes[0].evidence, Evidence::Source);
    }

    // (E-pos) `this.LocalProc()` — SelfObject receiver dispatching to the
    // caller's own `local` procedure (gap E is pre-gated/self-no-op).
    #[test]
    fn resolve_member_self_object_local_procedure_resolves_to_source() {
        use crate::program::resolve::receiver::ReceiverType;

        let src: &'static str = r#"
codeunit 53940 "SelfLocalCaller"
{
    local procedure LocalProc()
    begin
    end;

    procedure Trigger()
    begin
        this.LocalProc();
    end;
}
"#;
        let app_id = make_app_id("TestApp");
        let unit = make_unit(app_id, "SelfLocalCaller.al", src);
        let units = [unit];
        let graph = build_graph(&units, None);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let from_obj = find_obj(&graph, "SelfLocalCaller");
        let (shape, routes) = resolve_member(
            &ReceiverType::SelfObject,
            "localproc",
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
            "`this.LocalProc()` (SelfObject receiver) must resolve its own \
             `local` procedure to Source (gap E self-no-op control); got {:?}",
            routes[0].target
        );
        assert_eq!(routes[0].evidence, Evidence::Source);
    }

    // --- NEGATIVES: must become honest Unknown ------------------------------

    // (D-neg-1) Object receiver, cross-app `internal` — pre-fix this
    // false-resolved to `RouteTarget::Routine(IntNTarget.Secret)`.
    #[test]
    fn resolve_member_object_cross_app_internal_method_excluded() {
        use crate::program::resolve::receiver::ReceiverType;

        let src_target: &'static str = r#"
codeunit 53950 "IntNTarget"
{
    internal procedure Secret()
    begin
    end;
}
"#;
        let src_caller: &'static str = r#"
codeunit 53951 "IntNCaller"
{
    procedure Trigger()
    begin
    end;
}
"#;
        let app_a = make_app_id("PrimaryApp2");
        let app_b = make_app_id("DepApp2");
        let unit_target = make_unit(app_b, "IntNTarget.al", src_target);
        let unit_caller = make_unit(app_a, "IntNCaller.al", src_caller);
        let units = [unit_target, unit_caller];
        let graph = build_graph_multi_dep(&units, &[("PrimaryApp2", "DepApp2")]);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let from_obj = find_obj(&graph, "IntNCaller");
        let receiver = ReceiverType::Object {
            kind: ObjectKind::Codeunit,
            name_lc: "intntarget".into(),
            id: None,
        };
        let (shape, routes) =
            resolve_member(&receiver, "secret", 0, from_obj, &graph, &index, &body_map);

        assert_eq!(shape, DispatchShape::Exact);
        assert_eq!(routes.len(), 1);
        assert_eq!(
            routes[0].target,
            RouteTarget::Unresolved,
            "a cross-app `internal` method reached via an Object receiver \
             must NOT resolve to Source (gap D — pre-fix this false-resolved \
             to RouteTarget::Routine(IntNTarget.Secret)); got {:?}",
            routes[0].target
        );
        assert!(matches!(routes[0].evidence, Evidence::Unknown(_)));
    }

    // -----------------------------------------------------------------------
    // Task 1.5 (feat/resolve-access-uniform-and-compound-receiver, inserted
    // after Task 1): `internalsVisibleTo` friend apps. Task 1 correctly fails
    // closed on cross-app `internal`, but 100% of the resulting
    // `InternalNotVisible` bucket measured against CDO turned out to be
    // AL-LEGAL friend calls (the declaring app's manifest explicitly lists
    // the caller app in `<InternalsVisibleTo>`). `internal_visible_across`
    // (above) models the friend exception; these fixtures pin the full
    // matrix: friend-authorized resolves, a true-stranger control still
    // declines, friendship doesn't imply the reverse direction, and same-app
    // `internal` is unaffected. See `.superpowers/sdd/task-1.5-report.md` and
    // `tests/r0-corpus/ws-friend-app-internal/` for the compiler-semantics
    // writeup.
    // -----------------------------------------------------------------------

    // (1.5-a) cross-app `internal`, declaring app lists the caller as a
    // friend → must resolve to Source. Pre-fix (Task 1 alone) this is
    // `Unknown`/`InternalNotVisible` — asserted exactly below before the fix
    // narrative note (kept as documentation of the exact prior route; the
    // assertion itself checks the POST-fix behavior since this test file
    // only ships the fixed code).
    #[test]
    fn resolve_member_object_cross_app_internal_friend_authorized_resolves_to_source() {
        use crate::program::resolve::receiver::ReceiverType;

        let src_target: &'static str = r#"
codeunit 53970 "FriendTarget"
{
    internal procedure Secret()
    begin
    end;
}
"#;
        let src_caller: &'static str = r#"
codeunit 53971 "FriendCaller"
{
    procedure Trigger()
    begin
    end;
}
"#;
        // FriendTarget's app ("DepAppFriend") declares FriendCaller's app
        // ("PrimaryAppFriend") a friend via <InternalsVisibleTo>.
        let app_a = make_app_id("PrimaryAppFriend");
        let app_b = make_app_id("DepAppFriend");
        let unit_target = make_unit(app_b, "FriendTarget.al", src_target);
        let unit_caller = make_unit(app_a, "FriendCaller.al", src_caller);
        let units = [unit_target, unit_caller];
        let graph = build_graph_multi_dep_friends(
            &units,
            &[("PrimaryAppFriend", "DepAppFriend")],
            &[("DepAppFriend", "PrimaryAppFriend")],
        );
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let from_obj = find_obj(&graph, "FriendCaller");
        let receiver = ReceiverType::Object {
            kind: ObjectKind::Codeunit,
            name_lc: "friendtarget".into(),
            id: None,
        };
        let (shape, routes) =
            resolve_member(&receiver, "secret", 0, from_obj, &graph, &index, &body_map);

        assert_eq!(shape, DispatchShape::Exact);
        assert_eq!(routes.len(), 1);
        assert!(
            matches!(routes[0].target, RouteTarget::Routine(_)),
            "a cross-app `internal` method whose declaring app lists the \
             caller as an InternalsVisibleTo friend must resolve to Source \
             (Task 1.5); got {:?}",
            routes[0].target
        );
        assert_eq!(routes[0].evidence, Evidence::Source);
    }

    // (1.5-b) CONTROL: cross-app `internal`, declaring app does NOT list the
    // caller as a friend (a true stranger) — must stay honest Unknown.
    #[test]
    fn resolve_member_object_cross_app_internal_stranger_control_stays_unknown() {
        use crate::program::resolve::receiver::ReceiverType;

        let src_target: &'static str = r#"
codeunit 53972 "StrangerTarget"
{
    internal procedure Secret()
    begin
    end;
}
"#;
        let src_caller: &'static str = r#"
codeunit 53973 "StrangerCaller"
{
    procedure Trigger()
    begin
    end;
}
"#;
        let app_a = make_app_id("PrimaryAppStranger");
        let app_b = make_app_id("DepAppStranger");
        let unit_target = make_unit(app_b, "StrangerTarget.al", src_target);
        let unit_caller = make_unit(app_a, "StrangerCaller.al", src_caller);
        let units = [unit_target, unit_caller];
        // No friends entry at all — DepAppStranger declares NO friends.
        let graph =
            build_graph_multi_dep_friends(&units, &[("PrimaryAppStranger", "DepAppStranger")], &[]);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let from_obj = find_obj(&graph, "StrangerCaller");
        let receiver = ReceiverType::Object {
            kind: ObjectKind::Codeunit,
            name_lc: "strangertarget".into(),
            id: None,
        };
        let (shape, routes) =
            resolve_member(&receiver, "secret", 0, from_obj, &graph, &index, &body_map);

        assert_eq!(shape, DispatchShape::Exact);
        assert_eq!(routes.len(), 1);
        assert_eq!(
            routes[0].target,
            RouteTarget::Unresolved,
            "a cross-app `internal` method whose declaring app does NOT \
             list the caller as a friend (a true stranger) must stay \
             honest Unknown (Task 1.5 control — declining a stranger is \
             sound, not an over-decline); got {:?}",
            routes[0].target
        );
        assert!(matches!(
            routes[0].evidence,
            Evidence::Unknown(UnknownReason::InternalNotVisible)
        ));
    }

    // (1.5-c) DIRECTIONALITY: friendship is declared BY the exposing app and
    // is never inherited by the app it names. App A declares App B a friend
    // (`friends[A] = {B}`), so B → A internal resolves. That grant is
    // one-directional: it must NOT be readable backwards as `friends[B] =
    // {A}`. The original version of this fixture only asserted the GRANTED
    // direction (B → A) plus a same-app B → B sanity check, never the actual
    // REVERSE call (A → B, where B declares no friends of its own) — so a
    // bidirectionality bug in `internal_visible_across` could have slipped
    // through untested (Task 1.5 review, minor finding (a), folded in here).
    // This now exercises all three: B → A resolves `Source` (granted); B → B
    // same-app sanity check; A → B, calling an `internal` member of the app
    // that RECEIVED trust rather than granted it, stays honest `Unknown`
    // (no reciprocal grant exists).
    #[test]
    fn resolve_member_object_cross_app_internal_friendship_not_bidirectional() {
        use crate::program::resolve::receiver::ReceiverType;

        let src_a_target: &'static str = r#"
codeunit 53974 "DirATarget"
{
    internal procedure SecretA()
    begin
    end;
}
"#;
        let src_b_target: &'static str = r#"
codeunit 53975 "DirBTarget"
{
    internal procedure SecretB()
    begin
    end;
}
"#;
        let src_b_caller: &'static str = r#"
codeunit 53976 "DirBCaller"
{
    procedure Trigger()
    begin
    end;
}
"#;
        let src_a_caller: &'static str = r#"
codeunit 53979 "DirACaller"
{
    procedure Trigger()
    begin
    end;
}
"#;
        let app_a = make_app_id("PrimaryAppDirA");
        let app_b = make_app_id("DepAppDirB");
        let unit_a_target = make_unit(app_a.clone(), "DirATarget.al", src_a_target);
        let unit_b_target = make_unit(app_b.clone(), "DirBTarget.al", src_b_target);
        let unit_b_caller = make_unit(app_b.clone(), "DirBCaller.al", src_b_caller);
        let unit_a_caller = make_unit(app_a, "DirACaller.al", src_a_caller);
        let units = [unit_a_target, unit_b_target, unit_b_caller, unit_a_caller];
        // Dependencies are declared BOTH ways (B depends on A, AND A depends
        // on B) purely so each app's caller is topologically able to reach
        // the other app's object — this fixture tests the resolver's access
        // predicate in isolation, not real-world AL's (acyclic)
        // dependency-graph constraints. Friends are declared ONE way only:
        // App A lists App B as a friend (`friends[A] = {B}`); App B declares
        // NO friends of its own (`friends[B]` absent). So B → A internal is
        // friend-authorized; A → B internal is NOT (B never granted A
        // anything) — there is no "friends[A].contains(B) implies
        // friends[B].contains(A)" shortcut in the implementation, and this
        // fixture proves it by actually calling in that direction, not just
        // asserting same-app control cases.
        let graph = build_graph_multi_dep_friends(
            &units,
            &[
                ("DepAppDirB", "PrimaryAppDirA"),
                ("PrimaryAppDirA", "DepAppDirB"),
            ],
            &[("PrimaryAppDirA", "DepAppDirB")],
        );
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let from_obj = find_obj(&graph, "DirBCaller");

        // B → A: A declared B a friend → resolves Source.
        let receiver_a = ReceiverType::Object {
            kind: ObjectKind::Codeunit,
            name_lc: "diratarget".into(),
            id: None,
        };
        let (shape_a, routes_a) = resolve_member(
            &receiver_a,
            "secreta",
            0,
            from_obj,
            &graph,
            &index,
            &body_map,
        );
        assert_eq!(shape_a, DispatchShape::Exact);
        assert_eq!(routes_a.len(), 1);
        assert!(
            matches!(routes_a[0].target, RouteTarget::Routine(_)),
            "B → A: A's manifest lists B as a friend, must resolve to \
             Source; got {:?}",
            routes_a[0].target
        );

        // B → B's own DirBTarget: same-app, unaffected — sanity check the
        // fixture's own app is still internally consistent.
        let receiver_b = ReceiverType::Object {
            kind: ObjectKind::Codeunit,
            name_lc: "dirbtarget".into(),
            id: None,
        };
        let (shape_b, routes_b) = resolve_member(
            &receiver_b,
            "secretb",
            0,
            from_obj,
            &graph,
            &index,
            &body_map,
        );
        assert_eq!(shape_b, DispatchShape::Exact);
        assert_eq!(routes_b.len(), 1);
        assert!(
            matches!(routes_b[0].target, RouteTarget::Routine(_)),
            "B → B: same-app internal must still resolve to Source \
             (directionality fixture sanity check); got {:?}",
            routes_b[0].target
        );

        // A → B: A declared B a friend, but that grant runs ONE way — B
        // declared no friends of its own, so A calling B's `internal`
        // member must stay honest Unknown. This is the actual reverse-
        // direction proof: if `internal_visible_across` ever regressed to
        // a symmetric/bidirectional check, this assertion (not merely the
        // same-app B → B control above) would catch it.
        let from_obj_a = find_obj(&graph, "DirACaller");
        let receiver_b_target = ReceiverType::Object {
            kind: ObjectKind::Codeunit,
            name_lc: "dirbtarget".into(),
            id: None,
        };
        let (shape_rev, routes_rev) = resolve_member(
            &receiver_b_target,
            "secretb",
            0,
            from_obj_a,
            &graph,
            &index,
            &body_map,
        );
        assert_eq!(shape_rev, DispatchShape::Exact);
        assert_eq!(routes_rev.len(), 1);
        assert_eq!(
            routes_rev[0].target,
            RouteTarget::Unresolved,
            "A → B: B never friended A back (friendship is declared BY \
             the exposing app, never inherited), so this must stay \
             honest Unknown, not Source; got {:?}",
            routes_rev[0].target
        );
        assert!(
            matches!(
                routes_rev[0].evidence,
                Evidence::Unknown(UnknownReason::InternalNotVisible)
            ),
            "A → B must be excluded with InternalNotVisible, not some \
             other reason; got {:?}",
            routes_rev[0].evidence
        );
    }

    // (1.5-d) same-app `internal` is unaffected by friend modeling (no
    // friends declared at all) — unchanged Task 1 positive control,
    // re-pinned here to keep the Task 1.5 matrix self-contained.
    #[test]
    fn resolve_member_object_same_app_internal_unaffected_by_friend_modeling() {
        use crate::program::resolve::receiver::ReceiverType;

        let src_target: &'static str = r#"
codeunit 53977 "SameAppTarget"
{
    internal procedure DoWork()
    begin
    end;
}
"#;
        let src_caller: &'static str = r#"
codeunit 53978 "SameAppCaller"
{
    procedure Trigger()
    begin
    end;
}
"#;
        let app_id = make_app_id("SoloFriendApp");
        let unit_target = make_unit(app_id.clone(), "SameAppTarget.al", src_target);
        let unit_caller = make_unit(app_id, "SameAppCaller.al", src_caller);
        let units = [unit_target, unit_caller];
        let graph = build_graph_multi_dep_friends(&units, &[], &[]);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let from_obj = find_obj(&graph, "SameAppCaller");
        let receiver = ReceiverType::Object {
            kind: ObjectKind::Codeunit,
            name_lc: "sameapptarget".into(),
            id: None,
        };
        let (shape, routes) =
            resolve_member(&receiver, "dowork", 0, from_obj, &graph, &index, &body_map);

        assert_eq!(shape, DispatchShape::Exact);
        assert_eq!(routes.len(), 1);
        assert!(
            matches!(routes[0].target, RouteTarget::Routine(_)),
            "same-app internal must still resolve to Source with zero \
             friends declared (Task 1.5 unaffected-scope control); got {:?}",
            routes[0].target
        );
        assert_eq!(routes[0].evidence, Evidence::Source);
    }

    // (D-neg-2) Object receiver, same-app but DIFFERENT (non-extension)
    // object's `local` — pre-fix this false-resolved to
    // `RouteTarget::Routine(LocNTarget.Hidden)`.
    #[test]
    fn resolve_member_object_same_app_local_cross_object_excluded() {
        use crate::program::resolve::receiver::ReceiverType;

        let src_target: &'static str = r#"
codeunit 53960 "LocNTarget"
{
    local procedure Hidden()
    begin
    end;
}
"#;
        let src_caller: &'static str = r#"
codeunit 53961 "LocNCaller"
{
    procedure Trigger()
    begin
    end;
}
"#;
        let app_id = make_app_id("TestApp");
        let unit_target = make_unit(app_id.clone(), "LocNTarget.al", src_target);
        let unit_caller = make_unit(app_id, "LocNCaller.al", src_caller);
        let units = [unit_target, unit_caller];
        let graph = build_graph(&units, None);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let from_obj = find_obj(&graph, "LocNCaller");
        let receiver = ReceiverType::Object {
            kind: ObjectKind::Codeunit,
            name_lc: "locntarget".into(),
            id: None,
        };
        let (shape, routes) =
            resolve_member(&receiver, "hidden", 0, from_obj, &graph, &index, &body_map);

        assert_eq!(shape, DispatchShape::Exact);
        assert_eq!(routes.len(), 1);
        assert_eq!(
            routes[0].target,
            RouteTarget::Unresolved,
            "a same-app but DIFFERENT object's `local` method reached via an \
             Object receiver must NOT resolve to Source (gap D — AL `local` \
             is OBJECT-scoped, not app-scoped; pre-fix this false-resolved \
             to RouteTarget::Routine(LocNTarget.Hidden)); got {:?}",
            routes[0].target
        );
        assert!(matches!(routes[0].evidence, Evidence::Unknown(_)));
    }

    // (D-neg-3) THE OVERLOAD-NARROWING GUARD: two textually distinct
    // same-arity overloads of `Foo` (Integer vs Text) differing only in
    // access modifier + param TYPE — `RoutineNodeId` collides for both
    // (source `sig_fp` is always `0`; see node.rs /
    // `build::dedup_routines_preserving_genuine_overloads`), so the
    // pre-filter set is genuinely ambiguous (2 same-arity candidates).
    // Calling cross-app with 1 (unproven-type) argument must NEVER resolve
    // to Source, even though exactly one physical overload (`Foo(Integer)`,
    // `public`) happens to be visible and the other (`Foo(Text)`,
    // `internal`) is cross-app-excluded — access alone cannot prove which
    // overload the call meant.
    #[test]
    fn resolve_member_object_mixed_access_same_arity_overload_never_resolves_to_source() {
        use crate::program::resolve::receiver::ReceiverType;

        let src_target: &'static str = r#"
codeunit 53970 "OverloadNTarget"
{
    procedure Foo(p: Integer)
    begin
    end;

    internal procedure Foo(p: Text)
    begin
    end;
}
"#;
        let src_caller: &'static str = r#"
codeunit 53971 "OverloadNCaller"
{
    procedure Trigger()
    begin
    end;
}
"#;
        let app_a = make_app_id("PrimaryApp3");
        let app_b = make_app_id("DepApp3");
        let unit_target = make_unit(app_b, "OverloadNTarget.al", src_target);
        let unit_caller = make_unit(app_a, "OverloadNCaller.al", src_caller);
        let units = [unit_target, unit_caller];
        let graph = build_graph_multi_dep(&units, &[("PrimaryApp3", "DepApp3")]);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        // Sanity: both overloads really did survive as one params_count==1
        // collision (proves the fixture actually exercises the guard, not a
        // degenerate single-candidate case).
        let target_obj = find_obj(&graph, "OverloadNTarget");
        let foo_candidates = index.routines_in_object(&target_obj.id, "foo");
        assert_eq!(
            foo_candidates.len(),
            2,
            "fixture must produce TWO same-arity `Foo` candidates; got {:?}",
            foo_candidates
        );

        let from_obj = find_obj(&graph, "OverloadNCaller");
        let receiver = ReceiverType::Object {
            kind: ObjectKind::Codeunit,
            name_lc: "overloadntarget".into(),
            id: None,
        };
        let (shape, routes) =
            resolve_member(&receiver, "foo", 1, from_obj, &graph, &index, &body_map);

        assert_eq!(shape, DispatchShape::Exact);
        assert_eq!(routes.len(), 1);
        assert!(
            !matches!(routes[0].target, RouteTarget::Routine(_)),
            "mixed-access same-arity overload (public Foo(Integer) + \
             internal Foo(Text)) called cross-app with an unproven-type arg \
             must NEVER resolve to Source — access-narrowing to the lone \
             visible overload would manufacture a false resolution (the \
             overload-narrowing guard); got {:?}",
            routes[0].target
        );
        assert_eq!(routes[0].target, RouteTarget::Unresolved);
        assert!(matches!(routes[0].evidence, Evidence::Unknown(_)));
    }

    // (D-neg-4) Object receiver, same-app but UNRELATED (non-extension)
    // object's `protected` method.
    #[test]
    fn resolve_member_object_same_app_non_extension_protected_excluded() {
        use crate::program::resolve::receiver::ReceiverType;

        let src_target: &'static str = r#"
codeunit 53980 "ProtNTarget"
{
    protected procedure P()
    begin
    end;
}
"#;
        let src_caller: &'static str = r#"
codeunit 53981 "ProtNCaller"
{
    procedure Trigger()
    begin
    end;
}
"#;
        let app_id = make_app_id("TestApp");
        let unit_target = make_unit(app_id.clone(), "ProtNTarget.al", src_target);
        let unit_caller = make_unit(app_id, "ProtNCaller.al", src_caller);
        let units = [unit_target, unit_caller];
        let graph = build_graph(&units, None);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let from_obj = find_obj(&graph, "ProtNCaller");
        let receiver = ReceiverType::Object {
            kind: ObjectKind::Codeunit,
            name_lc: "protntarget".into(),
            id: None,
        };
        let (shape, routes) =
            resolve_member(&receiver, "p", 0, from_obj, &graph, &index, &body_map);

        assert_eq!(shape, DispatchShape::Exact);
        assert_eq!(routes.len(), 1);
        assert_eq!(
            routes[0].target,
            RouteTarget::Unresolved,
            "a same-app but UNRELATED (non-extension) object's `protected` \
             method reached via an Object receiver must NOT resolve to \
             Source (gap D); got {:?}",
            routes[0].target
        );
        assert!(matches!(routes[0].evidence, Evidence::Unknown(_)));
    }

    // (D-neg-5) Object receiver, cross-app UNRELATED (non-extension)
    // object's `protected` method — a fortiori excluded vs the same-app case.
    #[test]
    fn resolve_member_object_cross_app_non_extension_protected_excluded() {
        use crate::program::resolve::receiver::ReceiverType;

        let src_target: &'static str = r#"
codeunit 53990 "ProtXNTarget"
{
    protected procedure P()
    begin
    end;
}
"#;
        let src_caller: &'static str = r#"
codeunit 53991 "ProtXNCaller"
{
    procedure Trigger()
    begin
    end;
}
"#;
        let app_a = make_app_id("PrimaryApp4");
        let app_b = make_app_id("DepApp4");
        let unit_target = make_unit(app_b, "ProtXNTarget.al", src_target);
        let unit_caller = make_unit(app_a, "ProtXNCaller.al", src_caller);
        let units = [unit_target, unit_caller];
        let graph = build_graph_multi_dep(&units, &[("PrimaryApp4", "DepApp4")]);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let from_obj = find_obj(&graph, "ProtXNCaller");
        let receiver = ReceiverType::Object {
            kind: ObjectKind::Codeunit,
            name_lc: "protxntarget".into(),
            id: None,
        };
        let (shape, routes) =
            resolve_member(&receiver, "p", 0, from_obj, &graph, &index, &body_map);

        assert_eq!(shape, DispatchShape::Exact);
        assert_eq!(routes.len(), 1);
        assert_eq!(
            routes[0].target,
            RouteTarget::Unresolved,
            "a cross-app UNRELATED (non-extension) object's `protected` \
             method reached via an Object receiver must NOT resolve to \
             Source (gap D, a fortiori excluded vs the same-app case); \
             got {:?}",
            routes[0].target
        );
        assert!(matches!(routes[0].evidence, Evidence::Unknown(_)));
    }

    // (D-neg-6) Object receiver, WRONG-KIND extension: a Table and a Page
    // share the literal name "Shared"; a PageExtension `extends Shared`
    // resolves (kind-compatibly) against the PAGE, never the Table. The
    // Table's `protected` method must stay invisible — the PageExtension
    // does NOT directly extend the Table (wrong kind), despite sharing the
    // extends-target NAME with an object it DOES directly extend.
    #[test]
    fn resolve_member_object_wrong_kind_extension_protected_excluded() {
        use crate::program::resolve::receiver::ReceiverType;

        let src_table: &'static str = r#"
table 54000 "Shared"
{
    protected procedure P()
    begin
    end;
}
"#;
        let src_page: &'static str = r#"
page 54001 "Shared"
{
}
"#;
        let src_ext: &'static str = r#"
pageextension 54002 "SharedExt" extends Shared
{
    procedure Trigger()
    begin
    end;
}
"#;
        let app_id = make_app_id("TestApp");
        let unit_table = make_unit(app_id.clone(), "SharedTable.al", src_table);
        let unit_page = make_unit(app_id.clone(), "SharedPage.al", src_page);
        let unit_ext = make_unit(app_id, "SharedExt.al", src_ext);
        let units = [unit_table, unit_page, unit_ext];
        let graph = build_graph(&units, None);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let from_obj = find_obj(&graph, "SharedExt");
        assert_eq!(from_obj.id.kind, ObjectKind::PageExtension);
        let table_obj = graph
            .objects
            .iter()
            .find(|o| o.name.eq_ignore_ascii_case("shared") && o.id.kind == ObjectKind::Table)
            .expect("table Shared");
        let page_obj = graph
            .objects
            .iter()
            .find(|o| o.name.eq_ignore_ascii_case("shared") && o.id.kind == ObjectKind::Page)
            .expect("page Shared");

        // Sanity: kind-compatible resolution — the extension relationship
        // holds against the Page, never the Table.
        assert!(index.object_extends(&graph, &from_obj.id, &page_obj.id));
        assert!(!index.object_extends(&graph, &from_obj.id, &table_obj.id));

        let receiver = ReceiverType::Object {
            kind: ObjectKind::Table,
            name_lc: "shared".into(),
            id: None,
        };
        let (shape, routes) =
            resolve_member(&receiver, "p", 0, from_obj, &graph, &index, &body_map);

        assert_eq!(shape, DispatchShape::Exact);
        assert_eq!(routes.len(), 1);
        assert_eq!(
            routes[0].target,
            RouteTarget::Unresolved,
            "a PageExtension must NOT see a same-named-but-WRONG-KIND \
             Table's `protected` method via an Object receiver (gap D); \
             got {:?}",
            routes[0].target
        );
        assert!(matches!(routes[0].evidence, Evidence::Unknown(_)));
    }

    // (G-neg / G-pos combined) Interface fan-out: a `public` implementer and
    // a CROSS-APP `internal` implementer of the SAME interface member. AL
    // does not require an interface-implementing procedure to be `public`
    // (a compiler-valid construct) — the `internal` implementer dispatches
    // fine for a SAME-app caller (see the sibling positive test below) but
    // must be excluded for a caller in a DIFFERENT app, while the `public`
    // implementer's route is unaffected either way.
    #[test]
    fn resolve_member_interface_cross_app_internal_impl_excluded_public_impl_still_resolves() {
        use crate::program::resolve::receiver::ReceiverType;

        let src_pub_impl: &'static str = r#"
codeunit 54010 "PubImplX" implements IFoo
{
    procedure Bar()
    begin
    end;
}
"#;
        let src_int_impl: &'static str = r#"
codeunit 54011 "IntImplX" implements IFoo
{
    internal procedure Bar()
    begin
    end;
}
"#;
        let src_caller: &'static str = r#"
codeunit 54012 "IfaceCallerX"
{
    procedure Trigger()
    begin
    end;
}
"#;
        let app_impl = make_app_id("ImplAppX");
        let app_caller = make_app_id("CallerAppX");
        let unit_pub_impl = make_unit(app_impl.clone(), "PubImplX.al", src_pub_impl);
        let unit_int_impl = make_unit(app_impl, "IntImplX.al", src_int_impl);
        let unit_caller = make_unit(app_caller, "IfaceCallerX.al", src_caller);
        let units = [unit_pub_impl, unit_int_impl, unit_caller];
        let graph = build_graph_multi_dep(&units, &[("CallerAppX", "ImplAppX")]);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let from_obj = find_obj(&graph, "IfaceCallerX");
        let receiver = ReceiverType::Interface {
            name_lc: "ifoo".into(),
        };
        let (shape, routes) =
            resolve_member(&receiver, "bar", 0, from_obj, &graph, &index, &body_map);

        assert_eq!(shape, DispatchShape::Polymorphic);
        assert_eq!(
            routes.len(),
            2,
            "two implementers -> two routes; got {:?}",
            routes
        );

        let pub_impl_id = find_obj(&graph, "PubImplX").id.clone();
        let int_impl_id = find_obj(&graph, "IntImplX").id.clone();

        let pub_route = routes
            .iter()
            .find(|r| matches!(&r.target, RouteTarget::Routine(rid) if rid.object == pub_impl_id))
            .expect("public implementer must have a Routine route");
        assert_eq!(pub_route.evidence, Evidence::Source);

        let int_route_resolved_to_source = routes
            .iter()
            .any(|r| matches!(&r.target, RouteTarget::Routine(rid) if rid.object == int_impl_id));
        assert!(
            !int_route_resolved_to_source,
            "the cross-app `internal` implementer must NOT emit a \
             Routine/Source route (gap G — pre-fix this false-resolved); \
             got {:?}",
            routes
        );

        // The other route (not the public implementer's) must be an honest
        // Unresolved/Unknown — the internal implementer's excluded route.
        let other_route = routes
            .iter()
            .find(|r| !matches!(&r.target, RouteTarget::Routine(rid) if rid.object == pub_impl_id))
            .expect("the internal implementer's route must be present, not dropped");
        assert_eq!(other_route.target, RouteTarget::Unresolved);
        assert!(matches!(other_route.evidence, Evidence::Unknown(_)));
    }

    // (G-pos) SAME-app `internal` interface implementer — a positive control
    // proving `internal` is app-scoped, not interface-scoped: the sibling
    // test above proves the CROSS-app case is excluded.
    #[test]
    fn resolve_member_interface_same_app_internal_impl_resolves_to_source() {
        use crate::program::resolve::receiver::ReceiverType;

        let src_int_impl: &'static str = r#"
codeunit 54020 "IntImplY" implements IFoo
{
    internal procedure Bar()
    begin
    end;
}
"#;
        let src_caller: &'static str = r#"
codeunit 54021 "IfaceCallerY"
{
    procedure Trigger()
    begin
    end;
}
"#;
        let app_id = make_app_id("TestApp");
        let unit_impl = make_unit(app_id.clone(), "IntImplY.al", src_int_impl);
        let unit_caller = make_unit(app_id, "IfaceCallerY.al", src_caller);
        let units = [unit_impl, unit_caller];
        let graph = build_graph(&units, None);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let from_obj = find_obj(&graph, "IfaceCallerY");
        let receiver = ReceiverType::Interface {
            name_lc: "ifoo".into(),
        };
        let (shape, routes) =
            resolve_member(&receiver, "bar", 0, from_obj, &graph, &index, &body_map);

        assert_eq!(shape, DispatchShape::Polymorphic);
        assert_eq!(routes.len(), 1);
        assert!(
            matches!(routes[0].target, RouteTarget::Routine(_)),
            "a SAME-app `internal` interface implementer must resolve to \
             Source (gap G positive control — Internal is app-scoped, not \
             interface-scoped); got {:?}",
            routes[0].target
        );
        assert_eq!(routes[0].evidence, Evidence::Source);
    }

    // (D-neg-7) A user-defined member LITERALLY named `Run`, arity 2 —
    // the `Codeunit.Run(arity<=1)` OnRun-trigger special case only engages
    // for arity<=1, so this falls through to the GENERAL resolve_in_object
    // dispatch. Proves "Run" is not a blanket access-filter exemption — only
    // the actual entry-trigger dispatch is.
    #[test]
    fn resolve_member_object_user_defined_run_cross_app_internal_excluded_not_run_exempt() {
        use crate::program::resolve::receiver::ReceiverType;

        let src_target: &'static str = r#"
codeunit 54030 "RunNTarget"
{
    internal procedure Run(a: Integer; b: Integer)
    begin
    end;
}
"#;
        let src_caller: &'static str = r#"
codeunit 54031 "RunNCaller"
{
    procedure Trigger()
    begin
    end;
}
"#;
        let app_a = make_app_id("PrimaryApp5");
        let app_b = make_app_id("DepApp5");
        let unit_target = make_unit(app_b, "RunNTarget.al", src_target);
        let unit_caller = make_unit(app_a, "RunNCaller.al", src_caller);
        let units = [unit_target, unit_caller];
        let graph = build_graph_multi_dep(&units, &[("PrimaryApp5", "DepApp5")]);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let from_obj = find_obj(&graph, "RunNCaller");
        let receiver = ReceiverType::Object {
            kind: ObjectKind::Codeunit,
            name_lc: "runntarget".into(),
            id: None,
        };
        let (shape, routes) =
            resolve_member(&receiver, "run", 2, from_obj, &graph, &index, &body_map);

        assert_eq!(shape, DispatchShape::Exact);
        assert_eq!(routes.len(), 1);
        assert_eq!(
            routes[0].target,
            RouteTarget::Unresolved,
            "a user-defined 2-arg `Run` procedure declared `internal` in a \
             cross-app codeunit must NOT resolve to Source — the \
             OnRun-trigger special case is scoped to arity<=1 and must not \
             blanket-exempt every member literally named \"Run\"; got {:?}",
            routes[0].target
        );
        assert!(matches!(routes[0].evidence, Evidence::Unknown(_)));
    }

    // --- Run/ObjectRun exemption control: bypasses resolve_in_object -------

    // (Run-control) `Codeunit.Run()` on a codeunit with NO `OnRun` trigger —
    // must emit an Opaque AbiSymbol boundary route, never a synthesized
    // Source. This path (`resolve_member`'s inline Codeunit.Run special
    // case) bypasses `resolve_in_object` entirely and is untouched by Task 1.
    #[test]
    fn resolve_member_codeunit_run_no_onrun_trigger_emits_opaque_not_source() {
        use crate::program::resolve::receiver::ReceiverType;

        let src_target: &'static str = r#"
codeunit 54040 "NoOnRunTarget"
{
    procedure SomethingElse()
    begin
    end;
}
"#;
        let src_caller: &'static str = r#"
codeunit 54041 "NoOnRunCaller"
{
    procedure Trigger()
    begin
    end;
}
"#;
        let app_id = make_app_id("TestApp");
        let unit_target = make_unit(app_id.clone(), "NoOnRunTarget.al", src_target);
        let unit_caller = make_unit(app_id, "NoOnRunCaller.al", src_caller);
        let units = [unit_target, unit_caller];
        let graph = build_graph(&units, None);
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &units);

        let from_obj = find_obj(&graph, "NoOnRunCaller");
        let receiver = ReceiverType::Object {
            kind: ObjectKind::Codeunit,
            name_lc: "noonruntarget".into(),
            id: None,
        };
        let (shape, routes) =
            resolve_member(&receiver, "run", 0, from_obj, &graph, &index, &body_map);

        assert_eq!(shape, DispatchShape::Exact);
        assert_eq!(routes.len(), 1);
        assert!(
            !matches!(routes[0].target, RouteTarget::Routine(_)),
            "Codeunit.Run() on a codeunit with NO OnRun trigger must never \
             synthesize a Source route; got {:?}",
            routes[0].target
        );
        assert!(
            matches!(routes[0].target, RouteTarget::AbiSymbol { .. }),
            "must be an Opaque AbiSymbol boundary route (object exists, \
             OnRun trigger absent — unaffected by the access filter since \
             Codeunit.Run bypasses resolve_in_object entirely); got {:?}",
            routes[0].target
        );
        assert_eq!(routes[0].evidence, Evidence::Opaque);
    }
}
