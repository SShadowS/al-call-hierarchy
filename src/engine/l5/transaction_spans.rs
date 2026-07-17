//! Transaction spans — faithful port of al-sem
//! `src/engine/transaction-spans.ts`.
//!
//! `compute_transaction_spans` produces a `TransactionSpan` per Commit (and per
//! checked Codeunit.Run implicit commit). For each seed it walks callers BACKWARD
//! over the reverse call graph to find every routine that participates in the
//! transaction; the walk stops at any routine that itself commits (a prior span's
//! domain) or at `MAX_DEPTH`. The span aggregates writes/events (union over the
//! span via the capability-query helpers) and a `coverage_complete` flag.
//!
//! ## Role threading
//!
//! al-sem reads `roleOf(r)` to restrict seeding + aggregation to PRIMARY
//! routines. The Rust model has no `roleOf`; role is `is_dep =
//! dep_routine_ids.contains(&r.id)` (empty set ⇒ all primary). We thread the role
//! oracle as `&BTreeSet<String>` (see `entry_points` for the same convention).
//!
//! ## Summaries
//!
//! al-sem reads `routine.summary` (each routine carries its own). The Rust model
//! keeps facts/coverage SEPARATE, so the caller passes a
//! `&HashMap<RoutineId, FullRoutineSummary>` (internal id → summary). A routine
//! with NO entry behaves like al-sem's `r.summary === undefined`:
//! `coverage_complete ← false` and it contributes no tables/events. This mirrors
//! `transaction-spans.ts` lines 100-108 / 155-163 exactly.
//!
//! Determinism: `visited` is a `BTreeSet` (so the per-seed walk's collected set
//! is order-independent); `writes`/`publishes` are `BTreeSet`s; every output Vec
//! is sorted. The seed iteration order over routines follows the input slice
//! order (al-sem iterates `model.routines` for the §B pass and a `Map` keyed by
//! insertion order for the explicit-commit pass — both yield spans in a stable
//! order which downstream Task 2b sorts by the span's own key).

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};

use crate::engine::l2::features::PCallee;
use crate::engine::l3::l3_workspace::L3Routine;
use crate::engine::l5::capability_query::{
    publishes_events_of, reachable_coverage, writes_tables_of,
};
use crate::engine::l5::full_summary::FullRoutineSummary;
use crate::engine::l5::reverse_call_graph::ReverseCallGraph;

const MAX_DEPTH: usize = 50;

/// Distinguishes an explicit `Commit()` span from a synthetic checked-Run
/// implicit commit. Mirrors al-sem `seedKind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeedKind {
    ExplicitCommit,
    CheckedRunImplicit,
}

/// A transaction span (al-sem `TransactionSpan`). All id lists SORTED.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransactionSpan {
    pub seed_kind: SeedKind,
    /// The Commit operation that bounds the span (for checked-run-implicit, the
    /// checked callsite id — al-sem stores the callsite id here too).
    pub commit_operation_id: String,
    /// For checked-run-implicit seeds only: the callsite id of the checked
    /// Codeunit.Run forming the implicit-commit boundary. `None` for explicit.
    pub seed_callsite_id: Option<String>,
    /// The routine containing the bounding Commit.
    pub commit_routine_id: String,
    /// All routines reachable backward from `commit_routine_id` up to another
    /// commit or root. SORTED.
    pub routines_in_span: Vec<String>,
    /// Union of tables written by any routine in the span. SORTED + deduped.
    pub writes_tables: Vec<String>,
    /// Union of events published by any routine in the span. SORTED + deduped.
    pub publishes_events: Vec<String>,
    /// Span entry roots — routines in the span with no upstream caller. SORTED.
    pub span_roots: Vec<String>,
    /// True iff EVERY routine in `routines_in_span` has a defined summary AND
    /// `reachable_coverage(summary) == "complete"`.
    pub coverage_complete: bool,
}

/// `roleOf(r) === "primary"` — true when NOT in the dependency universe.
fn is_primary(routine: &L3Routine, dep_routine_ids: &BTreeSet<String>) -> bool {
    !dep_routine_ids.contains(&routine.id)
}

