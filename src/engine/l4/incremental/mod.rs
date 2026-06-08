//! L4 INCREMENTAL (Phase R3b, Task 1 — Stage 1: WRAPPED, no incrementality yet).
//!
//! Salsa version: **0.27.0** (PINNED in Cargo.toml — the new-Salsa
//! `#[salsa::db]` / `#[salsa::input]` / `#[salsa::tracked]` / `#[salsa::interned]`
//! API). The interning + cycle behavior differs materially across Salsa
//! versions, so the dependency is pinned with `=0.27.0`.
//!
//! This module makes the R3a L4 fixed point INCREMENTAL by expressing it as a
//! demand-driven Salsa query graph. R3b Task 1 (Stage 1) introduces the framework
//! + the query TOPOLOGY + the nondeterminism audit, and PROVES the Salsa-WRAPPED
//! result is byte-identical to the R3a from-scratch goldens. No input EDITS / no
//! incrementality yet — every query recomputes on a fresh DB (Stage 2 wires the
//! edit surface, Stage 3 proves reverse-cone minimality).
//!
//! ## The query TOPOLOGY (externally-reviewed; the crux — see the plan's
//! "## SCC identity + projection queries" + "## Routine universe + id churn")
//!
//! FINE-GRAINED inputs (NOT a monolithic `resolved_model`):
//!   - [`inputs::RoutineUniverse`] — `routine_ids` : `BTreeSet<StableRoutineId>`,
//!     the authoritative routine universe `combined_graph` enumerates.
//!   - [`inputs::RoutineInput`] (per StableRoutineId) — the routine's own body
//!     facts (`L3Routine`), resolved call edges, typed edges, direct dbEffects
//!     (its base summary), direct capability facts + direct coverage, the
//!     `is_leaf` flag + retained leaf summary, `body_available`.
//!   - [`inputs::AppContext`] — `app_identity` + the L3-from-scratch shared
//!     context the combined-graph rebuild needs (objects/tables/event graph /
//!     upgraded bindings / field index). L0–L3 stay from-scratch INPUTS to L4.
//!   - [`inputs::DepStamp`] — the dep-artifact stamp (cross-app invalidation key).
//!
//! TRACKED queries (the data-flow that makes invalidation non-vacuous):
//!   `combined_graph` (structural; over `routine_ids`)
//!     → `scc_condensation` (the Tarjan pass; its output POPULATES the
//!        projections, it is NOT depended on directly by `scc_summaries`)
//!     → an INTERNED [`queries::SccKey`] = the interned SORTED member
//!        `StableRoutineId` set (an unchanged SCC re-interns to the SAME key)
//!     → the PROJECTION queries `scc_for_routine` / `scc_members` /
//!        `scc_successors` (these EARLY-CUT for an unchanged SCC)
//!     → `scc_summaries(scc_key)` — the internal JACOBI loop over `scc_members`
//!        in SORTED order; depends on `scc_members` / `scc_successors` / the
//!        members' inputs / successor `scc_summaries` — NOT the monolithic
//!        condensation. Reuses the PROVEN R3a `run_one_scc` (no re-port).
//!     → `routine_summary(stable_id)` → the cone `inherited_facts` + `coverage`.
//!
//! The intra-SCC fixed point is the already-byte-parity R3a JACOBI algorithm
//! (`summary_runner::run_one_scc`), called from the `scc_summaries` query body —
//! Salsa handles only the INTER-SCC incrementality (an SCC query depends on its
//! successor SCCs' queries). We do NOT use Salsa's cycle-recovery API (the plan's
//! preferred internal-loop pattern).
//!
//! ## Determinism (R3b Rev 2 #4)
//!
//! The nondeterminism audit (Task 1) swept every L4 query body + the R3a code the
//! queries call for internal `HashMap`/`HashSet` whose iteration order could leak
//! into output. One latent source was fixed at the source
//! (`cfg_walker::param_field_accesses` now iterates its position keys sorted).
//! The SCC member iteration order is the SORTED `StableRoutineId` set (asserted at
//! the `scc_summaries` entry), so the JACOBI loop iterates members canonically.

pub mod inputs;
pub mod queries;
pub mod wrap;

/// The R3b L4 Salsa database trait. Query functions take `&dyn L4Db`.
#[salsa::db]
pub trait L4Db: salsa::Database {}

/// The concrete Salsa database for the L4 incremental query graph.
#[salsa::db]
#[derive(Default, Clone)]
pub struct L4Database {
    storage: salsa::Storage<Self>,
}

#[salsa::db]
impl salsa::Database for L4Database {}

#[salsa::db]
impl L4Db for L4Database {}
