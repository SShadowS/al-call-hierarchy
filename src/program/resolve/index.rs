//! `ResolveIndex` and `WorldMode`: topology-scoped lookup indexes built from a
//! [`ProgramGraph`].
//!
//! ## Scoping model
//!
//! Two scoping modes govern which objects are visible:
//!
//! - **[`WorldMode::CallerClosure`]**: the caller's compile-time view —
//!   `from` itself plus its transitive dependency closure.  Used for
//!   name/id-based object resolution (`object_by_number`).  Mirrors the
//!   semantics of [`ProgramGraph::resolve_object`] but keys on numeric id
//!   rather than name.
//!
//! - **[`WorldMode::AnalyzedSnapshot`]**: whole-program, all-apps view.  Used
//!   for reverse-dependency queries whose answers depend on apps that live
//!   *outside* a caller's closure (extension targets, interface implementers,
//!   event subscribers).
//!
//! The mode is baked into each method signature rather than passed as a
//! runtime parameter, making the scoping visible and compiler-checkable at
//! each call site.

use std::collections::HashMap;

use al_syntax::ir::ObjectKind;

use crate::program::graph::ProgramGraph;
use crate::program::node::{AppRef, ObjectNodeId, RoutineNodeId};

// ---------------------------------------------------------------------------
// WorldMode
// ---------------------------------------------------------------------------

/// Which slice of the world a lookup is scoped to.
///
/// Callers may carry this value to dispatch between lookup strategies; the
/// `ResolveIndex` methods themselves have the mode baked into their signatures.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorldMode {
    /// Resolve as seen **from** one specific app: self first, then its
    /// transitive dependency closure.  Objects outside the closure are
    /// invisible — same rule the AL compiler applies.
    CallerClosure(AppRef),
    /// Whole-snapshot view: all apps, no scoping.  Required for queries whose
    /// answer depends on reverse-dependency relationships (extension targets,
    /// interface implementers, event subscribers).
    AnalyzedSnapshot,
}

// ---------------------------------------------------------------------------
// ResolveIndex
// ---------------------------------------------------------------------------

/// Pre-built lookup indexes over a [`ProgramGraph`].
///
/// All internal `Vec`s are populated by iterating `graph.objects` and
/// `graph.routines` in their already-sorted (by `NodeId`) order, so every
/// returned list is deterministic without a secondary sort.
pub struct ResolveIndex {
    /// `(object_id, name_lc)` → list of `RoutineNodeId`s (overloads, ≤1 in practice).
    routines_by_obj_name: HashMap<(ObjectNodeId, String), Vec<RoutineNodeId>>,
    /// `(app, kind, declared_id)` → `ObjectNodeId` (first in sorted order for
    /// that app; duplicates within one app silently ignored).
    objs_by_number: HashMap<(AppRef, ObjectKind, i64), ObjectNodeId>,
    /// Lowercased `extends_target` of a `TableExtension` → all extension ids.
    table_extensions: HashMap<String, Vec<ObjectNodeId>>,
    /// Lowercased interface name → all object ids that implement it.
    implementers: HashMap<String, Vec<ObjectNodeId>>,
}

impl ResolveIndex {
    /// Build all indexes from `graph`.
    ///
    /// `graph.objects` and `graph.routines` are already sorted by `NodeId`;
    /// the index preserves that order so every returned `Vec` is deterministic.
    pub fn build(graph: &ProgramGraph) -> Self {
        let mut routines_by_obj_name: HashMap<(ObjectNodeId, String), Vec<RoutineNodeId>> =
            HashMap::new();
        for r in &graph.routines {
            routines_by_obj_name
                .entry((r.id.object.clone(), r.id.name_lc.clone()))
                .or_default()
                .push(r.id.clone());
        }

        let mut objs_by_number: HashMap<(AppRef, ObjectKind, i64), ObjectNodeId> = HashMap::new();
        let mut table_extensions: HashMap<String, Vec<ObjectNodeId>> = HashMap::new();
        let mut implementers: HashMap<String, Vec<ObjectNodeId>> = HashMap::new();

        for obj in &graph.objects {
            // By-number: first sorted entry wins for a given (app, kind, id).
            if let Some(n) = obj.declared_id {
                objs_by_number
                    .entry((obj.id.app, obj.id.kind, n))
                    .or_insert_with(|| obj.id.clone());
            }

            // TableExtension → base table name (lowercased).
            if obj.id.kind == ObjectKind::TableExtension
                && let Some(ref target) = obj.extends_target
            {
                table_extensions
                    .entry(target.to_ascii_lowercase())
                    .or_default()
                    .push(obj.id.clone());
            }

            // Interface implementers.
            for iface in &obj.implements {
                implementers
                    .entry(iface.to_ascii_lowercase())
                    .or_default()
                    .push(obj.id.clone());
            }
        }

        ResolveIndex {
            routines_by_obj_name,
            objs_by_number,
            table_extensions,
            implementers,
        }
    }

