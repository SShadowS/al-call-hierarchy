//! `BodyMap`: maps each `RoutineNodeId` in the program graph to the
//! corresponding borrowed `RoutineDecl` from the parsed IR.
//!
//! The map is built from `&'a [ParsedUnit]` so all entries borrow from the
//! caller-owned parsed snapshot for the lifetime `'a`.

use std::collections::HashMap;

use al_syntax::ir::RoutineDecl;

use crate::program::graph::ProgramGraph;
use crate::program::node::{ObjKey, ObjectNodeId, RoutineNodeId};
use crate::program::sig_fp::source_routine_node_id;
use crate::snapshot::ParsedUnit;

/// Borrows `RoutineDecl`s from a parsed snapshot, indexed by `RoutineNodeId`.
///
/// The object-key logic mirrors [`crate::program::node_extract::extract_nodes`]:
/// use the numeric id when present, otherwise lowercase-name.
/// [`RoutineNodeId`] now includes `enclosing_member_lc` and `params_count`, so
/// same-named member triggers on different fields (e.g. two `OnValidate` field
/// triggers) are stored under distinct keys.  Genuine AL overloads (same name,
/// same enclosing member, DIFFERENT `params_count`) also produce distinct keys
/// and are stored under separate entries.
///
/// Each entry also records the `virtual_path` of the file that declared the
/// routine so that [`get_with_path`][`BodyMap::get_with_path`] can supply the
/// file coordinate for `Witness::SourceSpan` construction in the resolver.
pub struct BodyMap<'a> {
    /// `(RoutineDecl, virtual_path)` — virtual_path is owned so it can outlive
    /// the `ParsedFile` it came from without additional lifetime coupling.
    map: HashMap<RoutineNodeId, (&'a RoutineDecl, String)>,
}

impl<'a> BodyMap<'a> {
    /// Build a `BodyMap` from a parsed snapshot.
    ///
    /// Parsed units whose `AppId` is not present in `graph.apps` are silently
    /// skipped — they were not included in the graph build (open-world gap).
    pub fn build(graph: &ProgramGraph, parsed: &'a [ParsedUnit]) -> Self {
        let mut map = HashMap::new();
        for unit in parsed {
            let Some(app_ref) = graph.apps.find(&unit.app) else {
                continue;
            };
            for pf in &unit.files {
                for obj in &pf.file.objects {
                    let key = match obj.id {
                        Some(n) => ObjKey::Id(n),
                        None => ObjKey::Name(obj.name.to_ascii_lowercase()),
                    };
                    let obj_id = ObjectNodeId {
                        app: app_ref,
                        kind: obj.kind,
                        key,
                    };
                    for routine in &obj.routines {
                        let r_id = source_routine_node_id(obj_id.clone(), routine);
                        // Last-write wins on true same-key collision (same
                        // object + name + enclosing_member); distinct
                        // enclosing_member_lc values produce distinct keys.
                        map.insert(r_id, (routine, pf.virtual_path.clone()));
                    }
                }
            }
        }
        BodyMap { map }
    }

