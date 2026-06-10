//! D44 — event multi-subscriber overlap. Port of al-sem
//! `src/detectors/d44-event-multi-subscriber-overlap.ts`.
//!
//! Two findings families over an event's resolved subscribers:
//!   - WRITE/WRITE: ≥2 distinct subscribers write the SAME table (id
//!     `d44/{eventId}|{tableId}`, severity medium).
//!   - READ-AFTER-WRITE: one subscriber writes a table that a DIFFERENT subscriber
//!     reads on the same event (id `d44-rw/{eventId}|{tableId}`, severity low).
//!
//! Both set `event_kind` (via `event_kind_of`) and `cross_extension_subscribers`
//! (via `build_cross_extension_subscribers`). Output is capped per-event
//! (`D44_MAX_PER_EVENT = 32`) across BOTH families via `group_and_cap`, then sorted
//! by `compareStrings(id)`. Fingerprint computed PER-FINDING (al-sem computes it in
//! the build loop, BEFORE the cap).

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::engine::l3::l3_workspace::L3Resolved;
use crate::engine::l5::capability_query::find_capabilities;
use crate::engine::l5::detector_context::DetectorContext;
use crate::engine::l5::event_flow::{build_cross_extension_subscribers, event_kind_of};
use crate::engine::l5::finding::{
    Evidence, EvidenceStep, Finding, FindingConfidence, FixOption, SourceAnchor,
};
use crate::engine::l5::fingerprint::FingerprintIndex;
use crate::engine::l5::registry::{DetectorOutput, DetectorStats};

use super::{anchor_of, group_and_cap};

const DETECTOR: &str = "d44-event-multi-subscriber-overlap";
const D44_MAX_PER_EVENT: usize = 32;

/// `WRITE_OPS = {insert, modify, delete}`.
fn is_write_op(op: &str) -> bool {
    matches!(op, "insert" | "modify" | "delete")
}

struct SubWrite {
    subscriber: String,
    op: String,
}

