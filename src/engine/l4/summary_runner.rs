//! L4 JACOBI fixed-point summary runner (R3a-2).
//!
//! Ports al-sem's `src/engine/summary-runner.ts` (`computeSummaries` /
//! `runSummaries` — the SCC walk, the per-SCC JACOBI fixed-point loop,
//! the fingerprint, the `composeRoutineCtx`, the parameterRoles cross-call
//! composition) and `src/engine/summary-engine.ts`
//! (`baseIntraproceduralSummaryCtx` / `computeRecordRolesCtx`).
//!
//! ## JACOBI discipline (THE LOAD-BEARING CORRECTNESS RULE)
//!
//! Each pass FREEZES the entire prior-pass summary map; ALL reads within a
//! pass see the frozen snapshot; writes go to a NEW map; maps swap at end of
//! pass. This is JACOBI, NOT Gauss-Seidel. The trace oracle (R3a-2 Rev 2 #3)
//! captures the per-pass fingerprint sequence — it diverges under Gauss-Seidel
//! because the trajectory differs (different iteration count, different per-pass
//! `changed`). The `snapshot` clone inside the loop MUST be a true deep copy of
//! the PRIOR pass; the `in_progress` accumulator must ONLY be written, never
//! read, during a pass.

use std::collections::{BTreeMap, HashMap};

use super::combined_graph::CombinedGraph;
use super::effect_lattice::{
    effect_key_of, join_presence, merge_via_owned, via_for_edge_kind, EffectPresence,
};
use super::scc::SccResult;
use super::summary::{
    project_routine_summary_core_internal, stable_summary_fingerprint, DbEffect, FieldList,
    PRoutineSummaryCore, RecordRoleSummary, RoutineSummary, TempState, Uncertainty,
};
use crate::engine::l3::call_resolver::UpgradedBinding;
use crate::engine::l3::l3_workspace::L3Routine;

const MAX_FIXED_POINT_ITERATIONS: usize = 1000;

// ---------------------------------------------------------------------------
// Trace hook types.
// ---------------------------------------------------------------------------

/// One raw SCC trace (internal ids).
pub struct RawSccTrace {
    pub members: Vec<String>,
    pub passes: Vec<RawSccTracePass>,
}

/// One pass in the raw SCC trace.
pub struct RawSccTracePass {
    pub iteration: usize,
    pub changed: bool,
    /// Projected summaries in member order (used to build per-pass fingerprint).
    pub member_summaries: Vec<PRoutineSummaryCore>,
}

// ---------------------------------------------------------------------------
// Op classification (ports src/engine/op-classification.ts).
// ---------------------------------------------------------------------------

fn is_db_touching(op: &str) -> bool {
    matches!(
        op,
        "FindSet"
            | "FindFirst"
            | "FindLast"
            | "Find"
            | "Get"
            | "Next"
            | "Count"
            | "CountApprox"
            | "IsEmpty"
            | "CalcFields"
            | "CalcSums"
            | "Modify"
            | "ModifyAll"
            | "Insert"
            | "Delete"
            | "DeleteAll"
            | "LockTable"
    )
}

pub(crate) fn record_flow_role(op: &str) -> &'static str {
    match op {
        "Get" | "FindFirst" | "FindLast" | "FindSet" | "Find" | "Next" => "loadsFromDb",
        "Init" => "initialises",
        "Modify" | "Insert" => "persistsCurrent",
        "ModifyAll" | "DeleteAll" => "setBasedWrite",
        "Validate" => "validates",
        "Copy" | "TransferFields" => "copiesInto",
        "Reset" => "resetsFilter",
        _ => "neutral",
    }
}

// ---------------------------------------------------------------------------
// Base intraprocedural summary (ports baseIntraproceduralSummaryCtx).
// ---------------------------------------------------------------------------

