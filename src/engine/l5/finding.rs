//! The L5 `Finding` model + the STABLE projection (`project_r4_findings`).
//!
//! Ports al-sem `src/model/finding.ts` (Finding / EvidenceStep / FixOption /
//! FindingConfidence; Evidence is `{source, note?}` from `model/graph.ts`) and the
//! stable projection in `scripts/r4-finding-projection.ts`.
//!
//! ## Two id spaces
//! The INTERNAL Finding carries internal RoutineIds (`${modelInstanceId}/${hash}`),
//! internal ObjectIds (`${appGuid}/${type}/${num}`) and internal TableIds
//! (`${appGuid}/table/${num}`). The detector computes its `id`/`rootCauseKey`/
//! `fingerprint` over THOSE. `project_r4_findings` then projects every id to its
//! stable, modelInstanceId-independent form — the comparison surface.
//!
//! ## Byte-parity serde field order (highest-risk)
//! `serde_json` emits struct fields in DECLARATION order. The STABLE projection
//! types below are declared in the EXACT insertion order al-sem's
//! `projectFinding` / `projectEvidenceStep` / `projectAnchor` use — verified
//! against `scripts/r4-goldens/ws-d4-repeated-get.r4.golden.json`. Empty `Vec`s
//! ARE serialized; only the `Option` tail fields are `skip_serializing_if`.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::engine::l3::l3_workspace::L3Resolved;
use crate::engine::l5::d1_cohort::{LoopSetId, LoopSetRegistry};
use crate::engine::l5::d1_witness::WitnessSummary;
use crate::engine::l5::registry::{Detector, RunOutput, run_detectors, run_detectors_cross_app};

// ===========================================================================
// INTERNAL model (model/finding.ts). Not serialized — the detector populates it
// with internal ids, then the projection consumes it.
// ===========================================================================

/// `Evidence` (`model/graph.ts`): `{ source, note? }`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Evidence {
    pub source: String,
    pub note: Option<String>,
}

/// `FixOption` (`model/finding.ts`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixOption {
    pub description: String,
    pub safety: String,
}

/// `FindingConfidence` (`model/finding.ts`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FindingConfidence {
    pub level: String,
    pub capped_by: Option<Vec<String>>,
    pub evidence: Vec<Evidence>,
}

/// `SourceAnchor` (`model/identity.ts`) — INTERNAL form. `enclosing_routine_id` is
/// an internal RoutineId; the projection maps it to stable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceAnchor {
    pub source_unit_id: String,
    pub start_line: u32,
    pub start_column: u32,
    pub end_line: u32,
    pub end_column: u32,
    pub enclosing_routine_id: String,
    pub syntax_kind: String,
    pub normalized_text_hash: Option<String>,
    pub leading_context_hash: Option<String>,
    pub trailing_context_hash: Option<String>,
}

/// `EvidenceStep` (`model/finding.ts`) — INTERNAL form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceStep {
    pub routine_id: String,
    pub operation_id: Option<String>,
    pub callsite_id: Option<String>,
    pub loop_id: Option<String>,
    pub source_anchor: SourceAnchor,
    pub note: String,
}

/// `LoopContext` — INTERNAL form. One per loop that reaches a d1 finding's
/// terminal op (the terminal-centric schema, `.superpowers/sdd/task-5-brief.md`).
/// `contexts[0]` is the WINNER (context order: severity rank desc, verdict
/// quality desc, loop routine id asc, loop id asc); the finding's severity,
/// confidence, `evidence_path`, temp/setup notes and wording all come from that
/// same winning context.
///
/// Field NAMES are locked (the brief's schema). Following this file's
/// internal/stable split (see the module doc — the INTERNAL model is never
/// serialized), this INTERNAL form is a plain struct; its serialized surface is
/// [`StableLoopContext`] below (camelCase, `StableEvidenceStep`-mirrored
/// witness), emitted by the R4 projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoopContext {
    pub loop_id: String,
    pub loop_routine_id: String,
    pub entry_callsite_id: Option<String>,
    /// `temporary | physical | uncertain | flowfield-on-temp` (`TempVerdict::label`).
    pub verdict: String,
    pub reachable_verdicts: Vec<String>,
    /// `"single-loop" | "nested-loop"` — `"nested-loop"` iff this context's
    /// scoring depth bucket >= 2.
    pub depth_class: String,
    pub severity: String,
    pub confidence: FindingConfidence,
    pub witness: Vec<EvidenceStep>,
}

/// One entry in a d1 run's loop catalog — the shared, run-level identity table a
/// compressed cohort's `loop_set` (interned via
/// [`crate::engine::l5::d1_cohort::LoopSetRegistry`]) decompresses into. ONE
/// entry per DISTINCT loop across the whole run (the loop-group universe,
/// `search_loops`'s `groups`), NOT one per `(loop, terminal)` — the catalog is
/// indexed by `loop_ix` (`catalog[ix].loop_ix == ix`), so a decompressed
/// `GroupBitmap`'s set bits index directly into it. Lives at the run level (the
/// catalog + [`crate::engine::l5::d1_cohort::LoopSetRegistry`] attach to the d1
/// run's output, NOT per-finding — many findings' cohorts share the SAME
/// catalog), following this file's internal/stable split: this INTERNAL form is
/// a plain struct; [`StableLoopCatalogEntry`] is its serialized mirror.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoopCatalogEntry {
    pub loop_ix: u32,
    pub loop_routine_id: String,
    pub loop_id: String,
    pub anchor: SourceAnchor,
    pub entry_callsite_id: Option<String>,
}

