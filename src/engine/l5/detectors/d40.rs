//! D40 — transitive load missing. Port of al-sem
//! `src/detectors/d40-transitive-load-missing.ts`.
//!
//! For each resolved call edge where the callee requires its parameter loaded at
//! entry (`callee.parameterRoles[Q].requiresLoadedAtEntry === "yes"`), verify the
//! caller has loaded the forwarded record before the callsite; otherwise emit at
//! the caller's callsite.
//!
//! Severity `medium`, escalating to `high` when the callee mutates the unloaded
//! record (`mutatesBeforeLoad === "yes"`).
//!
//! OPT-IN: al-sem keeps D40 out of the default registry (the straight-line walker
//! over-flags loop-loaded records). The Rust port registers it; `project_r4_findings`
//! filters by name, so it only contributes when explicitly requested.
//!
//! Reads the CORE `RoutineSummary.parameterRoles` via `ctx.parameter_roles_by_routine`
//! and the resolved per-callsite edge via `ctx.resolved_call_edge_by_callsite`. Joins
//! to the L3 source-side bindings (`cs.argument_bindings`) — the source-side fields
//! (sourceKind / sourceTempState / sourceVariableName / sourceRecordVariableId) live
//! on the L3 binding directly; the post-upgrade `bindingResolution` lives on
//! `ctx.upgraded_bindings_by_callsite` joined POSITIONALLY by index.

use std::collections::HashMap;

use crate::engine::l3::l3_workspace::{L3RecordOperation, L3Resolved};
use crate::engine::l4::effect_lattice::EffectPresence;
use crate::engine::l5::confidence::to_confidence;
use crate::engine::l5::detector_context::DetectorContext;
use crate::engine::l5::detectors::{anchor_of, before_anchor};
use crate::engine::l5::finding::{Evidence, EvidenceStep, Finding, FixOption};
use crate::engine::l5::fingerprint::FingerprintIndex;
use crate::engine::l5::registry::{DetectorError, DetectorOutput, DetectorStats};

const DETECTOR: &str = "d40-transitive-load-missing";

/// Record-op roles that put the record in a well-defined loaded/initialised state.
/// Mirrors al-sem's `isLoadingOp` (loadsFromDb / initialises / copiesInto). The L5
/// `recordFlowRoleOf` op classifier is reproduced here by op name (the set is
/// closed: Get/Find*/Next load; Init initialises; Copy/TransferFields copy into).
fn is_loading_op(op: &str) -> bool {
    matches!(
        op,
        // loadsFromDb
        "Get" | "FindFirst" | "FindLast" | "FindSet" | "Find" | "Next"
        // initialises
        | "Init"
        // copiesInto
        | "Copy" | "TransferFields"
    )
}

