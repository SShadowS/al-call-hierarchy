//! R4-F Stage-3b — DIGEST witness + effects + occurrence-build path.
//!
//! Byte-parity port of al-sem's digest query slice:
//!   - `src/digest/effect-taxonomy.ts`  → effect_type_of / effect_detail_of / resource_display_of
//!   - `src/query/indexes.ts`           → build_fingerprint_indexes
//!   - `src/query/witness.ts`           → reconstruct_witness_paths (direct + inherited BFS)
//!   - `src/query/hop-projection.ts`    → project_path / project_hop (QueryWitnessHop)
//!   - `src/digest/digest-query.ts`     → digest_query (per-root effect build + dedupe + merge)
//!   - `src/digest/ordering-engine.ts`  → the OCCURRENCE-BUILD slice (canonical_key + occurrence_id)
//!
//! The `occurrenceId` (= `factId`) is the parity crux:
//!
//! ```text
//! factId = sha256Hex( routineId + "|" + linkSignature + "|"
//!                    + (evidenceOperationId? "operation":"callsite") + "|"
//!                    + (evidenceOperationId ?? evidenceCallsiteId ?? "") + "|"
//!                    + effectType )[0..16]
//! ```
//!
//! where `linkSignature = viaPaths[0].map(hop ->
//!   "{fromRoutineId}>{toRoutineId??""}@{callsiteId??""}/{kind}/{edgeId??""}/{""}").join(",")`.
//! `QueryWitnessHop` has NO `edgeId` → always "" → each hop segment ends "//".
//!
//! ## Determinism (R4-F Rev2)
//!
//! - effectMap, the index buckets, seenCanonicalKeys = `IndexMap`-style ordered `Vec`s;
//!   NO `HashMap` iteration reaches output / path-choice / hash.
//! - BFS queue = `VecDeque` (FIFO). All sorts stable (`sort_by`, chained `.cmp`).
//! - The JSON-stringify tiebreak serializer (`hops_json` / `value_source_json`)
//!   reproduces V8 `JSON.stringify` byte-for-byte: struct-field declaration order =
//!   the TS object-literal field order per hop variant, `None`/`undefined` OMITTED,
//!   no whitespace, standard escaping. ASCII corpus → ordinal `str::cmp` everywhere.
//! - Conditionality / transactionContext / guarantees / scopedGuarantees are STAGE 4
//!   and EXCLUDED from this projection.

use std::collections::HashMap;

use serde::Serialize;

use crate::engine::ids::sha256_hex;
use crate::engine::l3::l3_workspace::L3Resolved;
use crate::engine::l5::snapshot::{
    compose_snapshot, CapabilitySnapshot, SnapCapabilityExtra, SnapTempState, SnapValueSource,
    SnapshotCallsiteEvidence, SnapshotGraphEdge,
};

// ===========================================================================
// effect-taxonomy.ts — effectTypeOf / effectDetailOf / resourceDisplayOf
// ===========================================================================

/// `mapOp` (effect-taxonomy.ts). Returns `None` for execute/subscribe/read.
fn map_op(op: &str) -> Option<&'static str> {
    match op {
        "commit" => Some("COMMIT"),
        "insert" => Some("DB_INSERT"),
        "modify" => Some("DB_MODIFY"),
        "delete" => Some("DB_DELETE"),
        "publish" => Some("EVENT_PUBLISH"),
        "send" => Some("HTTP"),
        "store-read" | "store-write" | "store-delete" => Some("ISOLATED_STORAGE"),
        "open" | "write-blob" => Some("FILE"),
        "start" => Some("BACKGROUND_TASK"),
        "log" => Some("TELEMETRY"),
        "ui-message" => Some("UI_MESSAGE"),
        "ui-confirm" => Some("UI_CONFIRM"),
        "ui-error" => Some("UI_ERROR"),
        "ui-window-open" => Some("UI_WINDOW_OPEN"),
        "error-throw" => Some("ERROR_THROW"),
        _ => None,
    }
}

/// The capability fact shape digest reads — the snapshot's `SnapshotCapabilityFact`.
/// We read straight off the composed snapshot (no re-projection).
type Fact = crate::engine::l5::snapshot::SnapshotCapabilityFact;

fn effect_type_of(fact: &Fact) -> Option<&'static str> {
    map_op(&fact.op)
}

/// `resourceDisplayOf` (effect-taxonomy.ts).
fn resource_display_of(
    fact: &Fact,
    stable_id_to_display: &HashMap<String, String>,
) -> Option<String> {
    let rid = fact.resource_id.as_ref()?;
    if fact.resource_kind == "table" {
        let parts: Vec<&str> = rid.split('/').collect();
        if parts.len() == 3 && parts[1] == "table" {
            let stable = format!("{}:Table:{}", parts[0], parts[2]);
            if let Some(name) = stable_id_to_display.get(&stable) {
                if !name.is_empty() {
                    return Some(name.clone());
                }
            }
        }
        return None;
    }
    if fact.resource_kind == "event" {
        let marker = "/event/";
        if let Some(idx) = rid.rfind(marker) {
            let name = &rid[idx + marker.len()..];
            if !name.is_empty() {
                return Some(name.to_string());
            }
        }
        return None;
    }
    None
}

/// `effectDetailOf` (effect-taxonomy.ts). Returns an ORDERED list of (key, value)
/// pairs in insertion order: resourceId, resourceDisplay, (eventClass|method|storageOp), fileOp.
fn effect_detail_of(
    fact: &Fact,
    stable_id_to_display: &HashMap<String, String>,
) -> Vec<(String, String)> {
    let mut detail: Vec<(String, String)> = Vec::new();

    if let Some(rid) = &fact.resource_id {
        detail.push(("resourceId".to_string(), rid.clone()));
    }

    if let Some(display) = resource_display_of(fact, stable_id_to_display) {
        detail.push(("resourceDisplay".to_string(), display));
    }

    match &fact.extra {
        Some(SnapCapabilityExtra::Event { event_class, .. }) => {
            // `fact.extra.eventClass !== undefined` — always present on the event variant.
            detail.push(("eventClass".to_string(), event_class.clone()));
        }
        Some(SnapCapabilityExtra::Http { method, .. }) => {
            detail.push(("method".to_string(), method.clone()));
        }
        Some(SnapCapabilityExtra::Storage { .. }) => {
            detail.push(("storageOp".to_string(), fact.op.clone()));
        }
        _ => {}
    }

    if fact.resource_kind == "file" {
        detail.push(("fileOp".to_string(), fact.op.clone()));
    }

    detail
}

// ===========================================================================
// indexes.ts — buildFingerprintIndexes (ordered buckets, no HashMap-iteration-to-output)
// ===========================================================================

const ROUTINE_ID_SEPARATOR: char = '#';

fn is_routine_stable_id(id: &str) -> bool {
    id.contains(ROUTINE_ID_SEPARATOR)
}

struct FingerprintIndexes<'a> {
    stable_id_to_display: HashMap<String, String>,
    routine_display_by_id: HashMap<String, String>,
    /// Per-from ordered edge bucket (source array order PRESERVED).
    outgoing_edges: HashMap<String, Vec<&'a SnapshotGraphEdge>>,
    /// Per-subject ordered fact bucket (direct ∪ inherited, source order PRESERVED).
    facts_by_routine: HashMap<String, Vec<&'a Fact>>,
    direct_facts_by_routine: HashMap<String, Vec<&'a Fact>>,
    coverage_by_routine: HashMap<String, &'a crate::engine::l5::snapshot::SnapshotCoverageRecord>,
    callsite_by_id: HashMap<String, &'a crate::engine::l5::snapshot::SnapshotCallsiteEvidence>,
    operation_by_id: HashMap<String, &'a crate::engine::l5::snapshot::SnapshotOperationEvidence>,
    event_display_by_id: HashMap<String, String>,
}

fn build_fingerprint_indexes(snap: &CapabilitySnapshot) -> FingerprintIndexes<'_> {
    let mut stable_id_to_display: HashMap<String, String> = HashMap::new();
    let mut routine_display_by_id: HashMap<String, String> = HashMap::new();

    for i in 0..snap.identities.stable_ids.len() {
        let id = snap
            .identities
            .stable_ids
            .get(i)
            .cloned()
            .unwrap_or_default();
        let display = snap
            .identities
            .display_names
            .get(i)
            .cloned()
            .unwrap_or_default();
        if id.is_empty() {
            continue;
        }
        stable_id_to_display.insert(id.clone(), display.clone());
        if is_routine_stable_id(&id) {
            routine_display_by_id.insert(id.clone(), display.clone());
        }
    }

    // outgoingEdges[from] ← typedEdges, ORDER PRESERVED (source array order).
    let mut outgoing_edges: HashMap<String, Vec<&SnapshotGraphEdge>> = HashMap::new();
    for edge in &snap.typed_edges {
        outgoing_edges
            .entry(edge_from(edge).to_string())
            .or_default()
            .push(edge);
    }

    // factsByRoutine ← capabilityFacts; directFactsByRoutine ← provenance=="direct".
    let mut facts_by_routine: HashMap<String, Vec<&Fact>> = HashMap::new();
    let mut direct_facts_by_routine: HashMap<String, Vec<&Fact>> = HashMap::new();
    for fact in &snap.capability_facts {
        facts_by_routine
            .entry(fact.subject.clone())
            .or_default()
            .push(fact);
        if fact.provenance == "direct" {
            direct_facts_by_routine
                .entry(fact.subject.clone())
                .or_default()
                .push(fact);
        }
    }

    let mut coverage_by_routine: HashMap<
        String,
        &crate::engine::l5::snapshot::SnapshotCoverageRecord,
    > = HashMap::new();
    for rec in &snap.coverage {
        coverage_by_routine.insert(rec.subject.clone(), rec);
    }

    let mut callsite_by_id: HashMap<
        String,
        &crate::engine::l5::snapshot::SnapshotCallsiteEvidence,
    > = HashMap::new();
    for cs in &snap.callsite_index {
        callsite_by_id.insert(cs.callsite_id.clone(), cs);
    }

    let mut operation_by_id: HashMap<
        String,
        &crate::engine::l5::snapshot::SnapshotOperationEvidence,
    > = HashMap::new();
    for op in &snap.operation_index {
        operation_by_id.insert(op.operation_id.clone(), op);
    }

    // eventDisplayById ← publisher event declarations; eventName = eventId.split("::")[1].
    let mut event_display_by_id: HashMap<String, String> = HashMap::new();
    for decl in &snap.event_declarations {
        if decl.kind != "publisher" {
            continue;
        }
        let parts: Vec<&str> = decl.event_id.split("::").collect();
        let event_name = parts
            .get(1)
            .map(|s| s.to_string())
            .unwrap_or_else(|| decl.event_id.clone());
        event_display_by_id.insert(decl.event_id.clone(), event_name);
    }

    FingerprintIndexes {
        stable_id_to_display,
        routine_display_by_id,
        outgoing_edges,
        facts_by_routine,
        direct_facts_by_routine,
        coverage_by_routine,
        callsite_by_id,
        operation_by_id,
        event_display_by_id,
    }
}

// ---------------------------------------------------------------------------
// SnapshotGraphEdge accessors digest needs (the snapshot's accessors are private).
// ---------------------------------------------------------------------------

fn edge_kind(e: &SnapshotGraphEdge) -> &str {
    match e {
        SnapshotGraphEdge::DirectCall { kind, .. }
        | SnapshotGraphEdge::VariableTypedCall { kind, .. }
        | SnapshotGraphEdge::InterfaceDispatch { kind, .. }
        | SnapshotGraphEdge::ObjectRunResolved { kind, .. }
        | SnapshotGraphEdge::ObjectRunUnresolved { kind, .. }
        | SnapshotGraphEdge::EventDispatch { kind, .. } => kind,
    }
}

fn edge_from(e: &SnapshotGraphEdge) -> &str {
    match e {
        SnapshotGraphEdge::DirectCall { from, .. }
        | SnapshotGraphEdge::VariableTypedCall { from, .. }
        | SnapshotGraphEdge::InterfaceDispatch { from, .. }
        | SnapshotGraphEdge::ObjectRunResolved { from, .. }
        | SnapshotGraphEdge::ObjectRunUnresolved { from, .. }
        | SnapshotGraphEdge::EventDispatch { from, .. } => from,
    }
}

/// `to` endpoint. None only for object-run-unresolved (no `to`).
fn edge_to(e: &SnapshotGraphEdge) -> Option<&str> {
    match e {
        SnapshotGraphEdge::DirectCall { to, .. }
        | SnapshotGraphEdge::VariableTypedCall { to, .. }
        | SnapshotGraphEdge::InterfaceDispatch { to, .. }
        | SnapshotGraphEdge::ObjectRunResolved { to, .. }
        | SnapshotGraphEdge::EventDispatch { to, .. } => Some(to),
        SnapshotGraphEdge::ObjectRunUnresolved { .. } => None,
    }
}

