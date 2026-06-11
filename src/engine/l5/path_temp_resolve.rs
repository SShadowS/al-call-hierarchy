//! Per-PATH temp-state resolution (Component 3, RV-6).
//!
//! A path-walker [`WalkResult`] terminates at a db-operation that may carry
//! `temp_state = ParameterDependent(i)` — its temporariness depends on parameter
//! `i` of the routine the op LIVES IN. That symbolic index is only resolvable in
//! the context of a CONCRETE caller chain: the SAME op reached from two different
//! callers can resolve differently (caller-A passes a temp var → `Known(true)`;
//! caller-B passes a physical var → `Known(false)`). This is *per-finding* truth,
//! and it is what [`resolve_temp_along_path`] computes.
//!
//! ## Path orientation (verified against `path_walker::visit`)
//!
//! `WalkResult.path` is in **ROOT → TERMINAL** order. The walker descends from
//! `start` into each `edge.to`, PUSHING a hop step (`build_hop_step`) as it goes,
//! and appends the terminal step LAST. So:
//!   - `path.last()` is the TERMINAL step; its `routine_id` is the routine that
//!     OWNS the terminal op (frame T).
//!   - A HOP step at index `k` has `routine_id == edge.from` (the PARENT / UPSTREAM
//!     routine, closer to the root) and `callsite_id == edge.callsite_id` (the call
//!     site IN THAT PARENT that invokes the next-deeper routine `edge.to`).
//!   - The hop that ENTERS the terminal frame T is therefore the LAST hop step in
//!     the path — the step immediately before the terminal whose `callsite_id` is
//!     `Some` and whose `routine_id` is T's caller. Stepping "toward the path root"
//!     means walking the hop steps from the END of the vec toward the FRONT.
//!
//! Detector-supplied prefix steps (d1 seeds `[loopStep, callStep]`) are part of the
//! same vec; the `loopStep` carries `callsite_id == None` and the `callStep` carries
//! the in-loop callsite that enters the FIRST walked routine. The resolver only ever
//! consumes hop steps that carry a `Some(callsite_id)`, and it stops the moment it
//! runs out of caller hops — so seed steps that lack a callsite are simply the path
//! root for resolution purposes (still-PD there → `Unknown`).
//!
//! ## Callee-param index — DERIVED, not a new serialized field (RV-6 decision)
//!
//! RV-6 asks the walker to expose, per hop, the callee-param index needed to step
//! frames. We DERIVE it at resolve time from the L3 routine map instead of adding a
//! field to a serialized walker/`EvidenceStep` struct: given a hop's `callsite_id`,
//! the parent routine's `call_sites[*].argument_bindings` already carry
//! `parameter_index` (= callee param index) and `source_temp_state` /
//! `source_parameter_index`. Deriving avoids touching any serialized struct, so NO
//! R3a/trace/R4 golden can move (lower golden impact — the explicitly preferred
//! option). The resolver receives the routine map it needs as an explicit argument.
//!
//! ## Soundness
//!
//! Resolution only ever yields `Known(true)` when a concrete binding source ON THE
//! PATH is itself `Known(true)`. EVERY uncertainty — a missing caller hop, a missing
//! callsite, a missing binding, a `Some(Unknown)` / `None` source, or a still-`PD`
//! state at the path root — collapses to `Unknown` (the conservative, FIRING
//! direction). This mirrors the L4 per-callsite substitution table
//! (`summary_runner::substitute_pd_temp_state`) applied frame-by-frame.

use std::collections::HashMap;

use crate::engine::l3::l3_workspace::L3Routine;
use crate::engine::l4::effect_lattice::TempStateKind;
use crate::engine::l5::finding::EvidenceStep;