/// Build a routine's summary from its OWN intraprocedural features only — no
/// callee composition. Mirrors al-sem `baseIntraproceduralSummaryCtx`.
pub fn base_intraprocedural_summary(
    routine: &L3Routine,
    _routines_by_id: &HashMap<String, &L3Routine>,
) -> RoutineSummary {
    let parameter_roles = compute_record_roles(routine);

    // Opaque (.app symbol, no body).
    if !routine.body_available {
        return RoutineSummary {
            routine_id: routine.id.clone(),
            db_effects: Vec::new(),
            in_recursive_cycle: false,
            has_unresolved_calls: true,
            uncertainties: Vec::new(),
            parameter_roles,
        };
    }

    // Parse-incomplete — body present but unparseable.
    if routine.parse_incomplete {
        return RoutineSummary {
            routine_id: routine.id.clone(),
            db_effects: Vec::new(),
            in_recursive_cycle: false,
            has_unresolved_calls: true,
            uncertainties: vec![Uncertainty {
                kind: "parse-incomplete".to_string(),
                callsite_id: None,
                operation_id: None,
                routine_id: Some(routine.id.clone()),
                interface_name: None,
            }],
            parameter_roles,
        };
    }

    // Body available + parsed — derive direct facts from the operation stream.
    let mut db_effects: Vec<DbEffect> = Vec::new();
    for op in &routine.record_operations {
        if !is_db_touching(&op.op) {
            continue;
        }
        let table_id = op.table_id.clone().unwrap_or_else(|| "unknown".to_string());
        let temp_state = op
            .temp_state
            .as_ref()
            .map(TempState::from_p)
            .unwrap_or(TempState::Unknown);
        let temp_kind = temp_state.to_kind();
        let effect_key = effect_key_of(&op.op, &table_id, &op.id, &temp_kind);
        db_effects.push(DbEffect {
            effect_key,
            operation_id: op.id.clone(),
            op: op.op.clone(),
            table_id,
            record_variable_id: op.record_variable_id.clone(),
            temp_state,
            via: "direct".to_string(),
        });
    }

    // Sort by effect_key for determinism (matches al-sem sort).
    db_effects.sort_by(|a, b| a.effect_key.cmp(&b.effect_key));

    RoutineSummary {
        routine_id: routine.id.clone(),
        db_effects,
        in_recursive_cycle: false,
        has_unresolved_calls: false,
        uncertainties: Vec::new(),
        parameter_roles,
    }
}

