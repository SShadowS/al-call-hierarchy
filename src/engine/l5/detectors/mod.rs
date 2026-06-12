//! The ported L5 detectors. Each module ports one al-sem detector; the registered
//! list grows as each wave lands. Currently: d4 (R4-0), d5/d10/d11/d18/d21/d36 (R4-A),
//! d22/d33 (R4-B), d7/d12/d38 (R4-C), d8/d9/d34/d35 (R4-D), d32 (reverse-call-graph wave),
//! d50 (R4-H checked-run-implicit-commit).

pub mod d1;
pub mod d10;
pub mod d11;
pub mod d12;
pub mod d13;
pub mod d14;
pub mod d16;
pub mod d17;
pub mod d18;
pub mod d19;
pub mod d2;
pub mod d20;
pub mod d21;
pub mod d22;
pub mod d29;
pub mod d3;
pub mod d32;
pub mod d33;
pub mod d34;
pub mod d35;
pub mod d36;
pub mod d37;
pub mod d38;
pub mod d39;
pub mod d4;
pub mod d40;
pub mod d41;
pub mod d42;
pub mod d43;
pub mod d44;
pub mod d45;
pub mod d46;
pub mod d47;
pub mod d48;
pub mod d49;
pub mod d5;
pub mod d50;
pub mod d51;
pub mod d7;
pub mod d8;
pub mod d9;

use std::collections::{HashMap, HashSet};

use crate::engine::l2::features::{PAnchor, PCallSite, PCallee, PExpressionInfo};
use crate::engine::l3::l3_workspace::{L3RecordOperation, L3Routine, L3Table};
use crate::engine::l5::detector_context::DetectorContext;
use crate::engine::l5::finding::SourceAnchor;
use crate::engine::l5::registry::Detector;

/// `beforeAnchor(a, b)` — true iff `a` is strictly before `b` by source position.
/// Port of al-sem `src/engine/source-anchor.ts:beforeAnchor`.
/// Strict less-than: same position is NOT "before".
pub(crate) fn before_anchor(a: &PAnchor, b: &PAnchor) -> bool {
    if a.start_line != b.start_line {
        return a.start_line < b.start_line;
    }
    a.start_column < b.start_column
}

/// G-12/G-15: the table's PRIMARY KEY (first key) field NAMES, lowercased.
/// The PK is always loaded regardless of `SetLoadFields`, so PK accesses never
/// need load coverage (d3) and PK entry-requirements never force a wider
/// narrow (d42). An empty/missing first key excludes nothing — when the PK
/// cannot be determined the detectors keep firing.
pub(crate) fn primary_key_field_names_lc(table: &L3Table) -> HashSet<String> {
    let field_name_by_id: HashMap<&str, &str> = table
        .fields
        .iter()
        .map(|f| (f.id.as_str(), f.name.as_str()))
        .collect();
    table
        .keys
        .first()
        .map(|k| {
            k.fields
                .iter()
                .filter_map(|fid| field_name_by_id.get(fid.as_str()).map(|n| n.to_lowercase()))
                .collect()
        })
        .unwrap_or_default()
}

/// Normalize a `SetLoadFields` / `AddLoadFields` field argument (or any raw
/// field-name token) for matching against field-access names: trim, strip ONE
/// pair of surrounding quotes (the L2 body walk keeps the raw `"Unit Price"`
/// argument text, while field accesses are recorded unquoted), lowercase.
/// (G-12 refinement 4; shared with d42's G-15 PK exclusion.)
pub(crate) fn normalize_load_field_arg(raw: &str) -> String {
    let t = raw.trim();
    let t = if t.len() >= 2 && t.starts_with('"') && t.ends_with('"') {
        &t[1..t.len() - 1]
    } else {
        t
    };
    t.to_lowercase()
}

/// G-9: page triggers in which the platform has already loaded the implicit
/// `Rec` before the trigger body runs (docs/engine-gaps.md G-9).
/// G-14: extended with the field-level lookup triggers `OnLookup` /
/// `OnAssistEdit` — the platform loads `Rec` before those too, and a
/// `Validate` inside `OnLookup` is persisted by the page framework.
const PAGE_TRIGGERS_REC_LOADED: &[&str] = &[
    "OnValidate",
    "OnAction",
    "OnAfterGetRecord",
    "OnDrillDown",
    "OnAfterGetCurrRecord",
    "OnLookup",
    "OnAssistEdit",
];

/// Detector-audit class B: the table-LEVEL triggers. The AL platform loads the
/// implicit `Rec` before each of these runs AND auto-persists it afterwards:
/// `OnInsert`/`OnModify`/`OnRename` end in the platform writing `Rec` to the
/// table; `OnDelete` ends in the platform deleting it (so "validate without
/// persist" is moot there). EXACT names, matched case-insensitively, only
/// meaningful for `Table`/`TableExtension` objects.
const TABLE_TRIGGERS_REC_AUTO_PERSIST: &[&str] = &["OnInsert", "OnModify", "OnDelete", "OnRename"];

