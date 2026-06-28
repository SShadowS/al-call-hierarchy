//! L4 summary core types + projection (R3a-2).
//!
//! Ports al-sem's `src/model/summary.ts` (RoutineSummary / DbEffect /
//! Uncertainty / RecordRoleSummary) and `scripts/r3a2-projection.ts`
//! (projectR3a2 / projectR3a2Trace / stable-id mapping).
//!
//! HARD-FORBIDDEN in the R3a-2 projection: `capabilityFactsDirect` /
//! `capabilityFactsInherited` / `coverage` (R3a-3 cone), `fieldEffects`
//! (lazy/detector), the dep-hook output (R3a-4). These are never declared
//! on the projected types so they cannot appear.

use serde::{Deserialize, Serialize};

use super::effect_lattice::{EffectPresence, TempStateKind, effect_key_of};
use super::summary_runner::compute_summaries;
use crate::engine::l3::call_resolver::{DeclaredDependency, resolve_calls};
use crate::engine::l3::event_graph::build_event_graph;
use crate::engine::l3::l3_workspace::L3Resolved;
use crate::engine::l3::symbol_table::SymbolTable;
use crate::engine::l4::combined_graph::build_combined_graph;
use crate::engine::l4::scc::{SccInputGraph, tarjan_scc};

// ---------------------------------------------------------------------------
// Internal summary core types (NOT the serde projection shape). Internal ids.
// ---------------------------------------------------------------------------

/// The temp-state of a record operation (internal form — NOT the serde
/// projection shape). Mirrors al-sem `TempState`.
#[derive(Debug, Clone, PartialEq, Eq, salsa::Update)]
pub enum TempState {
    Known(bool),
    ParameterDependent(u32),
    Unknown,
}

impl TempState {
    pub fn from_p(ts: &crate::engine::l2::features::PTempState) -> Self {
        match ts.kind.as_str() {
            "known" => TempState::Known(ts.value.unwrap_or(false)),
            "parameter-dependent" => TempState::ParameterDependent(ts.parameter_index.unwrap_or(0)),
            _ => TempState::Unknown,
        }
    }

    pub fn to_kind(&self) -> TempStateKind {
        match self {
            TempState::Known(v) => TempStateKind::Known(*v),
            TempState::ParameterDependent(i) => TempStateKind::ParameterDependent(*i),
            TempState::Unknown => TempStateKind::Unknown,
        }
    }
}

/// One de-duplicated DB effect (internal form). Mirrors al-sem `DbEffect`.
#[derive(Debug, Clone, PartialEq, Eq, salsa::Update)]
pub struct DbEffect {
    /// `effectKeyOf(op, tableId, operationId, tempState)` — EXCLUDES via.
    pub effect_key: String,
    pub operation_id: String,
    pub op: String,
    pub table_id: String, // "unknown" or internal form
    pub record_variable_id: Option<String>,
    pub temp_state: TempState,
    /// "direct" | "inherited" | "implicit-trigger" | "event-subscriber" | "dynamic"
    pub via: String,
}

/// One uncertainty (internal form). Mirrors al-sem `Uncertainty`.
#[derive(Debug, Clone, PartialEq, Eq, salsa::Update)]
pub struct Uncertainty {
    pub kind: String,
    pub callsite_id: Option<String>,
    pub operation_id: Option<String>,
    pub routine_id: Option<String>,
    pub interface_name: Option<String>,
}

/// The combined-graph `UncertaintyEdge` carries its own structurally-identical
/// `Uncertainty` (`combined_graph::Uncertainty`). They are the SAME al-sem
/// `Uncertainty` shape modelled in two modules (the L4 summary core vs. the
/// to-less combined-graph edge). This converts an edge uncertainty into the
/// summary form so the path-walker (which consumes `summary::Uncertainty`) can
/// union the summary-carried and edge-carried sources with one type. Field-for-
/// field copy — no information is lost or gained.
impl From<&crate::engine::l4::combined_graph::Uncertainty> for Uncertainty {
    fn from(u: &crate::engine::l4::combined_graph::Uncertainty) -> Self {
        Uncertainty {
            kind: u.kind.clone(),
            callsite_id: u.callsite_id.clone(),
            operation_id: u.operation_id.clone(),
            routine_id: u.routine_id.clone(),
            interface_name: u.interface_name.clone(),
        }
    }
}