/// callsiteId for a call-family edge (event-dispatch has none → None).
fn edge_callsite_id(e: &SnapshotGraphEdge) -> Option<&str> {
    match e {
        SnapshotGraphEdge::DirectCall { callsite_id, .. }
        | SnapshotGraphEdge::VariableTypedCall { callsite_id, .. }
        | SnapshotGraphEdge::InterfaceDispatch { callsite_id, .. }
        | SnapshotGraphEdge::ObjectRunResolved { callsite_id, .. }
        | SnapshotGraphEdge::ObjectRunUnresolved { callsite_id, .. } => Some(callsite_id),
        SnapshotGraphEdge::EventDispatch { .. } => None,
    }
}

/// `edgeCompare` (witness.ts:542): kind, then String(callsiteId??""), then String(to??"").
/// Chained `.cmp` (stable). All ordinal.
///
/// **Stability note (witness.ts:542 / V8-stable-preserves-equal-key-order, matches Rust
/// stable sort_by):** the TS comparator uses `?-1:1` (never returns 0), but an empirical
/// 2000-array V8 stress test confirmed that V8's Array.sort preserves insertion order for
/// equal keys under this comparator — identical to Rust's `stable sort_by` returning
/// `Equal`. The current `.cmp` is therefore CORRECT; do NOT change it to `?-1:1`.
fn edge_compare(a: &SnapshotGraphEdge, b: &SnapshotGraphEdge) -> std::cmp::Ordering {
    let ka = edge_kind(a);
    let kb = edge_kind(b);
    if ka != kb {
        return ka.cmp(kb);
    }
    let csa = edge_callsite_id(a).unwrap_or("");
    let csb = edge_callsite_id(b).unwrap_or("");
    if csa != csb {
        return csa.cmp(csb);
    }
    let toa = edge_to(a).unwrap_or("");
    let tob = edge_to(b).unwrap_or("");
    toa.cmp(tob)
}

// ===========================================================================
// witness.ts — WitnessHop union + reconstructWitnessPaths
// ===========================================================================

const HARD_PATH_CAP: usize = 256;
const MAX_DEPTH: usize = 64;
const MAX_STATES: usize = 25_000;