/// Compute RecordRoleSummary per record parameter. Mirrors al-sem
/// `computeRecordRolesCtx`. Path-aware facts (requiresLoadedAtEntry etc.) are
/// populated as "unknown" here; `compose_routine` overwrites them with the
/// flat-walker facts (which need the current fixed-point `lookup`).
fn compute_record_roles(routine: &L3Routine) -> Vec<RecordRoleSummary> {
    let mut out: Vec<RecordRoleSummary> = Vec::new();
    for param in &routine.parameters {
        if !param.is_record {
            continue;
        }
        let rec_var = routine
            .record_variables
            .iter()
            .find(|rv| rv.is_parameter && rv.parameter_index == Some(param.index));
        let rec_var = match rec_var {
            Some(rv) => rv,
            None => continue,
        };
        let table_id = rec_var
            .table_id
            .clone()
            .unwrap_or_else(|| "unknown".to_string());

        let mut reads_fields: Vec<String> = Vec::new();
        let mut writes_fields: Vec<String> = Vec::new();
        let mut may_reset_filters = false;
        let mut may_change_load_fields = false;
        let mut may_assign_record = false;
        let mut loads_from_db_param = EffectPresence::No;
        let mut initialises_param = EffectPresence::No;
        let mut persists_current_record = EffectPresence::No;
        let mut set_based_db_writes = EffectPresence::No;
        let mut validates_param = EffectPresence::No;
        let mut copies_into_param = EffectPresence::No;
        let mut resets_filters_on_param = EffectPresence::No;

        let rec_var_name_lc = rec_var.name.to_lowercase();

        // Field accesses — readsFields.
        for fa in &routine.field_accesses {
            if fa.record_variable_name.to_lowercase() != rec_var_name_lc {
                continue;
            }
            if let Some(fid) = resolve_field(&table_id, &fa.field_name, routine) {
                reads_fields.push(fid);
            }
        }

        // Record operations — may-fact bootstrap.
        for op in &routine.record_operations {
            if op.record_variable_name.to_lowercase() != rec_var_name_lc {
                continue;
            }
            if op.op == "Validate" {
                if let Some(args) = &op.field_arguments {
                    for arg in args {
                        if let Some(fid) = resolve_field(&table_id, arg, routine) {
                            writes_fields.push(fid);
                        }
                    }
                }
            }
            if op.op == "Reset" || op.op == "Copy" {
                may_reset_filters = true;
            }
            if op.op == "SetLoadFields" || op.op == "AddLoadFields" || op.op == "Reset" {
                may_change_load_fields = true;
            }
            if op.op == "Copy" || op.op == "TransferFields" {
                may_assign_record = true;
            }
            match record_flow_role(&op.op) {
                "loadsFromDb" => loads_from_db_param = EffectPresence::Yes,
                "initialises" => initialises_param = EffectPresence::Yes,
                "persistsCurrent" => persists_current_record = EffectPresence::Yes,
                "setBasedWrite" => set_based_db_writes = EffectPresence::Yes,
                "validates" => validates_param = EffectPresence::Yes,
                "copiesInto" => copies_into_param = EffectPresence::Yes,
                "resetsFilter" => resets_filters_on_param = EffectPresence::Yes,
                _ => {}
            }
        }

        let may_use_record_ref = param.type_text.to_lowercase().contains("recordref")
            || param.type_text.to_lowercase().contains("fieldref")
            || param.type_text.to_lowercase().contains("variant");

        let (reads_fields_fl, writes_fields_fl) = if may_use_record_ref {
            (FieldList::Unknown, FieldList::Unknown)
        } else {
            let rf: Vec<String> = reads_fields
                .into_iter()
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect();
            let wf: Vec<String> = writes_fields
                .into_iter()
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect();
            (FieldList::Known(rf), FieldList::Known(wf))
        };

        let mutates_param = join_presence(
            join_presence(persists_current_record, validates_param),
            copies_into_param,
        );

        out.push(RecordRoleSummary {
            parameter_index: param.index,
            table_id,
            reads_fields: reads_fields_fl,
            writes_fields: writes_fields_fl,
            may_reset_filters,
            may_change_load_fields,
            may_assign_record,
            may_use_record_ref,
            // Path-aware entry-req + exit-effect facts are "unknown" until
            // `compose_routine` runs the flat walker with the current lookup.
            requires_loaded_at_entry: EffectPresence::Unknown,
            required_loaded_fields_at_entry: FieldList::Unknown,
            mutates_before_load: EffectPresence::Unknown,
            persists_current_record,
            set_based_db_writes,
            validates_param,
            copies_into_param,
            resets_filters_on_param,
            dirty_at_exit: EffectPresence::Unknown,
            current_loaded_fields_at_exit: FieldList::Unknown,
            mutates_param,
            loads_from_db_param,
            initialises_param,
        });
    }
    out.sort_by_key(|r| r.parameter_index);
    out
}

/// Resolve a field name to its internal FieldId by table. Returns None if the
/// table or field is not found.
fn resolve_field(_table_id: &str, _field_name: &str, _routine: &L3Routine) -> Option<String> {
    // Field-id resolution deferred to R3a-3 (requires direct table access).
    // The base parameterRoles will have empty reads/writesFields, matching
    // vector expectations for routines without field-accessing operations.
    None
}

// ---------------------------------------------------------------------------
// compose_routine — compose one routine's summary (mirrors composeRoutineCtx).
// ---------------------------------------------------------------------------

