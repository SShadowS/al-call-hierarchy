//! Capability-query helpers — faithful port of al-sem
//! `src/detectors/capability-query.ts`.
//!
//! Pure functions over a `FullRoutineSummary`. Every helper reads
//! `reachable(s) = capability_facts_direct ∪ capability_facts_inherited` and
//! returns a derived view. The tri-state helpers honour G6 coverage semantics:
//! when a fact is absent AND the inherited cone is not "complete", they return
//! `Unknown` rather than `No` (absence of evidence is not evidence of absence
//! when the cone is partial / coverage data is missing).
//!
//! `writes_tables_of` / `publishes_events_of` drop facts with no `resource_id`
//! and return SORTED + DEDUPED lists. Determinism: both use a `BTreeSet`, so the
//! output ordering is a stable function of the inputs.

use std::collections::BTreeSet;

use crate::engine::l4::capability_cone::{CapabilityExtra, CapabilityFact};
use crate::engine::l5::full_summary::FullRoutineSummary;

/// True when a capability fact is a write/read on a PROVABLY temporary record
/// (`extra == Table { temp_state: known/true }`). Such ops are in-memory — they
/// never touch the physical database, so they cannot create cross-routine /
/// cross-extension table conflicts or exposure. Mirrors the detector-side
/// `is_known_temp` gate, lifted to the cone-fact level (the `temp_state` is
/// preserved across inheritance via `retag`). Suppression-direction safe: only
/// the exact `known/true` signal qualifies; `Unknown` / parameter-dependent /
/// absent temp_state keep counting.
pub fn fact_is_known_temp(f: &CapabilityFact) -> bool {
    matches!(
        &f.extra,
        Some(CapabilityExtra::Table { temp_state: Some(ts), .. })
            if ts.kind == "known" && ts.value == Some(true)
    )
}

/// Tri-state effect presence (al-sem `EffectPresence = "yes" | "no" | "unknown"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectPresence {
    Yes,
    No,
    Unknown,
}

/// Table-write ops (al-sem `TABLE_WRITE_OPS = {insert, modify, delete}`).
fn is_table_write_op(op: &str) -> bool {
    matches!(op, "insert" | "modify" | "delete")
}

/// Filter reachable facts (direct + inherited) by an arbitrary predicate.
/// Returns a fresh `Vec` of references; never mutates the summary. Mirrors
/// al-sem `findCapabilities`.
pub fn find_capabilities<P>(s: &FullRoutineSummary, predicate: P) -> Vec<&CapabilityFact>
where
    P: Fn(&CapabilityFact) -> bool,
{
    s.reachable().into_iter().filter(|f| predicate(f)).collect()
}

/// True when at least one reachable fact has the given `(op, resource_kind)`.
/// Strict discrimination — no fuzzy / substring matching. Mirrors al-sem
/// `hasCapability`.
pub fn has_capability(s: &FullRoutineSummary, op: &str, kind: &str) -> bool {
    s.reachable()
        .iter()
        .any(|f| f.op == op && f.resource_kind == kind)
}

/// Sorted + deduped TableIds targeted by any reachable insert/modify/delete fact
/// on `resource_kind == "table"` with a known `resource_id`. Facts whose table
/// identity is unresolved (no `resource_id`) are DROPPED. Read facts are NOT
/// included. Mirrors al-sem `writesTablesOf`.
pub fn writes_tables_of(s: &FullRoutineSummary) -> Vec<String> {
    let mut ids: BTreeSet<String> = BTreeSet::new();
    for f in s.reachable() {
        if f.resource_kind != "table" {
            continue;
        }
        if !is_table_write_op(&f.op) {
            continue;
        }
        let Some(rid) = &f.resource_id else {
            continue;
        };
        ids.insert(rid.clone());
    }
    ids.into_iter().collect()
}

/// Like `writes_tables_of` but DROPS writes to provably-temporary records
/// (`fact_is_known_temp`). A temp-table write is in-memory and does no physical
/// DB work, so it must not inflate the transaction-manager table-count gate (d8)
/// nor produce cross-extension conflict/exposure findings (d43/d45). Physical /
/// Unknown / parameter-dependent writes are retained (suppression-direction safe).
pub fn writes_physical_tables_of(s: &FullRoutineSummary) -> Vec<String> {
    let mut ids: BTreeSet<String> = BTreeSet::new();
    for f in s.reachable() {
        if f.resource_kind != "table" {
            continue;
        }
        if !is_table_write_op(&f.op) {
            continue;
        }
        if fact_is_known_temp(f) {
            continue;
        }
        let Some(rid) = &f.resource_id else {
            continue;
        };
        ids.insert(rid.clone());
    }
    ids.into_iter().collect()
}

