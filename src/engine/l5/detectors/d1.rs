//! D1 — database operation inside a loop (direct or through an in-loop call
//! chain). Port of al-sem `src/detectors/d1-db-op-in-loop.ts`.
//!
//! THE most complex L5 detector: it consumes the PW-0 path-walker substrate
//! end-to-end. Its byte-match validates `walk_evidence` + `merge_by_terminal` +
//! `describe_table` + `pick_actionable_anchor` + `classify_op` together.
//!
//! Two emission paths:
//!   (a) DIRECT in-loop db-touching ops in THIS routine → a synthetic two-step
//!       WalkResult (`[loopStep, opStep]`, `effectiveLoopDepth = loopStack.len()`,
//!       no uncertainties).
//!   (b) IN-LOOP CALLS to db-touching callees → `walk_evidence` from the callee,
//!       seeded with `[loopStep, callStep]` and `initial_loop_depth =
//!       cs.loopStack.len()`. Each Complete result's terminal op is recovered from
//!       `last_step.operation_id`.
//!
//! Two-stage collapse: (1) dedup by `id` (first-wins), (2) `merge_by_terminal`
//! (folds M ancestor loops on the same terminal op into one finding with
//! `additionalPaths`). Fingerprint is computed AFTER merge (the union grows
//! affectedTables); then sort by `id`.
//!
//! ## Dependency-role path is DEAD (source-only)
//! al-sem's `terminalsAt` and the finding-build op-recovery both fall back to
//! `summary.dbEffects` for `roleOf(r) === "dependency"` routines. In the
//! SOURCE-ONLY Rust pipeline every routine is primary, so that fallback never
//! engages; it is documented inline but not implemented (mirrors `run_detectors`).

use std::collections::{HashMap, HashSet};

use crate::engine::l3::l3_workspace::L3Table;
use crate::engine::l3::l3_workspace::{L3RecordOperation, L3Resolved, L3Routine};
use crate::engine::l4::combined_graph::CombinedEdge;
use crate::engine::l4::summary::Uncertainty;
use crate::engine::l5::actionable_anchor::pick_actionable_anchor;
use crate::engine::l5::capability_query::{touches_db_of, EffectPresence};
use crate::engine::l5::confidence::{to_confidence, UncertaintyLite};
use crate::engine::l5::detector_context::DetectorContext;
use crate::engine::l5::detectors::anchor_of;
use crate::engine::l5::finding::{
    Evidence, EvidenceStep, Finding, FindingConfidence, FixOption, SourceAnchor,
};
use crate::engine::l5::fingerprint::FingerprintIndex;
use crate::engine::l5::op_classification::{classify_op, is_db_touching_class};
use crate::engine::l5::path_merge::merge_by_terminal;
use crate::engine::l5::path_walker::{
    walk_evidence, PathCtx, Terminal, WalkBounds, WalkOpts, WalkPolicy, WalkResult, WalkStop,
};
use crate::engine::l5::registry::{DetectorOutput, DetectorStats};
use crate::engine::l5::table_display::{describe_table, DescribeOp};

const DETECTOR: &str = "d1-db-op-in-loop";

/// The path-walker's depth/node budget for the interprocedural call-chain walk.
const BOUNDS: WalkBounds = WalkBounds {
    max_depth: 20,
    max_nodes: 500,
};

const WRITE_OPS: [&str; 5] = ["Modify", "ModifyAll", "Insert", "Delete", "DeleteAll"];
const HEAVY_READ_OPS: [&str; 2] = ["CalcFields", "CalcSums"];
const RETRIEVAL_OPS: [&str; 6] = ["FindSet", "FindFirst", "FindLast", "Find", "Get", "Next"];
/// Ops that open a recordset cursor BEFORE a `repeat..until` loop. An in-loop
/// `Next` on the same record-var IS the cursor advance, not an N+1 antipattern.
const CURSOR_OPENER_OPS: [&str; 4] = ["FindSet", "FindFirst", "FindLast", "Find"];

/// `temp_state.kind === "known" && value === true`. A `None` temp_state (al-sem
/// always sets `{kind:"unknown"}`) is NOT a known-temp.
fn is_known_temp(op: &L3RecordOperation) -> bool {
    matches!(&op.temp_state, Some(ts) if ts.kind == "known" && ts.value == Some(true))
}

