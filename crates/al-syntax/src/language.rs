//! Tree-sitter AL grammar FFI binding. The grammar C is compiled into this crate
//! by `build.rs`; this is the single link point for `tree_sitter_al`.

use tree_sitter::Language;

extern "C" {
    fn tree_sitter_al() -> Language;
}

/// Get the tree-sitter AL language.
///
/// # Safety
/// Calls into the compiled C code from tree-sitter-al.
pub fn language() -> Language {
    unsafe { tree_sitter_al() }
}
