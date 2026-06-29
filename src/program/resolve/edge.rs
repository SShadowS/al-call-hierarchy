//! The multi-axis behaviour-edge model + the obligation-based real-unknown metric.
//! Spec §3 / §3.2.

use crate::program::node::{AppRef, RoutineNodeId};

/// Caller / target identity is a 1B.1 app-qualified routine node.
pub type NodeId = RoutineNodeId;

/// A platform builtin's catalog identity (clean-room catalog id; Phase 2+).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BuiltinId(pub String);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SourcePos {
    pub line: u32,
    pub col: u32,
}

/// Line/col span in a named source unit — the coordinate BOTH engines align on
/// (L3 records line/col via `PAnchor`; the fresh side converts IR byte-origins).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CanonicalSpan {
    pub unit: String,
    pub start: SourcePos,
    pub end: SourcePos,
}

/// Stable SEMANTIC identity of an originating site (spec §6.1) — span-based, never positional.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SiteId {
    pub caller: NodeId,
    pub span: CanonicalSpan,
    pub callee_fingerprint: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EdgeKind {
    Call,
    Run,
    ImplicitTrigger,
    EventFlow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DispatchShape {
    Exact,
    Polymorphic,
    Multicast,
    DynamicOpen,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum OpenWorldReason {
    ReverseDependentImplementers,
    ReverseDependentSubscribers,
    ReverseDependentExtensions,
    RuntimeTypeUnbounded,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SetCompleteness {
    /// Provably exhaustive (sealed / closed-world snapshot) — NOT merely "enumerated the snapshot".
    Complete,
    /// Open world may add routes; also the edge-level home of a DynamicOpen blocker
    /// (`RuntimeTypeUnbounded`) and of a legal empty fan-out.
    Partial { reason: OpenWorldReason },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Evidence {
    Source,
    Abi,
    Catalog,
    /// ABI body-unavailable boundary ONLY — never a visibility conclusion (spec §5.4).
    Opaque,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Condition {
    RunTriggerGuarded,
    ManualBinding,
    SkipOnMissingLicense,
    SkipOnMissingPermission,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RouteTarget {
    Routine(NodeId),
    Builtin(BuiltinId),
    /// Known public boundary whose body is unavailable — retains symbol identity.
    AbiSymbol {
        app: AppRef,
        symbol_key: String,
    },
    /// Genuine failure only — pairs with `Evidence::Unknown`.
    Unresolved,
}

/// Independent-checkability handle for a route's evidence (spec §5.5).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Witness {
    SourceSpan {
        file: String,
        span: (u32, u32),
    },
    AbiSymbol {
        app: AppRef,
        symbol_key: String,
    },
    CatalogEntry {
        id: BuiltinId,
        catalog_version: String,
    },
    None,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Route {
    pub target: RouteTarget,
    pub evidence: Evidence,
    pub condition: Option<Condition>,
    pub witness: Witness,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Edge {
    pub from: NodeId,
    pub site: SiteId,
    pub kind: EdgeKind,
    pub shape: DispatchShape,
    pub completeness: SetCompleteness,
    pub routes: Vec<Route>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ObligationOutcome {
    Resolved,
    HonestDynamic,
    HonestEmpty,
    Unknown,
}

/// Classify one edge's resolution obligation (spec §3.2).
pub fn classify_obligation(e: &Edge) -> ObligationOutcome {
    let has_real_route = e
        .routes
        .iter()
        .any(|r| r.evidence != Evidence::Unknown && r.target != RouteTarget::Unresolved);
    if has_real_route {
        return ObligationOutcome::Resolved;
    }
    if e.shape == DispatchShape::DynamicOpen {
        return ObligationOutcome::HonestDynamic;
    }
    let is_fanout = matches!(
        e.shape,
        DispatchShape::Polymorphic | DispatchShape::Multicast
    );
    let is_open = matches!(e.completeness, SetCompleteness::Partial { .. });
    if e.routes.is_empty() && is_fanout && is_open {
        return ObligationOutcome::HonestEmpty;
    }
    ObligationOutcome::Unknown
}

/// real-unknown = Unknown obligations / all obligations (spec §3.2).
pub fn real_unknown_rate(edges: &[Edge]) -> f64 {
    if edges.is_empty() {
        return 0.0;
    }
    let unknown = edges
        .iter()
        .filter(|e| classify_obligation(e) == ObligationOutcome::Unknown)
        .count();
    unknown as f64 / edges.len() as f64
}

/// Deterministic (within a process run) fingerprint of a callee's text, used as
/// part of a call site's identity. BOTH the fresh and L3 projections must use THIS
/// function so the differential cannot drift.
pub(crate) fn callee_fp(text: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    text.to_ascii_lowercase().hash(&mut h);
    h.finish()
}

/// Stratified counts for `--program-call-graph-stats`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Histogram {
    pub total: usize,
    pub resolved: usize,
    pub honest_dynamic: usize,
    pub honest_empty: usize,
    pub unknown: usize,
}

impl Histogram {
    pub fn of_edges(edges: &[Edge]) -> Histogram {
        let mut h = Histogram::default();
        for e in edges {
            h.total += 1;
            match classify_obligation(e) {
                ObligationOutcome::Resolved => h.resolved += 1,
                ObligationOutcome::HonestDynamic => h.honest_dynamic += 1,
                ObligationOutcome::HonestEmpty => h.honest_empty += 1,
                ObligationOutcome::Unknown => h.unknown += 1,
            }
        }
        h
    }
    pub fn real_unknown_rate(&self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            self.unknown as f64 / self.total as f64
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::program::node::{AppRef, ObjKey, ObjectKind, ObjectNodeId, RoutineNodeId};

    fn rid(app: u32, name: &str) -> RoutineNodeId {
        RoutineNodeId {
            object: ObjectNodeId {
                app: AppRef(app),
                kind: ObjectKind::Codeunit,
                key: ObjKey::Id(1),
            },
            name_lc: name.to_string(),
            enclosing_member_lc: None,
        }
    }

    #[test]
    fn edge_constructs_and_is_orderable() {
        let e = Edge {
            from: rid(0, "post"),
            site: SiteId {
                caller: rid(0, "post"),
                span: CanonicalSpan {
                    unit: "u".into(),
                    start: SourcePos { line: 1, col: 1 },
                    end: SourcePos { line: 1, col: 9 },
                },
                callee_fingerprint: 42,
            },
            kind: EdgeKind::Call,
            shape: DispatchShape::Exact,
            completeness: SetCompleteness::Complete,
            routes: vec![Route {
                target: RouteTarget::Routine(rid(0, "helper")),
                evidence: Evidence::Source,
                condition: None,
                witness: Witness::SourceSpan {
                    file: "f.al".into(),
                    span: (10, 20),
                },
            }],
        };
        assert_eq!(e.routes.len(), 1);
        // Hashable + comparable (needed by the differential).
        let mut v = vec![e.clone(), e];
        v.sort();
        assert_eq!(v.len(), 2);
    }

    fn edge_with(
        kind: EdgeKind,
        shape: DispatchShape,
        comp: SetCompleteness,
        routes: Vec<Route>,
    ) -> Edge {
        Edge {
            from: rid(0, "c"),
            site: SiteId {
                caller: rid(0, "c"),
                span: CanonicalSpan {
                    unit: "u".into(),
                    start: SourcePos { line: 1, col: 1 },
                    end: SourcePos { line: 1, col: 2 },
                },
                callee_fingerprint: 1,
            },
            kind,
            shape,
            completeness: comp,
            routes,
        }
    }

    fn src_route() -> Route {
        Route {
            target: RouteTarget::Routine(rid(0, "t")),
            evidence: Evidence::Source,
            condition: None,
            witness: Witness::SourceSpan {
                file: "f".into(),
                span: (0, 1),
            },
        }
    }

    #[test]
    fn obligation_outcomes_are_correct() {
        // Resolved: >=1 non-Unknown route.
        assert_eq!(
            classify_obligation(&edge_with(
                EdgeKind::Call,
                DispatchShape::Exact,
                SetCompleteness::Complete,
                vec![src_route()]
            )),
            ObligationOutcome::Resolved
        );
        // HonestDynamic: DynamicOpen.
        assert_eq!(
            classify_obligation(&edge_with(
                EdgeKind::Run,
                DispatchShape::DynamicOpen,
                SetCompleteness::Partial {
                    reason: OpenWorldReason::RuntimeTypeUnbounded
                },
                vec![]
            )),
            ObligationOutcome::HonestDynamic
        );
        // HonestEmpty: fan-out, zero routes, Partial.
        assert_eq!(
            classify_obligation(&edge_with(
                EdgeKind::EventFlow,
                DispatchShape::Multicast,
                SetCompleteness::Partial {
                    reason: OpenWorldReason::ReverseDependentSubscribers
                },
                vec![]
            )),
            ObligationOutcome::HonestEmpty
        );
        // Unknown: Exact Call with no target.
        assert_eq!(
            classify_obligation(&edge_with(
                EdgeKind::Call,
                DispatchShape::Exact,
                SetCompleteness::Complete,
                vec![]
            )),
            ObligationOutcome::Unknown
        );
        // Metric: 1 Unknown out of 4 obligations.
        let edges = vec![
            edge_with(
                EdgeKind::Call,
                DispatchShape::Exact,
                SetCompleteness::Complete,
                vec![src_route()],
            ),
            edge_with(
                EdgeKind::Run,
                DispatchShape::DynamicOpen,
                SetCompleteness::Partial {
                    reason: OpenWorldReason::RuntimeTypeUnbounded,
                },
                vec![],
            ),
            edge_with(
                EdgeKind::EventFlow,
                DispatchShape::Multicast,
                SetCompleteness::Partial {
                    reason: OpenWorldReason::ReverseDependentSubscribers,
                },
                vec![],
            ),
            edge_with(
                EdgeKind::Call,
                DispatchShape::Exact,
                SetCompleteness::Complete,
                vec![],
            ),
        ];
        assert!((real_unknown_rate(&edges) - 0.25).abs() < 1e-9);
    }
}
