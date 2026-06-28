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

/// The fully stable-projected Finding. Field order = al-sem `projectFinding`
/// insertion order; the OPTION tail is in golden order:
/// additionalPaths, actionableAnchor, fingerprint, eventKind, crossExtensionSubscribers.
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
///   input (stable-identity.ts). `run_detectors` wraps detector runs in
///   `catch_unwind`; a projection failure is treated as a hard error.
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
}
