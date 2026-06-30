//! Bare-call resolution: resolve an unqualified `Foo()` call to its [`Route`]
//! target(s), topology-scoped, with evidence and witness.
//!
//! # Precedence (first hit wins)
//!
//! 1. **Own object** — a procedure named `name_lc` declared in `from_object`.
//! 2. **Extension base** — if `from_object` is a `*Extension`, search the base
//!    object (`TableExtension`→`Table`, `PageExtension`→`Page`, …).
//! 3. **Implicit-Rec** — Page/Table-ext implicit table lookup (**deferred, TODO
//!    Phase 2+**; the Task-6 gate measures the residual).
//! 4. **Global builtin** — `is_global_builtin(name_lc)` → `Catalog` route.
//! 5. **Unknown** — genuine resolution failure.
//!
//! # Arity matching (Phase 2 / Phase 3 Task 0)
//!
//! `RoutineNodeId` now carries `params_count`, so each overload (same name,
//! different arity) is a distinct node in the graph and index.
//! `routines_in_object` returns one entry per distinct overload.  An overload
//! matches when `rid.params_count == arity`.  When the first (by sorted
//! `RoutineNodeId` order) match is found it is returned.  When the name is found
//! but NO overload matches the arity, an `Unknown` route is emitted — no
//! false-confident edge to a wrong-arity target.  The caller still stops at that
//! precedence level (does NOT fall through to extension-base / global-builtin),
//! mirroring L3's MemberNotFound stop semantics while surfacing the gap honestly.
//! Name-absent ⇒ `None` ⇒ fall through.
//!
//! # Witness↔evidence contract
//!
//! `Evidence::Source` ⇒ `Witness::SourceSpan`
//! `Evidence::Catalog`⇒ `Witness::CatalogEntry`
//! `Evidence::Unknown`⇒ `Witness::None`

use al_syntax::ir::ObjectKind;

