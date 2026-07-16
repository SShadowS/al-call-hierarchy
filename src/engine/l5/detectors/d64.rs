//! D64 — API page write-surface exposure (OPT-IN). BCQuality
//! `disable-write-operations-on-read-only-api-pages`: an API page that is
//! Editable=false but leaves Insert/Modify/DeleteAllowed unset still exposes
//! OData writes (shape A, low). An API page declaring NO write-surface
//! property at all ships the default-open surface silently (shape B, info).
//!
//! OUT OF SCOPE (recorded here deliberately): the ReadIsolation :=
//! ReadCommitted body signal from `expose-only-committed-data-from-api-reads`
//! — member-property assignments are not captured by the L2 walk; revisit if
//! identifier_references ever carry member writes.
//!
//! Object-level findings: the page may have NO routines, so the evidence
//! step's routine_id carries the OBJECT id and the anchor is the object's own
//! decl anchor (Task-2 `L3Object.source_anchor`), falling back to a 1:1 anchor
//! in the object's first source unit when absent.
//!
//! `enclosing_routine_id`/`routine_id` carrying an OBJECT id (rather than a
//! routine id) is a convention new to this detector — the first
//! object-anchored finding in the engine. Two downstream consumers assumed a
//! routine-anchored `enclosing_routine_id` and were extended (behavior-
//! preserving for every other detector, whose findings are always routine-
//! anchored and hit the routine branch first) to also recognise a direct
//! object-id match: `FingerprintIndex::fingerprint_of`
//! (`src/engine/l5/fingerprint.rs`) — so the fingerprint's object-type/number
//! component still resolves instead of going empty — and the gate's
//! `to_location` (`src/engine/gate/projection.rs`) — so the production
//! SARIF/JSON/HTML/terminal output still shows the owning object's id/name
//! instead of a blank location.

use crate::engine::l3::l3_workspace::L3Resolved;
use crate::engine::l5::confidence::to_confidence;
use crate::engine::l5::detector_context::DetectorContext;
use crate::engine::l5::finding::{
    Evidence, EvidenceStep, Finding, FindingConfidence, FixOption, SourceAnchor,
};
use crate::engine::l5::fingerprint::FingerprintIndex;
use crate::engine::l5::registry::{DetectorError, DetectorOutput, DetectorStats};

const DETECTOR: &str = "d64-api-page-write-surface";

fn object_anchor(o: &crate::engine::l3::l3_workspace::L3Object) -> SourceAnchor {
    match &o.source_anchor {
        Some(a) => SourceAnchor {
            source_unit_id: a.source_unit_id.clone(),
            start_line: a.start_line,
            start_column: a.start_column,
            end_line: a.end_line,
            end_column: a.end_column,
            enclosing_routine_id: o.id.clone(), // object-level: object id by convention
            syntax_kind: a.syntax_kind.clone(),
            normalized_text_hash: None,
            leading_context_hash: None,
            trailing_context_hash: None,
        },
        None => SourceAnchor {
            source_unit_id: String::new(),
            start_line: 1,
            start_column: 1,
            end_line: 1,
            end_column: 1,
            enclosing_routine_id: o.id.clone(),
            syntax_kind: "object".to_string(),
            normalized_text_hash: None,
            leading_context_hash: None,
            trailing_context_hash: None,
        },
    }
}

pub fn detect_d64(
    resolved: &L3Resolved,
    _ctx: &DetectorContext,
) -> Result<DetectorOutput, DetectorError> {
    let ws = &resolved.workspace;
    let fp_index = FingerprintIndex::build(&ws.routines, &ws.objects);
    let mut findings: Vec<Finding> = Vec::new();
    let mut candidates_considered = 0usize;
    let mut skipped_declared_closed = 0u64;

    for o in &ws.objects {
        if o.object_type != "Page" {
            continue;
        }
        if !o
            .page_type
            .as_deref()
            .is_some_and(|p| p.eq_ignore_ascii_case("api"))
        {
            continue;
        }
        candidates_considered += 1;

        let writes_closed = o.insert_allowed == Some(false)
            && o.modify_allowed == Some(false)
            && o.delete_allowed == Some(false);
        let nothing_declared = o.editable.is_none()
            && o.insert_allowed.is_none()
            && o.modify_allowed.is_none()
            && o.delete_allowed.is_none();

        let (severity, title, root_cause) = if o.editable == Some(false) && !writes_closed {
            let mut missing: Vec<&str> = Vec::new();
            if o.insert_allowed != Some(false) {
                missing.push("InsertAllowed");
            }
            if o.modify_allowed != Some(false) {
                missing.push("ModifyAllowed");
            }
            if o.delete_allowed != Some(false) {
                missing.push("DeleteAllowed");
            }
            (
                "low",
                "Read-only API page leaves write operations enabled".to_string(),
                format!(
                    "API page {} declares Editable = false but does not disable {} — the \
                     OData surface still accepts those writes.",
                    o.name,
                    missing.join("/")
                ),
            )
        } else if nothing_declared {
            (
                "info",
                "API page write surface not declared".to_string(),
                format!(
                    "API page {} declares none of Editable/InsertAllowed/ModifyAllowed/\
                     DeleteAllowed — the default-open write surface ships silently; \
                     declare the intent explicitly.",
                    o.name
                ),
            )
        } else {
            skipped_declared_closed += 1;
            continue;
        };

        let anchor = object_anchor(o);
        let confidence: FindingConfidence = to_confidence(&[], "possible");
        let id = format!("d64/{}", o.id);
        let mut finding = Finding {
            id: id.clone(),
            root_cause_key: id,
            detector: DETECTOR.to_string(),
            title,
            root_cause,
            severity: severity.to_string(),
            confidence,
            primary_location: anchor.clone(),
            evidence_path: vec![EvidenceStep {
                routine_id: o.id.clone(), // object-level finding (see module doc)
                operation_id: None,
                callsite_id: None,
                loop_id: None,
                source_anchor: anchor,
                note: format!("API page {}", o.name),
            }],
            additional_paths: None,
            affected_objects: vec![o.id.clone()],
            affected_tables: Vec::new(),
            fix_options: vec![FixOption {
                description: "Declare the write surface explicitly: set InsertAllowed/\
                              ModifyAllowed/DeleteAllowed = false on read-only API pages \
                              (and Editable = false), or document the writable intent."
                    .to_string(),
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

    findings.sort_by(|a, b| a.id.cmp(&b.id));
    let emitted = findings.len();
    let mut stats = DetectorStats::new(DETECTOR, candidates_considered, emitted);
    stats.add_skip("declaredClosed", skipped_declared_closed);
    Ok(DetectorOutput::no_diag(findings, stats))
}