/// Backward BFS over the reverse graph from `seed`, stopping at another
/// committing routine (other than the seed) and at `MAX_DEPTH`. Returns the
/// visited set (a `BTreeSet` for order-independence). Mirrors the inner while-loop
/// in both al-sem seed passes verbatim.
fn backward_cone(
    seed: &str,
    commits_by_routine: &BTreeMap<String, Vec<String>>,
    reverse: &ReverseCallGraph,
) -> BTreeSet<String> {
    let mut visited: BTreeSet<String> = BTreeSet::new();
    let mut queue: VecDeque<(String, usize)> = VecDeque::new();
    queue.push_back((seed.to_string(), 0));
    while let Some((id, depth)) = queue.pop_front() {
        if visited.contains(&id) {
            continue;
        }
        visited.insert(id.clone());
        if depth >= MAX_DEPTH {
            continue;
        }
        // Don't walk past another committing routine (prior span bounds the trace).
        if id != seed && commits_by_routine.contains_key(&id) {
            continue;
        }
        if let Some(callers) = reverse.get(&id) {
            for caller in callers {
                if !visited.contains(&caller.from) {
                    queue.push_back((caller.from.clone(), depth + 1));
                }
            }
        }
    }
    visited
}

/// Aggregate the writes/events/coverage over a visited span. Mirrors al-sem
/// lines 97-109 / 152-164: a routine with no summary → `coverage_complete` false
/// and contributes nothing; otherwise union its `writes_tables_of` /
/// `publishes_events_of` and AND-in its `reachable_coverage == "complete"`.
fn aggregate_span(
    visited: &BTreeSet<String>,
    summaries: &HashMap<String, FullRoutineSummary>,
) -> (Vec<String>, Vec<String>, bool) {
    let mut writes: BTreeSet<String> = BTreeSet::new();
    let mut events: BTreeSet<String> = BTreeSet::new();
    let mut coverage_complete = true;
    for rid in visited {
        let Some(summary) = summaries.get(rid) else {
            coverage_complete = false;
            continue;
        };
        for t in writes_tables_of(summary) {
            writes.insert(t);
        }
        for e in publishes_events_of(summary) {
            events.insert(e);
        }
        if reachable_coverage(summary, None) != "complete" {
            coverage_complete = false;
        }
    }
    (
        writes.into_iter().collect(),
        events.into_iter().collect(),
        coverage_complete,
    )
}

/// span roots = visited routines with no reverse callers. Mirrors al-sem
/// `[...visited].filter((rid) => (reverse.get(rid) ?? []).length === 0)`.
fn span_roots_of(visited: &BTreeSet<String>, reverse: &ReverseCallGraph) -> Vec<String> {
    let mut roots: Vec<String> = visited
        .iter()
        .filter(|rid| reverse.get(*rid).map(|v| v.is_empty()).unwrap_or(true))
        .cloned()
        .collect();
    roots.sort();
    roots
}

