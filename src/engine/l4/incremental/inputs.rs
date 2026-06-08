//! Fine-grained Salsa INPUTS for the L4 incremental query graph (R3b Task 1).
//!
//! These are the equality-aware inputs keyed by `StableRoutineId` (NOT a
//! monolithic `resolved_model`): an edit changes ONE input → only its reverse
//! cone recomputes (Stage 2/3). Stage 1 just demands them on a fresh DB.
//!
//! Heavyweight L3 structures (`L3Routine`, the combined-graph edge slices, the
//! shared object/table/event context) are carried via `Arc` so Salsa accepts
//! them as input field values without a structural `Eq` bound. (Stage 2 may
//! refine the per-routine equality for tighter early-cutoff; Stage 1's wrapped
//! parity does not depend on it.)

use std::sync::Arc;

use crate::engine::l3::call_resolver::UpgradedBinding;
use crate::engine::l3::event_graph::EventGraph;
use crate::engine::l3::l3_workspace::{L3Object, L3Routine, L3Table};
use crate::engine::l4::capability_cone::CapabilityFact;
use crate::engine::l4::combined_graph::{CombinedEdge, TypedEdge, UncertaintyEdge};
use crate::engine::l4::summary::RoutineSummary;

/// The authoritative routine universe per app — `routine_ids(app_key)`. A fine-
/// grained input with early-cutoff. `combined_graph` enumerates THIS, not
/// scattered inputs, so an add/remove is one universe edit + one per-routine
/// input create/tombstone. The members are the internal RoutineIds (the
/// combined-graph node ids); each maps 1:1 to a [`RoutineInput`].
#[salsa::input(debug)]
pub struct RoutineUniverse {
    /// Internal RoutineIds in sorted order (the combined-graph node universe).
    #[returns(ref)]
    pub routine_ids: Vec<String>,
}

/// Per-routine fine-grained input, keyed (externally) by internal RoutineId.
/// Carries the routine's OWN facts: body facts, its resolved outgoing call edges
/// + typed edges, its base (direct) summary, its direct capability facts +
/// coverage, and the leaf/retained-summary seam (R3a-5 dep routines). One input
/// per routine in the [`RoutineUniverse`].
#[salsa::input(debug)]
pub struct RoutineInput {
    /// Internal RoutineId (== the universe member + the combined-graph node id).
    #[returns(ref)]
    pub routine_id: String,
    /// Body facts — the resolved `L3Routine` (the `compose_routine` /
    /// `direct_facts_for_routine` / cfg-walker input).
    #[returns(ref)]
    pub routine: Arc<L3Routine>,
    /// This routine's OUTGOING resolved combined edges (sorted by edgeSortKey),
    /// the per-`from` slice of the combined graph.
    #[returns(ref)]
    pub combined_edges: Arc<Vec<CombinedEdge>>,
    /// This routine's OUTGOING typed edges (the cone substrate slice).
    #[returns(ref)]
    pub typed_edges: Arc<Vec<TypedEdge>>,
    /// This routine's uncertainty edges (to-less callsites).
    #[returns(ref)]
    pub uncertainty_edges: Arc<Vec<UncertaintyEdge>>,
    /// The routine's BASE intraprocedural summary (direct dbEffects, via:"direct")
    /// — the per-routine summary seed the JACOBI folds callee facts into.
    #[returns(ref)]
    pub base_summary: Arc<RoutineSummary>,
    /// The routine's DIRECT capability facts (the cone's distance-0 facts).
    #[returns(ref)]
    pub direct_facts: Arc<Vec<CapabilityFact>>,
    /// The routine's DIRECT coverage `(direct_status, reasons)`.
    #[returns(ref)]
    pub direct_coverage: Arc<(String, Vec<String>)>,
    /// `bodyAvailable` — the opaque-callee guards read this for every callee.
    pub body_available: bool,
    /// Is this a FIXED LEAF (an R3a-5 dep routine carrying a RETAINED summary,
    /// never recomputed)? When true, `base_summary` IS the retained summary.
    pub is_leaf: bool,
}

/// App identity + the L3-from-scratch SHARED context the combined-graph rebuild
/// and the JACOBI / cone need but which is NOT per-routine (objects/tables for the
/// field index + the resolved event graph + the upgraded-binding side table).
/// L0–L3 stay from-scratch INPUTS to L4 (incrementalizing them is later work).
#[salsa::input(debug)]
pub struct AppContext {
    /// The app identity (GUID namespace for every StableObjectId) — `app_identity`.
    #[returns(ref)]
    pub app_identity: String,
    /// Resolved objects (the typed-edge object-run target resolution + projection).
    #[returns(ref)]
    pub objects: Arc<Vec<L3Object>>,
    /// Resolved tables (the field-resolution index for parameterRoles).
    #[returns(ref)]
    pub tables: Arc<Vec<L3Table>>,
    /// The resolved event graph (event-dispatch edges + publisher injection +
    /// stable event-id projection).
    #[returns(ref)]
    pub event_graph: Arc<EventGraph>,
    /// Per-callsite upgraded bindings (cross-call parameterRoles composition).
    #[returns(ref)]
    pub upgraded_bindings: Arc<std::collections::HashMap<String, Vec<UpgradedBinding>>>,
    /// internal RoutineId → StableRoutineId (the projection map; every merged
    /// routine carries `stable_routine_id`).
    #[returns(ref)]
    pub stable_map: Arc<std::collections::HashMap<String, String>>,
}

/// The handle registry: internal RoutineId → its [`RoutineInput`] handle, so a
/// tracked query over the [`RoutineUniverse`] can resolve each member's per-
/// routine input. Salsa input handles are `Copy + 'static`, so storing them in a
/// map is sound. (Carried as its own input so adding/removing a routine touches
/// only the universe + this registry, not a per-routine fan-out.)
#[salsa::input(debug)]
pub struct RoutineRegistry {
    #[returns(ref)]
    pub by_id: Arc<std::collections::HashMap<String, RoutineInput>>,
}

/// The dep-artifact stamp — the cross-app invalidation key (R3a-5). A change to a
/// fetched dep `.app`'s identity/content bumps this, invalidating the cross-app
/// reverse cone. Stage 1 sets it once; Stage 2/3 exercise the bump.
#[salsa::input(debug)]
pub struct DepStamp {
    #[returns(ref)]
    pub stamp: String,
}
