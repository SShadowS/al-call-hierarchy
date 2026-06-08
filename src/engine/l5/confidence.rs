//! `toConfidence` — port of al-sem `src/detectors/confidence.ts`.
//!
//! Maps a list of uncertainties to a [`FindingConfidence`]. Any uncertainty caps
//! `level` at `possible`. Uncertainty kinds that are valid `cappedBy` values
//! (directly or via the alias map) are listed in `cappedBy`; the others still cap
//! the level but appear only in `evidence`. `base_level` is never raised.
//!
//! For the R4-A wave (d4) the only call is `to_confidence(&[], "likely")` →
//! `{ level: "likely", evidence: [] }`. The full uncertainty-mapping path is
//! ported for fidelity even though no ported detector exercises it yet.

use crate::engine::l5::finding::{Evidence, FindingConfidence};

/// The `Uncertainty` kinds that are also valid `cappedBy` values
/// (`VALID_CAPPED_BY` in confidence.ts).
const VALID_CAPPED_BY: &[&str] = &[
    "unresolved-call",
    "opaque-callee",
    "dynamic-dispatch",
    "parse-incomplete",
    "version-mismatch",
];

/// A minimal Uncertainty for the confidence mapper. al-sem's `Uncertainty` is a
/// discriminated union carrying a `kind` plus one id field (callsiteId /
/// operationId / routineId). The mapper only reads `kind` (for cappedBy) and the
/// id (for the evidence note) — this carries exactly that subset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UncertaintyLite {
    pub kind: String,
    /// The descriptive id (callsiteId | operationId | routineId) — used only for
    /// the evidence note (`"<kind> at <id>"`).
    pub at: String,
}

/// Map a new resolver-upgrade uncertainty kind onto an existing `cappedBy` value
/// (`UNCERTAINTY_TO_CAPPED_BY` alias map in confidence.ts).
fn alias_capped_by(kind: &str) -> Option<&'static str> {
    match kind {
        "ambiguous-overload" => Some("unresolved-call"),
        "member-not-found" => Some("unresolved-call"),
        "external-target" => Some("opaque-callee"),
        "interface-open-world" => Some("dynamic-dispatch"),
        _ => None,
    }
}

/// `toCappedByKind` — alias mapping first, then identity against
/// `VALID_CAPPED_BY`.
fn to_capped_by_kind(kind: &str) -> Option<String> {
    if let Some(alias) = alias_capped_by(kind) {
        return Some(alias.to_string());
    }
    if VALID_CAPPED_BY.contains(&kind) {
        return Some(kind.to_string());
    }
    None
}

/// Port of `toConfidence`. Empty `uncertainties` ⇒ `{ level: base_level,
/// evidence: [] }`. Otherwise level is capped at `possible`, `cappedBy` carries
/// the sorted valid-mapped kinds (absent when none mapped), and `evidence`
/// carries `"<kind> at <id>"` notes in input order.
pub fn to_confidence(uncertainties: &[UncertaintyLite], base_level: &str) -> FindingConfidence {
    if uncertainties.is_empty() {
        return FindingConfidence {
            level: base_level.to_string(),
            capped_by: None,
            evidence: Vec::new(),
        };
    }

    // Sorted, de-duped capped-by set (al-sem `new Set(...).sort()`).
    let mut capped_by_set: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for u in uncertainties {
        if let Some(mapped) = to_capped_by_kind(&u.kind) {
            capped_by_set.insert(mapped);
        }
    }
    let capped_by = if capped_by_set.is_empty() {
        None
    } else {
        Some(capped_by_set.into_iter().collect::<Vec<_>>())
    };

    let evidence = uncertainties
        .iter()
        .map(|u| Evidence {
            source: "tree-sitter".to_string(),
            note: Some(format!("{} at {}", u.kind, u.at)),
        })
        .collect();

    FindingConfidence {
        level: "possible".to_string(),
        capped_by,
        evidence,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_uncertainties_keeps_base_level_and_empty_evidence() {
        let c = to_confidence(&[], "likely");
        assert_eq!(c.level, "likely");
        assert!(c.capped_by.is_none());
        assert!(c.evidence.is_empty());
    }

    #[test]
    fn any_uncertainty_caps_to_possible() {
        let c = to_confidence(
            &[UncertaintyLite {
                kind: "interface-dispatch".to_string(),
                at: "r/cs0".to_string(),
            }],
            "likely",
        );
        assert_eq!(c.level, "possible");
        // interface-dispatch is NOT a valid cappedBy → only evidence.
        assert!(c.capped_by.is_none());
        assert_eq!(c.evidence.len(), 1);
        assert_eq!(
            c.evidence[0].note.as_deref(),
            Some("interface-dispatch at r/cs0")
        );
    }

    #[test]
    fn alias_and_identity_capped_by_sorted_unique() {
        let c = to_confidence(
            &[
                UncertaintyLite {
                    kind: "external-target".to_string(),
                    at: "r/cs1".to_string(),
                },
                UncertaintyLite {
                    kind: "parse-incomplete".to_string(),
                    at: "r".to_string(),
                },
                UncertaintyLite {
                    kind: "external-target".to_string(),
                    at: "r/cs2".to_string(),
                },
            ],
            "confirmed",
        );
        assert_eq!(c.level, "possible");
        assert_eq!(
            c.capped_by,
            Some(vec![
                "opaque-callee".to_string(),
                "parse-incomplete".to_string()
            ])
        );
        assert_eq!(c.evidence.len(), 3);
    }
}