pub fn detect_d44(resolved: &L3Resolved, ctx: &DetectorContext) -> DetectorOutput {
    let ws = &resolved.workspace;
    let fp_index = FingerprintIndex::build(&ws.routines, &ws.objects);
    let ix = &ctx.event_flow_indexes;

    let mut findings: Vec<Finding> = Vec::new();
    let mut candidates = 0usize;

    // eventKind per internal eventId.
    let mut event_kind_by_id: HashMap<&str, &'static str> = HashMap::new();
    for ev in &ctx.event_graph.events {
        event_kind_by_id.insert(ev.id.as_str(), event_kind_of(&ev.event_kind));
    }
    let cross_ext_by_event = build_cross_extension_subscribers(&ctx.event_graph, &ws.objects);

    // anchor lookup helper.
    let anchor_for = |routine_id: &str, fallback: &SourceAnchor| -> SourceAnchor {
        match ctx.routine_by_id.get(routine_id) {
            Some(r) => anchor_of(&r.source_anchor, r),
            None => fallback.clone(),
        }
    };

    // --- WRITE/WRITE: (event, table) → subscriber writes ---------------------
    // key = `${eventId}|${tableId}`. BTreeMap → sorted-key iteration (deterministic;
    // the final id sort makes insertion order irrelevant either way).
    let mut grouped: BTreeMap<String, Vec<SubWrite>> = BTreeMap::new();
    for (event_id, subs) in &ix.subscribers_by_event {
        for sub in subs {
            let Some(r) = ctx.routine_by_id.get(sub.as_str()).copied() else {
                continue;
            };
            let Some(summary) = ctx.summaries.get(&r.id) else {
                continue;
            };
            let writes = find_capabilities(summary, |f| {
                f.resource_kind == "table" && is_write_op(&f.op) && f.resource_id.is_some()
            });
            for w in writes {
                let key = format!("{event_id}|{}", w.resource_id.as_deref().unwrap());
                grouped.entry(key).or_default().push(SubWrite {
                    subscriber: sub.clone(),
                    op: w.op.clone(),
                });
            }
        }
    }

    for (key, writes) in &grouped {
        let unique_subs: BTreeSet<&str> = writes.iter().map(|w| w.subscriber.as_str()).collect();
        if unique_subs.len() < 2 {
            continue;
        }
        candidates += 1;
        let (event_id, table_id) = split_once_pipe(key);
        let sub_list: Vec<&str> = unique_subs.iter().copied().collect();
        let op_union: BTreeSet<&str> = writes.iter().map(|w| w.op.as_str()).collect();
        let op_union: Vec<&str> = op_union.into_iter().collect();
        let Some(first_id) = sub_list.first().copied() else {
            continue;
        };
        let Some(first) = ctx.routine_by_id.get(first_id).copied() else {
            continue;
        };
        let first_anchor = anchor_of(&first.source_anchor, first);

        let root_cause_key = format!("d44/{event_id}|{table_id}");
        let evidence: Vec<EvidenceStep> = sub_list
            .iter()
            .map(|sub| EvidenceStep {
                routine_id: (*sub).to_string(),
                operation_id: None,
                callsite_id: None,
                loop_id: None,
                source_anchor: anchor_for(sub, &first_anchor),
                note: format!("writes table {table_id}"),
            })
            .collect();
        let cross_ext = cross_ext_by_event
            .get(event_id)
            .filter(|v| !v.is_empty())
            .cloned();

        let mut finding = Finding {
            id: root_cause_key.clone(),
            root_cause_key: root_cause_key.clone(),
            detector: DETECTOR.to_string(),
            title: "Multiple event subscribers write the same table".to_string(),
            root_cause: format!(
                "{} subscribers of event {event_id} write table {table_id} (ops: {})",
                sub_list.len(),
                op_union.join(", ")
            ),
            severity: "medium".to_string(),
            confidence: FindingConfidence {
                level: "likely".to_string(),
                capped_by: None,
                evidence: Vec::new(),
            },
            primary_location: first_anchor.clone(),
            evidence_path: evidence,
            additional_paths: None,
            affected_objects: Vec::new(),
            affected_tables: vec![table_id.to_string()],
            fix_options: vec![FixOption {
                description: "Coordinate the writes (single subscriber, or merge intent) to avoid lost-update / ordering surprises.".to_string(),
                safety: "medium".to_string(),
            }],
            provenance: vec![Evidence {
                source: "tree-sitter".to_string(),
                note: None,
            }],
            actionable_anchor: None,
            fingerprint: None,
            event_kind: event_kind_by_id.get(event_id).map(|s| s.to_string()),
            cross_extension_subscribers: cross_ext,
        };
        finding.fingerprint = Some(fp_index.fingerprint_of(&finding));
        findings.push(finding);
    }

    // --- READ-AFTER-WRITE ----------------------------------------------------
    let mut writers_by_event_table: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut readers_by_event_table: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (event_id, subs) in &ix.subscribers_by_event {
        for sub in subs {
            let Some(r) = ctx.routine_by_id.get(sub.as_str()).copied() else {
                continue;
            };
            let Some(summary) = ctx.summaries.get(&r.id) else {
                continue;
            };
            let write_facts = find_capabilities(summary, |f| {
                f.resource_kind == "table" && is_write_op(&f.op) && f.resource_id.is_some()
            });
            for w in write_facts {
                let k = format!("{event_id}|{}", w.resource_id.as_deref().unwrap());
                writers_by_event_table
                    .entry(k)
                    .or_default()
                    .insert(sub.clone());
            }
            let read_facts = find_capabilities(summary, |f| {
                f.resource_kind == "table" && f.op == "read" && f.resource_id.is_some()
            });
            for rd in read_facts {
                let k = format!("{event_id}|{}", rd.resource_id.as_deref().unwrap());
                readers_by_event_table
                    .entry(k)
                    .or_default()
                    .insert(sub.clone());
            }
        }
    }

    let empty_set: BTreeSet<String> = BTreeSet::new();
    for (key, writers) in &writers_by_event_table {
        let readers = readers_by_event_table.get(key).unwrap_or(&empty_set);
        let distinct_readers: Vec<&str> = readers
            .iter()
            .filter(|rd| !writers.contains(rd.as_str()))
            .map(|s| s.as_str())
            .collect();
        if distinct_readers.is_empty() {
            continue;
        }
        let (event_id, table_id) = split_once_pipe(key);
        let writer_list: Vec<&str> = writers.iter().map(|s| s.as_str()).collect();
        let reader_list: Vec<&str> = distinct_readers; // already sorted (BTreeSet iter + filter)
        let Some(first_id) = writer_list.first().copied() else {
            continue;
        };
        let Some(first) = ctx.routine_by_id.get(first_id).copied() else {
            continue;
        };
        let first_anchor = anchor_of(&first.source_anchor, first);

        let root_cause_key = format!("d44-rw/{event_id}|{table_id}");
        let mut evidence: Vec<EvidenceStep> = Vec::new();
        for sub in &writer_list {
            evidence.push(EvidenceStep {
                routine_id: (*sub).to_string(),
                operation_id: None,
                callsite_id: None,
                loop_id: None,
                source_anchor: anchor_for(sub, &first_anchor),
                note: format!("writes {table_id}"),
            });
        }
        for sub in &reader_list {
            evidence.push(EvidenceStep {
                routine_id: (*sub).to_string(),
                operation_id: None,
                callsite_id: None,
                loop_id: None,
                source_anchor: anchor_for(sub, &first_anchor),
                note: format!("reads {table_id}"),
            });
        }
        let cross_ext = cross_ext_by_event
            .get(event_id)
            .filter(|v| !v.is_empty())
            .cloned();

        let mut finding = Finding {
            id: root_cause_key.clone(),
            root_cause_key: root_cause_key.clone(),
            detector: DETECTOR.to_string(),
            title: "Event subscriber reads a table that another subscriber writes".to_string(),
            root_cause: format!(
                "On event {event_id}, subscribers {{{}}} write {table_id}; subscribers {{{}}} read {table_id}. AL subscriber order is undefined — reads may see pre- or post-mutation state.",
                writer_list.join(", "),
                reader_list.join(", ")
            ),
            severity: "low".to_string(),
            confidence: FindingConfidence {
                level: "likely".to_string(),
                capped_by: None,
                evidence: Vec::new(),
            },
            primary_location: first_anchor.clone(),
            evidence_path: evidence,
            additional_paths: None,
            affected_objects: Vec::new(),
            affected_tables: vec![table_id.to_string()],
            fix_options: vec![FixOption {
                description: "Make subscriber ordering explicit, or move the read into the writing subscriber.".to_string(),
                safety: "medium".to_string(),
            }],
            provenance: vec![Evidence {
                source: "tree-sitter".to_string(),
                note: None,
            }],
            actionable_anchor: None,
            fingerprint: None,
            event_kind: event_kind_by_id.get(event_id).map(|s| s.to_string()),
            cross_extension_subscribers: cross_ext,
        };
        finding.fingerprint = Some(fp_index.fingerprint_of(&finding));
        findings.push(finding);
    }

    // Apply the per-event output cap across BOTH families.
    let (kept, _truncated) = group_and_cap(
        findings,
        |f| {
            // ^d44(?:-rw)?\/([^|]+)
            let rest = f
                .root_cause_key
                .strip_prefix("d44-rw/")
                .or_else(|| f.root_cause_key.strip_prefix("d44/"));
            match rest {
                Some(r) => r.split('|').next().unwrap_or(&f.root_cause_key).to_string(),
                None => f.root_cause_key.clone(),
            }
        },
        D44_MAX_PER_EVENT,
    );

    let mut kept = kept;
    kept.sort_by(|a, b| a.id.cmp(&b.id));
    let emitted = kept.len();
    DetectorOutput {
        findings: kept,
        stats: DetectorStats::new(DETECTOR, candidates, emitted),
        diagnostics: vec![],
    }
}

/// Split a `${a}|${b}` key into (a, b) at the FIRST pipe (al-sem `key.split("|", 2)`).
fn split_once_pipe(key: &str) -> (&str, &str) {
    match key.split_once('|') {
        Some((a, b)) => (a, b),
        None => (key, ""),
    }
}