/// `temp_state.kind !== "known"` (uncertain). A `None` temp_state maps to
/// `{kind:"unknown"}`, which is NOT "known".
fn is_temp_uncertain(op: &L3RecordOperation) -> bool {
    !matches!(&op.temp_state, Some(ts) if ts.kind == "known")
}

/// `describeTable(op, routine, tableById)`. Builds the `DescribeOp` view from an
/// `L3RecordOperation`.
fn describe_op_table(
    op: &L3RecordOperation,
    routine: Option<&L3Routine>,
    table_by_id: &HashMap<&str, &L3Table>,
) -> String {
    let describe = DescribeOp {
        table_id: op.table_id.as_deref(),
        record_variable_name: &op.record_variable_name,
    };
    describe_table(&describe, routine, table_by_id)
}

/// `tableNote(op, routine, tableById)` → `"<Op> on <table>"`.
fn table_note(
    op: &L3RecordOperation,
    routine: Option<&L3Routine>,
    table_by_id: &HashMap<&str, &L3Table>,
) -> String {
    format!(
        "{} on {}",
        op.op,
        describe_op_table(op, routine, table_by_id)
    )
}

/// `isSetupSingletonGet`: op is `Get` AND the rendered table name (minus the
/// `(type not loaded)` suffix) ends in `Setup` (case-insensitive) AND is not a
/// `var ` / `unknown table` / empty placeholder.
fn is_setup_singleton_get(
    op: &L3RecordOperation,
    routine: Option<&L3Routine>,
    table_by_id: &HashMap<&str, &L3Table>,
) -> bool {
    if op.op != "Get" {
        return false;
    }
    let display = describe_op_table(op, routine, table_by_id);
    // Strip the `(type not loaded)` suffix (case-insensitive) then trim.
    let name = strip_type_not_loaded(&display);
    let name = name.trim();
    if name.is_empty() || name.starts_with("var ") || name == "unknown table" {
        return false;
    }
    ends_with_setup_ci(name)
}

/// `display.replace(/\s*\(type not loaded\)$/i, "")`: strip a trailing
/// (case-insensitive) `(type not loaded)` plus any whitespace immediately before
/// it. Anchored at the end only.
fn strip_type_not_loaded(display: &str) -> String {
    // The suffix is pure ASCII, so match it case-insensitively over the trailing
    // BYTES of `display` directly (never via a lowercased copy — `to_lowercase` is
    // not length-preserving, so a byte offset from the lowercased string would slice
    // `display` mid-char for non-ASCII names). A trailing match guarantees the cut
    // byte is `(` (ASCII) → a valid char boundary.
    let suffix = b"(type not loaded)";
    let db = display.as_bytes();
    if db.len() >= suffix.len() {
        let start = db.len() - suffix.len();
        if db[start..].eq_ignore_ascii_case(suffix) {
            return display[..start].trim_end().to_string(); // `\s*` before the suffix
        }
    }
    display.to_string()
}

/// `/\bSetup$/i.test(name)`: the name ends in `Setup` (case-insensitive) on a word
/// boundary. JS `\b`/`\w` are ASCII-only, so the boundary char (from the ORIGINAL
/// `name`, never a lowercased copy) is tested with ASCII word-ness.
fn ends_with_setup_ci(name: &str) -> bool {
    let suf = b"setup";
    let nb = name.as_bytes();
    if nb.len() < suf.len() {
        return false;
    }
    let start = nb.len() - suf.len();
    if !nb[start..].eq_ignore_ascii_case(suf) {
        return false;
    }
    // `start` is a char boundary (nb[start] is the ASCII 's'/'S' of "setup").
    if start == 0 {
        return true; // "Setup" is the whole name — boundary at string start.
    }
    let prev = name[..start].chars().next_back().unwrap();
    !(prev.is_ascii_alphanumeric() || prev == '_')
}

/// `representativeLoopId(loopStack)` — the innermost (last) loop.
fn representative_loop_id(loop_stack: &[String]) -> Option<&str> {
    loop_stack.last().map(|s| s.as_str())
}

