//! R3a-2 — JACOBI fingerprint-TRACE differential (Rev 2 #3 — the JACOBI proof
//! over the corpus).
//!
//! For each committed al-sem trace golden under
//! `tests/r3a2-goldens/<fixture>.r3a2-trace.golden.json`, run the Rust source-only
//! pipeline WITH the per-recursive-SCC trace hook
//! (`assemble_and_resolve_workspace_default(...)` → `project_r3a2_with_trace(...)`)
//! over the matching `tests/r0-corpus/<fixture>` workspace and assert the Rust
//! per-SCC trace (sccId + members + iteration COUNT + per-pass `changed` flag +
//! per-pass stable FINGERPRINT) BYTE-MATCHES the golden.
//!
//! This catches a Gauss-Seidel / trajectory divergence: a Gauss-Seidel fixed point
//! reaches the SAME final summary (monotone lattice) but via a DIFFERENT trajectory
//! — different iteration count, different per-pass `changed` sequence, different
//! intermediate fingerprints. The trace would diverge HERE even when the
//! summary-core differential (final state only) passes. Proving the trace matches
//! proves the Rust fixed point is JACOBI (frozen prior-pass snapshot).
//!
//! Only fixtures with ≥1 recursive SCC carry a trace golden (the manifest's
//! `traceFile`); there are 3 in the corpus. The harness performs a direct strict
//! comparison — any divergence fails the test outright. There is no allowlist.

use std::path::PathBuf;

use al_call_hierarchy::engine::l3::l3_workspace::assemble_and_resolve_workspace_default;
use al_call_hierarchy::engine::l4::summary::{R3a2Trace, project_r3a2_with_trace};
use serde_json::Value;

#[path = "common/regen.rs"]
mod regen;

const R3A2_TRACE_TEST_NAME: &str = "differential_r3a2_trace_match_goldens";

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn goldens_dir() -> PathBuf {
    repo_root().join("tests").join("r3a2-goldens")
}

fn corpus_dir() -> PathBuf {
    repo_root().join("tests").join("r0-corpus")
}

#[derive(Debug, Clone)]
struct Divergence {
    fixture: String,
    path: String,
    golden_value: String,
    rust_value: String,
}

/// Discover every `tests/r3a2-goldens/*.r3a2-trace.golden.json`, sorted by fixture.
fn discover_trace_goldens() -> Vec<(String, PathBuf)> {
    let dir = goldens_dir();
    let mut out = Vec::new();
    let entries = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("failed to read R3a-2 goldens dir {}: {e}", dir.display()));
    for entry in entries {
        let entry = entry.expect("dir entry");
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.ends_with(".r3a2-trace.golden.json") {
            continue;
        }
        let fixture = name.trim_end_matches(".r3a2-trace.golden.json").to_string();
        out.push((fixture, entry.path()));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Recursively diff two trace values POSITIONALLY (both sides are already
/// canonically sorted by the projection — traces by sccId, members sorted, passes
/// by iteration).
fn diff_value(fixture: &str, path: &str, golden: &Value, rust: &Value, out: &mut Vec<Divergence>) {
    match (golden, rust) {
        (Value::Object(g), Value::Object(r)) => {
            for (k, gv) in g {
                let child = format!("{path}.{k}");
                match r.get(k) {
                    Some(rv) => diff_value(fixture, &child, gv, rv, out),
                    None => out.push(Divergence {
                        fixture: fixture.to_string(),
                        path: format!("{child}:MISSING_IN_RUST"),
                        golden_value: compact(gv),
                        rust_value: "<absent>".to_string(),
                    }),
                }
            }
            for (k, rv) in r {
                if !g.contains_key(k) {
                    out.push(Divergence {
                        fixture: fixture.to_string(),
                        path: format!("{path}.{k}:EXTRA_IN_RUST"),
                        golden_value: "<absent>".to_string(),
                        rust_value: compact(rv),
                    });
                }
            }
        }
        (Value::Array(g), Value::Array(r)) => {
            if g.len() != r.len() {
                out.push(Divergence {
                    fixture: fixture.to_string(),
                    path: format!("{path}:LENGTH"),
                    golden_value: g.len().to_string(),
                    rust_value: r.len().to_string(),
                });
            }
            let n = g.len().min(r.len());
            for i in 0..n {
                diff_value(fixture, &format!("{path}[{i}]"), &g[i], &r[i], out);
            }
            for (i, gv) in g.iter().enumerate().skip(n) {
                out.push(Divergence {
                    fixture: fixture.to_string(),
                    path: format!("{path}[{i}]:MISSING_IN_RUST"),
                    golden_value: compact(gv),
                    rust_value: "<absent>".to_string(),
                });
            }
            for (i, rv) in r.iter().enumerate().skip(n) {
                out.push(Divergence {
                    fixture: fixture.to_string(),
                    path: format!("{path}[{i}]:EXTRA_IN_RUST"),
                    golden_value: "<absent>".to_string(),
                    rust_value: compact(rv),
                });
            }
        }
        _ => {
            if golden != rust {
                out.push(Divergence {
                    fixture: fixture.to_string(),
                    path: path.to_string(),
                    golden_value: compact(golden),
                    rust_value: compact(rust),
                });
            }
        }
    }
}

fn compact(v: &Value) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| format!("{v:?}"))
}

