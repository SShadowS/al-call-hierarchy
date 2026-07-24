//! ⟨C1⟩ The DUAL-RUN parity oracle — derived cone substrate vs. the raw
//! `capability_facts_inherited` Vec, asserted **per routine**.
//!
//! While both representations exist (Tasks 1-2), this is the real proof that the
//! compact [`ConeDerivedStore`] is a sufficient replacement. Findings goldens are
//! not: they only observe routines that produce a finding, and only through
//! whichever predicate that detector happens to call. This oracle asserts EVERY
//! derived predicate for EVERY routine in the context.
//!
//! It is opt-in and value-tested — `C1_CONE_PARITY=1` (the repo's
//! `REGEN_TEMP_GOLDENS` convention; `C1_CONE_PARITY=0` does NOT enable it) — and
//! panics on the first divergence with the routine id, the predicate, and both
//! values. Run it over the golden corpora and over a real workspace (`CDO_WS`).
//!
//! The raw side is computed HERE, independently: `capability_query`'s helpers for
//! the shared predicates, and hand-replicated copies of d44's and d48's own
//! closures for theirs — so the oracle re-derives the detector's view rather than
//! echoing a shared helper that could itself be wrong.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::OnceLock;

use crate::engine::l4::capability_cone::CapabilityFact;
use crate::engine::l4::cone_derived::ConeDerivedStore;
use crate::engine::l5::capability_query::{
    fact_is_known_temp, may_commit, may_commit_derived, publishes_events_of, reachable_coverage,
    touches_db_derived, touches_db_of, writes_physical_tables_of, writes_tables_of,
};
use crate::engine::l5::full_summary::FullRoutineSummary;

/// `C1_CONE_PARITY=1` — value-tested once per process.
fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("C1_CONE_PARITY").as_deref() == Ok("1"))
}

/// Run [`assert_cone_parity`] when `C1_CONE_PARITY=1`; otherwise a no-op.
pub fn assert_cone_parity_if_enabled(
    summaries: &HashMap<String, FullRoutineSummary>,
    store: &ConeDerivedStore,
) {
    if enabled() {
        assert_cone_parity(summaries, store);
    }
}

/// Assert, for every routine, that the derived substrate reproduces the raw-Vec
/// computation exactly.
///
/// # Panics
/// Panics on the first divergence, naming the routine and the predicate.
pub fn assert_cone_parity(
    summaries: &HashMap<String, FullRoutineSummary>,
    store: &ConeDerivedStore,
) {
    // Deterministic order so a divergence reproduces identically run-to-run.
    let mut ids: Vec<&str> = summaries.keys().map(|s| s.as_str()).collect();
    ids.sort_unstable();

    for id in &ids {
        let s = &summaries[*id];

        // --- tri-states (incl. the coverage-driven No-vs-Unknown arm) --------
        check(
            id,
            "touches_db_of",
            touches_db_of(s),
            touches_db_derived(store, s),
        );
        check(
            id,
            "may_commit",
            may_commit(s),
            may_commit_derived(store, s),
        );
        // Coverage is untouched by C1 — pinned so a future task cannot silently
        // re-route the tri-state's second arm through a different record.
        check(
            id,
            "reachable_coverage",
            reachable_coverage(s, None),
            s.inherited_status(),
        );

        // --- id-sets (exact Vec<String>, order included) ---------------------
        check(
            id,
            "writes_tables_of",
            writes_tables_of(s),
            store.writes_tables_of(id),
        );
        check(
            id,
            "writes_physical_tables_of",
            writes_physical_tables_of(s),
            store.writes_physical_tables_of(id),
        );
        check(
            id,
            "publishes_events_of",
            publishes_events_of(s),
            store.publishes_events_of(id),
        );

        // --- d44's own view ---------------------------------------------------
        check(
            id,
            "d44::write_ops",
            d44_raw_write_ops(s),
            store
                .physical_table_write_ops_of(id)
                .into_iter()
                .map(|(t, ops)| (t, ops.into_iter().collect::<BTreeSet<&str>>()))
                .collect::<BTreeMap<String, BTreeSet<&str>>>(),
        );
        check(
            id,
            "d44::reads",
            d44_raw_reads(s),
            store
                .physical_table_reads_of(id)
                .into_iter()
                .collect::<BTreeSet<String>>(),
        );

        // --- d48's pruning bool ------------------------------------------------
        check(
            id,
            "d48::routine_touches_external_io",
            raw_touches_external_io(s),
            store.touches_io(id),
        );
    }

    eprintln!(
        "[C1_CONE_PARITY] OK — {} routines verified against the raw cone",
        ids.len()
    );
}

fn check<T: PartialEq + std::fmt::Debug>(routine_id: &str, predicate: &str, raw: T, derived: T) {
    assert!(
        raw == derived,
        "C1 cone parity FAILED\n  routine:   {routine_id}\n  predicate: {predicate}\n  raw:       {raw:?}\n  derived:   {derived:?}"
    );
}