/// One `(terminal, ContextKey)` cohort — "reached by N loops", a verdict-class
/// group replacing [`LoopContext`]'s per-LOOP repetition (the compressed report
/// schema, Task C4, `.superpowers/sdd/task-c4-brief.md`). `loop_set` is a handle
/// into the run's `LoopSetRegistry` (see [`LoopCatalogEntry`]'s doc); every loop
/// it names shares this SAME verdict/depth_bucket/uncertain — the cohort sink's
/// disjointness + per-class-grouping invariant (Task C1: [`crate::engine::l5::
/// d1_cohort::TerminalSink::insert`]) guarantees a loop lands in exactly one
/// cohort per terminal, and that cohort's `ContextKey` IS its
/// verdict/depth_bucket/unc — so ONE representative `witness` (Task C3's bounded
/// witness) suffices as evidence for the whole class. INTERNAL form; see
/// [`StableD1CohortContext`] for the serialized mirror.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct D1CohortContext {
    pub severity: String,
    pub verdict: String,
    pub depth_bucket: i64,
    pub uncertain: bool,
    pub loop_set: LoopSetId,
    pub loop_count: u64,
    pub witness: WitnessSummary,
}

/// `Finding` (`model/finding.ts`) — INTERNAL form. Only the fields the ported
/// detectors populate are present; later-wave optional fields (additionalPaths /
/// actionableAnchor / eventKind / crossExtensionSubscribers) are added as detectors
/// that emit them land.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub id: String,
    pub root_cause_key: String,
    pub detector: String,
    pub title: String,
    pub root_cause: String,
    pub severity: String,
    pub confidence: FindingConfidence,
    pub primary_location: SourceAnchor,
    pub evidence_path: Vec<EvidenceStep>,
    pub additional_paths: Option<Vec<Vec<EvidenceStep>>>,
    pub affected_objects: Vec<String>,
    pub affected_tables: Vec<String>,
    pub fix_options: Vec<FixOption>,
    pub provenance: Vec<Evidence>,
    pub actionable_anchor: Option<SourceAnchor>,
    pub fingerprint: Option<String>,
    pub event_kind: Option<String>,
    pub cross_extension_subscribers: Option<Vec<String>>,
    /// Terminal-centric per-loop contexts (d1 only; `None` for every other
    /// detector). `contexts[0]` is the winner — see [`LoopContext`].
    pub contexts: Option<Vec<LoopContext>>,
    /// Compressed, verdict-class cohorts (d1 only; `None` for every other
    /// detector, and for every `contexts`-path d1 finding until the consumer
    /// cutover, Task C6) — the output-shape replacement for `contexts`: "reached
    /// by N loops" per verdict class instead of one [`LoopContext`] per loop. See
    /// [`D1CohortContext`].
    pub cohort_contexts: Option<Vec<D1CohortContext>>,
}

// ===========================================================================
// STABLE projection types — the parity surface. Field declaration order MUST
// match the golden's key insertion order exactly.
// ===========================================================================

/// `{ source, note? }`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StableEvidence {
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// `{ description, safety }`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StableFixOption {
    pub description: String,
    pub safety: String,
}

/// `{ level, evidence, [cappedBy] }` — NOTE: `evidence` BEFORE the optional
/// `cappedBy`, matching al-sem's `projectFinding` insertion order.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StableConfidence {
    pub level: String,
    pub evidence: Vec<StableEvidence>,
    #[serde(rename = "cappedBy", skip_serializing_if = "Option::is_none")]
    pub capped_by: Option<Vec<String>>,
}

/// `{ startLine, startColumn, endLine, endColumn }`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StableRange {
    #[serde(rename = "startLine")]
    pub start_line: u32,
    #[serde(rename = "startColumn")]
    pub start_column: u32,
    #[serde(rename = "endLine")]
    pub end_line: u32,
    #[serde(rename = "endColumn")]
    pub end_column: u32,
}

/// `{ sourceUnitId, range, enclosingRoutineId, syntaxKind, [normalizedTextHash],
/// [leadingContextHash], [trailingContextHash] }`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StableSourceAnchor {
    #[serde(rename = "sourceUnitId")]
    pub source_unit_id: String,
    pub range: StableRange,
    #[serde(rename = "enclosingRoutineId")]
    pub enclosing_routine_id: String,
    #[serde(rename = "syntaxKind")]
    pub syntax_kind: String,
    #[serde(rename = "normalizedTextHash", skip_serializing_if = "Option::is_none")]
    pub normalized_text_hash: Option<String>,
    #[serde(rename = "leadingContextHash", skip_serializing_if = "Option::is_none")]
    pub leading_context_hash: Option<String>,
    #[serde(
        rename = "trailingContextHash",
        skip_serializing_if = "Option::is_none"
    )]
    pub trailing_context_hash: Option<String>,
}

/// `{ routineId, sourceAnchor, note, [operationId], [callsiteId], [loopId] }` —
/// NOTE: `note` BEFORE the optional id fields (verified against the golden:
/// `loopId` appears after `note`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StableEvidenceStep {
    #[serde(rename = "routineId")]
    pub routine_id: String,
    #[serde(rename = "sourceAnchor")]
    pub source_anchor: StableSourceAnchor,
    pub note: String,
    #[serde(rename = "operationId", skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
    #[serde(rename = "callsiteId", skip_serializing_if = "Option::is_none")]
    pub callsite_id: Option<String>,
    #[serde(rename = "loopId", skip_serializing_if = "Option::is_none")]
    pub loop_id: Option<String>,
}

