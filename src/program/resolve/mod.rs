//! Plan 1B.2: fresh call/behaviour-edge resolver over `ProgramGraph`.
//! Phase 0 = edge model + dual-run differential harness (this module set).
//! Phase 1 Task 3 adds `body_map` + `index` for body lookup and topology-scoped indexes.

pub mod body_map;
pub mod differential;
pub mod edge;
pub mod extract;
pub mod extract_min;
pub mod index;
pub mod stub;

pub use edge::{
    DispatchShape, Edge, EdgeKind, Evidence, ObligationOutcome, Route, RouteTarget,
    SetCompleteness, Witness, classify_obligation, real_unknown_rate,
};
pub use stub::resolve_program;
