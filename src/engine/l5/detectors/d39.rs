//! D39 — record left dirty across helper chain. Port of al-sem
//! `src/detectors/d39-record-left-dirty-across-chain.ts`.
//!
//! For each var-param P of every primary callee where the path-aware walker PROVED
//! `dirtyAtExit[P] === "yes"`, walk the reverse call graph. Every primary caller that
//! forwards a record to P by-var, does NOT persist that source after the callsite, and
//! does NOT pass it from a by-value parameter, is flagged: the Validate's field write
//! is silently discarded across the chain.
//!
//! Reads the CORE `RoutineSummary.parameterRoles` via `ctx.parameter_roles_by_routine`
//! (the `dirtyAtExit` fact), `ctx.reverse_call_graph`, and the post-upgrade per-callsite
//! bindings via `ctx.upgraded_bindings_by_callsite` joined positionally with
//! `call_site.argument_bindings`.

use crate::engine::l3::l3_workspace::L3Resolved;
use crate::engine::l4::effect_lattice::EffectPresence;
use crate::engine::l5::confidence::to_confidence;
use crate::engine::l5::detector_context::DetectorContext;
use crate::engine::l5::detectors::{anchor_of, before_anchor};
use crate::engine::l5::finding::{Evidence, EvidenceStep, Finding, FixOption};
use crate::engine::l5::fingerprint::FingerprintIndex;
use crate::engine::l5::registry::{DetectorOutput, DetectorStats};

const DETECTOR: &str = "d39-record-left-dirty-across-chain";

const PERSIST_OPS: &[&str] = &["Modify", "Insert", "Rename"];

pub fn detect_d39(resolved: &L3Resolved, ctx: &DetectorContext) -> DetectorOutput {
    let ws = &resolved.workspace;
    let fp_index = FingerprintIndex::build(&ws.routines, &ws.objects);
    let mut findings: Vec<Finding> = Vec::new();
    let mut candidates_considered = 0usize;
    let mut skipped_caller_persists = 0u64;

    for callee in &ws.routines {
        if !callee.body_available {
            continue;
        }
        let roles = match ctx.parameter_roles_by_routine.get(&callee.id) {
            Some(r) => r,
            None => continue,
        };
        for role in roles {
            match role.dirty_at_exit {
                EffectPresence::Unknown => continue, // unknownDirtyCallee
                EffectPresence::Yes => {}
                EffectPresence::No => continue,
            }

            // All resolved callers that forward a record to this var-parameter.
            let caller_edges = match ctx.reverse_call_graph.get(&callee.id) {
                Some(e) => e,
                None => continue,
            };
            for edge in caller_edges {
                let callsite_id = match &edge.callsite_id {
                    Some(c) => c,
                    None => continue,
                };
                let caller = match ctx.routine_by_id.get(edge.from.as_str()) {
                    Some(c) => *c,
                    None => continue,
                };
                // roleOf(caller) !== "primary" → skip. Source-only ⇒ all primary.
                if !caller.body_available {
                    continue;
                }

                let cs = match caller.call_sites.iter().find(|c| &c.id == callsite_id) {
                    Some(c) => c,
                    None => continue,
                };

                // binding = cs.argumentBindings.find(parameterIndex == role.parameterIndex
                //   && bindingResolution === "resolved" && calleeParameterIsVar)
                let upgraded = ctx.upgraded_bindings_by_callsite.get(&cs.id);
                let binding_idx = cs.argument_bindings.iter().enumerate().find(|(i, b)| {
                    if b.parameter_index != role.parameter_index {
                        return false;
                    }
                    let up = upgraded.and_then(|u| u.get(*i));
                    up.map(|u| u.binding_resolution == "resolved" && u.callee_parameter_is_var)
                        .unwrap_or(false)
                });
                let (i, binding) = match binding_idx {
                    Some((i, b)) => (i, b),
                    None => continue,
                };
                let _ = i;

                // Only source kinds the caller can actually persist.
                if binding.source_kind != "parameter"
                    && binding.source_kind != "local"
                    && binding.source_kind != "implicit-rec"
                {
                    continue;
                }

                // For parameter sources, require the caller-side parameter to be var.
                if binding.source_kind == "parameter"
                    && !binding.caller_source_parameter_is_var.unwrap_or(false)
                {
                    continue;
                }

                let source_name_lc = match &binding.source_variable_name {
                    Some(n) => n.clone(),
                    None => continue,
                };

                candidates_considered += 1;

                // Did caller persist the source variable after the callsite?
                let persisted_after = caller.record_operations.iter().any(|op| {
                    PERSIST_OPS.contains(&op.op.as_str())
                        && op.record_variable_name.to_lowercase() == source_name_lc
                        && before_anchor(&cs.source_anchor, &op.source_anchor)
                });
                if persisted_after {
                    skipped_caller_persists += 1;
                    continue; // callerPersists
                }

                // Emit.
                let path = vec![
                    EvidenceStep {
                        routine_id: caller.id.clone(),
                        operation_id: None,
                        callsite_id: Some(cs.id.clone()),
                        loop_id: None,
                        source_anchor: anchor_of(&binding.argument_anchor, caller),
                        note: format!(
                            "forwards {} to {}; never persists after the call",
                            binding.source_variable_name.as_deref().unwrap_or(""),
                            callee.name
                        ),
                    },
                    EvidenceStep {
                        routine_id: callee.id.clone(),
                        operation_id: None,
                        callsite_id: None,
                        loop_id: None,
                        source_anchor: anchor_of(&callee.source_anchor, callee),
                        note: format!(
                            "{} validates and exits dirty on at least one path",
                            callee.name
                        ),
                    },
                ];

                let id = format!("d39/{}/{}/{}", caller.id, cs.id, role.parameter_index);
                let mut affected_objects = vec![caller.object_id.clone(), callee.object_id.clone()];
                affected_objects.sort();

                let mut finding = Finding {
                    id: id.clone(),
                    root_cause_key: id,
                    detector: DETECTOR.to_string(),
                    title: "Record left dirty across helper chain".to_string(),
                    root_cause: format!(
                        "{} forwards {} to {}, which leaves the record in a Validate-dirty state on at least one exit path. {} never persists after the call — the field write is silently discarded.",
                        caller.name,
                        binding.source_variable_name.as_deref().unwrap_or(""),
                        callee.name,
                        caller.name
                    ),
                    severity: "medium".to_string(),
                    confidence: to_confidence(&[], "likely"),
                    primary_location: anchor_of(&binding.argument_anchor, caller),
                    evidence_path: path,
                    additional_paths: None,
                    affected_objects,
                    affected_tables: Vec::new(),
                    fix_options: vec![FixOption {
                        description: format!(
                            "Add {}.Modify() in {} after the call to {}, or have {} persist before returning.",
                            binding.source_variable_name.as_deref().unwrap_or(""),
                            caller.name,
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
    stats.add_skip("callerPersists", skipped_caller_persists);
    DetectorOutput::no_diag(findings, stats)
}