/// Internal WitnessHop union (witness.ts). Field set per the TS literal.
#[derive(Debug, Clone)]
enum WitnessHop {
    Call {
        routine_id: String,
        routine_display: String,
        callee_display: String,
        callsite_id: String,
        source_file: Option<String>,
        line: Option<u32>,
        column: Option<u32>,
    },
    ObjectRun {
        routine_id: String,
        routine_display: String,
        target_object_id: Option<String>,
        target_display: Option<String>,
        resolved: bool,
        callsite_id: Option<String>,
        source_file: Option<String>,
        line: Option<u32>,
        column: Option<u32>,
    },
    EventDispatch {
        routine_id: String,
        routine_display: String,
        event_id: String,
        event_display: String,
    },
    VariableTypedCall {
        routine_id: String,
        routine_display: String,
        receiver_type: String,
        callee_display: Option<String>,
        callsite_id: String,
        source_file: Option<String>,
        line: Option<u32>,
        column: Option<u32>,
    },
    InterfaceDispatch {
        routine_id: String,
        routine_display: String,
        interface_name: String,
        candidate_count: usize,
        callee_display: Option<String>,
        callsite_id: String,
        source_file: Option<String>,
        line: Option<u32>,
        column: Option<u32>,
    },
    Terminal {
        evidence_kind: TerminalKind,
        operation_id: Option<String>,
        callsite_id: Option<String>,
        #[allow(dead_code)]
        display_text: String,
        source_file: Option<String>,
        line: Option<u32>,
        column: Option<u32>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TerminalKind {
    Operation,
    Callsite,
    Synthetic,
}

impl WitnessHop {
    /// `hop.routineId` — the edge destination for every non-terminal hop kind.
    fn routine_id(&self) -> Option<&str> {
        match self {
            WitnessHop::Call { routine_id, .. }
            | WitnessHop::ObjectRun { routine_id, .. }
            | WitnessHop::EventDispatch { routine_id, .. }
            | WitnessHop::VariableTypedCall { routine_id, .. }
            | WitnessHop::InterfaceDispatch { routine_id, .. } => Some(routine_id),
            WitnessHop::Terminal { .. } => None,
        }
    }
}

#[derive(Debug, Clone)]
struct WitnessPath {
    hops: Vec<WitnessHop>,
}

struct WitnessOutcome {
    paths: Vec<WitnessPath>,
    truncated: bool,
}

/// `buildDirectTerminal` (witness.ts).
fn build_direct_terminal(
    evidence_kind: TerminalKind,
    witness_id: &str,
    op_ev: Option<&crate::engine::l5::snapshot::SnapshotOperationEvidence>,
    cs_ev: Option<&crate::engine::l5::snapshot::SnapshotCallsiteEvidence>,
) -> WitnessHop {
    match evidence_kind {
        TerminalKind::Operation => {
            let display_text = op_ev
                .map(|e| e.display_text.clone())
                .unwrap_or_else(|| witness_id.to_string());
            WitnessHop::Terminal {
                evidence_kind: TerminalKind::Operation,
                operation_id: Some(witness_id.to_string()),
                callsite_id: None,
                display_text,
                source_file: op_ev.map(|e| e.source_file.clone()),
                line: op_ev.map(|e| e.start_line),
                column: op_ev.map(|e| e.start_column),
            }
        }
        TerminalKind::Callsite => {
            let display_text = cs_ev
                .map(|e| e.callee_display.clone())
                .unwrap_or_else(|| witness_id.to_string());
            WitnessHop::Terminal {
                evidence_kind: TerminalKind::Callsite,
                operation_id: None,
                callsite_id: Some(witness_id.to_string()),
                display_text,
                source_file: cs_ev.map(|e| e.source_file.clone()),
                line: cs_ev.map(|e| e.start_line),
                column: cs_ev.map(|e| e.start_column),
            }
        }
        TerminalKind::Synthetic => unreachable!("build_direct_terminal not called for synthetic"),
    }
}

/// `terminalHopFromFact` (witness.ts) — terminal for the matched direct fact.
fn terminal_hop_from_fact(fact: &Fact, idx: &FingerprintIndexes) -> WitnessHop {
    if let Some(wo) = &fact.witness_operation_id {
        let ev = idx.operation_by_id.get(wo.as_str()).copied();
        return WitnessHop::Terminal {
            evidence_kind: TerminalKind::Operation,
            operation_id: Some(wo.clone()),
            callsite_id: None,
            display_text: ev
                .map(|e| e.display_text.clone())
                .unwrap_or_else(|| wo.clone()),
            source_file: ev.map(|e| e.source_file.clone()),
            line: ev.map(|e| e.start_line),
            column: ev.map(|e| e.start_column),
        };
    }
    if let Some(wc) = &fact.witness_callsite_id {
        let ev = idx.callsite_by_id.get(wc.as_str()).copied();
        return WitnessHop::Terminal {
            evidence_kind: TerminalKind::Callsite,
            operation_id: None,
            callsite_id: Some(wc.clone()),
            display_text: ev
                .map(|e| e.callee_display.clone())
                .unwrap_or_else(|| wc.clone()),
            source_file: ev.map(|e| e.source_file.clone()),
            line: ev.map(|e| e.start_line),
            column: ev.map(|e| e.start_column),
        };
    }
    WitnessHop::Terminal {
        evidence_kind: TerminalKind::Synthetic,
        operation_id: None,
        callsite_id: None,
        display_text: format!("{} {}", fact.op, fact.resource_kind),
        source_file: None,
        line: None,
        column: None,
    }
}

/// `edgeToHop` (witness.ts). object-run-unresolved → None (BFS cannot walk through).
fn edge_to_hop(edge: &SnapshotGraphEdge, idx: &FingerprintIndexes) -> Option<WitnessHop> {
    match edge {
        SnapshotGraphEdge::DirectCall {
            to, callsite_id, ..
        } => {
            let display = idx
                .routine_display_by_id
                .get(to)
                .cloned()
                .unwrap_or_else(|| to.clone());
            let cs = idx.callsite_by_id.get(callsite_id.as_str()).copied();
            Some(WitnessHop::Call {
                routine_id: to.clone(),
                routine_display: display,
                callee_display: cs.map(|c| c.callee_display.clone()).unwrap_or_default(),
                callsite_id: callsite_id.clone(),
                source_file: cs.map(|c| c.source_file.clone()),
                line: cs.map(|c| c.start_line),
                column: cs.map(|c| c.start_column),
            })
        }
        SnapshotGraphEdge::ObjectRunResolved {
            to,
            callsite_id,
            target_object,
            ..
        } => {
            let display = idx
                .routine_display_by_id
                .get(to)
                .cloned()
                .unwrap_or_else(|| to.clone());
            let cs = idx.callsite_by_id.get(callsite_id.as_str()).copied();
            Some(WitnessHop::ObjectRun {
                routine_id: to.clone(),
                routine_display: display,
                target_object_id: Some(target_object.clone()),
                target_display: idx.stable_id_to_display.get(target_object).cloned(),
                resolved: true,
                callsite_id: Some(callsite_id.clone()),
                source_file: cs.map(|c| c.source_file.clone()),
                line: cs.map(|c| c.start_line),
                column: cs.map(|c| c.start_column),
            })
        }
        SnapshotGraphEdge::ObjectRunUnresolved { .. } => None,
        SnapshotGraphEdge::EventDispatch { to, event_id, .. } => {
            let display = idx
                .routine_display_by_id
                .get(to)
                .cloned()
                .unwrap_or_else(|| to.clone());
            Some(WitnessHop::EventDispatch {
                routine_id: to.clone(),
                routine_display: display,
                event_id: event_id.clone(),
                event_display: idx
                    .event_display_by_id
                    .get(event_id)
                    .cloned()
                    .unwrap_or_else(|| event_id.clone()),
            })
        }
        SnapshotGraphEdge::VariableTypedCall {
            to,
            callsite_id,
            receiver_type,
            ..
        } => {
            let display = idx
                .routine_display_by_id
                .get(to)
                .cloned()
                .unwrap_or_else(|| to.clone());
            let cs = idx.callsite_by_id.get(callsite_id.as_str()).copied();
            Some(WitnessHop::VariableTypedCall {
                routine_id: to.clone(),
                routine_display: display,
                receiver_type: receiver_type.clone(),
                callee_display: cs.map(|c| c.callee_display.clone()),
                callsite_id: callsite_id.clone(),
                source_file: cs.map(|c| c.source_file.clone()),
                line: cs.map(|c| c.start_line),
                column: cs.map(|c| c.start_column),
            })
        }
        SnapshotGraphEdge::InterfaceDispatch {
            to,
            callsite_id,
            interface_name,
            candidate_count,
            ..
        } => {
            let display = idx
                .routine_display_by_id
                .get(to)
                .cloned()
                .unwrap_or_else(|| to.clone());
            let cs = idx.callsite_by_id.get(callsite_id.as_str()).copied();
            Some(WitnessHop::InterfaceDispatch {
                routine_id: to.clone(),
                routine_display: display,
                interface_name: interface_name.clone(),
                candidate_count: *candidate_count,
                callee_display: cs.map(|c| c.callee_display.clone()),
                callsite_id: callsite_id.clone(),
                source_file: cs.map(|c| c.source_file.clone()),
                line: cs.map(|c| c.start_line),
                column: cs.map(|c| c.start_column),
            })
        }
    }
}

/// `factEquivalent` (witness.ts). resourceId compared ONLY when BOTH Some (asymmetric).
/// resourceArgSource compared via canonical JSON when both Some. dispatch → objectType
/// compared (undefined-tolerant).
fn fact_equivalent(a: &Fact, b: &Fact) -> bool {
    if a.op != b.op {
        return false;
    }
    if a.resource_kind != b.resource_kind {
        return false;
    }
    if let (Some(ra), Some(rb)) = (&a.resource_id, &b.resource_id) {
        if ra != rb {
            return false;
        }
    }
    if let (Some(sa), Some(sb)) = (&a.resource_arg_source, &b.resource_arg_source) {
        if value_source_json(sa) != value_source_json(sb) {
            return false;
        }
    }
    let oa = dispatch_object_type(a);
    let ob = dispatch_object_type(b);
    let a_is_dispatch = matches!(&a.extra, Some(SnapCapabilityExtra::Dispatch { .. }));
    let b_is_dispatch = matches!(&b.extra, Some(SnapCapabilityExtra::Dispatch { .. }));
    if a_is_dispatch || b_is_dispatch {
        // TS reads extra?.objectType on both; non-dispatch → undefined. Compare both
        // (undefined-tolerant: only the dispatch variant carries objectType).
        if oa != ob {
            return false;
        }
    }
    true
}

fn dispatch_object_type(f: &Fact) -> Option<&str> {
    match &f.extra {
        Some(SnapCapabilityExtra::Dispatch { object_type, .. }) => Some(object_type.as_str()),
        _ => None,
    }
}

/// `reconstructWitnessPaths(req)` with `limit:"all"` (→ HARD_PATH_CAP).
fn reconstruct_witness_paths(
    root_id: &str,
    fact: &Fact,
    idx: &FingerprintIndexes,
) -> WitnessOutcome {
    let cap = HARD_PATH_CAP;

    if fact.provenance == "direct" {
        // Case A: witnessOperationId → terminal "operation".
        if let Some(wo) = &fact.witness_operation_id {
            let ev = idx.operation_by_id.get(wo.as_str()).copied();
            let hop = build_direct_terminal(TerminalKind::Operation, wo, ev, None);
            return WitnessOutcome {
                paths: vec![WitnessPath { hops: vec![hop] }],
                truncated: false,
            };
        }
        // Case B: only witnessCallsiteId → terminal "callsite".
        if let Some(wc) = &fact.witness_callsite_id {
            let ev = idx.callsite_by_id.get(wc.as_str()).copied();
            let hop = build_direct_terminal(TerminalKind::Callsite, wc, None, ev);
            return WitnessOutcome {
                paths: vec![WitnessPath { hops: vec![hop] }],
                truncated: false,
            };
        }
        // Direct with no witness anchor → synthetic.
        return WitnessOutcome {
            paths: vec![WitnessPath {
                hops: vec![WitnessHop::Terminal {
                    evidence_kind: TerminalKind::Synthetic,
                    operation_id: None,
                    callsite_id: None,
                    display_text: format!("{} {}", fact.op, fact.resource_kind),
                    source_file: None,
                    line: None,
                    column: None,
                }],
            }],
            truncated: false,
        };
    }

    // --- Case C: inherited fact (BFS) ---
    let mut paths: Vec<WitnessPath> = Vec::new();

    let Some(witness_cs) = &fact.witness_callsite_id else {
        // first-hop-not-found (no witnessCallsiteId).
        return WitnessOutcome {
            paths: Vec::new(),
            truncated: false,
        };
    };

    let empty: Vec<&SnapshotGraphEdge> = Vec::new();
    let out_from_root = idx.outgoing_edges.get(root_id).unwrap_or(&empty);
    let first_edges: Vec<&SnapshotGraphEdge> = out_from_root
        .iter()
        .filter(|e| edge_callsite_id(e) == Some(witness_cs.as_str()))
        .copied()
        .collect();
    if first_edges.is_empty() {
        // first-hop-not-found.
        return WitnessOutcome {
            paths: Vec::new(),
            truncated: false,
        };
    }

    struct State {
        routine: String,
        hops: Vec<WitnessHop>,
        visited: std::collections::HashSet<String>,
    }

    let mut queue: std::collections::VecDeque<State> = std::collections::VecDeque::new();
    for edge in &first_edges {
        let Some(hop) = edge_to_hop(edge, idx) else {
            continue;
        };
        let Some(to) = edge_to(edge) else {
            continue;
        };
        let mut visited = std::collections::HashSet::new();
        visited.insert(root_id.to_string());
        visited.insert(to.to_string());
        queue.push_back(State {
            routine: to.to_string(),
            hops: vec![hop],
            visited,
        });
    }

    // seed-sort by routine (`.cmp`, stable). witness.ts:276 uses `(a,b)=> a.routine<b.routine?-1:1`
    // — an empirical V8 2000-array stress test confirmed V8-stable-preserves-equal-key-order for
    // this `?-1:1` comparator, so Rust's stable `.cmp` (which returns Equal on a tie) is CORRECT
    // and preserves typedEdges-insertion order for equal-routine seeds. Do NOT change to `?-1:1`.
    let mut seeds: Vec<State> = queue.into_iter().collect();
    seeds.sort_by(|a, b| a.routine.cmp(&b.routine));
    let mut queue: std::collections::VecDeque<State> = seeds.into_iter().collect();

    let mut state_count = 0usize;
    let mut truncated = false;

    while !queue.is_empty() && paths.len() < cap {
        let Some(state) = queue.pop_front() else {
            break;
        };
        state_count += 1;
        if state_count > MAX_STATES {
            break;
        }
        if state.hops.len() > MAX_DEPTH {
            continue;
        }
        // Terminal check: FIRST matching direct fact in insertion order.
        let directs: &[&Fact] = idx
            .direct_facts_by_routine
            .get(&state.routine)
            .map(|v| v.as_slice())
            .unwrap_or(&[]);
        if let Some(equivalent) = directs.iter().find(|d| fact_equivalent(d, fact)) {
            let terminal = terminal_hop_from_fact(equivalent, idx);
            let mut hops = state.hops.clone();
            hops.push(terminal);
            paths.push(WitnessPath { hops });
            continue;
        }
        // Opaque-or-unresolved-boundary: no out, no directs, coverage.directStatus=="unknown".
        let routine_out_len = idx
            .outgoing_edges
            .get(&state.routine)
            .map(|v| v.len())
            .unwrap_or(0);
        let cov_unknown = idx
            .coverage_by_routine
            .get(&state.routine)
            .map(|c| c.direct_status == "unknown")
            .unwrap_or(false);
        if routine_out_len == 0 && directs.is_empty() && cov_unknown {
            if !state.hops.is_empty() {
                paths.push(WitnessPath {
                    hops: state.hops.clone(),
                });
            }
            continue;
        }
        // Expand: out.clone().sort(edgeCompare) [stable], skip visited.
        let out = idx
            .outgoing_edges
            .get(&state.routine)
            .cloned()
            .unwrap_or_default();
        let mut sorted = out;
        sorted.sort_by(|a, b| edge_compare(a, b));
        for edge in sorted {
            let Some(to) = edge_to(edge) else {
                continue;
            };
            if state.visited.contains(to) {
                continue;
            }
            let Some(hop) = edge_to_hop(edge, idx) else {
                continue;
            };
            let mut new_visited = state.visited.clone();
            new_visited.insert(to.to_string());
            let mut new_hops = state.hops.clone();
            new_hops.push(hop);
            queue.push_back(State {
                routine: to.to_string(),
                hops: new_hops,
                visited: new_visited,
            });
        }
    }

    if paths.len() >= cap && !queue.is_empty() {
        truncated = true;
    }

    // FINAL path sort: shortest-first, then JSON.stringify(raw WitnessHop[]) ordinal tiebreak.
    paths.sort_by(|a, b| {
        if a.hops.len() != b.hops.len() {
            return a.hops.len().cmp(&b.hops.len());
        }
        witness_hops_json(&a.hops).cmp(&witness_hops_json(&b.hops))
    });

    WitnessOutcome { paths, truncated }
}

// ===========================================================================
// hop-projection.ts — QueryWitnessHop + projectPath
// ===========================================================================

/// `QueryWitnessHop` — the projected hop. Field order = the TS literal order in
/// `projectHop` PER variant (see hop-projection.ts). Custom Serialize emits in
/// declaration order, None/undefined OMITTED.
#[derive(Debug, Clone)]
pub struct QueryWitnessHop {
    pub kind: &'static str,
    pub from_routine_id: String,
    pub from_display: String,
    pub to_routine_id: Option<String>,
    pub to_display: Option<String>,
    pub callee_display: Option<String>,
    pub callsite_id: Option<String>,
    pub event_id: Option<String>,
    pub target_app_guid: Option<String>,
    pub edge_kind: Option<String>,
    pub anchor: Option<HopAnchor>,
    pub receiver_type: Option<String>,
    pub interface_name: Option<String>,
    pub candidate_count: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct HopAnchor {
    pub file: String,
    pub line: Option<u32>,
    pub column: Option<u32>,
}

fn hop_anchor(
    source_file: &Option<String>,
    line: Option<u32>,
    column: Option<u32>,
) -> Option<HopAnchor> {
    let file = source_file.as_ref()?;
    // normalizeAnchorPath(file, workspaceRoot): `ws:`-prefixed paths never start with
    // the absolute workspace root → returned verbatim (with `\` → `/`, no-op for ASCII corpus).
    Some(HopAnchor {
        file: file.replace('\\', "/"),
        line,
        column,
    })
}

/// `projectHop` (hop-projection.ts). Terminal hops → None (evidence, not edges).
fn project_hop(
    hop: &WitnessHop,
    from_id: &str,
    from_display: &str,
    idx: &FingerprintIndexes,
) -> Option<QueryWitnessHop> {
    let resolve_to_display = |routine_id: &str, hop_routine_display: &str| -> String {
        idx.routine_display_by_id
            .get(routine_id)
            .cloned()
            .unwrap_or_else(|| hop_routine_display.to_string())
    };

    match hop {
        WitnessHop::Terminal { .. } => None,
        WitnessHop::Call {
            routine_id,
            routine_display,
            callee_display,
            callsite_id,
            source_file,
            line,
            column,
        } => Some(QueryWitnessHop {
            kind: "call",
            from_routine_id: from_id.to_string(),
            from_display: from_display.to_string(),
            to_routine_id: Some(routine_id.clone()),
            to_display: Some(resolve_to_display(routine_id, routine_display)),
            callee_display: Some(callee_display.clone()),
            callsite_id: Some(callsite_id.clone()),
            event_id: None,
            target_app_guid: None,
            edge_kind: Some("direct-call".to_string()),
            anchor: hop_anchor(source_file, *line, *column),
            receiver_type: None,
            interface_name: None,
            candidate_count: None,
        }),
        WitnessHop::ObjectRun {
            routine_id,
            routine_display,
            target_display,
            resolved,
            callsite_id,
            source_file,
            line,
            column,
            ..
        } => Some(QueryWitnessHop {
            kind: "object-run",
            from_routine_id: from_id.to_string(),
            from_display: from_display.to_string(),
            to_routine_id: if *resolved {
                Some(routine_id.clone())
            } else {
                None
            },
            to_display: if *resolved {
                Some(resolve_to_display(routine_id, routine_display))
            } else {
                None
            },
            callee_display: target_display.clone(),
            callsite_id: callsite_id.clone(),
            event_id: None,
            target_app_guid: None,
            edge_kind: Some(
                if *resolved {
                    "object-run-resolved"
                } else {
                    "object-run-unresolved"
                }
                .to_string(),
            ),
            anchor: hop_anchor(source_file, *line, *column),
            receiver_type: None,
            interface_name: None,
            candidate_count: None,
        }),
        WitnessHop::EventDispatch {
            routine_id,
            routine_display,
            event_id,
            ..
        } => Some(QueryWitnessHop {
            kind: "event-dispatch",
            from_routine_id: from_id.to_string(),
            from_display: from_display.to_string(),
            to_routine_id: Some(routine_id.clone()),
            to_display: Some(resolve_to_display(routine_id, routine_display)),
            callee_display: None,
            callsite_id: None,
            event_id: Some(event_id.clone()),
            target_app_guid: None,
            edge_kind: Some("event-dispatch".to_string()),
            anchor: None,
            receiver_type: None,
            interface_name: None,
            candidate_count: None,
        }),
        WitnessHop::VariableTypedCall {
            routine_id,
            routine_display,
            receiver_type,
            callee_display,
            callsite_id,
            source_file,
            line,
            column,
        } => Some(QueryWitnessHop {
            kind: "variable-typed-call",
            from_routine_id: from_id.to_string(),
            from_display: from_display.to_string(),
            to_routine_id: Some(routine_id.clone()),
            to_display: Some(resolve_to_display(routine_id, routine_display)),
            callee_display: callee_display.clone(),
            callsite_id: Some(callsite_id.clone()),
            event_id: None,
            target_app_guid: None,
            edge_kind: Some("variable-typed-call".to_string()),
            anchor: hop_anchor(source_file, *line, *column),
            receiver_type: Some(receiver_type.clone()),
            interface_name: None,
            candidate_count: None,
        }),
        WitnessHop::InterfaceDispatch {
            routine_id,
            routine_display,
            interface_name,
            candidate_count,
            callee_display,
            callsite_id,
            source_file,
            line,
            column,
        } => Some(QueryWitnessHop {
            kind: "interface-dispatch",
            from_routine_id: from_id.to_string(),
            from_display: from_display.to_string(),
            to_routine_id: Some(routine_id.clone()),
            to_display: Some(resolve_to_display(routine_id, routine_display)),
            callee_display: callee_display.clone(),
            callsite_id: Some(callsite_id.clone()),
            event_id: None,
            target_app_guid: None,
            edge_kind: Some("interface-dispatch".to_string()),
            anchor: hop_anchor(source_file, *line, *column),
            receiver_type: None,
            interface_name: Some(interface_name.clone()),
            candidate_count: Some(*candidate_count),
        }),
    }
}

/// `projectPath` (hop-projection.ts). Terminal hops dropped; from-chains head-to-tail.
fn project_path(
    path: &WitnessPath,
    root_id: &str,
    root_display: &str,
    idx: &FingerprintIndexes,
) -> Vec<QueryWitnessHop> {
    let mut hops: Vec<QueryWitnessHop> = Vec::new();
    let mut prev_destination: String = root_id.to_string();

    for hop in &path.hops {
        if matches!(hop, WitnessHop::Terminal { .. }) {
            continue;
        }
        let from_id = prev_destination.clone();
        let from_display = if from_id == root_id {
            root_display.to_string()
        } else {
            idx.routine_display_by_id
                .get(&from_id)
                .cloned()
                .unwrap_or_else(|| from_id.clone())
        };
        if let Some(projected) = project_hop(hop, &from_id, &from_display, idx) {
            hops.push(projected);
        }
        // Advance: hop.routineId is the edge destination for every non-terminal hop.
        if let Some(rid) = hop.routine_id() {
            prev_destination = rid.to_string();
        }
    }

    hops
}

// ===========================================================================
// JSON.stringify TIEBREAK SERIALIZERS — byte-identical to V8 JSON.stringify.
// No whitespace, declaration field order, None/undefined OMITTED.
// ===========================================================================

/// JSON-escape a string per V8 JSON.stringify (standard JSON escaping).
fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0C}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// `JSON.stringify(ValueSource)` — for factEquivalent's resourceArgSource compare.
/// Field order = the snapshot SnapValueSource declaration order (= al-sem ValueSource).
fn value_source_json(vs: &SnapValueSource) -> String {
    match vs {
        SnapValueSource::Literal { value } => {
            format!("{{\"kind\":\"literal\",\"value\":{}}}", json_escape(value))
        }
        SnapValueSource::Enum { enum_name, member } => {
            let mut s = format!(
                "{{\"kind\":\"enum\",\"enumName\":{}",
                json_escape(enum_name)
            );
            if let Some(m) = member {
                s.push_str(&format!(",\"member\":{}", json_escape(m)));
            }
            s.push('}');
            s
        }
        SnapValueSource::ConstantVar {
            var_name,
            initializer,
        } => {
            format!(
                "{{\"kind\":\"constant-var\",\"varName\":{},\"initializer\":{}}}",
                json_escape(var_name),
                value_source_json(initializer)
            )
        }
        SnapValueSource::Parameter { index, var_name } => {
            format!(
                "{{\"kind\":\"parameter\",\"index\":{},\"varName\":{}}}",
                index,
                json_escape(var_name)
            )
        }
        SnapValueSource::TableField {
            table_id,
            field_name,
        } => {
            format!(
                "{{\"kind\":\"table-field\",\"tableId\":{},\"fieldName\":{}}}",
                json_escape(table_id),
                json_escape(field_name)
            )
        }
        SnapValueSource::Expression => "{\"kind\":\"expression\"}".to_string(),
        SnapValueSource::Unknown => "{\"kind\":\"unknown\"}".to_string(),
    }
}

/// `JSON.stringify(WitnessHop[])` — RAW witness hops, for the witness final-sort tiebreak.
/// Field order = the TS WitnessHop union literal order per variant (witness.ts). Optional
/// (undefined) fields OMITTED. Used ONLY for sort stability (never emitted to golden).
fn witness_hops_json(hops: &[WitnessHop]) -> String {
    let mut s = String::from("[");
    for (i, hop) in hops.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push_str(&witness_hop_json(hop));
    }
    s.push(']');
    s
}

fn opt_num(s: &mut String, written: &mut bool, key: &str, v: Option<u32>) {
    if let Some(n) = v {
        if *written {
            s.push(',');
        }
        s.push_str(&format!("{}:{}", json_escape(key), n));
        *written = true;
    }
}

fn opt_str(s: &mut String, written: &mut bool, key: &str, v: &Option<String>) {
    if let Some(val) = v {
        if *written {
            s.push(',');
        }
        s.push_str(&format!("{}:{}", json_escape(key), json_escape(val)));
        *written = true;
    }
}

fn req_str(s: &mut String, written: &mut bool, key: &str, v: &str) {
    if *written {
        s.push(',');
    }
    s.push_str(&format!("{}:{}", json_escape(key), json_escape(v)));
    *written = true;
}

fn witness_hop_json(hop: &WitnessHop) -> String {
    let mut s = String::from("{");
    let mut w = false;
    match hop {
        // WitnessHop union literal field order (witness.ts):
        WitnessHop::Call {
            routine_id,
            routine_display,
            callee_display,
            callsite_id,
            source_file,
            line,
            column,
        } => {
            req_str(&mut s, &mut w, "kind", "call");
            req_str(&mut s, &mut w, "routineId", routine_id);
            req_str(&mut s, &mut w, "routineDisplay", routine_display);
            req_str(&mut s, &mut w, "calleeDisplay", callee_display);
            req_str(&mut s, &mut w, "callsiteId", callsite_id);
            opt_str(&mut s, &mut w, "sourceFile", source_file);
            opt_num(&mut s, &mut w, "line", *line);
            opt_num(&mut s, &mut w, "column", *column);
        }
        WitnessHop::ObjectRun {
            routine_id,
            routine_display,
            target_object_id,
            target_display,
            resolved,
            callsite_id,
            source_file,
            line,
            column,
        } => {
            req_str(&mut s, &mut w, "kind", "object-run");
            req_str(&mut s, &mut w, "routineId", routine_id);
            req_str(&mut s, &mut w, "routineDisplay", routine_display);
            opt_str(&mut s, &mut w, "targetObjectId", target_object_id);
            opt_str(&mut s, &mut w, "targetDisplay", target_display);
            // resolved: boolean (always present)
            if w {
                s.push(',');
            }
            s.push_str(&format!("{}:{}", json_escape("resolved"), resolved));
            w = true;
            opt_str(&mut s, &mut w, "callsiteId", callsite_id);
            opt_str(&mut s, &mut w, "sourceFile", source_file);
            opt_num(&mut s, &mut w, "line", *line);
            opt_num(&mut s, &mut w, "column", *column);
        }
        WitnessHop::EventDispatch {
            routine_id,
            routine_display,
            event_id,
            event_display,
        } => {
            req_str(&mut s, &mut w, "kind", "event-dispatch");
            req_str(&mut s, &mut w, "routineId", routine_id);
            req_str(&mut s, &mut w, "routineDisplay", routine_display);
            req_str(&mut s, &mut w, "eventId", event_id);
            req_str(&mut s, &mut w, "eventDisplay", event_display);
        }
        WitnessHop::VariableTypedCall {
            routine_id,
            routine_display,
            receiver_type,
            callee_display,
            callsite_id,
            source_file,
            line,
            column,
        } => {
            req_str(&mut s, &mut w, "kind", "variable-typed-call");
            req_str(&mut s, &mut w, "routineId", routine_id);
            req_str(&mut s, &mut w, "routineDisplay", routine_display);
            req_str(&mut s, &mut w, "receiverType", receiver_type);
            opt_str(&mut s, &mut w, "calleeDisplay", callee_display);
            req_str(&mut s, &mut w, "callsiteId", callsite_id);
            opt_str(&mut s, &mut w, "sourceFile", source_file);
            opt_num(&mut s, &mut w, "line", *line);
            opt_num(&mut s, &mut w, "column", *column);
        }
        WitnessHop::InterfaceDispatch {
            routine_id,
            routine_display,
            interface_name,
            candidate_count,
            callee_display,
            callsite_id,
            source_file,
            line,
            column,
        } => {
            req_str(&mut s, &mut w, "kind", "interface-dispatch");
            req_str(&mut s, &mut w, "routineId", routine_id);
            req_str(&mut s, &mut w, "routineDisplay", routine_display);
            req_str(&mut s, &mut w, "interfaceName", interface_name);
            if w {
                s.push(',');
            }
            s.push_str(&format!(
                "{}:{}",
                json_escape("candidateCount"),
                candidate_count
            ));
            w = true;
            opt_str(&mut s, &mut w, "calleeDisplay", callee_display);
            req_str(&mut s, &mut w, "callsiteId", callsite_id);
            opt_str(&mut s, &mut w, "sourceFile", source_file);
            opt_num(&mut s, &mut w, "line", *line);
            opt_num(&mut s, &mut w, "column", *column);
        }
        WitnessHop::Terminal {
            evidence_kind,
            operation_id,
            callsite_id,
            display_text,
            source_file,
            line,
            column,
        } => {
            req_str(&mut s, &mut w, "kind", "terminal");
            let ek = match evidence_kind {
                TerminalKind::Operation => "operation",
                TerminalKind::Callsite => "callsite",
                TerminalKind::Synthetic => "synthetic",
            };
            req_str(&mut s, &mut w, "evidenceKind", ek);
            opt_str(&mut s, &mut w, "operationId", operation_id);
            opt_str(&mut s, &mut w, "callsiteId", callsite_id);
            req_str(&mut s, &mut w, "displayText", display_text);
            opt_str(&mut s, &mut w, "sourceFile", source_file);
            opt_num(&mut s, &mut w, "line", *line);
            opt_num(&mut s, &mut w, "column", *column);
        }
    }
    s.push('}');
    s
}

/// `JSON.stringify(QueryWitnessHop[])` — projected hops, for the digest merge tiebreak
/// + exact-dup dedupe. Field order = the QueryWitnessHop literal per projectHop variant.
///
/// Optional (undefined) fields OMITTED.
fn query_hops_json(hops: &[QueryWitnessHop]) -> String {
    let mut s = String::from("[");
    for (i, hop) in hops.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push_str(&query_hop_json(hop));
    }
    s.push(']');
    s
}

