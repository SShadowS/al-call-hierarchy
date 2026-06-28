//! Routine identity + scope value types shared by the IR L2/L3 projection.
//!
//! The tree-sitter scope EXTRACTION (parameters / record vars / variables /
//! globals) is retired — those now come from the owned IR ([`super::ir_walk`]).
//! What remains here is tree-sitter-free and production-shared: the
//! [`ParameterSymbol`] / [`RecordVariable`] value types, the `PTempState`
//! constructors, type-text canonicalization, and routine-id computation.

use super::features::PTempState;
use crate::engine::ids::{CanonicalRoutineKey, ParamSpec, encode_routine_id};

#[derive(Clone)]
pub struct ParameterSymbol {
    pub index: u32,
    pub name: String,
    pub type_text: String,
    pub is_var: bool,
    pub is_record: bool,
    pub table_name: Option<String>,
}

#[derive(Clone)]
pub struct RecordVariable {
    pub id: String,
    pub name: String,
    pub table_name: Option<String>,
    pub temp_state: PTempState,
    pub is_parameter: bool,
    pub parameter_index: Option<u32>,
}

/// `{ kind: "known", value }` — the single shared PTempState "known" constructor.
/// `pub(crate)` so the L3 record-type override (`record_types.rs`) and the ABI→L3
/// projection (`deps/cross_app_l3.rs`) reuse ONE definition (compiler-enforced on
/// any future `PTempState` shape change), instead of duplicating the literal.
pub(crate) fn ts_known(value: bool) -> PTempState {
    PTempState {
        kind: "known".to_string(),
        value: Some(value),
        parameter_index: None,
    }
}

/// `{ kind: "parameter-dependent", parameterIndex }` — the single shared PD
/// constructor. `pub(crate)` for the same reuse reason as [`ts_known`].
pub(crate) fn ts_param_dependent(index: u32) -> PTempState {
    PTempState {
        kind: "parameter-dependent".to_string(),
        value: None,
        parameter_index: Some(index),
    }
}

/// Collapse internal whitespace runs while preserving quoted AL identifiers.
pub fn canonicalize_type_text(raw: &str) -> String {
    let mut out = String::new();
    let mut in_quotes = false;
    let mut last_was_space = false;
    for ch in raw.trim().chars() {
        if ch == '"' {
            in_quotes = !in_quotes;
            out.push(ch);
            last_was_space = false;
            continue;
        }
        if in_quotes {
            out.push(ch);
            continue;
        }
        if ch == ' ' || ch == '\t' || ch == '\n' || ch == '\r' {
            if !last_was_space {
                out.push(' ');
                last_was_space = true;
            }
            continue;
        }
        out.push(ch);
        last_was_space = false;
    }
    out
}

/// Compute the internal RoutineId (`{modelInstanceId}/{canonicalKeyHash}`).
#[allow(clippy::too_many_arguments)]
pub fn compute_routine_id(
    app_guid: &str,
    object_type: &str,
    object_number: i64,
    routine_kind: &str,
    routine_name: &str,
    parameters: &[ParameterSymbol],
    return_type_text: Option<&str>,
    model_instance_id: &str,
) -> String {
    let param_specs: Vec<ParamSpec> = parameters
        .iter()
        .map(|p| ParamSpec {
            type_text: p.type_text.clone(),
            is_var: p.is_var,
        })
        .collect();
    let normalized_signature_hash =
        crate::engine::ids::normalized_signature_hash(routine_name, &param_specs, return_type_text);
    let key = CanonicalRoutineKey {
        app_guid: app_guid.to_string(),
        object_type: object_type.to_string(),
        object_number,
        routine_kind: routine_kind.to_string(),
        routine_name: routine_name.to_string(),
        normalized_signature_hash,
    };
    encode_routine_id(&key, model_instance_id)
}