/// Returns `Yes` when any reachable fact is a commit on the transaction
/// resource; `No` when no commit fact AND the inherited cone is "complete";
/// `Unknown` otherwise (G6 honesty). Mirrors al-sem `mayCommit`.
pub fn may_commit(s: &FullRoutineSummary) -> EffectPresence {
    for f in s.reachable() {
        if f.op == "commit" && f.resource_kind == "transaction" {
            return EffectPresence::Yes;
        }
    }
    if s.inherited_status() == "complete" {
        EffectPresence::No
    } else {
        EffectPresence::Unknown
    }
}

/// Returns `Yes` when any reachable fact has `resource_kind == "table"`
/// (regardless of op — read or write); `No` when no such fact AND the inherited
/// cone is "complete"; `Unknown` otherwise. Mirrors al-sem `touchesDbOf`.
pub fn touches_db_of(s: &FullRoutineSummary) -> EffectPresence {
    for f in s.reachable() {
        if f.resource_kind == "table" {
            return EffectPresence::Yes;
        }
    }
    if s.inherited_status() == "complete" {
        EffectPresence::No
    } else {
        EffectPresence::Unknown
    }
}

/// Sorted + deduped EventIds (plain strings) from reachable `op == "publish"`
/// facts on `resource_kind == "event"` with a known `resource_id`. Facts whose
/// event identity is unresolved are DROPPED. Mirrors al-sem `publishesEventsOf`.
pub fn publishes_events_of(s: &FullRoutineSummary) -> Vec<String> {
    let mut ids: BTreeSet<String> = BTreeSet::new();
    for f in s.reachable() {
        if f.op != "publish" {
            continue;
        }
        if f.resource_kind != "event" {
            continue;
        }
        let Some(rid) = &f.resource_id else {
            continue;
        };
        ids.insert(rid.clone());
    }
    ids.into_iter().collect()
}

/// Returns the routine's inherited coverage status (`coverage.inherited_status`),
/// or "unknown" when there is no coverage record. The optional `kind` is accepted
/// for al-sem signature parity but (as in al-sem Phase 1a) does NOT narrow — the
/// per-routine overall status is the only roll-up maintained. Mirrors al-sem
/// `reachableCoverage`.
pub fn reachable_coverage<'a>(s: &'a FullRoutineSummary, _kind: Option<&str>) -> &'a str {
    s.inherited_status()
}