fn query_hop_json(hop: &QueryWitnessHop) -> String {
    // Field order = QueryWitnessHop literal in projectHop (per-kind, but the object
    // literal lists the same field sequence). projectHop builds each variant in the
    // order: kind, fromRoutineId, fromDisplay, toRoutineId, toDisplay, calleeDisplay,
    // callsiteId, eventId, targetAppGuid, edgeKind, receiverType, interfaceName,
    // candidateCount, anchor — BUT the exact V8 order is the literal order PER variant.
    // The TS literals place `anchor` LAST in call/object-run/var-typed/iface/dep-export
    // and `edgeKind` last in event-dispatch (no anchor). We reproduce per-kind below.
    let mut s = String::from("{");
    let mut w = false;
    match hop.kind {
        "call" => {
            req_str(&mut s, &mut w, "kind", "call");
            req_str(&mut s, &mut w, "fromRoutineId", &hop.from_routine_id);
            req_str(&mut s, &mut w, "fromDisplay", &hop.from_display);
            opt_str(&mut s, &mut w, "toRoutineId", &hop.to_routine_id);
            opt_str(&mut s, &mut w, "toDisplay", &hop.to_display);
            opt_str(&mut s, &mut w, "calleeDisplay", &hop.callee_display);
            opt_str(&mut s, &mut w, "callsiteId", &hop.callsite_id);
            opt_str(&mut s, &mut w, "edgeKind", &hop.edge_kind);
            opt_anchor(&mut s, &mut w, "anchor", &hop.anchor);
        }
        "object-run" => {
            req_str(&mut s, &mut w, "kind", "object-run");
            req_str(&mut s, &mut w, "fromRoutineId", &hop.from_routine_id);
            req_str(&mut s, &mut w, "fromDisplay", &hop.from_display);
            opt_str(&mut s, &mut w, "toRoutineId", &hop.to_routine_id);
            opt_str(&mut s, &mut w, "toDisplay", &hop.to_display);
            opt_str(&mut s, &mut w, "calleeDisplay", &hop.callee_display);
            opt_str(&mut s, &mut w, "callsiteId", &hop.callsite_id);
            opt_str(&mut s, &mut w, "edgeKind", &hop.edge_kind);
            opt_anchor(&mut s, &mut w, "anchor", &hop.anchor);
        }
        "event-dispatch" => {
            req_str(&mut s, &mut w, "kind", "event-dispatch");
            req_str(&mut s, &mut w, "fromRoutineId", &hop.from_routine_id);
            req_str(&mut s, &mut w, "fromDisplay", &hop.from_display);
            opt_str(&mut s, &mut w, "toRoutineId", &hop.to_routine_id);
            opt_str(&mut s, &mut w, "toDisplay", &hop.to_display);
            opt_str(&mut s, &mut w, "eventId", &hop.event_id);
            opt_str(&mut s, &mut w, "edgeKind", &hop.edge_kind);
        }
        "implicit-trigger" => {
            req_str(&mut s, &mut w, "kind", "implicit-trigger");
            req_str(&mut s, &mut w, "fromRoutineId", &hop.from_routine_id);
            req_str(&mut s, &mut w, "fromDisplay", &hop.from_display);
            opt_str(&mut s, &mut w, "toRoutineId", &hop.to_routine_id);
            opt_str(&mut s, &mut w, "toDisplay", &hop.to_display);
            opt_str(&mut s, &mut w, "edgeKind", &hop.edge_kind);
            opt_anchor(&mut s, &mut w, "anchor", &hop.anchor);
        }
        "dependency-export" => {
            req_str(&mut s, &mut w, "kind", "dependency-export");
            req_str(&mut s, &mut w, "fromRoutineId", &hop.from_routine_id);
            req_str(&mut s, &mut w, "fromDisplay", &hop.from_display);
            opt_str(&mut s, &mut w, "toRoutineId", &hop.to_routine_id);
            opt_str(&mut s, &mut w, "toDisplay", &hop.to_display);
            opt_str(&mut s, &mut w, "calleeDisplay", &hop.callee_display);
            opt_str(&mut s, &mut w, "callsiteId", &hop.callsite_id);
            opt_str(&mut s, &mut w, "targetAppGuid", &hop.target_app_guid);
            opt_str(&mut s, &mut w, "edgeKind", &hop.edge_kind);
            opt_anchor(&mut s, &mut w, "anchor", &hop.anchor);
        }
        "variable-typed-call" => {
            req_str(&mut s, &mut w, "kind", "variable-typed-call");
            req_str(&mut s, &mut w, "fromRoutineId", &hop.from_routine_id);
            req_str(&mut s, &mut w, "fromDisplay", &hop.from_display);
            opt_str(&mut s, &mut w, "toRoutineId", &hop.to_routine_id);
            opt_str(&mut s, &mut w, "toDisplay", &hop.to_display);
            opt_str(&mut s, &mut w, "calleeDisplay", &hop.callee_display);
            opt_str(&mut s, &mut w, "callsiteId", &hop.callsite_id);
            opt_str(&mut s, &mut w, "edgeKind", &hop.edge_kind);
            opt_str(&mut s, &mut w, "receiverType", &hop.receiver_type);
            opt_anchor(&mut s, &mut w, "anchor", &hop.anchor);
        }
        "interface-dispatch" => {
            req_str(&mut s, &mut w, "kind", "interface-dispatch");
            req_str(&mut s, &mut w, "fromRoutineId", &hop.from_routine_id);
            req_str(&mut s, &mut w, "fromDisplay", &hop.from_display);
            opt_str(&mut s, &mut w, "toRoutineId", &hop.to_routine_id);
            opt_str(&mut s, &mut w, "toDisplay", &hop.to_display);
            opt_str(&mut s, &mut w, "calleeDisplay", &hop.callee_display);
            opt_str(&mut s, &mut w, "callsiteId", &hop.callsite_id);
            opt_str(&mut s, &mut w, "edgeKind", &hop.edge_kind);
            opt_str(&mut s, &mut w, "interfaceName", &hop.interface_name);
            if let Some(cc) = hop.candidate_count {
                if w {
                    s.push(',');
                }
                s.push_str(&format!("{}:{}", json_escape("candidateCount"), cc));
                w = true;
            }
            opt_anchor(&mut s, &mut w, "anchor", &hop.anchor);
        }
        _ => {}
    }
    s.push('}');
    s
}