/// `severityFor(op, effectiveLoopDepth, isSetupSingleton)`.
fn severity_for(
    op: &L3RecordOperation,
    effective_loop_depth: i64,
    is_setup_singleton: bool,
) -> &'static str {
    if is_known_temp(op) {
        return "info";
    }
    if is_setup_singleton {
        return "info";
    }
    // al-sem orders these as distinct branches (write → high, heavy-read → high,
    // retrieval → medium, db-lock → low, else medium). The write + heavy-read arms
    // both yield "high"; they are merged here (clippy `if_same_then_else`) with the
    // SAME precedence — `op` is in at most one of the disjoint op-sets, so the OR is
    // behaviourally identical to the two ordered branches.
    let mut base: &'static str =
        if WRITE_OPS.contains(&op.op.as_str()) || HEAVY_READ_OPS.contains(&op.op.as_str()) {
            "high" // write inside loop / FlowField materialisation = high
        } else if RETRIEVAL_OPS.contains(&op.op.as_str()) {
            "medium" // pure retrieval = medium
        } else if classify_op(&op.op).as_str() == "db-lock" {
            "low"
        } else {
            "medium"
        };
    if effective_loop_depth >= 2 {
        if base == "high" {
            base = "critical";
        } else if base == "medium" {
            base = "high";
        }
    }
    base
}

/// Convert a walk's accumulated `Uncertainty` set to the `UncertaintyLite` shape
/// `to_confidence` consumes. Mirrors al-sem `describe(u)` id-precedence
/// (callsiteId → operationId → routineId).
fn uncertainty_lites(uncertainties: &[Uncertainty]) -> Vec<UncertaintyLite> {
    uncertainties
        .iter()
        .map(|u| {
            let at = if let Some(cs) = &u.callsite_id {
                cs.clone()
            } else if let Some(op) = &u.operation_id {
                op.clone()
            } else {
                u.routine_id.clone().unwrap_or_default()
            };
            UncertaintyLite {
                kind: u.kind.clone(),
                at,
            }
        })
        .collect()
}

/// `buildFinding(...)` — assemble the internal Finding (fingerprint DEFERRED until
/// after `merge_by_terminal`).
///
/// `terminal_routine_id` is al-sem's `terminalOp.routineId` (a separate field on
/// `RecordOperation`; the Rust `L3RecordOperation` carries no routine id, so the
/// caller threads the owning routine's internal id). `terminal_op_anchor` is the
/// op's INTERNAL `SourceAnchor` (built by the caller via `anchor_of`).
#[allow(clippy::too_many_arguments)]
fn build_finding(
    loop_routine: &L3Routine,
    representative_loop: &str,
    result: &WalkResult,
    terminal_op: &L3RecordOperation,
    terminal_routine_id: &str,
    terminal_op_anchor: SourceAnchor,
    routine_by_id: &HashMap<&str, &L3Routine>,
    table_by_id: &HashMap<&str, &L3Table>,
    role_by_routine: &HashMap<&str, &str>,
) -> Finding {
    let terminal_routine = routine_by_id.get(terminal_routine_id).copied();
    let setup_singleton = is_setup_singleton_get(terminal_op, terminal_routine, table_by_id);
    let severity = severity_for(terminal_op, result.effective_loop_depth, setup_singleton);

    let temp_note = if is_known_temp(terminal_op) {
        " (temporary record — not a SQL round-trip)"
    } else if is_temp_uncertain(terminal_op) {
        " (temp state uncertain)"
    } else {
        ""
    };
    let setup_note = if setup_singleton {
        " (Setup singleton — BC caches Get() per session, so the round-trip happens at most once.)"
    } else {
        ""
    };

    let id = format!(
        "d1/{}/{}/{}",
        representative_loop, terminal_routine_id, terminal_op.id
    );
    let root_cause_key = format!("d1/{}/{}", terminal_routine_id, terminal_op.id);

    let root_cause = format!(
        "A loop in {} reaches {}{}{}.",
        loop_routine.name,
        table_note(terminal_op, terminal_routine, table_by_id),
        temp_note,
        setup_note
    );

    // affectedObjects = sorted-dedup [loopRoutine.objectId, terminalRoutine?.objectId].
    let mut affected_set: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    affected_set.insert(loop_routine.object_id.clone());
    if let Some(tr) = terminal_routine {
        affected_set.insert(tr.object_id.clone());
    }
    let affected_objects: Vec<String> = affected_set.into_iter().collect();

    let affected_tables: Vec<String> = match &terminal_op.table_id {
        Some(t) => vec![t.clone()],
        None => Vec::new(),
    };

    let confidence: FindingConfidence =
        to_confidence(&uncertainty_lites(&result.uncertainties), "likely");

    let fix_options = if setup_singleton {
        vec![FixOption {
            description: "Setup tables are session-cached by BC, so a Get() inside a loop is \
                          typically O(1) after the first hit. Hoist the Get() outside the loop \
                          only if the call site shows up in a CPU profile."
                .to_string(),
            safety: "high".to_string(),
        }]
    } else {
        vec![FixOption {
            description: "Move the database operation outside the loop, or batch it into a \
                          set-based operation."
                .to_string(),
            safety: "medium".to_string(),
        }]
    };

    let mut finding = Finding {
        id,
        root_cause_key,
        detector: DETECTOR.to_string(),
        title: "Database operation inside a loop".to_string(),
        root_cause,
        severity: severity.to_string(),
        confidence,
        primary_location: terminal_op_anchor,
        evidence_path: result.path.clone(),
        additional_paths: None,
        affected_objects,
        affected_tables,
        fix_options,
        provenance: vec![Evidence {
            source: "tree-sitter".to_string(),
            note: None,
        }],
        actionable_anchor: None,
        fingerprint: None,
        event_kind: None,
        cross_extension_subscribers: None,
    };

    let actionable = pick_actionable_anchor(&finding, role_by_routine);
    if actionable.is_some() {
        finding.actionable_anchor = actionable;
    }
    finding
}

