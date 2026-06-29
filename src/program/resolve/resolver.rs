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
use crate::program::node::{ObjectNodeId, RoutineNodeId};
use crate::program::node_extract::ObjectNode;
use crate::program::resolve::body_map::BodyMap;
use crate::program::resolve::builtins::{catalog_version, global_builtin_id};
use crate::program::resolve::edge::{Evidence, Route, RouteTarget, Witness};
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
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    use crate::program::graph::{ObjectIndex, ProgramGraph};
    use crate::program::node::AppRegistry;
    use crate::program::node_extract::{ObjectNode, RoutineNode, extract_nodes};
    use crate::program::resolve::body_map::BodyMap;
    use crate::program::resolve::edge::{Evidence, RouteTarget, Witness};
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