/// The R3a-2 fingerprint-TRACE differential. Asserts the Rust JACOBI trajectory
/// (iteration count + per-pass `changed` + per-pass fingerprint) matches the
/// al-sem trace golden for every recursive-SCC-bearing fixture in the corpus.
#[test]
fn differential_r3a2_trace_match_goldens() {
    let goldens = discover_trace_goldens();
    assert!(
        !goldens.is_empty(),
        "no R3a-2 trace goldens discovered under {} — corpus missing?",
        goldens_dir().display()
    );

    let mut all_divergences: Vec<Divergence> = Vec::new();
    // Corpus-level invariant cross-checks (anti-degenerate): the trace corpus
    // must actually exercise the JACOBI loop.
    let mut total_recursive_sccs = 0usize;
    let mut sccs_requiring_2plus = 0usize;
    let mut max_iterations = 0usize;

    for (fixture, golden_path) in &goldens {
        let fixture_dir = corpus_dir().join(fixture);
        assert!(
            fixture_dir.is_dir(),
            "R3a-2 trace golden {} has no matching in-repo fixture at {}",
            golden_path.display(),
            fixture_dir.display()
        );

        // Rust side: source-only assemble+resolve → project_r3a2_with_trace → JSON.
        let trace = match assemble_and_resolve_workspace_default(&fixture_dir) {
            Some(resolved) => project_r3a2_with_trace(&resolved).1,
            None => R3a2Trace { traces: vec![] },
        };

        // REGEN path (Task T0.6 — this family previously had none; mirrors
        // `differential.rs` / r2_5a). When `REGEN_TEMP_GOLDENS=1`, write the
        // ENGINE-serialized trace straight to the golden file instead of
        // comparing — the goldens are Rust-owned baselines (TS oracle retired).
        if regen::regen_mode() {
            let mut pretty = serde_json::to_string_pretty(&trace)
                .unwrap_or_else(|e| panic!("regen serialize R3a-2 trace {fixture}: {e}"));
            pretty.push('\n');
            std::fs::write(golden_path, pretty)
                .unwrap_or_else(|e| panic!("regen write {}: {e}", golden_path.display()));
            eprintln!("REGEN r3a2-trace golden: {}", golden_path.display());
            continue;
        }

        let golden_text = std::fs::read_to_string(golden_path)
            .unwrap_or_else(|e| panic!("read R3a-2 trace golden {}: {e}", golden_path.display()));
        let golden_json: Value = serde_json::from_str(&golden_text).unwrap_or_else(|e| {
            panic!(
                "R3a-2 trace golden {} is not valid JSON: {e}",
                golden_path.display()
            )
        });
        // Shape guard: parses as the R3a2Trace serde type.
        let _: R3a2Trace = serde_json::from_value(golden_json.clone()).unwrap_or_else(|e| {
            panic!(
                "R3a-2 trace golden {} does not parse as R3a2Trace: {e}",
                golden_path.display()
            )
        });
        let rust_json = serde_json::to_value(&trace)
            .unwrap_or_else(|e| panic!("serialize Rust R3a-2 trace for {fixture}: {e}"));

        // Tally the anti-degenerate trace stats from the RUST output.
        for scc_trace in &trace.traces {
            total_recursive_sccs += 1;
            if scc_trace.iterations >= 2 {
                sccs_requiring_2plus += 1;
            }
            if scc_trace.iterations > max_iterations {
                max_iterations = scc_trace.iterations;
            }
        }

        diff_value(fixture, "", &golden_json, &rust_json, &mut all_divergences);
    }

    // REGEN mode wrote every golden above and asserts nothing (including the
    // anti-degenerate/manifest cross-checks below, which read committed goldens).
    if regen::regen_mode() {
        eprintln!("REGEN r3a2-trace: wrote {} golden(s)", goldens.len());
        return;
    }

    all_divergences
        .sort_by(|a, b| (a.fixture.as_str(), &a.path).cmp(&(b.fixture.as_str(), &b.path)));

    // --- ANTI-DEGENERATE trace gate (fail-on-zero) --------------------------
    // The JACOBI loop must actually iterate: ≥1 recursive SCC, and ≥1 recursive
    // SCC requiring ≥2 iterations (the cycle genuinely converged over passes).
    eprintln!(
        "R3a-2 trace ({} fixture(s)): recursiveSccs={} sccsRequiring2+={} maxIterations={}",
        goldens.len(),
        total_recursive_sccs,
        sccs_requiring_2plus,
        max_iterations,
    );
    assert!(
        total_recursive_sccs >= 1,
        "DEGENERATE R3a-2 trace: 0 recursive SCCs — the JACOBI loop never ran"
    );
    assert!(
        sccs_requiring_2plus >= 1,
        "DEGENERATE R3a-2 trace: no recursive SCC required ≥2 iterations — the \
         fixed point converged in one pass everywhere (a trajectory divergence \
         would NOT be caught)"
    );

    // Cross-check the trace stats against the al-sem manifest's matrix block.
    let manifest_path = goldens_dir().join("manifest.json");
    let manifest_text = std::fs::read_to_string(&manifest_path)
        .unwrap_or_else(|e| panic!("read R3a-2 manifest {}: {e}", manifest_path.display()));
    let manifest: Value = serde_json::from_str(&manifest_text)
        .unwrap_or_else(|e| panic!("R3a-2 manifest not valid JSON: {e}"));
    let mat = manifest
        .get("matrix")
        .expect("R3a-2 manifest carries a matrix block");
    let m_recursive = mat
        .get("recursiveSccCount")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;
    let m_2plus = mat
        .get("recursiveSccsRequiring2PlusIterations")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;
    let m_max = mat
        .get("maxRecursiveSccIterations")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;
    assert_eq!(
        total_recursive_sccs, m_recursive,
        "R3a-2 trace recursiveSccCount MISMATCH vs manifest: rust={total_recursive_sccs} manifest={m_recursive}"
    );
    assert_eq!(
        sccs_requiring_2plus, m_2plus,
        "R3a-2 trace recursiveSccsRequiring2PlusIterations MISMATCH vs manifest: \
         rust={sccs_requiring_2plus} manifest={m_2plus}"
    );
    assert_eq!(
        max_iterations, m_max,
        "R3a-2 trace maxRecursiveSccIterations MISMATCH vs manifest: rust={max_iterations} manifest={m_max}"
    );

    // --- Direct strict divergence gate (no allowlist) -----------------------
    let mut failure = String::new();
    if !all_divergences.is_empty() {
        failure.push_str(&format!(
            "\n{} R3a-2 trace divergence(s) found ({R3A2_TRACE_TEST_NAME}):\n",
            all_divergences.len()
        ));
        for d in &all_divergences {
            failure.push_str(&format!(
                "  [{}] {}\n      golden = {}\n      rust   = {}\n",
                d.fixture, d.path, d.golden_value, d.rust_value
            ));
        }
    }

    assert!(
        failure.is_empty(),
        "R3a-2 fingerprint-TRACE differential FAILED:{failure}"
    );

    eprintln!(
        "R3a-2 trace differential: {} fixture(s), 0 divergences.",
        goldens.len(),
    );
}
