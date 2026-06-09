//! The ported L5 detectors. Each module ports one al-sem detector; the registered
//! list grows as each wave lands. Currently: d4 (R4-0), d5/d10/d11/d18/d21/d36 (R4-A),
//! d22/d33 (R4-B), d7/d12/d38 (R4-C), d8/d9/d34/d35 (R4-D), d32 (reverse-call-graph wave).

pub mod d10;
pub mod d11;
pub mod d12;
pub mod d18;
pub mod d19;
pub mod d20;
pub mod d21;
pub mod d22;
pub mod d29;
pub mod d32;
pub mod d33;
pub mod d34;
pub mod d35;
pub mod d36;
pub mod d38;
pub mod d4;
pub mod d5;
pub mod d7;
pub mod d8;
pub mod d9;

use crate::engine::l2::features::{PAnchor, PExpressionInfo};
use crate::engine::l3::l3_workspace::L3Routine;
use crate::engine::l5::finding::SourceAnchor;
use crate::engine::l5::registry::Detector;

/// `beforeAnchor(a, b)` — true iff `a` is strictly before `b` by source position.
/// Port of al-sem `src/engine/source-anchor.ts:beforeAnchor`.
/// Strict less-than: same position is NOT "before".
pub(crate) fn before_anchor(a: &PAnchor, b: &PAnchor) -> bool {
    if a.start_line != b.start_line {
        return a.start_line < b.start_line;
    }
    a.start_column < b.start_column
}

/// `unquotedFieldName` from `model/expression.ts`:
/// Resolve a field-name argument to its unquoted form. Prefers `.value` (set on
/// `quoted_identifier` / `string_literal` / `qualified_enum_value`) over `.text`.
/// Preserves original case — callers lowercase for comparison where needed.
/// Shared across d18 (CalcFields-arg literal check) and d22 (CalcFields-arg coverage).
pub(crate) fn unquoted_field_name(info: &PExpressionInfo) -> String {
    if let Some(v) = &info.value {
        return v.clone();
    }
    info.text.clone()
}

/// Build the internal `SourceAnchor` from an L2 `PAnchor` + the owning routine.
/// Drops `enclosingRoutineId` from the PAnchor (which doesn't carry one) and
/// stamps the routine's own id. Hash fields default to `None`.
pub(crate) fn anchor_of(a: &PAnchor, routine: &L3Routine) -> SourceAnchor {
    SourceAnchor {
        source_unit_id: a.source_unit_id.clone(),
        start_line: a.start_line,
        start_column: a.start_column,
        end_line: a.end_line,
        end_column: a.end_column,
        enclosing_routine_id: routine.id.clone(),
        syntax_kind: a.syntax_kind.clone(),
        normalized_text_hash: None,
        leading_context_hash: None,
        trailing_context_hash: None,
    }
}

/// The registered detector list. Re-sorted findings come out of `run_detectors`;
/// registration order does not affect output. Grows one detector per wave.
pub fn registered_detectors() -> Vec<Detector> {
    vec![
        Detector {
            name: "d4-repeated-lookup-in-loop".to_string(),
            run: d4::detect_d4,
        },
        Detector {
            name: "d5-set-based-opportunity".to_string(),
            run: d5::detect_d5,
        },
        Detector {
            name: "d7-recursive-event-expansion".to_string(),
            run: d7::detect_d7,
        },
        Detector {
            name: "d10-self-modifying-loop".to_string(),
            run: d10::detect_d10,
        },
        Detector {
            name: "d11-modify-without-get".to_string(),
            run: d11::detect_d11,
        },
        Detector {
            name: "d12-dead-integration-event".to_string(),
            run: d12::detect_d12,
        },
        Detector {
            name: "d18-constant-filter-in-loop".to_string(),
            run: d18::detect_d18,
        },
        Detector {
            name: "d19-unused-parameter".to_string(),
            run: d19::detect_d19,
        },
        Detector {
            name: "d20-unreachable-after-exit".to_string(),
            run: d20::detect_d20,
        },
        Detector {
            name: "d21-read-without-load".to_string(),
            run: d21::detect_d21,
        },
        Detector {
            name: "d22-flowfield-without-calcfields".to_string(),
            run: d22::detect_d22,
        },
        Detector {
            name: "d29-subscriber-modify-on-event-record".to_string(),
            run: d29::detect_d29,
        },
        Detector {
            name: "d32-constant-boolean-parameter".to_string(),
            run: d32::detect_d32,
        },
        Detector {
            name: "d33-unfiltered-bulk-write".to_string(),
            run: d33::detect_d33,
        },
        Detector {
            name: "d36-late-setloadfields".to_string(),
            run: d36::detect_d36,
        },
        Detector {
            name: "d38-subscriber-to-obsolete-event".to_string(),
            run: d38::detect_d38,
        },
        Detector {
            name: "d8-commit-in-transaction".to_string(),
            run: d8::detect_d8,
        },
        Detector {
            name: "d9-transaction-span-summary".to_string(),
            run: d9::detect_d9,
        },
        Detector {
            name: "d34-commit-in-loop".to_string(),
            run: d34::detect_d34,
        },
        Detector {
            name: "d35-commit-in-event-subscriber".to_string(),
            run: d35::detect_d35,
        },
    ]
}
