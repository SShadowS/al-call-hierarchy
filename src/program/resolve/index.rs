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
use crate::program::resolve::edge::Condition;
use crate::program::resolve::event::ParsedSubscriberArgs;

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
// SubscriberEntry / AmbiguousSub — public types produced by the event index
// ---------------------------------------------------------------------------

/// A resolved event-subscriber for one publisher routine.
pub struct SubscriberEntry {
    /// The subscriber routine that will fire when the publisher fires.
    pub subscriber: RoutineNodeId,
    /// Dispatch conditions on this subscription (empty = unconditional).
    pub conditions: Vec<Condition>,
    /// Element filter from the `[EventSubscriber]` attribute, if present.
    pub element: Option<String>,
}

/// A subscription that could not be resolved to exactly one publisher overload.
pub struct AmbiguousSub {
    /// The subscriber routine carrying the unresolvable `[EventSubscriber]`.
    pub subscriber: RoutineNodeId,
    /// The publisher object that was found.
    pub publisher_object: ObjectNodeId,
    /// Lowercased event name from the attribute.
    pub event_name_lc: String,
    /// Number of candidate overloads that matched the arity filter.
    pub candidate_count: usize,
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
    /// Publisher `RoutineNodeId` → ordered list of resolved subscribers.
    subscribers_map: HashMap<RoutineNodeId, Vec<SubscriberEntry>>,
    /// Subscriptions that could not be resolved to a single overload.
    ambiguous_subscriptions: Vec<AmbiguousSub>,
}

