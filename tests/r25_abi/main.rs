//! Umbrella test crate: R2.5 ABI/dependency suites (test-crate consolidation, 2026-07-15 spec).
#[path = "../common/regen.rs"]
mod regen;

mod r2_5a_abi_native_vectors;
mod r2_5a_aldump_cli;
mod r2_5a_attr_vectors;
mod r2_5a_differential;
mod r2_5a_oracles;
mod r2_5a_stable_id_vectors;
mod r2_5b_cg_differential;
mod r2_5b_cg_oracles;
mod r2_5b_cov_differential;
mod r2_5b_cov_oracles;
mod r2_5b_eg_differential;
mod r2_5b_eg_oracles;
mod r2_5b_rt_differential;
mod r2_5b_rt_oracles;