/// Compose a routine's full summary: start from its base intraprocedural
/// summary, then fold in every outgoing combined edge's callee summary.
/// Mirrors al-sem `composeRoutineCtx`.
///
/// `snapshot` is the FROZEN prior-pass map (JACOBI: reads must go here).
/// `final_map` is the settled summaries for already-processed SCCs.
/// `base_summaries` provides the intraprocedural-only base for each routine.
/// `upgraded_bindings` is the per-callsite side table from the call resolver.
#[allow(clippy::too_many_arguments)]
fn compose_routine(
    routine: &L3Routine,
    snapshot: &HashMap<String, RoutineSummary>,
    final_map: &HashMap<String, RoutineSummary>,
    base_summaries: &HashMap<String, RoutineSummary>,
    upgraded_bindings: &HashMap<String, Vec<UpgradedBinding>>,
    graph: &CombinedGraph,
    body_avail_by_id: &HashMap<String, bool>,
) -> RoutineSummary {
    // For non-recursive SCCs `snapshot` is empty; reads fall through to `final_map`.
    let lookup =
        |id: &str| -> Option<&RoutineSummary> { snapshot.get(id).or_else(|| final_map.get(id)) };

    let base = base_summaries
        .get(&routine.id)
        .cloned()
        .unwrap_or_else(|| base_intraprocedural_summary(routine, &HashMap::new()));

    let mut has_unresolved_calls = base.has_unresolved_calls;

    // Seed accumulators from the base.
    let mut db_effects_by_key: BTreeMap<String, DbEffect> = BTreeMap::new();
    for e in &base.db_effects {
        db_effects_by_key.insert(e.effect_key.clone(), e.clone());
    }
    let mut uncertainties_by_key: BTreeMap<String, Uncertainty> = BTreeMap::new();
    for u in &base.uncertainties {
        uncertainties_by_key.insert(uncertainty_key(u), u.clone());
    }

    // Fold in callee summaries from the combined graph.
    let empty_edges: Vec<super::combined_graph::CombinedEdge> = Vec::new();
    let edges = graph.edges_by_from.get(&routine.id).unwrap_or(&empty_edges);

    for edge in edges {
        let callee_summary = lookup(&edge.to);
        if callee_summary.is_none() {
            has_unresolved_calls = true;
            continue;
        }
        let callee_summary = callee_summary.unwrap();
        let via = via_for_edge_kind(&edge.kind);

        // Fold db_effects.
        for e in &callee_summary.db_effects {
            let key = &e.effect_key;
            match db_effects_by_key.get(key) {
                Some(existing) => {
                    let merged_via = merge_via_owned(&existing.via, via);
                    let updated = DbEffect {
                        via: merged_via,
                        ..existing.clone()
                    };
                    db_effects_by_key.insert(key.clone(), updated);
                }
                None => {
                    db_effects_by_key.insert(
                        key.clone(),
                        DbEffect {
                            via: via.to_string(),
                            ..e.clone()
                        },
                    );
                }
            }
        }

        // Fold uncertainties (skip callsite-local kinds).
        for u in &callee_summary.uncertainties {
            if matches!(
                u.kind.as_str(),
                "member-not-found"
                    | "external-target"
                    | "ambiguous-overload"
                    | "interface-open-world"
            ) {
                continue;
            }
            let k = uncertainty_key(u);
            uncertainties_by_key.entry(k).or_insert_with(|| u.clone());
        }

        if callee_summary.has_unresolved_calls {
            has_unresolved_calls = true;
        }

        // Interface / dynamic edges, AND opaque (bodyless) callees → add an
        // opaque-callee uncertainty (al-sem summary-runner.ts:213:
        // `edge.kind === "interface" || edge.kind === "dynamic" ||
        // calleeOpaque(edge.to)`). FIX 3: the `calleeOpaque` disjunct was dropped
        // as "unreachable source-only"; it IS reachable (a body-available caller
        // with a resolved DIRECT edge to a bodyless callee), so restore it for
        // faithfulness. `calleeOpaque(id)` === `bodyAvailable === false`.
        let callee_opaque = !body_avail_by_id.get(&edge.to).copied().unwrap_or(false);
        let add_opaque = edge.kind == "interface" || edge.kind == "dynamic" || callee_opaque;
        if add_opaque {
            if let Some(cs_id) = &edge.callsite_id {
                let u = Uncertainty {
                    kind: "opaque-callee".to_string(),
                    callsite_id: Some(cs_id.clone()),
                    operation_id: None,
                    routine_id: None,
                    interface_name: None,
                };
                let k = uncertainty_key(&u);
                uncertainties_by_key.entry(k).or_insert(u);
            }
            has_unresolved_calls = true;
        }
    }

    // Uncertainty edges (to-less call sites) — walk the global list for this routine.
    for ue in &graph.uncertainty_edges {
        if ue.from != routine.id {
            continue;
        }
        let u = Uncertainty {
            kind: ue.uncertainty.kind.clone(),
            callsite_id: ue.uncertainty.callsite_id.clone(),
            operation_id: ue.uncertainty.operation_id.clone(),
            routine_id: ue.uncertainty.routine_id.clone(),
            interface_name: ue.uncertainty.interface_name.clone(),
        };
        let k = uncertainty_key(&u);
        uncertainties_by_key.entry(k).or_insert(u);
        has_unresolved_calls = true;
    }

    // Materialize sorted arrays.
    let db_effects: Vec<DbEffect> = {
        let mut v: Vec<DbEffect> = db_effects_by_key.into_values().collect();
        v.sort_by(|a, b| {
            a.effect_key
                .cmp(&b.effect_key)
                .then_with(|| a.operation_id.cmp(&b.operation_id))
        });
        v
    };
    let uncertainties: Vec<Uncertainty> = {
        let mut v: Vec<Uncertainty> = uncertainties_by_key.into_values().collect();
        v.sort_by_key(uncertainty_key);
        v
    };

    // parameterRoles: cross-call exit-effect composition.
    // Deep-copy the base parameterRoles so we can mutate them independently.
    let mut parameter_roles: Vec<RecordRoleSummary> =
        base.parameter_roles.iter().map(clone_role).collect();

    // Cross-call exit-effect composition (spec §(c1b)).
    // `binding_resolution` and `callee_parameter_is_var` live in the upgraded-
    // bindings side table (from the call resolver), NOT on PCallArgumentBinding.
    for cs in &routine.call_sites {
        // Get the upgraded bindings for this callsite (if any).
        let upgraded = upgraded_bindings.get(&cs.id);

        // Find the resolved callee edge for this callsite.
        let edge = graph.edges_by_from.get(&routine.id).and_then(|edges| {
            edges
                .iter()
                .find(|e| e.callsite_id.as_deref() == Some(&cs.id))
        });

        for (arg_idx, binding) in cs.argument_bindings.iter().enumerate() {
            // Get the upgraded state for this binding position.
            let upgraded_b = upgraded.and_then(|ub| ub.get(arg_idx));

            // Only proceed if the binding is resolved.
            let resolution = upgraded_b
                .map(|ub| ub.binding_resolution.as_str())
                .unwrap_or("unresolved-callee");
            if resolution != "resolved" {
                continue;
            }
            let source_param_idx = match binding.source_parameter_index {
                Some(i) => i,
                None => continue,
            };
            if binding.caller_source_parameter_is_var != Some(true) {
                continue;
            }
            let callee_param_is_var = upgraded_b
                .map(|ub| ub.callee_parameter_is_var)
                .unwrap_or(false);
            if !callee_param_is_var {
                continue;
            }

            let callee_id = match edge.map(|e| e.to.as_str()) {
                Some(id) => id,
                None => continue,
            };
            let callee_summary = lookup(callee_id);
            // FIX 2: the opaque guard takes the "unknown" branch on ANY of the three
            // al-sem reasons (summary-runner.ts:267-270): no callee summary/role, OR
            // `callee.bodyAvailable === false`. A bodyless callee carries a role with
            // all-`No` facts; without this guard we would join "no" (an unsound flip).
            let callee_body_available = body_avail_by_id.get(callee_id).copied().unwrap_or(false);
            let callee_role = callee_summary.and_then(|s| {
                s.parameter_roles
                    .iter()
                    .find(|r| r.parameter_index == binding.parameter_index)
            });
            let opaque =
                callee_summary.is_none() || callee_role.is_none() || !callee_body_available;

            let p = parameter_roles
                .iter_mut()
                .find(|r| r.parameter_index == source_param_idx);
            let p = match p {
                Some(p) => p,
                None => continue,
            };
            if opaque {
                p.persists_current_record =
                    join_presence(p.persists_current_record, EffectPresence::Unknown);
                p.set_based_db_writes =
                    join_presence(p.set_based_db_writes, EffectPresence::Unknown);
                p.validates_param = join_presence(p.validates_param, EffectPresence::Unknown);
                p.copies_into_param = join_presence(p.copies_into_param, EffectPresence::Unknown);
                p.resets_filters_on_param =
                    join_presence(p.resets_filters_on_param, EffectPresence::Unknown);
            } else {
                let cr = callee_role.unwrap();
                p.persists_current_record =
                    join_presence(p.persists_current_record, cr.persists_current_record);
                p.set_based_db_writes =
                    join_presence(p.set_based_db_writes, cr.set_based_db_writes);
                p.validates_param = join_presence(p.validates_param, cr.validates_param);
                p.copies_into_param = join_presence(p.copies_into_param, cr.copies_into_param);
                p.resets_filters_on_param =
                    join_presence(p.resets_filters_on_param, cr.resets_filters_on_param);
            }
            p.mutates_param = join_presence(
                join_presence(p.persists_current_record, p.validates_param),
                p.copies_into_param,
            );
        }
    }

    // Path-aware entry-requirement + exit-effect composition (spec §(c1a)/(c1b)).
    // Mirrors al-sem summary-runner.ts lines 310-325: after cross-call c1b, run
    // the BRANCH-AWARE walker (`cfg_walker::walk_param`, the port of `walkRoutine`
    // → `walkCFG`) with the current JACOBI `lookup` so callee summaries are from
    // the current iteration. The walker overwrites the "unknown" entry-req +
    // exit-effect placeholders from the base summary with PATH-PROVEN facts:
    // a Validate/Modify/field-access INSIDE a conditional yields a branch-joined
    // (often "unknown") result, not the straight-line "yes"/"no".
    // Only runs when the body is available + parsed (opaque/parse-incomplete stay
    // "unknown" as set by the base summary).
    if routine.body_available && !routine.parse_incomplete {
        for param_role in &mut parameter_roles {
            let rec_var = routine.record_variables.iter().find(|rv| {
                rv.is_parameter && rv.parameter_index == Some(param_role.parameter_index)
            });
            let (rec_var_name_lc, rec_var_id) = match rec_var {
                Some(rv) => (rv.name.to_lowercase(), Some(rv.id.as_str())),
                None => continue,
            };
            let f = crate::engine::l4::cfg_walker::walk_param(
                routine,
                &rec_var_name_lc,
                rec_var_id,
                snapshot,
                final_map,
                upgraded_bindings,
                graph,
                body_avail_by_id,
            );
            param_role.requires_loaded_at_entry = f.requires_loaded_at_entry;
            param_role.mutates_before_load = f.mutates_before_load;
            param_role.dirty_at_exit = f.dirty_at_exit;
            param_role.current_loaded_fields_at_exit = f.current_loaded_fields_at_exit;
            param_role.required_loaded_fields_at_entry = f.required_loaded_fields_at_entry;
        }
    }

    RoutineSummary {
        routine_id: routine.id.clone(),
        db_effects,
        in_recursive_cycle: base.in_recursive_cycle,
        has_unresolved_calls,
        uncertainties,
        parameter_roles,
    }
}

