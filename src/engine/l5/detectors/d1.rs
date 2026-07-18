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
use crate::engine::l4::effect_lattice::TempStateKind;
use crate::engine::l4::summary::Uncertainty;
use crate::engine::l5::actionable_anchor::pick_actionable_anchor;
use crate::engine::l5::capability_query::{EffectPresence, touches_db_of};
use crate::engine::l5::closed_world_temp::ClosedWorldTempParams;
use crate::engine::l5::confidence::{UncertaintyLite, to_confidence};
use crate::engine::l5::detector_context::DetectorContext;
use crate::engine::l5::detectors::{
    anchor_of, is_known_temp, is_terminator_next, op_targets_virtual_system_table,
    unquoted_field_name,
};
use crate::engine::l5::finding::{
    Evidence, EvidenceStep, Finding, FindingConfidence, FixOption, SourceAnchor,
};
use crate::engine::l5::op_classification::{classify_op, is_db_touching_class};
use crate::engine::l5::path_merge::{merge_by_terminal, sev_rank};
use crate::engine::l5::path_temp_resolve::resolve_temp_along_path_closed_world;
use crate::engine::l5::path_walker::{
    PathCtx, Terminal, WalkBounds, WalkOpts, WalkPolicy, WalkResult, WalkStop, walk_evidence,
};
use crate::engine::l5::registry::{DetectorError, DetectorOutput, DetectorStats};
use crate::engine::l5::table_display::{DescribeOp, describe_table};

const DETECTOR: &str = "d1-db-op-in-loop";

/// The path-walker's depth/node budget for the interprocedural call-chain walk.
const BOUNDS: WalkBounds = WalkBounds {
    max_depth: 20,
    max_nodes: 500,
};

const WRITE_OPS: [&str; 5] = ["Modify", "ModifyAll", "Insert", "Delete", "DeleteAll"];
const HEAVY_READ_OPS: [&str; 2] = ["CalcFields", "CalcSums"];
/// RV-1 (Task 11): ops whose temp-downgrade is GATED on the field arguments. A
/// FlowField calculation queries the (physical) flow-target tables even on a
/// temporary host record, so the temp ⇒ info downgrade only applies when EVERY
/// named field argument is a non-FlowField (Blob/Normal → in-memory).
const FLOWFIELD_GATED_OPS: [&str; 2] = ["CalcFields", "SetAutoCalcFields"];
const RETRIEVAL_OPS: [&str; 6] = ["FindSet", "FindFirst", "FindLast", "Find", "Get", "Next"];
/// Ops that open a recordset cursor BEFORE a `repeat..until` loop. An in-loop
/// `Next` on the same record-var IS the cursor advance, not an N+1 antipattern.
const CURSOR_OPENER_OPS: [&str; 4] = ["FindSet", "FindFirst", "FindLast", "Find"];

/// The terminal op's `temp_state` as a [`TempStateKind`] (the resolver's input).
/// A `None` temp_state → `Unknown` (al-sem always sets `{kind:"unknown"}` for
/// untracked ops, so the absence maps the same way).
fn op_temp_state_kind(op: &L3RecordOperation) -> TempStateKind {
    match &op.temp_state {
        Some(ts) => TempStateKind::from_p_temp_state(ts),
        None => TempStateKind::Unknown,
    }
}