/// G-9 suppression signal (exact, structural): true iff `routine` is a page
/// trigger (`OnValidate` / `OnAction` / `OnAfterGetRecord` / `OnDrillDown` /
/// `OnAfterGetCurrRecord`) or a table field `OnValidate` trigger or a
/// table-LEVEL `OnInsert`/`OnModify`/`OnDelete`/`OnRename` trigger
/// (detector-audit class B), AND the op's receiver is the trigger's implicit
/// current record `Rec`. In all of these the AL platform loaded `Rec` before
/// the trigger ran, so "never loaded" / "never persisted" detectors
/// (d11/d21/d37) must not fire on it (for d37 the table-level triggers also
/// auto-persist — see `is_auto_persist_trigger_rec`). When unsure (any other
/// kind / object type / trigger name / receiver) returns `false` — the
/// detectors keep firing.
pub(crate) fn is_platform_loaded_trigger_rec(
    routine: &L3Routine,
    record_variable_name: &str,
) -> bool {
    if !record_variable_name.eq_ignore_ascii_case("rec") {
        return false;
    }
    if routine.kind != "trigger" {
        return false;
    }
    match routine.object_type.as_str() {
        "Page" | "PageExtension" => PAGE_TRIGGERS_REC_LOADED
            .iter()
            .any(|t| routine.name.eq_ignore_ascii_case(t)),
        // A `trigger OnValidate` in a table object is always a FIELD trigger;
        // the table-LEVEL triggers (OnInsert/OnModify/OnDelete/OnRename) run
        // with `Rec` platform-loaded too (class B).
        "Table" | "TableExtension" => {
            routine.name.eq_ignore_ascii_case("OnValidate")
                || TABLE_TRIGGERS_REC_AUTO_PERSIST
                    .iter()
                    .any(|t| routine.name.eq_ignore_ascii_case(t))
        }
        _ => false,
    }
}

/// Class-B auto-persist signal (exact, structural): true iff `routine` is a
/// table-LEVEL `OnInsert`/`OnModify`/`OnDelete`/`OnRename` trigger of a
/// `Table`/`TableExtension` object AND `record_variable_name` is the trigger's
/// implicit current record `Rec`. The platform persists `Rec` after these
/// triggers return (`OnDelete` deletes it), so a Validate-dirty `Rec` at
/// trigger exit is NOT discarded — d39 (record-left-dirty) must not flag the
/// trigger for "never persists after the call". When unsure returns `false` —
/// the detector keeps firing.
pub(crate) fn is_auto_persist_trigger_rec(routine: &L3Routine, record_variable_name: &str) -> bool {
    if !record_variable_name.eq_ignore_ascii_case("rec") {
        return false;
    }
    if routine.kind != "trigger" {
        return false;
    }
    matches!(routine.object_type.as_str(), "Table" | "TableExtension")
        && TABLE_TRIGGERS_REC_AUTO_PERSIST
            .iter()
            .any(|t| routine.name.eq_ignore_ascii_case(t))
}

/// The record-op names d11/d21 recognize as satisfying the "record is loaded"
/// precondition (Get/Find* load from the DB; Init initialises; Insert/Copy put
/// the record in a well-defined state; Next advances a loaded cursor). Shared so
/// the G-10 one-hop callee summary (`record_loaded_by_call_before`) provably
/// uses the SAME set the detectors apply intraprocedurally.
pub(crate) const RECORD_LOAD_OPS: &[&str] = &[
    "Get",
    "FindFirst",
    "FindLast",
    "FindSet",
    "Find",
    "Next",
    "Init",
    "Insert",
    "Copy",
];

/// G-10 tier 1: platform BUILT-IN record loaders that are NOT in the L2
/// record-op map (`record_op.rs`) — they surface as member CALL SITES, not
/// record operations — but perform a complete row fetch, equivalent to `Get`
/// for the "record is loaded" precondition. EXACT method names, matched
/// case-insensitively. Deliberately conservative: only platform built-ins that
/// load the full current record are listed; custom `FindXxx`/`GetXxx` wrappers
/// are covered by the tier-2 one-hop callee summary instead.
const PLATFORM_LOADER_METHODS: &[&str] = &["GetBySystemId"];

/// G-10 suppression signal: true iff a CALL SITE strictly before `op_anchor`
/// in `routine` provably loaded the record variable `record_variable_name`
/// (docs/engine-gaps.md G-10). Two exact, structural tiers:
///
/// - Tier 1 (platform built-in): a member call `<var>.GetBySystemId(...)` —
///   receiver text matches the record variable exactly (case-insensitive).
/// - Tier 2 (one-hop callee summary): the record variable is passed as an
///   argument whose binding RESOLVED to a by-`var` record parameter of a
///   workspace callee, and that callee performs a recognized load op
///   (`RECORD_LOAD_OPS`) on that parameter. The check mirrors the detectors'
///   own intraprocedural semantics (any load op in the callee body counts,
///   regardless of branching — same as the in-routine `loaded_before` scan).
///
/// Anything uncertain (unresolved callee, by-value binding, different
/// variable, non-loading callee, cross-app context with no resolved-edge
/// index) returns `false` — the detectors keep firing.
pub(crate) fn record_loaded_by_call_before(
    routine: &L3Routine,
    ctx: &DetectorContext,
    record_variable_name: &str,
    op_anchor: &PAnchor,
) -> bool {
    let var_lc = record_variable_name.to_lowercase();
    for cs in &routine.call_sites {
        if !before_anchor(&cs.source_anchor, op_anchor) {
            continue;
        }
        // Tier 1: platform built-in loader called ON the record itself.
        if let PCallee::Member { receiver, method } = &cs.callee {
            if receiver.to_lowercase() == var_lc
                && PLATFORM_LOADER_METHODS
                    .iter()
                    .any(|m| m.eq_ignore_ascii_case(method))
            {
                return true;
            }
        }
        // Tier 2: by-var argument into a resolved callee that loads it.
        if callee_loads_by_var_arg(ctx, cs, &var_lc) {
            return true;
        }
    }
    false
}

