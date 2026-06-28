//! Library target for benchmarks. The binary continues to use `main.rs`.
//!
//! This file exists primarily so `criterion` benches can import telemetry
//! types via the crate name. Adding more modules here is fine, but be aware
//! that any module declared in both `lib.rs` and `main.rs` becomes a
//! duplicate compilation — keep this small.
//!
//! `config` is included alongside `telemetry` because `telemetry/mod.rs`
//! references `crate::config::TelemetryFileConfig`. Without it, the lib
//! crate cannot compile.

pub mod config;
pub mod engine;
/// Tree-sitter AL language bindings. Exposed from the library so additive
/// binaries (e.g. the R0 `aldump`) can parse without duplicating the `extern`
/// declaration. `main.rs` keeps its own `mod language;` for the LSP binary;
/// the duplicate compilation is benign and pre-existing in this repo.
pub mod language;
pub mod snapshot;
pub mod telemetry;
