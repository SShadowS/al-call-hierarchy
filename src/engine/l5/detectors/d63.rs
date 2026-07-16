//! D63 — HTML built by string concatenation (OPT-IN heuristic). BCQuality
//! `al-has-no-built-in-htmlencode`: AL has no HtmlEncode; splicing data into
//! HTML literals is an injection (XSS-shaped) risk wherever the string reaches
//! a browser/mail surface. Pure TEXT heuristic over call-site argument source
//! text — one finding per call site (first matching argument), OPT-IN because
//! the engine cannot see where the string ends up.
//!
//! Severity: low. Confidence: possible.

use crate::engine::l3::l3_workspace::L3Resolved;
use crate::engine::l5::confidence::to_confidence;
use crate::engine::l5::detector_context::DetectorContext;
use crate::engine::l5::detectors::anchor_of;
use crate::engine::l5::finding::{Evidence, EvidenceStep, Finding, FindingConfidence, FixOption};
use crate::engine::l5::fingerprint::FingerprintIndex;
use crate::engine::l5::registry::{DetectorError, DetectorOutput, DetectorStats};

const DETECTOR: &str = "d63-html-concat-injection";

/// Does the literal contain an HTML-tag-ish `<x` / `</x` sequence?
fn html_tagish(lit: &str) -> bool {
    let b = lit.as_bytes();
    b.windows(2)
        .any(|w| w[0] == b'<' && (w[1].is_ascii_alphabetic() || w[1] == b'/'))
}

/// Argument-text heuristic: at least one single-quoted AL literal containing an
/// HTML-tag-ish sequence AND at least one `+` OUTSIDE the literals (a real
/// concatenation). AL escapes `'` inside literals as `''`.
fn looks_like_html_concat(text: &str) -> bool {
    let mut in_lit = false;
    let mut lit = String::new();
    let mut html_lit = false;
    let mut concat_outside = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if in_lit {
            if c == '\'' {
                if chars.peek() == Some(&'\'') {
                    chars.next();
                    lit.push('\'');
                } else {
                    in_lit = false;
                    if html_tagish(&lit) {
                        html_lit = true;
                    }
                    lit.clear();
                }
            } else {
                lit.push(c);
            }
        } else if c == '\'' {
            in_lit = true;
        } else if c == '+' {
            concat_outside = true;
        }
    }
    html_lit && concat_outside
}

pub fn detect_d63(
    resolved: &L3Resolved,
    _ctx: &DetectorContext,
) -> Result<DetectorOutput, DetectorError> {
    let ws = &resolved.workspace;
    let fp_index = FingerprintIndex::build(&ws.routines, &ws.objects);
    let mut findings: Vec<Finding> = Vec::new();
    let mut candidates_considered = 0usize;

    for routine in &ws.routines {
        if !routine.body_available || routine.parse_incomplete {
            continue;
        }
        for cs in &routine.call_sites {
            let Some(arg) = cs.argument_texts.iter().find(|t| looks_like_html_concat(t)) else {
                continue;
            };
            candidates_considered += 1;

            let confidence: FindingConfidence = to_confidence(&[], "possible");
            let id = format!("d63/{}/{}", routine.id, cs.id);
            let mut finding = Finding {
                id: id.clone(),
                root_cause_key: id,
                detector: DETECTOR.to_string(),
                title: "HTML built by string concatenation".to_string(),
                root_cause: format!(
                    "{} concatenates data into an HTML literal ({}) — AL has no built-in \
                     HtmlEncode, so any user-influenced value is an injection risk where \
                     this string reaches a browser or mail body.",
                    routine.name,
                    arg.chars().take(60).collect::<String>()
                ),
                severity: "low".to_string(),
                confidence,
                primary_location: anchor_of(&cs.source_anchor, routine),
                evidence_path: vec![EvidenceStep {
                    routine_id: routine.id.clone(),
                    operation_id: None,
                    callsite_id: Some(cs.id.clone()),
                    loop_id: None,
                    source_anchor: anchor_of(&cs.source_anchor, routine),
                    note: "HTML literal + concatenation in argument".to_string(),
                }],
                additional_paths: None,
                affected_objects: vec![routine.object_id.clone()],
                affected_tables: Vec::new(),
                fix_options: vec![FixOption {
                    description: "Encode interpolated values (replace <, >, &, \" before \
                                  splicing) or build the document with an XmlDocument/\
                                  template API instead of concatenation."
                        .to_string(),
                    safety: "medium".to_string(),
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

    findings.sort_by(|a, b| a.id.cmp(&b.id));
    let emitted = findings.len();
    let stats = DetectorStats::new(DETECTOR, candidates_considered, emitted);
    Ok(DetectorOutput::no_diag(findings, stats))
}

#[cfg(test)]
mod tests {
    use super::{html_tagish, looks_like_html_concat};

    #[test]
    fn html_literal_plus_concat_flags() {
        assert!(looks_like_html_concat("'<b>' + UserName + '</b>'"));
        assert!(looks_like_html_concat("'<div class=x>' + V"));
        assert!(looks_like_html_concat("Body + '</table>'"));
    }

    #[test]
    fn plain_literals_and_math_do_not_flag() {
        assert!(!looks_like_html_concat("'<b>static</b>'")); // no concat
        assert!(!looks_like_html_concat("'a' + 'b'")); // concat, no HTML tag... see note
        assert!(!looks_like_html_concat("X + Y")); // no literal
        assert!(!looks_like_html_concat("'2 < 3 and 4 > 1' + V")); // `< ` not tag-ish
    }

    #[test]
    fn escaped_quotes_inside_literals_handled() {
        assert!(!looks_like_html_concat("'it''s fine' + V"));
        assert!(looks_like_html_concat("'it''s <b>' + V"));
    }

    #[test]
    fn tagish_needs_letter_or_slash_after_lt() {
        assert!(html_tagish("<b>"));
        assert!(html_tagish("</td>"));
        assert!(!html_tagish("2 < 3"));
        assert!(!html_tagish("no tags"));
    }
}