/// RV-1 (Task 11): the FlowField gate for a temp `CalcFields`/`SetAutoCalcFields`.
///
/// A temporary host record's FlowField is still computed by evaluating its
/// CalcFormula against the (physical) flow-target tables — a real SQL query,
/// host-tempness irrelevant. Blob/Normal field loads ARE in-memory. So the temp ⇒
/// info downgrade may only apply when EVERY named field argument resolves (via the
/// table model) to `field_class != "FlowField"`.
///
/// Returns `true` when the downgrade is BLOCKED (keep firing): ANY field arg is a
/// FlowField, OR any field arg is UNRESOLVABLE (name not in the table, table_id is
/// None, or the table is not in `table_by_id`), OR there are NO capturable field
/// arguments (conservative). Returns `false` only when every field arg is a
/// confirmed non-FlowField → safe to downgrade as in-memory.
///
/// SOUNDNESS: this only ever PREVENTS a downgrade (keeps firing) when uncertain; it
/// never suppresses a finding that would otherwise fire.
fn flowfield_gate_blocks_downgrade(
    op: &L3RecordOperation,
    table_by_id: &HashMap<&str, &L3Table>,
) -> bool {
    // Resolve the op's table; an unresolved table is conservative → block.
    let Some(table_id) = op.table_id.as_deref() else {
        return true;
    };
    let Some(table) = table_by_id.get(table_id).copied() else {
        return true;
    };

    // The named field arguments. `field_argument_infos` carries the structured,
    // unquoted-resolvable form (mirrors d22/d18); an empty/None list means we could
    // not capture any field name → conservative → block.
    let Some(infos) = &op.field_argument_infos else {
        return true;
    };
    if infos.is_empty() {
        return true;
    }

    for info in infos {
        let arg_lc = unquoted_field_name(info).to_lowercase();
        let field = table
            .fields
            .iter()
            .find(|f| f.name.to_lowercase() == arg_lc);
        match field {
            // Unresolvable field name (not in the table) → conservative → block.
            None => return true,
            // ANY FlowField field arg → the calculation queries the flow targets.
            Some(f) if f.field_class == "FlowField" => return true,
            Some(_) => {}
        }
    }
    // Every field arg is a confirmed non-FlowField → in-memory → allow the downgrade.
    false
}

/// The PATH-RESOLVED temp verdict for a single finding (Component 3 / RV-6).
/// Derived from `resolve_temp_along_path` over THIS finding's evidence path:
///   - `Temporary`  ← resolved `Known(true)`  → severity forced to `info`;
///   - `Physical`   ← resolved `Known(false)` → fires at normal severity, no temp note;
///   - `Uncertain`  ← resolved `Unknown`      → fires at normal severity, "(temp state uncertain)".
///   - `FlowFieldGated` ← RV-1 (Task 11): the path resolved `Temporary`, but the
///     terminal op is a `CalcFields`/`SetAutoCalcFields` whose FlowField gate BLOCKS
///     the info-downgrade (a FlowField — or unresolvable — field arg). It fires at
///     NORMAL severity (like `Physical` — no info downgrade) but carries its OWN note
///     (`NOTE_TEMP_FLOWFIELD`): the host record is in-memory yet the FlowField
///     CalcFormula still queries the physical flow targets. A DEDICATED variant (not a
///     faked `Physical`) so the merge-tie reconciliation preserves the FlowField fact
///     in the dual-verdict note instead of silently dropping it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TempVerdict {
    Temporary,
    Physical,
    Uncertain,
    FlowFieldGated,
}

impl TempVerdict {
    fn from_resolved(state: &TempStateKind) -> Self {
        match state {
            TempStateKind::Known(true) => TempVerdict::Temporary,
            TempStateKind::Known(false) => TempVerdict::Physical,
            // PD should never survive resolution (the resolver always returns a
            // concrete Known/Unknown), but a residual PD is treated as uncertain.
            TempStateKind::Unknown | TempStateKind::ParameterDependent(_) => TempVerdict::Uncertain,
        }
    }

    /// The dual-verdict note fragment for this verdict (`temporary` / `physical` /
    /// `uncertain` / `flowfield-on-temp`), used to build the merge-tie note
    /// "temporary via <A>; physical via <B>".
    fn label(self) -> &'static str {
        match self {
            TempVerdict::Temporary => "temporary",
            TempVerdict::Physical => "physical",
            TempVerdict::Uncertain => "uncertain",
            TempVerdict::FlowFieldGated => "flowfield-on-temp",
        }
    }
}

