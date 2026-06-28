//! R3a-4 — the dep-hook PROJECTION (the differential/oracle surface). Rust port of
//! al-sem's `scripts/r3a4-projection.ts` (`projectR3a4` + `DepIdStabilizer`).
//!
//! ## What this is
//!
//! Given a workspace (a root `app.json` + `.alpackages` deps), this:
//!   1. builds each dep `.app`'s R3a-4 producer artifact (`build_dep_artifact_l4`,
//!      the embedded-source path),
//!   2. drives the consumer hooks (`inject_intra_app_call_edges` /
//!      `collect_cited_dep_evidence` / `collect_dep_order_index`) over a merged
//!      model whose routine membership = the workspace's own routines + every dep's
//!      own routines (mirrors al-sem's `withDependencyArtifacts` merge — both ends
//!      of an intra-dep edge are dep routines, so they are present),
//!   3. STABLE-PROJECTS every id-bearing field internal→stable via [`DepIdStabilizer`],
//!   4. emits the SAME stable JSON shape/key-order as al-sem's
//!      `cross-app-dep-hooks.r3a4.golden.json`.
//!
//! ## Stable id form (THE Task-3 fix)
//!
//! A dep routine's INTERNAL id (`<modelInstanceId>/<keyHash>[/opN|/csN]`) is
//! modelInstanceId/devFingerprint-keyed → NOT reproducible by another engine. al-sem
//! and the Rust port both STABLE-PROJECT it to
//! `<appGuid>:<Type>:<Num>#<normalizedSignatureHash>[/opN|/csN]`, which is
//! appGuid/signature-derived → cache/modelInstanceId/devFingerprint-INDEPENDENT.
//! The Rust dep routine already carries `stable_routine_id` =
//! `to_stable_object_id(object_id) + "#" + normalized_signature_hash` — exactly
//! al-sem's `DepIdStabilizer` base. The `/opN` / `/csN` suffix (everything after
//! the routine id, which is exactly two `/`-parts) is re-attached onto the stable
//! base.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::engine::deps::dep_artifact_l4::{
    ConsumerModel, DepCallEdge, DepOperationEvidence, DepReturnSummaryRecord, DepRoutineOrderEntry,
    DependencyArtifactL4, InjectedTypedEdge, TriBool, build_dep_artifact_l4,
    collect_cited_dep_evidence, collect_dep_order_index, inject_intra_app_call_edges,
    is_dep_order_index_stamp_fresh,
};
use crate::engine::deps::merged_index::collect_app_paths;
use crate::engine::l3::l3_workspace::assemble_and_resolve_workspace_default;

/// modelInstanceId for the R3a-4 dep producer (the emitted ids are stable-projected
/// → modelInstanceId-INDEPENDENT; pinned to match the al-sem capture's `r0`).
pub const R3A4_MODEL_INSTANCE_ID: &str = "r0";

// ---------------------------------------------------------------------------
// The DepIdStabilizer — internal dep id → stable, cache-INDEPENDENT id.
// ---------------------------------------------------------------------------

/// Maps INTERNAL dep ids (`<modelInstanceId>/<keyHash>[/opN|/csN]`) → STABLE,
/// cache/devFingerprint-INDEPENDENT ids
/// (`<appGuid>:<Type>:<Num>#<sigHash>[/opN|/csN]`). Port of al-sem's
/// `DepIdStabilizer` (`scripts/r3a4-projection.ts`).
///
/// Built from the dep producer's own routines: each carries the internal `id` plus
/// the cache-independent `stable_routine_id`. al-sem uses longest-internal-prefix
/// matching; here the internal RoutineId is exactly two `/`-parts, so an op/callsite
/// sub-id splits cleanly at the LAST `/`, and the base maps directly.
pub struct DepIdStabilizer {
    /// internal routine-base id → stable routine-base id.
    by_internal: BTreeMap<String, String>,
}

