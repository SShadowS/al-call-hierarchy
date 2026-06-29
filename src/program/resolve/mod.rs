//! Plan 1B.2: fresh call/behaviour-edge resolver over `ProgramGraph`.
//! Phase 0 = edge model + dual-run differential harness (this module set).

pub mod differential;
pub mod edge;
pub mod extract_min;
pub mod stub;

pub use edge::{
    DispatchShape, Edge, EdgeKind, Evidence, Route, RouteTarget, SetCompleteness, Witness,
};