/// G-16: the BOUND on the callee-load summary's wrapper-chain depth. A callee
/// counts as loading the by-var arg if the load op lands in its own body
/// (1 hop) or in a callee it forwards the same by-var arg to, up to this many
/// callee hops total — `FindTemplate` -> `FindTemplateWithReportID` ->
/// `FindSet` is 2 hops. A load any deeper proves nothing (no unbounded
/// recursion; the detectors keep firing).
const MAX_LOAD_WRAPPER_HOPS: u32 = 3;

/// G-10/G-16 tier 2: does the RESOLVED callee of `cs` perform a recognized
/// load op on the by-`var` record parameter bound to caller variable `var_lc`
/// (lowercased)? G-16 extends G-10's one-hop summary to a BOUNDED multi-hop
/// chain (`MAX_LOAD_WRAPPER_HOPS`): every hop must be the SAME
/// resolved-binding join (resolved callee, by-`var` record parameter), and the
/// recognized load op must land on the forwarded parameter within the bound.
/// Every uncertainty returns `false`.
fn callee_loads_by_var_arg(ctx: &DetectorContext, cs: &PCallSite, var_lc: &str) -> bool {
    callee_applies_op_to_by_var_arg(ctx, cs, var_lc, |callee, param_lc| {
        callee_loads_param(ctx, callee, param_lc, MAX_LOAD_WRAPPER_HOPS - 1)
    })
}

/// Does `callee`'s body prove its record parameter `param_lc` (lowercased)
/// loaded — directly (a recognized `RECORD_LOAD_OPS` op or a tier-1 platform
/// loader call on it), or by forwarding it by-`var` into a deeper resolved
/// callee that loads it within `remaining_hops` further hops? `remaining_hops`
/// strictly decreases per hop, so recursion is bounded even across mutually
/// recursive wrappers.
fn callee_loads_param(
    ctx: &DetectorContext,
    callee: &L3Routine,
    param_lc: &str,
    remaining_hops: u32,
) -> bool {
    // Direct load op on the parameter anywhere in the callee body (same
    // semantics as the detectors' intraprocedural scan — branching ignored,
    // e.g. `if not R.Get(..) then begin R.Init(); R.Insert(); end` leaves the
    // record loaded-or-inserted either way).
    if callee.record_operations.iter().any(|op| {
        RECORD_LOAD_OPS.contains(&op.op.as_str())
            && op.record_variable_name.to_lowercase() == param_lc
    }) {
        return true;
    }
    for cs in &callee.call_sites {
        // Tier 1 inside the wrapper: `<param>.GetBySystemId(...)`.
        if let PCallee::Member { receiver, method } = &cs.callee {
            if receiver.to_lowercase() == param_lc
                && PLATFORM_LOADER_METHODS
                    .iter()
                    .any(|m| m.eq_ignore_ascii_case(method))
            {
                return true;
            }
        }
        // Deeper wrapper hop: the parameter forwarded by-`var` into a
        // resolved callee that loads it (bounded).
        if remaining_hops > 0
            && callee_applies_op_to_by_var_arg(ctx, cs, param_lc, |inner, inner_param_lc| {
                callee_loads_param(ctx, inner, inner_param_lc, remaining_hops - 1)
            })
        {
            return true;
        }
    }
    false
}

/// Shared G-10/G-3 one-hop callee-summary join: does the RESOLVED callee of
/// `cs` receive caller variable `var_lc` (lowercased) as a by-`var` record
/// parameter for which `param_check(callee, param_name_lc)` holds? One hop
/// only — the callee's own body is inspected directly, never its transitive
/// callees. Every uncertainty (unresolved callee, missing body, by-value
/// binding, unresolved binding, unknown parameter) returns `false` — the
/// detectors keep firing.
fn callee_applies_op_to_by_var_arg(
    ctx: &DetectorContext,
    cs: &PCallSite,
    var_lc: &str,
    param_check: impl Fn(&L3Routine, &str) -> bool,
) -> bool {
    let Some(edge) = ctx.resolved_call_edge_by_callsite.get(&cs.id) else {
        return false;
    };
    let Some(to) = edge.to.as_deref() else {
        return false;
    };
    let Some(callee) = ctx.routine_by_id.get(to) else {
        return false;
    };
    if !callee.body_available || callee.parse_incomplete {
        return false;
    }
    let upgraded = ctx.upgraded_bindings_by_callsite.get(&cs.id);
    for (i, binding) in cs.argument_bindings.iter().enumerate() {
        // The L3 binding's sourceVariableName is stored lowercased.
        if binding.source_variable_name.as_deref() != Some(var_lc) {
            continue;
        }
        // The post-upgrade resolution lives in the resolver's side table,
        // index-aligned with `argument_bindings` (same join as d37/d39/d40).
        let Some(up) = upgraded.and_then(|u| u.get(i)) else {
            continue;
        };
        // Only a RESOLVED by-`var` binding aliases the caller's record — a
        // by-value callee operates on its own copy, proving nothing for the
        // caller.
        if up.binding_resolution != "resolved" || !up.callee_parameter_is_var {
            continue;
        }
        // The callee's record parameter at this position…
        let Some(param_rv) = callee
            .record_variables
            .iter()
            .find(|rv| rv.is_parameter && rv.parameter_index == Some(binding.parameter_index))
        else {
            continue;
        };
        // …must satisfy the caller-supplied op predicate in the callee body.
        if param_check(callee, &param_rv.name.to_lowercase()) {
            return true;
        }
    }
    false
}