// ---------------------------------------------------------------------------
// Raw-side replicas of the detector-local closures (verbatim — see citations).
// ---------------------------------------------------------------------------

/// d44's `WRITE_OPS = {insert, modify, delete}` (`d44.rs`'s `is_write_op`).
fn d44_is_write_op(op: &str) -> bool {
    matches!(op, "insert" | "modify" | "delete")
}

/// d44's write view: its `find_capabilities` closure, grouped exactly as its
/// `op_union: BTreeSet<&str>` per `(event, table)` group.
fn d44_raw_write_ops(s: &FullRoutineSummary) -> BTreeMap<String, BTreeSet<&str>> {
    let mut out: BTreeMap<String, BTreeSet<&str>> = BTreeMap::new();
    for f in s.reachable_iter().filter(|f: &&CapabilityFact| {
        f.resource_kind == "table"
            && d44_is_write_op(&f.op)
            && f.resource_id.is_some()
            && !fact_is_known_temp(f)
    }) {
        out.entry(f.resource_id.clone().unwrap())
            .or_default()
            .insert(f.op.as_str());
    }
    out
}

/// d44's read set: its second `find_capabilities` closure.
fn d44_raw_reads(s: &FullRoutineSummary) -> BTreeSet<String> {
    s.reachable_iter()
        .filter(|f| {
            f.resource_kind == "table"
                && f.op == "read"
                && f.resource_id.is_some()
                && !fact_is_known_temp(f)
        })
        .map(|f| f.resource_id.clone().unwrap())
        .collect()
}

/// d48's `routine_touches_external_io` over `direct ∪ inherited`, with its own
/// `is_io_resource_kind` (`{http, file}`).
fn raw_touches_external_io(s: &FullRoutineSummary) -> bool {
    s.capability_facts_direct
        .iter()
        .chain(s.capability_facts_inherited.iter())
        .any(|f| f.resource_kind == "http" || f.resource_kind == "file")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::l3::l3_workspace::assemble_and_resolve_default;
    use crate::engine::l5::detector_context::build_detector_context;
    use crate::engine::l5::registry::substrate;

    /// Two page actions each declaring `trigger OnAction()` COLLIDE on one
    /// internal routine id (`compute_routine_id` carries no member
    /// discriminator — gap G-18). `build_detector_context` assembles summaries
    /// by `remove()`-ing each routine's cone entry, so the second occurrence
    /// gets nothing and the summary that SURVIVES is fully degenerate.
    ///
    /// This pins the derived substrate to that reality UNCONDITIONALLY (the
    /// `C1_CONE_PARITY` oracle is opt-in; this test is not): the degenerate
    /// summary must come with an EMPTY derived row, or Task 3 — where detectors
    /// read the row instead of the Vec — would silently change output for every
    /// colliding trigger in a real BC workspace.
    #[test]
    fn colliding_routine_ids_leave_summary_and_derived_row_equally_degenerate() {
        let src = r#"
table 50811 "CP Setup"
{
    fields { field(1; "No."; Code[20]) { } }
    keys { key(PK; "No.") { } }
}

page 50811 "CP Wizard"
{
    PageType = Card;

    actions
    {
        area(Processing)
        {
            action(First)
            {
                trigger OnAction()
                begin
                    Touch();
                end;
            }
            action(Second)
            {
                trigger OnAction()
                begin
                    Touch();
                end;
            }
        }
    }

    local procedure Touch()
    var
        Setup: Record "CP Setup";
    begin
        Setup.Insert();
    end;
}
"#;
        let files = vec![("src/CPWizard.al".to_string(), src.to_string())];
        let resolved = assemble_and_resolve_default(&files, "11111111-0000-0000-0000-0000000cp001");
        let ctx = build_detector_context(&resolved, substrate::SUMMARIES);

        // The collision is real: two routines share one id, so the routine list
        // is longer than the summaries map.
        let ids: BTreeSet<&str> = resolved
            .workspace
            .routines
            .iter()
            .map(|r| r.id.as_str())
            .collect();
        assert!(
            ids.len() < resolved.workspace.routines.len(),
            "fixture precondition: the two OnAction triggers must collide on one routine id"
        );

        // The colliding id's summary is degenerate — and its derived row matches.
        let degenerate: Vec<&String> = ctx
            .summaries
            .iter()
            .filter(|(_, s)| {
                s.capability_facts_direct.is_empty()
                    && s.capability_facts_inherited.is_empty()
                    && s.coverage.is_none()
            })
            .map(|(id, _)| id)
            .collect();
        assert!(
            !degenerate.is_empty(),
            "fixture precondition: the collision must produce a degenerate summary"
        );
        for id in &degenerate {
            assert_eq!(
                ctx.cone_derived.flags_of(id),
                0,
                "{id}: a degenerate summary must carry an empty derived row"
            );
            assert!(ctx.cone_derived.writes_tables_of(id).is_empty());
        }

        // And the full oracle holds for EVERY routine, flag or no flag.
        assert_cone_parity(&ctx.summaries, &ctx.cone_derived);
    }
}
