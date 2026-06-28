//! App-set snapshot ingestion substrate (Spec 1 / Plan 1A).
//!
//! Turns "workspace + symbol-only dep tables" into an explicit set of
//! identity-verified, per-app source roots ready for deep resolution.

pub mod identity;

pub use identity::{AppId, Provenance, TrustTier};