/// G-16(b): the BOUND on the record-assignment chain depth (`C := B; B := A;`
/// with `A` loaded proves `C` through 2 links). Bounded so a cyclic
/// assignment pair can never recurse forever.
const MAX_ASSIGN_CHAIN_DEPTH: u32 = 3;

/// G-16(b) suppression signal: true iff a whole-record assignment
/// `<record_variable_name> := <rhs>` strictly before `op_anchor` in `routine`
/// copies a PROVABLY-LOADED record var into the op's record
/// (docs/engine-gaps.md G-16). The RHS must be provably loaded AT THE
/// ASSIGNMENT POINT: a recognized load op / loading call strictly before the
/// assignment, the platform-loaded trigger `Rec`, a parameter record (caller-
/// loaded — the same skip d11/d21 apply to direct ops on parameters), or a
/// further assignment from a loaded var (bounded chain). Anything uncertain
/// (expression RHS, field write, unknown/unloaded RHS, assignment after the
/// op) returns `false` — the detectors keep firing.
pub(crate) fn record_loaded_by_assignment_before(
    routine: &L3Routine,
    ctx: &DetectorContext,
    record_variable_name: &str,
    op_anchor: &PAnchor,
) -> bool {
    assigned_from_loaded_var_before(
        routine,
        ctx,
        &record_variable_name.to_lowercase(),
        op_anchor,
        MAX_ASSIGN_CHAIN_DEPTH,
    )
}

/// One assignment-chain link: is there a `var_lc := <rhs-identifier>`
/// strictly before `anchor` whose RHS is provably loaded at that assignment?
/// Consumes one unit of `depth` per link.
fn assigned_from_loaded_var_before(
    routine: &L3Routine,
    ctx: &DetectorContext,
    var_lc: &str,
    anchor: &PAnchor,
    depth: u32,
) -> bool {
    if depth == 0 {
        return false;
    }
    routine.var_assignments.iter().any(|asg| {
        // `lhs_name` / `rhs_identifier` are stored lowercased; `rhs_identifier`
        // is only Some for a whole-variable copy (bare identifier on BOTH sides).
        asg.lhs_name == var_lc
            && before_anchor(&asg.source_anchor, anchor)
            && asg.rhs_identifier.as_deref().is_some_and(|rhs_lc| {
                variable_proven_loaded_before(routine, ctx, rhs_lc, &asg.source_anchor, depth - 1)
            })
    })
}

/// Is record variable `var_lc` (lowercased) PROVABLY loaded strictly before
/// `anchor` in `routine`? Exactly the signals d11/d21 already accept for the
/// op's own record: the platform-loaded trigger `Rec` (G-9), a parameter
/// record (caller-loaded — the detectors' own skip), a recognized load op, a
/// loading call (G-10/G-16a), or — recursively, bounded — an assignment from
/// another loaded var. A non-record or unknown identifier matches none of
/// these and returns `false`.
fn variable_proven_loaded_before(
    routine: &L3Routine,
    ctx: &DetectorContext,
    var_lc: &str,
    anchor: &PAnchor,
    depth: u32,
) -> bool {
    if is_platform_loaded_trigger_rec(routine, var_lc) {
        return true;
    }
    if routine
        .record_variables
        .iter()
        .any(|rv| rv.is_parameter && rv.name.eq_ignore_ascii_case(var_lc))
    {
        return true;
    }
    if routine.record_operations.iter().any(|op| {
        RECORD_LOAD_OPS.contains(&op.op.as_str())
            && op.record_variable_name.eq_ignore_ascii_case(var_lc)
            && before_anchor(&op.source_anchor, anchor)
    }) {
        return true;
    }
    if record_loaded_by_call_before(routine, ctx, var_lc, anchor) {
        return true;
    }
    assigned_from_loaded_var_before(routine, ctx, var_lc, anchor, depth)
}

/// The narrowing-filter op names d33 recognizes intraprocedurally. Shared so
/// the G-3 one-hop callee summary (`record_filtered_by_call_before`) provably
/// uses the SAME set d33 applies in-routine.
pub(crate) const RECORD_FILTER_OPS: &[&str] = &["SetRange", "SetFilter"];

