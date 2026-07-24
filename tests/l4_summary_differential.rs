//! Task 1 (l4-summary-fixpoint-redesign, Phase 1): the differential harness
//! that pins Phase 1's STRICT-PARITY constraint — every future `db_effect_solver`
//! change must produce a byte-identical complete `RoutineSummary` (incl.
//! `db_effects`/`uncertainties`/`has_unresolved_calls`/`parameter_roles`) to the
//! existing JACOBI solver (`compute_summaries_with_leaves`), per routine.
//!
//! `compute_summaries_v2_with_leaves` (the new-solver seam, added to
//! `src/engine/l4/summary_runner.rs`) currently DELEGATES to the old solver —
//! this task is scaffolding, so every fixture below is trivially green. Tasks
//! 2-8 replace v2's internals (db_effects / uncertainty / has_unresolved_calls
//! computation) while keeping `parameter_roles` sourced from the old
//! `compose_routine`; these ten fixtures are the acceptance set every later
//! task must keep green (see the plan's Phase 1 task list).

use std::collections::HashMap;

use al_call_hierarchy::engine::l4::summary::RoutineSummary;
use al_call_hierarchy::engine::l4::summary_runner::{
    FieldIndex, compute_summaries_v2_with_leaves_core, compute_summaries_with_leaves,
};

// Task 9 (l4-summary-fixpoint-redesign): the CDO_WS/ENFORCE_CDO_WS gating helper
// lives in `tests/common/cdo.rs` (shared with `program_resolve_harness.rs` and
// siblings) — separate test-binary crates can't `use` each other's `mod`s, so
// every CDO-gated test file includes it via `#[path]`. See that file's doc
// comment for the skip/panic contract.
#[path = "common/cdo.rs"]
mod cdo;
use cdo::cdo_ws_or_enforce;

/// Compare v2 against the old Jacobi solver over the COMPLETE `RoutineSummary`,
/// with no fixed leaves.
fn assert_parity(name: &str) {
    assert_parity_with_leaves(name, &HashMap::new());
}

/// Like [`assert_parity`], but with an explicit `leaf_summaries` map — needed by
/// `fixed_leaf_in_scc`, whose whole point is a routine that arrives as a
/// pre-settled leaf rather than being recomputed.
fn assert_parity_with_leaves(name: &str, leaves: &HashMap<String, RoutineSummary>) {
    let (routines, graph, scc, fields, ub) = fixtures::build(name);
    let (old, _t, _d) =
        compute_summaries_with_leaves(&routines, &graph, &scc, &ub, &fields, false, leaves);
    // The differential compares the COMPLETE RoutineSummary (map); the `_core`
    // fn also returns the roles cap-hit diagnostics, which are irrelevant here
    // (and empty on every fixture — roles converge).
    let (new, _diags) =
        compute_summaries_v2_with_leaves_core(&routines, &graph, &scc, &ub, &fields, leaves);
    assert_eq!(old.len(), new.len(), "[{name}] routine count");
    for (id, old_s) in &old {
        let new_s = new
            .get(id)
            .unwrap_or_else(|| panic!("[{name}] missing {id}"));
        assert_eq!(old_s, new_s, "[{name}] summary mismatch for {id}");
    }
}

#[test]
fn parity_linear_two_routine_known_effect() {
    assert_parity("linear_known");
}

#[test]
fn parity_recursive_self_loop() {
    assert_parity("recursive_self_loop");
}

#[test]
fn parity_recursive_pair_pd() {
    assert_parity("recursive_pair_pd");
}

#[test]
fn parity_pd_to_known() {
    assert_parity("pd_to_known");
}

#[test]
fn parity_pd_to_unknown() {
    assert_parity("pd_to_unknown");
}

#[test]
fn parity_multi_callsite_same_callee() {
    assert_parity("multi_callsite_same_callee");
}

#[test]
fn parity_via_collision() {
    assert_parity("via_collision");
}

// ---------------------------------------------------------------------------
// Task A2 via-rank-merge guards (spec Step 3 test list) — see the fixture
// doc comments in `mod fixtures` for what each one exercises and why the
// differential (byte-identical to the untouched OLD solver) is the oracle.
// ---------------------------------------------------------------------------

#[test]
fn parity_via_collision_edges_reversed() {
    assert_parity("via_collision_edges_reversed");
}

#[test]
fn parity_pd_substituted_via_collision() {
    assert_parity("pd_substituted_via_collision");
}

#[test]
fn parity_pd_substituted_via_collision_reversed() {
    assert_parity("pd_substituted_via_collision_reversed");
}

#[test]
fn parity_direct_terminal_beats_colliding_pd_substitution() {
    assert_parity("direct_terminal_beats_colliding_pd_substitution");
}

#[test]
fn parity_direct_pd_base_beats_colliding_pd_substitution_and_dedup_transition() {
    assert_parity("direct_pd_base_beats_colliding_pd_substitution_and_dedup_transition");
}

#[test]
fn parity_external_successor_pd() {
    assert_parity("external_successor_pd");
}

#[test]
fn parity_fixed_leaf_in_scc() {
    let leaves = fixtures::fixed_leaf_in_scc_leaves();
    assert_parity_with_leaves("fixed_leaf_in_scc", &leaves);
}

#[test]
fn parity_missing_routine_in_scc() {
    assert_parity("missing_routine_in_scc");
}

