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

pub mod app_package;
/// Shared big-stack execution for anywhere the `al_syntax` lowerer runs (T2.1,
/// stack-overflow hardening) — see the module doc.
pub mod big_stack;
/// Bounded-read helper shared by every zip/gzip decompression site (Task
/// T2.2, DoS hardening) — see the module doc.
pub mod capped_io;
pub mod config;
pub mod dependencies;
pub mod engine;
/// The legacy LSP call-hierarchy pipeline (T0.5): moved here — not duplicated
/// — so `benches/` and `tests/` can index a project and query the graph
/// in-process (no LSP stdio loop). `main.rs` re-exports these five under
/// `crate::{graph,handlers,indexer,parser,protocol}` via `pub use
/// al_call_hierarchy::{...}` (same pattern as `config`/`telemetry` above) so
/// the binary-only modules (`server.rs`, `watcher.rs`, `analysis.rs`) keep
/// compiling unchanged.
pub mod graph;
pub mod handlers;
pub mod indexer;
/// Tree-sitter AL language bindings. Exposed from the library so additive
/// binaries (e.g. the R0 `aldump`) can parse without duplicating the `extern`
/// declaration. `main.rs` keeps its own `mod language;` for the LSP binary;
/// the duplicate compilation is benign and pre-existing in this repo.
pub mod language;
pub mod parser;
pub mod program;
pub mod protocol;
pub mod snapshot;
pub mod telemetry;
/// Core AL object-type enum shared between lib and binary targets.
pub mod types;
