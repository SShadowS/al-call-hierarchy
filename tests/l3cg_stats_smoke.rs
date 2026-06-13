//! Smoke test for `aldump --l3-call-graph-stats`: the honest-taxonomy histogram
//! harness (spec §1/§8 measurement). Builds a tiny workspace on disk, runs the
//! binary, and asserts the JSON carries the bucket fields + a real-unknown rate.

use std::process::Command;

fn aldump_bin() -> std::path::PathBuf {
    let mut p = std::env::current_exe().unwrap();
    p.pop(); // test exe name
    if p.ends_with("deps") {
        p.pop();
    }
    p.push(if cfg!(windows) { "aldump.exe" } else { "aldump" });
    p
}

#[test]
fn l3_call_graph_stats_emits_histogram() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("app");
    std::fs::create_dir_all(src.join("src")).unwrap();
    std::fs::write(
        src.join("app.json"),
        r#"{"id":"00000000-0000-0000-0000-0000000000aa","name":"T","publisher":"P","version":"1.0.0.0","runtime":"13.0","idRanges":[{"from":50000,"to":50099}]}"#,
    )
    .unwrap();
    std::fs::write(
        src.join("src").join("a.al"),
        "codeunit 50100 A { procedure Caller() var C: Record Customer; begin C.FieldNo(\"No.\"); end; }",
    )
    .unwrap();

    let out = Command::new(aldump_bin())
        .arg("--l3-call-graph-stats")
        .arg(src.to_str().unwrap())
        .output()
        .expect("run aldump");
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).expect("valid JSON");
    assert!(json.get("total").is_some(), "histogram has total");
    assert!(json.get("builtin").is_some(), "histogram has builtin");
    assert!(json.get("unknown").is_some(), "histogram has unknown");
    assert!(json.get("realUnknownRate").is_some(), "histogram has realUnknownRate");
}