/// `LoopContext` — STABLE (serialized) form. camelCase field names; the witness
/// is `StableEvidenceStep`-mirrored and the confidence is `StableConfidence`, so
/// the whole context projects to the same stable id space as the finding.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StableLoopContext {
    pub loop_id: String,
    pub loop_routine_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry_callsite_id: Option<String>,
    pub verdict: String,
    pub reachable_verdicts: Vec<String>,
    pub depth_class: String,
    pub severity: String,
    pub confidence: StableConfidence,
    pub witness: Vec<StableEvidenceStep>,
}

/// `WitnessSummary` (`d1_witness.rs`) — STABLE (serialized) form. `first_steps`/
/// `last_steps`/`terminal_step` are `StableEvidenceStep`-mirrored (same as
/// `StableLoopContext::witness`), so a compressed cohort's witness projects to
/// the same stable id space as the rest of the finding.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StableWitnessSummary {
    pub total_hops: u32,
    pub first_steps: Vec<StableEvidenceStep>,
    pub omitted_hops: u32,
    pub last_steps: Vec<StableEvidenceStep>,
    pub terminal_step: StableEvidenceStep,
}

/// `LoopCatalogEntry` — STABLE (serialized) form.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StableLoopCatalogEntry {
    pub loop_ix: u32,
    pub loop_routine_id: String,
    pub loop_id: String,
    pub anchor: StableSourceAnchor,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry_callsite_id: Option<String>,
}

/// `D1CohortContext` — STABLE (serialized) form. `loop_set` serializes as its
/// raw interned index (`LoopSetId` is `#[serde(transparent)]`) — an opaque,
/// run-scoped handle; a consumer decompresses it via the run's
/// `StableLoopSetRegistry` + `StableLoopCatalogEntry` catalog, neither of which
/// live on the finding itself (see the module's Task C4 doc).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StableD1CohortContext {
    pub severity: String,
    pub verdict: String,
    pub depth_bucket: i64,
    pub uncertain: bool,
    pub loop_set: LoopSetId,
    pub loop_count: u64,
    pub witness: StableWitnessSummary,
}

/// The fully stable-projected Finding. Field order = al-sem `projectFinding`
/// insertion order; the OPTION tail is in golden order:
/// additionalPaths, contexts, actionableAnchor, fingerprint, eventKind,
/// crossExtensionSubscribers.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StableFinding {
    pub detector: String,
    pub id: String,
    #[serde(rename = "rootCauseKey")]
    pub root_cause_key: String,
    pub title: String,
    #[serde(rename = "rootCause")]
    pub root_cause: String,
    pub severity: String,
    pub confidence: StableConfidence,
    #[serde(rename = "primaryLocation")]
    pub primary_location: StableSourceAnchor,
    #[serde(rename = "evidencePath")]
    pub evidence_path: Vec<StableEvidenceStep>,
    #[serde(rename = "affectedObjects")]
    pub affected_objects: Vec<String>,
    #[serde(rename = "affectedTables")]
    pub affected_tables: Vec<String>,
    #[serde(rename = "fixOptions")]
    pub fix_options: Vec<StableFixOption>,
    pub provenance: Vec<StableEvidence>,
    #[serde(rename = "additionalPaths", skip_serializing_if = "Option::is_none")]
    pub additional_paths: Option<Vec<Vec<StableEvidenceStep>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contexts: Option<Vec<StableLoopContext>>,
    #[serde(rename = "cohortContexts", skip_serializing_if = "Option::is_none")]
    pub cohort_contexts: Option<Vec<StableD1CohortContext>>,
    #[serde(rename = "actionableAnchor", skip_serializing_if = "Option::is_none")]
    pub actionable_anchor: Option<StableSourceAnchor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
    #[serde(rename = "eventKind", skip_serializing_if = "Option::is_none")]
    pub event_kind: Option<String>,
    #[serde(
        rename = "crossExtensionSubscribers",
        skip_serializing_if = "Option::is_none"
    )]
    pub cross_extension_subscribers: Option<Vec<String>>,
}

/// The full R4 findings projection for one fixture run — `{ fixtureName,
/// detectors, findingCount, findings }`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct R4FindingsProjection {
    #[serde(rename = "fixtureName")]
    pub fixture_name: String,
    pub detectors: Vec<String>,
    #[serde(rename = "findingCount")]
    pub finding_count: usize,
    pub findings: Vec<StableFinding>,
}

// ===========================================================================
// Stable-id projection helpers (mirror scripts/r4-finding-projection.ts).
// ===========================================================================

/// Project an internal RoutineId to StableRoutineId; pass through if unmapped.
fn map_routine_id(internal: &str, map: &HashMap<String, String>) -> String {
    crate::engine::l4::summary::stable_routine_id(internal, map)
}

/// Project a sub-id (`${routineId}/op${n}` etc.) to stable form.
fn map_sub_id(internal: &str, map: &HashMap<String, String>) -> String {
    crate::engine::l4::summary::stable_sub_id(internal, map)
}

/// Project an internal ObjectId (`appGuid/Type/Num`) to StableObjectId
/// (`appGuid:Type:Num`).
fn map_object_id(internal: &str) -> String {
    crate::engine::ids::to_stable_object_id(internal)
}

/// Project an internal TableId (`appGuid/table/Num` or "unknown") to
/// StableTableId (`appGuid:Table:Num`). Mirrors `cvt.toStableTableId`.
///
/// - `"unknown"` → `"unknown"` (sentinel pass-through).
/// - Well-formed `*/ table/*` → stable colon form.
/// - Any other shape → `panic!` — mirrors `toStableTableId` throwing on malformed
///   input (stable-identity.ts). This runs in `project_finding`, during the R4
///   stable-id projection — AFTER `run_detectors` has already returned — so it is
///   NOT covered by any per-detector isolation (neither the `Result` contract in
///   `registry::run_each` nor its debug-only `catch_unwind` backstop; both wrap only
///   the detector call itself, not the projection step). A malformed internal
///   TableId here is a genuine engine bug (every TableId a detector emits is
///   constructed by this crate, never external input), so it is left as a hard
///   panic — an uncaught failure of the whole run — matching al-sem's uncaught throw.
fn map_table_id(internal: &str) -> String {
    if internal == "unknown" {
        return "unknown".to_string();
    }
    // Internal: `${appGuid}/table/${N}` → `${appGuid}:Table:${N}`.
    let parts: Vec<&str> = internal.split('/').collect();
    if parts.len() == 3 && parts[1] == "table" {
        return format!("{}:Table:{}", parts[0], parts[2]);
    }
    panic!("map_table_id: malformed TableId: {internal:?}");
}