fn opt_anchor(s: &mut String, written: &mut bool, key: &str, anchor: &Option<HopAnchor>) {
    if let Some(a) = anchor {
        if *written {
            s.push(',');
        }
        // SourceAnchorContract literal order: sourceKind, file, line, column (line/column
        // optional). normalizeAnchorPath produces { sourceKind:"source", file, line, column }.
        let mut inner = String::from("{");
        let mut iw = false;
        req_str(&mut inner, &mut iw, "sourceKind", "source");
        req_str(&mut inner, &mut iw, "file", &a.file);
        opt_num(&mut inner, &mut iw, "line", a.line);
        opt_num(&mut inner, &mut iw, "column", a.column);
        inner.push('}');
        s.push_str(&format!("{}:{}", json_escape(key), inner));
        *written = true;
    }
}

// ===========================================================================
// digest-query.ts — digestQuery driver (per-root effect build, dedupe, merge)
// + ordering-engine.ts occurrence-build slice.
// ===========================================================================

/// The terminal hop of a path (LAST terminal in hops, scanning back).
fn find_terminal(hops: &[WitnessHop]) -> Option<&WitnessHop> {
    hops.iter()
        .rev()
        .find(|h| matches!(h, WitnessHop::Terminal { .. }))
}

/// SourceAnchorContract — the evidence form. `unavailable` when no citable anchor.
#[derive(Debug, Clone)]
struct SourceAnchorContract {
    source_kind: &'static str, // "source" | "unavailable"
    file: Option<String>,
    line: Option<u32>,
    column: Option<u32>,
    excerpt: Option<String>,
}

/// `evidenceFromTerminalHop` (digest-query.ts).
fn evidence_from_terminal(terminal: Option<&WitnessHop>) -> SourceAnchorContract {
    match terminal {
        Some(WitnessHop::Terminal {
            source_file: Some(sf),
            line,
            column,
            display_text,
            ..
        }) => SourceAnchorContract {
            source_kind: "source",
            file: Some(sf.replace('\\', "/")),
            line: *line,
            column: *column,
            excerpt: Some(display_text.clone()),
        },
        _ => SourceAnchorContract {
            source_kind: "unavailable",
            file: None,
            line: None,
            column: None,
            excerpt: None,
        },
    }
}

/// `dedupeKey` (digest-query.ts).
fn dedupe_key(
    effect_type: &str,
    terminal: Option<&WitnessHop>,
    fact: &Fact,
    detail: &[(String, String)],
) -> String {
    let anchor_id = match terminal {
        Some(WitnessHop::Terminal {
            operation_id: Some(op),
            ..
        }) => format!("op:{op}"),
        Some(WitnessHop::Terminal {
            callsite_id: Some(cs),
            ..
        }) => format!("cs:{cs}"),
        Some(WitnessHop::Terminal {
            source_file: Some(sf),
            line,
            ..
        }) => {
            format!("file:{}:{}", sf, line.unwrap_or(0))
        }
        _ => format!(
            "synthetic:{}:{}:{}",
            fact.op,
            fact.resource_kind,
            fact.resource_id.clone().unwrap_or_default()
        ),
    };
    let resource_id = fact.resource_id.clone().unwrap_or_default();
    format!(
        "{}|{}|{}|{}|{}",
        effect_type,
        anchor_id,
        fact.resource_kind,
        resource_id,
        detail_json(detail)
    )
}

/// `JSON.stringify(Record<string,string>)` — insertion-ordered detail object.
fn detail_json(detail: &[(String, String)]) -> String {
    let mut s = String::from("{");
    for (i, (k, v)) in detail.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push_str(&format!("{}:{}", json_escape(k), json_escape(v)));
    }
    s.push('}');
    s
}

/// `buildCanonicalOccurrenceKey` (ordering-engine.ts) — link-signature from viaPaths[0].
fn build_canonical_key(
    root_routine_id: &str,
    via_paths: &[Vec<QueryWitnessHop>],
    terminal_evidence_kind: &str,
    terminal_evidence_id: &str,
    effect_type: &str,
) -> (String, String) {
    let mut link_signature = String::new();
    if let Some(first_path) = via_paths.first() {
        if !first_path.is_empty() {
            let segments: Vec<String> = first_path
                .iter()
                .map(|hop| {
                    // QueryWitnessHop has NO edgeId → "". Trailing slot also "".
                    format!(
                        "{}>{}@{}/{}//",
                        hop.from_routine_id,
                        hop.to_routine_id.clone().unwrap_or_default(),
                        hop.callsite_id.clone().unwrap_or_default(),
                        hop.kind
                    )
                })
                .collect();
            link_signature = segments.join(",");
        }
    }

    let canonical_key = [
        root_routine_id,
        link_signature.as_str(),
        terminal_evidence_kind,
        terminal_evidence_id,
        effect_type,
    ]
    .join("|");

    (canonical_key, link_signature)
}

/// `buildOccurrenceIdFromKey` (ordering-engine.ts) — ordinal 0 in practice.
fn occurrence_id_from_key(canonical_key: &str, ordinal: u32) -> String {
    let raw = if ordinal == 0 {
        canonical_key.to_string()
    } else {
        format!("{canonical_key}#{ordinal}")
    };
    sha256_hex(&raw)[..16].to_string()
}

// ---------------------------------------------------------------------------
// Result types (the R4-F golden shape).
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct DigestEffectResult {
    pub effect_type: String,
    pub detail: Vec<(String, String)>,
    pub provenance: &'static str,
    pub evidence: ProjectedEvidence,
    pub evidence_operation_id: Option<String>,
    pub evidence_callsite_id: Option<String>,
    pub via_paths: Vec<Vec<ProjectedHop>>,
    pub via_paths_truncated: bool,
    pub fact_id: String,
    /// The originating CapabilityFact's subject stable id — used for the
    /// evidence-unavailable diagnostic's `factSubject` field.
    pub fact_subject: String,
    pub canonical_key: String,
    pub link_signature: String,
    /// Sort key fields (evidence.file, evidence.line) — for the effects sort.
    sort_file: String,
    sort_line: u32,
    /// S4-internal (NOT serialized in the digest-effects golden): tempState fed to
    /// the ordering engine's physical-write filter.
    pub temp_state: Option<SnapTempState>,
    /// S4: per-effect scoped guarantees (attached by `compute_ordering`).
    pub scoped_guarantees: Vec<crate::engine::l5::ordering_engine::ScopedGuarantee>,
}

#[derive(Debug, Clone)]
pub struct ProjectedEvidence {
    pub source_kind: &'static str,
    pub file: Option<String>,
    pub line: Option<u32>,
    pub column: Option<u32>,
    pub excerpt: Option<String>,
}

/// A projected hop carried into the result (already V8-field-ordered on serialize).
#[derive(Debug, Clone)]
pub struct ProjectedHop {
    pub inner: QueryWitnessHop,
}

#[derive(Debug, Clone)]
pub struct DigestEntryResult {
    pub routine_id: String,
    pub effects: Vec<DigestEffectResult>,
}

