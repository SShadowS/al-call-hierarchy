//! Plan 1B.2: fresh call/behaviour-edge resolver over `ProgramGraph`.
//! Phase 0 = edge model + dual-run differential harness (this module set).
//! Phase 1 Task 3 adds `body_map` + `index` for body lookup and topology-scoped indexes.

pub mod abi_check;
pub mod anon;
pub mod applicability;
pub mod arg_dispatch;
pub mod body_map;
pub mod builtins;
pub mod differential;
pub mod edge;
pub mod event;
pub mod extract;
pub mod extract_min;
pub mod framework_returns;
pub mod full;
pub mod index;
pub mod member_catalog;
pub mod receiver;
pub mod recordref_returns;
pub mod resolver;
pub mod semantic_golden;
pub mod stub;

pub use edge::{
    DispatchShape, Edge, EdgeKind, Evidence, ObligationOutcome, Route, RouteTarget,
    SetCompleteness, Witness, classify_obligation, real_unknown_rate,
};
pub use resolver::emit_event_flow_edges;
pub use stub::resolve_program;