/// The D1 WalkPolicy — holds references to the eager indexes the closures read.
struct D1Policy<'a> {
    routine_by_id: &'a HashMap<&'a str, &'a L3Routine>,
    table_by_id: &'a HashMap<&'a str, &'a L3Table>,
    summaries: &'a HashMap<String, crate::engine::l5::full_summary::FullRoutineSummary>,
    edges_by_from: &'a HashMap<String, Vec<CombinedEdge>>,
    call_site_by_id: &'a HashMap<&'a str, &'a crate::engine::l2::features::PCallSite>,
}

impl<'a> WalkPolicy for D1Policy<'a> {
    fn terminals_at(&self, node: &str, _ctx: &PathCtx) -> Vec<Terminal> {
        let Some(r) = self.routine_by_id.get(node).copied() else {
            return Vec::new();
        };
        // Source-only: every routine is primary (roleOf != "dependency"). The
        // dependency `summary.dbEffects` fallback is DEAD here.
        r.record_operations
            .iter()
            .filter(|op| is_db_touching_class(classify_op(&op.op)))
            .map(|op| Terminal {
                routine_id: node.to_string(),
                local_loop_depth: op.loop_stack.len() as i64,
                op_id: Some(op.id.clone()),
            })
            .collect()
    }

    fn expand(&self, node: &str, _ctx: &PathCtx) -> Vec<CombinedEdge> {
        let Some(edges) = self.edges_by_from.get(node) else {
            return Vec::new();
        };
        edges
            .iter()
            .filter(|e| {
                // event fan-out is D2's job
                if e.kind == "event-dispatch" {
                    return false;
                }
                match self.summaries.get(&e.to) {
                    Some(s) => touches_db_of(s) != EffectPresence::No,
                    None => false,
                }
            })
            .cloned()
            .collect()
    }

    fn build_hop_step(&self, edge: &CombinedEdge, _ctx: &PathCtx) -> EvidenceStep {
        let from_routine = self.routine_by_id.get(edge.from.as_str()).copied();
        let cs = edge.callsite_id.as_ref().and_then(|cid| {
            from_routine.and_then(|fr| fr.call_sites.iter().find(|c| &c.id == cid))
        });
        let to_name = self
            .routine_by_id
            .get(edge.to.as_str())
            .map(|r| r.name.clone())
            .unwrap_or_else(|| edge.to.clone());
        let trigger_note = if edge.kind == "implicit-trigger" {
            format!(" (via implicit {to_name} trigger)")
        } else {
            String::new()
        };
        let source_anchor = if let Some(cs) = cs {
            anchor_of(&cs.source_anchor, from_routine.unwrap())
        } else if let Some(fr) = from_routine {
            anchor_of(&fr.source_anchor, fr)
        } else {
            SourceAnchor {
                source_unit_id: String::new(),
                start_line: 0,
                start_column: 0,
                end_line: 0,
                end_column: 0,
                enclosing_routine_id: edge.from.clone(),
                syntax_kind: "call".to_string(),
                normalized_text_hash: None,
                leading_context_hash: None,
                trailing_context_hash: None,
            }
        };
        EvidenceStep {
            routine_id: edge.from.clone(),
            operation_id: None,
            callsite_id: edge.callsite_id.clone(),
            loop_id: None,
            source_anchor,
            note: format!("calls {to_name}{trigger_note}"),
        }
    }