/// Stable key for an Uncertainty — mirrors al-sem `uncertaintyKey`.
pub fn uncertainty_key(u: &Uncertainty) -> String {
    if let Some(cs) = &u.callsite_id {
        return format!("{}|{}", u.kind, cs);
    }
    if let Some(op) = &u.operation_id {
        return format!("{}|{}", u.kind, op);
    }
    format!("{}|{}", u.kind, u.routine_id.as_deref().unwrap_or(""))
}

/// De-duplicate a list of [`Uncertainty`] values by key, then sort by key. Mirrors
/// al-sem `dedupeUncertainties` (uncertainty-util.ts) EXACTLY: al-sem builds a JS
/// `Map` with `byKey.set(key, u)` in order — so on a key collision the LAST value
/// wins — then emits `[...byKey.values()].sort(byKey)`. A `BTreeMap` reproduces both:
/// `insert` is last-write-wins and iteration is key-sorted (byte order == al-sem's
/// ASCII-key `compareStrings`). (Keep-first would diverge only for same-key
/// `interface-open-world` uncertainties with differing `interfaceName`, but matching
/// keep-last removes the reliance on that one-interface-per-callsite invariant.)
pub(crate) fn dedupe_uncertainties(list: Vec<Uncertainty>) -> Vec<Uncertainty> {
    use std::collections::BTreeMap;
    let mut seen: BTreeMap<String, Uncertainty> = BTreeMap::new();
    for u in list {
        seen.insert(uncertainty_key(&u), u); // last-write-wins, matching JS Map.set
    }
    seen.into_values().collect() // BTreeMap iterates keys in sorted order
}

/// Per-record-parameter role summary (internal form). Mirrors al-sem
/// `RecordRoleSummary`.
#[derive(Debug, Clone, PartialEq, Eq, salsa::Update)]
pub struct RecordRoleSummary {
    pub parameter_index: u32,
    pub table_id: String,
    pub reads_fields: FieldList,
    pub writes_fields: FieldList,
    pub may_reset_filters: bool,
    pub may_change_load_fields: bool,
    pub may_assign_record: bool,
    pub may_use_record_ref: bool,
    // Entry requirements
    pub requires_loaded_at_entry: EffectPresence,
    pub required_loaded_fields_at_entry: FieldList,
    pub mutates_before_load: EffectPresence,
    // Exit effects
    pub persists_current_record: EffectPresence,
    pub set_based_db_writes: EffectPresence,
    pub validates_param: EffectPresence,
    pub copies_into_param: EffectPresence,
    pub resets_filters_on_param: EffectPresence,
    pub dirty_at_exit: EffectPresence,
    pub current_loaded_fields_at_exit: FieldList,
    // Convenience derivations
    pub mutates_param: EffectPresence,
    pub loads_from_db_param: EffectPresence,
    pub initialises_param: EffectPresence,
}

/// A field list value: a sorted list of field ids, or a sentinel.
#[derive(Debug, Clone, PartialEq, Eq, salsa::Update)]
pub enum FieldList {
    Known(Vec<String>),
    Unknown,
    Full,
}

/// One routine summary core (internal form). Mirrors al-sem `RoutineSummary`
/// (CORE fields only — capabilityFacts/coverage/fieldEffects excluded).
#[derive(Debug, Clone, PartialEq, Eq, salsa::Update)]
pub struct RoutineSummary {
    pub routine_id: String,
    pub db_effects: Vec<DbEffect>,
    pub in_recursive_cycle: bool,
    pub has_unresolved_calls: bool,
    pub uncertainties: Vec<Uncertainty>,
    pub parameter_roles: Vec<RecordRoleSummary>,
}