/// `buildIdReplacementFn` — globally replace every internal RoutineId occurrence
/// in a string with its stable form using a TRUE single left-to-right pass over
/// the ORIGINAL string. At each byte position we try the LONGEST key that starts
/// there (keys pre-sorted by length desc, stable-tiebreak by key); on a match we
/// append the replacement and advance PAST the matched key without re-scanning the
/// substituted text. This mirrors al-sem's single-regex-alternation pass so a
/// shorter key can never corrupt an already-substituted stable value.
fn make_stable_finding_id_fn(map: &HashMap<String, String>) -> impl Fn(&str) -> String + '_ {
    // Sort entries by key length descending; ties broken by key asc (total order).
    let mut entries: Vec<(&String, &String)> = map.iter().collect();
    entries.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then_with(|| a.0.cmp(b.0)));
    move |s: &str| {
        let bytes = s.as_bytes();
        let len = bytes.len();
        let mut out = String::with_capacity(len);
        let mut pos = 0usize;
        'outer: while pos < len {
            // Try keys longest-first; take the first (longest) that matches.
            for (k, v) in &entries {
                let kb = k.as_bytes();
                if bytes.len() >= pos + kb.len() && &bytes[pos..pos + kb.len()] == kb {
                    out.push_str(v.as_str());
                    pos += kb.len();
                    continue 'outer;
                }
            }
            // No key matched at this position — copy one byte (all ids are ASCII).
            out.push(bytes[pos] as char);
            pos += 1;
        }
        out
    }
}

fn project_anchor(a: &SourceAnchor, map: &HashMap<String, String>) -> StableSourceAnchor {
    StableSourceAnchor {
        source_unit_id: a.source_unit_id.clone(),
        range: StableRange {
            start_line: a.start_line,
            start_column: a.start_column,
            end_line: a.end_line,
            end_column: a.end_column,
        },
        enclosing_routine_id: map_routine_id(&a.enclosing_routine_id, map),
        syntax_kind: a.syntax_kind.clone(),
        normalized_text_hash: a.normalized_text_hash.clone(),
        leading_context_hash: a.leading_context_hash.clone(),
        trailing_context_hash: a.trailing_context_hash.clone(),
    }
}

/// Project an internal `EvidenceStep[]` to stable form (routineIds → `:`-form via the
/// supplied internal→stable map). Used by the gate's opt-in `--with-evidence` JSON path
/// to surface a finding's `evidence_path` with the SAME stable id mapping the R4 finding
/// projection applies. Not on any default/parity surface (gated behind the flag).
pub(crate) fn project_evidence_path(
    steps: &[EvidenceStep],
    map: &HashMap<String, String>,
) -> Vec<StableEvidenceStep> {
    steps
        .iter()
        .map(|s| project_evidence_step(s, map))
        .collect()
}

fn project_evidence_step(s: &EvidenceStep, map: &HashMap<String, String>) -> StableEvidenceStep {
    StableEvidenceStep {
        routine_id: map_routine_id(&s.routine_id, map),
        source_anchor: project_anchor(&s.source_anchor, map),
        note: s.note.clone(),
        operation_id: s.operation_id.as_ref().map(|id| map_sub_id(id, map)),
        callsite_id: s.callsite_id.as_ref().map(|id| map_sub_id(id, map)),
        loop_id: s.loop_id.as_ref().map(|id| map_sub_id(id, map)),
    }
}

fn project_evidence(e: &Evidence) -> StableEvidence {
    StableEvidence {
        source: e.source.clone(),
        note: e.note.clone(),
    }
}

fn project_loop_context(c: &LoopContext, map: &HashMap<String, String>) -> StableLoopContext {
    StableLoopContext {
        loop_id: map_sub_id(&c.loop_id, map),
        loop_routine_id: map_routine_id(&c.loop_routine_id, map),
        entry_callsite_id: c.entry_callsite_id.as_ref().map(|id| map_sub_id(id, map)),
        verdict: c.verdict.clone(),
        reachable_verdicts: c.reachable_verdicts.clone(),
        depth_class: c.depth_class.clone(),
        severity: c.severity.clone(),
        confidence: StableConfidence {
            level: c.confidence.level.clone(),
            evidence: c.confidence.evidence.iter().map(project_evidence).collect(),
            capped_by: c.confidence.capped_by.clone(),
        },
        witness: c
            .witness
            .iter()
            .map(|s| project_evidence_step(s, map))
            .collect(),
    }
}

fn project_witness_summary(
    w: &WitnessSummary,
    map: &HashMap<String, String>,
) -> StableWitnessSummary {
    StableWitnessSummary {
        total_hops: w.total_hops,
        first_steps: w
            .first_steps
            .iter()
            .map(|s| project_evidence_step(s, map))
            .collect(),
        omitted_hops: w.omitted_hops,
        last_steps: w
            .last_steps
            .iter()
            .map(|s| project_evidence_step(s, map))
            .collect(),
        terminal_step: project_evidence_step(&w.terminal_step, map),
    }
}

