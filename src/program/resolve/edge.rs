//! The multi-axis behaviour-edge model + the obligation-based real-unknown metric.
//! Spec §3 / §3.2.
//!
//! # Reachability contract
//!
//! Routes stored in `Edge.routes` encode ALL possible dispatch targets, including
//! routes that only fire when the caller explicitly calls `BindSubscription` at
//! runtime (`Condition::ManualBinding`).  Code that builds a "who actually fires by
//! default" reachability set **MUST** use [`Edge::default_reachable_routes`] (only
//! unconditionally-bound routes) and **MUST NOT** traverse `ManualBinding` routes as
//! unconditional edges.
//!
//! Use [`Edge::may_reachable_routes`] for opt-in "could this fire" queries (includes
//! `ManualBinding`).  Use [`Edge::all_routes`] exclusively for resolution/gate/
//! classification context (classify_obligation, differential projection).
//!
//! The `routes` field itself is kept `pub` to allow struct-literal construction
//! across the crate.  The named accessors are the enforced API for consumers.

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
    /// The subscriber fires ONLY when `BindSubscription` is called explicitly at runtime.
    /// Routes with this condition do NOT fire by default and MUST be excluded from
    /// default reachability traversal. Use [`Route::fires_by_default`].
    ManualBinding,
    /// The subscriber fires by default but may be skipped at runtime when the
    /// required license is absent.  Treated as fires-by-default for reachability.
    SkipOnMissingLicense,
    /// As `SkipOnMissingLicense` but for permission checks.
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
    /// Zero or more dispatch conditions on this route.  Empty means the route fires
    /// unconditionally.  See [`Condition`] and [`Route::fires_by_default`].
    pub conditions: Vec<Condition>,
    pub witness: Witness,
}

impl Route {
    /// Returns `true` when this route fires by default (no explicit binding required).
    ///
    /// Returns `false` iff `conditions` contains [`Condition::ManualBinding`] — meaning
    /// the subscriber fires only when `BindSubscription` is explicitly called at runtime.
    ///
    /// `SkipOnMissingLicense` and `SkipOnMissingPermission` fire by default (they are
    /// bound; they may runtime-skip but are NOT deferred by a caller binding step).
    /// Only `ManualBinding` causes the route to not fire by default.
    pub fn fires_by_default(&self) -> bool {
        !self.conditions.contains(&Condition::ManualBinding)
    }
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

impl Edge {
    /// All routes on this edge, for **RESOLUTION context** only — gate checks,
    /// `classify_obligation`, and canonical differential projection.
    ///
    /// Reachability consumers MUST use [`default_reachable_routes`][Self::default_reachable_routes]
    /// or [`may_reachable_routes`][Self::may_reachable_routes] instead.
    pub fn all_routes(&self) -> impl Iterator<Item = &Route> {
        self.routes.iter()
    }

    /// Routes that fire by default — excludes [`Condition::ManualBinding`] routes.
    ///
    /// Use for default reachability traversal: "what fires without any explicit
    /// caller action?"  A `ManualBinding` subscriber does NOT fire unless
    /// `BindSubscription` is called; including it in unconditional traversal would
    /// overstate reachability.
    pub fn default_reachable_routes(&self) -> impl Iterator<Item = &Route> {
        self.routes.iter().filter(|r| r.fires_by_default())
    }

