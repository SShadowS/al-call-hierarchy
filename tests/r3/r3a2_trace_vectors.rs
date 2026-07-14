//! R3a-2 Task 2 — JACOBI fingerprint-TRACE vector parity (Rev 2 #3).
//!
//! For each recursive SCC in the pipeline vectors, the Rust per-iteration
//! fingerprint sequence + iteration count + per-pass `changed` flags must
//! match the al-sem trace golden BYTE-FOR-BYTE.
//!
//! This test PROVES the Rust fixed-point is JACOBI (frozen prior-pass snapshot),
//! not Gauss-Seidel. A Gauss-Seidel implementation reaches the same final fixed
//! point but via a DIFFERENT trajectory (different iteration counts, different
//! per-pass changed sequence) — the trace would diverge here.
//!
//! The oracle: `r3a2-vectors.json` `expectedTrace` per pipeline vector.

use al_call_hierarchy::engine::l3::l3_workspace::assemble_and_resolve;
use al_call_hierarchy::engine::l4::summary::project_r3a2_with_trace;
use serde_json::Value;

const MODEL_INSTANCE_ID: &str = "r0";

#[derive(serde::Deserialize)]
struct VectorsDoc {
    #[serde(rename = "appGuid")]
    app_guid: String,
    #[serde(rename = "pipelineVectors")]
    pipeline_vectors: Vec<PipelineVector>,
}

#[derive(serde::Deserialize)]
struct PipelineVector {
    name: String,
    files: Vec<Vec<String>>,
    #[serde(rename = "expectedTrace")]
    expected_trace: TraceExpected,
}

#[derive(serde::Deserialize)]
struct TraceExpected {
    traces: Vec<Value>,
}

fn load_vectors() -> VectorsDoc {
    let raw = include_str!("../r3a2-vectors/r3a2-vectors.json");
    serde_json::from_str(raw).expect("r3a2-vectors.json parses into VectorsDoc")
}

fn files_of(files: &[Vec<String>]) -> Vec<(String, String)> {
    files
        .iter()
        .map(|pair| (pair[0].clone(), pair[1].clone()))
        .collect()
}

/// Assert that the Rust JACOBI trace (per-SCC iteration count + per-pass fingerprints
/// + changed flags) matches the al-sem trace golden byte-for-byte.
#[test]
fn all_r3a2_trace_vectors_match() {
    let doc = load_vectors();
    let mut failures: Vec<String> = Vec::new();

    for vec in &doc.pipeline_vectors {
        let files = files_of(&vec.files);
        let resolved = assemble_and_resolve(&files, &doc.app_guid, MODEL_INSTANCE_ID);
        let (_, trace) = project_r3a2_with_trace(&resolved);

        let actual_traces = serde_json::to_value(&trace.traces).unwrap();
        let expected_traces = Value::Array(vec.expected_trace.traces.clone());

        if actual_traces != expected_traces {
            failures.push(format!(
                "[{}] trace mismatch\n  expected: {}\n  actual:   {}",
                vec.name,
                serde_json::to_string_pretty(&expected_traces).unwrap(),
                serde_json::to_string_pretty(&actual_traces).unwrap()
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "R3a-2 JACOBI trace parity failures ({}):\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}

/// Guard: for all non-empty traces, the LAST pass must have `changed=false`
/// and all earlier passes must have `changed=true` — proving convergence.
#[test]
fn trace_convergence_invariant() {
    let doc = load_vectors();

    for vec in &doc.pipeline_vectors {
        let files = files_of(&vec.files);
        let resolved = assemble_and_resolve(&files, &doc.app_guid, MODEL_INSTANCE_ID);
        let (_, trace) = project_r3a2_with_trace(&resolved);

        for scc_trace in &trace.traces {
            let passes = &scc_trace.passes;
            if passes.len() < 2 {
                continue;
            }
            // Last pass: changed=false.
            let last = passes.last().unwrap();
            assert!(
                !last.changed,
                "[{}] SCC {} last pass must have changed=false (iteration {})",
                vec.name, scc_trace.scc_id, last.iteration
            );
            // All prior passes: changed=true.
            for pass in &passes[..passes.len() - 1] {
                assert!(
                    pass.changed,
                    "[{}] SCC {} pass {} (not last) must have changed=true",
                    vec.name, scc_trace.scc_id, pass.iteration
                );
            }
        }
    }
}

/// Guard: the recursive_3cycle_trace vector has exactly one SCC trace with 3 members
/// and exactly 3 iterations — the JACOBI trajectory requirement.
#[test]
fn recursive_3cycle_requires_3_iterations() {
    let doc = load_vectors();

    let vec = doc
        .pipeline_vectors
        .iter()
        .find(|v| v.name == "recursive_3cycle_trace")
        .expect("recursive_3cycle_trace vector present");

    let files = files_of(&vec.files);
    let resolved = assemble_and_resolve(&files, &doc.app_guid, MODEL_INSTANCE_ID);
    let (_, trace) = project_r3a2_with_trace(&resolved);

    assert_eq!(trace.traces.len(), 1, "exactly one recursive SCC trace");
    let scc_trace = &trace.traces[0];
    assert_eq!(scc_trace.members.len(), 3, "the SCC has 3 members");
    assert_eq!(
        scc_trace.iterations, 3,
        "JACOBI requires exactly 3 iterations for this 3-cycle"
    );
    assert_eq!(scc_trace.passes.len(), 3, "3 passes recorded");

    // Pass 1: changed=true (A gains Insert; C was already seeded)
    assert!(scc_trace.passes[0].changed, "pass 1: changed=true");
    // Pass 2: changed=true (B gains Insert from A)
    assert!(scc_trace.passes[1].changed, "pass 2: changed=true");
    // Pass 3: changed=false (convergence)
    assert!(!scc_trace.passes[2].changed, "pass 3: changed=false");
}
