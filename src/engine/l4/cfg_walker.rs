//! L4 branch-aware control-flow walker (R3a-2 FIX) — faithful port of al-sem's
//! `src/engine/control-flow-walker.ts` (`walkRoutine` / `walkCFG`).
//!
//! Replaces the prior straight-line `walk_flat_param`. The walker does a recursive
//! traversal of the routine's CFN `statement_tree`, maintaining branch-aware
//! per-record-parameter state and JOINING the state-sets at `if`/`case`/loop
//! joins (loops via a BOUNDED fixed-point, max 3, `unknown`-saturating on
//! overshoot). A `Validate`/`Modify`/`Insert`/field-access INSIDE a conditional
//! therefore yields a branch-joined `dirtyAtExit` / `loaded` state — `unknown`
//! when branches disagree — not the `yes`/`no` a straight-line pass produced.
//!
//! Output facts (per record parameter):
//!  - Entry requirements: `requires_loaded_at_entry`, `mutates_before_load`,
//!    `required_fields` (accumulator-style — only grow, never lowered).
//!  - Exit effects: `dirty_at_exit`, `current_loaded_fields_at_exit`.
//!
//! When `statement_tree` is `None` (opaque / TryFunction / bodyless), the walker
//! falls back to a straight-line pass over the flat features — mirroring al-sem's
//! `walkFlat` fallback.

use std::collections::HashMap;

use super::combined_graph::CombinedGraph;
use super::effect_lattice::{EffectPresence, join_presence};
use super::summary::{FieldList, RoutineSummary};
use crate::engine::l2::features::{PAnchor, PCFNNode, PCallSite, PFieldAccess};
use crate::engine::l3::call_resolver::UpgradedBinding;
use crate::engine::l3::l3_workspace::{L3RecordOperation, L3Routine};

use super::summary_runner::record_flow_role;

// ---------------------------------------------------------------------------
// Lattice element types (mirror control-flow-walker.ts).
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Loaded {
    Yes,
    No,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dirty {
    Pristine,
    DirtyV,
    Persisted,
    Unknown,
}

/// `LoadedFields` / `PendingNarrow`: a sorted unique field list, or a sentinel.
/// Mirrors al-sem `FieldId[] | "full" | "unknown"` and `... | "none" | "unknown"`.
#[derive(Debug, Clone, PartialEq, Eq)]
enum LoadedFields {
    List(Vec<String>),
    Full,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PendingNarrow {
    List(Vec<String>),
    None,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RequiredFields {
    Set(Vec<String>), // sorted-unique
    Unknown,
}

/// Per-parameter mutable state threaded along control flow.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PerParamState {
    loaded: Loaded,
    dirty: Dirty,
    pending_narrow: PendingNarrow,
    current_loaded_fields: LoadedFields,
    requires_loaded_at_entry: EffectPresence,
    mutates_before_load: EffectPresence,
    required_fields: RequiredFields,
}

impl PerParamState {
    fn initial() -> Self {
        PerParamState {
            loaded: Loaded::No,
            dirty: Dirty::Pristine,
            pending_narrow: PendingNarrow::None,
            current_loaded_fields: LoadedFields::Full,
            requires_loaded_at_entry: EffectPresence::No,
            mutates_before_load: EffectPresence::No,
            required_fields: RequiredFields::Set(Vec::new()),
        }
    }
}

// ----- lattice joins (mirror control-flow-walker.ts) ------------------------

fn join_loaded(a: Loaded, b: Loaded) -> Loaded {
    if a == b { a } else { Loaded::Unknown }
}

fn join_dirty(a: Dirty, b: Dirty) -> Dirty {
    if a == b {
        return a;
    }
    if a == Dirty::DirtyV || b == Dirty::DirtyV {
        return Dirty::DirtyV;
    }
    Dirty::Unknown
}

fn join_pending(a: &PendingNarrow, b: &PendingNarrow) -> PendingNarrow {
    match (a, b) {
        (PendingNarrow::Unknown, _) | (_, PendingNarrow::Unknown) => PendingNarrow::Unknown,
        (PendingNarrow::None, PendingNarrow::None) => PendingNarrow::None,
        (PendingNarrow::None, _) | (_, PendingNarrow::None) => PendingNarrow::Unknown,
        (PendingNarrow::List(la), PendingNarrow::List(lb)) => {
            if la == lb {
                PendingNarrow::List(la.clone())
            } else {
                PendingNarrow::Unknown
            }
        }
    }
}

fn join_loaded_fields(a: &LoadedFields, b: &LoadedFields) -> LoadedFields {
    match (a, b) {
        (LoadedFields::Unknown, _) | (_, LoadedFields::Unknown) => LoadedFields::Unknown,
        (LoadedFields::Full, LoadedFields::Full) => LoadedFields::Full,
        (LoadedFields::Full, _) | (_, LoadedFields::Full) => LoadedFields::Unknown,
        (LoadedFields::List(la), LoadedFields::List(lb)) => {
            if la == lb {
                LoadedFields::List(la.clone())
            } else {
                LoadedFields::Unknown
            }
        }
    }
}

fn join_required_fields(a: &RequiredFields, b: &RequiredFields) -> RequiredFields {
    match (a, b) {
        (RequiredFields::Unknown, _) | (_, RequiredFields::Unknown) => RequiredFields::Unknown,
        (RequiredFields::Set(sa), RequiredFields::Set(sb)) => {
            let mut out: Vec<String> = sa.clone();
            for f in sb {
                if !out.contains(f) {
                    out.push(f.clone());
                }
            }
            out.sort();
            RequiredFields::Set(out)
        }
    }
}

fn join_states(a: &PerParamState, b: &PerParamState) -> PerParamState {
    PerParamState {
        loaded: join_loaded(a.loaded, b.loaded),
        dirty: join_dirty(a.dirty, b.dirty),
        pending_narrow: join_pending(&a.pending_narrow, &b.pending_narrow),
        current_loaded_fields: join_loaded_fields(
            &a.current_loaded_fields,
            &b.current_loaded_fields,
        ),
        requires_loaded_at_entry: join_presence(
            a.requires_loaded_at_entry,
            b.requires_loaded_at_entry,
        ),
        mutates_before_load: join_presence(a.mutates_before_load, b.mutates_before_load),
        required_fields: join_required_fields(&a.required_fields, &b.required_fields),
    }
}

/// Saturate decidable fields to unknown (bounded-loop overshoot / opaque / "other").
/// Accumulated entry-requirement contributions are NOT lowered.
fn saturate_unknown(s: &PerParamState) -> PerParamState {
    PerParamState {
        loaded: Loaded::Unknown,
        dirty: Dirty::Unknown,
        pending_narrow: PendingNarrow::Unknown,
        current_loaded_fields: LoadedFields::Unknown,
        requires_loaded_at_entry: s.requires_loaded_at_entry,
        mutates_before_load: s.mutates_before_load,
        required_fields: RequiredFields::Unknown,
    }
}

// ---------------------------------------------------------------------------
// Walker context + entry point.
// ---------------------------------------------------------------------------

struct ParamCtx<'a> {
    name_lc: &'a str,
    rec_var_id: Option<&'a str>,
}

/// Indexes for O(1) op/call/fa lookup during the walk.
struct WalkIndexes<'a> {
    op_by_id: HashMap<&'a str, &'a L3RecordOperation>,
    call_by_id: HashMap<&'a str, &'a PCallSite>,
    /// Field accesses keyed by `(startLine, startColumn)` — al-sem `indexFieldAccesses`.
    fa_by_pos: HashMap<(u32, u32), Vec<&'a PFieldAccess>>,
}