pub fn detect_d40(
    resolved: &L3Resolved,
    ctx: &DetectorContext,
) -> Result<DetectorOutput, DetectorError> {
    let ws = &resolved.workspace;
    let fp_index = FingerprintIndex::build(&ws.routines, &ws.objects);
    let mut findings: Vec<Finding> = Vec::new();
    let mut candidates_considered = 0usize;
    let mut skipped_unresolved = 0u64;
    let mut skipped_implicit_rec = 0u64;
    let mut skipped_temp_record = 0u64;
    let mut skipped_caller_loaded = 0u64;
    let mut skipped_callee_unknown = 0u64;

    for routine in &ws.routines {
        // roleOf(routine) !== "primary" → skip. Source-only ⇒ all primary.
        if !routine.body_available {
            continue;
        }
        if routine.parse_incomplete {
            continue;
        }

        // Precompute load-op buckets per source variable (key = lowercase name).
        let mut loads_by_source_lc: HashMap<String, Vec<&L3RecordOperation>> = HashMap::new();
        for op in &routine.record_operations {
            if !is_loading_op(&op.op) {
                continue;
            }
            loads_by_source_lc
                .entry(op.record_variable_name.to_lowercase())
                .or_default()
                .push(op);
        }

        for cs in &routine.call_sites {
            let edge = match ctx.resolved_call_edge_by_callsite.get(&cs.id) {
                Some(e) => e,
                None => {
                    skipped_unresolved += 1;
                    continue;
                }
            };
            let to = match &edge.to {
                Some(t) => t,
                None => continue,
            };
            let callee = match ctx.routine_by_id.get(to.as_str()) {
                Some(c) => *c,
                None => continue,
            };

            let upgraded = ctx.upgraded_bindings_by_callsite.get(&cs.id);
            for (i, binding) in cs.argument_bindings.iter().enumerate() {
                // C2(a): implicit-rec narrowing — checked BEFORE bindingResolution.
                if binding.source_kind == "implicit-rec" {
                    skipped_implicit_rec += 1;
                    continue;
                }
                let binding_resolution = upgraded
                    .and_then(|u| u.get(i))
                    .map(|u| u.binding_resolution.as_str());
                if binding_resolution != Some("resolved") {
                    skipped_unresolved += 1;
                    continue;
                }
                // sourceTempState known/true → temp record, no DB load concept.
                if let Some(ts) = &binding.source_temp_state
                    && ts.kind == "known"
                    && ts.value == Some(true)
                {
                    skipped_temp_record += 1;
                    continue;
                }
                let callee_role =
                    match ctx
                        .parameter_roles_by_routine
                        .get(&callee.id)
                        .and_then(|roles| {
                            roles
                                .iter()
                                .find(|r| r.parameter_index == binding.parameter_index)
                        }) {
                        Some(r) => r,
                        None => {
                            skipped_callee_unknown += 1;
                            continue;
                        }
                    };
                if callee_role.requires_loaded_at_entry != EffectPresence::Yes {
                    continue;
                }
                candidates_considered += 1;

                let source_name_lc = match &binding.source_variable_name {
                    Some(n) => n.clone(),
                    None => continue,
                };
                let empty: Vec<&L3RecordOperation> = Vec::new();
                let bucket = loads_by_source_lc.get(&source_name_lc).unwrap_or(&empty);
                let source_id = binding.source_record_variable_id.as_deref();
                let loaded_before = bucket.iter().any(|op| {
                    if !before_anchor(&op.source_anchor, &cs.source_anchor) {
                        return false;
                    }
                    match (source_id, op.record_variable_id.as_deref()) {
                        (Some(sid), Some(oid)) => oid == sid,
                        _ => true, // name-match already established by bucket lookup
                    }
                });
                if loaded_before {
                    skipped_caller_loaded += 1;
                    continue;
                }

                let mutates = callee_role.mutates_before_load == EffectPresence::Yes;
                let severity = if mutates { "high" } else { "medium" };
                let verb = if mutates { "mutates" } else { "reads" };
                let verb_ing = if mutates { "mutating" } else { "reading" };

                let path = vec![
                    EvidenceStep {
                        routine_id: routine.id.clone(),
                        operation_id: None,
                        callsite_id: Some(cs.id.clone()),
                        loop_id: None,
                        source_anchor: anchor_of(&binding.argument_anchor, routine),
                        note: format!(
                            "forwards {} to {} (param[{}])",
                            binding.source_variable_name.as_deref().unwrap_or(""),
                            callee.name,
                            binding.parameter_index
                        ),
                    },
                    EvidenceStep {
                        routine_id: callee.id.clone(),
                        operation_id: None,
                        callsite_id: None,
                        loop_id: None,
                        source_anchor: anchor_of(&callee.source_anchor, callee),
                        note: format!("{} {} this record before loading it", callee.name, verb),
                    },
                ];

                let id = format!("d40/{}/{}/{}", routine.id, cs.id, binding.parameter_index);
                let mut affected_objects =
                    vec![routine.object_id.clone(), callee.object_id.clone()];
                affected_objects.sort();

                let mut finding = Finding {
                    id: id.clone(),
                    root_cause_key: id,
                    detector: DETECTOR.to_string(),
                    title: format!("Forwarded record not loaded before {verb_ing} helper"),
                    root_cause: format!(
                        "{} forwards {} to {}, which {} the record without loading it — the caller must Get/Find the record before the call.",
                        routine.name,
                        binding.source_variable_name.as_deref().unwrap_or(""),
                        callee.name,
                        verb
                    ),
                    severity: severity.to_string(),
                    confidence: to_confidence(&[], "likely"),
                    primary_location: anchor_of(&binding.argument_anchor, routine),
                    evidence_path: path,
                    additional_paths: None,
                    affected_objects,
                    affected_tables: Vec::new(),
                    fix_options: vec![FixOption {
                        description: format!(
                            "Load {} with Get / FindFirst before forwarding to {}, or have {} load its parameter internally.",
                            binding.source_variable_name.as_deref().unwrap_or(""),
                            callee.name,
                            callee.name
                        ),
                        safety: "high".to_string(),
                    }],
                    provenance: vec![Evidence {
                        source: "tree-sitter".to_string(),
                        note: None,
                    }],
                    actionable_anchor: None,
                    fingerprint: None,
                    event_kind: None,
                    cross_extension_subscribers: None,
                };
                finding.fingerprint = Some(fp_index.fingerprint_of(&finding));
                findings.push(finding);
            }
        }
    }

    findings.sort_by(|a, b| a.id.cmp(&b.id));

    let emitted = findings.len();
    let mut stats = DetectorStats::new(DETECTOR, candidates_considered, emitted);
    stats.add_skip("unresolved", skipped_unresolved);
    stats.add_skip("implicitRec", skipped_implicit_rec);
    stats.add_skip("tempRecord", skipped_temp_record);
    stats.add_skip("callerLoaded", skipped_caller_loaded);
    stats.add_skip("calleeUnknown", skipped_callee_unknown);
    Ok(DetectorOutput::no_diag(findings, stats))
}