/// Compute transaction spans. For each primary-app routine that contains a Commit
/// (and each checked Codeunit.Run implicit commit), walk callers backward to find
/// every routine that participates in the transaction. Each Commit operation
/// yields one `TransactionSpan`. Mirrors al-sem `computeTransactionSpans`.
///
/// `routines` — the model routines (al-sem `model.routines`).
/// `dep_routine_ids` — the role oracle (empty ⇒ all primary).
/// `reverse` — the reverse call graph (`build_reverse_call_graph`).
/// `summaries` — internal RoutineId → its `FullRoutineSummary`; a routine with no
/// entry behaves like al-sem's `summary === undefined`.
pub fn compute_transaction_spans(
    routines: &[L3Routine],
    dep_routine_ids: &BTreeSet<String>,
    reverse: &ReverseCallGraph,
    summaries: &HashMap<String, FullRoutineSummary>,
) -> Vec<TransactionSpan> {
    let mut spans: Vec<TransactionSpan> = Vec::new();

    // routineId → its Commit operationIds (operationSites with kind == "commit"),
    // PRIMARY routines only. A BTreeMap so the explicit-commit seed iteration is
    // deterministic (al-sem's Map is insertion-ordered over model.routines; we
    // key-sort, which Task 2b re-sorts by span key anyway).
    let mut commits_by_routine: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for r in routines {
        if !is_primary(r, dep_routine_ids) {
            continue;
        }
        let commit_ops: Vec<String> = r
            .operation_sites
            .iter()
            .filter(|os| os.kind == "commit")
            .map(|os| os.id.clone())
            .collect();
        if !commit_ops.is_empty() {
            commits_by_routine.insert(r.id.clone(), commit_ops);
        }
    }

    // --- explicit-commit seeds ---
    for (commit_routine_id, commit_ops) in &commits_by_routine {
        for commit_operation_id in commit_ops {
            let __probe_t = std::time::Instant::now();
            let visited = backward_cone(commit_routine_id, &commits_by_routine, reverse);
            crate::stage_probe::accum(crate::stage_probe::ACC_SPAN_BFS, __probe_t.elapsed());
            let (writes_tables, publishes_events, coverage_complete) =
                aggregate_span(&visited, summaries);
            let span_roots = span_roots_of(&visited, reverse);
            spans.push(TransactionSpan {
                seed_kind: SeedKind::ExplicitCommit,
                commit_operation_id: commit_operation_id.clone(),
                seed_callsite_id: None,
                commit_routine_id: commit_routine_id.clone(),
                routines_in_span: visited.iter().cloned().collect(),
                writes_tables,
                publishes_events,
                span_roots,
                coverage_complete,
            });
        }
    }

    // --- §B: synthetic seeds for CHECKED codeunit-run implicit commits ---
    // Codeunit.Run only; objectRunReturnUsed === true only (the STRICT affirmative
    // predicate). NOT Page.Run / Report.Run.
    for r in routines {
        if !is_primary(r, dep_routine_ids) {
            continue;
        }
        for cs in &r.call_sites {
            let PCallee::ObjectRun { object_kind, .. } = &cs.callee else {
                continue;
            };
            if object_kind != "Codeunit" {
                continue;
            }
            if cs.object_run_return_used != Some(true) {
                continue;
            }
            let visited = backward_cone(&r.id, &commits_by_routine, reverse);
            let (writes_tables, publishes_events, coverage_complete) =
                aggregate_span(&visited, summaries);
            let span_roots = span_roots_of(&visited, reverse);
            // commitOperationId uses the callsite id (same opaque-string type at
            // runtime); seed_callsite_id provides the typed accessor.
            spans.push(TransactionSpan {
                seed_kind: SeedKind::CheckedRunImplicit,
                commit_operation_id: cs.id.clone(),
                seed_callsite_id: Some(cs.id.clone()),
                commit_routine_id: r.id.clone(),
                routines_in_span: visited.iter().cloned().collect(),
                writes_tables,
                publishes_events,
                span_roots,
                coverage_complete,
            });
        }
    }

    spans
}