/// `digestQuery` (digest-query.ts) — per-root effect build. Roots in input order;
/// entries sorted by routineId at the end. `return_summaries` + `isolated_event_ids`
/// drive the S4 ordering engine (compute_ordering) — None for the S3-only path.
fn digest_query(
    snap: &CapabilitySnapshot,
    roots: &[String],
    return_summaries: Option<&HashMap<String, crate::engine::return_summary::RoutineReturnSummary>>,
    isolated_event_ids: Option<&std::collections::HashSet<String>>,
) -> Vec<DigestEntryResult> {
    const MAX_PATHS: usize = 3;
    let idx = build_fingerprint_indexes(snap);
    let mut entries: Vec<DigestEntryResult> = Vec::new();

    // callsiteById (&str-keyed) for the ordering engine's cross-hop substrate.
    let mut callsite_by_id_str: HashMap<&str, &SnapshotCallsiteEvidence> = HashMap::new();
    for cs in &snap.callsite_index {
        callsite_by_id_str.insert(cs.callsite_id.as_str(), cs);
    }

    for rid in roots {
        let Some(display) = idx.routine_display_by_id.get(rid).cloned() else {
            continue;
        };

        let empty_facts: Vec<&Fact> = Vec::new();
        let all_facts = idx.facts_by_routine.get(rid).unwrap_or(&empty_facts);

        // Effect-fact filter: drop op=="execute"&&dispatch; keep iff effectTypeOf!=None.
        let effect_facts: Vec<&Fact> = all_facts
            .iter()
            .copied()
            .filter(|f| {
                if f.op == "execute"
                    && matches!(&f.extra, Some(SnapCapabilityExtra::Dispatch { .. }))
                {
                    return false;
                }
                effect_type_of(f).is_some()
            })
            .collect();

        // effectMap (IndexMap, insertion order) — ordered Vec of (key, AccumulatedEffect).
        struct AccumulatedEffect {
            effect_type: &'static str,
            detail: Vec<(String, String)>,
            provenance: &'static str,
            evidence: SourceAnchorContract,
            evidence_operation_id: Option<String>,
            evidence_callsite_id: Option<String>,
            via_paths: Vec<Vec<QueryWitnessHop>>,
            had_truncation: bool,
            all_paths: Vec<Vec<QueryWitnessHop>>,
            /// S4-internal (NOT serialized in the digest-effects golden): the
            /// originating table-write fact's tempState (for the physical-write filter).
            temp_state: Option<SnapTempState>,
            /// The fact's subject (stable id) — used for the evidence-unavailable diagnostic.
            fact_subject: String,
        }
        let mut effect_map: Vec<(String, AccumulatedEffect)> = Vec::new();

        for fact in &effect_facts {
            let Some(effect_type) = effect_type_of(fact) else {
                continue;
            };
            let detail = effect_detail_of(fact, &idx.stable_id_to_display);

            let outcome = reconstruct_witness_paths(rid, fact, &idx);

            let shortest = outcome.paths.first();
            let terminal = shortest.and_then(|p| find_terminal(&p.hops));

            let evidence = evidence_from_terminal(terminal);
            let evidence_operation_id = match terminal {
                Some(WitnessHop::Terminal {
                    operation_id: Some(op),
                    ..
                }) => Some(op.clone()),
                _ => None,
            };
            let evidence_callsite_id = match terminal {
                Some(WitnessHop::Terminal {
                    callsite_id: Some(cs),
                    ..
                }) => Some(cs.clone()),
                _ => None,
            };

            // Project all paths to QueryWitnessHop[][].
            let projected_paths: Vec<Vec<QueryWitnessHop>> = outcome
                .paths
                .iter()
                .map(|p| project_path(p, rid, &display, &idx))
                .collect();

            let key = dedupe_key(effect_type, terminal, fact, &detail);

            // tempState of the originating table-write fact (physical-write filter).
            let fact_temp_state: Option<SnapTempState> = match &fact.extra {
                Some(SnapCapabilityExtra::Table { temp_state, .. }) => temp_state.clone(),
                _ => None,
            };

            // Find existing in ordered effect_map.
            let existing_pos = effect_map.iter().position(|(k, _)| k == &key);

            if let Some(pos) = existing_pos {
                // Merge: combine all paths, sort shortest-first + JSON tiebreak, dedupe exact dups.
                let mut merged: Vec<Vec<QueryWitnessHop>> = Vec::new();
                merged.extend(effect_map[pos].1.all_paths.clone());
                merged.extend(projected_paths.clone());
                merged.sort_by(|a, b| {
                    if a.len() != b.len() {
                        return a.len().cmp(&b.len());
                    }
                    query_hops_json(a).cmp(&query_hops_json(b))
                });
                let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
                let mut unique: Vec<Vec<QueryWitnessHop>> = Vec::new();
                for p in merged {
                    let k = query_hops_json(&p);
                    if seen.insert(k) {
                        unique.push(p);
                    }
                }
                let had_truncation = effect_map[pos].1.had_truncation
                    || outcome.truncated
                    || unique.len() > MAX_PATHS;
                let via: Vec<Vec<QueryWitnessHop>> =
                    unique.iter().take(MAX_PATHS).cloned().collect();
                // Merge tempState conservatively: stays known-temp only if BOTH are.
                let is_known_temp = |t: &Option<SnapTempState>| {
                    matches!(t, Some(SnapTempState::Known { value: true }))
                };
                let existing_temp = effect_map[pos].1.temp_state.clone();
                let merged_temp =
                    if is_known_temp(&existing_temp) && is_known_temp(&fact_temp_state) {
                        existing_temp
                    } else if is_known_temp(&existing_temp) {
                        fact_temp_state.clone()
                    } else {
                        existing_temp
                    };
                let acc = &mut effect_map[pos].1;
                acc.via_paths = via;
                acc.had_truncation = had_truncation;
                acc.all_paths = unique;
                acc.temp_state = merged_temp;
            } else {
                let via: Vec<Vec<QueryWitnessHop>> =
                    projected_paths.iter().take(MAX_PATHS).cloned().collect();
                let had_truncation = outcome.truncated || projected_paths.len() > MAX_PATHS;
                effect_map.push((
                    key,
                    AccumulatedEffect {
                        effect_type,
                        detail,
                        provenance: if fact.provenance == "direct" {
                            "direct"
                        } else {
                            "transitive"
                        },
                        evidence,
                        evidence_operation_id,
                        evidence_callsite_id,
                        via_paths: via,
                        had_truncation,
                        all_paths: projected_paths,
                        temp_state: fact_temp_state,
                        fact_subject: fact.subject.clone(),
                    },
                ));
            }
        }

        // Materialize effects from effectMap.values() insertion order.
        let mut effects: Vec<DigestEffectResult> = Vec::new();
        for (_key, acc) in effect_map {
            // Occurrence-build (canonical key + factId).
            let terminal_evidence_kind = if acc.evidence_operation_id.is_some() {
                "operation"
            } else {
                "callsite"
            };
            let terminal_evidence_id = acc
                .evidence_operation_id
                .clone()
                .or_else(|| acc.evidence_callsite_id.clone())
                .unwrap_or_default();

            let (canonical_key, link_signature) = build_canonical_key(
                rid,
                &acc.via_paths,
                terminal_evidence_kind,
                &terminal_evidence_id,
                acc.effect_type,
            );

            let sort_file = acc.evidence.file.clone().unwrap_or_default();
            let sort_line = acc.evidence.line.unwrap_or(0);

            effects.push(DigestEffectResult {
                effect_type: acc.effect_type.to_string(),
                detail: acc.detail,
                provenance: acc.provenance,
                evidence: ProjectedEvidence {
                    source_kind: acc.evidence.source_kind,
                    file: acc.evidence.file,
                    line: acc.evidence.line,
                    column: acc.evidence.column,
                    excerpt: acc.evidence.excerpt,
                },
                evidence_operation_id: acc.evidence_operation_id,
                evidence_callsite_id: acc.evidence_callsite_id,
                via_paths: acc
                    .via_paths
                    .into_iter()
                    .map(|p| p.into_iter().map(|h| ProjectedHop { inner: h }).collect())
                    .collect(),
                via_paths_truncated: acc.had_truncation,
                fact_id: String::new(), // filled below (after sort, via seenCanonicalKeys)
                fact_subject: acc.fact_subject,
                canonical_key,
                link_signature,
                sort_file,
                sort_line,
                temp_state: acc.temp_state,
                scoped_guarantees: Vec::new(),
            });
        }

        // Sort by (type, evidence.file ?? "", evidence.line ?? 0).
        effects.sort_by(|a, b| {
            if a.effect_type != b.effect_type {
                return a.effect_type.cmp(&b.effect_type);
            }
            if a.sort_file != b.sort_file {
                return a.sort_file.cmp(&b.sort_file);
            }
            a.sort_line.cmp(&b.sort_line)
        });

        // Occurrence-build: seenCanonicalKeys (canonicalKey → first occurrenceId).
        let mut seen_canonical_keys: Vec<(String, String)> = Vec::new();
        for eff in &mut effects {
            let existing = seen_canonical_keys
                .iter()
                .find(|(k, _)| k == &eff.canonical_key);
            let occ_id = if let Some((_, id)) = existing {
                id.clone()
            } else {
                let id = occurrence_id_from_key(&eff.canonical_key, 0);
                seen_canonical_keys.push((eff.canonical_key.clone(), id.clone()));
                id
            };
            eff.fact_id = occ_id;
        }

        // S4: compute_ordering — ALWAYS runs the ordering engine (mirrors TS digestQuery which
        // always calls computeOrdering regardless of whether routineReturnSummaries is provided).
        // return_summaries is forwarded as-is to compute_ordering; when None, the engine
        // degrades gracefully (checkCalleeReturnability → "ok"; errorEscapesChain → false).
        {
            // Pre-compute conditionality for each effect (needed by COMMIT_ON_SUCCESS_PATH).
            // Mirrors `computeConditionalityForEffect` from digest-query.ts.
            let compute_eff_conditionality = |e: &DigestEffectResult| -> &'static str {
                use crate::engine::l5::conditionality::{
                    context_to_conditionality, effect_conditionality, path_conditionality, UNKNOWN,
                };
                if e.via_paths.is_empty() {
                    return UNKNOWN;
                }
                let cs_idx: HashMap<&str, Option<&str>> = snap
                    .callsite_index
                    .iter()
                    .map(|cs| (cs.callsite_id.as_str(), cs.control_context.as_deref()))
                    .collect();
                let op_idx: HashMap<&str, Option<&str>> = snap
                    .operation_index
                    .iter()
                    .map(|op| (op.operation_id.as_str(), op.control_context.as_deref()))
                    .collect();
                let terminal_ctx = if let Some(op_id) = &e.evidence_operation_id {
                    context_to_conditionality(op_idx.get(op_id.as_str()).copied().flatten())
                } else if let Some(cs_id) = &e.evidence_callsite_id {
                    context_to_conditionality(cs_idx.get(cs_id.as_str()).copied().flatten())
                } else {
                    UNKNOWN
                };
                let mut all_path_conds = Vec::new();
                for path_hops in &e.via_paths {
                    let mut hop_ctxs: Vec<&'static str> = Vec::new();
                    for hop in path_hops {
                        let csid = hop.inner.callsite_id.as_deref();
                        if let Some(csid) = csid {
                            let ctx = cs_idx.get(csid).copied().flatten();
                            hop_ctxs.push(context_to_conditionality(ctx));
                        } else {
                            hop_ctxs.push(UNKNOWN);
                        }
                    }
                    all_path_conds.push(path_conditionality(&hop_ctxs, terminal_ctx));
                }
                effect_conditionality(&all_path_conds, e.via_paths_truncated)
            };
            let ordering_inputs: Vec<crate::engine::l5::ordering_engine::OrderingEffectInput> =
                effects
                    .iter()
                    .map(
                        |e| crate::engine::l5::ordering_engine::OrderingEffectInput {
                            effect_type: e.effect_type.clone(),
                            evidence_operation_id: e.evidence_operation_id.clone(),
                            evidence_callsite_id: e.evidence_callsite_id.clone(),
                            via_paths: e
                                .via_paths
                                .iter()
                                .map(|p| p.iter().map(|h| h.inner.clone()).collect())
                                .collect(),
                            via_paths_truncated: e.via_paths_truncated,
                            temp_state: e.temp_state.clone(),
                            occurrence_id: e.fact_id.clone(),
                            conditionality: compute_eff_conditionality(e),
                        },
                    )
                    .collect();
            let scoped = crate::engine::l5::ordering_engine::compute_ordering(
                rid,
                &ordering_inputs,
                snap,
                &callsite_by_id_str,
                return_summaries,
                isolated_event_ids,
            );
            for (i, eff) in effects.iter_mut().enumerate() {
                if let Some(sg) = scoped.get(i) {
                    eff.scoped_guarantees = sg.clone();
                }
            }
        }

        entries.push(DigestEntryResult {
            routine_id: rid.clone(),
            effects,
        });
    }

    // Sort entries by routineId.
    entries.sort_by(|a, b| a.routine_id.cmp(&b.routine_id));
    entries
}

// ===========================================================================
// Reportable roots (ordering-facts.ts isReportableRoutine + roots build).
// ===========================================================================

/// Roots = stable ids of reportable workspace routines, deduped + sorted.
/// `isReportableRoutine` = primary && body_available && !parse_incomplete. In the
/// source-only corpus every workspace routine is "primary" (no dependency role).
fn reportable_roots(resolved: &L3Resolved) -> Vec<String> {
    let mut roots: Vec<String> = Vec::new();
    for r in &resolved.workspace.routines {
        // isReportableRoutine = body_available && !parse_incomplete (primary is implicit
        // in the source-only corpus). De Morgan of `!(body_available && !parse_incomplete)`.
        if !r.body_available || r.parse_incomplete {
            continue;
        }
        if r.stable_routine_id.is_empty() {
            continue;
        }
        roots.push(r.stable_routine_id.clone());
    }
    // dedupe + sort.
    roots.sort();
    roots.dedup();
    roots
}

// ===========================================================================
// R4-F STABLE PROJECTION — project_r4f_digest_effects.
// Top-level + per-effect key order MIRRORS the al-sem golden EXACTLY.
// ===========================================================================

/// Ordered evidence (SourceAnchorContract) serialize — sourceKind, [file], [line],
/// [column], [excerpt]; "unavailable" emits ONLY sourceKind.
struct EvidenceSer<'a>(&'a ProjectedEvidence);
impl Serialize for EvidenceSer<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let e = self.0;
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("sourceKind", e.source_kind)?;
        if let Some(f) = &e.file {
            map.serialize_entry("file", f)?;
        }
        if let Some(l) = e.line {
            map.serialize_entry("line", &l)?;
        }
        if let Some(c) = e.column {
            map.serialize_entry("column", &c)?;
        }
        if let Some(x) = &e.excerpt {
            map.serialize_entry("excerpt", x)?;
        }
        map.end()
    }
}

/// Ordered detail (Record<string,string>) serialize — insertion order.
struct DetailSer<'a>(&'a [(String, String)]);
impl Serialize for DetailSer<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for (k, v) in self.0 {
            map.serialize_entry(k, v)?;
        }
        map.end()
    }
}

/// Ordered QueryWitnessHop serialize — per-variant V8 field order (= projectHop literal).
struct HopSer<'a>(&'a QueryWitnessHop);
impl Serialize for HopSer<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let h = self.0;
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("kind", h.kind)?;
        map.serialize_entry("fromRoutineId", &h.from_routine_id)?;
        map.serialize_entry("fromDisplay", &h.from_display)?;
        if let Some(v) = &h.to_routine_id {
            map.serialize_entry("toRoutineId", v)?;
        }
        if let Some(v) = &h.to_display {
            map.serialize_entry("toDisplay", v)?;
        }
        match h.kind {
            "event-dispatch" => {
                if let Some(v) = &h.event_id {
                    map.serialize_entry("eventId", v)?;
                }
                if let Some(v) = &h.edge_kind {
                    map.serialize_entry("edgeKind", v)?;
                }
            }
            "implicit-trigger" => {
                if let Some(v) = &h.edge_kind {
                    map.serialize_entry("edgeKind", v)?;
                }
                anchor_entry(&mut map, &h.anchor)?;
            }
            "dependency-export" => {
                if let Some(v) = &h.callee_display {
                    map.serialize_entry("calleeDisplay", v)?;
                }
                if let Some(v) = &h.callsite_id {
                    map.serialize_entry("callsiteId", v)?;
                }
                if let Some(v) = &h.target_app_guid {
                    map.serialize_entry("targetAppGuid", v)?;
                }
                if let Some(v) = &h.edge_kind {
                    map.serialize_entry("edgeKind", v)?;
                }
                anchor_entry(&mut map, &h.anchor)?;
            }
            "variable-typed-call" => {
                if let Some(v) = &h.callee_display {
                    map.serialize_entry("calleeDisplay", v)?;
                }
                if let Some(v) = &h.callsite_id {
                    map.serialize_entry("callsiteId", v)?;
                }
                if let Some(v) = &h.edge_kind {
                    map.serialize_entry("edgeKind", v)?;
                }
                if let Some(v) = &h.receiver_type {
                    map.serialize_entry("receiverType", v)?;
                }
                anchor_entry(&mut map, &h.anchor)?;
            }
            "interface-dispatch" => {
                if let Some(v) = &h.callee_display {
                    map.serialize_entry("calleeDisplay", v)?;
                }
                if let Some(v) = &h.callsite_id {
                    map.serialize_entry("callsiteId", v)?;
                }
                if let Some(v) = &h.edge_kind {
                    map.serialize_entry("edgeKind", v)?;
                }
                if let Some(v) = &h.interface_name {
                    map.serialize_entry("interfaceName", v)?;
                }
                if let Some(v) = h.candidate_count {
                    map.serialize_entry("candidateCount", &v)?;
                }
                anchor_entry(&mut map, &h.anchor)?;
            }
            // call / object-run
            _ => {
                if let Some(v) = &h.callee_display {
                    map.serialize_entry("calleeDisplay", v)?;
                }
                if let Some(v) = &h.callsite_id {
                    map.serialize_entry("callsiteId", v)?;
                }
                if let Some(v) = &h.edge_kind {
                    map.serialize_entry("edgeKind", v)?;
                }
                anchor_entry(&mut map, &h.anchor)?;
            }
        }
        map.end()
    }
}

