//! D8 — Commit inside a posting transaction span. Port of al-sem
//! `src/detectors/d8-commit-in-transaction.ts`.
//!
//! For each ExplicitCommit transaction span, if the span includes a
//! transaction-managing routine (name matches `^(Post|Apply|Release)[A-Z]` OR writes
//! ≥3 tables) AND the Commit is in a DIFFERENT routine, emit a high-severity finding.
//!
//! Dedup: same commit_operation_id → keep first, skip subsequent.
//! Within-detector sort by `a.id.cmp(&b.id)` (byte order).

use std::collections::HashSet;

use crate::engine::l3::l3_workspace::L3Resolved;
use crate::engine::l5::capability_query::writes_physical_tables_of;
use crate::engine::l5::confidence::to_confidence;
use crate::engine::l5::detector_context::DetectorContext;
use crate::engine::l5::detectors::anchor_of;
use crate::engine::l5::finding::{Evidence, EvidenceStep, Finding, FindingConfidence, FixOption};
use crate::engine::l5::fingerprint::FingerprintIndex;
use crate::engine::l5::registry::{DetectorOutput, DetectorStats};
use crate::engine::l5::transaction_spans::SeedKind;

const DETECTOR: &str = "d8-commit-in-transaction";
const TRANSACTION_THRESHOLD_TABLES: usize = 3;

/// `isTransactionManaging` — name matches `^(Post|Apply|Release)[A-Z]` OR writes ≥3
/// tables. Mirrors the al-sem regex `/^(Post|Apply|Release)[A-Z]/`.
fn is_transaction_managing(routine_id: &str, ctx: &DetectorContext) -> bool {
    let Some(r) = ctx.routine_by_id.get(routine_id) else {
        return false;
    };
    if posting_name_matches(&r.name) {
        return true;
    }
    let Some(summary) = ctx.summaries.get(routine_id) else {
        return false;
    };
    writes_physical_tables_of(summary).len() >= TRANSACTION_THRESHOLD_TABLES
}

/// Hand-rolled `^(Post|Apply|Release)[A-Z]` check: the name must start with
/// "Post", "Apply", or "Release", followed immediately by an uppercase ASCII letter.
fn posting_name_matches(name: &str) -> bool {
    for prefix in &["Post", "Apply", "Release"] {
        if let Some(rest) = name.strip_prefix(prefix)
            && let Some(next) = rest.chars().next()
            && next.is_ascii_uppercase()
        {
            return true;
        }
    }
    false
}

pub fn detect_d8(resolved: &L3Resolved, ctx: &DetectorContext) -> DetectorOutput {
    let ws = &resolved.workspace;
    let fp_index = FingerprintIndex::build(&ws.routines, &ws.objects);
    let mut findings: Vec<Finding> = Vec::new();
    let mut candidates_considered = 0usize;
    let mut skipped_other = 0u64;

    for span in &ctx.transaction_spans {
        // §B: checked-run-implicit seeds are for D50 only — D8 ignores them.
        if span.seed_kind != SeedKind::ExplicitCommit {
            continue;
        }
        let Some(commit_routine) = ctx.routine_by_id.get(span.commit_routine_id.as_str()) else {
            continue;
        };
        // roleOf(routine) === "primary": source-only ⇒ every routine is primary.
        candidates_considered += 1;

        // managers = routinesInSpan minus commitRoutineId, filtered to managing.
        let managers: Vec<&str> = span
            .routines_in_span
            .iter()
            .filter(|id| id.as_str() != span.commit_routine_id.as_str())
            .filter(|id| is_transaction_managing(id, ctx))
            .map(|id| id.as_str())
            .collect();
        if managers.is_empty() {
            skipped_other += 1;
            continue;
        }

        let manager_id = managers[0];
        let Some(manager) = ctx.routine_by_id.get(manager_id) else {
            continue;
        };

        // Evidence path: manager step + commit step.
        let manager_anchor = anchor_of(&manager.source_anchor, manager);
        let commit_op_site = commit_routine
            .operation_sites
            .iter()
            .find(|os| os.id == span.commit_operation_id);
        let commit_anchor = match commit_op_site {
            Some(os) => anchor_of(&os.source_anchor, commit_routine),
            None => anchor_of(&commit_routine.source_anchor, commit_routine),
        };

        let path = vec![
            EvidenceStep {
                routine_id: manager.id.clone(),
                operation_id: None,
                callsite_id: None,
                loop_id: None,
                source_anchor: manager_anchor,
                note: format!("transaction-managing routine: {}", manager.name),
            },
            EvidenceStep {
                routine_id: commit_routine.id.clone(),
                operation_id: Some(span.commit_operation_id.clone()),
                callsite_id: None,
                loop_id: None,
                source_anchor: commit_anchor.clone(),
                note: format!("Commit inside {}'s transaction span", manager.name),
            },
        ];

        let write_count = ctx
            .summaries
            .get(manager_id)
            .map(|s| writes_physical_tables_of(s).len())
            .unwrap_or(0);

        // affectedObjects: [commitRoutine.objectId, manager.objectId].sort()
        let mut affected_objects =
            vec![commit_routine.object_id.clone(), manager.object_id.clone()];
        affected_objects.sort();

        let confidence: FindingConfidence = to_confidence(&[], "likely");
        let id = format!("d8/{}", span.commit_operation_id);
        let root_cause_key = id.clone();

        let mut finding = Finding {
            id,
            root_cause_key,
            detector: DETECTOR.to_string(),
            title: "Commit inside a posting transaction span".to_string(),
            root_cause: format!(
                "{} calls Commit while reachable from {}, which writes {} tables. \
                 A mid-transaction Commit breaks rollback semantics \u{2014} if the surrounding \
                 operation later fails, the data is left half-written.",
                commit_routine.name, manager.name, write_count
            ),
            severity: "high".to_string(),
            confidence,
            primary_location: commit_anchor,
            evidence_path: path,
            additional_paths: None,
            affected_objects,
            affected_tables: span.writes_tables.clone(),
            fix_options: vec![FixOption {
                description: "Remove the Commit, or restructure so the surrounding transaction \
                              completes (returns control to its caller) before this code runs."
                    .to_string(),
                safety: "low".to_string(),
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

    // Dedup by id (same commit operation flagged from multiple manager routines → one finding).
    let mut seen: HashSet<String> = HashSet::new();
    let mut deduped: Vec<Finding> = Vec::new();
    for f in findings {
        if seen.contains(&f.id) {
            continue;
        }
        seen.insert(f.id.clone());
        deduped.push(f);
    }

    deduped.sort_by(|a, b| a.id.cmp(&b.id));

    let emitted = deduped.len();
    let mut stats = DetectorStats::new(DETECTOR, candidates_considered, emitted);
    stats.add_skip("other", skipped_other);
    DetectorOutput {
        findings: deduped,
        stats,
        diagnostics: vec![],
    }
}