/// Project one [`LoopCatalogEntry`] to its stable form. `pub` (not routed through
/// `project_finding`, unlike the per-finding helpers below it) — the run-level
/// catalog lives alongside the findings, not inside one, so its consumer (the
/// eventual run-level report envelope, C5/C6) calls this directly.
pub fn project_loop_catalog_entry(
    e: &LoopCatalogEntry,
    map: &HashMap<String, String>,
) -> StableLoopCatalogEntry {
    StableLoopCatalogEntry {
        loop_ix: e.loop_ix,
        loop_routine_id: map_routine_id(&e.loop_routine_id, map),
        loop_id: map_sub_id(&e.loop_id, map),
        anchor: project_anchor(&e.anchor, map),
        entry_callsite_id: e.entry_callsite_id.as_ref().map(|id| map_sub_id(id, map)),
    }
}

fn project_d1_cohort_context(
    c: &D1CohortContext,
    map: &HashMap<String, String>,
) -> StableD1CohortContext {
    StableD1CohortContext {
        severity: c.severity.clone(),
        verdict: c.verdict.clone(),
        depth_bucket: c.depth_bucket,
        uncertain: c.uncertain,
        loop_set: c.loop_set,
        loop_count: c.loop_count,
        witness: project_witness_summary(&c.witness, map),
    }
}

/// Decompress `ctx`'s cohort into its per-loop `(catalog entry, verdict,
/// depth_bucket, uncertain)` tuples: every loop in `ctx.loop_set` shares this
/// SAME verdict/depth_bucket/uncertain — the cohort sink's disjointness +
/// per-class-grouping invariant guarantees it (a cohort IS one verdict class,
/// see [`D1CohortContext`]'s doc) — so this is a plain broadcast of `ctx`'s own
/// scalar fields across the registry-decompressed loop indices, each resolved
/// through the run's `catalog` (indexed by `loop_ix`, i.e. `catalog[g as usize]`
/// is loop-group `g`'s entry).
pub fn decompress_cohort_context<'a>(
    ctx: &'a D1CohortContext,
    registry: &LoopSetRegistry,
    catalog: &'a [LoopCatalogEntry],
) -> Vec<(&'a LoopCatalogEntry, &'a str, i64, bool)> {
    registry
        .iter(ctx.loop_set)
        .map(|g| {
            (
                &catalog[g as usize],
                ctx.verdict.as_str(),
                ctx.depth_bucket,
                ctx.uncertain,
            )
        })
        .collect()
}

fn project_finding(
    f: &Finding,
    map: &HashMap<String, String>,
    stable_finding_id: &impl Fn(&str) -> String,
) -> StableFinding {
    StableFinding {
        detector: f.detector.clone(),
        id: stable_finding_id(&f.id),
        root_cause_key: stable_finding_id(&f.root_cause_key),
        title: f.title.clone(),
        root_cause: f.root_cause.clone(),
        severity: f.severity.clone(),
        confidence: StableConfidence {
            level: f.confidence.level.clone(),
            evidence: f.confidence.evidence.iter().map(project_evidence).collect(),
            capped_by: f.confidence.capped_by.clone(),
        },
        primary_location: project_anchor(&f.primary_location, map),
        evidence_path: f
            .evidence_path
            .iter()
            .map(|s| project_evidence_step(s, map))
            .collect(),
        affected_objects: f
            .affected_objects
            .iter()
            .map(|o| map_object_id(o))
            .collect(),
        affected_tables: f.affected_tables.iter().map(|t| map_table_id(t)).collect(),
        fix_options: f
            .fix_options
            .iter()
            .map(|x| StableFixOption {
                description: x.description.clone(),
                safety: x.safety.clone(),
            })
            .collect(),
        provenance: f.provenance.iter().map(project_evidence).collect(),
        additional_paths: f.additional_paths.as_ref().map(|paths| {
            paths
                .iter()
                .map(|p| p.iter().map(|s| project_evidence_step(s, map)).collect())
                .collect()
        }),
        contexts: f
            .contexts
            .as_ref()
            .map(|cs| cs.iter().map(|c| project_loop_context(c, map)).collect()),
        cohort_contexts: f.cohort_contexts.as_ref().map(|cs| {
            cs.iter()
                .map(|c| project_d1_cohort_context(c, map))
                .collect()
        }),
        actionable_anchor: f.actionable_anchor.as_ref().map(|a| project_anchor(a, map)),
        fingerprint: f.fingerprint.clone(),
        event_kind: f.event_kind.clone(),
        cross_extension_subscribers: f
            .cross_extension_subscribers
            .as_ref()
            .map(|ids| ids.iter().map(|id| map_routine_id(id, map)).collect()),
    }
}

/// `stablePrimaryLocationKey` — `${sourceUnitId}:${startLine}:${startColumn}`.
fn stable_primary_location_key(f: &StableFinding) -> String {
    let a = &f.primary_location;
    format!(
        "{}:{}:{}",
        a.source_unit_id, a.range.start_line, a.range.start_column
    )
}

// ===========================================================================
// Main entry point.
// ===========================================================================