// ===========================================================================
// Native oracles — ground-truth-free invariants on synthetic inputs.
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::l5::test_support::{coverage, fact, summary};

    #[test]
    fn writes_tables_is_sorted_deduped_drops_unresolved_and_reads() {
        let s = summary(
            "r",
            vec![
                fact("insert", "table", Some("t/B")), // write, kept
                fact("modify", "table", Some("t/A")), // write, kept
                fact("modify", "table", Some("t/A")), // dup → deduped
                fact("delete", "table", None),        // no resource_id → dropped
                fact("read", "table", Some("t/C")),   // read → not a write
                fact("insert", "event", Some("e/X")), // wrong kind → dropped
            ],
            vec![],
            None,
        );
        assert_eq!(writes_tables_of(&s), vec!["t/A", "t/B"]);
    }

    /// Build a `table` write fact carrying a `CapabilityExtra::Table` with the
    /// given temp_state kind/value — for the physical-vs-temp gate tests.
    fn temp_table_write_fact(resource_id: &str, kind: &str, value: Option<bool>) -> CapabilityFact {
        let mut f = fact("insert", "table", Some(resource_id));
        f.extra = Some(CapabilityExtra::Table {
            record_variable_id: None,
            temp_state: Some(crate::engine::l2::features::PTempState {
                kind: kind.to_string(),
                value,
                parameter_index: None,
            }),
            op_subtype: Some("Insert".to_string()),
        });
        f
    }

    #[test]
    fn writes_physical_drops_known_temp_but_keeps_physical_and_unknown() {
        let s = summary(
            "r",
            vec![
                temp_table_write_fact("t/Temp", "known", Some(true)), // in-memory → dropped
                temp_table_write_fact("t/Phys", "known", Some(false)), // not temp → kept
                temp_table_write_fact("t/Unk", "unknown", None),      // unknown → kept
                fact("insert", "table", Some("t/Plain")),             // no extra → kept
            ],
            vec![],
            None,
        );
        // The faithful port still counts every write (parity unchanged).
        assert_eq!(
            writes_tables_of(&s),
            vec!["t/Phys", "t/Plain", "t/Temp", "t/Unk"]
        );
        // The temp-aware variant drops only the provably-temp write.
        assert_eq!(
            writes_physical_tables_of(&s),
            vec!["t/Phys", "t/Plain", "t/Unk"]
        );
    }

    #[test]
    fn fact_is_known_temp_is_exact() {
        assert!(fact_is_known_temp(&temp_table_write_fact(
            "t/A",
            "known",
            Some(true)
        )));
        assert!(!fact_is_known_temp(&temp_table_write_fact(
            "t/A",
            "known",
            Some(false)
        )));
        assert!(!fact_is_known_temp(&temp_table_write_fact(
            "t/A", "unknown", None
        )));
        // No extra at all → not known-temp.
        assert!(!fact_is_known_temp(&fact("insert", "table", Some("t/A"))));
    }

    #[test]
    fn writes_tables_spans_direct_and_inherited() {
        let s = summary(
            "r",
            vec![fact("insert", "table", Some("t/direct"))],
            vec![fact("modify", "table", Some("t/inherited"))],
            None,
        );
        assert_eq!(writes_tables_of(&s), vec!["t/direct", "t/inherited"]);
    }

    #[test]
    fn may_commit_yes_on_matching_fact() {
        let s = summary("r", vec![fact("commit", "transaction", None)], vec![], None);
        assert_eq!(may_commit(&s), EffectPresence::Yes);
    }

    #[test]
    fn may_commit_no_only_when_absent_and_complete() {
        // absent + complete → No
        let s = summary("r", vec![], vec![], Some(coverage("complete")));
        assert_eq!(may_commit(&s), EffectPresence::No);
        // absent + partial → Unknown
        let s = summary("r", vec![], vec![], Some(coverage("partial")));
        assert_eq!(may_commit(&s), EffectPresence::Unknown);
        // absent + no coverage record → Unknown
        let s = summary("r", vec![], vec![], None);
        assert_eq!(may_commit(&s), EffectPresence::Unknown);
    }

    #[test]
    fn touches_db_yes_no_unknown() {
        // any table fact (even a read) → Yes
        let s = summary(
            "r",
            vec![fact("read", "table", Some("t/A"))],
            vec![],
            Some(coverage("complete")),
        );
        assert_eq!(touches_db_of(&s), EffectPresence::Yes);
        // no table fact + complete → No
        let s = summary(
            "r",
            vec![fact("commit", "transaction", None)],
            vec![],
            Some(coverage("complete")),
        );
        assert_eq!(touches_db_of(&s), EffectPresence::No);
        // no table fact + partial → Unknown
        let s = summary("r", vec![], vec![], Some(coverage("partial")));
        assert_eq!(touches_db_of(&s), EffectPresence::Unknown);
    }

    #[test]
    fn publishes_events_sorted_deduped_drops_unresolved() {
        let s = summary(
            "r",
            vec![
                fact("publish", "event", Some("e/B")),
                fact("publish", "event", Some("e/A")),
                fact("publish", "event", Some("e/A")),   // dup
                fact("publish", "event", None),          // dropped
                fact("subscribe", "event", Some("e/Z")), // wrong op
                fact("publish", "table", Some("t/Q")),   // wrong kind
            ],
            vec![],
            None,
        );
        assert_eq!(publishes_events_of(&s), vec!["e/A", "e/B"]);
    }

    #[test]
    fn has_capability_is_strict() {
        let s = summary("r", vec![fact("send", "http", None)], vec![], None);
        assert!(has_capability(&s, "send", "http"));
        assert!(!has_capability(&s, "send", "table"));
        assert!(!has_capability(&s, "read", "http"));
    }

    #[test]
    fn find_capabilities_filters_reachable() {
        let s = summary(
            "r",
            vec![fact("insert", "table", Some("t/A"))],
            vec![fact("read", "table", Some("t/B"))],
            None,
        );
        let writes = find_capabilities(&s, |f| f.op == "insert");
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].resource_id.as_deref(), Some("t/A"));
    }

    #[test]
    fn reachable_coverage_reports_inherited_status_or_unknown() {
        let s = summary("r", vec![], vec![], Some(coverage("complete")));
        assert_eq!(reachable_coverage(&s, None), "complete");
        let s = summary("r", vec![], vec![], None);
        assert_eq!(reachable_coverage(&s, Some("table")), "unknown");
    }
}
