//! Port of al-sem `src/digest/conditionality.ts`.
//!
//! Conditionality captures how reliably an effect fires relative to the
//! routine's normal (success) exit path.
//!
//! ## Lattice (restrictiveness rank, 0 = least restrictive)
//!   unconditional-on-success → rank 0
//!   conditional              → rank 1
//!   loop-body                → rank 2
//!   guarded-by-IsHandled     → rank 3
//!   error-path               → rank 4
//!   unknown (contaminates — not ranked)
//!
//! ## path conditionality
//!   Most-restrictive along the path; any "unknown" contaminates.
//!
//! ## effect conditionality
//!   Least-restrictive across all paths; if ALL paths are "unknown" OR
//!   truncated && no unconditional path → "unknown".

/// Effect conditionality — the string form used in the JSON golden.
pub type EffectConditionality = &'static str;

pub const UNCONDITIONAL: EffectConditionality = "unconditional-on-success";
pub const CONDITIONAL: EffectConditionality = "conditional";
pub const LOOP_BODY: EffectConditionality = "loop-body";
pub const GUARDED_BY_ISHANDLED: EffectConditionality = "guarded-by-IsHandled";
pub const ERROR_PATH: EffectConditionality = "error-path";
pub const UNKNOWN: EffectConditionality = "unknown";

/// Restrictiveness rank for non-unknown conditionalities.
fn rank(c: EffectConditionality) -> Option<u32> {
    match c {
        "unconditional-on-success" => Some(0),
        "conditional" => Some(1),
        "loop-body" => Some(2),
        "guarded-by-IsHandled" => Some(3),
        "error-path" => Some(4),
        _ => None, // "unknown" → None (contaminates)
    }
}

/// `contextToConditionality` — map a ControlContext string (or None) to
/// EffectConditionality.
///
/// `None` / `"unreachable"` → `"unknown"` (defensive).
pub fn context_to_conditionality(ctx: Option<&str>) -> EffectConditionality {
    match ctx {
        Some("top-level") => UNCONDITIONAL,
        Some("conditional") => CONDITIONAL,
        Some("loop-body") => LOOP_BODY,
        Some("error-path") => ERROR_PATH,
        Some("is-handled-guarded") => GUARDED_BY_ISHANDLED,
        _ => UNKNOWN,
    }
}

/// `pathConditionality` — most-restrictive along the path; unknown contaminates.
pub fn path_conditionality(
    hop_contexts: &[EffectConditionality],
    terminal_ctx: EffectConditionality,
) -> EffectConditionality {
    let all: Vec<EffectConditionality> = hop_contexts
        .iter()
        .copied()
        .chain(std::iter::once(terminal_ctx))
        .collect();
    // Any unknown → contaminate.
    if all.iter().any(|c| *c == UNKNOWN) {
        return UNKNOWN;
    }
    // Most restrictive = highest rank.
    let mut max_rank: u32 = 0;
    let mut result: EffectConditionality = UNCONDITIONAL;
    for c in &all {
        if let Some(r) = rank(c) {
            if r > max_rank {
                max_rank = r;
                result = c;
            }
        }
    }
    result
}

/// `effectConditionality` — least-restrictive across all paths.
///
/// - All unknown → unknown.
/// - Truncated && no unconditional found → unknown.
pub fn effect_conditionality(
    path_conds: &[EffectConditionality],
    truncated: bool,
) -> EffectConditionality {
    if path_conds.is_empty() {
        return UNKNOWN;
    }
    let mut least_rank: Option<u32> = None;
    let mut result: EffectConditionality = UNKNOWN;
    let mut has_unconditional = false;

    for &c in path_conds {
        if c == UNKNOWN {
            continue;
        }
        if let Some(r) = rank(c) {
            if least_rank.map(|lr| r < lr).unwrap_or(true) {
                least_rank = Some(r);
                result = c;
            }
            if c == UNCONDITIONAL {
                has_unconditional = true;
            }
        }
    }

    if least_rank.is_none() {
        return UNKNOWN;
    }
    if truncated && !has_unconditional {
        return UNKNOWN;
    }
    result
}