    /// Return the `RoutineDecl` for `id`, or `None` if not present.
    pub fn get(&self, id: &RoutineNodeId) -> Option<&'a RoutineDecl> {
        self.map.get(id).map(|(d, _)| *d)
    }

    /// Return the `RoutineDecl` and the `virtual_path` of the file it came from,
    /// or `None` if the routine was not parsed (symbol-only / absent).
    ///
    /// The virtual path is suitable for `Witness::SourceSpan { file, .. }`.
    pub fn get_with_path(&self, id: &RoutineNodeId) -> Option<(&'a RoutineDecl, &str)> {
        self.map.get(id).map(|(d, p)| (*d, p.as_str()))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::program::graph::{ObjectIndex, ProgramGraph};
    use crate::program::node::{AppRef, AppRegistry, ObjKey, ObjectNodeId, RoutineNodeId};
    use crate::program::topology::DependencyGraph;
    use crate::snapshot::{AppId, ParsedFile, ParsedUnit, Provenance, TrustTier};
    use al_syntax::ir::ObjectKind;

    fn make_app_id(name: &str) -> AppId {
        AppId {
            guid: String::new(),
            name: name.into(),
            publisher: "Test".into(),
            version: "1.0.0.0".into(),
        }
    }

    /// Minimal single-app `ProgramGraph` (no objects/routines — BodyMap only
    /// needs the `apps` registry to resolve `AppId` → `AppRef`).
    fn single_app_graph(app_id: &AppId) -> ProgramGraph {
        let mut apps = AppRegistry::default();
        apps.intern(app_id);
        ProgramGraph {
            apps,
            topology: DependencyGraph::default(),
            objects: vec![],
            routines: vec![],
            obj_index: ObjectIndex::build(&[]),
            ..Default::default()
        }
    }

    fn make_unit(app_id: AppId, src: &'static str) -> ParsedUnit {
        let file = al_syntax::parse(src);
        let provenance = Provenance {
            app: app_id.clone(),
            tier: TrustTier::Workspace,
            content_hash: String::new(),
        };
        ParsedUnit {
            app: app_id,
            files: vec![ParsedFile {
                virtual_path: "Test.al".into(),
                file,
                provenance,
                text: src.to_string(),
            }],
        }
    }

    #[test]
    fn get_returns_correct_routine_decl() {
        let app_id = make_app_id("TestApp");
        let graph = single_app_graph(&app_id);

        let src = r#"
codeunit 50100 "My Codeunit"
{
    procedure DoSomething() begin end;
    procedure DoOther() begin end;
}
"#;
        let unit = make_unit(app_id, src);
        let units = [unit];
        let body_map = BodyMap::build(&graph, &units);

        let obj_id = ObjectNodeId {
            app: AppRef(0),
            kind: ObjectKind::Codeunit,
            key: ObjKey::Id(50100),
        };

        let r1 = RoutineNodeId {
            object: obj_id.clone(),
            name_lc: "dosomething".into(),
            enclosing_member_lc: None,
            params_count: 0,
            sig_fp: 0,
        };
        let decl = body_map.get(&r1).expect("DoSomething must be found");
        assert_eq!(decl.name, "DoSomething");

        let r2 = RoutineNodeId {
            object: obj_id.clone(),
            name_lc: "doother".into(),
            enclosing_member_lc: None,
            params_count: 0,
            sig_fp: 0,
        };
        let decl2 = body_map.get(&r2).expect("DoOther must be found");
        assert_eq!(decl2.name, "DoOther");

        // Absent routine must yield None.
        let absent = RoutineNodeId {
            object: obj_id,
            name_lc: "notexist".into(),
            enclosing_member_lc: None,
            params_count: 0,
            sig_fp: 0,
        };
        assert!(
            body_map.get(&absent).is_none(),
            "absent routine must be None"
        );
    }

    #[test]
    fn build_is_infallible_for_extension_objects() {
        // Extension objects carry an explicit numeric id — key is `ObjKey::Id(n)`.
        let app_id = make_app_id("TestApp");
        let graph = single_app_graph(&app_id);

        let src = r#"
tableextension 50100 "Customer Ext" extends Customer
{
    procedure ExtraHelper() begin end;
}
"#;
        let unit = make_unit(app_id, src);
        let units = [unit];
        let body_map = BodyMap::build(&graph, &units);

        // The extension has id=50100, so the parser uses ObjKey::Id(50100).
        let obj_id = ObjectNodeId {
            app: AppRef(0),
            kind: ObjectKind::TableExtension,
            key: ObjKey::Id(50100),
        };
        let r_id = RoutineNodeId {
            object: obj_id,
            name_lc: "extrahelper".into(),
            enclosing_member_lc: None,
            params_count: 0,
            sig_fp: 0,
        };
        // Verify the routine is found and correctly indexed.
        assert!(
            body_map.get(&r_id).is_some(),
            "ExtraHelper must be found in extension with id=50100"
        );
    }

    #[test]
    fn skips_unit_whose_app_is_not_in_graph() {
        let registered = make_app_id("TestApp");
        let graph = single_app_graph(&registered);

        let other = make_app_id("OtherApp");
        let src = r#"codeunit 50100 "C" { procedure F() begin end; }"#;
        let unit = make_unit(other, src);
        let units = [unit];
        let body_map = BodyMap::build(&graph, &units);

        // `OtherApp` is not in the graph; the map must be empty.
        assert!(
            body_map.map.is_empty(),
            "unit from unknown app must be skipped"
        );
    }

    /// Two `OnValidate` field-triggers on DIFFERENT fields in the same
    /// tableextension must produce DISTINCT `RoutineNodeId`s (differing in
    /// `enclosing_member_lc`) so that `BodyMap` stores both and `get` can
    /// retrieve each independently — no last-write collision.
    ///
    /// This is the canonical regression test for the `enclosing_member_lc`
    /// discriminator added in Phase 2 prereq C.
    #[test]
    fn same_named_field_triggers_are_distinct() {
        let src = r#"
tableextension 50100 "Cust Ext" extends Customer
{
    fields
    {
        field(50100; Foo; Integer) { trigger OnValidate() begin Bar(); end; }
        field(50101; Baz; Integer) { trigger OnValidate() begin Qux(); end; }
    }
}
"#;
        let app_id = make_app_id("TestApp");
        let graph = single_app_graph(&app_id);
        let unit = make_unit(app_id, src);

        // Confirm the IR populates `enclosing_member` for both field triggers.
        // This is the KEY pre-condition for the discriminator to work.
        let parsed_file = &unit.files[0];
        let obj = &parsed_file.file.objects[0];
        let onvalidates: Vec<_> = obj
            .routines
            .iter()
            .filter(|r| r.name.eq_ignore_ascii_case("OnValidate"))
            .collect();
        assert_eq!(
            onvalidates.len(),
            2,
            "IR must expose two OnValidate routines; got {}",
            onvalidates.len()
        );
        let members: Vec<_> = onvalidates
            .iter()
            .map(|r| {
                r.enclosing_member
                    .as_ref()
                    .map(|(n, _)| n.to_ascii_lowercase())
            })
            .collect();
        assert!(
            members.contains(&Some("foo".to_string()))
                && members.contains(&Some("baz".to_string())),
            "IR enclosing_member must be 'Foo'/'Baz' for the two field triggers; got {members:?}"
        );

        // Build the BodyMap and verify both triggers are stored with distinct ids.
        let units = [unit];
        let body_map = BodyMap::build(&graph, &units);

        let obj_id = ObjectNodeId {
            app: AppRef(0),
            kind: ObjectKind::TableExtension,
            key: ObjKey::Id(50100),
        };

        let foo_id = RoutineNodeId {
            object: obj_id.clone(),
            name_lc: "onvalidate".into(),
            enclosing_member_lc: Some("foo".into()),
            params_count: 0,
            sig_fp: 0,
        };
        let baz_id = RoutineNodeId {
            object: obj_id.clone(),
            name_lc: "onvalidate".into(),
            enclosing_member_lc: Some("baz".into()),
            params_count: 0,
            sig_fp: 0,
        };

        // The two RoutineNodeIds must be distinct (the discriminator must differ).
        assert_ne!(
            foo_id, baz_id,
            "two OnValidate triggers on different fields must have distinct RoutineNodeIds"
        );

        // Both must be retrievable from the BodyMap.
        let foo_decl = body_map
            .get(&foo_id)
            .expect("OnValidate for Foo must be in BodyMap");
        let baz_decl = body_map
            .get(&baz_id)
            .expect("OnValidate for Baz must be in BodyMap");

        // Sanity: verify each decl is the trigger for the right field.
        assert_eq!(
            foo_decl
                .enclosing_member
                .as_ref()
                .map(|(n, _)| n.to_ascii_lowercase()),
            Some("foo".to_string()),
            "foo_decl must reference the Foo field"
        );
        assert_eq!(
            baz_decl
                .enclosing_member
                .as_ref()
                .map(|(n, _)| n.to_ascii_lowercase()),
            Some("baz".to_string()),
            "baz_decl must reference the Baz field"
        );
    }
}