/// Run the registered detectors over a resolved (source-only) workspace, then
/// project + RE-SORT the Finding[] in stable space — the byte-parity surface.
///
/// `resolved` is the L0→L3 source-only model; `detectors` are the registered L5
/// detectors. `fixture_name` + `detector_names` populate the projection envelope.
///
/// Mirrors al-sem's `projectR4Findings`: only findings from the detectors listed in
/// `detector_names` are included in the output, matching the per-fixture golden scope.
/// (al-sem passes `detectorNames` to `analyzeWorkspace({ detectors: selectedDetectors })`
/// so only those detectors run; the Rust port runs all registered detectors and then
/// filters to the named set — byte-equivalent for ANY requested detector set, because
/// fingerprint and role-scope are per-finding, and the final stable re-sort is applied
/// post-filter; the filter is not a single-detector-only crutch.)
pub fn project_r4_findings(
    resolved: &L3Resolved,
    detectors: &[Detector],
    fixture_name: &str,
    detector_names: &[String],
) -> R4FindingsProjection {
    let RunOutput { findings, .. } = run_detectors(resolved, detectors);

    // Filter to only the named detectors (mirrors al-sem: only selected detectors run).
    let detector_name_set: std::collections::HashSet<&str> =
        detector_names.iter().map(|s| s.as_str()).collect();

    let map = crate::engine::l4::summary::build_routine_stable_map(&resolved.workspace.routines);
    let stable_finding_id = make_stable_finding_id_fn(&map);

    let mut stable: Vec<StableFinding> = findings
        .iter()
        .filter(|f| detector_name_set.contains(f.detector.as_str()))
        .map(|f| project_finding(f, &map, &stable_finding_id))
        .collect();

    // RE-SORT in stable space: (detector compareNatural, stable primaryLocationKey
    // compareStrings, stable rootCauseKey compareStrings).
    stable.sort_by(|a, b| {
        crate::engine::l5::registry::compare_natural(&a.detector, &b.detector)
            .then_with(|| stable_primary_location_key(a).cmp(&stable_primary_location_key(b)))
            .then_with(|| a.root_cause_key.cmp(&b.root_cause_key))
    });

    R4FindingsProjection {
        fixture_name: fixture_name.to_string(),
        detectors: detector_names.to_vec(),
        finding_count: stable.len(),
        findings: stable,
    }
}