/// A pre-merge finding paired with the data the RV-6 merge-tie reconciliation needs:
/// the PATH-RESOLVED temp verdict and the root caller's display name (the ancestor
/// the path starts in). `loop_routine.name` is the caller label surfaced in the
/// dual-verdict note when paths disagree.
struct FindingRec {
    finding: Finding,
    verdict: TempVerdict,
    caller_label: String,
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
///
/// Component 3 / RV-6 (Task 10): the temp-derived `info` downgrade now keys off the
/// PATH-RESOLVED verdict (`TempVerdict::Temporary`), not the raw `op.temp_state`. A
/// terminal op that is already `Known(true)` resolves `Temporary` immediately (no
/// stepping), so this is BEHAVIOUR-PRESERVING for non-PD ops; only PD-terminal
/// (by-var param) ops gain per-path precision.
fn severity_for(
    op: &L3RecordOperation,
    verdict: TempVerdict,
    effective_loop_depth: i64,
    is_setup_singleton: bool,
) -> &'static str {
    // Only `Temporary` forces the info downgrade. `FlowFieldGated` (RV-1 / Task 11)
    // deliberately does NOT — it fires at the op-based severity, like `Physical`.
    if verdict == TempVerdict::Temporary {
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
    edge_kind_by_callsite: &HashMap<&str, &str>,
    closed_world_temp_params: &ClosedWorldTempParams,
) -> (Finding, TempVerdict) {
    let terminal_routine = routine_by_id.get(terminal_routine_id).copied();
    let setup_singleton = is_setup_singleton_get(terminal_op, terminal_routine, table_by_id);

    // Component 3 / RV-6 (Task 10): resolve the terminal op's temp_state EXACTLY
    // along THIS finding's evidence path. A non-PD op resolves immediately (no
    // stepping) so the verdict equals the raw state — behaviour-preserving. A
    // PD-terminal (by-var param) op resolves per-path: temp on a temp-caller path,
    // physical on a physical-caller path, uncertain at a path root. The edge-kind
    // allowlist guard inside the resolver keeps dynamic/interface/run hops sound.
    // G-19: the closed-world proven set lets a PD frame belonging to a `local`
    // all-temp-callers routine resolve Known(true) even at a path root (the
    // intra-callee shape) — see `closed_world_temp`.
    let resolved = resolve_temp_along_path_closed_world(
        &result.path,
        op_temp_state_kind(terminal_op),
        routine_by_id,
        edge_kind_by_callsite,
        closed_world_temp_params,
    );
    let resolved_verdict = TempVerdict::from_resolved(&resolved);

    // RV-1 (Task 11): the FlowField gate. A temp `CalcFields`/`SetAutoCalcFields`
    // only downgrades to info when EVERY named field arg is a confirmed
    // non-FlowField (Blob/Normal → in-memory). A FlowField — or any unresolvable
    // field arg — keeps the op FIRING because its CalcFormula queries the physical
    // flow targets. When the gate blocks, the verdict becomes the DEDICATED
    // `FlowFieldGated` (fires at normal severity like `Physical`, but carries its own
    // FlowField note) — NOT a faked `Physical`, so the merge-tie reconciliation can
    // preserve the FlowField fact when this path merges with a genuinely-physical one.
    let verdict = if resolved_verdict == TempVerdict::Temporary
        && FLOWFIELD_GATED_OPS.contains(&terminal_op.op.as_str())
        && flowfield_gate_blocks_downgrade(terminal_op, table_by_id)
    {
        TempVerdict::FlowFieldGated
    } else {
        resolved_verdict
    };

    let severity = severity_for(
        terminal_op,
        verdict,
        result.effective_loop_depth,
        setup_singleton,
    );

    let temp_note = match verdict {
        TempVerdict::Temporary => NOTE_TEMPORARY,
        TempVerdict::Uncertain => NOTE_UNCERTAIN,
        TempVerdict::FlowFieldGated => NOTE_TEMP_FLOWFIELD,
        // Physical: a concrete physical record reached along this path — honest
        // omission (no temp note), matching the prior Known(false) `""` branch.
        TempVerdict::Physical => "",
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

    // G-4 (docs/engine-gaps.md): PURE-TRANSITIVE wording. When the terminal op's
    // OWN routine is not the loop routine AND the op sits in no loop of its own
    // (empty loop_stack), the original "A loop in X reaches <op on table>." reads
    // as if the terminal routine loops. The finding is GENUINELY REAL (the op runs
    // once per ancestor iteration — real SQL cost), so the fix is WORDING ONLY:
    // name the terminal routine and attribute the loop to the ancestor explicitly.
    // Severity / confidence / id / rootCauseKey / fingerprint are all unchanged.
    let pure_transitive = terminal_routine_id != loop_routine.id
        && terminal_op.loop_stack.is_empty()
        && terminal_routine.is_some();
    let root_cause = if pure_transitive {
        let tr = terminal_routine.expect("guarded by pure_transitive");
        format!(
            "A loop in {} reaches {} in {}, which has no loop of its own \u{2014} the \
             operation runs once per iteration of that loop{}{}.",
            loop_routine.name,
            table_note(terminal_op, terminal_routine, table_by_id),
            tr.name,
            temp_note,
            setup_note
        )
    } else {
        format!(
            "A loop in {} reaches {}{}{}.",
            loop_routine.name,
            table_note(terminal_op, terminal_routine, table_by_id),
            temp_note,
            setup_note
        )
    };

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
    (finding, verdict)
}

/// G-18 (docs/engine-gaps.md): does this resolved edge's TARGET routine actually
/// carry the call site's own callee name?
///
/// Why this is needed: the internal routine id has NO member discriminator, so
/// two same-name same-signature triggers in one object (two page actions'
/// `OnAction`, two fields' `OnValidate`, …) collide on the id — and with them
/// every derived id (`{rid}/cs{n}`). The combined graph then files BOTH bodies'
/// edges under the one shared `from` key, and a lookup by callsite id alone can
/// return the SIBLING body's edge — splicing an in-loop call site onto a call
/// chain the loop is not on (the G-18 false positive).
///
/// Why it can never reject a genuinely-own edge: the call resolver is NAME-keyed
/// — a `direct`/`method` edge's target routine always carries the call site's
/// callee name (case-insensitive, quotes stripped). Un-nameable callees
/// (object-run / unknown) and out-of-source targets (no routine entry) are
/// ACCEPTED — the pre-G-18 behavior — so the guard only ever filters cross-body
/// edges under a colliding id; it cannot suppress a genuine transitive finding.
/// (Implicit-trigger edges never reach this guard: their `callsite_id` is the
/// record-op id `{rid}/op{n}`, which can never equal a call site's `{rid}/cs{n}`.)
fn edge_target_matches_callsite_callee(
    edge: &CombinedEdge,
    cs: &crate::engine::l2::features::PCallSite,
    routine_by_id: &HashMap<&str, &L3Routine>,
) -> bool {
    use crate::engine::l2::features::PCallee;
    let callee_name = match &cs.callee {
        PCallee::Bare { name } => name,
        PCallee::Member { method, .. } => method,
        // No comparable method name — accept (cannot disambiguate; conservative
        // in the keep-firing direction).
        PCallee::ObjectRun { .. } | PCallee::Unknown => return true,
    };
    let Some(target) = routine_by_id.get(edge.to.as_str()) else {
        return true; // out-of-source target — accept (pre-G-18 behavior)
    };
    crate::engine::l2::node_util::strip_quotes(callee_name).to_lowercase()
        == target.name.to_lowercase()
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
            // G-1: a callee's own `until <var>.Next() …` terminator is the callee
            // loop's advancement, never an actionable db op for ANY ancestor loop —
            // exclude it from the interprocedural terminals too.
            .filter(|op| !is_terminator_next(op))
            // G-6: ops on a BC virtual/system table (AllObjWithCaption, Field, …)
            // read the platform's in-memory metadata store — no SQL round-trip, so
            // they are never d1 terminals for ANY ancestor loop either.
            .filter(|op| !op_targets_virtual_system_table(op, r, self.table_by_id))
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

pub fn detect_d1(
    resolved: &L3Resolved,
    ctx: &DetectorContext,
) -> Result<DetectorOutput, DetectorError> {
    let ws = &resolved.workspace;
    let fp_index = &ctx.fingerprint_index;

    // Source-only role map (every routine primary) — used by pick_actionable_anchor.
    let role_by_routine: HashMap<&str, &str> = ws
        .routines
        .iter()
        .map(|r| (r.id.as_str(), "primary"))
        .collect();

    // Component 3 / RV-6 (Task 10): callsite_id → resolved edge KIND, derived from
    // the combined graph d1 already holds. `resolve_temp_along_path` consults this to
    // enforce the edge-kind allowlist (only `direct | method | implicit-trigger` hops
    // carry usable binding semantics; everything else stops the PD chase → Unknown).
    // First edge per callsite wins (edges_by_from is edgeSortKey-sorted, matching the
    // resolver's deterministic per-callsite view).
    let mut edge_kind_by_callsite: HashMap<&str, &str> = HashMap::new();
    for edges in ctx.graph.edges_by_from.values() {
        for e in edges {
            if let Some(cs) = e.callsite_id.as_deref() {
                edge_kind_by_callsite.entry(cs).or_insert(e.kind.as_str());
            }
        }
    }

    // Each finding is tracked with its PATH-RESOLVED temp verdict + the root caller's
    // display name, so the post-dedup merge-tie pass (RV-6) can reconcile paths that
    // DISAGREE on the temp-derived severity into one finding (worst severity wins +
    // dual-verdict note).
    let mut findings: Vec<FindingRec> = Vec::new();
    let mut candidates_considered = 0usize;
    let mut skipped_parse_incomplete = 0u64;
    let mut skipped_opaque_callee = 0u64;
    let mut skipped_dynamic_dispatch = 0u64;
    // G-6: direct in-loop ops skipped because they target a BC virtual/system
    // table (no SQL backing). Counted PER DIRECT OP, like the other direct skips.
    let mut skipped_virtual_table = 0u64;
    // downgradedToInfo: counted PER DIRECT IN-LOOP OP, PRE-merge, in the direct-op
    // branch only (mirrors d1.ts:320-322). NOT reconstructed post-merge by rootCause
    // text — that would under-count when ≥2 temp ops merge into one finding and
    // over-count transitive (callee-terminal) temp findings TS never counts.
    let mut downgraded_to_info = 0u64;

    let policy = D1Policy {
        routine_by_id: &ctx.routine_by_id,
        table_by_id: &ctx.table_by_id,
        summaries: &ctx.summaries,
        edges_by_from: &ctx.graph.edges_by_from,
        call_site_by_id: &ctx.call_site_by_id,
    };

    for routine in &ws.routines {
        // roleOf(routine) === "primary": source-only ⇒ always true, so the
        // `roleOf(r) !== "primary"` candidate gate was dropped. TRACKED LATENT GAP
        // (applies to ALL primary-scoped default detectors): in cross-app mode this
        // gate's absence would overcount dependency routines in candidatesConsidered.
        // A1's corpus is source-only (all-primary), so it's not exercised; to be
        // locked with a cross-app stats fixture in a later pass.
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
            // G-1: the `until <var>.Next() …` TERMINATOR of the enclosing repeat loop
            // is the loop's own cursor advancement — it cannot be hoisted or removed
            // without breaking the loop, so it is never an actionable finding.
            if is_terminator_next(op) {
                continue;
            }
            // G-6: an op on a BC virtual/system table reads the platform's in-memory
            // metadata store — no physical SQL backing, never a SQL round-trip, so
            // an in-loop read of one is never a d1 finding (docs/engine-gaps.md G-6).
            if op_targets_virtual_system_table(op, routine, &ctx.table_by_id) {
                skipped_virtual_table += 1;
                continue;
            }
            let Some(representative_loop) = representative_loop_id(&op.loop_stack) else {
                continue;
            };
            let Some(loop_info) = loop_by_id.get(representative_loop).copied() else {
                continue;
            };
            // d1.ts:320-322 — known-temp direct op ⇒ severity forced to "info".
            // Count it here, PER OP, before the finding is built (NOT post-merge).
            // RV-1 (Task 11): a known-temp `CalcFields`/`SetAutoCalcFields` whose
            // FlowField gate BLOCKS the downgrade now FIRES (not info), so it must NOT
            // increment `downgradedToInfo`. Exclude the gated ops here so the stat
            // tracks the ops that genuinely downgrade.
            let flowfield_gated_direct = FLOWFIELD_GATED_OPS.contains(&op.op.as_str())
                && flowfield_gate_blocks_downgrade(op, &ctx.table_by_id);
            if is_known_temp(op) && !flowfield_gated_direct {
                downgraded_to_info += 1;
            }

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
            let (finding, verdict) = build_finding_internal(
                routine,
                loop_info.id.as_str(),
                &result,
                op,
                routine,
                &ctx.routine_by_id,
                &ctx.table_by_id,
                &role_by_routine,
                &edge_kind_by_callsite,
                &ctx.closed_world_temp_params,
            );
            findings.push(FindingRec {
                finding,
                verdict,
                caller_label: routine.name.clone(),
            });
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
            //
            // G-18 (docs/engine-gaps.md): the callsite-id match alone is NOT
            // sufficient. Two same-name same-signature triggers in one object
            // (e.g. two page actions, each `trigger OnAction()`) COLLIDE on the
            // internal routine id (`compute_routine_id` carries no member
            // discriminator), so their call-site ids (`{rid}/cs{n}`) collide too
            // and `edges_by_from[{rid}]` mixes BOTH bodies' edges under one key.
            // Picking the sibling body's edge for THIS body's in-loop call site
            // attributed the loop to a call chain it is not on (the CDO batch-7
            // `eDocumentsConfigExists` false positive). The edge's TARGET must
            // also match this call site's own callee name — see
            // `edge_target_matches_callsite_callee` for why this can never
            // reject a genuinely-own edge.
            let edge = ctx.graph.edges_by_from.get(&routine.id).and_then(|edges| {
                edges.iter().find(|e| {
                    e.callsite_id.as_deref() == Some(cs.id.as_str())
                        && edge_target_matches_callsite_callee(e, cs, &ctx.routine_by_id)
                })
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
                let (finding, verdict) = build_finding_internal(
                    routine,
                    loop_info.id.as_str(),
                    result,
                    terminal_op,
                    terminal_routine,
                    &ctx.routine_by_id,
                    &ctx.table_by_id,
                    &role_by_routine,
                    &edge_kind_by_callsite,
                    &ctx.closed_world_temp_params,
                );
                findings.push(FindingRec {
                    finding,
                    verdict,
                    caller_label: routine.name.clone(),
                });
            }
        }
    }

    // Two-stage collapse:
    //   1. Dedupe by id (loop+op pair), first-wins.
    //   1b. RV-6 merge-tie reconciliation (see `reconcile_merge_tie`).
    //   2. merge_by_terminal — fold ancestor loops on the same terminal op.
    let mut seen: HashSet<String> = HashSet::new();
    let mut deduped: Vec<FindingRec> = Vec::new();
    for f in findings {
        if seen.contains(&f.finding.id) {
            continue;
        }
        seen.insert(f.finding.id.clone());
        deduped.push(f);
    }

    // RV-6 merge-tie: `merge_by_terminal` collapses every path sharing a terminal
    // (rootCauseKey) into ONE finding. Post path-resolution, paths in the SAME group
    // can DISAGREE on the temp-derived severity (caller-A path → info/temp; caller-B
    // path → normal/physical). Reconcile BEFORE merge: the WORST severity wins
    // (deterministic, conservative — never let a temp path hide a physical path's
    // finding) AND the temp note lists BOTH verdicts ("temporary via A; physical via
    // B", sorted). Reconcile rewrites every group member to agree so the downstream
    // `merge_by_terminal` (which picks the canonical and lifts its rootCause) emits
    // the reconciled severity + dual-verdict note regardless of which member it picks.
    let deduped = reconcile_merge_tie(deduped);

    let mut merged = merge_by_terminal(deduped);
    // downgradedSetupSingleton: counted POST-merge by rootCause text — TS counts THAT
    // one post-merge too (d1.ts:439), so the text filter is correct here.
    let mut downgraded_setup_singleton = 0u64;
    for f in &mut merged {
        if f.root_cause.contains("Setup singleton") {
            downgraded_setup_singleton += 1;
        }
    }

    // G-7 (docs/engine-gaps.md): DOWN-CONFIDENCE (never suppress) a finding whose
    // EVERY path root routine — the canonical evidence path's first step plus every
    // additionalPaths first step — is provably dead per d14's EXACT criteria
    // (`provably_dead_routine_ids`: unreachable from the entry-point closure +
    // `local`/app-scoped-`internal` + not a Test object + not a property-expression
    // host + not itself a root). d14's dead-determination has its own open-world FPs
    // (e.g. reflection-style invocation the resolver cannot see), so the finding
    // KEEPS FIRING at the SAME severity — only the confidence drops one notch and
    // the rootCause gains an explanatory note. Any live (or merely unprovable) path
    // root keeps full confidence. Runs POST-merge so a terminal reachable from BOTH
    // a dead and a live loop is judged across ALL its merged paths.
    let mut down_confidenced_dead_routine = 0u64;
    if !merged.is_empty() {
        let dead = crate::engine::l5::detectors::d14::provably_dead_routine_ids(resolved, ctx);
        if !dead.is_empty() {
            for f in &mut merged {
                let mut roots: Vec<&str> = Vec::new();
                if let Some(first) = f.evidence_path.first() {
                    roots.push(first.routine_id.as_str());
                }
                for path in f.additional_paths.iter().flatten() {
                    if let Some(first) = path.first() {
                        roots.push(first.routine_id.as_str());
                    }
                }
                if roots.is_empty() || !roots.iter().all(|r| dead.contains(*r)) {
                    continue;
                }
                down_confidenced_dead_routine += 1;
                // One notch down; `possible` is already the floor (al-sem's capped
                // level), so it stays put.
                f.confidence.level = match f.confidence.level.as_str() {
                    "confirmed" => "likely".to_string(),
                    "likely" => "possible".to_string(),
                    other => other.to_string(),
                };
                f.root_cause = insert_temp_note(&f.root_cause, NOTE_DEAD_ROUTINE);
            }
        }
    }
    // Fingerprint AFTER merge — affectedObjects/affectedTables are unioned.
    for f in &mut merged {
        f.fingerprint = Some(fp_index.fingerprint_of(f));
    }
    // merge_by_terminal already sorts by compareStrings(id); the explicit final
    // sort by id (al-sem `sorted = merged.sort(...)`) is a no-op duplicate but
    // kept for faithfulness.
    merged.sort_by(|a, b| a.id.cmp(&b.id));

    let emitted = merged.len();
    let mut stats = DetectorStats::new(DETECTOR, candidates_considered, emitted);
    stats.add_skip("opaqueCallee", skipped_opaque_callee);
    stats.add_skip("dynamicDispatch", skipped_dynamic_dispatch);
    stats.add_skip("parseIncomplete", skipped_parse_incomplete);
    stats.add_skip("virtualTable", skipped_virtual_table);
    stats.add_skip("downgradedToInfo", downgraded_to_info);
    stats.add_skip("downgradedSetupSingleton", downgraded_setup_singleton);
    stats.add_skip("downConfidencedDeadRoutine", down_confidenced_dead_routine);
    Ok(DetectorOutput {
        findings: merged,
        stats,
        diagnostics: vec![],
    })
}

/// The fixed temp-note fragments (leading space included) `build_finding` appends to
/// a finding's rootCause. `reconcile_merge_tie` strips whichever one a member carries
/// before inserting the dual-verdict note.
const NOTE_TEMPORARY: &str = " (temporary record — not a SQL round-trip)";
const NOTE_UNCERTAIN: &str = " (temp state uncertain)";
/// RV-1 (Task 11): the temp-record CalcFields/SetAutoCalcFields finding that the
/// FlowField gate KEEPS FIRING (a FlowField field arg, or an unresolvable one).
/// The host record is in-memory, but the FlowField CalcFormula is evaluated against
/// the physical flow targets — a real SQL round-trip.
const NOTE_TEMP_FLOWFIELD: &str =
    " (temporary record, but FlowField calculation queries the flow targets)";
/// G-7 (docs/engine-gaps.md): appended (with the one-notch confidence drop) when
/// EVERY path root routine of the finding is provably dead per d14's exact
/// criteria. The finding still fires — the loop cost is real IF the routine is
/// ever wired up — but a dead host makes it less actionable today.
const NOTE_DEAD_ROUTINE: &str =
    " (looping routine appears unreachable from any entry point; see d14-dead-routine)";

/// RV-6 merge-tie reconciliation. `merge_by_terminal` groups by `rootCauseKey`; a
/// group whose members DISAGREE on the temp-derived severity must collapse with the
/// WORST severity (conservative) and a note that lists every distinct verdict +
/// caller ("temporary via A; physical via B", sorted). This pass rewrites each tied
/// member so they AGREE before the merge runs (the merge then lifts the canonical's
/// already-reconciled severity + note). Groups whose members already agree on
/// severity are left untouched (byte-preserving for the common single-verdict case).
fn reconcile_merge_tie(recs: Vec<FindingRec>) -> Vec<Finding> {
    // First-seen ordered grouping by rootCauseKey (preserve finding order overall).
    let mut order: Vec<String> = Vec::new();
    let mut group_idx: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, rec) in recs.iter().enumerate() {
        let key = rec.finding.root_cause_key.clone();
        match group_idx.get_mut(&key) {
            Some(v) => v.push(i),
            None => {
                order.push(key.clone());
                group_idx.insert(key, vec![i]);
            }
        }
    }

    let mut recs = recs;
    for key in &order {
        let idxs = &group_idx[key];
        if idxs.len() < 2 {
            continue;
        }
        // A tie needs reconciling iff either (i) the members disagree on severity, OR
        // (ii) RV-1 (Task 11): the members disagree on VERDICT and at least one is
        // `FlowFieldGated`. Case (ii) matters even when severities AGREE: the canonical
        // pick (worst, then position, then id) could otherwise lift a `Physical`
        // member's NOTE-LESS rootCause and silently drop the FlowField fact. Forcing
        // the dual-verdict note here surfaces "flowfield-on-temp via <caller>" so the
        // FlowField fact survives the merge regardless of which member is canonical.
        let first_sev = recs[idxs[0]].finding.severity.clone();
        let severities_agree = idxs.iter().all(|&i| recs[i].finding.severity == first_sev);
        let first_verdict = recs[idxs[0]].verdict;
        let verdicts_agree = idxs.iter().all(|&i| recs[i].verdict == first_verdict);
        let has_flowfield_gated = idxs
            .iter()
            .any(|&i| recs[i].verdict == TempVerdict::FlowFieldGated);
        let needs_reconcile = !severities_agree || (!verdicts_agree && has_flowfield_gated);
        if !needs_reconcile {
            continue;
        }

        // Worst severity wins (deterministic, conservative).
        let worst = idxs
            .iter()
            .map(|&i| recs[i].finding.severity.as_str())
            .max_by_key(|s| sev_rank(s))
            .unwrap_or(first_sev.as_str())
            .to_string();

        // Distinct "<verdict> via <caller>" parts, deduped + sorted for determinism.
        let mut parts: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for &i in idxs {
            let rec = &recs[i];
            parts.insert(format!("{} via {}", rec.verdict.label(), rec.caller_label));
        }
        let dual_note = format!(" (temp state varies by caller: {})", parts_join(&parts));

        // Rewrite every member: worst severity + the dual-verdict temp note (replacing
        // whichever single-verdict note — or none, for physical — the member carried).
        for &i in idxs {
            recs[i].finding.severity = worst.clone();
            let rc = &recs[i].finding.root_cause;
            let stripped = strip_temp_note(rc);
            recs[i].finding.root_cause = insert_temp_note(&stripped, &dual_note);
        }
    }

    recs.into_iter().map(|r| r.finding).collect()
}

/// `parts.join("; ")` over a sorted set.
fn parts_join(parts: &std::collections::BTreeSet<String>) -> String {
    parts.iter().cloned().collect::<Vec<_>>().join("; ")
}

/// Remove the single-verdict temp note (`NOTE_TEMPORARY` / `NOTE_UNCERTAIN`) from a
/// rootCause if present. Physical findings carry no temp note, so a no-op. The note
/// always sits immediately before the trailing setup-note (if any) and the final
/// `.`, so a substring removal is exact.
fn strip_temp_note(root_cause: &str) -> String {
    for note in [NOTE_TEMPORARY, NOTE_UNCERTAIN, NOTE_TEMP_FLOWFIELD] {
        if let Some(pos) = root_cause.find(note) {
            let mut s = root_cause.to_string();
            s.replace_range(pos..pos + note.len(), "");
            return s;
        }
    }
    root_cause.to_string()
}

/// Insert `note` (which carries its own leading space) right before the trailing
/// setup-note/`.`. Both rootCause shapes — `"A loop in X reaches <tableNote>[tempNote]
/// [setupNote]."` and the G-4 pure-transitive `"… in Z, which has no loop of its own
/// — … of that loop[tempNote][setupNote]."` — keep `[tempNote][setupNote].` as the
/// tail, so re-inserting before the setup note if present (else before the final `.`)
/// lands the dual-verdict note exactly where the single-verdict note sat.
fn insert_temp_note(root_cause: &str, note: &str) -> String {
    const SETUP_NOTE_PREFIX: &str = " (Setup singleton";
    if let Some(pos) = root_cause.find(SETUP_NOTE_PREFIX) {
        let mut s = root_cause.to_string();
        s.insert_str(pos, note);
        return s;
    }
    // Insert before the trailing period.
    if let Some(stripped) = root_cause.strip_suffix('.') {
        return format!("{stripped}{note}.");
    }
    format!("{root_cause}{note}")
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
    edge_kind_by_callsite: &HashMap<&str, &str>,
    closed_world_temp_params: &ClosedWorldTempParams,
) -> (Finding, TempVerdict) {
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
        edge_kind_by_callsite,
        closed_world_temp_params,
    )
}
