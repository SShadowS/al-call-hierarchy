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
//! # Arity matching (Phase 2)
//!
//! `routines_in_object` returns all overloads for a name.  An overload matches
//! when `params.len() == arity`.  When multiple overloads match the first (by
//! sorted `RoutineNodeId` order) is returned.  When the name is found but NO
//! overload matches the arity, an `Unknown` route is emitted — no false-confident
//! edge to a wrong-arity target.  The caller still stops at that precedence level
//! (does NOT fall through to extension-base / global-builtin), mirroring L3's
//! MemberNotFound stop semantics while surfacing the gap honestly.
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
    DispatchShape, Evidence, OpenWorldReason, Route, RouteTarget, SetCompleteness, Witness,
};
use crate::program::resolve::index::ResolveIndex;
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
/// - If the routine is NOT in the `BodyMap` (integration gap): `Evidence::Unknown` +
///   `Witness::None`.  In Phase 2 all ProgramGraph routines are source-tier, so a
///   BodyMap miss is a real integration bug that must surface in `real_unknown_rate`.
fn make_routine_route(rid: &RoutineNodeId, obj_tier: TrustTier, body_map: &BodyMap<'_>) -> Route {
    if let Some((decl, path)) = body_map.get_with_path(rid) {
        Route {
            target: RouteTarget::Routine(rid.clone()),
            evidence: tier_evidence(obj_tier),
            condition: None,
            witness: Witness::SourceSpan {
                file: path.to_string(),
                span: (decl.origin.byte.start as u32, decl.origin.byte.end as u32),
            },
        }
    } else {
        // BodyMap miss: the routine was resolved from the ProgramGraph (always
        // source-tier in Phase 2) but is absent from the parsed snapshot — a
        // real integration gap.  Surface it as Unknown so the gap shows up in
        // `real_unknown_rate` rather than being silently hidden.
        Route {
            target: RouteTarget::Unresolved,
            evidence: Evidence::Unknown,
            condition: None,
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

    // Arity-exact match preferred.
    if let Some(rid) = candidates.iter().find(|rid| {
        body_map
            .get(rid)
            .map(|d| d.params.len() == arity)
            .unwrap_or(false)
    }) {
        return Some(make_routine_route(rid, obj_tier, body_map));
    }

    // Name found but no overload matches the requested arity: emit Unknown
    // rather than a false-confident route to the wrong-arity candidate.
    Some(Route {
        target: RouteTarget::Unresolved,
        evidence: Evidence::Unknown,
        condition: None,
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
            condition: None,
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
        condition: None,
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
fn opaque_boundary_route() -> Route {
    Route {
        target: RouteTarget::Unresolved,
        evidence: Evidence::Opaque,
        condition: None,
        witness: Witness::None,
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
/// | Target named/numbered but absent from graph | `Exact` | `Partial{RuntimeTypeUnbounded}` | `[{Unresolved, Opaque, None}]` |
/// | Target found; entry trigger resolved | `Exact` | `Complete` | `[{Routine(trigger), Source/Abi, SourceSpan/…}]` |
/// | Target found; entry trigger absent from index | `Exact` | `Partial{RuntimeTypeUnbounded}` | `[{Unresolved, Opaque, None}]` |
///
/// # Opaque-vs-Unknown choice (Phase 2 note)
///
/// When the target is not found in the graph we use `Evidence::Opaque` (not
/// `Unknown`) because we know the target *exists* somewhere — it just isn't in
/// our source snapshot.  The `classify_obligation` metric still counts this as
/// `Unknown` (since `RouteTarget::Unresolved` is the target), which is honest:
/// it surfaces as a gap to close when dependency source is added.
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
                condition: None,
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
        return (
            DispatchShape::Exact,
            SetCompleteness::Partial {
                reason: OpenWorldReason::RuntimeTypeUnbounded,
            },
            vec![opaque_boundary_route()],
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
        return (
            DispatchShape::Exact,
            SetCompleteness::Partial {
                reason: OpenWorldReason::RuntimeTypeUnbounded,
            },
            vec![opaque_boundary_route()],
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
        let (shape, _completeness, routes) = resolve_object_run(
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
        assert_eq!(routes.len(), 1);
        let r = &routes[0];
        assert_eq!(r.target, RouteTarget::Unresolved);
        assert_eq!(
            r.evidence,
            Evidence::Opaque,
            "not-in-source boundary must use Opaque evidence"
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
}
