//! The whole-program node graph: app-qualified nodes + topology-scoped lookup.

use al_syntax::ir::ObjectKind;
use std::collections::HashMap;

use crate::program::node::{AppRef, AppRegistry};
use crate::program::node_extract::{ObjectNode, RoutineNode};
use crate::program::topology::DependencyGraph;

/// Index from (app, kind, lowercase-name) to position in `ProgramGraph::objects`.
///
/// Built once after `objects` is sorted; first entry wins on a same-app duplicate.
#[derive(Default)]
pub struct ObjectIndex {
    by_app_kind_name: HashMap<(AppRef, ObjectKind, String), usize>,
}

impl ObjectIndex {
    /// Build the index from an already-sorted `objects` slice.
    /// On a duplicate `(app, kind, name_lc)` key the first (lowest-`NodeId`) entry wins.
    pub fn build(objects: &[ObjectNode]) -> Self {
        let mut idx = ObjectIndex::default();
        for (i, obj) in objects.iter().enumerate() {
            let key = (obj.id.app, obj.id.kind, obj.name.to_ascii_lowercase());
            idx.by_app_kind_name.entry(key).or_insert(i);
        }
        idx
    }
}

/// The assembled whole-program graph: app-qualified nodes + dependency topology.
pub struct ProgramGraph {
    pub apps: AppRegistry,
    pub topology: DependencyGraph,
    /// All object nodes, sorted by `ObjectNodeId` for determinism.
    pub objects: Vec<ObjectNode>,
    /// All routine nodes, sorted by `RoutineNodeId` for determinism.
    pub routines: Vec<RoutineNode>,
    pub obj_index: ObjectIndex,
}

impl ProgramGraph {
    /// Resolve `(kind, name)` as seen FROM `from`.
    ///
    /// Search order:
    /// 1. An object declared in `from` itself (prefer nearest).
    /// 2. The lowest-`ObjectNodeId`-ordered object among `from`'s dependency
    ///    closure (deterministic tiebreak).
    ///
    /// An app that is **not** in `from`'s transitive dependency closure is
    /// never matched — topology-scoped, never flat-global.
    pub fn resolve_object(
        &self,
        from: AppRef,
        kind: ObjectKind,
        name: &str,
    ) -> Option<&ObjectNode> {
        let name_lc = name.to_ascii_lowercase();

        // Prefer `from` itself — short-circuit before building the full closure.
        if let Some(&idx) = self
            .obj_index
            .by_app_kind_name
            .get(&(from, kind, name_lc.clone()))
        {
            return Some(&self.objects[idx]);
        }

        // Search the rest of the closure (excludes `from` — already checked).
        let closure = self.topology.closure(from);
        let mut best: Option<usize> = None;
        for &app in &closure {
            if app == from {
                continue;
            }
            if let Some(&idx) = self
                .obj_index
                .by_app_kind_name
                .get(&(app, kind, name_lc.clone()))
            {
                best = Some(match best {
                    Some(b) if self.objects[b].id <= self.objects[idx].id => b,
                    _ => idx,
                });
            }
        }
        best.map(|i| &self.objects[i])
    }

    /// Look up an interned `AppRef` by name (case-insensitive).
    /// Panics if the name is not present — intended for tests and CLI helpers.
    pub fn app_ref_by_name(&self, name: &str) -> AppRef {
        self.apps
            .find_by_name(name)
            .unwrap_or_else(|| panic!("app not found in registry: {name}"))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use al_syntax::ir::ObjectKind;

    use crate::program::node::{ObjKey, ObjectNodeId};
    use crate::program::node_extract::ObjectNode;
    use crate::program::topology::DependencyGraph;
    use crate::snapshot::{AppId, TrustTier};

    /// Construct a minimal two-app `ProgramGraph` entirely in memory (no I/O).
    ///
    /// Topology: A depends on B; B has no dependencies.
    /// Objects:
    ///   - codeunit "Util"   in A
    ///   - codeunit "Util"   in B
    ///   - codeunit "OnlyInA" in A
    fn build_two_app_fixture() -> ProgramGraph {
        let mut apps = AppRegistry::default();
        let a_id = AppId {
            guid: String::new(),
            name: "AppA".into(),
            publisher: "Test".into(),
            version: "1.0.0.0".into(),
        };
        let b_id = AppId {
            guid: String::new(),
            name: "AppB".into(),
            publisher: "Test".into(),
            version: "1.0.0.0".into(),
        };
        let a = apps.intern(&a_id);
        let b = apps.intern(&b_id);

        let mut topology = DependencyGraph::default();
        topology.add_dependency(a, b); // A → B; B has no reverse dep on A.

        let make_obj = |app: AppRef, name: &str| ObjectNode {
            id: ObjectNodeId {
                app,
                kind: ObjectKind::Codeunit,
                key: ObjKey::Name(name.to_ascii_lowercase()),
            },
            name: name.to_string(),
            declared_id: None,
            extends_target: None,
            implements: vec![],
            tier: TrustTier::Workspace,
            source_table: None,
            table_no: None,
            source_table_temporary: false,
            page_controls: vec![],
        };

        let mut objects = vec![
            make_obj(a, "Util"),
            make_obj(b, "Util"),
            make_obj(a, "OnlyInA"),
        ];
        objects.sort_by(|x, y| x.id.cmp(&y.id));

        let obj_index = ObjectIndex::build(&objects);

        ProgramGraph {
            apps,
            topology,
            objects,
            routines: vec![],
            obj_index,
        }
    }

    #[test]
    fn resolve_object_is_topology_scoped_not_global() {
        // App A depends on B. Both define codeunit "Util". A call from A must
        // resolve to A's own Util (nearest), and B cannot see A's Util at all.
        let g = build_two_app_fixture();
        let a = g.app_ref_by_name("AppA");
        let b = g.app_ref_by_name("AppB");

        let from_a = g.resolve_object(a, ObjectKind::Codeunit, "Util").unwrap();
        assert_eq!(from_a.id.app, a, "A resolves its own Util");

        let from_b = g.resolve_object(b, ObjectKind::Codeunit, "Util").unwrap();
        assert_eq!(from_b.id.app, b, "B resolves its own Util, never A's");

        // B does NOT depend on A, so an A-only object is invisible from B.
        assert!(
            g.resolve_object(b, ObjectKind::Codeunit, "OnlyInA")
                .is_none(),
            "OnlyInA must not be visible from B (B's closure excludes A)"
        );
    }
}