// ---------------------------------------------------------------------------
// Task 9: the real-corpus whole-program v2 parity gate.
// ---------------------------------------------------------------------------

/// Returns a short, focused description of the FIRST field at which two
/// `RoutineSummary`s diverge — used by [`cdo_whole_program_v2_parity`]'s
/// mismatch diagnostic instead of dumping both complete summaries raw (a real
/// workspace's `db_effects`/`uncertainties` lists can be long; this walks each
/// list element-by-element and reports only the first differing entry, or a
/// length mismatch, whichever comes first).
fn first_diff_field(old: &RoutineSummary, new: &RoutineSummary) -> String {
    if old.db_effects.len() != new.db_effects.len() {
        return format!(
            "db_effects length differs: old={} new={}",
            old.db_effects.len(),
            new.db_effects.len()
        );
    }
    for (i, (o, n)) in old.db_effects.iter().zip(new.db_effects.iter()).enumerate() {
        if o != n {
            return format!("db_effects[{i}] differs: old={o:?} new={n:?}");
        }
    }
    if old.in_recursive_cycle != new.in_recursive_cycle {
        return format!(
            "in_recursive_cycle differs: old={} new={}",
            old.in_recursive_cycle, new.in_recursive_cycle
        );
    }
    if old.has_unresolved_calls != new.has_unresolved_calls {
        return format!(
            "has_unresolved_calls differs: old={} new={}",
            old.has_unresolved_calls, new.has_unresolved_calls
        );
    }
    if old.uncertainties.len() != new.uncertainties.len() {
        return format!(
            "uncertainties length differs: old={} new={}",
            old.uncertainties.len(),
            new.uncertainties.len()
        );
    }
    for (i, (o, n)) in old
        .uncertainties
        .iter()
        .zip(new.uncertainties.iter())
        .enumerate()
    {
        if o != n {
            return format!("uncertainties[{i}] differs: old={o:?} new={n:?}");
        }
    }
    if old.parameter_roles.len() != new.parameter_roles.len() {
        return format!(
            "parameter_roles length differs: old={} new={}",
            old.parameter_roles.len(),
            new.parameter_roles.len()
        );
    }
    for (i, (o, n)) in old
        .parameter_roles
        .iter()
        .zip(new.parameter_roles.iter())
        .enumerate()
    {
        if o != n {
            return format!("parameter_roles[{i}] differs: old={o:?} new={n:?}");
        }
    }
    "RoutineSummary != by derived PartialEq but no per-field comparison above caught it \
     (should be unreachable — report as an engine bug)"
        .to_string()
}

