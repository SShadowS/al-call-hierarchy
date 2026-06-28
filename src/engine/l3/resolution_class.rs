//! Honest resolution taxonomy (spec §6) — a READ-ONLY classifier over resolved
//! call edges. Maps the fine-grained `(resolution, dispatch_kind)` pair into the
//! 4 honest buckets the spec's north-star metric needs: `builtin` (platform
//! method, not a hole), `dynamic` (RecordRef/runtime — genuinely indeterminate),
//! `external` (resolves into a dependency object), and `unknown` (a TRUE
//! resolution failure — the FN signal to drive toward zero). `resolved` /
//! `ambiguous` / `member-not-found` are kept as their own buckets so the metric
//! can report them separately.
//!
//! This module does NOT mutate any edge — it is the measurement lens used by the
//! `aldump --l3-call-graph-stats` harness and the Phase-2 contract oracles.

use super::call_resolver::CallEdge;
use super::taxonomy::{DispatchKind, Resolution};

/// The honest taxonomy bucket for one edge (spec §6 + the reporting extras).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolutionClass {
    /// Resolved to a concrete AL routine (`resolved`) or a polymorphic impl set
    /// (`maybe`, interface dispatch). Not a hole.
    Resolved,
    /// Platform/intrinsic method — no AL target. Not a hole.
    Builtin,
    /// RecordRef/runtime/variant dispatch — genuinely indeterminate.
    Dynamic,
    /// Resolves into a dependency object (member `external-target`, or `opaque`).
    External,
    /// Overload set with >1 surviving candidate.
    Ambiguous,
    /// Object found, method (or arity) not present on it.
    MemberNotFound,
    /// TRUE resolution failure — the FN signal.
    Unknown,
}

/// Classify ONE edge from its `(Resolution, DispatchKind)` enum pair. Pure, total,
/// never panics. `Dynamic` dispatch kind is keyed specially because a dynamic
/// object-run target has `resolution == Unknown(...)` but is NOT a true failure.
pub fn classify(resolution: Resolution, dispatch_kind: DispatchKind) -> ResolutionClass {
    if dispatch_kind == DispatchKind::Dynamic {
        return ResolutionClass::Dynamic;
    }
    match resolution {
        Resolution::Resolved | Resolution::Maybe => ResolutionClass::Resolved,
        Resolution::Builtin => ResolutionClass::Builtin,
        Resolution::ExternalTarget | Resolution::Opaque => ResolutionClass::External,
        Resolution::Ambiguous => ResolutionClass::Ambiguous,
        Resolution::MemberNotFound => ResolutionClass::MemberNotFound,
        Resolution::Unknown(_) => ResolutionClass::Unknown,
    }
}

/// A resolution histogram over an edge set. `total` counts every edge; the named
/// fields are the per-bucket tallies. The "real-unknown rate" is the spec's
/// north-star metric (true `unknown` / total).
#[derive(Debug, Default, Clone, Copy, serde::Serialize)]
pub struct Histogram {
    pub total: usize,
    pub resolved: usize,
    pub builtin: usize,
    pub dynamic: usize,
    pub external: usize,
    pub ambiguous: usize,
    #[serde(rename = "memberNotFound")]
    pub member_not_found: usize,
    pub unknown: usize,
}

impl Histogram {
    pub fn of_edges(edges: &[CallEdge]) -> Histogram {
        let mut h = Histogram::default();
        for e in edges {
            h.total += 1;
            match classify(e.resolution, e.dispatch_kind) {
                ResolutionClass::Resolved => h.resolved += 1,
                ResolutionClass::Builtin => h.builtin += 1,
                ResolutionClass::Dynamic => h.dynamic += 1,
                ResolutionClass::External => h.external += 1,
                ResolutionClass::Ambiguous => h.ambiguous += 1,
                ResolutionClass::MemberNotFound => h.member_not_found += 1,
                ResolutionClass::Unknown => h.unknown += 1,
            }
        }
        h
    }