    fn build_terminal_step(&self, t: &Terminal, _ctx: &PathCtx) -> EvidenceStep {
        let routine = self.routine_by_id.get(t.routine_id.as_str()).copied();
        let op = t.op_id.as_ref().and_then(|oid| {
            routine.and_then(|r| r.record_operations.iter().find(|o| &o.id == oid))
        });
        // op is always Some on the primary path (the op_id was just emitted by
        // terminals_at over the SAME routine's record_operations).
        let (op_id, anchor, note) = match op {
            Some(op) => (
                Some(op.id.clone()),
                anchor_of(&op.source_anchor, routine.unwrap()),
                table_note(op, routine, self.table_by_id),
            ),
            None => (
                t.op_id.clone(),
                SourceAnchor {
                    source_unit_id: String::new(),
                    start_line: 0,
                    start_column: 0,
                    end_line: 0,
                    end_column: 0,
                    enclosing_routine_id: t.routine_id.clone(),
                    syntax_kind: String::new(),
                    normalized_text_hash: None,
                    leading_context_hash: None,
                    trailing_context_hash: None,
                },
                String::new(),
            ),
        };
        EvidenceStep {
            routine_id: t.routine_id.clone(),
            operation_id: op_id,
            callsite_id: None,
            loop_id: None,
            source_anchor: anchor,
            note,
        }
    }

    fn loop_depth_of_edge(&self, edge: &CombinedEdge) -> i64 {
        // al-sem `loopDepthOfEdge`: ctx.callSiteById.get(edge.callsiteId).loopStack.length.
        edge.callsite_id
            .as_ref()
            .and_then(|cid| self.call_site_by_id.get(cid.as_str()))
            .map(|cs| cs.loop_stack.len() as i64)
            .unwrap_or(0)
    }
}

