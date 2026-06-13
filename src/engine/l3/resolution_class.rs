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

/// Classify ONE edge from its `(resolution, dispatch_kind)` pair. Pure, total,
/// never panics. `dynamic` is keyed on the dispatch kind because a dynamic
/// object-run target stores `resolution == "unknown"` today (it is NOT a true
/// failure — it is a known-dynamic site).
pub fn classify(resolution: &str, dispatch_kind: &str) -> ResolutionClass {
    if dispatch_kind == "dynamic" {
        return ResolutionClass::Dynamic;
    }
    match resolution {
        "resolved" | "maybe" => ResolutionClass::Resolved,
        "builtin" => ResolutionClass::Builtin,
        "external-target" | "opaque" => ResolutionClass::External,
        "ambiguous" => ResolutionClass::Ambiguous,
        "member-not-found" => ResolutionClass::MemberNotFound,
        _ => ResolutionClass::Unknown,
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
            match classify(&e.resolution, &e.dispatch_kind) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_the_honest_buckets() {
        assert_eq!(classify("resolved", "direct"), ResolutionClass::Resolved);
        assert_eq!(classify("maybe", "interface"), ResolutionClass::Resolved);
        assert_eq!(classify("builtin", "builtin"), ResolutionClass::Builtin);
        assert_eq!(classify("unknown", "dynamic"), ResolutionClass::Dynamic);
        assert_eq!(
            classify("external-target", "method"),
            ResolutionClass::External
        );
        assert_eq!(
            classify("opaque", "codeunit-run"),
            ResolutionClass::External
        );
        assert_eq!(classify("ambiguous", "direct"), ResolutionClass::Ambiguous);
        assert_eq!(
            classify("member-not-found", "method"),
            ResolutionClass::MemberNotFound
        );
        assert_eq!(classify("unknown", "method"), ResolutionClass::Unknown);
        assert_eq!(classify("unknown", "unresolved"), ResolutionClass::Unknown);
    }

    #[test]
    fn histogram_counts_and_real_unknown_rate() {
        use crate::engine::l3::call_resolver::CallEdge;
        let edge = |res: &str, dk: &str| {
            let mut e = CallEdge::base("f", "c", "o");
            e.resolution = res.to_string();
            e.dispatch_kind = dk.to_string();
            e
        };
        let edges = vec![
            edge("resolved", "direct"),
            edge("builtin", "builtin"),
            edge("builtin", "builtin"),
            edge("unknown", "method"),
            edge("unknown", "dynamic"),
        ];
        let h = Histogram::of_edges(&edges);
        assert_eq!(h.total, 5);
        assert_eq!(h.resolved, 1);
        assert_eq!(h.builtin, 2);
        assert_eq!(h.unknown, 1);
        assert_eq!(h.dynamic, 1);
        assert!((h.real_unknown_rate() - 0.2).abs() < 1e-9);
    }
}