    /// All routes that **could** fire, including those requiring explicit
    /// `BindSubscription` (`ManualBinding`).
    ///
    /// Use for opt-in / "may reach" reachability queries.
    pub fn may_reachable_routes(&self) -> impl Iterator<Item = &Route> {
        self.routes.iter()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ObligationOutcome {
    /// ≥1 real route fires by default (unconditional obligation met).
    Resolved,
    /// All real routes require explicit `BindSubscription` (`ManualBinding`).
    ///
    /// Treated as **resolved-for-resolution** (not a gap; the engine found the
    /// targets) but **distinct from `Resolved`** so reachability consumers can
    /// choose whether to traverse these edges.  `real_unknown_rate` does NOT count
    /// this as Unknown.  Use [`Edge::default_reachable_routes`] to exclude these
    /// from unconditional traversal; [`Edge::may_reachable_routes`] to include them.
    ConditionalResolved,
    HonestDynamic,
    HonestEmpty,
    Unknown,
}

/// Classify one edge's resolution obligation (spec §3.2).
pub fn classify_obligation(e: &Edge) -> ObligationOutcome {
    // Collect real routes: non-Unknown evidence AND non-Unresolved target.
    let mut has_real = false;
    let mut all_manual = true; // only meaningful when has_real is true

    for r in &e.routes {
        if r.evidence != Evidence::Unknown && r.target != RouteTarget::Unresolved {
            has_real = true;
            if r.fires_by_default() {
                all_manual = false;
            }
        }
    }

    if has_real {
        return if all_manual {
            // All real routes require explicit BindSubscription — conditional obligation.
            ObligationOutcome::ConditionalResolved
        } else {
            // ≥1 real route fires by default — unconditional obligation met.
            ObligationOutcome::Resolved
        };
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
///
/// `ConditionalResolved` edges count as resolved-for-resolution (not Unknown).
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
    /// Edges where all real routes require explicit `BindSubscription`.
    /// Counted as resolved-for-resolution (not unknown).
    pub conditional_resolved: usize,
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
                ObligationOutcome::ConditionalResolved => h.conditional_resolved += 1,
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
            params_count: 0,
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
                conditions: vec![],
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
            conditions: vec![],
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

    // ---- Phase 4b Task 0: conditions Vec + fires_by_default + ConditionalResolved ----

    fn manual_route() -> Route {
        Route {
            target: RouteTarget::Routine(rid(0, "manual_sub")),
            evidence: Evidence::Source,
            conditions: vec![Condition::ManualBinding],
            witness: Witness::SourceSpan {
                file: "f".into(),
                span: (0, 1),
            },
        }
    }

    fn skip_license_route() -> Route {
        Route {
            target: RouteTarget::Routine(rid(0, "skip_sub")),
            evidence: Evidence::Source,
            conditions: vec![Condition::SkipOnMissingLicense],
            witness: Witness::SourceSpan {
                file: "f".into(),
                span: (2, 3),
            },
        }
    }

    #[test]
    fn route_conditions_vec_holds_multiple() {
        let r = Route {
            target: RouteTarget::Routine(rid(0, "t")),
            evidence: Evidence::Source,
            conditions: vec![Condition::ManualBinding, Condition::SkipOnMissingLicense],
            witness: Witness::SourceSpan {
                file: "f".into(),
                span: (0, 1),
            },
        };
        assert_eq!(r.conditions.len(), 2);
        assert!(r.conditions.contains(&Condition::ManualBinding));
        assert!(r.conditions.contains(&Condition::SkipOnMissingLicense));
    }

    #[test]
    fn fires_by_default_semantics() {
        // empty conditions → fires by default
        let r_empty = Route {
            target: RouteTarget::Routine(rid(0, "a")),
            evidence: Evidence::Source,
            conditions: vec![],
            witness: Witness::SourceSpan {
                file: "f".into(),
                span: (0, 1),
            },
        };
        assert!(
            r_empty.fires_by_default(),
            "empty conditions must fire by default"
        );

        // ManualBinding → does NOT fire by default
        let r_manual = Route {
            target: RouteTarget::Routine(rid(0, "b")),
            evidence: Evidence::Source,
            conditions: vec![Condition::ManualBinding],
            witness: Witness::SourceSpan {
                file: "f".into(),
                span: (0, 1),
            },
        };
        assert!(
            !r_manual.fires_by_default(),
            "ManualBinding must NOT fire by default"
        );

        // SkipOnMissingLicense alone → fires by default (may runtime-skip but is bound)
        let r_skip = Route {
            target: RouteTarget::Routine(rid(0, "c")),
            evidence: Evidence::Source,
            conditions: vec![Condition::SkipOnMissingLicense],
            witness: Witness::SourceSpan {
                file: "f".into(),
                span: (0, 1),
            },
        };
        assert!(
            r_skip.fires_by_default(),
            "SkipOnMissingLicense must fire by default"
        );

        // [ManualBinding, SkipOnMissingLicense] → ManualBinding dominates → false
        let r_both = Route {
            target: RouteTarget::Routine(rid(0, "d")),
            evidence: Evidence::Source,
            conditions: vec![Condition::ManualBinding, Condition::SkipOnMissingLicense],
            witness: Witness::SourceSpan {
                file: "f".into(),
                span: (0, 1),
            },
        };
        assert!(
            !r_both.fires_by_default(),
            "[ManualBinding, Skip*] must NOT fire by default"
        );
    }

    #[test]
    fn conditional_resolved_vs_resolved_vs_empty() {
        // Edge whose ONLY real route is Manual → ConditionalResolved (NOT plain Resolved)
        let manual_edge = edge_with(
            EdgeKind::EventFlow,
            DispatchShape::Multicast,
            SetCompleteness::Complete,
            vec![manual_route()],
        );
        assert_eq!(
            classify_obligation(&manual_edge),
            ObligationOutcome::ConditionalResolved,
            "all-Manual-route edge must classify as ConditionalResolved, not Resolved"
        );

        // Edge with a fires-by-default real route → Resolved
        let default_edge = edge_with(
            EdgeKind::Call,
            DispatchShape::Exact,
            SetCompleteness::Complete,
            vec![src_route()],
        );
        assert_eq!(
            classify_obligation(&default_edge),
            ObligationOutcome::Resolved,
            "fires-by-default route must classify as Resolved"
        );

        // Mixed: one Manual + one fires-by-default → Resolved (≥1 default-firing route)
        let mixed_edge = edge_with(
            EdgeKind::EventFlow,
            DispatchShape::Multicast,
            SetCompleteness::Partial {
                reason: OpenWorldReason::ReverseDependentSubscribers,
            },
            vec![manual_route(), src_route()],
        );
        assert_eq!(
            classify_obligation(&mixed_edge),
            ObligationOutcome::Resolved,
            "mixed Manual+default edge must be Resolved (has ≥1 default-firing route)"
        );

        // SkipOnMissingLicense alone fires-by-default → Resolved (not ConditionalResolved)
        let skip_edge = edge_with(
            EdgeKind::EventFlow,
            DispatchShape::Multicast,
            SetCompleteness::Partial {
                reason: OpenWorldReason::ReverseDependentSubscribers,
            },
            vec![skip_license_route()],
        );
        assert_eq!(
            classify_obligation(&skip_edge),
            ObligationOutcome::Resolved,
            "SkipOnMissingLicense fires by default → Resolved"
        );

        // Empty Multicast + Partial → HonestEmpty (unchanged)
        let empty_edge = edge_with(
            EdgeKind::EventFlow,
            DispatchShape::Multicast,
            SetCompleteness::Partial {
                reason: OpenWorldReason::ReverseDependentSubscribers,
            },
            vec![],
        );
        assert_eq!(
            classify_obligation(&empty_edge),
            ObligationOutcome::HonestEmpty,
            "empty fan-out must still be HonestEmpty"
        );
    }

    #[test]
    fn reachability_accessors_split_manual_from_default() {
        // Edge with one Manual route + one fires-by-default route.
        let edge = edge_with(
            EdgeKind::EventFlow,
            DispatchShape::Multicast,
            SetCompleteness::Partial {
                reason: OpenWorldReason::ReverseDependentSubscribers,
            },
            vec![manual_route(), src_route()],
        );

        // default_reachable_routes: EXCLUDES the Manual route
        let default_r: Vec<_> = edge.default_reachable_routes().collect();
        assert_eq!(
            default_r.len(),
            1,
            "default_reachable_routes must exclude ManualBinding"
        );
        assert!(
            !default_r[0].conditions.contains(&Condition::ManualBinding),
            "the remaining route must not have ManualBinding"
        );

        // may_reachable_routes: INCLUDES both
        let may_r: Vec<_> = edge.may_reachable_routes().collect();
        assert_eq!(
            may_r.len(),
            2,
            "may_reachable_routes must include ManualBinding routes"
        );

        // all_routes: also includes both (resolution context)
        let all_r: Vec<_> = edge.all_routes().collect();
        assert_eq!(all_r.len(), 2, "all_routes must include all routes");
    }
}
