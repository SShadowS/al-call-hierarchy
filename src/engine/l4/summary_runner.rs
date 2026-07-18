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
    EffectPresence, effect_key_of, join_presence, merge_via_owned, via_for_edge_kind,
};
use super::scc::SccResult;
use super::summary::{
    DbEffect, FieldList, PRoutineSummaryCore, RecordRoleSummary, RoutineSummary, TempState,
    Uncertainty, project_routine_summary_core_internal, stable_summary_fingerprint,
};
use crate::engine::l3::call_resolver::UpgradedBinding;
use crate::engine::l3::l3_workspace::L3Routine;

const MAX_FIXED_POINT_ITERATIONS: usize = 1000;

// ---------------------------------------------------------------------------
// Trace hook types.
// ---------------------------------------------------------------------------

/// One raw SCC trace (internal ids).
#[derive(Clone, PartialEq, Eq, salsa::Update)]
pub struct RawSccTrace {
    pub members: Vec<String>,
    pub passes: Vec<RawSccTracePass>,
}

/// One pass in the raw SCC trace.
#[derive(Clone, PartialEq, Eq, salsa::Update)]
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
    fields: &FieldIndex,
) -> RoutineSummary {
    let parameter_roles = compute_record_roles(routine, fields);

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
fn compute_record_roles(routine: &L3Routine, fields: &FieldIndex) -> Vec<RecordRoleSummary> {
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
            if let Some(fid) = resolve_field(&table_id, &fa.field_name, fields) {
                reads_fields.push(fid);
            }
        }

        // Record operations — may-fact bootstrap.
        for op in &routine.record_operations {
            if op.record_variable_name.to_lowercase() != rec_var_name_lc {
                continue;
            }
            if op.op == "Validate"
                && let Some(args) = &op.field_arguments
            {
                for arg in args {
                    if let Some(fid) = resolve_field(&table_id, arg, fields) {
                        writes_fields.push(fid);
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

/// A field-resolution index: `(internal tableId, lowercased field name) →
/// internal FieldId`. Built once per workspace from the resolved tables (and
/// their merged extension fields). Mirrors the case-insensitive
/// `resolveField` lookup al-sem performs against `ctx.tableById` in
/// `src/engine/summary-engine.ts`.
pub type FieldIndex = HashMap<(String, String), String>;

/// Resolve a field name to its internal FieldId by table, case-insensitively.
/// Mirrors al-sem `resolveField` (summary-engine.ts):
///   `table?.fields.find(f => f.name.toLowerCase() === fieldName.toLowerCase())?.id`.
/// Returns None when the table is unresolved (`"unknown"`) or the field is not
/// found on the table.
fn resolve_field(table_id: &str, field_name: &str, fields: &FieldIndex) -> Option<String> {
    if table_id == "unknown" {
        return None;
    }
    fields
        .get(&(table_id.to_string(), field_name.to_lowercase()))
        .cloned()
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
    uncertainty_edges_by_from: &HashMap<String, Vec<usize>>,
) -> RoutineSummary {
    // For non-recursive SCCs `snapshot` is empty; reads fall through to `final_map`.
    let lookup =
        |id: &str| -> Option<&RoutineSummary> { snapshot.get(id).or_else(|| final_map.get(id)) };

    // Every routine has a precomputed base summary, so this fallback is dead;
    // an empty field index is fine for the unreachable branch.
    let base = base_summaries.get(&routine.id).cloned().unwrap_or_else(|| {
        base_intraprocedural_summary(routine, &HashMap::new(), &FieldIndex::new())
    });

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

        // Fold db_effects. A callee effect whose temp_state is
        // ParameterDependent(i) is SUBSTITUTED per-callsite (G5 / RV-7): the
        // callee-frame index `i` is meaningless in the caller's frame, so we
        // resolve it through the caller's argument binding for callee param `i`
        // and fold the effect under a RE-COMPUTED effect_key. Non-PD effects
        // (Known/Unknown) fold unchanged. The re-keying naturally dedupes by
        // (op, tableId, operationId, tempfrag): identical substitution results
        // merge; divergent results (mixed callers) stay DISTINCT as two effects.
        for e in &callee_summary.db_effects {
            let (key_owned, folded): (String, DbEffect) = match &e.temp_state {
                TempState::ParameterDependent(i) => {
                    let sub = substitute_pd_temp_state(edge, *i, routine);
                    let new_key =
                        effect_key_of(&e.op, &e.table_id, &e.operation_id, &sub.to_kind());
                    let folded = DbEffect {
                        effect_key: new_key.clone(),
                        temp_state: sub,
                        via: via.to_string(),
                        ..e.clone()
                    };
                    (new_key, folded)
                }
                _ => (
                    e.effect_key.clone(),
                    DbEffect {
                        via: via.to_string(),
                        ..e.clone()
                    },
                ),
            };
            match db_effects_by_key.get(&key_owned) {
                Some(existing) => {
                    let merged_via = merge_via_owned(&existing.via, &folded.via);
                    let updated = DbEffect {
                        via: merged_via,
                        ..existing.clone()
                    };
                    db_effects_by_key.insert(key_owned, updated);
                }
                None => {
                    db_effects_by_key.insert(key_owned, folded);
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

    // Uncertainty edges (to-less call sites) — indexed lookup by source routine
    // (indices into `graph.uncertainty_edges`, pushed in global order), NOT a
    // linear scan of the whole workspace's uncertainty-edge list per routine.
    if let Some(idxs) = uncertainty_edges_by_from.get(&routine.id) {
        for &i in idxs {
            let ue = &graph.uncertainty_edges[i];
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
        // Built ONCE per routine, not once per parameter: the op/call/fa index is
        // identical across every parameter's walk of the SAME routine.
        let walk_indexes = crate::engine::l4::cfg_walker::build_indexes(routine);
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
                &walk_indexes,
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

/// Substitute a callee effect's `ParameterDependent(callee_param_index)` temp
/// state through the caller's per-callsite argument binding (G5 / RV-7).
///
/// Resolution (all uncertainty → `Unknown`, which FIRES — the sound direction):
///   1. event-dispatch edge (no `callsite_id`) → `Unknown`.
///   2. edge kinds with no binding semantics modeled
///      (`interface | codeunit-run | report-run | page-run | dynamic`) → `Unknown`.
///      Only `direct | method | implicit-trigger` carry usable bindings.
///   3. no binding whose `parameter_index == callee_param_index` → `Unknown`.
///   4. apply the SUBSTITUTION TABLE on the binding's `source_temp_state`:
///
/// ```text
/// Some(Known(true))  → Known(true)
/// Some(Known(false)) → Known(false)
/// Some(PD(j))        → PD(j)   (RE-SYMBOLIZE upward — TASK 8 / RV-7)
/// Some(Unknown)      → Unknown
/// None               → Unknown
/// ```
///
/// SOUNDNESS: only NARROWS symbolic → binding-derived, or RE-SYMBOLIZES a
/// forwarded caller param's PD to the caller's own param index (propagating the
/// symbolic dependency, never inventing it); never yields `Known(true)` unless
/// the binding source is itself `Known(true)`. A PD chasing itself around a
/// recursive cycle stays PD (monotone) and the fixed point converges.
fn substitute_pd_temp_state(
    edge: &super::combined_graph::CombinedEdge,
    callee_param_index: u32,
    routine: &L3Routine,
) -> TempState {
    // (1) event-dispatch / any to-less edge: no caller-frame binding.
    let cs_id = match &edge.callsite_id {
        Some(id) => id,
        None => return TempState::Unknown,
    };
    // (2) only binding-carrying edge kinds substitute. This is intentionally a
    // POSITIVE allowlist: only `direct | method | implicit-trigger` carry usable
    // bindings; ANY other kind — including future edge kinds — falls to Unknown
    // (sound = fires). event-dispatch is already excluded by the `callsite_id:
    // None` guard above.
    if !matches!(edge.kind.as_str(), "direct" | "method" | "implicit-trigger") {
        return TempState::Unknown;
    }
    // Find THIS edge's callsite among the caller's call sites.
    let cs = match routine.call_sites.iter().find(|cs| cs.id == *cs_id) {
        Some(cs) => cs,
        None => return TempState::Unknown,
    };
    // (3) the binding for the callee param the PD refers to.
    let binding = match cs
        .argument_bindings
        .iter()
        .find(|b| b.parameter_index == callee_param_index)
    {
        Some(b) => b,
        None => return TempState::Unknown,
    };
    // (4) substitution table over the binding's captured source temp state.
    //
    // A record-typed PARAMETER is present in the caller's
    // `enclosing_record_variables` at L2, so a forwarded-param arg's binding
    // ALREADY carries `source_temp_state` = that caller param's OWN temp_state
    // (verified — see `extract_record_variables` / `extract_argument_bindings`):
    //   keyword `temporary`  -> Known(true)
    //   keyword-less by-var  -> ParameterDependent(caller_param_index)
    //   by-value             -> Known(false)
    //
    // TASK 8 (RV-7 binding gap): RE-SYMBOLIZE the PD case. When the caller
    // forwards its OWN keyword-less by-var record param onward, the inherited
    // effect's tempness depends on the CALLER's param `j`, not a concrete var —
    // so it becomes `ParameterDependent(j)`, chaining the symbolic dependency
    // UPWARD instead of collapsing to Unknown. The substituted PD index is the
    // CALLER-frame index carried in `source_temp_state` (the binding already
    // re-anchored it from the callee frame to the caller frame at L2).
    //
    // SOUNDNESS: re-symbolizing PD->PD only PROPAGATES a symbolic dependency; it
    // never invents Known(true). A forwarded keyword param yields Known(true)
    // ONLY because its source param IS Known(true). Around a recursive cycle a
    // PD chasing itself stays PD (monotone) and the fixed point converges — the
    // effect_key includes the PD index, so the state space stays finite.
    match &binding.source_temp_state {
        Some(ts) => match TempState::from_p(ts) {
            TempState::Known(v) => TempState::Known(v),
            // Caller's-own-param source (forwarded keyword-less by-var param):
            // re-symbolize to the caller's own param index (chains upward).
            TempState::ParameterDependent(j) => TempState::ParameterDependent(j),
            // Genuinely unknown source → Unknown (conservative = fires).
            TempState::Unknown => TempState::Unknown,
        },
        // No captured source temp state (arg is not a record var/param the
        // caller declares — e.g. an implicit-rec or unresolved name): Unknown.
        None => TempState::Unknown,
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
/// - `cap_diagnostics`: summarize-stage diagnostics (JACOBI cap-hit); empty
///   unless some SCC failed to converge within `MAX_FIXED_POINT_ITERATIONS`.
#[allow(clippy::too_many_arguments)]
pub fn compute_summaries(
    routines: &[L3Routine],
    graph: &CombinedGraph,
    scc: &SccResult,
    upgraded_bindings: &HashMap<String, Vec<UpgradedBinding>>,
    fields: &FieldIndex,
    collect_trace: bool,
) -> (
    HashMap<String, RoutineSummary>,
    Vec<RawSccTrace>,
    Vec<SummarizeDiagnostic>,
) {
    let no_leaves: HashMap<String, RoutineSummary> = HashMap::new();
    compute_summaries_with_leaves(
        routines,
        graph,
        scc,
        upgraded_bindings,
        fields,
        collect_trace,
        &no_leaves,
    )
}

/// Like [`compute_summaries`], but treats every routine id present in
/// `leaf_summaries` as a FIXED LEAF carrying that pre-computed summary (al-sem's
/// `isLeaf(r) = r.summary !== undefined` default). Leaves are pre-seeded into the
/// final map and NEVER recomputed; non-leaf callers fold the leaf summaries in via
/// the combined graph. This is the R3a-5 seam: dependency routines arrive with a
/// RETAINED summary (their own `via:"direct"` dbEffects) and must propagate to
/// primary callers without being re-derived from their EMPTY merged features.
/// Mirrors al-sem `computeSummaries` (`src/engine/summary-runner.ts:400-505`).
pub fn compute_summaries_with_leaves(
    routines: &[L3Routine],
    graph: &CombinedGraph,
    scc: &SccResult,
    upgraded_bindings: &HashMap<String, Vec<UpgradedBinding>>,
    fields: &FieldIndex,
    collect_trace: bool,
    leaf_summaries: &HashMap<String, RoutineSummary>,
) -> (
    HashMap<String, RoutineSummary>,
    Vec<RawSccTrace>,
    Vec<SummarizeDiagnostic>,
) {
    // Build O(1) lookup indexes.
    let routines_by_id: HashMap<String, &L3Routine> =
        routines.iter().map(|r| (r.id.clone(), r)).collect();

    // internal RoutineId → bodyAvailable, for the opaque-callee guards (FIX 2 / FIX 3
    // / the branch-aware walker's `callee.bodyAvailable === false` check).
    let body_avail_by_id: HashMap<String, bool> = routines
        .iter()
        .map(|r| (r.id.clone(), r.body_available))
        .collect();

    // Precompute base intraprocedural summaries ONCE per NON-LEAF routine. A leaf
    // already carries its summary; al-sem skips `baseIntraproceduralSummaryCtx` for
    // it (summary-runner.ts:400-403). We skip it too so a leaf's EMPTY merged
    // features never overwrite its retained summary.
    let base_summaries: HashMap<String, RoutineSummary> = routines
        .iter()
        .filter(|r| !leaf_summaries.contains_key(&r.id))
        .map(|r| {
            (
                r.id.clone(),
                base_intraprocedural_summary(r, &routines_by_id, fields),
            )
        })
        .collect();

    // Build the stable id map for the trace oracle (used to project intermediate
    // summaries to stable form for fingerprinting).
    let stable_map: HashMap<String, String> = routines
        .iter()
        .map(|r| (r.id.clone(), r.stable_routine_id.clone()))
        .collect();

    // Index the global uncertainty-edge list by source routine, preserving the
    // GLOBAL list order per source (indices into graph.uncertainty_edges) so the
    // per-routine iteration below sees the same sequence the linear scan saw.
    let mut uncertainty_edges_by_from: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, ue) in graph.uncertainty_edges.iter().enumerate() {
        uncertainty_edges_by_from
            .entry(ue.from.clone())
            .or_default()
            .push(i);
    }

    let mut final_map: HashMap<String, RoutineSummary> = HashMap::new();
    let mut raw_traces: Vec<RawSccTrace> = Vec::new();
    let mut cap_diagnostics: Vec<SummarizeDiagnostic> = Vec::new();

    // Pre-seed FIXED LEAVES (routines that arrive with a retained summary) so
    // composition can look them up; they are never recomputed. (al-sem
    // summary-runner.ts:408-410.)
    for (id, summary) in leaf_summaries {
        final_map.insert(id.clone(), summary.clone());
    }

    for scc_entry in &scc.sccs {
        let out = run_one_scc(
            scc_entry,
            &final_map,
            &SccComputeCtx {
                routines_by_id: &routines_by_id,
                base_summaries: &base_summaries,
                upgraded_bindings,
                graph,
                body_avail_by_id: &body_avail_by_id,
                stable_map: &stable_map,
                leaf_summaries,
                uncertainty_edges_by_from: &uncertainty_edges_by_from,
            },
            collect_trace,
        );
        for (id, s) in out.summaries {
            final_map.insert(id, s);
        }
        if let Some(trace) = out.trace {
            raw_traces.push(trace);
        }
        cap_diagnostics.extend(out.cap_diagnostics);
    }

    (final_map, raw_traces, cap_diagnostics)
}

/// The SHARED per-SCC compute context — the workspace-wide lookup structures the
/// JACOBI loop reads (all keyed by internal RoutineId, so a single SCC's loop
/// reads only the entries it needs). Both the from-scratch
/// [`compute_summaries_with_leaves`] AND the R3b Salsa `scc_summaries` query call
/// `run_one_scc` with this context, so the JACOBI fixed point is the SAME proven
/// code on both paths (no re-port).
pub struct SccComputeCtx<'a> {
    pub routines_by_id: &'a HashMap<String, &'a L3Routine>,
    pub base_summaries: &'a HashMap<String, RoutineSummary>,
    pub upgraded_bindings: &'a HashMap<String, Vec<UpgradedBinding>>,
    pub graph: &'a CombinedGraph,
    pub body_avail_by_id: &'a HashMap<String, bool>,
    pub stable_map: &'a HashMap<String, String>,
    pub leaf_summaries: &'a HashMap<String, RoutineSummary>,
    /// `graph.uncertainty_edges` indexed by source routine id (indices into
    /// `graph.uncertainty_edges`, pushed in GLOBAL order) — lets `compose_routine`
    /// look up "this routine's uncertainty edges" in O(1) instead of scanning the
    /// whole workspace list per routine. Same edges, same order, byte-identical.
    pub uncertainty_edges_by_from: &'a HashMap<String, Vec<usize>>,
}

/// A diagnostic surfaced by the L4 summarize stage — presently just the JACOBI
/// fixed-point cap-hit (below). Structurally identical to
/// `root_classification::InfraDiagnostic` / `l5::registry::Diagnostic`
/// (severity/stage/message), the shape every engine layer uses for its own
/// diagnostics; kept local to `l4` rather than importing `l5::registry::Diagnostic`
/// so this module does not gain an upward dependency. `gate/run.rs` converts it
/// into the shared `Diagnostic` at the TS-order "summarizeDiagnostics" slot
/// exactly like it already does for `InfraDiagnostic` at the "overlay" slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SummarizeDiagnostic {
    pub severity: String,
    pub stage: String,
    pub message: String,
}

/// The result of computing one SCC: its members' settled summaries (to fold into
/// the caller's `final_map`) + the optional per-recursive-SCC fingerprint trace +
/// any summarize-stage diagnostics (cap-hit). `cap_diagnostics` is empty on every
/// SCC that converges within `MAX_FIXED_POINT_ITERATIONS` — the overwhelmingly
/// common case — so this field is purely additive.
pub struct SccComputeOut {
    pub summaries: Vec<(String, RoutineSummary)>,
    pub trace: Option<RawSccTrace>,
    pub cap_diagnostics: Vec<SummarizeDiagnostic>,
}

/// Compute ONE SCC's settled summaries, given `predecessor_final_map` = the
/// already-settled summaries of every SCC topologically AFTER this one (the
/// condensation successors / callees, which Tarjan emits FIRST) plus any fixed
/// leaves. Runs the PROVEN JACOBI fixed point for a recursive SCC, or a single
/// `compose_routine` pass for a non-recursive one — the EXACT logic the
/// from-scratch loop ran (extracted verbatim so the golden is untouched).
///
/// This is the R3b incrementality seam: the Salsa `scc_summaries(scc_key)` query
/// builds `predecessor_final_map` from its successor SCCs' `scc_summaries` and
/// calls this — so the intra-SCC fixed point depends only on `scc_members` +
/// successor summaries + the members' inputs, NOT on a monolithic condensation.
pub fn run_one_scc(
    scc_entry: &super::scc::Scc,
    predecessor_final_map: &HashMap<String, RoutineSummary>,
    ctx: &SccComputeCtx,
    collect_trace: bool,
) -> SccComputeOut {
    let leaf_summaries = ctx.leaf_summaries;

    if !scc_entry.recursive {
        // Non-recursive SCC: single pass.
        let id = match scc_entry.members.first() {
            Some(id) => id,
            None => {
                return SccComputeOut {
                    summaries: Vec::new(),
                    trace: None,
                    cap_diagnostics: Vec::new(),
                };
            }
        };
        // Fixed leaf — already in the predecessor map, never recomputed.
        if leaf_summaries.contains_key(id) {
            return SccComputeOut {
                summaries: Vec::new(),
                trace: None,
                cap_diagnostics: Vec::new(),
            };
        }
        let routine = match ctx.routines_by_id.get(id) {
            Some(r) => r,
            None => {
                return SccComputeOut {
                    summaries: Vec::new(),
                    trace: None,
                    cap_diagnostics: Vec::new(),
                };
            }
        };
        let empty_snapshot: HashMap<String, RoutineSummary> = HashMap::new();
        let summary = compose_routine(
            routine,
            &empty_snapshot,
            predecessor_final_map,
            ctx.base_summaries,
            ctx.upgraded_bindings,
            ctx.graph,
            ctx.body_avail_by_id,
            ctx.uncertainty_edges_by_from,
        );
        return SccComputeOut {
            summaries: vec![(id.clone(), summary)],
            trace: None,
            cap_diagnostics: Vec::new(),
        };
    }

    // Recursive SCC — JACOBI fixed-point.
    // Seed in_progress with base summaries (leaves are excluded: they have no
    // base entry and are read from the predecessor map).
    let mut in_progress: HashMap<String, RoutineSummary> = HashMap::new();
    for id in &scc_entry.members {
        if leaf_summaries.contains_key(id) {
            continue;
        }
        if let Some(base) = ctx.base_summaries.get(id) {
            in_progress.insert(id.clone(), base.clone());
        }
    }

    let mut iterations = 0usize;
    let mut changed = true;
    let mut scc_passes: Vec<RawSccTracePass> = Vec::new();
    // Set on cap-hit (below); drives BOTH the returned diagnostic and the
    // per-member `Uncertainty` marker so the partial summaries this SCC ships
    // are never silently definite.
    let mut cap_hit_stable_members: Option<Vec<String>> = None;

    while changed {
        changed = false;
        iterations += 1;

        // JACOBI: freeze the prior-pass snapshot (deep copy).
        let __probe_t = std::time::Instant::now();
        let snapshot: HashMap<String, RoutineSummary> = in_progress.clone();
        crate::stage_probe::accum(crate::stage_probe::ACC_JACOBI_CLONE, __probe_t.elapsed());

        // Accumulate this pass's new summaries separately so we don't read
        // our own writes during this pass (JACOBI, not Gauss-Seidel).
        let mut next_pass: HashMap<String, RoutineSummary> = HashMap::new();

        for id in &scc_entry.members {
            if leaf_summaries.contains_key(id) {
                continue;
            }
            let routine = match ctx.routines_by_id.get(id) {
                Some(r) => r,
                None => continue,
            };
            let __probe_t = std::time::Instant::now();
            let next = compose_routine(
                routine,
                &snapshot,             // FROZEN: all reads from the prior pass
                predecessor_final_map, // settled summaries for already-processed SCCs
                ctx.base_summaries,
                ctx.upgraded_bindings,
                ctx.graph,
                ctx.body_avail_by_id,
                ctx.uncertainty_edges_by_from,
            );
            crate::stage_probe::accum(crate::stage_probe::ACC_JACOBI_COMPOSE, __probe_t.elapsed());

            let __probe_t = std::time::Instant::now();
            let prev_proj = snapshot
                .get(id)
                .map(|s| project_summary_to_stable(id, s, ctx.stable_map));
            let next_proj = project_summary_to_stable(id, &next, ctx.stable_map);

            let fp_prev = prev_proj.as_ref().map(summary_fingerprint);
            let fp_next = summary_fingerprint(&next_proj);
            crate::stage_probe::accum(crate::stage_probe::ACC_JACOBI_FP, __probe_t.elapsed());

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
                        .map(|s| project_summary_to_stable(id, s, ctx.stable_map))
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
                .map(|m| {
                    ctx.stable_map
                        .get(m)
                        .map(|s| s.as_str())
                        .unwrap_or(m.as_str())
                })
                .collect();
            members.sort_unstable();
            eprintln!(
                "warning: summarize: Summary fixed-point did not converge for SCC [{}]",
                members.join(", ")
            );
            cap_hit_stable_members = Some(members.into_iter().map(str::to_string).collect());
            break;
        }
    }

    // Mark all SCC members as inRecursiveCycle=true. On a cap-hit, ALSO attach a
    // `fixpoint-capped` Uncertainty to every member: the transfer function is
    // non-monotone (via `apply_call`), so a capped SCC's summaries are partial
    // facts, not settled ones — they must never ship as silently definite (the
    // honesty bar this fix closes). This is the SAME `Uncertainty` mechanism
    // detectors already read via `uncertainties_by_node` (no parallel channel).
    let mut out_summaries: Vec<(String, RoutineSummary)> = Vec::new();
    for id in &scc_entry.members {
        if let Some(mut s) = in_progress.remove(id) {
            if cap_hit_stable_members.is_some() {
                let stable_id = ctx
                    .stable_map
                    .get(id)
                    .cloned()
                    .unwrap_or_else(|| id.clone());
                s.uncertainties.push(Uncertainty {
                    kind: "fixpoint-capped".to_string(),
                    callsite_id: None,
                    operation_id: None,
                    routine_id: Some(stable_id),
                    interface_name: None,
                });
            }
            out_summaries.push((
                id.clone(),
                RoutineSummary {
                    in_recursive_cycle: true,
                    ..s
                },
            ));
        }
    }

    let trace = if collect_trace && !scc_passes.is_empty() {
        Some(RawSccTrace {
            members: scc_entry.members.clone(),
            passes: scc_passes,
        })
    } else {
        None
    };

    let cap_diagnostics = match cap_hit_stable_members {
        Some(members) => vec![SummarizeDiagnostic {
            severity: "warning".to_string(),
            stage: "summarize".to_string(),
            message: format!(
                "Summary fixed-point did not converge for SCC [{}]; its facts are lower-confidence",
                members.join(", ")
            ),
        }],
        None => Vec::new(),
    };

    SccComputeOut {
        summaries: out_summaries,
        trace,
        cap_diagnostics,
    }
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