    /// TRUE-unknown edges / total. `0.0` for an empty set.
    pub fn real_unknown_rate(&self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            self.unknown as f64 / self.total as f64
        }
    }
}

/// Breakdown of the TRUE-`unknown` edges (those that `classify` as
/// [`ResolutionClass::Unknown`]) by their resolver-attributed `UnknownReason`
/// label. `"unattributed"` collects any `unknown` edge missing a reason (should be
/// zero — every unknown-emission site sets one). Attributes the residual
/// real-`unknown` rate to its causes (the typed-resolution work-list).
///
/// Returns a 4-tuple:
///   0 — `byReason` histogram (keyed by `UnknownReason::label()`).
///   1 — `frameworkMethodDetail` (keyed by `"Kind::method_lc"` for
///        `FrameworkMethodNotInCatalog` edges).
///   2 — `receiverShapeDetail` (keyed by `"{reason_label}::{shape}"` for edges
///        carrying a `receiver_shape` tag — sub-characterizes `untracked-receiver`
///        and `compound-receiver` buckets; for `untracked-receiver::other` and
///        `compound-receiver::member-of-member` the concrete name/expression is
///        embedded in the key so the detail is surfaced as counts-per-expression).
///   3 — `bareCallDetail` (keyed by lowercased bare call name for
///        `BareUnresolved` edges — names the residual for catalog-gap analysis).
#[allow(clippy::type_complexity)] // 3 parallel breakdown maps; a struct adds no clarity here
pub fn unknown_breakdown(
    edges: &[CallEdge],
) -> (
    std::collections::BTreeMap<&'static str, usize>,
    std::collections::BTreeMap<String, usize>,
    std::collections::BTreeMap<String, usize>,
    std::collections::BTreeMap<String, usize>,
) {
    use super::call_resolver::UnknownReason;
    let mut m: std::collections::BTreeMap<&'static str, usize> = std::collections::BTreeMap::new();
    let mut fw_detail: std::collections::BTreeMap<String, usize> =
        std::collections::BTreeMap::new();
    let mut shape_detail: std::collections::BTreeMap<String, usize> =
        std::collections::BTreeMap::new();
    let mut bare_detail: std::collections::BTreeMap<String, usize> =
        std::collections::BTreeMap::new();
    for e in edges {
        if classify(e.resolution, e.dispatch_kind) != ResolutionClass::Unknown {
            continue;
        }
        let reason = e.resolution.unknown_reason();
        let label = reason.map(|r| r.label()).unwrap_or("unattributed");
        *m.entry(label).or_insert(0) += 1;
        if let Some(ref name) = e.unknown_method_name {
            match reason {
                Some(UnknownReason::BareUnresolved) => {
                    *bare_detail.entry(name.clone()).or_insert(0) += 1;
                }
                _ => {
                    // FrameworkMethodNotInCatalog and any future users.
                    *fw_detail.entry(name.clone()).or_insert(0) += 1;
                }
            }
        }
        if let Some(ref shape) = e.receiver_shape {
            let key = format!("{label}::{shape}");
            *shape_detail.entry(key).or_insert(0) += 1;
        }
    }
    (m, fw_detail, shape_detail, bare_detail)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_the_honest_buckets() {
        use super::super::call_resolver::UnknownReason;
        assert_eq!(
            classify(Resolution::Resolved, DispatchKind::Direct),
            ResolutionClass::Resolved
        );
        assert_eq!(
            classify(Resolution::Maybe, DispatchKind::Interface),
            ResolutionClass::Resolved
        );
        assert_eq!(
            classify(Resolution::Builtin, DispatchKind::Builtin),
            ResolutionClass::Builtin
        );
        assert_eq!(
            classify(
                Resolution::Unknown(UnknownReason::BareUnresolved),
                DispatchKind::Dynamic
            ),
            ResolutionClass::Dynamic
        );
        assert_eq!(
            classify(Resolution::ExternalTarget, DispatchKind::Method),
            ResolutionClass::External
        );
        assert_eq!(
            classify(Resolution::Opaque, DispatchKind::CodeunitRun),
            ResolutionClass::External
        );
        assert_eq!(
            classify(Resolution::Ambiguous, DispatchKind::Direct),
            ResolutionClass::Ambiguous
        );
        assert_eq!(
            classify(Resolution::MemberNotFound, DispatchKind::Method),
            ResolutionClass::MemberNotFound
        );
        assert_eq!(
            classify(
                Resolution::Unknown(UnknownReason::BareUnresolved),
                DispatchKind::Method
            ),
            ResolutionClass::Unknown
        );
        assert_eq!(
            classify(
                Resolution::Unknown(UnknownReason::BareUnresolved),
                DispatchKind::Unresolved
            ),
            ResolutionClass::Unknown
        );
    }

    #[test]
    fn histogram_counts_and_real_unknown_rate() {
        use crate::engine::l3::call_resolver::{CallEdge, UnknownReason};
        let edges = vec![
            {
                let mut e = CallEdge::base("f", "c", "o");
                e.resolution = Resolution::Resolved;
                e.dispatch_kind = DispatchKind::Direct;
                e
            },
            {
                let mut e = CallEdge::base("f", "c", "o");
                e.resolution = Resolution::Builtin;
                e.dispatch_kind = DispatchKind::Builtin;
                e
            },
            {
                let mut e = CallEdge::base("f", "c", "o");
                e.resolution = Resolution::Builtin;
                e.dispatch_kind = DispatchKind::Builtin;
                e
            },
            {
                let mut e = CallEdge::base("f", "c", "o");
                e.resolution = Resolution::Unknown(UnknownReason::BareUnresolved);
                e.dispatch_kind = DispatchKind::Method;
                e
            },
            {
                let mut e = CallEdge::base("f", "c", "o");
                e.resolution = Resolution::Unknown(UnknownReason::DynamicObjectRunTarget);
                e.dispatch_kind = DispatchKind::Dynamic;
                e
            },
        ];
        let h = Histogram::of_edges(&edges);
        assert_eq!(h.total, 5);
        assert_eq!(h.resolved, 1);
        assert_eq!(h.builtin, 2);
        assert_eq!(h.unknown, 1);
        assert_eq!(h.dynamic, 1);
        assert!((h.real_unknown_rate() - 0.2).abs() < 1e-9);
    }

    #[test]
    fn unknown_breakdown_buckets_by_reason_and_excludes_non_unknown() {
        use crate::engine::l3::call_resolver::{CallEdge, UnknownReason};
        let unk = |reason: UnknownReason| {
            let mut e = CallEdge::base("f", "c", "o");
            e.resolution = Resolution::Unknown(reason);
            e.dispatch_kind = DispatchKind::Method;
            e
        };
        let mut edges = vec![
            unk(UnknownReason::RecordTableProcedure),
            unk(UnknownReason::RecordTableProcedure),
            unk(UnknownReason::UntrackedReceiver),
        ];
        // A dynamic-dispatch edge (resolution unknown but dispatch_kind dynamic) is
        // NOT a true unknown -- excluded. A resolved edge -- excluded.
        let mut dyn_edge = CallEdge::base("f", "c", "o");
        dyn_edge.resolution = Resolution::Unknown(UnknownReason::DynamicObjectRunTarget);
        dyn_edge.dispatch_kind = DispatchKind::Dynamic;
        edges.push(dyn_edge);
        let mut resolved = CallEdge::base("f", "c", "o");
        resolved.resolution = Resolution::Resolved;
        resolved.dispatch_kind = DispatchKind::Direct;
        edges.push(resolved);

        let (bd, _detail, _shape_detail, _bare_detail) = unknown_breakdown(&edges);
        assert_eq!(bd.get("record-table-procedure"), Some(&2));
        assert_eq!(bd.get("untracked-receiver"), Some(&1));
        // dynamic + resolved are excluded; no "unattributed".
        assert_eq!(bd.values().sum::<usize>(), 3);
        assert!(!bd.contains_key("unattributed"));
    }
}