impl DepIdStabilizer {
    /// Build from `(internal_id, stable_routine_id)` pairs (the dep's own routines).
    pub fn new<I>(routines: I) -> Self
    where
        I: IntoIterator<Item = (String, String)>,
    {
        let mut by_internal = BTreeMap::new();
        for (internal, stable) in routines {
            by_internal.insert(internal, stable);
        }
        Self { by_internal }
    }

    /// Stable-project an internal routine-base id (exact match required; an unknown
    /// id is a real divergence — fail LOUD rather than silently emit an internal id).
    fn stable_routine(&self, internal: &str) -> String {
        self.by_internal.get(internal).cloned().unwrap_or_else(|| {
            panic!(
                "DepIdStabilizer: no dependency routine base matches internal id {:?}. \
                 Known bases: {:?}",
                internal,
                self.by_internal.keys().collect::<Vec<_>>()
            )
        })
    }

    /// Stable-project an id that is either a routine-base id OR a routine-base id
    /// followed by a `/opN` / `/csN` suffix. The internal RoutineId is exactly two
    /// `/`-parts (`<modelInstanceId>/<keyHash>`), so the suffix is everything after
    /// the LAST `/`; a bare routine id has no third part and maps directly.
    pub fn stable(&self, internal_id: &str) -> String {
        // Exact routine-base match first.
        if let Some(s) = self.by_internal.get(internal_id) {
            return s.clone();
        }
        // Else split off the `/opN` | `/csN` suffix at the last `/`.
        match internal_id.rsplit_once('/') {
            Some((prefix, suffix)) => format!("{}/{}", self.stable_routine(prefix), suffix),
            None => self.stable_routine(internal_id),
        }
    }
}

// ---------------------------------------------------------------------------
// Projected types — the R3a-4 dep-hook comparison surface (al-sem key order).
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PDepCallEdge {
    pub from: String,
    pub to: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub callsite_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PDepOperationEvidence {
    pub operation_id: String,
    pub source_file: String,
    pub start_line: u32,
    pub start_column: u32,
    pub end_line: u32,
    pub end_column: u32,
    pub display_text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub control_context: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PDepReturnSummaryRecord {
    pub routine_id: String,
    pub has_normal_return_path: TriBoolJson,
    pub all_paths_error: TriBoolJson,
    pub has_try_function_boundary: bool,
    pub coverage: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit_behavior: Option<String>,
}

/// `boolean | "unknown"` JSON shape (serde-tagged untagged: a bool or the string
/// `"unknown"`), mirroring al-sem's `hasNormalReturnPath: boolean | "unknown"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriBoolJson {
    Bool(bool),
    Unknown,
}

impl From<TriBool> for TriBoolJson {
    fn from(t: TriBool) -> Self {
        match t {
            TriBool::True => TriBoolJson::Bool(true),
            TriBool::False => TriBoolJson::Bool(false),
            TriBool::Unknown => TriBoolJson::Unknown,
        }
    }
}

impl Serialize for TriBoolJson {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            TriBoolJson::Bool(b) => s.serialize_bool(*b),
            TriBoolJson::Unknown => s.serialize_str("unknown"),
        }
    }
}

