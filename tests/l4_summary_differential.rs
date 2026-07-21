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
    compute_summaries_v2_with_leaves, compute_summaries_with_leaves,
};

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
    let new = compute_summaries_v2_with_leaves(&routines, &graph, &scc, &ub, &fields, leaves);
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
