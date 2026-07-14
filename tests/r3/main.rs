//! Umbrella test crate: R3 obligation/trace suites (test-crate consolidation, 2026-07-15 spec).
#[path = "../common/regen.rs"]
mod regen;

mod r3a0_unfetched_dep_opaque;
mod r3a1_differential;
mod r3a1_oracles;
mod r3a1_vectors;
mod r3a2_branch_aware;
mod r3a2_differential;
mod r3a2_oracles;
mod r3a2_trace_differential;
mod r3a2_trace_vectors;
mod r3a2_vectors;
mod r3a3_differential;
mod r3a3_oracles;
mod r3a3_vectors;
mod r3a4_differential;
mod r3a4_oracles;
mod r3a4_vectors;
mod r3a5_differential;
mod r3a5_oracles;
mod r3b_incremental_equality;
mod r3b_incremental_nondeterminism;
mod r3b_minimality;
mod r3b_wrapped_parity;