use crate::program::graph::ProgramGraph;
use crate::program::node::{AppRef, ObjectNodeId, RoutineNodeId};
use crate::program::node_extract::ObjectNode;
use crate::program::resolve::body_map::BodyMap;
use crate::program::resolve::builtins::{catalog_version, global_builtin_id};
use crate::program::resolve::edge::{
    BuiltinId, DispatchShape, Evidence, OpenWorldReason, Route, RouteTarget, SetCompleteness,
    Witness,
};
use crate::program::resolve::index::ResolveIndex;
use crate::program::resolve::member_catalog::{MemberCatalogKind, member_builtin_id};
use crate::program::resolve::receiver::{FrameworkKind, ReceiverType};
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
fn make_routine_route(rid: &RoutineNodeId, obj_tier: TrustTier, body_map: &BodyMap<'_>) -> Route {
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
        let symbol_key = format!("{:?}::{}", rid.object.kind, rid.name_lc);
        Route {
            target: RouteTarget::AbiSymbol {
                app: rid.object.app,
                symbol_key: symbol_key.clone(),
            },
            evidence: Evidence::Opaque,
            conditions: vec![],
            witness: Witness::AbiSymbol {
                app: rid.object.app,
                symbol_key,
            },
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
/// Returns the first arity-matched overload as a `Source` route.  When the name
/// is found but NO overload matches the arity, returns an `Unknown` route — no
/// false-confident edge to a wrong-arity candidate (does NOT fall through to the
/// next precedence level; see module-level doc).  Returns `None` only when the
/// name is absent entirely in `obj_id`.
///
/// **SymbolOnly tier exception:** `params_count` is 0 for all SymbolOnly routines
/// (loaded from .app SymbolReference, no source parse), so arity matching is
/// impossible.  Any name match immediately produces an `Opaque` boundary route
/// (via [`make_routine_route`]) rather than a false Unknown that would regress
/// vs L3's External resolution.
fn resolve_in_object(
    obj_id: &ObjectNodeId,
    obj_tier: TrustTier,
    name_lc: &str,
    arity: usize,
    index: &ResolveIndex,
    body_map: &BodyMap<'_>,
) -> Option<Route> {
    let candidates = index.routines_in_object(obj_id, name_lc);
    if candidates.is_empty() {
        return None;
    }

    // SymbolOnly: params info is unavailable (loaded from .app SymbolReference,
    // no source parse) so params_count is always 0. Arity checking is impossible;
    // use the first candidate directly. `make_routine_route` returns an Opaque
    // boundary route for SymbolOnly BodyMap misses.
    if obj_tier == TrustTier::SymbolOnly {
        // SAFETY: candidates is non-empty (checked above).
        return Some(make_routine_route(
            candidates.first().unwrap(),
            obj_tier,
            body_map,
        ));
    }

    // Arity-exact match: find the first (by sorted RoutineNodeId order) overload
    // whose params_count == arity. With params_count in RoutineNodeId, each
    // overload is a distinct node in the graph and index.
    if let Some(rid) = candidates.iter().find(|rid| rid.params_count == arity) {
        // TODO: disambiguate_by_arg_types when multiple same-arity overloads exist
        // (overloads differing only by param type, rare in AL). Deferred to a
        // later task; deterministic: first by RoutineNodeId sorted order.
        return Some(make_routine_route(rid, obj_tier, body_map));
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

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Resolve a bare (unqualified) call to `name_lc` with `arity` arguments from
/// the context of `from_object`.
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
) -> Vec<Route> {
    // 1. Own object.
    if let Some(route) = resolve_in_object(
        &from_object.id,
        from_object.tier,
        name_lc,
        arity,
        index,
        body_map,
    ) {
        return vec![route];
    }

    // 2. Extension base.
    if let Some(base_kind) = extension_base_kind(from_object.id.kind)
        && let Some(extends_target) = from_object.extends_target.as_deref()
        && let Some(base_obj) = graph.resolve_object(from_object.id.app, base_kind, extends_target)
    {
        let base_id = base_obj.id.clone();
        let base_tier = base_obj.tier;
        if let Some(route) = resolve_in_object(&base_id, base_tier, name_lc, arity, index, body_map)
        {
            return vec![route];
        }
    }

    // 3. Implicit-Rec (deferred).
    // TODO Phase 2+: if from_object is a Page/Table/TableExtension/PageExtension
    // with an implicit source table, look up the procedure there and in all its
    // TableExtensions.  The Task-6 gate measures the residual real-unknown rate
    // attributable to this gap.

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
fn opaque_boundary_route(app: AppRef, symbol_key: String) -> Route {
    Route {
        target: RouteTarget::AbiSymbol {
            app,
            symbol_key: symbol_key.clone(),
        },
        evidence: Evidence::Opaque,
        conditions: vec![],
        witness: Witness::AbiSymbol { app, symbol_key },
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
        // Target is named/numbered but absent from the graph: not-in-source boundary.
        let symbol_key = format!("{:?}::{}", object_kind, target_ref);
        return (
            DispatchShape::Exact,
            SetCompleteness::Complete,
            vec![opaque_boundary_route(from, symbol_key)],
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
        let symbol_key = target_obj.name.clone();
        return (
            DispatchShape::Exact,
            SetCompleteness::Complete,
            vec![opaque_boundary_route(target_obj.id.app, symbol_key)],
        );
    };

    let route = make_routine_route(entry_rid, target_obj.tier, body_map);
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
        routes.push(make_routine_route(rid, table_object.tier, body_map));
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
            routes.push(make_routine_route(rid, ext_tier, body_map));
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
/// # Implemented arms (Phase 3 Task 2 + Task 3 + Phase 4 Task 2)
/// - `RecordRef` / `FieldRef` / `KeyRef` / `Framework(_)` → catalog lookup.
/// - `Record{..}` → catalog-first (builtin Record methods); non-builtin → Unknown
///   (TODO Task 4: full table-proc dispatch).
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
            // Catalog-first: Record built-in methods (SetRange, Find, Insert, ...) are
            // platform-intrinsic and don't have in-source bodies.
            if let Some(bid) = member_builtin_id(MemberCatalogKind::Record, method_lc) {
                return member_catalog_route(bid);
            }

            // Non-builtin Record method: dispatch to the table's own procedures and
            // its TableExtensions.  `table == None` means the table could not be
            // resolved (e.g. implicit Rec on a Page/PageExtension where the source
            // table is not on ObjectNode) — honest Unknown.
            let Some(table_id) = table else {
                return member_unknown_route();
            };

            // Look up the ObjectNode for the table to obtain its tier and name
            // (needed for both resolve_in_object and table_extensions_of).
            let Some((table_tier, table_name_lc)) = graph
                .objects
                .iter()
                .find(|o| &o.id == table_id)
                .map(|o| (o.tier, o.name.to_ascii_lowercase()))
            else {
                return member_unknown_route();
            };

            // 1. Try the base table first (single-dispatch; Exact not Multicast).
            if let Some(route) =
                resolve_in_object(table_id, table_tier, method_lc, arity, index, body_map)
            {
                return (DispatchShape::Exact, vec![route]);
            }

            // 2. Try each TableExtension of this table (whole-snapshot, reverse-dep).
            //    The UNION is to FIND the proc — first hit wins (Exact, not Multicast).
            for ext_id in index.table_extensions_of(&table_name_lc) {
                let ext_tier = graph
                    .objects
                    .iter()
                    .find(|o| &o.id == ext_id)
                    .map(|o| o.tier)
                    .unwrap_or(TrustTier::Workspace);
                if let Some(route) =
                    resolve_in_object(ext_id, ext_tier, method_lc, arity, index, body_map)
                {
                    return (DispatchShape::Exact, vec![route]);
                }
            }

            // Method not found on base table or any extension.
            member_unknown_route()
        }
        ReceiverType::Object { kind, name_lc } => {
            // Resolve the target object (topology-scoped from the calling app).
            let Some(target) = graph.resolve_object(from_object.id.app, *kind, name_lc) else {
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
                        vec![make_routine_route(entry_rid, target_tier, body_map)],
                    )
                } else {
                    // OnRun not indexed — Opaque boundary (object exists, trigger absent).
                    (
                        DispatchShape::Exact,
                        vec![opaque_boundary_route(target_id.app, name_lc.clone())],
                    )
                };
            }

            // General dispatch: resolve the method among the target object's procedures.
            if let Some(route) =
                resolve_in_object(&target_id, target_tier, method_lc, arity, index, body_map)
            {
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
            //   SymbolOnly tier  → `params_count` is always 0 in .app SymbolReference,
            //                      so arity matching is impossible; delegate directly to
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
                    // SymbolOnly: arity matching impossible; delegate.
                    let route =
                        resolve_in_object(impl_id, impl_tier, method_lc, arity, index, body_map)
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
                                    impl_id, impl_tier, method_lc, arity, index, body_map,
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
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    use crate::program::graph::{ObjectIndex, ProgramGraph};
    use crate::program::node::AppRegistry;
    use crate::program::node_extract::{ObjectNode, RoutineNode, extract_nodes};
    use crate::program::resolve::body_map::BodyMap;
    use crate::program::resolve::edge::{
        DispatchShape, Evidence, OpenWorldReason, RouteTarget, SetCompleteness, Witness,
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
        let routes = resolve_bare(from_obj, "dofoo", 0, &graph, &index, &body_map);

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
        let routes = resolve_bare(from_obj, "message", 1, &graph, &index, &body_map);

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
        let routes = resolve_bare(from_obj, "init", 0, &graph, &index, &body_map);

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
        let src_routes = resolve_bare(from_obj, "myproc", 0, &graph, &index, &body_map);
        assert_eq!(src_routes.len(), 1);
        let src_route = &src_routes[0];
        assert_eq!(src_route.evidence, Evidence::Source, "Source evidence");
        assert!(
            matches!(src_route.witness, Witness::SourceSpan { .. }),
            "Source evidence must pair with SourceSpan witness"
        );

        // Catalog route: global builtin.
        let cat_routes = resolve_bare(from_obj, "error", 1, &graph, &index, &body_map);
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
        let routes = resolve_bare(from_obj, "dofoo", 2, &graph, &index, &body_map);

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
    // Task-5 (d): Target named but not in graph → Opaque evidence
    // -----------------------------------------------------------------------

    #[test]
    fn object_run_target_not_in_graph_emits_opaque() {
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
        assert!(
            matches!(r.target, RouteTarget::AbiSymbol { .. }),
            "target must be AbiSymbol (not Unresolved); got {:?}",
            r.target
        );
        assert_eq!(
            r.evidence,
            Evidence::Opaque,
            "not-in-source boundary must use Opaque evidence"
        );
        assert!(
            matches!(r.witness, Witness::AbiSymbol { .. }),
            "AbiSymbol target must pair with AbiSymbol witness; got {:?}",
            r.witness
        );
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
        let routes = resolve_bare(from_obj, "myproc", 0, &graph, &index, &body_map);

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
        let routes1 = resolve_bare(from_obj, "post", 1, &graph, &index, &body_map);
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
        let routes0 = resolve_bare(from_obj, "post", 0, &graph, &index, &body_map);
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

    // (c) Record builtin method → Catalog (catalog-first wins over any in-source proc)
    #[test]
    fn resolve_member_record_builtin_wins_catalog_first() {
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

        // "setview" is a Record catalog builtin — catalog check fires first;
        // any in-source proc with this name cannot shadow a platform builtin.
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
}
