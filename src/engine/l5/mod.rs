//! L5 QUERY SUBSTRATE (R4-0 Task 2a) — the pure-function + struct-def + native
//! oracle layer the L5 detectors (Task 2b) run over. Byte-parity migration of
//! al-sem's L5 shared substrate.
//!
//! This is the QUERY SUBSTRATE only — NOT the detector harness. It ports:
//!   - `full_summary` — the `FullRoutineSummary` composite the query helpers
//!     read (al-sem carries facts + coverage together on `routine.summary`; the
//!     Rust CORE `RoutineSummary` does not, so this struct re-unifies them).
//!   - `reverse_call_graph` — al-sem `src/engine/reverse-call-graph.ts`.
//!   - `entry_points` — al-sem `src/engine/entry-points.ts`.
//!   - `capability_query` — al-sem `src/detectors/capability-query.ts`.
//!   - `transaction_spans` — al-sem `src/engine/transaction-spans.ts`.
//!
//! NONE of: the `Finding` type, the detector registry, `detector_context`,
//! `path_walker`, the pipeline entry, or aldump wiring — those are Task 2b.
//!
//! Determinism: every output collection that flows to a fingerprint or a dump is
//! a sorted `Vec` / `BTreeSet`. No `HashMap` iteration order reaches output.

pub mod capability_query;
pub mod entry_points;
pub mod full_summary;
pub mod reverse_call_graph;
pub mod transaction_spans;

// R4-0 Task 2b — the L5 HARNESS (Finding model + stable projection, fingerprint,
// confidence, detector registry, detector context, path walker) and the ported
// detectors (currently d4).
pub mod confidence;
pub mod detector_context;
pub mod detectors;
pub mod finding;
pub mod fingerprint;
pub mod path_walker;
pub mod registry;

#[cfg(test)]
mod test_support;
