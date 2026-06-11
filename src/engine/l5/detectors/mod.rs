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

use std::collections::HashMap;

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

/// G-9: page triggers in which the platform has already loaded the implicit
/// `Rec` before the trigger body runs (docs/engine-gaps.md G-9).
const PAGE_TRIGGERS_REC_LOADED: &[&str] = &[
    "OnValidate",
    "OnAction",
    "OnAfterGetRecord",
    "OnDrillDown",
    "OnAfterGetCurrRecord",
];

/// G-9 suppression signal (exact, structural): true iff `routine` is a page
/// trigger (`OnValidate` / `OnAction` / `OnAfterGetRecord` / `OnDrillDown` /
/// `OnAfterGetCurrRecord`) or a table field `OnValidate` trigger, AND the op's
/// receiver is the trigger's implicit current record `Rec`. In all of these the
/// AL platform loaded `Rec` before the trigger ran, so "never loaded" /
/// "never persisted" detectors (d11/d21/d37) must not fire on it. When unsure
/// (any other kind / object type / trigger name / receiver) returns `false` —
/// the detectors keep firing.
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
        // A `trigger OnValidate` in a table object is always a FIELD trigger
        // (table-level triggers are OnInsert/OnModify/OnDelete/OnRename).
        "Table" | "TableExtension" => routine.name.eq_ignore_ascii_case("OnValidate"),
        _ => false,
    }
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

/// G-10 tier 2: does the RESOLVED callee of `cs` perform a recognized load op
/// on the by-`var` record parameter bound to caller variable `var_lc`
/// (lowercased)? One hop only — the callee's own body is inspected directly,
/// never its transitive callees. Every uncertainty returns `false`.
fn callee_loads_by_var_arg(ctx: &DetectorContext, cs: &PCallSite, var_lc: &str) -> bool {
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
        // by-value callee loads its own copy, proving nothing for the caller.
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
        // …must be the receiver of a recognized load op in the callee body.
        let param_lc = param_rv.name.to_lowercase();
        if callee.record_operations.iter().any(|op| {
            RECORD_LOAD_OPS.contains(&op.op.as_str())
                && op.record_variable_name.to_lowercase() == param_lc
        }) {
            return true;
        }
    }
    false
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
