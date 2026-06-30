//! Phase 0 Task 4: stub resolver.
//!
//! Emits one `Unknown`-obligation [`Edge`] per call site extracted from the
//! parsed snapshot.  Every route is `Unresolved` / `Unknown` — a starting
//! obligation for the incremental resolver to fill in later phases.
//!
//! # Multi-object-file limitation
//! A single `.al` file may contain multiple objects.  [`extract_raw_sites`]
//! is called once per file and returns sites tagged only with
//! `caller_routine` (the routine name, lower-cased).  When two objects in the
//! same file share a routine name, the site is assigned to BOTH routines.
//! Phase 1 resolvers will re-extract sites per-object, eliminating this
//! ambiguity.

use crate::program::graph::ProgramGraph;
use crate::program::node::{ObjKey, ObjectNodeId, RoutineNodeId};
use crate::program::resolve::body_map::BodyMap;
use crate::program::resolve::edge::{
    DispatchShape, Edge, EdgeKind, Evidence, Route, RouteTarget, SetCompleteness, SiteId, Witness,
    callee_fp,
};
use crate::program::resolve::extract_min::extract_raw_sites;
use crate::program::resolve::index::ResolveIndex;
use crate::program::resolve::resolver::emit_event_flow_edges;
use crate::snapshot::ParsedUnit;

/// Emit one `Unknown`-route `Edge` per extracted call site across all parsed
/// units.
///
/// Sites are correlated to routines by name (lower-cased); the caller's
/// `RoutineNodeId` is built using the same `ObjKey` logic as
/// [`crate::program::node_extract::extract_nodes`].  Units whose `AppId` is
/// not present in `graph.apps` are silently skipped (they were not included in
/// the graph build — open-world gap, counted as a known limitation).
#[must_use]
pub fn resolve_program(graph: &ProgramGraph, parsed: &[ParsedUnit]) -> Vec<Edge> {
    let mut edges = Vec::new();

    for unit in parsed {
        let Some(app_ref) = graph.apps.find(&unit.app) else {
            continue;
        };

        for pf in &unit.files {
            let sites = extract_raw_sites(&pf.file, &pf.text, &pf.virtual_path);

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
                    let name_lc = routine.name.to_ascii_lowercase();
                    let caller = RoutineNodeId {
                        object: obj_id.clone(),
                        name_lc: name_lc.clone(),
                        enclosing_member_lc: routine
                            .enclosing_member
                            .as_ref()
                            .map(|(n, _)| n.to_ascii_lowercase()),
                        params_count: routine.params.len(),
                    };

                    for site in sites.iter().filter(|s| s.caller_routine == name_lc) {
                        let fingerprint = callee_fp(&site.callee_text);
                        edges.push(Edge {
                            from: caller.clone(),
                            site: SiteId {
                                caller: caller.clone(),
                                span: site.span.clone(),
                                callee_fingerprint: fingerprint,
                            },
                            kind: EdgeKind::Call,
                            shape: DispatchShape::Exact,
                            completeness: SetCompleteness::Complete,
                            routes: vec![Route {
                                target: RouteTarget::Unresolved,
                                evidence: Evidence::Unknown,
                                conditions: vec![],
                                witness: Witness::None,
                            }],
                        });
                    }
                }
            }
        }
    }

    // Phase 4b Task 3: append publisher-anchored EventFlow Multicast edges.
    // Build ResolveIndex + BodyMap from the same inputs; both are used only
    // within this call so the BodyMap lifetime is contained here.
    let index = ResolveIndex::build(graph);
    let body_map = BodyMap::build(graph, parsed);
    edges.extend(emit_event_flow_edges(graph, &index, &body_map));

    edges
}

/// Test helper: one stub `Edge` with a fabricated `RoutineNodeId` and an
/// `Unknown` route.  Used by the `differential` unit test without requiring a
/// real snapshot.
#[cfg(test)]
pub fn synthetic_unknown_edge_for_test() -> Vec<Edge> {
    use crate::program::node::{AppRef, ObjectKind};
    use crate::program::resolve::edge::{CanonicalSpan, SourcePos};

    let caller = RoutineNodeId {
        object: ObjectNodeId {
            app: AppRef(0),
            kind: ObjectKind::Codeunit,
            key: ObjKey::Id(99_999),
        },
        name_lc: "test_routine".to_string(),
        enclosing_member_lc: None,
        params_count: 0,
    };
    vec![Edge {
        from: caller.clone(),
        site: SiteId {
            caller: caller.clone(),
            span: CanonicalSpan {
                unit: "Test.al".to_string(),
                start: SourcePos { line: 0, col: 0 },
                end: SourcePos { line: 0, col: 10 },
            },
            callee_fingerprint: 42,
        },
        kind: EdgeKind::Call,
        shape: DispatchShape::Exact,
        completeness: SetCompleteness::Complete,
        routes: vec![Route {
            target: RouteTarget::Unresolved,
            evidence: Evidence::Unknown,
            conditions: vec![],
            witness: Witness::None,
        }],
    }]
}
