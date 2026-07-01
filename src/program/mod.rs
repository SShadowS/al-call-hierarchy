//! Whole-program semantic graph built from a parsed `AppSetSnapshot`
//! (charter §3). Plan 1B.1 = nodes + app-qualified identity + topology index.

pub mod abi_ingest;
pub mod build;
pub mod graph;
pub mod graphify_export;
pub mod integration_report;
pub mod l3_mint;
pub mod node;
pub mod node_extract;
pub mod resolve;
pub mod topology;

pub use build::build_program_graph;
pub use graph::{ObjectIndex, ProgramGraph};
pub use node::{AppRef, AppRegistry, ObjKey, ObjectNodeId, RoutineNodeId};
pub use node_extract::{Access, ObjectNode, RoutineNode, extract_nodes};
pub use topology::DependencyGraph;