/// G-3 suppression signal: true iff a CALL SITE strictly before `op_anchor`
/// in `routine` provably left a narrowing filter on the record variable
/// `record_variable_name` (docs/engine-gaps.md G-3, extended by G-17).
/// Three exact, structural tiers:
///
/// - By-`var` argument (G-3): the record variable is passed as an argument
///   whose binding RESOLVED to a by-`var` record parameter of a workspace
///   callee, and that callee's NET effect on that parameter is "filtered" —
///   its last filter-relevant op (`SetRange`/`SetFilter`/`Reset`) on the
///   parameter is a filter, not a `Reset`.
/// - Receiver method (G-17a): the record variable is the RECEIVER of a member
///   call that RESOLVED to a procedure defined on the receiver's own table,
///   and that table method's NET effect on its implicit self record is
///   "filtered" (bare `SetRange(...)` in a table method filters the implicit
///   self, which aliases the caller's receiver — e.g. CDO's
///   `LineReport.SetEMailTemplateLineFilter(Rec); LineReport.DeleteAll();`,
///   where the by-VALUE argument only supplies the filter VALUES).
/// - Page selection (G-17b): `CurrPage.SetSelectionFilter(<var>)` — the
///   platform builtin that copies the page's row selection onto the argument
///   record as filters.
///
/// A `Reset` on the receiver in the CALLER between the call and `op_anchor`
/// wipes the callee-applied filters, so that call site proves nothing
/// (mirrors d33's intraprocedural `was_filtered_before`).
///
/// Anything uncertain (unresolved callee, by-value binding, different
/// variable, non-filtering callee, call after the op) returns `false` —
/// d33 keeps firing.
pub(crate) fn record_filtered_by_call_before(
    routine: &L3Routine,
    ctx: &DetectorContext,
    record_variable_name: &str,
    op_anchor: &PAnchor,
) -> bool {
    let var_lc = record_variable_name.to_lowercase();
    for cs in &routine.call_sites {
        if !before_anchor(&cs.source_anchor, op_anchor) {
            continue;
        }
        // A caller-side Reset between the helper call and the bulk op wipes
        // whatever filters the callee applied.
        let reset_after_call = routine.record_operations.iter().any(|op| {
            op.op == "Reset"
                && op.record_variable_name.to_lowercase() == var_lc
                && before_anchor(&cs.source_anchor, &op.source_anchor)
                && before_anchor(&op.source_anchor, op_anchor)
        });
        if reset_after_call {
            continue;
        }
        // G-17(b): `<page>.SetSelectionFilter(<var>)` — page builtin.
        if call_is_set_selection_filter_on(cs, &var_lc) {
            return true;
        }
        // G-17(a): member call ON the receiver to a filter method defined on
        // the receiver's own table.
        if let PCallee::Member { receiver, method } = &cs.callee {
            if receiver.to_lowercase() == var_lc
                && receiver_table_method_net_filters_self(routine, ctx, &var_lc, method)
            {
                return true;
            }
        }
        // G-3: by-`var` argument into a resolved callee that filters it.
        if callee_applies_op_to_by_var_arg(ctx, cs, &var_lc, callee_net_filters_param) {
            return true;
        }
    }
    false
}

/// G-17(b): is `cs` a member call `<receiver>.SetSelectionFilter(<var_lc>)`?
/// `SetSelectionFilter` is the page/platform builtin that copies the page's
/// row selection onto the argument record as filters — a narrowing filter on
/// that record. Matched structurally (the receiver — `CurrPage` or a page
/// variable — is not a workspace routine, so no callee summary can exist);
/// the bound argument must be exactly the bulk-op record variable.
fn call_is_set_selection_filter_on(cs: &PCallSite, var_lc: &str) -> bool {
    let PCallee::Member { method, .. } = &cs.callee else {
        return false;
    };
    if !method.eq_ignore_ascii_case("SetSelectionFilter") {
        return false;
    }
    cs.argument_bindings
        .iter()
        .any(|b| b.source_variable_name.as_deref() == Some(var_lc))
}

/// G-17(a): does `method`, a procedure defined on the RECEIVER's own table,
/// provably leave its implicit self record (the caller's receiver) filtered?
///
/// The call resolver never resolves a member call through a RECORD receiver
/// (`parse_object_type_ref` has no `Record` keyword — the G-3 root cause), so
/// no resolved edge exists for this shape; the join is done here instead: the
/// receiver variable's RESOLVED `table_id` (the same resolution the bulk op's
/// own `table_id` uses) identifies the table object, and every in-source
/// procedure with that name on that object must net-filter its implicit self
/// (all-must-match is conservative under same-name overloads). An internal
/// table id is `${appGuid}/table/${n}` while an object id is
/// `${appGuid}/Table/${n}` — compared case-insensitively; a TableExtension's
/// `${appGuid}/TableExtension/${n}` can never match, so extension-defined
/// helpers (different implicit-self semantics on the SAME table) are excluded.
/// Every uncertainty (unresolved receiver table, no such method, missing
/// body, parse-incomplete) returns `false` — d33 keeps firing.
fn receiver_table_method_net_filters_self(
    routine: &L3Routine,
    ctx: &DetectorContext,
    var_lc: &str,
    method: &str,
) -> bool {
    let Some(table_id) = routine
        .record_variables
        .iter()
        .find(|rv| rv.name.to_lowercase() == var_lc)
        .and_then(|rv| rv.table_id.as_deref())
    else {
        return false;
    };
    let mut found = false;
    for callee in ctx.routine_by_id.values() {
        if !callee.object_id.eq_ignore_ascii_case(table_id)
            || callee.kind != "procedure"
            || !callee.name.eq_ignore_ascii_case(method)
        {
            continue;
        }
        if !callee.body_available || callee.parse_incomplete {
            return false;
        }
        if !callee_net_filters_implicit_self(callee) {
            return false;
        }
        found = true;
    }
    found
}