fn anchor_entry<M: serde::ser::SerializeMap>(
    map: &mut M,
    anchor: &Option<HopAnchor>,
) -> Result<(), M::Error> {
    if let Some(a) = anchor {
        map.serialize_entry("anchor", &AnchorSer(a))?;
    }
    Ok(())
}

struct AnchorSer<'a>(&'a HopAnchor);
impl Serialize for AnchorSer<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let a = self.0;
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("sourceKind", "source")?;
        map.serialize_entry("file", &a.file)?;
        if let Some(l) = a.line {
            map.serialize_entry("line", &l)?;
        }
        if let Some(c) = a.column {
            map.serialize_entry("column", &c)?;
        }
        map.end()
    }
}

/// Public helper for digest_cli: convert a `QueryWitnessHop` to a `serde_json::Value`
/// using the same field ordering as `HopSer`. Used by `project_digest_document`.
pub fn hop_to_json_value(hop: &QueryWitnessHop) -> serde_json::Value {
    serde_json::to_value(HopSer(hop)).unwrap_or(serde_json::Value::Null)
}

/// Ordered effect serialize — FIXED key order: type, detail, provenance, evidence,
/// [evidenceOperationId], [evidenceCallsiteId], viaPaths, viaPathsTruncated, factId,
/// canonicalKey, linkSignature.
struct EffectSer<'a>(&'a DigestEffectResult);
impl Serialize for EffectSer<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let e = self.0;
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("type", &e.effect_type)?;
        map.serialize_entry("detail", &DetailSer(&e.detail))?;
        map.serialize_entry("provenance", e.provenance)?;
        map.serialize_entry("evidence", &EvidenceSer(&e.evidence))?;
        if let Some(op) = &e.evidence_operation_id {
            map.serialize_entry("evidenceOperationId", op)?;
        }
        if let Some(cs) = &e.evidence_callsite_id {
            map.serialize_entry("evidenceCallsiteId", cs)?;
        }
        // viaPaths: Vec<Vec<ProjectedHop>>.
        let via: Vec<Vec<HopSer>> = e
            .via_paths
            .iter()
            .map(|p| p.iter().map(|h| HopSer(&h.inner)).collect())
            .collect();
        map.serialize_entry("viaPaths", &via)?;
        map.serialize_entry("viaPathsTruncated", &e.via_paths_truncated)?;
        map.serialize_entry("factId", &e.fact_id)?;
        map.serialize_entry("canonicalKey", &e.canonical_key)?;
        map.serialize_entry("linkSignature", &e.link_signature)?;
        map.end()
    }
}

struct EntrySer<'a>(&'a DigestEntryResult);
impl Serialize for EntrySer<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let e = self.0;
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("routineId", &e.routine_id)?;
        let effs: Vec<EffectSer> = e.effects.iter().map(EffectSer).collect();
        map.serialize_entry("effects", &effs)?;
        map.end()
    }
}

struct ProjectionSer<'a> {
    fixture_name: &'a str,
    entries: &'a [DigestEntryResult],
}
impl Serialize for ProjectionSer<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("fixtureName", self.fixture_name)?;
        map.serialize_entry("entryCount", &self.entries.len())?;
        let entries: Vec<EntrySer> = self.entries.iter().map(EntrySer).collect();
        map.serialize_entry("entries", &entries)?;
        map.end()
    }
}

/// Compute the per-root digest effects for a resolved source-only workspace
/// (S3 path — NO ordering engine; scopedGuarantees stay empty).
pub fn compute_digest_effects(resolved: &L3Resolved) -> Vec<DigestEntryResult> {
    let snap = compose_snapshot(resolved);
    let roots = reportable_roots(resolved);
    digest_query(&snap, &roots, None, None)
}

/// Compute the per-root digest effects WITH S4 ordering (scopedGuarantees attached).
/// Mirrors `computeOrderingFacts`: composeSnapshot + computeReturnSummaries +
/// isolatedEventIds + digestQuery(order:false).
///
/// Used by R4-F tests and any caller that needs summaries. For the CLI-B digest pipeline
/// use `compute_digest_effects_cli` instead (matches TS `runDigestPipeline` which does
/// NOT pass routineReturnSummaries to digestQuery).
pub fn compute_digest_effects_with_ordering(resolved: &L3Resolved) -> Vec<DigestEntryResult> {
    let snap = compose_snapshot(resolved);
    let roots = reportable_roots(resolved);
    let summaries = crate::engine::return_summary::compute_return_summaries(
        &resolved.workspace.routines,
        Some(&resolved.workspace.objects),
    );
    let isolated = crate::engine::l3::event_graph::isolated_event_ids(&resolved.workspace.routines);
    let isolated_opt = if isolated.is_empty() {
        None
    } else {
        Some(&isolated)
    };
    digest_query(&snap, &roots, Some(&summaries), isolated_opt)
}

/// Compute the per-root digest effects for the CLI-B digest pipeline.
///
/// Mirrors TS `runDigestPipeline → digestQuery({order:false})` exactly: S4 ordering
/// is computed but `routineReturnSummaries` is NOT passed (the TS CLI path doesn't
/// pass them, so `errorEscapesChain` always returns false and `IO_BEFORE_ESCAPING_ERROR`
/// / any error-escape-based labels never fire from this path).
pub fn compute_digest_effects_cli(
    snap: &CapabilitySnapshot,
    resolved: &L3Resolved,
) -> Vec<DigestEntryResult> {
    let roots = reportable_roots(resolved);
    let isolated = crate::engine::l3::event_graph::isolated_event_ids(&resolved.workspace.routines);
    let isolated_opt = if isolated.is_empty() {
        None
    } else {
        Some(&isolated)
    };
    // No routineReturnSummaries — matches TS runDigestPipeline behavior.
    digest_query(snap, &roots, None, isolated_opt)
}

/// Project the R4-F digest-effects differential document, PRETTY-serialized with a
/// trailing newline (the exact on-disk golden form).
pub fn project_r4f_digest_effects(resolved: &L3Resolved, fixture_name: &str) -> String {
    let entries = compute_digest_effects(resolved);
    let doc = ProjectionSer {
        fixture_name,
        entries: &entries,
    };
    let mut s =
        serde_json::to_string_pretty(&doc).expect("serialize R4-F digest-effects projection");
    s.push('\n');
    s
}

// ===========================================================================
// R4-F STABLE PROJECTION — project_r4f_scoped_guarantees (Stage-4).
// Per-ScopedGuarantee key order (FIXED, the al-sem golden shape): label, scope,
// [writeOccurrenceId], [commitOccurrenceId], [ioOccurrenceId], [returnOccurrenceId],
// supportingEdgeIds, [commitEffectiveness], interveningBoundary, validForRefutation.
// Effects with no relevant scopedGuarantees are DROPPED; entries with no remaining
// effects are DROPPED (negatives → entryCount 0).
// ===========================================================================

fn is_relevant_label(label: &str) -> bool {
    matches!(
        label,
        "WRITE_PENDING_AT_EXTERNAL_IO"
            | "EXTERNAL_IO_BEFORE_COMMIT"
            | "WRITE_PENDING_AT_UI"
            | "IO_BEFORE_ESCAPING_ERROR"
            | "EXTERNAL_IO_IN_EVENT_SUBSCRIBER_TXN"
    )
}

struct ScopedGuaranteeSer<'a>(&'a crate::engine::l5::ordering_engine::ScopedGuarantee);
impl Serialize for ScopedGuaranteeSer<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let g = self.0;
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("label", g.label)?;
        map.serialize_entry("scope", g.scope)?;
        if let Some(v) = &g.write_occurrence_id {
            map.serialize_entry("writeOccurrenceId", v)?;
        }
        if let Some(v) = &g.commit_occurrence_id {
            map.serialize_entry("commitOccurrenceId", v)?;
        }
        if let Some(v) = &g.io_occurrence_id {
            map.serialize_entry("ioOccurrenceId", v)?;
        }
        if let Some(v) = &g.return_occurrence_id {
            map.serialize_entry("returnOccurrenceId", v)?;
        }
        map.serialize_entry("supportingEdgeIds", &g.supporting_edge_ids)?;
        if let Some(v) = g.commit_effectiveness {
            map.serialize_entry("commitEffectiveness", v)?;
        }
        map.serialize_entry("interveningBoundary", g.intervening_boundary)?;
        map.serialize_entry("validForRefutation", &g.valid_for_refutation)?;
        map.end()
    }
}

struct ScopedEffectSer<'a> {
    fact_id: &'a str,
    effect_type: &'a str,
    guarantees: Vec<&'a crate::engine::l5::ordering_engine::ScopedGuarantee>,
}
impl Serialize for ScopedEffectSer<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("factId", self.fact_id)?;
        map.serialize_entry("type", self.effect_type)?;
        let sgs: Vec<ScopedGuaranteeSer> = self
            .guarantees
            .iter()
            .map(|g| ScopedGuaranteeSer(g))
            .collect();
        map.serialize_entry("scopedGuarantees", &sgs)?;
        map.end()
    }
}

struct ScopedEntrySer<'a> {
    routine_id: &'a str,
    effects: Vec<ScopedEffectSer<'a>>,
}
impl Serialize for ScopedEntrySer<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("routineId", self.routine_id)?;
        map.serialize_entry("effects", &self.effects)?;
        map.end()
    }
}

struct ScopedProjectionSer<'a> {
    fixture_name: &'a str,
    entries: Vec<ScopedEntrySer<'a>>,
}
impl Serialize for ScopedProjectionSer<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("fixtureName", self.fixture_name)?;
        map.serialize_entry("entryCount", &self.entries.len())?;
        map.serialize_entry("entries", &self.entries)?;
        map.end()
    }
}

/// Project the R4-F scoped-guarantees differential document, PRETTY-serialized with
/// a trailing newline (the exact on-disk golden form).
pub fn project_r4f_scoped_guarantees(resolved: &L3Resolved, fixture_name: &str) -> String {
    let entries = compute_digest_effects_with_ordering(resolved);

    // Drop effects with no relevant scopedGuarantees; drop entries with no effects.
    let mut out_entries: Vec<ScopedEntrySer> = Vec::new();
    for entry in &entries {
        let mut out_effects: Vec<ScopedEffectSer> = Vec::new();
        for eff in &entry.effects {
            let relevant: Vec<&crate::engine::l5::ordering_engine::ScopedGuarantee> = eff
                .scoped_guarantees
                .iter()
                .filter(|g| is_relevant_label(g.label))
                .collect();
            if relevant.is_empty() {
                continue;
            }
            out_effects.push(ScopedEffectSer {
                fact_id: &eff.fact_id,
                effect_type: &eff.effect_type,
                guarantees: relevant,
            });
        }
        if out_effects.is_empty() {
            continue;
        }
        out_entries.push(ScopedEntrySer {
            routine_id: &entry.routine_id,
            effects: out_effects,
        });
    }

    let doc = ScopedProjectionSer {
        fixture_name,
        entries: out_entries,
    };
    let mut s =
        serde_json::to_string_pretty(&doc).expect("serialize R4-F scoped-guarantees projection");
    s.push('\n');
    s
}