impl ResolveIndex {
    /// Build all indexes from `graph`.
    ///
    /// `graph.objects` and `graph.routines` are already sorted by `NodeId`;
    /// the index preserves that order so every returned `Vec` is deterministic.
    pub fn build(graph: &ProgramGraph) -> Self {
        let mut routines_by_obj_name: HashMap<(ObjectNodeId, String), Vec<RoutineNodeId>> =
            HashMap::new();
        // routine_by_id maps each RoutineNodeId to its index in graph.routines so we can
        // look up publisher_kind during subscriber resolution below.
        let mut routine_by_id: HashMap<RoutineNodeId, usize> = HashMap::new();
        for (i, r) in graph.routines.iter().enumerate() {
            routines_by_obj_name
                .entry((r.id.object.clone(), r.id.name_lc.clone()))
                .or_default()
                .push(r.id.clone());
            routine_by_id.insert(r.id.clone(), i);
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

        // ── Event subscriber index ────────────────────────────────────────────
        let mut subscribers_map: HashMap<RoutineNodeId, Vec<SubscriberEntry>> = HashMap::new();
        let mut ambiguous_subscriptions: Vec<AmbiguousSub> = Vec::new();

        for sub_routine in &graph.routines {
            if sub_routine.event_subscribers.is_empty() {
                continue;
            }
            let sub_app = sub_routine.id.object.app;
            let sub_params = sub_routine.id.params_count;

            for args in &sub_routine.event_subscribers {
                // (a) Map publisher_object_type → ObjectKind; unknown type → drop.
                let Some(kind) = kind_from_object_type_str(&args.publisher_object_type) else {
                    continue;
                };

                // (b) Resolve publisher object; unresolvable → drop.
                let Some(pub_obj) = graph.resolve_object(sub_app, kind, &args.publisher_name)
                else {
                    continue;
                };
                let pub_obj_id = pub_obj.id.clone();
                let event_name_lc = args.event_name.to_ascii_lowercase();

                // (c) Candidates: routines in that object matching name +
                //     publisher_kind.is_some() + params_count >= sub_params.
                let candidates: Vec<RoutineNodeId> = routines_by_obj_name
                    .get(&(pub_obj_id.clone(), event_name_lc.clone()))
                    .map(Vec::as_slice)
                    .unwrap_or(&[])
                    .iter()
                    .filter(|rid| {
                        rid.params_count >= sub_params
                            && routine_by_id
                                .get(*rid)
                                .is_some_and(|&i| graph.routines[i].publisher_kind.is_some())
                    })
                    .cloned()
                    .collect();

                // (d) Dispatch on candidate count.
                match candidates.len() {
                    0 => continue,
                    1 => {
                        subscribers_map
                            .entry(candidates[0].clone())
                            .or_default()
                            .push(build_entry(sub_routine, args));
                    }
                    _ => {
                        // MORE THAN ONE: check for exactly one strict arity match.
                        let strict: Vec<&RoutineNodeId> = candidates
                            .iter()
                            .filter(|rid| rid.params_count == sub_params)
                            .collect();
                        if strict.len() == 1 {
                            subscribers_map
                                .entry(strict[0].clone())
                                .or_default()
                                .push(build_entry(sub_routine, args));
                        } else {
                            ambiguous_subscriptions.push(AmbiguousSub {
                                subscriber: sub_routine.id.clone(),
                                publisher_object: pub_obj_id,
                                event_name_lc,
                                candidate_count: candidates.len(),
                            });
                        }
                    }
                }
            }
        }

        // Sort each entry list by subscriber RoutineNodeId for determinism.
        for entries in subscribers_map.values_mut() {
            entries.sort_by(|a, b| a.subscriber.cmp(&b.subscriber));
        }

        ResolveIndex {
            routines_by_obj_name,
            objs_by_number,
            table_extensions,
            implementers,
            subscribers_map,
            ambiguous_subscriptions,
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

    /// All resolved event subscribers of `publisher` — [`WorldMode::AnalyzedSnapshot`].
    ///
    /// Returns a deterministically sorted (by `subscriber` `RoutineNodeId`) slice.
    /// Empty when `publisher` is not a publisher routine or has no subscribers.
    pub fn subscribers_of(&self, publisher: &RoutineNodeId) -> &[SubscriberEntry] {
        self.subscribers_map
            .get(publisher)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Subscriptions that matched a publisher object but could not be resolved
    /// to a single overload (multiple candidates, no unique strict arity match).
    pub fn ambiguous_subscriptions(&self) -> &[AmbiguousSub] {
        &self.ambiguous_subscriptions
    }
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Build a [`SubscriberEntry`] from a resolved subscriber routine + parsed args.
fn build_entry(
    sub_routine: &crate::program::node_extract::RoutineNode,
    args: &ParsedSubscriberArgs,
) -> SubscriberEntry {
    let mut conditions = Vec::new();
    if sub_routine.subscriber_instance_manual {
        conditions.push(Condition::ManualBinding);
    }
    if args.skip_on_missing_license {
        conditions.push(Condition::SkipOnMissingLicense);
    }
    if args.skip_on_missing_permission {
        conditions.push(Condition::SkipOnMissingPermission);
    }
    SubscriberEntry {
        subscriber: sub_routine.id.clone(),
        conditions,
        element: args.element.clone(),
    }
}

/// Map a lowercased publisher-object-type string (as written in an
/// `[EventSubscriber]` attribute) to the corresponding [`ObjectKind`].
/// Returns `None` for unrecognised strings.
fn kind_from_object_type_str(s: &str) -> Option<ObjectKind> {
    match s {
        "codeunit" => Some(ObjectKind::Codeunit),
        "table" => Some(ObjectKind::Table),
        "tableextension" => Some(ObjectKind::TableExtension),
        "page" => Some(ObjectKind::Page),
        "pageextension" => Some(ObjectKind::PageExtension),
        "report" => Some(ObjectKind::Report),
        "reportextension" => Some(ObjectKind::ReportExtension),
        "query" => Some(ObjectKind::Query),
        "xmlport" => Some(ObjectKind::XmlPort),
        "enum" => Some(ObjectKind::Enum),
        "enumextension" => Some(ObjectKind::EnumExtension),
        "interface" => Some(ObjectKind::Interface),
        _ => None,
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
    use crate::program::node_extract::{Access, ObjectNode, RoutineNode};
    use crate::program::resolve::edge::Condition;
    use crate::program::resolve::event::{ParsedSubscriberArgs, PublisherKind};
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
            event_subscribers: vec![],
            subscriber_instance_manual: false,
            publisher_kind: None,
        }
    }

    fn make_publisher(
        obj_id: ObjectNodeId,
        name: &str,
        params: usize,
        kind: PublisherKind,
    ) -> RoutineNode {
        RoutineNode {
            id: RoutineNodeId {
                object: obj_id,
                name_lc: name.to_ascii_lowercase(),
                enclosing_member_lc: None,
                params_count: params,
            },
            name: name.to_string(),
            is_trigger: false,
            access: Access::Public,
            tier: TrustTier::Workspace,
            event_subscribers: vec![],
            subscriber_instance_manual: false,
            publisher_kind: Some(kind),
        }
    }

    fn make_subscriber(
        obj_id: ObjectNodeId,
        name: &str,
        params: usize,
        subs: Vec<ParsedSubscriberArgs>,
        manual: bool,
    ) -> RoutineNode {
        RoutineNode {
            id: RoutineNodeId {
                object: obj_id,
                name_lc: name.to_ascii_lowercase(),
                enclosing_member_lc: None,
                params_count: params,
            },
            name: name.to_string(),
            is_trigger: false,
            access: Access::Public,
            tier: TrustTier::Workspace,
            event_subscribers: subs,
            subscriber_instance_manual: manual,
            publisher_kind: None,
        }
    }

    fn sub_args(pub_name: &str, event: &str) -> ParsedSubscriberArgs {
        ParsedSubscriberArgs {
            publisher_object_type: "codeunit".to_string(),
            publisher_name: pub_name.to_string(),
            event_name: event.to_string(),
            element: None,
            skip_on_missing_license: false,
            skip_on_missing_permission: false,
        }
    }

    /// Single-app fixture with Codeunit 1 "Pub" and Codeunit 2 "Sub".
    fn build_event_fixture(
        pub_routines: Vec<RoutineNode>,
        sub_routines: Vec<RoutineNode>,
    ) -> (ProgramGraph, ObjectNodeId, ObjectNodeId) {
        let mut apps = AppRegistry::default();
        let app = apps.intern(&make_app_id("App"));
        let topology = DependencyGraph::default();

        let pub_id = ObjectNodeId {
            app,
            kind: ObjectKind::Codeunit,
            key: ObjKey::Id(1),
        };
        let sub_id = ObjectNodeId {
            app,
            kind: ObjectKind::Codeunit,
            key: ObjKey::Id(2),
        };

        let pub_obj = make_obj(app, ObjectKind::Codeunit, Some(1), "Pub", None, vec![]);
        let sub_obj = make_obj(app, ObjectKind::Codeunit, Some(2), "Sub", None, vec![]);

        let mut objects = vec![pub_obj, sub_obj];
        objects.sort_by(|x, y| x.id.cmp(&y.id));

        let mut routines: Vec<RoutineNode> = [pub_routines, sub_routines].concat();
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
            pub_id,
            sub_id,
        )
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

    // -- subscribers_of tests -------------------------------------------------

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
            "unknown publisher must return empty"
        );
    }

    // (a) Basic manual subscriber --------------------------------------------

    #[test]
    fn subscribers_of_basic_manual() {
        let app = AppRef(0); // deterministic: first intern in a fresh registry
        let pub_id = ObjectNodeId {
            app,
            kind: ObjectKind::Codeunit,
            key: ObjKey::Id(1),
        };
        let sub_id = ObjectNodeId {
            app,
            kind: ObjectKind::Codeunit,
            key: ObjKey::Id(2),
        };
        let pub_onafterx_id = RoutineNodeId {
            object: pub_id.clone(),
            name_lc: "onafterx".to_string(),
            enclosing_member_lc: None,
            params_count: 0,
        };

        let (graph, _, _) = build_event_fixture(
            vec![make_publisher(
                pub_id.clone(),
                "OnAfterX",
                0,
                PublisherKind::Integration,
            )],
            vec![make_subscriber(
                sub_id,
                "Handler",
                0,
                vec![sub_args("pub", "onafterx")],
                true,
            )],
        );

        let idx = ResolveIndex::build(&graph);
        let subs = idx.subscribers_of(&pub_onafterx_id);
        assert_eq!(subs.len(), 1);
        assert_eq!(subs[0].conditions, vec![Condition::ManualBinding]);
        assert_eq!(subs[0].element, None);
    }

    // (b) One handler subscribing to two different events --------------------

    #[test]
    fn subscribers_of_handler_with_two_event_subscriber_attrs() {
        let app = AppRef(0);
        let pub_id = ObjectNodeId {
            app,
            kind: ObjectKind::Codeunit,
            key: ObjKey::Id(1),
        };
        let sub_id = ObjectNodeId {
            app,
            kind: ObjectKind::Codeunit,
            key: ObjKey::Id(2),
        };
        let pub_onafterx_id = RoutineNodeId {
            object: pub_id.clone(),
            name_lc: "onafterx".to_string(),
            enclosing_member_lc: None,
            params_count: 0,
        };
        let pub_onbeforex_id = RoutineNodeId {
            object: pub_id.clone(),
            name_lc: "onbeforex".to_string(),
            enclosing_member_lc: None,
            params_count: 0,
        };

        let (graph, _, _) = build_event_fixture(
            vec![
                make_publisher(pub_id.clone(), "OnAfterX", 0, PublisherKind::Integration),
                make_publisher(pub_id.clone(), "OnBeforeX", 0, PublisherKind::Integration),
            ],
            vec![make_subscriber(
                sub_id,
                "Handler",
                0,
                vec![sub_args("pub", "onafterx"), sub_args("pub", "onbeforex")],
                false,
            )],
        );

        let idx = ResolveIndex::build(&graph);
        assert_eq!(idx.subscribers_of(&pub_onafterx_id).len(), 1);
        assert_eq!(idx.subscribers_of(&pub_onbeforex_id).len(), 1);
    }

    // (c) SkipOnMissingLicense condition -------------------------------------

    #[test]
    fn subscribers_of_skip_on_missing_license() {
        let app = AppRef(0);
        let pub_id = ObjectNodeId {
            app,
            kind: ObjectKind::Codeunit,
            key: ObjKey::Id(1),
        };
        let sub_id = ObjectNodeId {
            app,
            kind: ObjectKind::Codeunit,
            key: ObjKey::Id(2),
        };
        let pub_onafterx_id = RoutineNodeId {
            object: pub_id.clone(),
            name_lc: "onafterx".to_string(),
            enclosing_member_lc: None,
            params_count: 0,
        };

        let mut sa = sub_args("pub", "onafterx");
        sa.skip_on_missing_license = true;

        let (graph, _, _) = build_event_fixture(
            vec![make_publisher(
                pub_id.clone(),
                "OnAfterX",
                0,
                PublisherKind::Integration,
            )],
            vec![make_subscriber(sub_id, "Handler", 0, vec![sa], false)],
        );

        let idx = ResolveIndex::build(&graph);
        let subs = idx.subscribers_of(&pub_onafterx_id);
        assert_eq!(subs.len(), 1);
        assert!(
            subs[0]
                .conditions
                .contains(&Condition::SkipOnMissingLicense)
        );
        assert!(!subs[0].conditions.contains(&Condition::ManualBinding));
    }

    // (d) Ambiguous overloads — no strict arity match → AmbiguousSub --------

    #[test]
    fn subscribers_of_ambiguous_overloads_no_strict_match() {
        let app = AppRef(0);
        let pub_id = ObjectNodeId {
            app,
            kind: ObjectKind::Codeunit,
            key: ObjKey::Id(1),
        };
        let sub_id = ObjectNodeId {
            app,
            kind: ObjectKind::Codeunit,
            key: ObjKey::Id(2),
        };
        let pub_onafterx_1param_id = RoutineNodeId {
            object: pub_id.clone(),
            name_lc: "onafterx".to_string(),
            enclosing_member_lc: None,
            params_count: 1,
        };
        let pub_onafterx_2param_id = RoutineNodeId {
            object: pub_id.clone(),
            name_lc: "onafterx".to_string(),
            enclosing_member_lc: None,
            params_count: 2,
        };

        // Subscriber params=0: both overloads satisfy >=0 but neither equals 0.
        let (graph, _, _) = build_event_fixture(
            vec![
                make_publisher(pub_id.clone(), "OnAfterX", 1, PublisherKind::Integration),
                make_publisher(pub_id.clone(), "OnAfterX", 2, PublisherKind::Integration),
            ],
            vec![make_subscriber(
                sub_id,
                "Handler",
                0,
                vec![sub_args("pub", "onafterx")],
                false,
            )],
        );

        let idx = ResolveIndex::build(&graph);
        assert!(idx.subscribers_of(&pub_onafterx_1param_id).is_empty());
        assert!(idx.subscribers_of(&pub_onafterx_2param_id).is_empty());
        assert_eq!(idx.ambiguous_subscriptions().len(), 1);
        assert_eq!(idx.ambiguous_subscriptions()[0].candidate_count, 2);
    }

    // (e) Unresolvable publisher — no panic ----------------------------------

    #[test]
    fn subscribers_of_unresolvable_publisher_no_panic() {
        let app = AppRef(0);
        let sub_id = ObjectNodeId {
            app,
            kind: ObjectKind::Codeunit,
            key: ObjKey::Id(2),
        };

        let (graph, _, _) = build_event_fixture(
            vec![], // no publishers at all
            vec![make_subscriber(
                sub_id,
                "Handler",
                0,
                vec![sub_args("nonexistent", "onevent")],
                false,
            )],
        );

        let idx = ResolveIndex::build(&graph);
        // Publisher not found → silently dropped, no panic.
        assert!(idx.ambiguous_subscriptions().is_empty());
    }

    // (f) Two overloads, exactly one strict arity match → resolved -----------

    #[test]
    fn subscribers_of_unique_strict_arity_match_resolves() {
        let app = AppRef(0);
        let pub_id = ObjectNodeId {
            app,
            kind: ObjectKind::Codeunit,
            key: ObjKey::Id(1),
        };
        let sub_id = ObjectNodeId {
            app,
            kind: ObjectKind::Codeunit,
            key: ObjKey::Id(2),
        };
        let pub_onafterx_0param_id = RoutineNodeId {
            object: pub_id.clone(),
            name_lc: "onafterx".to_string(),
            enclosing_member_lc: None,
            params_count: 0,
        };
        let pub_onafterx_1param_id = RoutineNodeId {
            object: pub_id.clone(),
            name_lc: "onafterx".to_string(),
            enclosing_member_lc: None,
            params_count: 1,
        };

        // Subscriber params=0: both >=0, but exactly ONE has params==0.
        let (graph, _, _) = build_event_fixture(
            vec![
                make_publisher(pub_id.clone(), "OnAfterX", 0, PublisherKind::Integration),
                make_publisher(pub_id.clone(), "OnAfterX", 1, PublisherKind::Integration),
            ],
            vec![make_subscriber(
                sub_id,
                "Handler",
                0,
                vec![sub_args("pub", "onafterx")],
                false,
            )],
        );

        let idx = ResolveIndex::build(&graph);
        assert_eq!(idx.subscribers_of(&pub_onafterx_0param_id).len(), 1);
        assert!(idx.subscribers_of(&pub_onafterx_1param_id).is_empty());
        assert!(idx.ambiguous_subscriptions().is_empty());
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