    /// All overloads of `name_lc` declared in `obj` — [`WorldMode::CallerClosure`]
    /// or [`WorldMode::AnalyzedSnapshot`] (no scoping needed; the object id is
    /// already fully-qualified).
    ///
    /// Returns an empty slice when nothing is found.
    pub fn routines_in_object(&self, obj: &ObjectNodeId, name_lc: &str) -> &[RoutineNodeId] {
        self.routines_by_obj_name
            .get(&(obj.clone(), name_lc.to_string()))
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Resolve an object by its **numeric AL id** as seen from `from`
    /// ([`WorldMode::CallerClosure`]).
    ///
    /// Search order mirrors [`ProgramGraph::resolve_object`]:
    /// 1. `from` itself (short-circuit before computing the closure).
    /// 2. The lowest-`NodeId` object with the same `(kind, declared_id)` among
    ///    `from`'s transitive dependency closure.
    ///
    /// Objects whose declaring app is NOT in the closure are invisible.
    ///
    /// `graph` is required to compute the transitive closure on demand;
    /// the closure is NOT cached here (Phase 1 — if call-hot, cache in a later
    /// phase or pre-expand into a per-app index).
    pub fn object_by_number(
        &self,
        graph: &ProgramGraph,
        from: AppRef,
        kind: ObjectKind,
        declared_id: i64,
    ) -> Option<ObjectNodeId> {
        // Prefer `from` itself (avoids building the closure set in the common case).
        if let Some(oid) = self.objs_by_number.get(&(from, kind, declared_id)) {
            return Some(oid.clone());
        }

        // Search the rest of the closure (cycle-safe; `from` is skipped below).
        let closure = graph.topology.closure(from);
        let mut best: Option<&ObjectNodeId> = None;
        for &app in &closure {
            if app == from {
                continue;
            }
            if let Some(oid) = self.objs_by_number.get(&(app, kind, declared_id)) {
                best = Some(match best {
                    Some(b) if b <= oid => b,
                    _ => oid,
                });
            }
        }
        best.cloned()
    }

    /// All `TableExtension` objects whose `extends_target` (lowercased) equals
    /// `base_table_name_lc` — [`WorldMode::AnalyzedSnapshot`], whole-program
    /// view (extensions live in reverse-dependent apps, outside the base
    /// table's own closure).
    pub fn table_extensions_of(&self, base_table_name_lc: &str) -> &[ObjectNodeId] {
        self.table_extensions
            .get(base_table_name_lc)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// All objects whose `implements` list (lowercased) contains
    /// `interface_name_lc` — [`WorldMode::AnalyzedSnapshot`], whole-program
    /// view (implementers live in reverse-dependent apps).
    pub fn implementers_of(&self, interface_name_lc: &str) -> &[ObjectNodeId] {
        self.implementers
            .get(interface_name_lc)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Event subscribers of `publisher` — **Phase-1 stub, always empty**.
    ///
    /// Full event modelling (publisher → subscriber fan-out with attribute
    /// matching) is deferred to Phase 4.  The signature is declared here so
    /// Phase 4 can fill it in without changing call sites.
    pub fn subscribers_of(&self, _publisher: &RoutineNodeId) -> Vec<RoutineNodeId> {
        vec![]
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::program::graph::{ObjectIndex, ProgramGraph};
    use crate::program::node::{AppRegistry, ObjKey, ObjectNodeId, RoutineNodeId};
    use crate::program::node_extract::{Access, ObjectNode, RoutineNode};
    use crate::program::topology::DependencyGraph;
    use crate::snapshot::{AppId, TrustTier};
    use al_syntax::ir::ObjectKind;

    fn make_app_id(name: &str) -> AppId {
        AppId {
            guid: String::new(),
            name: name.into(),
            publisher: "Test".into(),
            version: "1.0.0.0".into(),
        }
    }

    fn make_obj(
        app: AppRef,
        kind: ObjectKind,
        declared_id: Option<i64>,
        name: &str,
        extends_target: Option<&str>,
        implements: Vec<&str>,
    ) -> ObjectNode {
        let key = match declared_id {
            Some(n) => ObjKey::Id(n),
            None => ObjKey::Name(name.to_ascii_lowercase()),
        };
        ObjectNode {
            id: ObjectNodeId { app, kind, key },
            name: name.to_string(),
            declared_id,
            extends_target: extends_target.map(str::to_string),
            implements: implements.into_iter().map(str::to_string).collect(),
            tier: TrustTier::Workspace,
        }
    }

    fn make_routine(obj_id: ObjectNodeId, name: &str) -> RoutineNode {
        RoutineNode {
            id: RoutineNodeId {
                object: obj_id,
                name_lc: name.to_ascii_lowercase(),
                enclosing_member_lc: None,
                params_count: 0,
            },
            name: name.to_string(),
            is_trigger: false,
            access: Access::Public,
            tier: TrustTier::Workspace,
        }
    }

    /// Builds a two-app fixture:
    ///
    /// - AppA (`a`, AppRef 0) depends on AppB (`b`, AppRef 1).
    /// - AppB has Table 18 "Customer" and Codeunit 50201 "TheirCU" with routine "Do".
    /// - AppA has TableExtension 50100 extending "Customer" and Codeunit 50200
    ///   "SomeImpl" implementing interface "IFoo".
    fn build_fixture() -> (ProgramGraph, AppRef, AppRef) {
        let mut apps = AppRegistry::default();
        let a = apps.intern(&make_app_id("AppA")); // AppRef(0)
        let b = apps.intern(&make_app_id("AppB")); // AppRef(1)

        let mut topology = DependencyGraph::default();
        topology.add_dependency(a, b); // A sees B's objects; B does not see A's.

        let their_cu_id = ObjectNodeId {
            app: b,
            kind: ObjectKind::Codeunit,
            key: ObjKey::Id(50201),
        };

        let mut objects = vec![
            make_obj(b, ObjectKind::Table, Some(18), "Customer", None, vec![]),
            make_obj(
                a,
                ObjectKind::TableExtension,
                Some(50100),
                "CustomerExt",
                Some("Customer"),
                vec![],
            ),
            make_obj(
                a,
                ObjectKind::Codeunit,
                Some(50200),
                "SomeImpl",
                None,
                vec!["IFoo"],
            ),
            make_obj(
                b,
                ObjectKind::Codeunit,
                Some(50201),
                "TheirCU",
                None,
                vec![],
            ),
        ];
        objects.sort_by(|x, y| x.id.cmp(&y.id));

        let mut routines = vec![make_routine(their_cu_id, "Do")];
        routines.sort_by(|x, y| x.id.cmp(&y.id));

        let obj_index = ObjectIndex::build(&objects);
        (
            ProgramGraph {
                apps,
                topology,
                objects,
                routines,
                obj_index,
            },
            a,
            b,
        )
    }

    // -- object_by_number tests -----------------------------------------------

    #[test]
    fn object_by_number_finds_dep_in_closure() {
        let (graph, a, b) = build_fixture();
        let idx = ResolveIndex::build(&graph);

        // A depends on B; Customer (Table 18) is in B → visible from A.
        let found = idx.object_by_number(&graph, a, ObjectKind::Table, 18);
        assert!(found.is_some(), "Table 18 must be visible from AppA");
        let oid = found.unwrap();
        assert_eq!(oid.app, b, "must resolve to AppB's Customer");
        assert_eq!(oid.key, ObjKey::Id(18));
    }

    #[test]
    fn object_by_number_prefers_self() {
        // Add a codeunit 50201 to AppA as well — from AppA, it should win.
        let (mut graph, a, _b) = build_fixture();
        let extra = make_obj(a, ObjectKind::Codeunit, Some(50201), "OurCU", None, vec![]);
        graph.objects.push(extra);
        graph.objects.sort_by(|x, y| x.id.cmp(&y.id));
        graph.obj_index = ObjectIndex::build(&graph.objects);

        let idx = ResolveIndex::build(&graph);
        let found = idx.object_by_number(&graph, a, ObjectKind::Codeunit, 50201);
        assert!(found.is_some());
        assert_eq!(found.unwrap().app, a, "own app must be preferred over dep");
    }

    #[test]
    fn object_by_number_outside_closure_returns_none() {
        let (graph, _a, b) = build_fixture();
        let idx = ResolveIndex::build(&graph);

        // B does not depend on A; TableExtension 50100 is in A → invisible from B.
        let found = idx.object_by_number(&graph, b, ObjectKind::TableExtension, 50100);
        assert!(
            found.is_none(),
            "TableExtension 50100 (AppA) must not be visible from AppB"
        );

        // Verify A itself also doesn't have it when queried by a completely unknown AppRef.
        let unknown = AppRef(99);
        let found2 = idx.object_by_number(&graph, unknown, ObjectKind::Table, 18);
        assert!(found2.is_none(), "unknown app has empty closure");
    }

    // -- table_extensions_of tests --------------------------------------------

    #[test]
    fn table_extensions_of_returns_extension() {
        let (graph, a, _b) = build_fixture();
        let idx = ResolveIndex::build(&graph);

        let exts = idx.table_extensions_of("customer");
        assert_eq!(exts.len(), 1, "expected exactly one extension of Customer");
        assert_eq!(exts[0].app, a);
        assert_eq!(exts[0].kind, ObjectKind::TableExtension);
    }

    #[test]
    fn table_extensions_of_missing_returns_empty() {
        let (graph, _, _) = build_fixture();
        let idx = ResolveIndex::build(&graph);

        let exts = idx.table_extensions_of("nosuchtable");
        assert!(exts.is_empty());
    }

    // -- implementers_of tests ------------------------------------------------

    #[test]
    fn implementers_of_returns_codeunit() {
        let (graph, a, _b) = build_fixture();
        let idx = ResolveIndex::build(&graph);

        let impls = idx.implementers_of("ifoo");
        assert_eq!(impls.len(), 1, "expected exactly one implementer of IFoo");
        assert_eq!(impls[0].app, a);
        assert_eq!(impls[0].kind, ObjectKind::Codeunit);
    }

    #[test]
    fn implementers_of_missing_returns_empty() {
        let (graph, _, _) = build_fixture();
        let idx = ResolveIndex::build(&graph);

        assert!(idx.implementers_of("ibar").is_empty());
    }

    // -- routines_in_object test ----------------------------------------------

    #[test]
    fn routines_in_object_finds_routine() {
        let (graph, _a, b) = build_fixture();
        let idx = ResolveIndex::build(&graph);

        let their_cu = ObjectNodeId {
            app: b,
            kind: ObjectKind::Codeunit,
            key: ObjKey::Id(50201),
        };
        let rids = idx.routines_in_object(&their_cu, "do");
        assert_eq!(rids.len(), 1);
        assert_eq!(rids[0].name_lc, "do");
    }

    #[test]
    fn routines_in_object_absent_returns_empty() {
        let (graph, _a, b) = build_fixture();
        let idx = ResolveIndex::build(&graph);

        let their_cu = ObjectNodeId {
            app: b,
            kind: ObjectKind::Codeunit,
            key: ObjKey::Id(50201),
        };
        assert!(idx.routines_in_object(&their_cu, "notexist").is_empty());
    }

    // -- subscribers_of stub test ---------------------------------------------

    #[test]
    fn subscribers_of_stub_returns_empty() {
        let (graph, a, _b) = build_fixture();
        let idx = ResolveIndex::build(&graph);

        let fake_pub = RoutineNodeId {
            object: ObjectNodeId {
                app: a,
                kind: ObjectKind::Codeunit,
                key: ObjKey::Id(50200),
            },
            name_lc: "publisher".into(),
            enclosing_member_lc: None,
            params_count: 0,
        };
        assert!(
            idx.subscribers_of(&fake_pub).is_empty(),
            "Phase-1 stub must return empty"
        );
    }

    // -- WorldMode is a value type test ---------------------------------------

    #[test]
    fn world_mode_variants_constructible() {
        let cc = WorldMode::CallerClosure(AppRef(0));
        let snap = WorldMode::AnalyzedSnapshot;
        assert_ne!(cc, snap);
        // CallerClosure equality is by contained AppRef.
        assert_eq!(cc, WorldMode::CallerClosure(AppRef(0)));
        assert_ne!(cc, WorldMode::CallerClosure(AppRef(1)));
    }
}