impl<'de> Deserialize<'de> for TriBoolJson {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = serde_json::Value::deserialize(d)?;
        match v {
            serde_json::Value::Bool(b) => Ok(TriBoolJson::Bool(b)),
            serde_json::Value::String(ref s) if s == "unknown" => Ok(TriBoolJson::Unknown),
            other => Err(serde::de::Error::custom(format!(
                "expected boolean | \"unknown\", got {other}"
            ))),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PDepOrderIndexStamp {
    pub app_id: String,
    pub version: String,
    pub order_index_schema_version: String,
    pub fresh: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PDepOrderIndex {
    pub stamp: PDepOrderIndexStamp,
    pub routine_count: usize,
    pub return_summary_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PDepArtifactPayload {
    pub app_guid: String,
    pub name: String,
    pub version: String,
    pub summary_mode: String,
    pub intra_app_call_edges: Vec<PDepCallEdge>,
    pub cited_operation_evidence: Vec<PDepOperationEvidence>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dep_order_index: Option<PDepOrderIndex>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PInjectedTypedEdge {
    pub kind: String,
    pub from: String,
    pub to: String,
    pub callsite_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PDepRoutineOrderEntry {
    pub routine_id: String,
    pub scope_frame_count: usize,
    pub operation_order_count: usize,
    pub callsite_order_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PConsumedEffect {
    pub injected_typed_edges: Vec<PInjectedTypedEdge>,
    pub cited_dep_operation_evidence: Vec<PDepOperationEvidence>,
    pub dep_routine_order_entries: Vec<PDepRoutineOrderEntry>,
    pub dep_return_summaries: Vec<PDepReturnSummaryRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct R3a4Projection {
    pub fixture_name: String,
    pub artifact_payloads: Vec<PDepArtifactPayload>,
    pub consumed_effect: PConsumedEffect,
    pub intra_app_call_edges_count: usize,
    pub injected_typed_edges_count: usize,
    pub cited_evidence_count: usize,
    pub order_entries_count: usize,
    pub return_summaries_count: usize,
    pub dep_order_index_present: bool,
    pub freshness_stamp_fresh: bool,
}

// ---------------------------------------------------------------------------
// Projection helpers.
// ---------------------------------------------------------------------------

fn cmp_str(a: &str, b: &str) -> std::cmp::Ordering {
    a.cmp(b)
}

fn project_dep_call_edge(e: &DepCallEdge, stab: &DepIdStabilizer) -> PDepCallEdge {
    PDepCallEdge {
        from: stab.stable(&e.from),
        to: stab.stable(&e.to),
        callsite_id: e.callsite_id.as_ref().map(|c| stab.stable(c)),
    }
}

fn project_dep_operation_evidence(
    e: &DepOperationEvidence,
    stab: &DepIdStabilizer,
) -> PDepOperationEvidence {
    PDepOperationEvidence {
        operation_id: stab.stable(&e.operation_id),
        source_file: e.source_file.clone(),
        start_line: e.start_line,
        start_column: e.start_column,
        end_line: e.end_line,
        end_column: e.end_column,
        display_text: e.display_text.clone(),
        control_context: e.control_context.clone(),
    }
}

fn project_dep_return_summary(
    rs: &DepReturnSummaryRecord,
    stab: &DepIdStabilizer,
) -> PDepReturnSummaryRecord {
    PDepReturnSummaryRecord {
        routine_id: stab.stable(&rs.routine_id),
        has_normal_return_path: rs.has_normal_return_path.into(),
        all_paths_error: rs.all_paths_error.into(),
        has_try_function_boundary: rs.has_try_function_boundary,
        coverage: rs.coverage.clone(),
        // al-sem always emits commitBehavior (it is a required field on the dep
        // return-summary record); the Rust producer always populates it.
        commit_behavior: Some(rs.commit_behavior.clone()),
    }
}

fn project_dep_artifact_payload(
    artifact: &DependencyArtifactL4,
    stab: &DepIdStabilizer,
) -> PDepArtifactPayload {
    let h = &artifact.header;
    let abi = &artifact.abi;

    let mut edges: Vec<PDepCallEdge> = abi
        .intra_app_call_edges
        .iter()
        .map(|e| project_dep_call_edge(e, stab))
        .collect();
    edges.sort_by(|a, b| cmp_str(&a.from, &b.from).then(cmp_str(&a.to, &b.to)));

    let mut evidence: Vec<PDepOperationEvidence> = abi
        .cited_operation_evidence
        .iter()
        .map(|e| project_dep_operation_evidence(e, stab))
        .collect();
    evidence.sort_by(|a, b| cmp_str(&a.operation_id, &b.operation_id));

    let dep_order_index = abi.dep_order_index.as_ref().map(|idx| {
        let fresh = is_dep_order_index_stamp_fresh(&idx.stamp, h);
        PDepOrderIndex {
            stamp: PDepOrderIndexStamp {
                app_id: idx.stamp.app_id.clone(),
                version: idx.stamp.version.clone(),
                order_index_schema_version: idx.stamp.order_index_schema_version.clone(),
                fresh,
            },
            routine_count: idx.routines.len(),
            return_summary_count: idx.return_summaries.len(),
        }
    });

    PDepArtifactPayload {
        app_guid: h.app_guid.clone(),
        name: h.name.clone(),
        version: h.version.clone(),
        summary_mode: h.summary_mode.clone(),
        intra_app_call_edges: edges,
        cited_operation_evidence: evidence,
        dep_order_index,
    }
}

fn project_injected_typed_edge(
    e: &InjectedTypedEdge,
    stab: &DepIdStabilizer,
) -> PInjectedTypedEdge {
    PInjectedTypedEdge {
        kind: e.kind.clone(),
        from: stab.stable(&e.from),
        to: stab.stable(&e.to),
        callsite_id: stab.stable(&e.callsite_id),
    }
}

fn project_consumed_effect(model: &ConsumerModel, stab: &DepIdStabilizer) -> PConsumedEffect {
    let mut injected: Vec<PInjectedTypedEdge> = model
        .injected_typed_edges
        .iter()
        .map(|e| project_injected_typed_edge(e, stab))
        .collect();
    injected.sort_by(|a, b| cmp_str(&a.from, &b.from).then(cmp_str(&a.to, &b.to)));

    let mut cited: Vec<PDepOperationEvidence> = model
        .cited_dep_operation_evidence
        .iter()
        .map(|e| project_dep_operation_evidence(e, stab))
        .collect();
    cited.sort_by(|a, b| cmp_str(&a.operation_id, &b.operation_id));

    let mut order_entries: Vec<PDepRoutineOrderEntry> = model
        .dep_routine_order_entries
        .values()
        .map(|entry: &DepRoutineOrderEntry| PDepRoutineOrderEntry {
            routine_id: stab.stable(&entry.routine_id),
            scope_frame_count: entry.scope_frames.len(),
            operation_order_count: entry.operation_orders.len(),
            callsite_order_count: entry.callsite_orders.len(),
        })
        .collect();
    order_entries.sort_by(|a, b| cmp_str(&a.routine_id, &b.routine_id));

    let mut return_summaries: Vec<PDepReturnSummaryRecord> = model
        .dep_return_summaries
        .values()
        .map(|rs| project_dep_return_summary(rs, stab))
        .collect();
    return_summaries.sort_by(|a, b| cmp_str(&a.routine_id, &b.routine_id));

    PConsumedEffect {
        injected_typed_edges: injected,
        cited_dep_operation_evidence: cited,
        dep_routine_order_entries: order_entries,
        dep_return_summaries: return_summaries,
    }
}

// ---------------------------------------------------------------------------
// The full R3a-4 projection from a workspace.
// ---------------------------------------------------------------------------

/// Run the R3a-4 producer + consumer hooks over a workspace and project both
/// surfaces in the stable golden shape. Port of al-sem `projectR3a4`.
///
/// The merged-model routine membership (the injection both-ends guard) = the
/// workspace's own routines + every dep's own routines. Engine-never-throws: a
/// fail-closed / dep-less workspace yields an empty projection.
pub fn project_r3a4_from_workspace(workspace: &Path, fixture_name: &str) -> R3a4Projection {
    // --- build each dep `.app`'s R3a-4 producer artifact ---
    let alpackages = workspace.join(".alpackages");
    let app_paths = collect_app_paths(&alpackages);
    let mut artifacts: Vec<DependencyArtifactL4> = Vec::new();
    for p in &app_paths {
        let Ok(bytes) = std::fs::read(p) else {
            continue;
        };
        if let Some(a) = build_dep_artifact_l4(&bytes, R3A4_MODEL_INSTANCE_ID) {
            artifacts.push(a);
        }
    }

    // --- merged-model routine membership: workspace own routines + dep own routines.
    // The workspace native routines (so a future cross-app edge with a workspace end
    // would be admitted) + each dep's own routines (both ends of an intra-dep edge).
    let mut routine_ids: Vec<String> = Vec::new();
    if let Some(resolved) = assemble_and_resolve_workspace_default(workspace) {
        for r in &resolved.workspace.routines {
            routine_ids.push(r.id.clone());
        }
    }
    for a in &artifacts {
        for id in &a.abi.routines_ids {
            routine_ids.push(id.clone());
        }
    }

    let mut model = ConsumerModel::with_routine_ids(routine_ids);
    inject_intra_app_call_edges(&mut model, &artifacts);
    collect_cited_dep_evidence(&mut model, &artifacts);
    collect_dep_order_index(&mut model, &artifacts);

    // --- build the internal→stable dep-id mapper from every dep's own routines.
    // Each carries (internal id, stable_routine_id) — both cache-INDEPENDENT.
    let stab = build_stabilizer(&artifacts, &app_paths);

    // --- producer payloads (sorted by appGuid) ---
    let mut artifact_payloads: Vec<PDepArtifactPayload> = artifacts
        .iter()
        .map(|a| project_dep_artifact_payload(a, &stab))
        .collect();
    artifact_payloads.sort_by(|a, b| cmp_str(&a.app_guid, &b.app_guid));

    let consumed_effect = project_consumed_effect(&model, &stab);

    let intra_app_call_edges_count: usize = artifact_payloads
        .iter()
        .map(|p| p.intra_app_call_edges.len())
        .sum();
    let injected_typed_edges_count = consumed_effect.injected_typed_edges.len();
    let cited_evidence_count = consumed_effect.cited_dep_operation_evidence.len();
    let order_entries_count = consumed_effect.dep_routine_order_entries.len();
    let return_summaries_count = consumed_effect.dep_return_summaries.len();
    let dep_order_index_present = artifact_payloads
        .iter()
        .any(|p| p.dep_order_index.is_some());
    let freshness_stamp_fresh = artifact_payloads
        .iter()
        .any(|p| p.dep_order_index.as_ref().map(|i| i.stamp.fresh) == Some(true));

    R3a4Projection {
        fixture_name: fixture_name.to_string(),
        artifact_payloads,
        consumed_effect,
        intra_app_call_edges_count,
        injected_typed_edges_count,
        cited_evidence_count,
        order_entries_count,
        return_summaries_count,
        dep_order_index_present,
        freshness_stamp_fresh,
    }
}

/// Build the internal→stable id mapper from every dep producer's own routines.
/// Re-derives each dep's L3 routine table (the producer discards it; we re-run the
/// embedded-source assemble+resolve to recover `(id, stable_routine_id)` pairs).
fn build_stabilizer(
    artifacts: &[DependencyArtifactL4],
    app_paths: &[std::path::PathBuf],
) -> DepIdStabilizer {
    use crate::engine::deps::app_manifest::parse_app_manifest_xml;
    use crate::engine::deps::app_package_zip::extract_navx_manifest_xml;
    use crate::engine::deps::dep_artifact_l4::iterate_embedded_source;
    use crate::engine::l3::l3_workspace::{L3Workspace, assemble_workspace_units, resolve};

    let _ = artifacts; // membership of stab is derived from the same .app bytes.
    let mut pairs: Vec<(String, String)> = Vec::new();
    for p in app_paths {
        let Ok(bytes) = std::fs::read(p) else {
            continue;
        };
        let Some(manifest_xml) = extract_navx_manifest_xml(&bytes) else {
            continue;
        };
        let manifest = parse_app_manifest_xml(&manifest_xml);
        if manifest.error.is_some() || manifest.identity.app_guid.is_empty() {
            continue;
        }
        let app_guid = manifest.identity.app_guid.clone();
        let embedded = iterate_embedded_source(&bytes);
        let units: Vec<(String, String)> = embedded
            .iter()
            .map(|f| {
                (
                    format!("dep:{app_guid}:{}", f.relative_path),
                    f.content.clone(),
                )
            })
            .collect();
        let mut ws: L3Workspace =
            assemble_workspace_units(&units, &app_guid, R3A4_MODEL_INSTANCE_ID);
        resolve(&mut ws);
        for r in &ws.routines {
            if r.app_guid == app_guid {
                pairs.push((r.id.clone(), r.stable_routine_id.clone()));
            }
        }
    }
    DepIdStabilizer::new(pairs)
}