/// G-17(a): is the table method `callee`'s NET effect on its implicit SELF
/// record "filtered"? Inside a table method the implicit self's ops surface
/// in two shapes the L2 walk produces: bare call sites (`SetRange(...)` —
/// table PROCEDURES carry no implicit-Rec frame, so the op is recorded as a
/// bare call) and explicit-`Rec` shapes (`Rec.SetRange(...)` as a member call
/// site, or a record operation on `Rec` where the walk recognized it). The
/// LAST filter-relevant event (by source position) must be a filter
/// (`RECORD_FILTER_OPS`), not a `Reset` — mirrors `callee_net_filters_param`.
/// No filter-relevant event at all returns `false`.
fn callee_net_filters_implicit_self(callee: &L3Routine) -> bool {
    /// `Some(true)` = filter, `Some(false)` = Reset, `None` = not filter-relevant.
    fn filter_event(name: &str) -> Option<bool> {
        if RECORD_FILTER_OPS
            .iter()
            .any(|m| m.eq_ignore_ascii_case(name))
        {
            Some(true)
        } else if name.eq_ignore_ascii_case("Reset") {
            Some(false)
        } else {
            None
        }
    }
    fn keep_latest<'a>(
        last: &mut Option<(&'a PAnchor, bool)>,
        anchor: &'a PAnchor,
        is_filter: bool,
    ) {
        match last {
            Some((prev, _)) if !before_anchor(prev, anchor) => {}
            _ => *last = Some((anchor, is_filter)),
        }
    }
    let mut last: Option<(&PAnchor, bool)> = None;
    for op in &callee.record_operations {
        if !op.record_variable_name.eq_ignore_ascii_case("rec") {
            continue;
        }
        if let Some(is_filter) = filter_event(&op.op) {
            keep_latest(&mut last, &op.source_anchor, is_filter);
        }
    }
    for cs in &callee.call_sites {
        let name = match &cs.callee {
            PCallee::Bare { name } => Some(name.as_str()),
            PCallee::Member { receiver, method } if receiver.eq_ignore_ascii_case("rec") => {
                Some(method.as_str())
            }
            _ => None,
        };
        if let Some(is_filter) = name.and_then(filter_event) {
            keep_latest(&mut last, &cs.source_anchor, is_filter);
        }
    }
    last.is_some_and(|(_, is_filter)| is_filter)
}

/// G-3: is the callee's NET effect on its parameter `param_lc` (lowercased)
/// "filtered"? Scans the callee body's filter-relevant ops (`SetRange` /
/// `SetFilter` / `Reset`) on that parameter and checks the LAST one (by
/// source position) is a filter — a trailing `Reset` un-filters the record.
/// No filter-relevant op at all returns `false`.
fn callee_net_filters_param(callee: &L3Routine, param_lc: &str) -> bool {
    let mut last: Option<&L3RecordOperation> = None;
    for op in &callee.record_operations {
        if op.record_variable_name.to_lowercase() != param_lc {
            continue;
        }
        if !RECORD_FILTER_OPS.contains(&op.op.as_str()) && op.op != "Reset" {
            continue;
        }
        match last {
            Some(prev) if !before_anchor(&prev.source_anchor, &op.source_anchor) => {}
            _ => last = Some(op),
        }
    }
    last.is_some_and(|op| RECORD_FILTER_OPS.contains(&op.op.as_str()))
}

/// G-6: BC VIRTUAL/system tables with NO physical SQL backing (docs/engine-gaps.md
/// G-6). Reads of these resolve against the platform's in-memory metadata store
/// (object/field/session metadata, the `Integer`/`Date` number generators), never
/// a SQL round-trip — so SQL-cost detectors (d1/d4) must not fire on ops targeting
/// them. (d3/d33 already skip unresolved-table ops structurally, and a virtual
/// table never resolves in the source-only workspace, so they need no gate.)
///
/// EXACT BC table names, matched case-insensitively. Deliberately CONSERVATIVE:
/// only tables confidently known to be virtual are listed; any unlisted table
/// keeps firing (the safe direction). Extend by appending the exact table name.
const VIRTUAL_SYSTEM_TABLES: &[&str] = &[
    "AllObj",
    "AllObjWithCaption",
    "Field",
    "Key",
    "Object",
    "Table Metadata",
    "Page Metadata",
    "Codeunit Metadata",
    "Report Metadata",
    "Database Locks",
    "Session",
    "Integer",
    "Date",
];

/// True iff `name` EXACTLY matches (case-insensitive) a known BC virtual/system
/// table from the G-6 allowlist.
pub(crate) fn is_virtual_system_table(name: &str) -> bool {
    VIRTUAL_SYSTEM_TABLES
        .iter()
        .any(|t| t.eq_ignore_ascii_case(name))
}