/// CROSS-APP variant of `project_r4_findings`: build the cross-app L4 base from a
/// disk workspace (its `.alpackages` dep `.app`(s) read off disk), run the registered
/// detectors in CROSS-APP mode (`run_detectors_cross_app` — `dep_routine_ids`-derived
/// roles), then project + RE-SORT in stable space. The stable id map is built from the
/// MERGED `base.ws_routines` (so dep callee ids in d16 ids project correctly).
///
/// Engine-never-throws: a fail-closed / dep-less workspace (`build_r3a5_cross_app_base`
/// → None) yields an empty projection.
pub fn project_r4_findings_cross_app(
    workspace: &std::path::Path,
    model_instance_id: &str,
    detectors: &[Detector],
    fixture_name: &str,
    detector_names: &[String],
) -> R4FindingsProjection {
    let Some(base) =
        crate::engine::l4::capability_cone::build_r4_cross_app_base(workspace, model_instance_id)
    else {
        return R4FindingsProjection {
            fixture_name: fixture_name.to_string(),
            detectors: detector_names.to_vec(),
            finding_count: 0,
            findings: vec![],
        };
    };

    let RunOutput { findings, .. } = run_detectors_cross_app(&base, detectors);

    let detector_name_set: std::collections::HashSet<&str> =
        detector_names.iter().map(|s| s.as_str()).collect();

    let map = crate::engine::l4::summary::build_routine_stable_map(&base.ws_routines);
    let stable_finding_id = make_stable_finding_id_fn(&map);

    let mut stable: Vec<StableFinding> = findings
        .iter()
        .filter(|f| detector_name_set.contains(f.detector.as_str()))
        .map(|f| project_finding(f, &map, &stable_finding_id))
        .collect();

    stable.sort_by(|a, b| {
        crate::engine::l5::registry::compare_natural(&a.detector, &b.detector)
            .then_with(|| stable_primary_location_key(a).cmp(&stable_primary_location_key(b)))
            .then_with(|| a.root_cause_key.cmp(&b.root_cause_key))
    });

    R4FindingsProjection {
        fixture_name: fixture_name.to_string(),
        detectors: detector_names.to_vec(),
        finding_count: stable.len(),
        findings: stable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_id_projection_to_colon_form() {
        assert_eq!(
            map_table_id("11111111-0000-0000-0000-00000000d40a/table/18"),
            "11111111-0000-0000-0000-00000000d40a:Table:18"
        );
        assert_eq!(map_table_id("unknown"), "unknown");
    }

    #[test]
    fn object_id_projection_to_colon_form() {
        assert_eq!(
            map_object_id("11111111-0000-0000-0000-00000000d40a/Codeunit/50104"),
            "11111111-0000-0000-0000-00000000d40a:Codeunit:50104"
        );
    }

    #[test]
    fn finding_id_replacement_longest_first() {
        let mut map = HashMap::new();
        map.insert("r0/aaa".to_string(), "STABLE_A".to_string());
        map.insert("r0/aaabbb".to_string(), "STABLE_AB".to_string());
        let f = make_stable_finding_id_fn(&map);
        // The longer id is replaced first so the shorter prefix cannot shadow it.
        assert_eq!(f("d4/r0/aaabbb/loop0/x"), "d4/STABLE_AB/loop0/x");
        assert_eq!(f("d4/r0/aaa/loop0/x"), "d4/STABLE_A/loop0/x");
    }

    /// FIX 2 — single-pass guard: the stable VALUE of the longer key happens to
    /// contain a substring equal to the shorter key. Under the old iterative
    /// approach the second loop pass would corrupt "STABLE_AB" by replacing the
    /// embedded "r0/aaa" fragment; the single-pass approach never re-scans already
    /// substituted text so the stable value is emitted verbatim.
    #[test]
    fn finding_id_replacement_single_pass_no_rescan() {
        let mut map = HashMap::new();
        // Shorter key "r0/aaa" → stable value that ITSELF contains "r0/aaa".
        map.insert("r0/aaa".to_string(), "PREFIX_r0/aaa_SUFFIX".to_string());
        // Longer key "r0/aaabbb" → clean stable value.
        map.insert("r0/aaabbb".to_string(), "STABLE_AB".to_string());
        let f = make_stable_finding_id_fn(&map);
        // The longer match fires first → "STABLE_AB"; the shorter key must NOT
        // match again inside the already-substituted "STABLE_AB".
        assert_eq!(f("d4/r0/aaabbb/x"), "d4/STABLE_AB/x");
        // When only the shorter key is present, its stable value is emitted once
        // and the embedded "r0/aaa" inside it is NOT re-substituted.
        assert_eq!(
            f("d4/r0/aaa/x"),
            "d4/PREFIX_r0/aaa_SUFFIX/x",
            "single-pass must not re-scan the already-substituted stable value"
        );
    }

    /// FIX 3 — malformed TableId panics.
    #[test]
    #[should_panic(expected = "malformed TableId")]
    fn table_id_malformed_panics() {
        map_table_id("not/a/valid/table/id");
    }

    /// FIX 3 — two-segment malformed TableId also panics.
    #[test]
    #[should_panic(expected = "malformed TableId")]
    fn table_id_wrong_segment_panics() {
        map_table_id("11111111-0000-0000-0000-00000000d40a/Codeunit/50104");
    }

    // === Task C4 — compressed report schema + loop-set interning =============

    use crate::engine::l5::d1_cohort::{GroupBitmap, GroupIx, StableLoopSetRegistry};

    fn dummy_anchor(enclosing: &str) -> SourceAnchor {
        SourceAnchor {
            source_unit_id: "ws:test.al".to_string(),
            start_line: 1,
            start_column: 0,
            end_line: 1,
            end_column: 10,
            enclosing_routine_id: enclosing.to_string(),
            syntax_kind: "procedure".to_string(),
            normalized_text_hash: None,
            leading_context_hash: None,
            trailing_context_hash: None,
        }
    }

    fn dummy_step(routine: &str, note: &str) -> EvidenceStep {
        EvidenceStep {
            routine_id: routine.to_string(),
            operation_id: None,
            callsite_id: None,
            loop_id: None,
            source_anchor: dummy_anchor(routine),
            note: note.to_string(),
        }
    }

    fn dummy_witness() -> WitnessSummary {
        WitnessSummary {
            total_hops: 1,
            first_steps: vec![dummy_step("Loop", "loop"), dummy_step("Loop", "call")],
            omitted_hops: 0,
            last_steps: vec![],
            terminal_step: dummy_step("Term", "terminal"),
        }
    }

    /// A `D1CohortContext` decompresses (via the registry + catalog) to the
    /// EXACT `(loop, verdict, depth_bucket, unc)` tuples it represents: every
    /// loop named by its `loop_set` carries the cohort's own verdict/
    /// depth_bucket/uncertain (the cohort IS a verdict class).
    #[test]
    fn d1_cohort_context_decompresses_to_exact_tuples() {
        let mut registry = LoopSetRegistry::new();
        let mut bm = GroupBitmap::new();
        bm.set(0);
        bm.set(2);
        bm.set(5);
        let loop_set = registry.intern(&bm);

        let catalog: Vec<LoopCatalogEntry> = (0..6)
            .map(|ix| LoopCatalogEntry {
                loop_ix: ix,
                loop_routine_id: format!("R{ix}"),
                loop_id: format!("R{ix}/loop0"),
                anchor: dummy_anchor(&format!("R{ix}")),
                entry_callsite_id: Some(format!("R{ix}/cs0")),
            })
            .collect();

        let ctx = D1CohortContext {
            severity: "high".to_string(),
            verdict: "physical".to_string(),
            depth_bucket: 1,
            uncertain: false,
            loop_set,
            loop_count: 3,
            witness: dummy_witness(),
        };

        let tuples = decompress_cohort_context(&ctx, &registry, &catalog);
        let got: Vec<(u32, &str, i64, bool)> = tuples
            .iter()
            .map(|(e, v, d, u)| (e.loop_ix, *v, *d, *u))
            .collect();
        assert_eq!(
            got,
            vec![
                (0, "physical", 1, false),
                (2, "physical", 1, false),
                (5, "physical", 1, false),
            ]
        );
        // The catalog identities resolved are the RIGHT loops, not just the right count.
        assert_eq!(tuples[0].0.loop_routine_id, "R0");
        assert_eq!(tuples[1].0.loop_routine_id, "R2");
        assert_eq!(tuples[2].0.loop_routine_id, "R5");
    }

    /// Two cohorts (different verdict classes) sharing NO loops decompress to
    /// disjoint tuple sets, and their `loop_set`s intern to distinct ids.
    #[test]
    fn d1_cohort_context_disjoint_classes_decompress_disjoint() {
        let mut registry = LoopSetRegistry::new();
        let mut bm_a = GroupBitmap::new();
        bm_a.set(1);
        bm_a.set(3);
        let mut bm_b = GroupBitmap::new();
        bm_b.set(2);
        let loop_set_a = registry.intern(&bm_a);
        let loop_set_b = registry.intern(&bm_b);
        assert_ne!(loop_set_a, loop_set_b);

        let catalog: Vec<LoopCatalogEntry> = (0..4)
            .map(|ix| LoopCatalogEntry {
                loop_ix: ix,
                loop_routine_id: format!("R{ix}"),
                loop_id: format!("R{ix}/loop0"),
                anchor: dummy_anchor(&format!("R{ix}")),
                entry_callsite_id: None,
            })
            .collect();

        let ctx_a = D1CohortContext {
            severity: "high".to_string(),
            verdict: "physical".to_string(),
            depth_bucket: 1,
            uncertain: false,
            loop_set: loop_set_a,
            loop_count: 2,
            witness: dummy_witness(),
        };
        let ctx_b = D1CohortContext {
            severity: "medium".to_string(),
            verdict: "temporary".to_string(),
            depth_bucket: 0,
            uncertain: true,
            loop_set: loop_set_b,
            loop_count: 1,
            witness: dummy_witness(),
        };

        let got_a: Vec<u32> = decompress_cohort_context(&ctx_a, &registry, &catalog)
            .iter()
            .map(|(e, ..)| e.loop_ix)
            .collect();
        let got_b: Vec<u32> = decompress_cohort_context(&ctx_b, &registry, &catalog)
            .iter()
            .map(|(e, ..)| e.loop_ix)
            .collect();
        assert_eq!(got_a, vec![1, 3]);
        assert_eq!(got_b, vec![2]);
    }

    /// Stable serialization round-trip for a finding carrying `cohort_contexts`,
    /// alongside its run-level loop catalog + loop-set registry: internal ->
    /// stable -> JSON -> deserialize -> matches, for all three.
    #[test]
    fn finding_with_cohort_contexts_stable_round_trip() {
        let mut registry = LoopSetRegistry::new();
        let mut bm_a = GroupBitmap::new();
        bm_a.set(0);
        bm_a.set(3);
        let loop_set_a = registry.intern(&bm_a);
        let mut bm_b = GroupBitmap::new();
        bm_b.set(1);
        let loop_set_b = registry.intern(&bm_b);

        let catalog: Vec<LoopCatalogEntry> = vec![
            LoopCatalogEntry {
                loop_ix: 0,
                loop_routine_id: "R0".to_string(),
                loop_id: "R0/loop0".to_string(),
                anchor: dummy_anchor("R0"),
                entry_callsite_id: None,
            },
            LoopCatalogEntry {
                loop_ix: 1,
                loop_routine_id: "R1".to_string(),
                loop_id: "R1/loop0".to_string(),
                anchor: dummy_anchor("R1"),
                entry_callsite_id: None,
            },
            LoopCatalogEntry {
                loop_ix: 3,
                loop_routine_id: "R3".to_string(),
                loop_id: "R3/loop0".to_string(),
                anchor: dummy_anchor("R3"),
                entry_callsite_id: None,
            },
        ];

        let finding = Finding {
            id: "d1/R0/T/T/op0".to_string(),
            root_cause_key: "d1/T/T/op0".to_string(),
            detector: "d1".to_string(),
            title: "title".to_string(),
            root_cause: "root cause".to_string(),
            severity: "high".to_string(),
            confidence: FindingConfidence {
                level: "high".to_string(),
                capped_by: None,
                evidence: vec![],
            },
            primary_location: dummy_anchor("T"),
            evidence_path: vec![],
            additional_paths: None,
            affected_objects: vec![],
            affected_tables: vec![],
            fix_options: vec![],
            provenance: vec![],
            actionable_anchor: None,
            fingerprint: None,
            event_kind: None,
            cross_extension_subscribers: None,
            contexts: None,
            cohort_contexts: Some(vec![
                D1CohortContext {
                    severity: "high".to_string(),
                    verdict: "physical".to_string(),
                    depth_bucket: 1,
                    uncertain: false,
                    loop_set: loop_set_a,
                    loop_count: 2,
                    witness: dummy_witness(),
                },
                D1CohortContext {
                    severity: "medium".to_string(),
                    verdict: "temporary".to_string(),
                    depth_bucket: 0,
                    uncertain: true,
                    loop_set: loop_set_b,
                    loop_count: 1,
                    witness: dummy_witness(),
                },
            ]),
        };

        let map: HashMap<String, String> = HashMap::new();
        let stable_id_fn = |s: &str| s.to_string();
        let stable = project_finding(&finding, &map, &stable_id_fn);

        let cc = stable
            .cohort_contexts
            .as_ref()
            .expect("cohort_contexts survives projection");
        assert_eq!(cc.len(), 2);
        assert_eq!(cc[0].loop_set, loop_set_a);
        assert_eq!(cc[1].loop_set, loop_set_b);

        // Round trip the finding's stable JSON.
        let json = serde_json::to_string_pretty(&stable).unwrap();
        assert!(json.contains("\"cohortContexts\""));
        let back: StableFinding = serde_json::from_str(&json).unwrap();
        assert_eq!(stable, back);

        // Round trip the run-level catalog alongside it.
        let stable_catalog: Vec<StableLoopCatalogEntry> = catalog
            .iter()
            .map(|e| project_loop_catalog_entry(e, &map))
            .collect();
        let catalog_json = serde_json::to_string(&stable_catalog).unwrap();
        let catalog_back: Vec<StableLoopCatalogEntry> =
            serde_json::from_str(&catalog_json).unwrap();
        assert_eq!(stable_catalog, catalog_back);

        // Round trip the run-level registry alongside it.
        let stable_registry = registry.to_stable();
        let registry_json = serde_json::to_string(&stable_registry).unwrap();
        let registry_back: StableLoopSetRegistry = serde_json::from_str(&registry_json).unwrap();
        assert_eq!(stable_registry, registry_back);

        // The rebuilt registry decompresses the SAME loop-set ids to the SAME
        // loop-group indices as the original.
        for c in cc {
            let orig: Vec<GroupIx> = registry.iter(c.loop_set).collect();
            let rebuilt_ids: Vec<GroupIx> = registry_back.to_registry().iter(c.loop_set).collect();
            assert_eq!(orig, rebuilt_ids);
        }
    }
}
