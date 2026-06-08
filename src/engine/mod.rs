//! R0 migration engine — additive, isolated from the LSP binary.
//!
//! Everything under `engine` is part of the al-sem → Rust port and is gated by
//! the differential harness. It must not depend on or alter the LSP method
//! surface.

pub mod deps;
pub mod ids;
pub mod l2;
pub mod l3;
pub mod l4;
pub mod l5;
pub mod snapshot;