/// G-6 suppression signal (exact, structural): true iff this record op targets a
/// known BC virtual/system table — i.e. its type did NOT resolve to a workspace
/// table (a workspace table with a colliding name is a USER-defined physical
/// table → keep firing) AND the receiving record variable's DECLARED type name is
/// on the `VIRTUAL_SYSTEM_TABLES` allowlist. The variable lookup mirrors
/// `describe_table`'s tier-2 (case-insensitive name match on the routine's record
/// variables). When unsure (resolved table, unknown variable, no declared type,
/// unlisted name) returns `false` — the detectors keep firing.
pub(crate) fn op_targets_virtual_system_table(
    op: &L3RecordOperation,
    routine: &L3Routine,
    table_by_id: &HashMap<&str, &L3Table>,
) -> bool {
    // A type that resolved to a workspace table is a user-defined physical table
    // (the source-only pipeline never loads platform tables) — never virtual.
    if let Some(tid) = op.table_id.as_deref() {
        if table_by_id.contains_key(tid) {
            return false;
        }
    }
    let lc = op.record_variable_name.to_lowercase();
    routine
        .record_variables
        .iter()
        .find(|v| v.name.to_lowercase() == lc)
        .and_then(|v| v.table_name.as_deref())
        .is_some_and(is_virtual_system_table)
}

/// `unquotedFieldName` from `model/expression.ts`:
/// Resolve a field-name argument to its unquoted form. Prefers `.value` (set on
/// `quoted_identifier` / `string_literal` / `qualified_enum_value`) over `.text`.
/// Preserves original case — callers lowercase for comparison where needed.
/// Shared across d18 (CalcFields-arg literal check) and d22 (CalcFields-arg coverage).
pub(crate) fn unquoted_field_name(info: &PExpressionInfo) -> String {
    if let Some(v) = &info.value {
        return v.clone();
    }
    info.text.clone()
}

/// Build the internal `SourceAnchor` from an L2 `PAnchor` + the owning routine.
/// Drops `enclosingRoutineId` from the PAnchor (which doesn't carry one) and
/// stamps the routine's own id. Hash fields default to `None`.
pub(crate) fn anchor_of(a: &PAnchor, routine: &L3Routine) -> SourceAnchor {
    SourceAnchor {
        source_unit_id: a.source_unit_id.clone(),
        start_line: a.start_line,
        start_column: a.start_column,
        end_line: a.end_line,
        end_column: a.end_column,
        enclosing_routine_id: routine.id.clone(),
        syntax_kind: a.syntax_kind.clone(),
        normalized_text_hash: None,
        leading_context_hash: None,
        trailing_context_hash: None,
    }
}

/// `groupAndCap` — group findings by `key_of(finding)`, sort each group by
/// `compareStrings(id)`, then keep the first `max_per_key` of each group (group
/// iteration in sorted-key order). Returns `(kept, truncated_count)`. Port of
/// al-sem `finding-grouping.ts:groupAndCap`. Used by d44 (per-event cap) and d45
/// (per-publisher cap) to bound output explosion.
pub(crate) fn group_and_cap<F>(
    findings: Vec<crate::engine::l5::finding::Finding>,
    key_of: F,
    max_per_key: usize,
) -> (Vec<crate::engine::l5::finding::Finding>, usize)
where
    F: Fn(&crate::engine::l5::finding::Finding) -> String,
{
    use std::collections::BTreeMap;
    let mut groups: BTreeMap<String, Vec<crate::engine::l5::finding::Finding>> = BTreeMap::new();
    for f in findings {
        groups.entry(key_of(&f)).or_default().push(f);
    }
    let mut kept: Vec<crate::engine::l5::finding::Finding> = Vec::new();
    let mut truncated_count = 0usize;
    // BTreeMap keys iterate in byte order (matching compareStrings).
    for (_k, mut bag) in groups {
        bag.sort_by(|a, b| a.id.cmp(&b.id));
        if bag.len() <= max_per_key {
            kept.extend(bag);
        } else {
            truncated_count += bag.len() - max_per_key;
            bag.truncate(max_per_key);
            kept.extend(bag);
        }
    }
    (kept, truncated_count)
}

