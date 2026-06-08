//! `FullRoutineSummary` — the per-routine composite the L5 query helpers read.
//!
//! al-sem's `routine.summary` (`RoutineSummary`, `model/summary.ts`) carries the
//! capability facts AND the coverage record together. The Rust CORE
//! `RoutineSummary` (the L4 summary engine) does NOT — the capability cone
//! (`l4/capability_cone.rs`) produces `CapabilityFact[]` + `CoverageRecord`
//! SEPARATELY. So the L5 substrate re-unifies them into this composite, which the
//! `capability_query` helpers and `transaction_spans` operate on.
//!
//! Mirrors al-sem's reachable-fact semantics exactly:
//!   `reachable(s) = capabilityFactsDirect ∪ capabilityFactsInherited`
//! and the coverage tri-state honours `coverage.inheritedStatus` (None ⇒
//! "unknown", matching al-sem's `s.coverage?.inheritedStatus ?? "unknown"`).
//!
//! Task 2b, when it assembles these from the real pipeline, may add
//! `db_effects` / `parameter_roles` / `uncertainties` / `in_recursive_cycle` /
//! `has_unresolved_calls`. They are OMITTED here because no query helper in this
//! task reads them — `capability_query` only needs facts + coverage, and
//! `transaction_spans` only needs facts + coverage via the query helpers.

use crate::engine::l4::capability_cone::{CapabilityFact, CoverageRecord};

/// A per-routine composite: the routine's direct + inherited capability facts and
/// its coverage record. The `capability_query` helpers read
/// `capability_facts_direct ∪ capability_facts_inherited` and honour
/// `coverage.inherited_status` for the tri-state / G6 semantics.
#[derive(Debug, Clone, PartialEq)]
pub struct FullRoutineSummary {
    /// The routine's INTERNAL id (matches `L3Routine::id`).
    pub routine_id: String,
    /// Direct capability facts emitted by this routine's body.
    pub capability_facts_direct: Vec<CapabilityFact>,
    /// Capability facts inherited from the transitive reachable closure.
    pub capability_facts_inherited: Vec<CapabilityFact>,
    /// Coverage status for the direct + inherited cone. `None` ⇒ helpers treat
    /// `inherited_status` as "unknown" (al-sem `s.coverage?.inheritedStatus ??
    /// "unknown"`).
    pub coverage: Option<CoverageRecord>,
}

impl FullRoutineSummary {
    /// `reachable(s)` — direct ∪ inherited, in al-sem's exact concatenation
    /// order (direct first, then inherited). Returns an empty `Vec` when both are
    /// empty (matching al-sem's early `return []`). Allocates a fresh `Vec` of
    /// references; never mutates the summary.
    pub fn reachable(&self) -> Vec<&CapabilityFact> {
        if self.capability_facts_direct.is_empty() && self.capability_facts_inherited.is_empty() {
            return Vec::new();
        }
        let mut out: Vec<&CapabilityFact> = Vec::with_capacity(
            self.capability_facts_direct.len() + self.capability_facts_inherited.len(),
        );
        out.extend(self.capability_facts_direct.iter());
        out.extend(self.capability_facts_inherited.iter());
        out
    }

    /// The inherited coverage status (`coverage.inherited_status`), or "unknown"
    /// when there is no coverage record. Mirrors al-sem
    /// `s.coverage?.inheritedStatus ?? "unknown"`.
    pub fn inherited_status(&self) -> &str {
        self.coverage
            .as_ref()
            .map(|c| c.inherited_status.as_str())
            .unwrap_or("unknown")
    }
}
