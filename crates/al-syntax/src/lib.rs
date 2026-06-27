//! `al-syntax` — the ONLY crate that knows tree-sitter.
//!
//! Owns: the AL grammar FFI (`language`), and (added across the owned-syntax-IR
//! migration, Phase 0+) the generated raw vocabulary + typed CST, the lowerer,
//! and the owned AL syntax IR that the rest of the workspace consumes. Raw
//! grammar details never leave this crate; consumers see only the IR.

pub mod language;
pub mod raw;