/// The path-aware facts the walker produces for ONE record parameter.
pub struct PathAwareFacts {
    pub requires_loaded_at_entry: EffectPresence,
    pub mutates_before_load: EffectPresence,
    pub required_loaded_fields_at_entry: FieldList,
    pub dirty_at_exit: EffectPresence,
    pub current_loaded_fields_at_exit: FieldList,
}

/// Walk the routine's body and return path-aware facts for the named record
/// parameter.
///
/// `snapshot` / `final_map` are the JACOBI maps for callee lookup.
/// `body_avail_by_id` maps an internal RoutineId → its `body_available` flag (the
/// `callee.bodyAvailable === false` opaque guard al-sem applies in `applyCall`).
#[allow(clippy::too_many_arguments)]
pub fn walk_param(
    routine: &L3Routine,
    rec_var_name_lc: &str,
    rec_var_id: Option<&str>,
    snapshot: &HashMap<String, RoutineSummary>,
    final_map: &HashMap<String, RoutineSummary>,
    upgraded_bindings: &HashMap<String, Vec<UpgradedBinding>>,
    graph: &CombinedGraph,
    body_avail_by_id: &HashMap<String, bool>,
) -> PathAwareFacts {
    let param = ParamCtx {
        name_lc: rec_var_name_lc,
        rec_var_id,
    };

    let indexes = build_indexes(routine);
    let mut exit_states: Vec<PerParamState> = Vec::new();
    let mut loop_stack: Vec<LoopFrame> = Vec::new();

    let final_state = if let Some(tree) = &routine.statement_tree {
        // The top-level `Reach` is always `Normal` in valid AL: a bare
        // break/continue outside any loop is a compile error, and a loop
        // node's own Reach never propagates past itself (see `Reach` doc).
        let (state, _reach) = walk_cfg(
            tree,
            PerParamState::initial(),
            &param,
            routine,
            snapshot,
            final_map,
            upgraded_bindings,
            graph,
            body_avail_by_id,
            &indexes,
            &mut exit_states,
            &mut loop_stack,
        );
        state
    } else {
        walk_flat(
            PerParamState::initial(),
            &param,
            routine,
            snapshot,
            final_map,
            upgraded_bindings,
            graph,
            body_avail_by_id,
        )
    };

    // Aggregate exit + fallthrough states.
    let mut all_states = exit_states;
    all_states.push(final_state);

    let dirty_at_exit = compute_dirty_at_exit(&all_states);
    let current_loaded_fields_at_exit = compute_current_loaded_at_exit(&all_states);

    // Entry-requirement facts: JOIN across all exit + fallthrough states.
    let mut requires = EffectPresence::No;
    let mut mutates = EffectPresence::No;
    let mut required = RequiredFields::Set(Vec::new());
    for s in &all_states {
        requires = join_presence(requires, s.requires_loaded_at_entry);
        mutates = join_presence(mutates, s.mutates_before_load);
        required = join_required_fields(&required, &s.required_fields);
    }

    let required_loaded_fields_at_entry = match required {
        RequiredFields::Unknown => FieldList::Unknown,
        RequiredFields::Set(mut v) => {
            v.sort();
            FieldList::Known(v)
        }
    };

    PathAwareFacts {
        requires_loaded_at_entry: requires,
        mutates_before_load: mutates,
        required_loaded_fields_at_entry,
        dirty_at_exit,
        current_loaded_fields_at_exit,
    }
}

fn build_indexes(routine: &L3Routine) -> WalkIndexes<'_> {
    let mut op_by_id: HashMap<&str, &L3RecordOperation> = HashMap::new();
    for op in &routine.record_operations {
        op_by_id.insert(op.id.as_str(), op);
    }
    let mut call_by_id: HashMap<&str, &PCallSite> = HashMap::new();
    for cs in &routine.call_sites {
        call_by_id.insert(cs.id.as_str(), cs);
    }
    let mut fa_by_pos: HashMap<(u32, u32), Vec<&PFieldAccess>> = HashMap::new();
    for fa in &routine.field_accesses {
        fa_by_pos
            .entry((fa.source_anchor.start_line, fa.source_anchor.start_column))
            .or_default()
            .push(fa);
    }
    WalkIndexes {
        op_by_id,
        call_by_id,
        fa_by_pos,
    }
}

// ---------------------------------------------------------------------------
// Recursive walker.
// ---------------------------------------------------------------------------

const LOOP_BOUND: usize = 3;

/// Whether a CFN subtree can fall through to whatever follows it in its
/// enclosing block (`Normal`), or every path through it ends in an
/// unconditional `break`/`continue` (`Abrupt` — statements after it in the
/// same block are dead for that path). A loop node always ABSORBS an
/// `Abrupt` reach produced by a `break`/`continue` inside its OWN body: code
/// after a `while`/`for`/`repeat` executes normally regardless of breaks
/// inside it, so `walk_cfg` never returns `Reach::Abrupt` for a loop node
/// itself (valid AL also never lets `break`/`continue` escape past its
/// enclosing loop). `exit`/`error`/`try` do NOT participate in this signal —
/// out of scope for T1.1, their pre-existing fallthrough behavior is
/// unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reach {
    Normal,
    Abrupt,
}

/// Per-loop break/continue state collector. Pushed when entering a
/// `while`/`for`/`foreach`/`repeat` body walk, popped when that arm returns —
/// `break`/`continue` always affect the INNERMOST enclosing loop
/// (`loop_stack.last_mut()`). `break` contributes its at-break state here,
/// folded into the loop's own exit once the bounded fixed-point settles.
/// `continue` contributes here too, folded into the loop-head join for the
/// CURRENT iteration only — `continues` is cleared at the start of each
/// iteration by the loop arm, mirroring `continue` jumping straight to the
/// condition re-check.
#[derive(Debug, Default)]
struct LoopFrame {
    breaks: Vec<PerParamState>,
    continues: Vec<PerParamState>,
}

