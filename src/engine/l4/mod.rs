//! L4 — per-routine effect summaries over the call graph's SCC condensation.
//!
//! R3a-1 (the FIRST L4 sub-gate) ports the GRAPH substrate:
//!   - `combined_graph` — `build_combined_graph` (the resolved call graph + event
//!     graph → `CombinedEdge`s / `UncertaintyEdge`s / typed `GraphEdge`s) + the
//!     R3a-1 stable projection (`project_r3a1`).
//!   - `scc` — `tarjan_scc` (ITERATIVE Tarjan, reverse-topological output,
//!     deterministic member sort, `recursive` flag).
//!
//! The fixed-point summary core (R3a-2+) layers on this SCC condensation.

pub mod cfg_walker;
pub mod combined_graph;
pub mod effect_lattice;
pub mod scc;
pub mod summary;
pub mod summary_runner;
