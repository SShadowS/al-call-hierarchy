use std::path::PathBuf;

fn main() {
    // Path to tree-sitter-al grammar
    let tree_sitter_al = PathBuf::from(
        std::env::var("TREE_SITTER_AL_PATH").unwrap_or_else(|_| "tree-sitter-al".to_string()),
    );

    let src_dir = tree_sitter_al.join("src");

    // Compile the tree-sitter AL parser
    let mut build = cc::Build::new();
    build
        .include(&src_dir)
        .file(src_dir.join("parser.c"))
        .warnings(false);

    // Check if scanner.c exists (some grammars have external scanners)
    let scanner_c = src_dir.join("scanner.c");
    if scanner_c.exists() {
        build.file(scanner_c);
    }

    // Check for scanner.cc (C++ scanner)
    let scanner_cc = src_dir.join("scanner.cc");
    if scanner_cc.exists() {
        build.cpp(true).file(scanner_cc);
    }

    build.compile("tree-sitter-al");

    println!("cargo:rerun-if-changed={}", src_dir.display());
    println!("cargo:rerun-if-env-changed=TREE_SITTER_AL_PATH");

    // Bake the App Insights connection string at build time when the
    // AL_CH_TELEMETRY_CONNECTION_STRING env var is set. Release pipelines set
    // this; local debug builds typically do not, so the binary falls back to
    // disabled telemetry at runtime.
    println!("cargo:rerun-if-env-changed=AL_CH_TELEMETRY_CONNECTION_STRING");
    if let Ok(cs) = std::env::var("AL_CH_TELEMETRY_CONNECTION_STRING") {
        println!("cargo:rustc-env=AL_CH_TELEMETRY_CONNECTION_STRING={}", cs);
    }
}