// ===========================================================================
// Native unit test — occurrence_id round-trips a hand-built canonical key.
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::l5::snapshot::{SnapshotIdentityTable, SnapshotRange, SnapshotSourceAnchor};

    #[test]
    fn occurrence_id_round_trips_hand_built_key() {
        // Hand-built canonical key (direct-fact form: empty linkSignature).
        let routine_id = "g:Codeunit:50000#abc";
        let (key, link) =
            build_canonical_key(routine_id, &[], "operation", "r0/h/op1", "DB_MODIFY");
        assert_eq!(link, "");
        assert_eq!(key, "g:Codeunit:50000#abc||operation|r0/h/op1|DB_MODIFY");
        let occ = occurrence_id_from_key(&key, 0);
        // Round-trip: occ == sha256Hex(key)[..16].
        assert_eq!(occ, sha256_hex(&key)[..16].to_string());
        assert_eq!(occ.len(), 16);
        assert!(occ.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn link_signature_event_dispatch_hop_has_no_callsite() {
        // An event-dispatch hop with no callsiteId → segment `@/event-dispatch//`.
        let hop = QueryWitnessHop {
            kind: "event-dispatch",
            from_routine_id: "A".to_string(),
            from_display: "a".to_string(),
            to_routine_id: Some("B".to_string()),
            to_display: Some("b".to_string()),
            callee_display: None,
            callsite_id: None,
            event_id: Some("evt".to_string()),
            target_app_guid: None,
            edge_kind: Some("event-dispatch".to_string()),
            anchor: None,
            receiver_type: None,
            interface_name: None,
            candidate_count: None,
        };
        let (_key, link) = build_canonical_key("R", &[vec![hop]], "callsite", "cs", "HTTP");
        assert_eq!(link, "A>B@/event-dispatch//");
    }

    // -----------------------------------------------------------------------
    // Helper: build a minimal Fact (SnapshotCapabilityFact) for unit tests.
    // -----------------------------------------------------------------------
    fn make_fact(
        op: &str,
        resource_kind: &str,
        resource_id: Option<&str>,
        provenance: &str,
        witness_operation_id: Option<&str>,
        witness_callsite_id: Option<&str>,
    ) -> Fact {
        Fact {
            subject: "root#sig".to_string(),
            op: op.to_string(),
            resource_kind: resource_kind.to_string(),
            resource_id: resource_id.map(|s| s.to_string()),
            resource_arg_source: None,
            confidence: "high".to_string(),
            provenance: provenance.to_string(),
            via: "direct".to_string(),
            witness_operation_id: witness_operation_id.map(|s| s.to_string()),
            witness_callsite_id: witness_callsite_id.map(|s| s.to_string()),
            extra: None,
        }
    }

    // -----------------------------------------------------------------------
    // Helper: build an empty CapabilitySnapshot (all vecs empty, no frames).
    // -----------------------------------------------------------------------
    fn empty_snapshot() -> CapabilitySnapshot {
        CapabilitySnapshot {
            identities: SnapshotIdentityTable {
                stable_ids: vec![],
                display_names: vec![],
            },
            capability_facts: vec![],
            typed_edges: vec![],
            operation_index: vec![],
            callsite_index: vec![],
            callsite_resolutions: vec![],
            analysis_gaps: vec![],
            coverage: vec![],
            event_declarations: vec![],
            root_classifications: vec![],
            routine_order_frames: None,
        }
    }

    // -----------------------------------------------------------------------
    // Oracle 1: multi-path TIE — two equal-length witness paths sort by
    // lexicographically-smaller `witness_hops_json` first (witness.ts:352-355).
    //
    // Level: serializer + sort-comparator (no BFS driver needed).
    // -----------------------------------------------------------------------
    #[test]
    fn multi_path_tie_sort_by_lex_smaller_json_first() {
        // Build two WitnessHop::Call paths with the SAME hop count (1 + 1 terminal = 2
        // hops each) but different routineIds so their JSON differs.
        // Path A: call to "routineA" + terminal op.
        let path_a = WitnessPath {
            hops: vec![
                WitnessHop::Call {
                    routine_id: "g:Codeunit:1#aaa".to_string(),
                    routine_display: "RouterA".to_string(),
                    callee_display: "DoA".to_string(),
                    callsite_id: "cs1".to_string(),
                    source_file: None,
                    line: None,
                    column: None,
                },
                WitnessHop::Terminal {
                    evidence_kind: TerminalKind::Synthetic,
                    operation_id: None,
                    callsite_id: None,
                    display_text: "insert table".to_string(),
                    source_file: None,
                    line: None,
                    column: None,
                },
            ],
        };
        // Path B: call to "routineZ" + same terminal. "g:Codeunit:1#zzz" > "g:Codeunit:1#aaa"
        // so path_a JSON < path_b JSON.
        let path_b = WitnessPath {
            hops: vec![
                WitnessHop::Call {
                    routine_id: "g:Codeunit:1#zzz".to_string(),
                    routine_display: "RouterZ".to_string(),
                    callee_display: "DoZ".to_string(),
                    callsite_id: "cs1".to_string(),
                    source_file: None,
                    line: None,
                    column: None,
                },
                WitnessHop::Terminal {
                    evidence_kind: TerminalKind::Synthetic,
                    operation_id: None,
                    callsite_id: None,
                    display_text: "insert table".to_string(),
                    source_file: None,
                    line: None,
                    column: None,
                },
            ],
        };

        // Confirm JSON serialization order matches expectation.
        let json_a = witness_hops_json(&path_a.hops);
        let json_b = witness_hops_json(&path_b.hops);
        assert!(
            json_a < json_b,
            "path_a JSON should be lex-smaller than path_b JSON: a={json_a:?} b={json_b:?}"
        );

        // Simulate the final sort (same as witness.ts:352-355 and the Rust BFS exit).
        // Intentionally reversed to verify the sort corrects the order.
        let mut paths: Vec<WitnessPath> = [path_b, path_a].into();
        paths.sort_by(|a, b| {
            if a.hops.len() != b.hops.len() {
                return a.hops.len().cmp(&b.hops.len());
            }
            witness_hops_json(&a.hops).cmp(&witness_hops_json(&b.hops))
        });

        // The lex-smaller path (path_a, routineA) must be first.
        assert!(
            matches!(&paths[0].hops[0], WitnessHop::Call { routine_id, .. } if routine_id == "g:Codeunit:1#aaa"),
            "expected path_a (routineA) to sort first; got {:?}",
            paths[0].hops[0]
        );
    }

    // -----------------------------------------------------------------------
    // Oracle 2: factEquivalent None-guard (witness.ts:380, asymmetric).
    //
    // Level: unit — fact_equivalent directly.
    // -----------------------------------------------------------------------
    #[test]
    fn fact_equivalent_none_guard_asymmetric() {
        // a.resource_id = Some("x"), b.resource_id = None → treated as equivalent
        // (the "either undefined → match" leniency).
        let a = make_fact(
            "insert",
            "table",
            Some("g/table/50000"),
            "direct",
            None,
            None,
        );
        let b = make_fact("insert", "table", None, "direct", None, None);
        assert!(
            fact_equivalent(&a, &b),
            "one-sided None resourceId should be treated as equivalent"
        );
        assert!(
            fact_equivalent(&b, &a),
            "symmetry: None ↔ Some should also be equivalent"
        );

        // Both Some and DIFFER → false.
        let c = make_fact(
            "insert",
            "table",
            Some("g/table/99999"),
            "direct",
            None,
            None,
        );
        assert!(
            !fact_equivalent(&a, &c),
            "both Some with different resourceIds should NOT be equivalent"
        );

        // Both Some and SAME → true.
        let d = make_fact(
            "insert",
            "table",
            Some("g/table/50000"),
            "direct",
            None,
            None,
        );
        assert!(
            fact_equivalent(&a, &d),
            "both Some with equal resourceIds should be equivalent"
        );
    }

    // -----------------------------------------------------------------------
    // Oracle 3: object-run-unresolved edge → edge_to_hop returns None
    // (witness.ts:461-464 "BFS cannot walk through").
    //
    // Level: unit — edge_to_hop directly.
    // -----------------------------------------------------------------------
    #[test]
    fn object_run_unresolved_edge_to_hop_is_none() {
        let edge = SnapshotGraphEdge::ObjectRunUnresolved {
            kind: "object-run-unresolved",
            callsite_id: "cs_unresolved".to_string(),
            from: "root#sig".to_string(),
            target_object: None,
            target_id_source: SnapValueSource::Unknown,
            object_type: "Codeunit".to_string(),
            source_anchor: SnapshotSourceAnchor {
                source_unit_id: "su1".to_string(),
                range: SnapshotRange {
                    start_line: 1,
                    start_column: 0,
                    end_line: 1,
                    end_column: 10,
                },
                enclosing_routine_id: "root#sig".to_string(),
                syntax_kind: "method_call".to_string(),
            },
            edge_id: "eid-unresolved".to_string(),
        };
        let snap = empty_snapshot();
        let idx = build_fingerprint_indexes(&snap);
        assert!(
            edge_to_hop(&edge, &idx).is_none(),
            "object-run-unresolved should produce None from edge_to_hop"
        );
    }

    // -----------------------------------------------------------------------
    // Oracle 4: occurrence dedup — two effects with the SAME canonicalKey get the
    // SAME factId (ordering-engine.ts seenCanonicalKeys); two with different keys
    // get different factIds.
    //
    // Level: unit — build_canonical_key + occurrence_id_from_key.
    // -----------------------------------------------------------------------
    #[test]
    fn occurrence_dedup_same_key_same_fact_id() {
        let routine = "g:Codeunit:50000#abc";

        // Two effects that produce the same canonical key (same routine, no via-paths,
        // same evidence kind/id, same effect type).
        let (key1, _) = build_canonical_key(routine, &[], "operation", "r0/op1", "DB_INSERT");
        let (key2, _) = build_canonical_key(routine, &[], "operation", "r0/op1", "DB_INSERT");
        assert_eq!(key1, key2);
        let id1 = occurrence_id_from_key(&key1, 0);
        let id2 = occurrence_id_from_key(&key2, 0);
        assert_eq!(id1, id2, "same canonical key must produce same factId");

        // Two effects with different effect types → different canonical keys → different factIds.
        let (key3, _) = build_canonical_key(routine, &[], "operation", "r0/op1", "DB_MODIFY");
        assert_ne!(key1, key3);
        let id3 = occurrence_id_from_key(&key3, 0);
        assert_ne!(
            id1, id3,
            "different canonical keys must produce different factIds"
        );
    }

    // -----------------------------------------------------------------------
    // Oracle 5: seed-tie order-preservation — stable sort_by(.cmp) is a no-op on
    // equal keys, meaning insertion order is preserved for equal-routine seeds.
    // This grounds finding B (witness.ts:276 V8-stable-preserves-equal-key-order).
    //
    // Level: unit — sort_by on a Vec of equal-key items.
    // -----------------------------------------------------------------------
    #[test]
    fn seed_sort_equal_routine_preserves_insertion_order() {
        // Simulate the seed-sort scenario: multiple "State"-like items with the SAME
        // `routine` value. Stable sort must leave them in their original order.
        #[derive(Debug, PartialEq, Eq)]
        struct SeedItem {
            routine: String,
            insertion_index: usize,
        }

        let items: Vec<SeedItem> = vec![
            SeedItem {
                routine: "same#abc".to_string(),
                insertion_index: 0,
            },
            SeedItem {
                routine: "same#abc".to_string(),
                insertion_index: 1,
            },
            SeedItem {
                routine: "same#abc".to_string(),
                insertion_index: 2,
            },
        ];

        let mut sorted = items;
        sorted.sort_by(|a, b| a.routine.cmp(&b.routine));

        // All routines are equal → sort must not reorder (stable sort invariant).
        assert_eq!(sorted[0].insertion_index, 0);
        assert_eq!(sorted[1].insertion_index, 1);
        assert_eq!(sorted[2].insertion_index, 2);
    }

    // -----------------------------------------------------------------------
    // Oracle 6: synthetic-terminal direct fact (no witnessOperationId, no
    // witnessCallsiteId) → terminal_hop_from_fact emits Synthetic terminal
    // with display_text = "{op} {resourceKind}" and no IDs. The dedupe-key for
    // this terminal uses the "synthetic:op:resourceKind:resourceId" branch.
    //
    // Level: unit — terminal_hop_from_fact + dedupe_key (synthetic branch).
    // -----------------------------------------------------------------------
    #[test]
    fn synthetic_terminal_direct_fact_no_witness_anchor() {
        let fact = make_fact(
            "commit",
            "table",
            Some("g/table/50000"),
            "direct",
            None,
            None,
        );

        let snap = empty_snapshot();
        let idx = build_fingerprint_indexes(&snap);

        let hop = terminal_hop_from_fact(&fact, &idx);
        match &hop {
            WitnessHop::Terminal {
                evidence_kind,
                operation_id,
                callsite_id,
                display_text,
                ..
            } => {
                assert_eq!(*evidence_kind, TerminalKind::Synthetic);
                assert!(
                    operation_id.is_none(),
                    "synthetic terminal must have no operationId"
                );
                assert!(
                    callsite_id.is_none(),
                    "synthetic terminal must have no callsiteId"
                );
                assert_eq!(
                    display_text, "commit table",
                    "display_text must be '{{op}} {{resourceKind}}'"
                );
            }
            other => panic!("expected Terminal hop, got {other:?}"),
        }

        // The dedupe_key for a synthetic terminal must use the
        // "synthetic:op:resourceKind:resourceId" branch (no op/cs anchor).
        let detail: Vec<(String, String)> = vec![];
        let key = dedupe_key("COMMIT", Some(&hop), &fact, &detail);
        assert!(
            key.starts_with("COMMIT|synthetic:commit:table:g/table/50000|"),
            "dedupe_key synthetic branch must embed op/resourceKind/resourceId: {key:?}"
        );
    }
}