pub fn detect_d1(resolved: &L3Resolved, ctx: &DetectorContext) -> DetectorOutput {
    let ws = &resolved.workspace;
    let fp_index = FingerprintIndex::build(&ws.routines, &ws.objects);

    // Source-only role map (every routine primary) — used by pick_actionable_anchor.
    let role_by_routine: HashMap<&str, &str> = ws
        .routines
        .iter()
        .map(|r| (r.id.as_str(), "primary"))
        .collect();

    let mut findings: Vec<Finding> = Vec::new();
    let mut candidates_considered = 0usize;
    let mut skipped_parse_incomplete = 0u64;
    let mut skipped_opaque_callee = 0u64;
    let mut skipped_dynamic_dispatch = 0u64;

    let policy = D1Policy {
        routine_by_id: &ctx.routine_by_id,
        table_by_id: &ctx.table_by_id,
        summaries: &ctx.summaries,
        edges_by_from: &ctx.graph.edges_by_from,
        call_site_by_id: &ctx.call_site_by_id,
    };

    for routine in &ws.routines {
        // roleOf(routine) === "primary": source-only ⇒ always true.
        if !routine.body_available {
            continue;
        }
        if routine.parse_incomplete {
            skipped_parse_incomplete += 1;
            continue;
        }
        candidates_considered += 1;

        let loop_by_id: HashMap<&str, &crate::engine::l2::features::PLoop> =
            routine.loops.iter().map(|l| (l.id.as_str(), l)).collect();

        // Record-vars that had a cursor opened before any loop.
        let mut cursor_opened_record_vars: HashSet<String> = HashSet::new();
        for op in &routine.record_operations {
            if !op.loop_stack.is_empty() {
                continue;
            }
            if !CURSOR_OPENER_OPS.contains(&op.op.as_str()) {
                continue;
            }
            cursor_opened_record_vars.insert(op.record_variable_name.to_lowercase());
        }

        // (a) Direct in-loop DB ops.
        for op in &routine.record_operations {
            if op.loop_stack.is_empty() {
                continue;
            }
            if !is_db_touching_class(classify_op(&op.op)) {
                continue;
            }
            if op.op == "Next"
                && cursor_opened_record_vars.contains(&op.record_variable_name.to_lowercase())
            {
                continue;
            }
            let Some(representative_loop) = representative_loop_id(&op.loop_stack) else {
                continue;
            };
            let Some(loop_info) = loop_by_id.get(representative_loop).copied() else {
                continue;
            };

            let loop_step = EvidenceStep {
                routine_id: routine.id.clone(),
                operation_id: None,
                callsite_id: None,
                loop_id: Some(loop_info.id.clone()),
                source_anchor: anchor_of(&loop_info.source_anchor, routine),
                note: format!("{} loop", loop_info.loop_type),
            };
            let op_step = EvidenceStep {
                routine_id: routine.id.clone(),
                operation_id: Some(op.id.clone()),
                callsite_id: None,
                loop_id: None,
                source_anchor: anchor_of(&op.source_anchor, routine),
                note: table_note(op, Some(routine), &ctx.table_by_id),
            };
            let result = WalkResult {
                path: vec![loop_step, op_step],
                effective_loop_depth: op.loop_stack.len() as i64,
                uncertainties: Vec::new(),
                stop: WalkStop::Complete,
            };
            findings.push(build_finding_internal(
                routine,
                loop_info.id.as_str(),
                &result,
                op,
                routine,
                &ctx.routine_by_id,
                &ctx.table_by_id,
                &role_by_routine,
            ));
        }

        // (b) In-loop calls to DB-touching callees — walk the call chain.
        for cs in &routine.call_sites {
            if cs.loop_stack.is_empty() {
                continue;
            }
            let Some(representative_loop) = representative_loop_id(&cs.loop_stack) else {
                continue;
            };
            let Some(loop_info) = loop_by_id.get(representative_loop).copied() else {
                continue;
            };

            // Resolve the edge from graph.edgesByFrom by callsiteId.
            let edge = ctx.graph.edges_by_from.get(&routine.id).and_then(|edges| {
                edges
                    .iter()
                    .find(|e| e.callsite_id.as_deref() == Some(cs.id.as_str()))
            });
            let Some(edge) = edge else {
                // No resolved edge — opaque callee.
                skipped_opaque_callee += 1;
                continue;
            };
            if edge.kind == "interface" || edge.kind == "dynamic" {
                skipped_dynamic_dispatch += 1;
                continue;
            }
            let Some(callee_summary) = ctx.summaries.get(&edge.to) else {
                continue;
            };
            if touches_db_of(callee_summary) == EffectPresence::No {
                continue;
            }

            let loop_step = EvidenceStep {
                routine_id: routine.id.clone(),
                operation_id: None,
                callsite_id: None,
                loop_id: Some(loop_info.id.clone()),
                source_anchor: anchor_of(&loop_info.source_anchor, routine),
                note: format!("{} loop", loop_info.loop_type),
            };
            let to_name = ctx
                .routine_by_id
                .get(edge.to.as_str())
                .map(|r| r.name.clone())
                .unwrap_or_else(|| edge.to.clone());
            let call_step = EvidenceStep {
                routine_id: routine.id.clone(),
                operation_id: None,
                callsite_id: Some(cs.id.clone()),
                loop_id: None,
                source_anchor: anchor_of(&cs.source_anchor, routine),
                note: format!("calls {to_name}"),
            };

            let results = walk_evidence(
                &edge.to,
                &policy,
                BOUNDS,
                WalkOpts {
                    initial_loop_depth: cs.loop_stack.len() as i64,
                    initial_steps: vec![loop_step, call_step],
                },
                &ctx.uncertainties_by_node,
            );

            for result in &results {
                if result.stop != WalkStop::Complete {
                    continue;
                }
                let Some(last_step) = result.path.last() else {
                    continue;
                };
                let Some(op_id) = last_step.operation_id.as_ref() else {
                    continue;
                };
                let terminal_routine = ctx
                    .routine_by_id
                    .get(last_step.routine_id.as_str())
                    .copied();
                // Primary routines have real RecordOperations; the dep
                // summary.dbEffects fallback is DEAD (source-only).
                let Some(terminal_routine) = terminal_routine else {
                    continue;
                };
                let terminal_op = terminal_routine
                    .record_operations
                    .iter()
                    .find(|o| &o.id == op_id);
                let Some(terminal_op) = terminal_op else {
                    continue;
                };
                findings.push(build_finding_internal(
                    routine,
                    loop_info.id.as_str(),
                    result,
                    terminal_op,
                    terminal_routine,
                    &ctx.routine_by_id,
                    &ctx.table_by_id,
                    &role_by_routine,
                ));
            }
        }
    }

    // Two-stage collapse:
    //   1. Dedupe by id (loop+op pair), first-wins.
    //   2. merge_by_terminal — fold ancestor loops on the same terminal op.
    let mut seen: HashSet<String> = HashSet::new();
    let mut deduped: Vec<Finding> = Vec::new();
    for f in findings {
        if seen.contains(&f.id) {
            continue;
        }
        seen.insert(f.id.clone());
        deduped.push(f);
    }
    let mut merged = merge_by_terminal(deduped);
    // Fingerprint AFTER merge — affectedObjects/affectedTables are unioned.
    // Also count setup-singleton downgrades (rootCause contains "Setup singleton").
    let mut downgraded_setup_singleton = 0u64;
    for f in &mut merged {
        f.fingerprint = Some(fp_index.fingerprint_of(f));
        if f.root_cause.contains("Setup singleton") {
            downgraded_setup_singleton += 1;
        }
    }
    // downgradedToInfo: direct in-loop ops that are known-temp (severity forced to "info").
    // Count the known-temp direct ops (those that entered severity_for with is_known_temp=true).
    // al-sem counts these as they are added to findings (before dedup/merge).
    // In the Rust path, severity "info" from is_known_temp is already baked into the finding;
    // we count them from the merged findings whose severity is "info" due to temp (but
    // setup-singletons also get "info" — exclude those). Use the same rootCause marker: "temporary record".
    let downgraded_to_info = merged
        .iter()
        .filter(|f| f.severity == "info" && f.root_cause.contains("temporary record"))
        .count() as u64;
    // merge_by_terminal already sorts by compareStrings(id); the explicit final
    // sort by id (al-sem `sorted = merged.sort(...)`) is a no-op duplicate but
    // kept for faithfulness.
    merged.sort_by(|a, b| a.id.cmp(&b.id));

    let emitted = merged.len();
    let mut stats = DetectorStats::new(DETECTOR, candidates_considered, emitted);
    stats.add_skip("opaqueCallee", skipped_opaque_callee);
    stats.add_skip("dynamicDispatch", skipped_dynamic_dispatch);
    stats.add_skip("parseIncomplete", skipped_parse_incomplete);
    stats.add_skip("downgradedToInfo", downgraded_to_info);
    stats.add_skip("downgradedSetupSingleton", downgraded_setup_singleton);
    DetectorOutput {
        findings: merged,
        stats,
    }
}

/// Wrapper around `build_finding` that recovers the terminal op's owning-routine
/// id + internal source anchor before delegating. `terminal_routine` is the
/// op's owning routine (the DIRECT case passes `routine`; the call case passes
/// the routine resolved from `last_step.routine_id`).
#[allow(clippy::too_many_arguments)]
fn build_finding_internal(
    loop_routine: &L3Routine,
    representative_loop: &str,
    result: &WalkResult,
    terminal_op: &L3RecordOperation,
    terminal_routine: &L3Routine,
    routine_by_id: &HashMap<&str, &L3Routine>,
    table_by_id: &HashMap<&str, &L3Table>,
    role_by_routine: &HashMap<&str, &str>,
) -> Finding {
    let terminal_op_anchor = anchor_of(&terminal_op.source_anchor, terminal_routine);
    build_finding(
        loop_routine,
        representative_loop,
        result,
        terminal_op,
        terminal_routine.id.as_str(),
        terminal_op_anchor,
        routine_by_id,
        table_by_id,
        role_by_routine,
    )
}