/// Resolve a terminal op's `temp_state` ALONG ONE WALK PATH to a concrete
/// `Known(_)` / `Unknown` (Component 3, RV-6).
///
/// - `path` is a [`WalkResult::path`](crate::engine::l5::path_walker::WalkResult)
///   in ROOT→TERMINAL order (see module docs).
/// - `terminal_state` is the terminal op's `temp_state` as a [`TempStateKind`]
///   (the caller maps `op.temp_state` via `TempStateKind::from_p_temp_state`, with
///   a `None` temp_state → `Unknown`).
/// - `routine_by_id` maps each routine's INTERNAL id to its `L3Routine` (so a hop's
///   `callsite_id` can be resolved against the parent routine's call sites). This is
///   the same `ctx.routine_by_id` index d1 already builds.
///
/// Steps one frame toward the path root per `ParameterDependent` level, applying the
/// L4 substitution table at each hop; terminates because each step consumes one more
/// caller hop and the path is finite.
///
/// Visibility: `pub` (not `pub(crate)`) SOLELY so the `tests/temp_state_path.rs`
/// integration test — a separate crate — can drive it directly per this task's TDD
/// mandate. It is otherwise an internal L5 helper; Task 10 wires d1 to it in-crate.
pub fn resolve_temp_along_path(
    path: &[EvidenceStep],
    terminal_state: TempStateKind,
    routine_by_id: &HashMap<&str, &L3Routine>,
) -> TempStateKind {
    // The hop steps that carry a real caller callsite, in ROOT→TERMINAL order. The
    // terminal step (last, callsite_id == None) and any seed loop step (callsite_id
    // == None) are naturally excluded. We consume these from the END (the hop that
    // enters the terminal frame) toward the FRONT (the root) as we chase PD levels.
    let caller_hops: Vec<&EvidenceStep> = path.iter().filter(|s| s.callsite_id.is_some()).collect();

    let mut state = terminal_state;
    // `next_hop` indexes the caller_hops vec; we start at the LAST hop (the one
    // entering the terminal frame) and walk backward (toward root) per PD level.
    let mut hop_idx = caller_hops.len();

    loop {
        let i = match &state {
            // Concrete — done.
            TempStateKind::Known(_) | TempStateKind::Unknown => return state,
            TempStateKind::ParameterDependent(i) => *i,
        };

        if hop_idx == 0 {
            // Reached the path ROOT while still PD: the op's tempness depends on an
            // ENTRY parameter with no caller in this path. Conservative → Unknown.
            return TempStateKind::Unknown;
        }
        hop_idx -= 1;
        let hop = caller_hops[hop_idx];

        // The hop's parent routine (it OWNS the callsite). `routine_id` is the
        // caller (edge.from); `callsite_id` is the call site in that caller.
        let parent = routine_by_id.get(hop.routine_id.as_str()).copied();
        let cs_id = hop.callsite_id.as_deref();
        state = step_one_frame(parent, cs_id, i);
    }
}

/// Apply the L4 per-callsite substitution table for one caller frame: resolve
/// `ParameterDependent(callee_param_index)` through the parent routine's argument
/// binding for that callee param. Mirrors
/// `summary_runner::substitute_pd_temp_state`'s table, but threaded for the
/// per-PATH walk (the parent routine + callsite are derived from the hop here,
/// not from a `CombinedEdge`).
///
/// Any missing piece → `Unknown` (sound = fires):
///   - no parent routine in the map, or no callsite id on the hop;
///   - no callsite with that id in the parent;
///   - no binding whose `parameter_index == callee_param_index`;
///   - `source_temp_state` is `Some(Unknown)` or `None`.
/// `Some(Known(v))` → `Known(v)`; `Some(PD(j))` → `ParameterDependent(j)` (re-anchored
/// to the PARENT frame at L2 — the same UPWARD re-symbolization Task 8 does — which
/// the next loop turn then chases through the parent's own caller hop).
fn step_one_frame(
    parent: Option<&L3Routine>,
    callsite_id: Option<&str>,
    callee_param_index: u32,
) -> TempStateKind {
    let (Some(parent), Some(cs_id)) = (parent, callsite_id) else {
        return TempStateKind::Unknown;
    };
    let Some(cs) = parent.call_sites.iter().find(|c| c.id == cs_id) else {
        return TempStateKind::Unknown;
    };
    let Some(binding) = cs
        .argument_bindings
        .iter()
        .find(|b| b.parameter_index == callee_param_index)
    else {
        return TempStateKind::Unknown;
    };
    match &binding.source_temp_state {
        Some(ts) => match TempStateKind::from_p_temp_state(ts) {
            TempStateKind::Known(v) => TempStateKind::Known(v),
            // Forwarded keyword-less by-var param: re-symbolize to the caller's own
            // param index (chains upward; chased on the next loop turn).
            TempStateKind::ParameterDependent(j) => TempStateKind::ParameterDependent(j),
            TempStateKind::Unknown => TempStateKind::Unknown,
        },
        None => TempStateKind::Unknown,
    }
}