// ---------------------------------------------------------------------------
// Projected (stable-id) types — the R3a-2 comparison surface.
// Matches scripts/r3a2-projection.ts field-for-field.
// ---------------------------------------------------------------------------

/// Projected TempState (in stable-id serialization form).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind")]
pub enum PDbEffectTempState {
    #[serde(rename = "known")]
    Known { value: bool },
    #[serde(rename = "parameter-dependent")]
    ParameterDependent {
        #[serde(rename = "parameterIndex")]
        parameter_index: u32,
    },
    #[serde(rename = "unknown")]
    Unknown,
}

impl PDbEffectTempState {
    fn from_temp_state(ts: &TempState) -> Self {
        match ts {
            TempState::Known(v) => PDbEffectTempState::Known { value: *v },
            TempState::ParameterDependent(i) => PDbEffectTempState::ParameterDependent {
                parameter_index: *i,
            },
            TempState::Unknown => PDbEffectTempState::Unknown,
        }
    }
}

/// Projected field list (stable-id form).
pub type PFieldList = serde_json::Value;

/// Projected DbEffect (stable-id form).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PDbEffect {
    #[serde(rename = "effectKey")]
    pub effect_key: String,
    pub op: String,
    #[serde(rename = "tableId")]
    pub table_id: String,
    #[serde(rename = "operationId")]
    pub operation_id: String,
    #[serde(rename = "tempState")]
    pub temp_state: PDbEffectTempState,
    pub via: String,
}

/// Projected Uncertainty (stable-id form).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PUncertainty {
    pub kind: String,
    #[serde(rename = "callsiteId", skip_serializing_if = "Option::is_none")]
    pub callsite_id: Option<String>,
    #[serde(rename = "operationId", skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
    #[serde(rename = "routineId", skip_serializing_if = "Option::is_none")]
    pub routine_id: Option<String>,
    #[serde(rename = "interfaceName", skip_serializing_if = "Option::is_none")]
    pub interface_name: Option<String>,
}

/// Stable key for a projected Uncertainty.
pub fn p_uncertainty_key(u: &PUncertainty) -> String {
    if let Some(cs) = &u.callsite_id {
        return format!("{}|{}", u.kind, cs);
    }
    if let Some(op) = &u.operation_id {
        return format!("{}|{}", u.kind, op);
    }
    format!("{}|{}", u.kind, u.routine_id.as_deref().unwrap_or(""))
}

/// Projected RecordRoleSummary (stable-id form).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PRecordRoleSummary {
    #[serde(rename = "parameterIndex")]
    pub parameter_index: u32,
    #[serde(rename = "tableId")]
    pub table_id: String,
    #[serde(rename = "readsFields")]
    pub reads_fields: PFieldList,
    #[serde(rename = "writesFields")]
    pub writes_fields: PFieldList,
    #[serde(rename = "mayResetFilters")]
    pub may_reset_filters: bool,
    #[serde(rename = "mayChangeLoadFields")]
    pub may_change_load_fields: bool,
    #[serde(rename = "mayAssignRecord")]
    pub may_assign_record: bool,
    #[serde(rename = "mayUseRecordRef")]
    pub may_use_record_ref: bool,
    #[serde(rename = "requiresLoadedAtEntry")]
    pub requires_loaded_at_entry: String,
    #[serde(rename = "requiredLoadedFieldsAtEntry")]
    pub required_loaded_fields_at_entry: PFieldList,
    #[serde(rename = "mutatesBeforeLoad")]
    pub mutates_before_load: String,
    #[serde(rename = "persistsCurrentRecord")]
    pub persists_current_record: String,
    #[serde(rename = "setBasedDbWrites")]
    pub set_based_db_writes: String,
    #[serde(rename = "validatesParam")]
    pub validates_param: String,
    #[serde(rename = "copiesIntoParam")]
    pub copies_into_param: String,
    #[serde(rename = "resetsFiltersOnParam")]
    pub resets_filters_on_param: String,
    #[serde(rename = "dirtyAtExit")]
    pub dirty_at_exit: String,
    #[serde(rename = "currentLoadedFieldsAtExit")]
    pub current_loaded_fields_at_exit: PFieldList,
    #[serde(rename = "mutatesParam")]
    pub mutates_param: String,
    #[serde(rename = "loadsFromDbParam")]
    pub loads_from_db_param: String,
    #[serde(rename = "initialisesParam")]
    pub initialises_param: String,
}