/// The real-workspace parity gate (Task 9, l4-summary-fixpoint-redesign Phase 1
/// capstone): assembles a REAL Business Central workspace (`CDO_WS`) into the
/// exact `(routines, graph, scc, upgraded_bindings, fields, leaf_summaries)`
/// tuple both `compute_summaries_with_leaves` (old JACOBI) and
/// `compute_summaries_v2_with_leaves` (new closed-form solver) take, runs BOTH
/// over the SAME inputs, and asserts complete-`RoutineSummary` parity — incl.
/// `db_effects` (and its `record_variable_id`), `uncertainties`,
/// `has_unresolved_calls`, `parameter_roles`, `in_recursive_cycle` — for EVERY
/// routine in the workspace.
///
/// This is the ONLY place Phase 1's synthetic-fixture parity (all ten fixtures
/// in this file use `record_variable_id: None`) gets checked at real scale: a
/// real workspace's `L3RecordOperation.record_variable_id` is frequently
/// `Some(..)`, so this is where a `record_variable_id` divergence between the
/// two solvers would actually surface.
///
/// ## Assembly
///
/// The six inputs are built by replaying — in the SAME order, via the SAME
/// public functions — exactly the sequence `build_detector_context`'s
/// `CORE_SUMMARIES` substrate uses (`src/engine/l5/detector_context.rs`,
/// roughly lines 236-517): `SymbolTable::build` → `resolve_calls` (no deps, no
/// fetched apps — matches the source-only detector-context path) →
/// `build_event_graph` → `build_combined_graph` → a Tarjan SCC over
/// `graph.edges_by_from` → the `(tableId, lowercased field name) -> fieldId`
/// `FieldIndex`. `leaf_summaries` is the empty map, exactly like
/// `build_detector_context`'s own `compute_summaries(...)` call (which always
/// passes `&no_leaves` — see `summary_runner::compute_summaries`). This makes
/// the graph/SCC/bindings/fields fed to both solvers here BYTE-IDENTICAL to
/// what a real `alsem`/`aldump` run feeds the production `compute_summaries`
/// call — not a hand-built graph — while still calling the two solver entry
/// points directly (so this test, not `DetectorContext`, owns the comparison).
///
/// ## record_variable_id out-of-contract proof
///
/// `PDbEffect` (`src/engine/l4/summary.rs:211`, the serde-projected `DbEffect`)
/// OMITS `record_variable_id` — it is carried on the internal `DbEffect` (used
/// by `RoutineSummary`/this differential) but never reaches the projected
/// surface any consumer serializes. A repo-wide grep for readers —
/// ```text
/// rg -n "record_variable_id" src/engine/l5 src/engine/l4 | rg -v "None|test_support|: Option<String>"
/// ```
/// — turns up ONLY: (a) `DbEffect`/`RoutineSummary` CONSTRUCTOR sites in
/// `summary_runner.rs` (old solver) and `db_effect_solver.rs` (new solver) —
/// i.e. the two solvers this test already differentials against each other,
/// and (b) same-NAMED-but-different fields on unrelated structs
/// (`L3RecordOperation.record_variable_id` read by `capability_cone.rs` /
/// `cfg_walker.rs`, `PCallArgumentBinding.source_record_variable_id` read by
/// `d37.rs`/`d40.rs`, `CapabilityExtra::Table.record_variable_id` read by
/// `snapshot.rs` — all capability-cone-family fields, never
/// `RoutineSummary.db_effects[].record_variable_id`). Zero hits read the
/// summary's `DbEffect.record_variable_id` back out. The complete-`RoutineSummary`
/// `assert_eq!` below already guards the field regardless of this proof — this
/// doc comment just records WHY a hypothetical divergence there could not reach
/// any l5 consumer today.
///
/// Skips (no-op) when `CDO_WS` is unset; PANICS when `ENFORCE_CDO_WS=1` is set
/// alongside a missing/invalid `CDO_WS` (`tests/common/cdo.rs`). Run against a
/// real workspace with:
/// ```text
/// CDO_WS=<path> cargo test -p al-call-hierarchy --test l4_summary_differential cdo_ -- --nocapture
/// ```
#[test]
fn cdo_whole_program_v2_parity() {
    use al_call_hierarchy::engine::l3::call_resolver::{DeclaredDependency, resolve_calls};
    use al_call_hierarchy::engine::l3::event_graph::build_event_graph;
    use al_call_hierarchy::engine::l3::l3_workspace::assemble_and_resolve_workspace_default;
    use al_call_hierarchy::engine::l3::symbol_table::SymbolTable;
    use al_call_hierarchy::engine::l4::combined_graph::build_combined_graph;
    use al_call_hierarchy::engine::l4::scc::{SccInputGraph, tarjan_scc};

    let Some(ws_path) = cdo_ws_or_enforce() else {
        return;
    };

    let resolved = assemble_and_resolve_workspace_default(&ws_path)
        .expect("assemble_and_resolve_workspace_default must succeed on CDO_WS");
    let ws = &resolved.workspace;

    // --- Assemble the SAME (graph, scc, upgraded_bindings, fields) the
    // detector-context CORE_SUMMARIES path builds — see the doc comment above. ---
    let symbols = SymbolTable::build(&ws.objects, &ws.tables, &ws.routines);
    let no_deps: Vec<DeclaredDependency> = Vec::new();
    let no_fetched: Vec<String> = Vec::new();
    let calls = resolve_calls(ws, &symbols, &no_deps, &no_fetched);

    let event_graph = build_event_graph(&ws.routines, &symbols);
    let graph = build_combined_graph(ws, &calls, &event_graph);

    let mut scc_adjacency: HashMap<String, Vec<String>> = HashMap::new();
    for (from, list) in &graph.edges_by_from {
        scc_adjacency.insert(from.clone(), list.iter().map(|e| e.to.clone()).collect());
    }
    let scc = tarjan_scc(&SccInputGraph {
        nodes: &graph.nodes,
        edges_by_from: &scc_adjacency,
    });

    let mut field_index: FieldIndex = HashMap::new();
    for table in &ws.tables {
        for field in &table.fields {
            field_index
                .entry((table.id.clone(), field.name.to_lowercase()))
                .or_insert_with(|| field.id.clone());
        }
    }

    let leaf_summaries: HashMap<String, RoutineSummary> = HashMap::new();

    // --- Run BOTH solvers over the identical inputs. ---
    let (old, _trace, _cap_diagnostics) = compute_summaries_with_leaves(
        &ws.routines,
        &graph,
        &scc,
        &calls.upgraded_bindings,
        &field_index,
        false,
        &leaf_summaries,
    );
    let (new, _diags) = compute_summaries_v2_with_leaves_core(
        &ws.routines,
        &graph,
        &scc,
        &calls.upgraded_bindings,
        &field_index,
        &leaf_summaries,
    );

    assert_eq!(
        old.len(),
        new.len(),
        "cdo_whole_program_v2_parity: routine count mismatch (old={}, new={})",
        old.len(),
        new.len()
    );

    let mut mismatches: Vec<String> = Vec::new();
    for (id, old_s) in &old {
        match new.get(id) {
            None => mismatches.push(format!("{id}: missing from v2 output")),
            Some(new_s) if old_s != new_s => {
                mismatches.push(format!("{id}: {}", first_diff_field(old_s, new_s)));
            }
            Some(_) => {}
        }
    }

    assert!(
        mismatches.is_empty(),
        "cdo_whole_program_v2_parity: {} of {} routine(s) diverged between old and v2:\n{}",
        mismatches.len(),
        old.len(),
        mismatches.join("\n")
    );
}

/// Fixture builders for the differential harness above. Each named fixture
/// builds real `L3Routine` / `CombinedGraph` / `SccResult` / `FieldIndex` /
/// `upgraded_bindings` inputs the OLD solver can process without panicking; the
/// differential itself is the oracle, so these are structurally-valid inputs
/// exercising a specific solver code path rather than hand-verified outputs.
///
/// NOTE: `src/engine/l5/test_support.rs` has similar `L3Routine`/`CombinedGraph`
/// constructors, but that module is `#![cfg(test)]`-gated (unit-test-only) so it
/// is not linkable from this integration-test binary — every builder below is a
/// local, from-scratch copy of that same pattern (mirroring how
/// `tests/temp_state/temp_state_path.rs` and siblings already do this).
mod fixtures {
    use std::collections::HashMap;