// ===========================================================================
// Native oracles — ground-truth-free invariants on synthetic inputs.
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::l5::reverse_call_graph::build_reverse_call_graph;
    use crate::engine::l5::test_support::{
        coverage, edge, fact, graph_from_edges, object_run_call_site, op_commit_routine, routine,
        summary,
    };

    #[test]
    fn explicit_commit_span_is_backward_cone_with_union_and_roots() {
        // Call chain: root → mid → committer.  committer holds Commit (op id "c/op").
        // Backward cone from committer = {committer, mid, root}.
        let routines = vec![
            routine("root", "trigger"),
            routine("mid", "procedure"),
            op_commit_routine("committer", "procedure", &["c/op"]),
        ];
        let graph = graph_from_edges(
            &["root", "mid", "committer"],
            &[edge("root", "mid", "cs1"), edge("mid", "committer", "cs2")],
        );
        let reverse = build_reverse_call_graph(&graph);

        let mut summaries: HashMap<String, FullRoutineSummary> = HashMap::new();
        summaries.insert(
            "committer".to_string(),
            summary(
                "committer",
                vec![fact("insert", "table", Some("t/A"))],
                vec![],
                Some(coverage("complete")),
            ),
        );
        summaries.insert(
            "mid".to_string(),
            summary(
                "mid",
                vec![fact("publish", "event", Some("e/E"))],
                vec![],
                Some(coverage("complete")),
            ),
        );
        summaries.insert(
            "root".to_string(),
            summary("root", vec![], vec![], Some(coverage("complete"))),
        );

        let no_deps = BTreeSet::new();
        let spans = compute_transaction_spans(&routines, &no_deps, &reverse, &summaries);
        assert_eq!(spans.len(), 1);
        let span = &spans[0];
        assert_eq!(span.seed_kind, SeedKind::ExplicitCommit);
        assert_eq!(span.commit_operation_id, "c/op");
        assert_eq!(span.commit_routine_id, "committer");
        assert_eq!(span.routines_in_span, vec!["committer", "mid", "root"]);
        assert_eq!(span.writes_tables, vec!["t/A"]);
        assert_eq!(span.publishes_events, vec!["e/E"]);
        // Only `root` has no reverse callers.
        assert_eq!(span.span_roots, vec!["root"]);
        assert!(span.coverage_complete);
    }

    #[test]
    fn walk_stops_at_another_committing_routine() {
        // outer (commits) → inner (commits). The span seeded at `inner` must NOT
        // include `outer` (the walk stops when it reaches another committer).
        let routines = vec![
            op_commit_routine("outer", "procedure", &["outer/op"]),
            op_commit_routine("inner", "procedure", &["inner/op"]),
        ];
        let graph = graph_from_edges(&["outer", "inner"], &[edge("outer", "inner", "cs1")]);
        let reverse = build_reverse_call_graph(&graph);
        let summaries: HashMap<String, FullRoutineSummary> = HashMap::new();
        let no_deps = BTreeSet::new();
        let spans = compute_transaction_spans(&routines, &no_deps, &reverse, &summaries);

        let inner_span = spans
            .iter()
            .find(|s| s.commit_routine_id == "inner")
            .unwrap();
        // backward_cone from inner: visits inner, then its caller outer — but outer
        // is enqueued and on dequeue it's a committer != seed, so it is visited
        // (added) yet its callers are not walked. al-sem adds outer to `visited`.
        // The stop-at-committer rule prevents walking PAST outer, but outer itself
        // is in the cone. Assert that the prior committer bounds the trace: nothing
        // upstream of outer is pulled in.
        assert!(inner_span.routines_in_span.contains(&"inner".to_string()));
        assert!(inner_span.routines_in_span.contains(&"outer".to_string()));

        let outer_span = spans
            .iter()
            .find(|s| s.commit_routine_id == "outer")
            .unwrap();
        // outer's cone: outer only (no callers). inner is DOWNSTREAM (forward), not
        // reached by a backward walk.
        assert_eq!(outer_span.routines_in_span, vec!["outer"]);
    }

    #[test]
    fn walk_does_not_pull_in_callers_past_the_boundary_committer() {
        // grandparent (NO commit) → outer (commits) → inner (commits).
        // Seeded at `inner`, the backward walk reaches `outer`, includes it (the
        // boundary committer is in the cone), but MUST NOT walk past it to
        // `grandparent`. This is the test that actually exercises the
        // `id != seed && commits_by_routine.contains_key(id)` stop guard — the
        // outer→inner-only fixture above cannot, since `outer` has no callers.
        let routines = vec![
            op_commit_routine("grandparent", "procedure", &[]), // no commit ⇒ not a committer
            op_commit_routine("outer", "procedure", &["outer/op"]),
            op_commit_routine("inner", "procedure", &["inner/op"]),
        ];
        let graph = graph_from_edges(
            &["grandparent", "inner", "outer"],
            &[
                edge("grandparent", "outer", "cs1"),
                edge("outer", "inner", "cs2"),
            ],
        );
        let reverse = build_reverse_call_graph(&graph);
        let summaries: HashMap<String, FullRoutineSummary> = HashMap::new();
        let no_deps = BTreeSet::new();
        let spans = compute_transaction_spans(&routines, &no_deps, &reverse, &summaries);

        let inner_span = spans
            .iter()
            .find(|s| s.commit_routine_id == "inner")
            .unwrap();
        // Exactly {inner, outer} — `grandparent` (upstream of the boundary
        // committer `outer`) is excluded. Deleting the stop guard would pull
        // `grandparent` in and fail this assertion.
        assert_eq!(inner_span.routines_in_span, vec!["inner", "outer"]);
    }

    #[test]
    fn missing_summary_makes_coverage_incomplete_and_contributes_nothing() {
        let routines = vec![op_commit_routine("c", "procedure", &["c/op"])];
        let graph = graph_from_edges(&["c"], &[]);
        let reverse = build_reverse_call_graph(&graph);
        // No summary entry for "c" → coverage_complete false, no writes/events.
        let summaries: HashMap<String, FullRoutineSummary> = HashMap::new();
        let no_deps = BTreeSet::new();
        let spans = compute_transaction_spans(&routines, &no_deps, &reverse, &summaries);
        assert_eq!(spans.len(), 1);
        assert!(!spans[0].coverage_complete);
        assert!(spans[0].writes_tables.is_empty());
        // c has no callers → it is its own span root.
        assert_eq!(spans[0].span_roots, vec!["c"]);
    }

    #[test]
    fn partial_coverage_makes_span_incomplete() {
        let routines = vec![op_commit_routine("c", "procedure", &["c/op"])];
        let graph = graph_from_edges(&["c"], &[]);
        let reverse = build_reverse_call_graph(&graph);
        let mut summaries: HashMap<String, FullRoutineSummary> = HashMap::new();
        summaries.insert(
            "c".to_string(),
            summary("c", vec![], vec![], Some(coverage("partial"))),
        );
        let no_deps = BTreeSet::new();
        let spans = compute_transaction_spans(&routines, &no_deps, &reverse, &summaries);
        assert!(!spans[0].coverage_complete);
    }

    #[test]
    fn checked_codeunit_run_yields_implicit_seed() {
        // A routine with a checked Codeunit.Run (objectRunReturnUsed = Some(true)).
        let mut caller = routine("caller", "procedure");
        caller
            .call_sites
            .push(object_run_call_site("caller/cs0", "Codeunit", Some(true)));
        let routines = vec![caller];
        let graph = graph_from_edges(&["caller"], &[]);
        let reverse = build_reverse_call_graph(&graph);
        let summaries: HashMap<String, FullRoutineSummary> = HashMap::new();
        let no_deps = BTreeSet::new();
        let spans = compute_transaction_spans(&routines, &no_deps, &reverse, &summaries);
        assert_eq!(spans.len(), 1);
        let span = &spans[0];
        assert_eq!(span.seed_kind, SeedKind::CheckedRunImplicit);
        assert_eq!(span.seed_callsite_id.as_deref(), Some("caller/cs0"));
        assert_eq!(span.commit_operation_id, "caller/cs0");
        assert_eq!(span.commit_routine_id, "caller");
    }

    #[test]
    fn unchecked_run_and_non_codeunit_run_do_not_seed() {
        let mut r = routine("r", "procedure");
        // unchecked Codeunit.Run (return not used) → no seed
        r.call_sites
            .push(object_run_call_site("r/cs0", "Codeunit", Some(false)));
        // checked Page.Run → no seed (only Codeunit has the implicit-commit rule)
        r.call_sites
            .push(object_run_call_site("r/cs1", "Page", Some(true)));
        let routines = vec![r];
        let graph = graph_from_edges(&["r"], &[]);
        let reverse = build_reverse_call_graph(&graph);
        let summaries: HashMap<String, FullRoutineSummary> = HashMap::new();
        let no_deps = BTreeSet::new();
        let spans = compute_transaction_spans(&routines, &no_deps, &reverse, &summaries);
        assert!(spans.is_empty());
    }

    #[test]
    fn dep_routines_do_not_seed() {
        let routines = vec![op_commit_routine("c", "procedure", &["c/op"])];
        let graph = graph_from_edges(&["c"], &[]);
        let reverse = build_reverse_call_graph(&graph);
        let summaries: HashMap<String, FullRoutineSummary> = HashMap::new();
        let deps: BTreeSet<String> = ["c".to_string()].into_iter().collect();
        let spans = compute_transaction_spans(&routines, &deps, &reverse, &summaries);
        assert!(spans.is_empty());
    }
}