/// Projected RoutineSummary CORE (stable-id form).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, salsa::Update)]
pub struct PRoutineSummaryCore {
    #[serde(rename = "routineId")]
    pub routine_id: String,
    #[serde(rename = "dbEffects")]
    pub db_effects: Vec<PDbEffect>,
    pub uncertainties: Vec<PUncertainty>,
    #[serde(rename = "parameterRoles")]
    pub parameter_roles: Vec<PRecordRoleSummary>,
    #[serde(rename = "inRecursiveCycle")]
    pub in_recursive_cycle: bool,
    #[serde(rename = "hasUnresolvedCalls")]
    pub has_unresolved_calls: bool,
}

/// The R3a-2 stable projection of the post-computeSummaries model.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct R3a2Projection {
    pub summaries: Vec<PRoutineSummaryCore>,
}

// ---------------------------------------------------------------------------
// Fingerprint TRACE types (R3a-2 JACOBI proof, Rev 2 #3).
// ---------------------------------------------------------------------------

/// One projected fixed-point pass of a recursive SCC.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PSccTracePass {
    pub iteration: usize,
    pub changed: bool,
    /// Stable per-pass fingerprint — sorted by stable routineId.
    pub fingerprint: String,
}

/// The full per-recursive-SCC trace.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PSccTrace {
    #[serde(rename = "sccId")]
    pub scc_id: String,
    pub members: Vec<String>,
    pub iterations: usize,
    pub passes: Vec<PSccTracePass>,
}

/// The R3a-2 fingerprint trace — one PSccTrace per recursive SCC.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct R3a2Trace {
    pub traces: Vec<PSccTrace>,
}

// ---------------------------------------------------------------------------
// Stable-id mapping helpers. Mirror scripts/r3a2-projection.ts exactly.
// ---------------------------------------------------------------------------

/// Build internal-RoutineId → StableRoutineId from the workspace routines.
pub(crate) fn build_routine_stable_map(
    routines: &[crate::engine::l3::l3_workspace::L3Routine],
) -> std::collections::HashMap<String, String> {
    let mut m = std::collections::HashMap::new();
    for r in routines {
        m.insert(r.id.clone(), r.stable_routine_id.clone());
    }
    m
}

/// Convert an internal RoutineId to StableRoutineId; pass through if unmapped.
pub(crate) fn stable_routine_id(
    internal: &str,
    map: &std::collections::HashMap<String, String>,
) -> String {
    map.get(internal)
        .cloned()
        .unwrap_or_else(|| internal.to_string())
}

/// Rewrite `${routineId}/<suffix>` (callsiteId `/csN`, operationId `/opN`) into
/// stable form. Internal RoutineId is `${modelInstanceId}/${hash}` (exactly two
/// `/`-separated parts), so the suffix is everything after the SECOND `/`.
/// Mirrors `stableSubId` in r3a2-projection.ts.
pub(crate) fn stable_sub_id(
    internal_sub_id: &str,
    map: &std::collections::HashMap<String, String>,
) -> String {
    // The internal sub-id looks like `<modelInstanceId>/<hash>/<suffix>`.
    // We need to split off the first two `/`-parts as the routineId.
    let first_slash = internal_sub_id.find('/');
    let second_slash =
        first_slash.and_then(|f| internal_sub_id[f + 1..].find('/').map(|s| f + 1 + s));
    match (first_slash, second_slash) {
        (Some(_), Some(sec)) => {
            let routine_id = &internal_sub_id[..sec];
            let suffix = &internal_sub_id[sec..]; // includes leading "/"
            match map.get(routine_id) {
                Some(stable) => format!("{stable}{suffix}"),
                None => internal_sub_id.to_string(),
            }
        }
        _ => internal_sub_id.to_string(),
    }
}

