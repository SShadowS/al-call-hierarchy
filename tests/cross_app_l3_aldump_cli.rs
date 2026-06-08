//! R2.5b Task 1 — `aldump --l3-cross-app` CLI smoke. Invokes the ACTUAL `aldump`
//! binary on the committed `.app`-bearing workspace fixture and asserts the emitted
//! envelope carries NON-EMPTY cross-app resolution (the four L3 surfaces). Locks the
//! CLI flag wiring + the end-to-end disk path (workspace `.al` source + `.alpackages`
//! `.app`s → merged L3).

use std::path::PathBuf;
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn aldump_l3_cross_app_emits_nonempty_resolution() {
    let bin = env!("CARGO_BIN_EXE_aldump");
    let fixture = repo_root().join("tests/r2-5b-fixtures/cross-app-resolution");

    let out = Command::new(bin)
        .arg("--l3-cross-app")
        .arg(&fixture)
        .output()
        .unwrap_or_else(|e| panic!("spawn aldump: {e}"));
    assert!(
        out.status.success(),
        "aldump --l3-cross-app exited non-zero: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("aldump emits valid JSON envelope");

    // The call graph resolves ≥1 cross-app member call to a dep StableRoutineId.
    let groups = v["callGraph"]["groups"]
        .as_array()
        .expect("callGraph.groups");
    let resolved_to_dep = groups.iter().any(|g| {
        g["edges"].as_array().is_some_and(|edges| {
            edges.iter().any(|e| {
                e["resolution"] == "resolved"
                    && e["to"]
                        .as_str()
                        .is_some_and(|t| t.starts_with("dddddddd-0000-0000-0000-000000000001"))
            })
        })
    });
    assert!(
        resolved_to_dep,
        "≥1 cross-app member call resolved to a dep routine"
    );

    // coverage.opaqueApps now lists the symbol-only dep apps (R3a-0 Fix 2 — latent bug
    // FIXED in al-sem `81d538a`+`f1650ba`): buildCoverage filters index.identity.apps by
    // sourceKind=="symbol-only", and withDependencyArtifacts now stamps the dep
    // AppIdentitys (with sourceKind) into identity.apps. The corpus's two deps are
    // symbol-only, so opaqueApps carries them.
    let opaque = v["coverage"]["opaqueApps"].as_array().expect("opaqueApps");
    assert!(
        !opaque.is_empty(),
        "opaqueApps lists the symbol-only dep apps (R3a-0 Fix 2)"
    );
    assert!(
        opaque
            .iter()
            .any(|a| a.as_str() == Some("dddddddd-0000-0000-0000-000000000001")),
        "opaqueApps includes the symbol-only Lib Core dep"
    );
    // The cross-app coverage WIN is still observable: the external-target member miss
    // stays IN unresolvedCallsites (proving the unresolved multiset reflects cross-app
    // resolution, not a source-only "everything unresolved" or empty surface).
    let unresolved = v["coverage"]["unresolvedCallsites"]
        .as_array()
        .expect("unresolvedCallsites");
    assert!(
        !unresolved.is_empty(),
        "unresolvedCallsites carries the member-not-found + external-target misses"
    );

    // ≥2 event edges (ws→dep + dep→ws).
    let edges = v["eventGraph"]["edges"]
        .as_array()
        .expect("eventGraph.edges");
    assert!(edges.len() >= 2, "≥2 cross-app subscriber edges");
}
