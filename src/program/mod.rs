//! Whole-program semantic graph built from a parsed `AppSetSnapshot`
//! (charter §3). Plan 1B.1 = nodes + app-qualified identity + topology index.

pub mod node;
pub mod node_extract;
pub mod topology;

pub use node::{AppRef, AppRegistry, ObjKey, ObjectNodeId, RoutineNodeId};
pub use node_extract::{Access, ObjectNode, RoutineNode, extract_nodes};
pub use topology::DependencyGraph;
