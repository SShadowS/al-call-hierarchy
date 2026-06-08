//! R3b Task 1 — STAGE 1 WRAPPED-PARITY proof.
//!
//! The R3b oracle FLIPS (vs R3a's TS-vs-Rust): the VALUE is identical to R3a, so
//! the proof is Salsa-WRAPPED-vs-Rust-from-scratch. This test demands the L4
//! summaries + cone THROUGH the Salsa query graph (`engine::l4::incremental`) and
//! asserts the wrapped projection is BYTE-IDENTICAL to the R3a from-scratch
//! projection — which, via R3a parity (KNOWN_DIVERGENCES=[]), byte-matches the
//! al-sem goldens. NO incrementality yet (Stage 1): every query recomputes on a
//! fresh DB.
//!
//! Surfaces re-run through the Salsa layer:
//!   - R3a-5 cross-app FULL RoutineSummary (the exit-gate; the representative
//!     cross-app fixture) — wrapped == from-scratch AND wrapped == golden bytes.
//!   - R3a-3 source-only cone/coverage over the whole `r0-corpus` (every fixture
//!     with a committed r3a3 golden) — wrapped == from-scratch per fixture.
//!
//! KNOWN_DIVERGENCES stays `[]` (no allowlist entries are consulted — the proof is
//! exact byte-equality, not an allowlisted differential).

use std::path::PathBuf;

use al_call_hierarchy::engine::l3::l3_workspace::assemble_and_resolve_workspace_default;
use al_call_hierarchy::engine::l4::capability_cone::{project_r3a3, project_r3a5_cross_app};
use al_call_hierarchy::engine::l4::incremental::wrap::{
    salsa_r3a3_source_only, salsa_r3a5_cross_app,
};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

// ---------------------------------------------------------------------------
// R3a-5 cross-app exit-gate — wrapped == from-scratch == golden (byte-for-byte).
// ---------------------------------------------------------------------------

#[test]
fn r3b_stage1_wrapped_r3a5_cross_app_byte_matches_from_scratch_and_golden() {
    const FIXTURE: &str = "cross-app-full-summary";
    const MODEL_INSTANCE_ID: &str = "r0";
    let ws = repo_root().join("tests").join("r3a5-fixtures").join("ws");

    // From-scratch (the R3a-5 exit-gate path).
    let from_scratch = project_r3a5_cross_app(&ws, MODEL_INSTANCE_ID, FIXTURE);
    // Salsa-WRAPPED (demanded through the L4 incremental query graph).
    let wrapped = salsa_r3a5_cross_app(&ws, MODEL_INSTANCE_ID, FIXTURE);

    let from_scratch_json = serde_json::to_string_pretty(&from_scratch).unwrap();
    let wrapped_json = serde_json::to_string_pretty(&wrapped).unwrap();

    assert_eq!(
        wrapped_json, from_scratch_json,
        "R3b Stage 1: Salsa-wrapped R3a-5 cross-app summary is NOT byte-identical to \
         the from-scratch projection"
    );

    // Transitively: the from-scratch path byte-matches the al-sem golden (R3a-5
    // exit gate, KNOWN_DIVERGENCES=[]). Re-assert here so the wrapped path is
    // tied directly to the golden bytes.
    let golden_path = repo_root()
        .join("tests")
        .join("r3a5-goldens")
        .join(format!("{FIXTURE}.r3a5.golden.json"));
    let golden_text = std::fs::read_to_string(&golden_path)
        .unwrap_or_else(|e| panic!("read R3a-5 golden {}: {e}", golden_path.display()));
    assert_eq!(
        wrapped_json.trim_end(),
        golden_text.trim_end(),
        "R3b Stage 1: Salsa-wrapped R3a-5 projection is NOT byte-identical to the \
         al-sem golden (KNOWN_DIVERGENCES=[])"
    );

    // Anti-degenerate: the cross-app cone genuinely fired through Salsa.
    assert!(
        wrapped.primary_routines_with_inherited_dep_facts >= 1,
        "wrapped cross-app cone must propagate ≥1 dep fact to a primary"
    );
    assert!(
        wrapped.primary_routines_with_dep_db_effects >= 1,
        "wrapped cross-app must fold ≥1 dep dbEffect into a primary"
    );

    eprintln!(
        "R3b Stage 1 (r3a5): {} summaries, Salsa-wrapped == from-scratch == golden, \
         KNOWN_DIVERGENCES=[]",
        wrapped.summaries.len()
    );
}

// ---------------------------------------------------------------------------
// R3a-3 source-only cone/coverage — wrapped == from-scratch over the corpus.
// ---------------------------------------------------------------------------

fn discover_r3a3_fixtures() -> Vec<String> {
    let dir = repo_root().join("tests").join("r3a3-goldens");
    let mut out = Vec::new();
    let entries = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read r3a3 goldens dir {}: {e}", dir.display()));
    for entry in entries {
        let name = entry
            .expect("dir entry")
            .file_name()
            .to_string_lossy()
            .to_string();
        if let Some(fixture) = name.strip_suffix(".r3a3.golden.json") {
            out.push(fixture.to_string());
        }
    }
    out.sort();
    out
}

#[test]
fn r3b_stage1_wrapped_r3a3_source_only_byte_matches_from_scratch() {
    let corpus = repo_root().join("tests").join("r0-corpus");
    let fixtures = discover_r3a3_fixtures();
    assert!(
        !fixtures.is_empty(),
        "no r3a3 goldens discovered — corpus missing?"
    );

    let mut checked = 0usize;
    let mut nonempty = 0usize;
    for fixture in &fixtures {
        let fixture_dir = corpus.join(fixture);
        if !fixture_dir.is_dir() {
            continue;
        }
        let resolved = match assemble_and_resolve_workspace_default(&fixture_dir) {
            Some(r) => r,
            None => continue,
        };

        let from_scratch = project_r3a3(&resolved);
        let wrapped = salsa_r3a3_source_only(&resolved);

        let fs_json = serde_json::to_string_pretty(&from_scratch).unwrap();
        let w_json = serde_json::to_string_pretty(&wrapped).unwrap();
        assert_eq!(
            w_json, fs_json,
            "R3b Stage 1: Salsa-wrapped R3a-3 cone/coverage for fixture `{fixture}` is NOT \
             byte-identical to the from-scratch projection"
        );
        checked += 1;
        if !wrapped.summaries.is_empty() {
            nonempty += 1;
        }
    }

    assert!(checked > 0, "no r3a3 fixtures were actually checked");
    assert!(
        nonempty > 0,
        "every wrapped r3a3 projection was empty — the wrap is not exercising the cone"
    );
    eprintln!(
        "R3b Stage 1 (r3a3): {checked} source-only fixture(s) checked, {nonempty} non-empty, \
         Salsa-wrapped == from-scratch"
    );
}