/// Project an internal TableId to stable form.
/// Internal: `${appGuid}/table/${N}` → `${appGuid}:Table:${N}`.
/// `"unknown"` passes through.
fn stable_table_id(internal: &str) -> String {
    if internal == "unknown" {
        return "unknown".to_string();
    }
    let parts: Vec<&str> = internal.split('/').collect();
    if parts.len() == 3 && parts[1] == "table" {
        format!("{}:Table:{}", parts[0], parts[2])
    } else {
        internal.to_string()
    }
}

/// Project an internal FieldId to stable form. Mirrors al-sem
/// `toStableFieldId` (src/model/stable-identity.ts): the internal FieldId is
/// `${tableId}/${fieldNumber}` (e.g. `${appGuid}/table/${N}/${M}`); split on
/// the LAST slash into the internal TableId + the field number, then convert
/// the table id to stable form: `${stableTableId}#${fieldNumber}`.
fn stable_field_id(internal: &str) -> String {
    match internal.rfind('/') {
        Some(last_slash) if last_slash > 0 => {
            let table_internal = &internal[..last_slash];
            let field_num = &internal[last_slash + 1..];
            format!("{}#{}", stable_table_id(table_internal), field_num)
        }
        _ => internal.to_string(),
    }
}

fn project_field_list_id(fl: &FieldList) -> PFieldList {
    match fl {
        FieldList::Unknown => serde_json::Value::String("unknown".to_string()),
        FieldList::Full => serde_json::Value::String("full".to_string()),
        FieldList::Known(fields) => {
            let mut stable: Vec<String> = fields.iter().map(|f| stable_field_id(f)).collect();
            stable.sort();
            serde_json::Value::Array(stable.into_iter().map(serde_json::Value::String).collect())
        }
    }
}

/// Project a field-name list (requiredLoadedFieldsAtEntry /
/// currentLoadedFieldsAtExit) — these are opaque strings, keep order.
fn project_field_name_list(fl: &FieldList) -> PFieldList {
    match fl {
        FieldList::Unknown => serde_json::Value::String("unknown".to_string()),
        FieldList::Full => serde_json::Value::String("full".to_string()),
        FieldList::Known(names) => serde_json::Value::Array(
            names
                .iter()
                .map(|n| serde_json::Value::String(n.clone()))
                .collect(),
        ),
    }
}

// ---------------------------------------------------------------------------
// Per-routine projection helpers.
// ---------------------------------------------------------------------------

fn project_db_effect(e: &DbEffect, map: &std::collections::HashMap<String, String>) -> PDbEffect {
    let table_id = stable_table_id(&e.table_id);
    let operation_id = stable_sub_id(&e.operation_id, map);
    let temp_state = e.temp_state.to_kind();
    let effect_key = effect_key_of(&e.op, &table_id, &operation_id, &temp_state);
    PDbEffect {
        effect_key,
        op: e.op.clone(),
        table_id,
        operation_id,
        temp_state: PDbEffectTempState::from_temp_state(&e.temp_state),
        via: e.via.clone(),
    }
}

fn project_uncertainty(
    u: &Uncertainty,
    map: &std::collections::HashMap<String, String>,
) -> PUncertainty {
    PUncertainty {
        kind: u.kind.clone(),
        callsite_id: u.callsite_id.as_ref().map(|c| stable_sub_id(c, map)),
        operation_id: u.operation_id.as_ref().map(|o| stable_sub_id(o, map)),
        routine_id: u.routine_id.as_ref().map(|r| stable_routine_id(r, map)),
        interface_name: u.interface_name.clone(),
    }
}

