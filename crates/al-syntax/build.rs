use std::path::PathBuf;

/// Compile the tree-sitter-al grammar C into this crate. `al-syntax` is the ONLY
/// crate that links the grammar; the rest of the workspace reaches it through
/// `al_syntax::language::language()`.
fn main() {
    // Default to the repo-root submodule (build.rs CWD is this crate dir).
    let tree_sitter_al = PathBuf::from(
        std::env::var("TREE_SITTER_AL_PATH").unwrap_or_else(|_| "../../tree-sitter-al".to_string()),
    );
    let src_dir = tree_sitter_al.join("src");

    let mut build = cc::Build::new();
    build
        .include(&src_dir)
        .file(src_dir.join("parser.c"))
        .warnings(false);

    let scanner_c = src_dir.join("scanner.c");
    if scanner_c.exists() {
        build.file(scanner_c);
    }
    let scanner_cc = src_dir.join("scanner.cc");
    if scanner_cc.exists() {
        build.cpp(true).file(scanner_cc);
    }

    build.compile("tree-sitter-al");

    println!("cargo:rerun-if-changed={}", src_dir.display());
    println!("cargo:rerun-if-env-changed=TREE_SITTER_AL_PATH");
}
