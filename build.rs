// The tree-sitter-al grammar is now compiled+linked by the `al-syntax` crate
// (owned-syntax-IR migration, Phase -1). This build script only bakes the
// optional App Insights connection string for release telemetry builds.
fn main() {
    println!("cargo:rerun-if-env-changed=AL_CH_TELEMETRY_CONNECTION_STRING");
    if let Ok(cs) = std::env::var("AL_CH_TELEMETRY_CONNECTION_STRING") {
        println!("cargo:rustc-env=AL_CH_TELEMETRY_CONNECTION_STRING={}", cs);
    }
}