fn project_record_role(r: &RecordRoleSummary) -> PRecordRoleSummary {
    PRecordRoleSummary {
        parameter_index: r.parameter_index,
        table_id: stable_table_id(&r.table_id),
        reads_fields: project_field_list_id(&r.reads_fields),
        writes_fields: project_field_list_id(&r.writes_fields),
        may_reset_filters: r.may_reset_filters,
        may_change_load_fields: r.may_change_load_fields,
        may_assign_record: r.may_assign_record,
        may_use_record_ref: r.may_use_record_ref,
        requires_loaded_at_entry: r.requires_loaded_at_entry.as_str().to_string(),
        required_loaded_fields_at_entry: project_field_name_list(
            &r.required_loaded_fields_at_entry,
        ),
        mutates_before_load: r.mutates_before_load.as_str().to_string(),
        persists_current_record: r.persists_current_record.as_str().to_string(),
        set_based_db_writes: r.set_based_db_writes.as_str().to_string(),
        validates_param: r.validates_param.as_str().to_string(),
        copies_into_param: r.copies_into_param.as_str().to_string(),
        resets_filters_on_param: r.resets_filters_on_param.as_str().to_string(),
        dirty_at_exit: r.dirty_at_exit.as_str().to_string(),
        current_loaded_fields_at_exit: project_field_name_list(&r.current_loaded_fields_at_exit),
        mutates_param: r.mutates_param.as_str().to_string(),
        loads_from_db_param: r.loads_from_db_param.as_str().to_string(),
        initialises_param: r.initialises_param.as_str().to_string(),
    }
}

/// Public projector for one internal RoutineSummary CORE → the stable R3a-2 shape.
/// Used by the R3a-5 cross-app full-summary projection (which composes the R3a-2
/// core with the R3a-3 cone over the MERGED model). The `map` covers BOTH primary
/// and dep routine ids (every merged L3Routine carries `stable_routine_id`).
pub fn project_routine_summary_core_pub(
    s: &RoutineSummary,
    map: &std::collections::HashMap<String, String>,
) -> PRoutineSummaryCore {
    project_routine_summary_core(s, map)
}

/// Public alias used by `summary_runner` to project an internal summary to
/// stable form for the JACOBI trace oracle.  The `routine_id` arg is ignored
/// (the id comes from `s.routine_id`); it exists only for call-site symmetry
/// with the internal helper.
pub fn project_routine_summary_core_internal(
    _routine_id: &str,
    s: &RoutineSummary,
    map: &std::collections::HashMap<String, String>,
) -> PRoutineSummaryCore {
    project_routine_summary_core(s, map)
}

fn project_routine_summary_core(
    s: &RoutineSummary,
    map: &std::collections::HashMap<String, String>,
) -> PRoutineSummaryCore {
    let mut db_effects: Vec<PDbEffect> = s
        .db_effects
        .iter()
        .map(|e| project_db_effect(e, map))
        .collect();
    db_effects.sort_by(|a, b| {
        a.effect_key
            .cmp(&b.effect_key)
            .then_with(|| a.operation_id.cmp(&b.operation_id))
    });

    let mut uncertainties: Vec<PUncertainty> = s
        .uncertainties
        .iter()
        .map(|u| project_uncertainty(u, map))
        .collect();
    uncertainties.sort_by_key(p_uncertainty_key);

    let mut parameter_roles: Vec<PRecordRoleSummary> =
        s.parameter_roles.iter().map(project_record_role).collect();
    parameter_roles.sort_by_key(|r| r.parameter_index);

    PRoutineSummaryCore {
        routine_id: stable_routine_id(&s.routine_id, map),
        db_effects,
        uncertainties,
        parameter_roles,
        in_recursive_cycle: s.in_recursive_cycle,
        has_unresolved_calls: s.has_unresolved_calls,
    }
}

// ---------------------------------------------------------------------------
// Stable fingerprint for the TRACE oracle (mirrors stableSummaryFingerprint).
// ---------------------------------------------------------------------------