fn clone_role(r: &RecordRoleSummary) -> RecordRoleSummary {
    RecordRoleSummary {
        parameter_index: r.parameter_index,
        table_id: r.table_id.clone(),
        reads_fields: r.reads_fields.clone(),
        writes_fields: r.writes_fields.clone(),
        may_reset_filters: r.may_reset_filters,
        may_change_load_fields: r.may_change_load_fields,
        may_assign_record: r.may_assign_record,
        may_use_record_ref: r.may_use_record_ref,
        requires_loaded_at_entry: r.requires_loaded_at_entry,
        required_loaded_fields_at_entry: r.required_loaded_fields_at_entry.clone(),
        mutates_before_load: r.mutates_before_load,
        persists_current_record: r.persists_current_record,
        set_based_db_writes: r.set_based_db_writes,
        validates_param: r.validates_param,
        copies_into_param: r.copies_into_param,
        resets_filters_on_param: r.resets_filters_on_param,
        dirty_at_exit: r.dirty_at_exit,
        current_loaded_fields_at_exit: r.current_loaded_fields_at_exit.clone(),
        mutates_param: r.mutates_param,
        loads_from_db_param: r.loads_from_db_param,
        initialises_param: r.initialises_param,
    }
}

fn uncertainty_key(u: &Uncertainty) -> String {
    if let Some(cs) = &u.callsite_id {
        return format!("{}|{}", u.kind, cs);
    }
    if let Some(op) = &u.operation_id {
        return format!("{}|{}", u.kind, op);
    }
    format!("{}|{}", u.kind, u.routine_id.as_deref().unwrap_or(""))
}