/// The registered detector list, in the EXACT order of al-sem's ALL_DETECTORS
/// (DEFAULT_DETECTORS first, then OPT_IN_DETECTORS). This order governs the
/// `detectorStats` array for the `all` slot; the `default` slot is a subset in this
/// same order (as `select_detectors` filters by name while preserving registry order).
///
/// DEFAULT order (34): d1, d2, d3, d4, d5, d7, d8, d9, d10, d11, d12, d13, d14,
///   d16, d17, d18, d19, d20, d21, d22, d29, d32, d33, d34, d35, d36, d37, d38,
///   d39, d41, d42, d43, d44, d45.
/// OPT_IN order (7):  d40, d46, d47, d48, d49, d50, d51.
pub fn registered_detectors() -> Vec<Detector> {
    vec![
        // --- DEFAULT_DETECTORS (34, in al-sem registry order) ---
        Detector {
            name: "d1-db-op-in-loop".to_string(),
            run: d1::detect_d1,
        },
        Detector {
            name: "d2-event-fanout-in-loop".to_string(),
            run: d2::detect_d2,
        },
        Detector {
            name: "d3-missing-setloadfields".to_string(),
            run: d3::detect_d3,
        },
        Detector {
            name: "d4-repeated-lookup-in-loop".to_string(),
            run: d4::detect_d4,
        },
        Detector {
            name: "d5-set-based-opportunity".to_string(),
            run: d5::detect_d5,
        },
        Detector {
            name: "d7-recursive-event-expansion".to_string(),
            run: d7::detect_d7,
        },
        Detector {
            name: "d8-commit-in-transaction".to_string(),
            run: d8::detect_d8,
        },
        Detector {
            name: "d9-transaction-span-summary".to_string(),
            run: d9::detect_d9,
        },
        Detector {
            name: "d10-self-modifying-loop".to_string(),
            run: d10::detect_d10,
        },
        Detector {
            name: "d11-modify-without-get".to_string(),
            run: d11::detect_d11,
        },
        Detector {
            name: "d12-dead-integration-event".to_string(),
            run: d12::detect_d12,
        },
        Detector {
            name: "d13-cross-app-internal-call".to_string(),
            run: d13::detect_d13,
        },
        Detector {
            name: "d14-dead-routine".to_string(),
            run: d14::detect_d14,
        },
        Detector {
            name: "d16-obsolete-routine-call".to_string(),
            run: d16::detect_d16,
        },
        Detector {
            name: "d17-min-version-drift".to_string(),
            run: d17::detect_d17,
        },
        Detector {
            name: "d18-constant-filter-in-loop".to_string(),
            run: d18::detect_d18,
        },
        Detector {
            name: "d19-unused-parameter".to_string(),
            run: d19::detect_d19,
        },
        Detector {
            name: "d20-unreachable-after-exit".to_string(),
            run: d20::detect_d20,
        },
        Detector {
            name: "d21-read-without-load".to_string(),
            run: d21::detect_d21,
        },
        Detector {
            name: "d22-flowfield-without-calcfields".to_string(),
            run: d22::detect_d22,
        },
        Detector {
            name: "d29-subscriber-modify-on-event-record".to_string(),
            run: d29::detect_d29,
        },
        Detector {
            name: "d32-constant-boolean-parameter".to_string(),
            run: d32::detect_d32,
        },
        Detector {
            name: "d33-unfiltered-bulk-write".to_string(),
            run: d33::detect_d33,
        },
        Detector {
            name: "d34-commit-in-loop".to_string(),
            run: d34::detect_d34,
        },
        Detector {
            name: "d35-commit-in-event-subscriber".to_string(),
            run: d35::detect_d35,
        },
        Detector {
            name: "d36-late-setloadfields".to_string(),
            run: d36::detect_d36,
        },
        Detector {
            name: "d37-validate-without-persist".to_string(),
            run: d37::detect_d37,
        },
        Detector {
            name: "d38-subscriber-to-obsolete-event".to_string(),
            run: d38::detect_d38,
        },
        Detector {
            name: "d39-record-left-dirty-across-chain".to_string(),
            run: d39::detect_d39,
        },
        Detector {
            name: "d41-transitive-filter-loss".to_string(),
            run: d41::detect_d41,
        },
        Detector {
            name: "d42-cross-call-wrong-setloadfields".to_string(),
            run: d42::detect_d42,
        },
        Detector {
            name: "d43-event-ishandled-skip".to_string(),
            run: d43::detect_d43,
        },
        Detector {
            name: "d44-event-multi-subscriber-overlap".to_string(),
            run: d44::detect_d44,
        },
        Detector {
            name: "d45-event-transitive-table-exposure".to_string(),
            run: d45::detect_d45,
        },
        // --- OPT_IN_DETECTORS (7, in al-sem registry order) ---
        // d40: OPT-IN in al-sem (transitive-load-missing).
        Detector {
            name: "d40-transitive-load-missing".to_string(),
            run: d40::detect_d40,
        },
        // d46: OPT-IN in al-sem (commit-in-lifecycle).
        Detector {
            name: "d46-commit-in-lifecycle".to_string(),
            run: d46::detect_d46,
        },
        // d47: OPT-IN (io-unsafe-txn, surfaced by transaction-integrity preset).
        Detector {
            name: "d47-io-unsafe-txn".to_string(),
            run: d47::detect_d47,
        },
        // d48: OPT-IN (io-in-loop, surfaced by transaction-integrity preset).
        Detector {
            name: "d48-io-in-loop".to_string(),
            run: d48::detect_d48,
        },
        // d49: OPT-IN (uncommitted-write-before-ui, surfaced by transaction-integrity preset).
        Detector {
            name: "d49-uncommitted-write-before-ui".to_string(),
            run: d49::detect_d49,
        },
        // d50: OPT-IN (checked-run-implicit-commit, advisory info/medium).
        Detector {
            name: "d50-checked-run-implicit-commit".to_string(),
            run: d50::detect_d50,
        },
        // d51: OPT-IN (retry-side-effect-duplication).
        Detector {
            name: "d51-retry-side-effect-duplication".to_string(),
            run: d51::detect_d51,
        },
    ]
}
