//! The ported L5 detectors. Each module ports one al-sem detector; the registered
//! list grows as each wave lands. Currently: d4 (R4-A intraprocedural).

pub mod d4;

use crate::engine::l5::registry::Detector;

/// The registered detector list. Re-sorted findings come out of `run_detectors`;
/// registration order does not affect output. Grows one detector per wave.
pub fn registered_detectors() -> Vec<Detector> {
    vec![Detector {
        name: "d4-repeated-lookup-in-loop".to_string(),
        run: d4::detect_d4,
    }]
}