    use al_call_hierarchy::engine::l2::features::{
        PAnchor, PCallArgumentBinding, PCallSite, PCallee, PTempState,
    };
    use al_call_hierarchy::engine::l3::call_resolver::UpgradedBinding;
    use al_call_hierarchy::engine::l3::l3_workspace::{L3RecordOperation, L3Routine};
    use al_call_hierarchy::engine::l4::combined_graph::{CombinedEdge, CombinedGraph};
    use al_call_hierarchy::engine::l4::effect_lattice::{TempStateKind, effect_key_of};
    use al_call_hierarchy::engine::l4::scc::{Scc, SccResult};
    use al_call_hierarchy::engine::l4::summary::{DbEffect, RoutineSummary, TempState};
    use al_call_hierarchy::engine::l4::summary_runner::FieldIndex;

    type FixtureOut = (
        Vec<L3Routine>,
        CombinedGraph,
        SccResult,
        FieldIndex,
        HashMap<String, Vec<UpgradedBinding>>,
    );

    /// Build the named fixture: `(routines, graph, scc, fields, upgraded_bindings)`.
    pub fn build(name: &str) -> FixtureOut {
        match name {
            "linear_known" => linear_known(),
            "recursive_self_loop" => recursive_self_loop(),
            "recursive_pair_pd" => recursive_pair_pd(),
            "pd_to_known" => pd_to_known(),
            "pd_to_unknown" => pd_to_unknown(),
            "multi_callsite_same_callee" => multi_callsite_same_callee(),
            "via_collision" => via_collision(),
            "via_collision_edges_reversed" => via_collision_edges_reversed(),
            "pd_substituted_via_collision" => pd_substituted_via_collision(false),
            "pd_substituted_via_collision_reversed" => pd_substituted_via_collision(true),
            "direct_terminal_beats_colliding_pd_substitution" => {
                direct_terminal_beats_colliding_pd_substitution()
            }
            "direct_pd_base_beats_colliding_pd_substitution_and_dedup_transition" => {
                direct_pd_base_beats_colliding_pd_substitution_and_dedup_transition()
            }
            "external_successor_pd" => external_successor_pd(),
            "fixed_leaf_in_scc" => fixed_leaf_in_scc(),
            "missing_routine_in_scc" => missing_routine_in_scc(),
            other => panic!("fixtures::build: unknown fixture {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // Shared low-level builders.
    // -----------------------------------------------------------------------

    fn anchor() -> PAnchor {
        PAnchor {
            source_unit_id: "ws:test.al".to_string(),
            start_line: 0,
            start_column: 0,
            end_line: 0,
            end_column: 0,
            syntax_kind: "test".to_string(),
        }
    }

    /// A bare, body-available `L3Routine` with just an id; callers push
    /// `record_operations`/`call_sites` onto it.
    fn routine(id: &str) -> L3Routine {
        L3Routine {
            id: id.to_string(),
            stable_routine_id: format!("stable::{id}"),
            object_id: "app/Codeunit/1".to_string(),
            object_type: "Codeunit".to_string(),
            name: id.to_string(),
            kind: "procedure".to_string(),
            attributes_parsed: Vec::new(),
            app_guid: "app".to_string(),
            object_number: 1,
            normalized_signature_hash: String::new(),
            body_available: true,
            parse_incomplete: false,
            record_variables: Vec::new(),
            record_operations: Vec::new(),
            field_accesses: Vec::new(),
            variables: Vec::new(),
            parameters: Vec::new(),
            access_modifier: None,
            return_type: None,
            call_sites: Vec::new(),
            operation_sites: Vec::new(),
            statement_tree: None,
            loops: Vec::new(),
            source_anchor: anchor(),
            identifier_references: Vec::new(),
            unreachable_statements: Vec::new(),
            has_branching: false,
            var_assignments: Vec::new(),
            condition_references: Vec::new(),
            enclosing_member: None,
            originating_object: None,
            enclosing_member_range: None,
            entry_temp_guard_receiver: None,
        }
    }

    fn ts_known(value: bool) -> PTempState {
        PTempState {
            kind: "known".to_string(),
            value: Some(value),
            parameter_index: None,
        }
    }

    fn ts_pd(idx: u32) -> PTempState {
        PTempState {
            kind: "parameter-dependent".to_string(),
            value: None,
            parameter_index: Some(idx),
        }
    }

    /// One db-touching record operation (`record_variable_name` is always
    /// `"Rec"` — no fixture below needs `RecordRoleSummary` field precision).
    fn record_op(
        id: &str,
        op: &str,
        table_id: &str,
        temp_state: Option<PTempState>,
    ) -> L3RecordOperation {
        L3RecordOperation {
            id: id.to_string(),
            op: op.to_string(),
            record_variable_name: "Rec".to_string(),
            record_variable_id: None,
            table_id: Some(table_id.to_string()),
            temp_state,
            field_arguments: None,
            source_anchor: anchor(),
            loop_stack: Vec::new(),
            field_argument_infos: None,
            in_until_condition: false,
            run_trigger: None,
        }
    }

    /// One argument binding of callee param `parameter_index` to
    /// `source_temp_state` (the shape `substitute_pd_temp_state` reads).
    fn arg_binding(
        parameter_index: u32,
        source_temp_state: Option<PTempState>,
    ) -> PCallArgumentBinding {
        PCallArgumentBinding {
            parameter_index,
            source_kind: "variable".to_string(),
            source_variable_name: Some("arg".to_string()),
            source_record_variable_id: None,
            source_parameter_index: None,
            caller_source_parameter_is_var: None,
            source_temp_state,
            argument_anchor: anchor(),
        }
    }

    /// A bare call site `id` calling `callee_name`, with the given argument
    /// bindings.
    fn call_site(
        id: &str,
        callee_name: &str,
        argument_bindings: Vec<PCallArgumentBinding>,
    ) -> PCallSite {
        PCallSite {
            id: id.to_string(),
            operation_id: format!("{id}/op"),
            callee_text: callee_name.to_string(),
            callee: PCallee::Bare {
                name: callee_name.to_string(),
            },
            argument_texts: Vec::new(),
            argument_infos: Vec::new(),
            argument_bindings,
            loop_stack: Vec::new(),
            source_anchor: anchor(),
            result_consumed: None,
            object_run_return_used: None,
            under_asserterror: None,
            control_context: None,
            order: None,
            in_statement_position: false,
        }
    }

    /// A resolved combined edge `from -> to` with the given `kind` + optional
    /// callsite id (event-dispatch edges carry none).
    fn edge(from: &str, to: &str, kind: &str, callsite_id: Option<&str>) -> CombinedEdge {
        CombinedEdge {
            from: from.to_string(),
            to: to.to_string(),
            kind: kind.to_string(),
            callsite_id: callsite_id.map(|s| s.to_string()),
            operation_id: None,
            event_id: None,
            subscriber_app_id: None,
            resolution: "resolved".to_string(),
        }
    }

    /// A `CombinedGraph` from a node list + flat edge list, grouped by `from`
    /// (mirrors `src/engine/l5/test_support.rs::graph_from_edges` — that helper
    /// is `#[cfg(test)]`-gated so it is unavailable to this integration test).
    fn graph(nodes: &[&str], edges: Vec<CombinedEdge>) -> CombinedGraph {
        let mut node_vec: Vec<String> = nodes.iter().map(|n| n.to_string()).collect();
        node_vec.sort();
        let mut edges_by_from: HashMap<String, Vec<CombinedEdge>> = HashMap::new();
        let mut edges_from_order: Vec<String> = Vec::new();
        for e in edges {
            if !edges_by_from.contains_key(&e.from) {
                edges_from_order.push(e.from.clone());
            }
            edges_by_from.entry(e.from.clone()).or_default().push(e);
        }
        CombinedGraph {
            nodes: node_vec,
            edges_by_from,
            edges_from_order,
            uncertainty_edges: Vec::new(),
            typed_edges: Vec::new(),
        }
    }

    /// An `SccResult` from an ordered list of `(members, recursive)` — the order
    /// IS the reverse-topological surface (callees before callers).
    fn scc(entries: Vec<(Vec<&str>, bool)>) -> SccResult {
        let mut sccs = Vec::new();
        let mut scc_id_by_routine = HashMap::new();
        for (i, (members, recursive)) in entries.into_iter().enumerate() {
            let members: Vec<String> = members.into_iter().map(|m| m.to_string()).collect();
            for m in &members {
                scc_id_by_routine.insert(m.clone(), i);
            }
            sccs.push(Scc { members, recursive });
        }
        SccResult {
            sccs,
            scc_id_by_routine,
        }
    }

    // -----------------------------------------------------------------------
    // Fixtures.
    // -----------------------------------------------------------------------

    /// `b` calls `a`; `a` has one `Insert` op on table `t1` (`Known(true)` temp).
    /// Two singleton non-recursive SCCs in reverse-topo order `[a, b]`.
    fn linear_known() -> FixtureOut {
        let mut a = routine("a");
        a.record_operations
            .push(record_op("a_op1", "Insert", "t1", Some(ts_known(true))));

        let mut b = routine("b");
        b.call_sites.push(call_site("b_cs1", "A", Vec::new()));

        let g = graph(&["a", "b"], vec![edge("b", "a", "direct", Some("b_cs1"))]);
        let s = scc(vec![(vec!["a"], false), (vec!["b"], false)]);

        (vec![a, b], g, s, FieldIndex::new(), HashMap::new())
    }

    /// `a` calls itself; one `ParameterDependent(0)` op whose self-call binding
    /// is `Known(true)` (a temp-record arg) — a single-member recursive SCC.
    fn recursive_self_loop() -> FixtureOut {
        let mut a = routine("a");
        a.record_operations
            .push(record_op("a_op1", "Insert", "t1", Some(ts_pd(0))));
        a.call_sites.push(call_site(
            "a_cs1",
            "A",
            vec![arg_binding(0, Some(ts_known(true)))],
        ));

        let g = graph(&["a"], vec![edge("a", "a", "direct", Some("a_cs1"))]);
        let s = scc(vec![(vec!["a"], true)]);

        (vec![a], g, s, FieldIndex::new(), HashMap::new())
    }

    /// `a` <-> `b` mutual recursion; `b` has a `ParameterDependent(0)` op that
    /// `a`'s callsite forwards as its OWN `ParameterDependent(0)` binding — the
    /// re-symbolize-upward substitution path (RV-7).
    fn recursive_pair_pd() -> FixtureOut {
        let mut a = routine("a");
        a.call_sites.push(call_site(
            "a_cs1",
            "B",
            vec![arg_binding(0, Some(ts_pd(0)))],
        ));

        let mut b = routine("b");
        b.record_operations
            .push(record_op("b_op1", "Insert", "t2", Some(ts_pd(0))));
        b.call_sites.push(call_site("b_cs1", "A", Vec::new()));

        let g = graph(
            &["a", "b"],
            vec![
                edge("a", "b", "direct", Some("a_cs1")),
                edge("b", "a", "direct", Some("b_cs1")),
            ],
        );
        let s = scc(vec![(vec!["a", "b"], true)]);

        (vec![a, b], g, s, FieldIndex::new(), HashMap::new())
    }

    /// `caller` passes a `temporary`-keyword record into `callee`, whose one op
    /// is `ParameterDependent(0)` — substitutes to `Known(true)`.
    fn pd_to_known() -> FixtureOut {
        let mut callee = routine("callee");
        callee
            .record_operations
            .push(record_op("callee_op1", "Insert", "t1", Some(ts_pd(0))));

        let mut caller = routine("caller");
        caller.call_sites.push(call_site(
            "caller_cs1",
            "Callee",
            vec![arg_binding(0, Some(ts_known(true)))],
        ));

        let g = graph(
            &["callee", "caller"],
            vec![edge("caller", "callee", "direct", Some("caller_cs1"))],
        );
        let s = scc(vec![(vec!["callee"], false), (vec!["caller"], false)]);

        (
            vec![callee, caller],
            g,
            s,
            FieldIndex::new(),
            HashMap::new(),
        )
    }

    /// Same shape as `pd_to_known`, but the binding carries no captured source
    /// temp state (e.g. an unresolved/non-record argument) — substitutes to
    /// `Unknown`.
    fn pd_to_unknown() -> FixtureOut {
        let mut callee = routine("callee");
        callee
            .record_operations
            .push(record_op("callee_op1", "Insert", "t1", Some(ts_pd(0))));

        let mut caller = routine("caller");
        caller.call_sites.push(call_site(
            "caller_cs1",
            "Callee",
            vec![arg_binding(0, None)],
        ));

        let g = graph(
            &["callee", "caller"],
            vec![edge("caller", "callee", "direct", Some("caller_cs1"))],
        );
        let s = scc(vec![(vec!["callee"], false), (vec!["caller"], false)]);

        (
            vec![callee, caller],
            g,
            s,
            FieldIndex::new(),
            HashMap::new(),
        )
    }

    /// `a` calls `b` twice via two DIFFERENT call sites with different bindings
    /// for the same `ParameterDependent(0)` op — divergent substitutions stay
    /// two DISTINCT folded effects (different re-keyed `effect_key`s), never
    /// merged.
    fn multi_callsite_same_callee() -> FixtureOut {
        let mut b = routine("b");
        b.record_operations
            .push(record_op("b_op1", "Insert", "t1", Some(ts_pd(0))));

        let mut a = routine("a");
        a.call_sites.push(call_site(
            "a_cs1",
            "B",
            vec![arg_binding(0, Some(ts_known(true)))],
        ));
        a.call_sites.push(call_site(
            "a_cs2",
            "B",
            vec![arg_binding(0, Some(ts_known(false)))],
        ));

        let g = graph(
            &["a", "b"],
            vec![
                edge("a", "b", "direct", Some("a_cs1")),
                edge("a", "b", "direct", Some("a_cs2")),
            ],
        );
        let s = scc(vec![(vec!["b"], false), (vec!["a"], false)]);

        (vec![a, b], g, s, FieldIndex::new(), HashMap::new())
    }

    /// `a` reaches the SAME `callee` effect (a `Known(true)` op, non-PD) via a
    /// `direct` edge AND an `event-dispatch` edge — the folded `via` must be the
    /// max-rank of the two (`event-subscriber`, rank 2, beats `direct`'s
    /// `inherited`, rank 0 — see `effect_lattice::via_for_edge_kind`/`merge_via`).
    fn via_collision() -> FixtureOut {
        let mut callee = routine("callee");
        callee.record_operations.push(record_op(
            "callee_op1",
            "Insert",
            "t1",
            Some(ts_known(true)),
        ));

        let mut a = routine("a");
        a.call_sites.push(call_site("a_cs1", "Callee", Vec::new()));

        let g = graph(
            &["a", "callee"],
            vec![
                edge("a", "callee", "direct", Some("a_cs1")),
                edge("a", "callee", "event-dispatch", None),
            ],
        );
        let s = scc(vec![(vec!["callee"], false), (vec!["a"], false)]);

        (vec![a, callee], g, s, FieldIndex::new(), HashMap::new())
    }

    // -----------------------------------------------------------------------
    // Task A2 via-rank-merge guard fixtures (spec Step 3 test list, lines
    // 149-152): the "simplest safe v1" chosen for A2 keeps `reconstruct_via`/
    // `attribute_pd_substituted_via`'s EXISTING traversal structure, only
    // retyping the via map's VALUE from `String` to `ViaRank` — so these
    // fixtures pin that the retyped max-rank merge stays byte-identical to
    // the old solver across the specific collision/order shapes the spec's
    // Step 2 write-up calls out, not a NEW algorithm discovery (the
    // differential's byte-for-byte comparison against the untouched OLD
    // solver is the correctness oracle for every one of them).
    // -----------------------------------------------------------------------

    /// `via_collision`, but with its two edges added in the OPPOSITE order
    /// (`event-dispatch` first, `direct` second) — pins that the max-rank
    /// merge for a TERMINAL effect is order-INDEPENDENT (spec: "first-
    /// transition rank-0 then a later-transition rank-2/3" / vice versa),
    /// not an artifact of `graph.edges_by_from`'s insertion order.
    fn via_collision_edges_reversed() -> FixtureOut {
        let mut callee = routine("callee");
        callee.record_operations.push(record_op(
            "callee_op1",
            "Insert",
            "t1",
            Some(ts_known(true)),
        ));

        let mut a = routine("a");
        a.call_sites.push(call_site("a_cs1", "Callee", Vec::new()));

        let g = graph(
            &["a", "callee"],
            vec![
                edge("a", "callee", "event-dispatch", None),
                edge("a", "callee", "direct", Some("a_cs1")),
            ],
        );
        let s = scc(vec![(vec!["callee"], false), (vec!["a"], false)]);

        (vec![a, callee], g, s, FieldIndex::new(), HashMap::new())
    }

    /// `a` reaches `b`'s SAME `ParameterDependent(0)` effect via TWO
    /// callsites of DIFFERENT edge kinds — `"direct"` (`via_for_edge_kind`
    /// -> `"inherited"`, rank 0) and `"implicit-trigger"` (rank 3) — both
    /// forwarding `b`'s param 0 to `a`'s own param 0, so BOTH substitute to
    /// the IDENTICAL produced identity on `a`. Exercises
    /// `attribute_pd_substituted_via`'s max-rank merge on the PD-substitution
    /// path specifically (spec item: "two callsites producing the same PD
    /// state at different ranks"). `reversed` controls edge insertion order
    /// — both orderings must reach the SAME old-solver-verified result (spec
    /// item: "first-transition rank-0 then a later-transition rank-2/3").
    fn pd_substituted_via_collision(reversed: bool) -> FixtureOut {
        let mut b = routine("b");
        b.record_operations
            .push(record_op("b_op1", "Insert", "t2", Some(ts_pd(0))));

        let mut a = routine("a");
        a.call_sites.push(call_site(
            "a_cs_direct",
            "B",
            vec![arg_binding(0, Some(ts_pd(0)))],
        ));
        a.call_sites.push(call_site(
            "a_cs_trigger",
            "B",
            vec![arg_binding(0, Some(ts_pd(0)))],
        ));

        let mut edges = vec![
            edge("a", "b", "direct", Some("a_cs_direct")),
            edge("a", "b", "implicit-trigger", Some("a_cs_trigger")),
        ];
        if reversed {
            edges.reverse();
        }
        let g = graph(&["a", "b"], edges);
        let s = scc(vec![(vec!["b"], false), (vec!["a"], false)]);

        (vec![a, b], g, s, FieldIndex::new(), HashMap::new())
    }

    /// `a` owns a base `Known(true)` effect at the SAME `(op, table_id,
    /// operation_id)` triple as `b`'s `ParameterDependent(0)` effect; `a`
    /// calls `b` over a `"direct"` edge (`via_for_edge_kind` -> `"inherited"`,
    /// rank 0) binding param 0 to `Known(true)` — the substitution's produced
    /// identity COLLIDES exactly with `a`'s own base effect. `reconstruct_via`'s
    /// init seeds `"direct"` (rank 4) for `a`'s own effect BEFORE
    /// `attribute_pd_substituted_via` runs; the rank-0 `"inherited"`
    /// contribution from the colliding edge must NOT downgrade it (spec item:
    /// "a PD→Known colliding with a direct terminal").
    fn direct_terminal_beats_colliding_pd_substitution() -> FixtureOut {
        let mut b = routine("b");
        b.record_operations
            .push(record_op("shared_op1", "Insert", "t5", Some(ts_pd(0))));

        let mut a = routine("a");
        a.record_operations.push(record_op(
            "shared_op1",
            "Insert",
            "t5",
            Some(ts_known(true)),
        ));
        a.call_sites.push(call_site(
            "a_cs1",
            "B",
            vec![arg_binding(0, Some(ts_known(true)))],
        ));

        let g = graph(&["a", "b"], vec![edge("a", "b", "direct", Some("a_cs1"))]);
        let s = scc(vec![(vec!["b"], false), (vec!["a"], false)]);

        (vec![a, b], g, s, FieldIndex::new(), HashMap::new())
    }

    /// Same shape as [`direct_terminal_beats_colliding_pd_substitution`], but
    /// `a`'s own base effect STAYS `ParameterDependent(2)` (rather than
    /// resolving to `Known`) and the substitution's produced identity is ALSO
    /// `ParameterDependent(2)` — exercising the collision on the PD/delta
    /// storage path (spec item: "a PD→PD colliding with a direct PD").
    /// Because `a`'s own base PD(2) fact (Step A Seed 1) and the
    /// edge-substituted PD(2) fact (Step A Seed 2, forwarding `a`'s own
    /// param 2 through the `a -> b` edge) are the IDENTICAL `PdState`, this
    /// ALSO exercises the spec's "a duplicate transition where
    /// `visited.insert` is false" item — Step A's worklist dedup must not
    /// lose either the fact or (independently) its via attribution.
    fn direct_pd_base_beats_colliding_pd_substitution_and_dedup_transition() -> FixtureOut {
        let mut b = routine("b");
        b.record_operations
            .push(record_op("shared_op2", "Insert", "t6", Some(ts_pd(0))));

        let mut a = routine("a");
        a.record_operations
            .push(record_op("shared_op2", "Insert", "t6", Some(ts_pd(2))));
        a.call_sites.push(call_site(
            "a_cs1",
            "B",
            vec![arg_binding(0, Some(ts_pd(2)))],
        ));

        let g = graph(&["a", "b"], vec![edge("a", "b", "direct", Some("a_cs1"))]);
        let s = scc(vec![(vec!["b"], false), (vec!["a"], false)]);

        (vec![a, b], g, s, FieldIndex::new(), HashMap::new())
    }

    /// `ext` is an already-settled successor SCC (processed BEFORE the recursive
    /// SCC below it) carrying a `ParameterDependent(0)` op; the self-recursive
    /// `a` calls both itself and `ext` — the `ext` edge substitutes through `a`'s
    /// own binding while `ext`'s summary is read from the PREDECESSOR final map,
    /// not the in-SCC snapshot (`compose_routine`'s `lookup` fallback).
    fn external_successor_pd() -> FixtureOut {
        let mut ext = routine("ext");
        ext.record_operations
            .push(record_op("ext_op1", "Insert", "t1", Some(ts_pd(0))));

        let mut a = routine("a");
        a.call_sites.push(call_site("a_self_cs", "A", Vec::new()));
        a.call_sites.push(call_site(
            "a_ext_cs",
            "Ext",
            vec![arg_binding(0, Some(ts_known(true)))],
        ));

        let g = graph(
            &["a", "ext"],
            vec![
                edge("a", "a", "direct", Some("a_self_cs")),
                edge("a", "ext", "direct", Some("a_ext_cs")),
            ],
        );
        let s = scc(vec![(vec!["ext"], false), (vec!["a"], true)]);

        (vec![a, ext], g, s, FieldIndex::new(), HashMap::new())
    }

    /// A 3-member cycle `a -> b -> c -> a`; `c` is a FIXED LEAF (its summary
    /// comes from `leaf_summaries` — see [`fixed_leaf_in_scc_leaves`] — and is
    /// never recomputed). `b`'s composition must read `c`'s db effect from the
    /// pre-seeded final map rather than `c`'s (nonexistent) base summary.
    fn fixed_leaf_in_scc() -> FixtureOut {
        let mut a = routine("a");
        a.call_sites.push(call_site("a_cs1", "B", Vec::new()));

        let mut b = routine("b");
        b.call_sites.push(call_site("b_cs1", "C", Vec::new()));

        let mut c = routine("c");
        c.call_sites.push(call_site("c_cs1", "A", Vec::new()));

        let g = graph(
            &["a", "b", "c"],
            vec![
                edge("a", "b", "direct", Some("a_cs1")),
                edge("b", "c", "direct", Some("b_cs1")),
                edge("c", "a", "direct", Some("c_cs1")),
            ],
        );
        let s = scc(vec![(vec!["a", "b", "c"], true)]);

        (vec![a, b, c], g, s, FieldIndex::new(), HashMap::new())
    }

    /// The fixed-leaf summary for `fixed_leaf_in_scc`'s `c` member: one settled
    /// `Insert`/`Known(true)` effect on table `t3`, via `"direct"` (a retained
    /// dependency-routine summary, R3a-5-shaped).
    pub fn fixed_leaf_in_scc_leaves() -> HashMap<String, RoutineSummary> {
        let table_id = "t3";
        let effect_key = effect_key_of("Insert", table_id, "c_op1", &TempStateKind::Known(true));
        let mut leaves = HashMap::new();
        leaves.insert(
            "c".to_string(),
            RoutineSummary {
                routine_id: "c".to_string(),
                db_effects: vec![DbEffect {
                    effect_key,
                    operation_id: "c_op1".to_string(),
                    op: "Insert".to_string(),
                    table_id: table_id.to_string(),
                    record_variable_id: None,
                    temp_state: TempState::Known(true),
                    via: "direct".to_string(),
                }],
                in_recursive_cycle: false,
                has_unresolved_calls: false,
                uncertainties: Vec::new(),
                parameter_roles: Vec::new(),
            },
        );
        leaves
    }

    /// `ghost` is present in `scc.members` (a 2-member recursive SCC alongside
    /// `a`) but ABSENT from `routines` — the solver must skip it gracefully
    /// (no panic) while `a`'s own composition records an unresolved call to it.
    fn missing_routine_in_scc() -> FixtureOut {
        let mut a = routine("a");
        a.call_sites.push(call_site("a_cs1", "Ghost", Vec::new()));

        let g = graph(
            &["a", "ghost"],
            vec![edge("a", "ghost", "direct", Some("a_cs1"))],
        );
        let s = scc(vec![(vec!["a", "ghost"], true)]);

        (vec![a], g, s, FieldIndex::new(), HashMap::new())
    }
}