// ---------------------------------------------------------------------------
// Summary fingerprint (internal ids, used for fixed-point change detection).
// ---------------------------------------------------------------------------

/// Internal-id fingerprint for change detection inside the JACOBI loop.
/// Uses the PROJECTED summary's stable-fingerprint function because the
/// routine ids in `PRoutineSummaryCore` are already stable (the stable map
/// is fixed across iterations).
fn summary_fingerprint(s: &PRoutineSummaryCore) -> String {
    stable_summary_fingerprint(s)
}

// ---------------------------------------------------------------------------
// compute_summaries — the main entry point.
// ---------------------------------------------------------------------------

/// Compute RoutineSummaries for all non-leaf routines via the JACOBI fixed-
/// point over the (R3a-1-parity) SCC condensation.
///
/// `upgraded_bindings`: per-callsite side table from the call resolver
/// (`ResolvedCalls.upgraded_bindings`). Needed for cross-call parameterRoles
/// composition. Pass an empty map when not available.
///
/// Returns:
/// - `final_summaries`: internal-id map of ALL computed summaries.
/// - `raw_traces`: per-recursive-SCC trace (empty when `collect_trace=false`).
pub fn compute_summaries(
    routines: &[L3Routine],
    graph: &CombinedGraph,
    scc: &SccResult,
    upgraded_bindings: &HashMap<String, Vec<UpgradedBinding>>,
    collect_trace: bool,
) -> (HashMap<String, RoutineSummary>, Vec<RawSccTrace>) {
    // Build O(1) lookup indexes.
    let routines_by_id: HashMap<String, &L3Routine> =
        routines.iter().map(|r| (r.id.clone(), r)).collect();

    // internal RoutineId → bodyAvailable, for the opaque-callee guards (FIX 2 / FIX 3
    // / the branch-aware walker's `callee.bodyAvailable === false` check).
    let body_avail_by_id: HashMap<String, bool> = routines
        .iter()
        .map(|r| (r.id.clone(), r.body_available))
        .collect();

    // Precompute base intraprocedural summaries ONCE per routine.
    let base_summaries: HashMap<String, RoutineSummary> = routines
        .iter()
        .map(|r| {
            (
                r.id.clone(),
                base_intraprocedural_summary(r, &routines_by_id),
            )
        })
        .collect();

    // Build the stable id map for the trace oracle (used to project intermediate
    // summaries to stable form for fingerprinting).
    let stable_map: HashMap<String, String> = routines
        .iter()
        .map(|r| (r.id.clone(), r.stable_routine_id.clone()))
        .collect();

    let mut final_map: HashMap<String, RoutineSummary> = HashMap::new();
    let mut raw_traces: Vec<RawSccTrace> = Vec::new();

    // Use an empty snapshot for non-recursive SCCs (reads fall through to final_map).
    let empty_snapshot: HashMap<String, RoutineSummary> = HashMap::new();

    for scc_entry in &scc.sccs {
        if !scc_entry.recursive {
            // Non-recursive SCC: single pass.
            let id = match scc_entry.members.first() {
                Some(id) => id,
                None => continue,
            };
            let routine = match routines_by_id.get(id) {
                Some(r) => r,
                None => continue,
            };
            let summary = compose_routine(
                routine,
                &empty_snapshot,
                &final_map,
                &base_summaries,
                upgraded_bindings,
                graph,
                &body_avail_by_id,
            );
            final_map.insert(id.clone(), summary);
            continue;
        }

        // Recursive SCC — JACOBI fixed-point.
        // Seed in_progress with base summaries.
        let mut in_progress: HashMap<String, RoutineSummary> = HashMap::new();
        for id in &scc_entry.members {
            if let Some(base) = base_summaries.get(id) {
                in_progress.insert(id.clone(), base.clone());
            }
        }

        let mut iterations = 0usize;
        let mut changed = true;
        let mut scc_passes: Vec<RawSccTracePass> = Vec::new();

        while changed {
            changed = false;
            iterations += 1;

            // JACOBI: freeze the prior-pass snapshot (deep copy).
            let snapshot: HashMap<String, RoutineSummary> = in_progress.clone();

            // Accumulate this pass's new summaries separately so we don't read
            // our own writes during this pass (JACOBI, not Gauss-Seidel).
            let mut next_pass: HashMap<String, RoutineSummary> = HashMap::new();

            for id in &scc_entry.members {
                let routine = match routines_by_id.get(id) {
                    Some(r) => r,
                    None => continue,
                };
                let next = compose_routine(
                    routine,
                    &snapshot,  // FROZEN: all reads from the prior pass
                    &final_map, // settled summaries for already-processed SCCs
                    &base_summaries,
                    upgraded_bindings,
                    graph,
                    &body_avail_by_id,
                );

                let prev_proj = snapshot
                    .get(id)
                    .map(|s| project_summary_to_stable(id, s, &stable_map));
                let next_proj = project_summary_to_stable(id, &next, &stable_map);

                let fp_prev = prev_proj.as_ref().map(summary_fingerprint);
                let fp_next = summary_fingerprint(&next_proj);

                if fp_prev.as_deref() != Some(&fp_next) {
                    changed = true;
                }
                next_pass.insert(id.clone(), next);
            }

            // Swap: in_progress becomes the new-pass map.
            in_progress = next_pass;

            // Trace hook (opt-in).
            if collect_trace {
                let pass_members: Vec<PRoutineSummaryCore> = scc_entry
                    .members
                    .iter()
                    .filter_map(|id| {
                        in_progress
                            .get(id)
                            .map(|s| project_summary_to_stable(id, s, &stable_map))
                    })
                    .collect();
                scc_passes.push(RawSccTracePass {
                    iteration: iterations,
                    changed,
                    member_summaries: pass_members,
                });
            }

            if iterations >= MAX_FIXED_POINT_ITERATIONS {
                // FIX 4: cap-hit diagnostic — mirrors al-sem summary-runner.ts:486-490
                // (`severity: "warning", stage: "summarize"`). The engine never throws;
                // surface as a warning and continue gracefully. Stable-id members for a
                // deterministic, modelInstanceId-independent message.
                let mut members: Vec<&str> = scc_entry
                    .members
                    .iter()
                    .map(|m| stable_map.get(m).map(|s| s.as_str()).unwrap_or(m.as_str()))
                    .collect();
                members.sort_unstable();
                eprintln!(
                    "warning: summarize: Summary fixed-point did not converge for SCC [{}]",
                    members.join(", ")
                );
                break;
            }
        }

        // Mark all SCC members as inRecursiveCycle=true.
        for id in &scc_entry.members {
            if let Some(s) = in_progress.remove(id) {
                final_map.insert(
                    id.clone(),
                    RoutineSummary {
                        in_recursive_cycle: true,
                        ..s
                    },
                );
            }
        }

        if collect_trace && !scc_passes.is_empty() {
            raw_traces.push(RawSccTrace {
                members: scc_entry.members.clone(),
                passes: scc_passes,
            });
        }
    }

    (final_map, raw_traces)
}

// ---------------------------------------------------------------------------
// Project one internal RoutineSummary to stable form (for the trace oracle).
// ---------------------------------------------------------------------------

fn project_summary_to_stable(
    routine_id: &str,
    s: &RoutineSummary,
    stable_map: &HashMap<String, String>,
) -> PRoutineSummaryCore {
    project_routine_summary_core_internal(routine_id, s, stable_map)
}
