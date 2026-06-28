//! R4-F — ROOT CLASSIFICATION stable-projection differential.
//!
//! For each committed al-sem golden under
//! `tests/r4f-goldens/<fixture>.rootclass.golden.json`, run the Rust source-only
//! L0→L3 pass (`assemble_and_resolve_workspace_default(...)`, which now classifies
//! AST roots + overlays `<workspace>/roots.config.json`) over the matching
//! `tests/r0-corpus/<fixture>` workspace, project it to the stable
//! RootClassification form (`project_r4f_root_classifications`), pretty-serialize
//! it (serde_json pretty + trailing newline — the exact on-disk golden form), and
//! assert BYTE-equality.
//!
//! ## Anti-degenerate
//!
//! - `ws-d51-jobqueue` MUST carry a classification whose `kinds` contain
//!   `"job-queue-entrypoint"` (the roots.config overlay path).
//! - `ws-txn-d47-event-pos` MUST carry an `"event-subscriber"` kind (the AST path).

use std::path::PathBuf;

use al_call_hierarchy::engine::l3::l3_workspace::assemble_and_resolve_workspace_default;
use al_call_hierarchy::engine::root_classification::{
    R4FRootClassProjection, project_r4f_root_classifications,
};

/// The R4-F root-classification corpus (mirrors al-sem
/// `scripts/r4f-root-classification-projection.ts` `R4F_ROOT_CLASS_FIXTURES`).
const FIXTURES: &[&str] = &[
    "ws-d51-jobqueue",
    "ws-d51-pos",
    "ws-d51-neg",
    "ws-d50-pos",
    "ws-d50-neg",
    "ws-txn-d47-event-pos",
    "ws-txn-d47-pos-http-nocommit",
];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn goldens_dir() -> PathBuf {
    repo_root().join("tests").join("r4f-goldens")
}

fn corpus_dir() -> PathBuf {
    repo_root().join("tests").join("r0-corpus")
}

/// Pretty-serialize + trailing newline — the exact on-disk golden form (matches
/// `r4_differential::pretty_with_newline`).
fn pretty_with_newline(proj: &R4FRootClassProjection) -> String {
    let mut s = serde_json::to_string_pretty(proj).expect("serialize R4-F projection");
    s.push('\n');
    s
}

/// Run the Rust source-only L0→L3 pass + stable projection for one fixture.
fn run_rust(fixture: &str) -> R4FRootClassProjection {
    let fixture_dir = corpus_dir().join(fixture);
    assert!(
        fixture_dir.is_dir(),
        "R4-F golden for {fixture} has no matching in-repo fixture at {} (offline corpus incomplete)",
        fixture_dir.display()
    );
    match assemble_and_resolve_workspace_default(&fixture_dir) {
        Some(resolved) => project_r4f_root_classifications(&resolved, fixture),
        None => R4FRootClassProjection {
            fixture_name: fixture.to_string(),
            classification_count: 0,
            classifications: vec![],
        },
    }
}

#[test]
fn r4f_root_classifications_match_goldens() {
    for fixture in FIXTURES {
        let golden_path = goldens_dir().join(format!("{fixture}.rootclass.golden.json"));
        let golden_text = std::fs::read_to_string(&golden_path)
            .unwrap_or_else(|e| panic!("cannot read R4-F golden {}: {e}", golden_path.display()));

        let projection = run_rust(fixture);
        let rust_text = pretty_with_newline(&projection);

        assert_eq!(
            rust_text,
            golden_text,
            "R4-F ACCEPTANCE GATE: {fixture} did NOT byte-match its golden ({})",
            golden_path.display()
        );
    }
}

#[test]
fn anti_degenerate_jobqueue_has_job_queue_entrypoint() {
    let projection = run_rust("ws-d51-jobqueue");
    assert!(
        projection
            .classifications
            .iter()
            .any(|c| c.kinds.iter().any(|k| k == "job-queue-entrypoint")),
        "ws-d51-jobqueue must produce a job-queue-entrypoint classification (config overlay path)"
    );
}

#[test]
fn anti_degenerate_event_pos_has_event_subscriber() {
    let projection = run_rust("ws-txn-d47-event-pos");
    assert!(
        projection
            .classifications
            .iter()
            .any(|c| c.kinds.iter().any(|k| k == "event-subscriber")),
        "ws-txn-d47-event-pos must produce an event-subscriber classification (AST path)"
    );
}