#[allow(clippy::too_many_arguments)]
fn walk_cfg(
    node: &PCFNNode,
    pre: PerParamState,
    param: &ParamCtx,
    routine: &L3Routine,
    snapshot: &HashMap<String, RoutineSummary>,
    final_map: &HashMap<String, RoutineSummary>,
    upgraded_bindings: &HashMap<String, Vec<UpgradedBinding>>,
    graph: &CombinedGraph,
    body_avail_by_id: &HashMap<String, bool>,
    idx: &WalkIndexes,
    exit_states: &mut Vec<PerParamState>,
    loop_stack: &mut Vec<LoopFrame>,
) -> (PerParamState, Reach) {
    match node.kind.as_str() {
        "block" => {
            // Interleave field-access events with child CFN nodes by source position.
            let empty: Vec<PCFNNode> = Vec::new();
            let children = node.children.as_ref().unwrap_or(&empty);
            let fas = collect_field_accesses_in_block(node, children, param, idx);

            enum Ev<'a> {
                Child(&'a PCFNNode, u32, u32),
                Fa(&'a PFieldAccess, u32, u32),
            }
            let mut events: Vec<Ev> = Vec::new();
            for c in children {
                let (l, col) = node_start_pos(c, idx);
                events.push(Ev::Child(c, l, col));
            }
            for fa in &fas {
                events.push(Ev::Fa(
                    fa,
                    fa.source_anchor.start_line,
                    fa.source_anchor.start_column,
                ));
            }
            events.sort_by(|a, b| {
                let (la, ca) = match a {
                    Ev::Child(_, l, c) | Ev::Fa(_, l, c) => (*l, *c),
                };
                let (lb, cb) = match b {
                    Ev::Child(_, l, c) | Ev::Fa(_, l, c) => (*l, *c),
                };
                la.cmp(&lb).then_with(|| ca.cmp(&cb))
            });

            let mut state = pre;
            let mut reach = Reach::Normal;
            for e in events {
                match e {
                    Ev::Child(c, _, _) => {
                        let (post, r) = walk_cfg(
                            c,
                            state,
                            param,
                            routine,
                            snapshot,
                            final_map,
                            upgraded_bindings,
                            graph,
                            body_avail_by_id,
                            idx,
                            exit_states,
                            loop_stack,
                        );
                        state = post;
                        reach = r;
                        if reach == Reach::Abrupt {
                            // Everything after this position in the block is
                            // dead for this path (unconditional break/continue).
                            break;
                        }
                    }
                    Ev::Fa(fa, _, _) => {
                        state = apply_field_read(state, fa, param);
                    }
                }
            }
            (state, reach)
        }
        "if" => {
            let pre_branch = apply_condition_leaves(
                pre,
                node.condition_leaves.as_deref(),
                param,
                routine,
                snapshot,
                final_map,
                upgraded_bindings,
                graph,
                body_avail_by_id,
                idx,
                exit_states,
                loop_stack,
            );
            let then_branch = node.children.as_ref().and_then(|c| c.first());
            let else_branch = node.else_children.as_ref().and_then(|c| c.first());
            let (then_state, then_reach) = match then_branch {
                Some(b) => walk_cfg(
                    b,
                    pre_branch.clone(),
                    param,
                    routine,
                    snapshot,
                    final_map,
                    upgraded_bindings,
                    graph,
                    body_avail_by_id,
                    idx,
                    exit_states,
                    loop_stack,
                ),
                None => (pre_branch.clone(), Reach::Normal),
            };
            let (else_state, else_reach) = match else_branch {
                Some(b) => walk_cfg(
                    b,
                    pre_branch.clone(),
                    param,
                    routine,
                    snapshot,
                    final_map,
                    upgraded_bindings,
                    graph,
                    body_avail_by_id,
                    idx,
                    exit_states,
                    loop_stack,
                ),
                None => (pre_branch, Reach::Normal), // missing else = no-match path = preBranch.
            };
            // A branch that ends in an unconditional break/continue (Abrupt)
            // never falls through into the continuation — its state was
            // already recorded into loop_stack at the break/continue leaf, so
            // it must NOT be joined into the if-statement's own fallthrough.
            match (then_reach, else_reach) {
                (Reach::Normal, Reach::Normal) => {
                    (join_states(&then_state, &else_state), Reach::Normal)
                }
                (Reach::Normal, Reach::Abrupt) => (then_state, Reach::Normal),
                (Reach::Abrupt, Reach::Normal) => (else_state, Reach::Normal),
                (Reach::Abrupt, Reach::Abrupt) => {
                    (join_states(&then_state, &else_state), Reach::Abrupt)
                }
            }
        }
        "case" => {
            let pre_branch = apply_condition_leaves(
                pre,
                node.condition_leaves.as_deref(),
                param,
                routine,
                snapshot,
                final_map,
                upgraded_bindings,
                graph,
                body_avail_by_id,
                idx,
                exit_states,
                loop_stack,
            );
            let empty: Vec<PCFNNode> = Vec::new();
            let branches = node.children.as_ref().unwrap_or(&empty);
            let mut has_else = false;
            let mut acc: Option<PerParamState> = None;
            for c in branches {
                if c.is_case_else {
                    has_else = true;
                }
                let (post, reach) = walk_cfg(
                    c,
                    pre_branch.clone(),
                    param,
                    routine,
                    snapshot,
                    final_map,
                    upgraded_bindings,
                    graph,
                    body_avail_by_id,
                    idx,
                    exit_states,
                    loop_stack,
                );
                // Abrupt branches contribute nothing to the case's own
                // fallthrough join — already recorded via loop_stack.
                if reach == Reach::Normal {
                    acc = Some(match acc {
                        None => post,
                        Some(a) => join_states(&a, &post),
                    });
                }
            }
            match acc {
                None => {
                    if has_else {
                        // Exhaustive AND every branch abrupt: no path falls
                        // through past the case statement.
                        (pre_branch, Reach::Abrupt)
                    } else {
                        // No branch had normal reach, but an absent else
                        // means the implicit no-match path always falls
                        // through normally.
                        (pre_branch, Reach::Normal)
                    }
                }
                Some(a) => {
                    if has_else {
                        (a, Reach::Normal)
                    } else {
                        (join_states(&a, &pre_branch), Reach::Normal)
                    }
                }
            }
        }
        "case-branch" => {
            let empty: Vec<PCFNNode> = Vec::new();
            let mut state = pre;
            let mut reach = Reach::Normal;
            for c in node.children.as_ref().unwrap_or(&empty) {
                if reach == Reach::Abrupt {
                    break;
                }
                let (post, r) = walk_cfg(
                    c,
                    state,
                    param,
                    routine,
                    snapshot,
                    final_map,
                    upgraded_bindings,
                    graph,
                    body_avail_by_id,
                    idx,
                    exit_states,
                    loop_stack,
                );
                state = post;
                reach = r;
            }
            (state, reach)
        }
        "while" | "for" | "foreach" => {
            // Bounded fixed-point (max LOOP_BOUND); saturate decidable fields on overshoot.
            let body_node = node.children.as_ref().and_then(|c| c.first());
            let pre_cond = apply_condition_leaves(
                pre,
                node.condition_leaves.as_deref(),
                param,
                routine,
                snapshot,
                final_map,
                upgraded_bindings,
                graph,
                body_avail_by_id,
                idx,
                exit_states,
                loop_stack,
            );
            let Some(body_node) = body_node else {
                return (pre_cond, Reach::Normal);
            };
            loop_stack.push(LoopFrame::default());
            let mut body_pre = pre_cond;
            let mut fixpoint_state: Option<PerParamState> = None;
            for _ in 0..LOOP_BOUND {
                loop_stack
                    .last_mut()
                    .expect("just pushed")
                    .continues
                    .clear();
                let (body_post_raw, body_reach) = walk_cfg(
                    body_node,
                    body_pre.clone(),
                    param,
                    routine,
                    snapshot,
                    final_map,
                    upgraded_bindings,
                    graph,
                    body_avail_by_id,
                    idx,
                    exit_states,
                    loop_stack,
                );
                let body_post = fold_continue_states(
                    body_reach,
                    body_post_raw,
                    &body_pre,
                    &loop_stack.last().expect("just pushed").continues,
                );
                let next_iter_pre = apply_condition_leaves(
                    body_post,
                    node.condition_leaves.as_deref(),
                    param,
                    routine,
                    snapshot,
                    final_map,
                    upgraded_bindings,
                    graph,
                    body_avail_by_id,
                    idx,
                    exit_states,
                    loop_stack,
                );
                let joined = join_states(&body_pre, &next_iter_pre);
                if joined == body_pre {
                    fixpoint_state = Some(joined);
                    break;
                }
                body_pre = joined;
            }
            let exit_state = fixpoint_state.unwrap_or_else(|| saturate_unknown(&body_pre));
            let frame = loop_stack.pop().expect("pushed at loop entry");
            let mut final_exit = exit_state;
            for b in &frame.breaks {
                final_exit = join_states(&final_exit, b);
            }
            (final_exit, Reach::Normal)
        }
        "repeat" => {
            let has_body = node.children.as_ref().is_some_and(|c| !c.is_empty());
            if !has_body {
                return (
                    apply_condition_leaves(
                        pre,
                        node.condition_leaves.as_deref(),
                        param,
                        routine,
                        snapshot,
                        final_map,
                        upgraded_bindings,
                        graph,
                        body_avail_by_id,
                        idx,
                        exit_states,
                        loop_stack,
                    ),
                    Reach::Normal,
                );
            }
            // `repeat` bodies are FLAT children (ir_walk.rs Repeat lowering) —
            // unlike while/for/foreach's single wrapped block child. Wrap them
            // in a SYNTHETIC "block" node (source_range: None, so it
            // reconstructs its own range purely from the body's leaves — NOT
            // the repeat statement's full range, which would also cover the
            // `until` condition) to reuse the "block" arm's sequential /
            // field-interleave / dead-code-cutoff logic verbatim. Mirrors the
            // same synthetic-block pattern control_context.rs::walk_loop_node
            // and operation_order.rs::walk_loop_node already use for this
            // exact flat-children shape.
            let synthetic = PCFNNode {
                kind: "block".to_string(),
                operation_id: None,
                callsite_id: None,
                condition_guard: None,
                condition_leaves: None,
                children: node.children.clone(),
                else_children: None,
                is_case_else: false,
                source_range: None,
            };
            loop_stack.push(LoopFrame::default());
            let mut body_pre = pre;
            let mut fixpoint_state: Option<PerParamState> = None;
            for _ in 0..LOOP_BOUND {
                loop_stack
                    .last_mut()
                    .expect("just pushed")
                    .continues
                    .clear();
                let (body_post_raw, body_reach) = walk_cfg(
                    &synthetic,
                    body_pre.clone(),
                    param,
                    routine,
                    snapshot,
                    final_map,
                    upgraded_bindings,
                    graph,
                    body_avail_by_id,
                    idx,
                    exit_states,
                    loop_stack,
                );
                let body_post = fold_continue_states(
                    body_reach,
                    body_post_raw,
                    &body_pre,
                    &loop_stack.last().expect("just pushed").continues,
                );
                let after_cond = apply_condition_leaves(
                    body_post,
                    node.condition_leaves.as_deref(),
                    param,
                    routine,
                    snapshot,
                    final_map,
                    upgraded_bindings,
                    graph,
                    body_avail_by_id,
                    idx,
                    exit_states,
                    loop_stack,
                );
                let joined = join_states(&body_pre, &after_cond);
                if joined == body_pre {
                    fixpoint_state = Some(joined);
                    break;
                }
                body_pre = joined;
            }
            let exit_state = fixpoint_state.unwrap_or_else(|| saturate_unknown(&body_pre));
            let frame = loop_stack.pop().expect("pushed at loop entry");
            let mut final_exit = exit_state;
            for b in &frame.breaks {
                final_exit = join_states(&final_exit, b);
            }
            (final_exit, Reach::Normal)
        }
        "break" => {
            // No enclosing loop is unreachable in valid AL (the compiler
            // rejects a bare break outside a loop) — fail soft to a no-op
            // rather than panic on malformed/recovered input.
            if let Some(frame) = loop_stack.last_mut() {
                frame.breaks.push(pre.clone());
            }
            (pre, Reach::Abrupt)
        }
        "continue" => {
            // Same fail-soft guard as "break" above.
            if let Some(frame) = loop_stack.last_mut() {
                frame.continues.push(pre.clone());
            }
            (pre, Reach::Abrupt)
        }
        "exit" => {
            exit_states.push(pre.clone());
            (pre, Reach::Normal)
        }
        "error" => {
            let post = apply_condition_leaves(
                pre,
                node.condition_leaves.as_deref(),
                param,
                routine,
                snapshot,
                final_map,
                upgraded_bindings,
                graph,
                body_avail_by_id,
                idx,
                exit_states,
                loop_stack,
            );
            exit_states.push(post.clone());
            (post, Reach::Normal)
        }
        "op" => {
            let pre_op = apply_condition_leaves(
                pre,
                node.condition_leaves.as_deref(),
                param,
                routine,
                snapshot,
                final_map,
                upgraded_bindings,
                graph,
                body_avail_by_id,
                idx,
                exit_states,
                loop_stack,
            );
            let op = node
                .operation_id
                .as_deref()
                .and_then(|id| idx.op_by_id.get(id));
            let state = match op {
                Some(op) => apply_op(pre_op, op, param),
                None => pre_op,
            };
            (state, Reach::Normal)
        }
        "call" => {
            let pre_call = apply_condition_leaves(
                pre,
                node.condition_leaves.as_deref(),
                param,
                routine,
                snapshot,
                final_map,
                upgraded_bindings,
                graph,
                body_avail_by_id,
                idx,
                exit_states,
                loop_stack,
            );
            let cs = node
                .callsite_id
                .as_deref()
                .and_then(|id| idx.call_by_id.get(id));
            let state = match cs {
                Some(cs) => apply_call(
                    pre_call,
                    cs,
                    param,
                    routine,
                    snapshot,
                    final_map,
                    upgraded_bindings,
                    graph,
                    body_avail_by_id,
                ),
                None => pre_call,
            };
            (state, Reach::Normal)
        }
        "try" => {
            let sat = saturate_unknown(&pre);
            exit_states.push(sat.clone());
            (sat, Reach::Normal)
        }
        _ => {
            // "other" (and any unrecognised kind).
            let mut state = apply_condition_leaves(
                pre,
                node.condition_leaves.as_deref(),
                param,
                routine,
                snapshot,
                final_map,
                upgraded_bindings,
                graph,
                body_avail_by_id,
                idx,
                exit_states,
                loop_stack,
            );
            let empty: Vec<PCFNNode> = Vec::new();
            let mut reach = Reach::Normal;
            for c in node.children.as_ref().unwrap_or(&empty) {
                if reach == Reach::Abrupt {
                    break;
                }
                let (post, r) = walk_cfg(
                    c,
                    state,
                    param,
                    routine,
                    snapshot,
                    final_map,
                    upgraded_bindings,
                    graph,
                    body_avail_by_id,
                    idx,
                    exit_states,
                    loop_stack,
                );
                state = post;
                reach = r;
            }
            (state, reach)
        }
    }
}

/// Fold a loop iteration's `continue` contributions into its post-body state.
/// `continue` jumps straight to the loop's condition re-check — the SAME
/// place a normal (non-abrupt) body completion joins from — so a `continue`'s
/// at-continue state is just another candidate for "what state enters the
/// condition check", alongside the body's own normal-completion state (when
/// it has one). If the body itself is `Abrupt` (e.g. every path breaks or
/// continues) and there were zero continues recorded, there is no live state
/// for this iteration; fall back to the incoming pre-state as a conservative
/// no-op (the surrounding fixed-point join stabilizes on it).
fn fold_continue_states(
    body_reach: Reach,
    body_post_raw: PerParamState,
    body_pre: &PerParamState,
    continues: &[PerParamState],
) -> PerParamState {
    let mut merged = (body_reach == Reach::Normal).then_some(body_post_raw);
    for c in continues {
        merged = Some(match merged {
            None => c.clone(),
            Some(m) => join_states(&m, c),
        });
    }
    merged.unwrap_or_else(|| body_pre.clone())
}

#[allow(clippy::too_many_arguments)]
fn apply_condition_leaves(
    pre: PerParamState,
    leaves: Option<&[PCFNNode]>,
    param: &ParamCtx,
    routine: &L3Routine,
    snapshot: &HashMap<String, RoutineSummary>,
    final_map: &HashMap<String, RoutineSummary>,
    upgraded_bindings: &HashMap<String, Vec<UpgradedBinding>>,
    graph: &CombinedGraph,
    body_avail_by_id: &HashMap<String, bool>,
    idx: &WalkIndexes,
    exit_states: &mut Vec<PerParamState>,
    loop_stack: &mut Vec<LoopFrame>,
) -> PerParamState {
    let Some(leaves) = leaves else {
        return pre;
    };
    let mut state = pre;
    for leaf in leaves {
        let (post, _reach) = walk_cfg(
            leaf,
            state,
            param,
            routine,
            snapshot,
            final_map,
            upgraded_bindings,
            graph,
            body_avail_by_id,
            idx,
            exit_states,
            loop_stack,
        );
        state = post;
    }
    state
}

// ---------------------------------------------------------------------------
// Op + call + field-read application.
// ---------------------------------------------------------------------------

fn op_affects_param(op: &L3RecordOperation, param: &ParamCtx) -> bool {
    if let (Some(pid), Some(oid)) = (param.rec_var_id, op.record_variable_id.as_deref())
        && pid == oid
    {
        return true;
    }
    op.record_variable_name.to_lowercase() == param.name_lc
}

fn apply_op(state: PerParamState, op: &L3RecordOperation, param: &ParamCtx) -> PerParamState {
    if !op_affects_param(op, param) {
        return state;
    }
    let role = record_flow_role(&op.op);
    let mut out = state;
    match role {
        "loadsFromDb" => {
            out.loaded = Loaded::Yes;
            out.current_loaded_fields = match &out.pending_narrow {
                PendingNarrow::Unknown => LoadedFields::Unknown,
                PendingNarrow::None => LoadedFields::Full,
                PendingNarrow::List(l) => {
                    let mut v = l.clone();
                    v.sort();
                    LoadedFields::List(v)
                }
            };
            out.pending_narrow = PendingNarrow::None;
            out.dirty = Dirty::Pristine;
        }
        "initialises" => {
            out.loaded = Loaded::Yes;
            out.current_loaded_fields = LoadedFields::Full;
            out.pending_narrow = PendingNarrow::None;
            out.dirty = Dirty::Pristine;
        }
        "copiesInto" => {
            out.loaded = Loaded::Yes;
            out.dirty = Dirty::Pristine;
        }
        "persistsCurrent" => {
            if out.loaded != Loaded::Yes {
                out.requires_loaded_at_entry = EffectPresence::Yes;
                out.mutates_before_load = EffectPresence::Yes;
            }
            out.dirty = Dirty::Persisted;
        }
        "validates" => {
            if out.loaded != Loaded::Yes {
                out.requires_loaded_at_entry = EffectPresence::Yes;
                out.mutates_before_load = EffectPresence::Yes;
            }
            if out.dirty == Dirty::Pristine || out.dirty == Dirty::Unknown {
                out.dirty = Dirty::DirtyV;
            }
        }
        "setBasedWrite" | "resetsFilter" => {
            if role == "resetsFilter" {
                out.pending_narrow = PendingNarrow::None;
            }
            // setBasedWrite: no dirty change.
        }
        "neutral" => {
            if op.op == "SetLoadFields" {
                let mut fields: Vec<String> = dedup_sorted(op.field_arguments.as_deref());
                fields.sort();
                out.pending_narrow = PendingNarrow::List(fields);
            } else if op.op == "AddLoadFields" {
                let additions = op.field_arguments.clone().unwrap_or_default();
                match &out.pending_narrow {
                    PendingNarrow::Unknown => {}
                    PendingNarrow::None => {
                        out.pending_narrow = PendingNarrow::List(dedup_sorted(Some(&additions)));
                    }
                    PendingNarrow::List(existing) => {
                        let mut merged = existing.clone();
                        for a in &additions {
                            if !merged.contains(a) {
                                merged.push(a.clone());
                            }
                        }
                        merged.sort();
                        out.pending_narrow = PendingNarrow::List(merged);
                    }
                }
            }
        }
        _ => {}
    }
    out
}

fn dedup_sorted(v: Option<&[String]>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for f in v.unwrap_or(&[]) {
        if !out.contains(f) {
            out.push(f.clone());
        }
    }
    out.sort();
    out
}

#[allow(clippy::too_many_arguments)]
fn apply_call(
    state: PerParamState,
    cs: &PCallSite,
    param: &ParamCtx,
    routine: &L3Routine,
    snapshot: &HashMap<String, RoutineSummary>,
    final_map: &HashMap<String, RoutineSummary>,
    upgraded_bindings: &HashMap<String, Vec<UpgradedBinding>>,
    graph: &CombinedGraph,
    body_avail_by_id: &HashMap<String, bool>,
) -> PerParamState {
    let lookup =
        |id: &str| -> Option<&RoutineSummary> { snapshot.get(id).or_else(|| final_map.get(id)) };

    // Find the binding that forwards THIS param to the callee (resolved only).
    let upgraded = upgraded_bindings.get(&cs.id);
    let binding_idx = cs.argument_bindings.iter().enumerate().find(|(i, b)| {
        let resolution = upgraded
            .and_then(|ub| ub.get(*i))
            .map(|ub| ub.binding_resolution.as_str())
            .unwrap_or("unresolved-callee");
        if resolution != "resolved" {
            return false;
        }
        let by_id = param
            .rec_var_id
            .zip(b.source_record_variable_id.as_deref())
            .map(|(p, s)| p == s)
            .unwrap_or(false);
        let by_name = b
            .source_variable_name
            .as_deref()
            .map(|n| n.to_lowercase() == param.name_lc)
            .unwrap_or(false);
        by_id || by_name
    });

    let Some((binding_idx, binding)) = binding_idx else {
        return state;
    };
    let upgraded_b = upgraded.and_then(|ub| ub.get(binding_idx));

    let caller_is_var = binding.caller_source_parameter_is_var == Some(true);
    let callee_is_var = upgraded_b
        .map(|ub| ub.callee_parameter_is_var)
        .unwrap_or(false);

    // Resolve the callee edge + summary.
    let callee_id = graph.edges_by_from.get(&routine.id).and_then(|edges| {
        edges
            .iter()
            .find(|e| e.callsite_id.as_deref() == Some(cs.id.as_str()))
            .map(|e| e.to.as_str())
    });
    let callee_summary = callee_id.and_then(lookup);

    // Opaque callee: undefined edge OR bodyAvailable == false (FIX 2 — al-sem
    // control-flow-walker.ts:957). Join "unknown" for entry-req; lose state if var/var.
    let callee_opaque = match callee_id {
        None => true,
        Some(id) => !body_avail_by_id.get(id).copied().unwrap_or(false),
    };
    if callee_opaque {
        let mut out = state;
        if out.loaded != Loaded::Yes {
            out.requires_loaded_at_entry =
                join_presence(out.requires_loaded_at_entry, EffectPresence::Unknown);
            out.mutates_before_load =
                join_presence(out.mutates_before_load, EffectPresence::Unknown);
        }
        if caller_is_var && callee_is_var {
            out.loaded = Loaded::Unknown;
            out.dirty = Dirty::Unknown;
            out.pending_narrow = PendingNarrow::Unknown;
            out.current_loaded_fields = LoadedFields::Unknown;
        }
        return out;
    }

    let callee_role = callee_summary.and_then(|s| {
        s.parameter_roles
            .iter()
            .find(|r| r.parameter_index == binding.parameter_index)
    });
    let Some(cr) = callee_role else {
        // Callee body available but no role for this param — treat as opaque-ish.
        let mut out = state;
        if out.loaded != Loaded::Yes {
            out.requires_loaded_at_entry =
                join_presence(out.requires_loaded_at_entry, EffectPresence::Unknown);
            out.mutates_before_load =
                join_presence(out.mutates_before_load, EffectPresence::Unknown);
        }
        if caller_is_var && callee_is_var {
            out.loaded = Loaded::Unknown;
            out.dirty = Dirty::Unknown;
            out.pending_narrow = PendingNarrow::Unknown;
            out.current_loaded_fields = LoadedFields::Unknown;
        }
        return out;
    };

    let mut out = state;

    // c1a — entry requirements compose regardless of var-ness, only when not loaded yet.
    if out.loaded != Loaded::Yes {
        out.requires_loaded_at_entry =
            join_presence(out.requires_loaded_at_entry, cr.requires_loaded_at_entry);
        out.mutates_before_load = join_presence(out.mutates_before_load, cr.mutates_before_load);
        if let RequiredFields::Set(set) = &mut out.required_fields {
            match &cr.required_loaded_fields_at_entry {
                FieldList::Unknown => {
                    out.required_fields = RequiredFields::Unknown;
                }
                FieldList::Known(fields) => {
                    for f in fields {
                        if !set.contains(f) {
                            set.push(f.clone());
                        }
                    }
                    set.sort();
                }
                FieldList::Full => {}
            }
        }
    }

    // c1b — exit effects compose only when BOTH caller-source and callee-param are var.
    if caller_is_var && callee_is_var {
        if cr.loads_from_db_param == EffectPresence::Yes
            || cr.initialises_param == EffectPresence::Yes
            || cr.copies_into_param == EffectPresence::Yes
        {
            out.loaded = Loaded::Yes;
            out.current_loaded_fields = field_list_to_loaded(&cr.current_loaded_fields_at_exit);
            out.pending_narrow = PendingNarrow::None;
        } else if cr.loads_from_db_param == EffectPresence::Unknown
            || cr.initialises_param == EffectPresence::Unknown
            || cr.copies_into_param == EffectPresence::Unknown
        {
            out.loaded = Loaded::Unknown;
            out.current_loaded_fields = LoadedFields::Unknown;
            out.pending_narrow = PendingNarrow::Unknown;
        }

        if cr.persists_current_record == EffectPresence::Yes && out.dirty == Dirty::Pristine {
            out.dirty = Dirty::Persisted;
        }
        if (cr.validates_param == EffectPresence::Yes
            || cr.copies_into_param == EffectPresence::Yes)
            && (out.dirty == Dirty::Pristine || out.dirty == Dirty::Unknown)
        {
            out.dirty = Dirty::DirtyV;
        }
        if cr.resets_filters_on_param == EffectPresence::Yes {
            out.pending_narrow = PendingNarrow::None;
        }
        if (cr.persists_current_record == EffectPresence::Unknown
            || cr.validates_param == EffectPresence::Unknown
            || cr.copies_into_param == EffectPresence::Unknown)
            && (out.dirty == Dirty::Pristine || out.dirty == Dirty::Persisted)
        {
            out.dirty = Dirty::Unknown;
        }
    }

    out
}

fn field_list_to_loaded(fl: &FieldList) -> LoadedFields {
    match fl {
        FieldList::Unknown => LoadedFields::Unknown,
        FieldList::Full => LoadedFields::Full,
        FieldList::Known(v) => LoadedFields::List(v.clone()),
    }
}

fn apply_field_read(state: PerParamState, fa: &PFieldAccess, param: &ParamCtx) -> PerParamState {
    if fa.record_variable_name.to_lowercase() != param.name_lc {
        return state;
    }
    if state.loaded == Loaded::Yes {
        return state;
    }
    let mut out = state;
    out.requires_loaded_at_entry = EffectPresence::Yes;
    if let RequiredFields::Set(set) = &mut out.required_fields
        && !set.contains(&fa.field_name)
    {
        set.push(fa.field_name.clone());
    }
    out
}

// ---------------------------------------------------------------------------
// Field-access block attribution (mirror collectFieldAccessesInBlock).
// ---------------------------------------------------------------------------

/// Compute a CFN node's start position. For op/call leaves it is the referenced
/// op/callsite anchor; for composite nodes it is the MIN start of any leaf
/// (op/call/conditionLeaf) reachable within the subtree. Used to order block
/// events. al-sem reads `node.sourceAnchor.range.start*`; the L2 projection drops
/// it, so we reconstruct it from the leaf anchors the node references.
fn node_start_pos(node: &PCFNNode, idx: &WalkIndexes) -> (u32, u32) {
    // Prefer the TRUE node range (carried from L2); fall back to leaf anchors.
    if let Some((sl, sc, _, _)) = node.source_range {
        return (sl, sc);
    }
    let mut best: Option<(u32, u32)> = None;
    fn consider(best: &mut Option<(u32, u32)>, a: &PAnchor) {
        let p = (a.start_line, a.start_column);
        match best {
            Some(cur) if *cur <= p => {}
            _ => *best = Some(p),
        }
    }
    fn walk(node: &PCFNNode, idx: &WalkIndexes, best: &mut Option<(u32, u32)>) {
        if let Some(op_id) = &node.operation_id
            && let Some(op) = idx.op_by_id.get(op_id.as_str())
        {
            consider(best, &op.source_anchor);
        }
        if let Some(cs_id) = &node.callsite_id
            && let Some(cs) = idx.call_by_id.get(cs_id.as_str())
        {
            consider(best, &cs.source_anchor);
        }
        if let Some(leaves) = &node.condition_leaves {
            for l in leaves {
                walk(l, idx, best);
            }
        }
        if let Some(children) = &node.children {
            for c in children {
                walk(c, idx, best);
            }
        }
        if let Some(children) = &node.else_children {
            for c in children {
                walk(c, idx, best);
            }
        }
    }
    walk(node, idx, &mut best);
    best.unwrap_or((u32::MAX, u32::MAX))
}

/// Compute a CFN node's (start, end) range from the min/max leaf anchors in its
/// subtree. Mirrors al-sem's `node.sourceAnchor.range` for the FA-in-block test.
fn node_range(node: &PCFNNode, idx: &WalkIndexes) -> Option<(u32, u32, u32, u32)> {
    // Prefer the TRUE node range (carried from L2 — covers conditions/branches the
    // leaf anchors miss, so condition-only field reads attribute to NO block,
    // matching al-sem). Fall back to leaf reconstruction for synthetic nodes.
    if node.source_range.is_some() {
        return node.source_range;
    }
    let mut min: Option<(u32, u32)> = None;
    let mut max: Option<(u32, u32)> = None;
    fn consider(min: &mut Option<(u32, u32)>, max: &mut Option<(u32, u32)>, a: &PAnchor) {
        let s = (a.start_line, a.start_column);
        let e = (a.end_line, a.end_column);
        match min {
            Some(cur) if *cur <= s => {}
            _ => *min = Some(s),
        }
        match max {
            Some(cur) if *cur >= e => {}
            _ => *max = Some(e),
        }
    }
    fn walk(
        node: &PCFNNode,
        idx: &WalkIndexes,
        min: &mut Option<(u32, u32)>,
        max: &mut Option<(u32, u32)>,
    ) {
        if let Some(op_id) = &node.operation_id
            && let Some(op) = idx.op_by_id.get(op_id.as_str())
        {
            consider(min, max, &op.source_anchor);
        }
        if let Some(cs_id) = &node.callsite_id
            && let Some(cs) = idx.call_by_id.get(cs_id.as_str())
        {
            consider(min, max, &cs.source_anchor);
        }
        for group in [&node.condition_leaves, &node.children, &node.else_children]
            .into_iter()
            .flatten()
        {
            for c in group {
                walk(c, idx, min, max);
            }
        }
    }
    walk(node, idx, &mut min, &mut max);
    match (min, max) {
        (Some((sl, sc)), Some((el, ec))) => Some((sl, sc, el, ec)),
        _ => None,
    }
}

fn child_recurses_into_fas(c: &PCFNNode) -> bool {
    match c.kind.as_str() {
        "op" | "call" | "exit" | "error" => false,
        "other" => c.children.as_ref().map(|v| !v.is_empty()).unwrap_or(false),
        _ => true, // block / if / case / case-branch / while / repeat / for / foreach / try
    }
}

fn falls_in_range(fa_line: u32, fa_col: u32, sl: u32, sc: u32, el: u32, ec: u32) -> bool {
    if fa_line < sl || fa_line > el {
        return false;
    }
    if fa_line == sl && fa_col < sc {
        return false;
    }
    if fa_line == el && fa_col > ec {
        return false;
    }
    true
}

/// Collect field accesses on `param` inside this block's range but NOT inside any
/// recursive child's range. Mirrors al-sem `collectFieldAccessesInBlock`.
fn collect_field_accesses_in_block<'a>(
    block: &PCFNNode,
    children: &[PCFNNode],
    param: &ParamCtx,
    idx: &'a WalkIndexes,
) -> Vec<&'a PFieldAccess> {
    let mut result: Vec<&PFieldAccess> = Vec::new();
    // The block's own range = the union of its children's leaf ranges (al-sem reads
    // the block sourceAnchor; we reconstruct from leaves). If the block has no
    // positioned leaves, it covers nothing — but then it also has no FAs to attribute.
    let block_range = node_range(block, idx);

    // Precompute recursive children's ranges once.
    let recursive_ranges: Vec<(u32, u32, u32, u32)> = children
        .iter()
        .filter(|c| child_recurses_into_fas(c))
        .filter_map(|c| node_range(c, idx))
        .collect();

    for fa in &param_field_accesses(idx, param) {
        let fr = &fa.source_anchor;
        // Must lie inside the block range (when known).
        if let Some((sl, sc, el, ec)) = block_range
            && !falls_in_range(fr.start_line, fr.start_column, sl, sc, el, ec)
        {
            continue;
        }
        // Must NOT lie inside any recursive child range.
        let in_recursive_child = recursive_ranges.iter().any(|(sl, sc, el, ec)| {
            falls_in_range(fr.start_line, fr.start_column, *sl, *sc, *el, *ec)
        });
        if in_recursive_child {
            continue;
        }
        result.push(fa);
    }
    result
}