pub fn stable_summary_fingerprint(s: &PRoutineSummaryCore) -> String {
    // Mirrors scripts/r3a2-projection.ts stableSummaryFingerprint EXACTLY:
    // JSON.stringify([
    //   s.dbEffects.map(e => `${e.effectKey}:${e.via}`),
    //   s.hasUnresolvedCalls,
    //   s.uncertainties.map(uncertaintyPKey),
    //   s.parameterRoles.map(r => [...]),
    // ])
    let db_effects_arr: Vec<serde_json::Value> = s
        .db_effects
        .iter()
        .map(|e| serde_json::Value::String(format!("{}:{}", e.effect_key, e.via)))
        .collect();

    let uncertainties_arr: Vec<serde_json::Value> = s
        .uncertainties
        .iter()
        .map(|u| serde_json::Value::String(p_uncertainty_key(u)))
        .collect();

    // FieldList helpers for fingerprint.
    fn field_list_fp(v: &serde_json::Value) -> serde_json::Value {
        // If it's an array, join with comma.
        if let serde_json::Value::Array(arr) = v {
            let joined: String = arr
                .iter()
                .filter_map(|x| x.as_str())
                .collect::<Vec<_>>()
                .join(",");
            serde_json::Value::String(joined)
        } else {
            v.clone()
        }
    }

    let param_roles_arr: Vec<serde_json::Value> = s
        .parameter_roles
        .iter()
        .map(|r| {
            serde_json::Value::Array(vec![
                serde_json::Value::Number(serde_json::Number::from(r.parameter_index)),
                serde_json::Value::String(r.loads_from_db_param.clone()),
                serde_json::Value::String(r.initialises_param.clone()),
                serde_json::Value::String(r.persists_current_record.clone()),
                serde_json::Value::String(r.set_based_db_writes.clone()),
                serde_json::Value::String(r.validates_param.clone()),
                serde_json::Value::String(r.copies_into_param.clone()),
                serde_json::Value::String(r.resets_filters_on_param.clone()),
                serde_json::Value::String(r.mutates_param.clone()),
                serde_json::Value::String(r.requires_loaded_at_entry.clone()),
                serde_json::Value::String(r.mutates_before_load.clone()),
                field_list_fp(&r.required_loaded_fields_at_entry),
                serde_json::Value::String(r.dirty_at_exit.clone()),
                field_list_fp(&r.current_loaded_fields_at_exit),
            ])
        })
        .collect();

    let arr = serde_json::Value::Array(vec![
        serde_json::Value::Array(db_effects_arr),
        serde_json::Value::Bool(s.has_unresolved_calls),
        serde_json::Value::Array(uncertainties_arr),
        serde_json::Value::Array(param_roles_arr),
    ]);
    serde_json::to_string(&arr).unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Top-level projection entry points.
// ---------------------------------------------------------------------------

/// Run the full pipeline (assemble + call-resolve + combined-graph + JACOBI
/// fixed-point) and project the post-computeSummaries model to the R3a-2
/// stable comparison surface. No trace hook (zero cost).
pub fn project_r3a2(resolved: &L3Resolved) -> R3a2Projection {
    let (summaries, _) = run_and_project(resolved, false);
    R3a2Projection { summaries }
}

/// Same as `project_r3a2` but ALSO collects the per-recursive-SCC fingerprint
/// TRACE (the JACOBI proof oracle). More expensive than `project_r3a2`.
pub fn project_r3a2_with_trace(resolved: &L3Resolved) -> (R3a2Projection, R3a2Trace) {
    let (summaries, traces) = run_and_project(resolved, true);
    let trace = R3a2Trace {
        traces: traces.unwrap_or_default(),
    };
    (R3a2Projection { summaries }, trace)
}

// ---------------------------------------------------------------------------
// Internal: run the pipeline and collect the projection.
// ---------------------------------------------------------------------------

fn run_and_project(
    resolved: &L3Resolved,
    collect_trace: bool,
) -> (Vec<PRoutineSummaryCore>, Option<Vec<PSccTrace>>) {
    let ws = &resolved.workspace;
    let symbols = SymbolTable::build(&ws.objects, &ws.tables, &ws.routines);
    let no_deps: Vec<DeclaredDependency> = Vec::new();
    let no_fetched: Vec<String> = Vec::new();
    let calls = resolve_calls(ws, &symbols, &no_deps, &no_fetched);
    let event_graph = build_event_graph(&ws.routines, &symbols);
    let graph = build_combined_graph(ws, &calls, &event_graph);

    // Tarjan SCC over the combined graph.
    let mut adjacency: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    for (from, list) in &graph.edges_by_from {
        adjacency.insert(from.clone(), list.iter().map(|e| e.to.clone()).collect());
    }
    let scc = tarjan_scc(&SccInputGraph {
        nodes: &graph.nodes,
        edges_by_from: &adjacency,
    });

    // Build the field-resolution index from the resolved tables (extension
    // fields are already merged into each base table's `fields` at L3) so the
    // parameterRoles readsFields/writesFields resolve to FieldIds — mirroring
    // al-sem's `resolveField` against `ctx.tableById`. Keyed (tableId,
    // lowercased field name).
    let mut field_index: crate::engine::l4::summary_runner::FieldIndex =
        std::collections::HashMap::new();
    for table in &ws.tables {
        for field in &table.fields {
            field_index
                .entry((table.id.clone(), field.name.to_lowercase()))
                .or_insert_with(|| field.id.clone());
        }
    }

    // Run the JACOBI fixed-point, optionally collecting the trace.
    let (final_summaries, raw_traces) = compute_summaries(
        &ws.routines,
        &graph,
        &scc,
        &calls.upgraded_bindings,
        &field_index,
        collect_trace,
    );

    let map = build_routine_stable_map(&ws.routines);

    // Project summaries.
    let mut projected: Vec<PRoutineSummaryCore> = final_summaries
        .values()
        .map(|s| project_routine_summary_core(s, &map))
        .collect();
    projected.sort_by(|a, b| a.routine_id.cmp(&b.routine_id));

    // Project traces (if collected).
    let traces = if collect_trace {
        Some(project_raw_scc_traces(raw_traces, &map))
    } else {
        Some(Vec::new())
    };

    (projected, traces)
}

/// Project a list of `RawSccTrace`s (per-recursive-SCC JACOBI traces, already in
/// internal-id form) to the stable [`PSccTrace`] form — the SAME projection the
/// R3a-2 trace golden carries. Exposed so the R3b Salsa `scc_summaries` path can
/// reproduce the byte-identical per-iteration fingerprint trace THROUGH the Salsa
/// query (the cyclic-fixed-point-through-Salsa proof). The output is sorted by
/// `sccId` (deterministic).
pub fn project_raw_scc_traces(
    raw_traces: Vec<crate::engine::l4::summary_runner::RawSccTrace>,
    map: &std::collections::HashMap<String, String>,
) -> Vec<PSccTrace> {
    let mut t: Vec<PSccTrace> = raw_traces
        .into_iter()
        .map(|raw| {
            let stable_members: Vec<String> = raw
                .members
                .iter()
                .map(|m| stable_routine_id(m, map))
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect();
            let scc_id = stable_members.join(",");
            let passes: Vec<PSccTracePass> = raw
                .passes
                .into_iter()
                .map(|p| {
                    // Sort members by stable routineId for a deterministic fingerprint.
                    let mut sorted: Vec<&PRoutineSummaryCore> = p.member_summaries.iter().collect();
                    sorted.sort_by(|a, b| a.routine_id.cmp(&b.routine_id));
                    let fp = serde_json::to_string(
                        &sorted
                            .iter()
                            .map(|s| {
                                serde_json::Value::Array(vec![
                                    serde_json::Value::String(s.routine_id.clone()),
                                    serde_json::Value::String(stable_summary_fingerprint(s)),
                                ])
                            })
                            .collect::<Vec<_>>(),
                    )
                    .unwrap_or_default();
                    PSccTracePass {
                        iteration: p.iteration,
                        changed: p.changed,
                        fingerprint: fp,
                    }
                })
                .collect();
            PSccTrace {
                scc_id,
                members: stable_members,
                iterations: passes.len(),
                passes,
            }
        })
        .collect();
    t.sort_by(|a, b| a.scc_id.cmp(&b.scc_id));
    t
}