/// All field accesses on `param` (used by the block-attribution scan).
fn param_field_accesses<'a>(idx: &'a WalkIndexes, param: &ParamCtx) -> Vec<&'a PFieldAccess> {
    // R3b nondeterminism audit: `fa_by_pos` is a `HashMap<(line,col), …>`, so
    // `.values()` yields a hash-random order. The downstream consumer
    // (`collect_field_accesses_in_block`) interleaves these with child CFN nodes
    // and SORTS by (line, col) with a STABLE sort — so two field accesses sharing
    // the exact same start position would retain this hash-random relative order,
    // a latent incremental nondeterminism. Iterate the position keys in sorted
    // order so the pre-sort input order is canonical. Output-neutral for the R3a
    // goldens (distinct FAs never share an identical start position there); the
    // canonical order only ever differs from hash order on an exact-position tie.
    let mut keys: Vec<&(u32, u32)> = idx.fa_by_pos.keys().collect();
    keys.sort_unstable();
    let mut out: Vec<&PFieldAccess> = Vec::new();
    for key in keys {
        if let Some(list) = idx.fa_by_pos.get(key) {
            for fa in list {
                if fa.record_variable_name.to_lowercase() == param.name_lc {
                    out.push(fa);
                }
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Exit-fact aggregation.
// ---------------------------------------------------------------------------

fn compute_dirty_at_exit(states: &[PerParamState]) -> EffectPresence {
    let mut any_unknown = false;
    for s in states {
        match s.dirty {
            Dirty::DirtyV => return EffectPresence::Yes,
            Dirty::Unknown => any_unknown = true,
            _ => {}
        }
    }
    if any_unknown {
        EffectPresence::Unknown
    } else {
        EffectPresence::No
    }
}

fn compute_current_loaded_at_exit(states: &[PerParamState]) -> FieldList {
    let mut acc: Option<LoadedFields> = None;
    for s in states {
        acc = Some(match acc {
            None => s.current_loaded_fields.clone(),
            Some(a) => join_loaded_fields(&a, &s.current_loaded_fields),
        });
    }
    loaded_to_field_list(&acc.unwrap_or(LoadedFields::Unknown))
}

fn loaded_to_field_list(lf: &LoadedFields) -> FieldList {
    match lf {
        LoadedFields::Unknown => FieldList::Unknown,
        LoadedFields::Full => FieldList::Full,
        LoadedFields::List(v) => FieldList::Known(v.clone()),
    }
}

// ---------------------------------------------------------------------------
// Straight-line fallback (no statement_tree). Mirrors walkFlat.
// ---------------------------------------------------------------------------

enum FlatEvent<'a> {
    Op(&'a L3RecordOperation, u32, u32),
    Field(&'a PFieldAccess, u32, u32),
    Call(&'a PCallSite, u32, u32),
}

#[allow(clippy::too_many_arguments)]
fn walk_flat(
    pre: PerParamState,
    param: &ParamCtx,
    routine: &L3Routine,
    snapshot: &HashMap<String, RoutineSummary>,
    final_map: &HashMap<String, RoutineSummary>,
    upgraded_bindings: &HashMap<String, Vec<UpgradedBinding>>,
    graph: &CombinedGraph,
    body_avail_by_id: &HashMap<String, bool>,
) -> PerParamState {
    let mut events: Vec<FlatEvent> = Vec::new();
    for op in &routine.record_operations {
        if !op_affects_param(op, param) {
            continue;
        }
        events.push(FlatEvent::Op(
            op,
            op.source_anchor.start_line,
            op.source_anchor.start_column,
        ));
    }
    for fa in &routine.field_accesses {
        if fa.record_variable_name.to_lowercase() != param.name_lc {
            continue;
        }
        events.push(FlatEvent::Field(
            fa,
            fa.source_anchor.start_line,
            fa.source_anchor.start_column,
        ));
    }
    for cs in &routine.call_sites {
        let upgraded = upgraded_bindings.get(&cs.id);
        for (i, b) in cs.argument_bindings.iter().enumerate() {
            let resolution = upgraded
                .and_then(|ub| ub.get(i))
                .map(|ub| ub.binding_resolution.as_str())
                .unwrap_or("unresolved-callee");
            if resolution != "resolved" {
                continue;
            }
            let by_id = param
                .rec_var_id
                .zip(b.source_record_variable_id.as_deref())
                .map(|(p, s)| p == s)
                .unwrap_or(false);
            let by_name = b
                .source_variable_name
                .as_deref()
                .map(|n| n.to_lowercase() == param.name_lc)
                .unwrap_or(false);
            if !by_id && !by_name {
                continue;
            }
            events.push(FlatEvent::Call(
                cs,
                cs.source_anchor.start_line,
                cs.source_anchor.start_column,
            ));
            break;
        }
    }
    events.sort_by(|a, b| {
        let (la, ca) = match a {
            FlatEvent::Op(_, l, c) | FlatEvent::Field(_, l, c) | FlatEvent::Call(_, l, c) => {
                (*l, *c)
            }
        };
        let (lb, cb) = match b {
            FlatEvent::Op(_, l, c) | FlatEvent::Field(_, l, c) | FlatEvent::Call(_, l, c) => {
                (*l, *c)
            }
        };
        la.cmp(&lb).then_with(|| ca.cmp(&cb))
    });

    let mut state = pre;
    for e in events {
        match e {
            FlatEvent::Op(op, _, _) => state = apply_op(state, op, param),
            FlatEvent::Field(fa, _, _) => state = apply_field_read(state, fa, param),
            FlatEvent::Call(cs, _, _) => {
                state = apply_call(
                    state,
                    cs,
                    param,
                    routine,
                    snapshot,
                    final_map,
                    upgraded_bindings,
                    graph,
                    body_avail_by_id,
                );
            }
        }
    }
    state
}
