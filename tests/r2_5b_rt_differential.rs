//! R2.5b-a — CROSS-APP L3 record-types differential + the anti-degenerate
//! CROSS-APP coverage matrix.
//!
//! For each committed al-sem golden under `tests/r2-5b-rt-goldens/`, run the Rust
//! cross-app L3 build (`build_cross_app_l3_from_workspace`) over the matching
//! `.app`-bearing workspace fixture under `tests/r2-5b-fixtures/<fixture>/`
//! (workspace `.al` source + `.alpackages/*.app` deps), project to the
//! golden-shaped record-type projection (`project_record_types`), and assert it
//! BYTE-MATCHES the golden — same projection shape as R2a, but the corpus carries
//! `.alpackages`, so a record var typed as a DEP table binds to the dep
//! StableTableId and a base-table field visible only via a dep TableExtension merge
//! resolves onto the dep base table.
//!
//! The `.app` fixtures are the EXACT SAME bytes the al-sem golden generator read
//! (the committed `test/fixtures/r2.5b-deps/` copied into the fixture's
//! `.alpackages/`), so this is a TRUE byte-parity diff. Default `cargo test` runs
//! OFFLINE: everything is committed in-repo.
//!
//! ## Capture point (R2.5b-a)
//!
//! POST-`resolveModel` over the MERGED index (`noDepSummaries:true`) — the dep
//! `tableId` is BACKFILLED in place; the dep-extension fields are merged onto the
//! dep base table. modelInstanceId pinned to `r2.5b` on BOTH sides (the
//! `operationId` carries that prefix; the StableTableId/StableRoutineId keys are
//! modelInstanceId-independent).
//!
//! ## CROSS-APP anti-degenerate matrix (fail-on-zero, Rev 2 #1)
//!
//! Computed from the RUST output (proves cross-app resolution actually FIRES, not
//! "opaque == opaque"): nonzero (1) record VARS bound to a DEP StableTableId, (2)
//! record OPS bound to a DEP StableTableId, (3) fields merged via a DEP
//! TableExtension. The corpus carries ≥2 dep tables (Dep Customer 50000 + Dep
//! Vendor 50001) so a wrong-but-same binding is detectable. An oracle cross-check
//! asserts the Rust matrix equals the al-sem GOLDEN matrix (ground truth).
//!
//! ## Divergence gating
//!
//! Direct strict comparison — the test fails on ANY divergence between the Rust
//! projection and the al-sem golden. No allowlist.

use std::path::PathBuf;

use al_call_hierarchy::engine::deps::cross_app_l3::build_cross_app_l3_from_workspace;
use serde_json::Value;

const R2_5B_MODEL_INSTANCE_ID: &str = "r2.5b";

/// The dep app guids — a StableTableId / declaringObjectId starting with one of
/// these is a cross-app (dep-origin) binding/field. Mirrors the al-sem capture's
/// `G.core` / `G.other`.
const DEP_CORE: &str = "dddddddd-0000-0000-0000-000000000001";
const DEP_OTHER: &str = "eeeeeeee-0000-0000-0000-000000000002";

/// Keys that must NEVER appear on either side of the record-type comparison —
/// later-gate / L4 surfaces. Mirrors the R2a `L3_FORBIDDEN_KEYS` + the manifest's
/// `forbiddenKeys`.
const R2_5B_RT_FORBIDDEN_KEYS: &[&str] = &[
    "callGraph",
    "eventGraph",
    "coverage",
    "typedEdges",
    "resourceId",
    "bindingResolution",
    "argumentBindings",
    "summary",
    "capabilityFactsDirect",
];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn goldens_dir() -> PathBuf {
    repo_root().join("tests").join("r2-5b-rt-goldens")
}

fn fixtures_dir() -> PathBuf {
    repo_root().join("tests").join("r2-5b-fixtures")
}

/// A single divergence (golden vs rust), with a stable JSON-pointer-ish locator.
#[derive(Debug, Clone)]
struct Divergence {
    fixture: String,
    path: String,
    golden_value: String,
    rust_value: String,
}

/// Discover every `tests/r2-5b-rt-goldens/*.r2.5b-rt.golden.json`, sorted by
/// fixture name (skips manifest.json).
fn discover_goldens() -> Vec<(String, PathBuf)> {
    let dir = goldens_dir();
    let mut out = Vec::new();
    let entries = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("failed to read R2.5b-rt goldens dir {}: {e}", dir.display()));
    for entry in entries {
        let entry = entry.expect("dir entry");
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.ends_with(".r2.5b-rt.golden.json") {
            continue;
        }
        let fixture = name.trim_end_matches(".r2.5b-rt.golden.json").to_string();
        out.push((fixture, entry.path()));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// The three cross-app anti-degenerate axes.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct CrossAppCoverage {
    dep_bound_record_vars: usize,
    dep_bound_record_ops: usize,
    dep_extension_merged_fields: usize,
}

impl CrossAppCoverage {
    fn add(&mut self, other: &CrossAppCoverage) {
        self.dep_bound_record_vars += other.dep_bound_record_vars;
        self.dep_bound_record_ops += other.dep_bound_record_ops;
        self.dep_extension_merged_fields += other.dep_extension_merged_fields;
    }
}

/// True iff the StableTableId / StableObjectId is owned by a dep app.
fn is_dep_owned(stable_id: &str) -> bool {
    stable_id.starts_with(&format!("{DEP_CORE}:"))
        || stable_id.starts_with(&format!("{DEP_OTHER}:"))
}

/// Compute the cross-app coverage matrix for ONE projection `Value` (golden OR
/// rust). The shapes are identical, so the same walker serves both.
fn coverage_of(proj: &Value) -> CrossAppCoverage {
    let mut m = CrossAppCoverage::default();
    if let Some(tables) = proj.get("tables").and_then(|t| t.as_array()) {
        for t in tables {
            if let Some(fields) = t.get("fields").and_then(|f| f.as_array()) {
                for f in fields {
                    if let Some(d) = f.get("declaringObjectId").and_then(|d| d.as_str())
                        && d.contains(":TableExtension:")
                        && is_dep_owned(d)
                    {
                        m.dep_extension_merged_fields += 1;
                    }
                }
            }
        }
    }
    if let Some(routines) = proj.get("routines").and_then(|r| r.as_array()) {
        for r in routines {
            if let Some(vars) = r.get("recordVariables").and_then(|v| v.as_array()) {
                for v in vars {
                    if let Some(t) = v.get("tableId").and_then(|t| t.as_str())
                        && is_dep_owned(t)
                    {
                        m.dep_bound_record_vars += 1;
                    }
                }
            }
            if let Some(ops) = r.get("recordOperations").and_then(|o| o.as_array()) {
                for o in ops {
                    if let Some(t) = o.get("tableId").and_then(|t| t.as_str())
                        && is_dep_owned(t)
                    {
                        m.dep_bound_record_ops += 1;
                    }
                }
            }
        }
    }
    m
}

/// Recursively collect every forbidden object-key in `value`, with its JSON
/// pointer path.
fn scan_forbidden(value: &Value, path: &str, hits: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (k, v) in map {
                let child = format!("{path}.{k}");
                if R2_5B_RT_FORBIDDEN_KEYS.contains(&k.as_str()) {
                    hits.push(child.clone());
                }
                scan_forbidden(v, &child, hits);
            }
        }
        Value::Array(arr) => {
            for (i, v) in arr.iter().enumerate() {
                scan_forbidden(v, &format!("{path}[{i}]"), hits);
            }
        }
        _ => {}
    }
}

/// Recursively diff two projection values POSITIONALLY (both sides are already
/// canonically sorted by the projection), emitting a `Divergence` per leaf /
/// shape / missing-or-extra mismatch.
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

/// The R2.5b-a cross-app record-types differential pass + cross-app coverage
/// matrix. The corpus is the single committed `.app`-bearing fixture; both sides
/// read byte-identical `.app`s.
#[test]
fn differential_r2_5b_record_types_match_goldens() {
    let goldens = discover_goldens();
    assert!(
        !goldens.is_empty(),
        "no R2.5b-rt goldens discovered under {} — corpus missing?",
        goldens_dir().display()
    );

    let mut all_divergences: Vec<Divergence> = Vec::new();
    let mut forbidden_hits: Vec<String> = Vec::new();
    let mut rust_cov = CrossAppCoverage::default();
    let mut golden_cov = CrossAppCoverage::default();

    for (fixture, golden_path) in &goldens {
        let fixture_dir = fixtures_dir().join(fixture);
        assert!(
            fixture_dir.is_dir(),
            "R2.5b-rt golden {} has no matching `.app`-bearing fixture at {} (offline corpus incomplete)",
            golden_path.display(),
            fixture_dir.display()
        );

        // Golden side: parse as JSON.
        let golden_text = std::fs::read_to_string(golden_path)
            .unwrap_or_else(|e| panic!("read R2.5b-rt golden {}: {e}", golden_path.display()));
        let golden_json: Value = serde_json::from_str(&golden_text).unwrap_or_else(|e| {
            panic!(
                "R2.5b-rt golden {} is not valid JSON: {e}",
                golden_path.display()
            )
        });

        // Rust side: cross-app assemble+resolve → project_record_types → JSON. The
        // cross-app build reads the workspace `.al` source + the `.alpackages/*.app`
        // deps; fail-closed layouts yield an empty projection (never throws).
        let projection =
            match build_cross_app_l3_from_workspace(&fixture_dir, R2_5B_MODEL_INSTANCE_ID) {
                Some(cross) => cross.project_record_types(),
                None => al_call_hierarchy::engine::l3::l3_workspace::L3RecordTypeProjection {
                    tables: vec![],
                    routines: vec![],
                },
            };
        let rust_json = serde_json::to_value(&projection)
            .unwrap_or_else(|e| panic!("serialize Rust R2.5b-rt projection for {fixture}: {e}"));

        // REGEN path (temp-state epoch rebaseline, Task 16). When
        // `REGEN_TEMP_GOLDENS` is set, write the ENGINE projection to the golden
        // file (matching the on-disk pretty form) instead of comparing — the
        // goldens are Rust-owned baselines (TS oracle retired).
        if std::env::var("REGEN_TEMP_GOLDENS").is_ok() {
            let mut pretty = serde_json::to_string_pretty(&projection)
                .unwrap_or_else(|e| panic!("regen serialize R2.5b-rt {fixture}: {e}"));
            pretty.push('\n');
            std::fs::write(golden_path, pretty)
                .unwrap_or_else(|e| panic!("regen write {}: {e}", golden_path.display()));
            eprintln!("REGEN r2.5b-rt golden: {}", golden_path.display());
            continue;
        }

        // Forbidden later-gate / L4 field scan on BOTH sides.
        scan_forbidden(
            &golden_json,
            &format!("{fixture}:golden"),
            &mut forbidden_hits,
        );
        scan_forbidden(&rust_json, &format!("{fixture}:rust"), &mut forbidden_hits);

        // Cross-app coverage matrices (Rust drives the anti-degenerate gate; golden
        // is the oracle cross-check).
        rust_cov.add(&coverage_of(&rust_json));
        golden_cov.add(&coverage_of(&golden_json));

        // Positional structural diff (both sides already canonically sorted).
        diff_value(fixture, "", &golden_json, &rust_json, &mut all_divergences);
    }

    // REGEN mode wrote every golden above and asserts nothing.
    if std::env::var("REGEN_TEMP_GOLDENS").is_ok() {
        eprintln!("REGEN r2.5b-rt: wrote {} golden(s)", goldens.len());
        return;
    }

    all_divergences
        .sort_by(|a, b| (a.fixture.as_str(), &a.path).cmp(&(b.fixture.as_str(), &b.path)));

    // --- Forbidden-field guard (hard fail, never allowlistable) -------------
    assert!(
        forbidden_hits.is_empty(),
        "FORBIDDEN later-gate/L4 field(s) leaked into the R2.5b-rt comparison \
         (golden or rust):\n  {}",
        forbidden_hits.join("\n  ")
    );

    // --- CROSS-APP coverage matrix gate (fail-on-zero, Rev 2 #1) ------------
    eprintln!(
        "R2.5b-a cross-app coverage matrix ({} fixture(s)): \
         depBoundRecordVars={} depBoundRecordOps={} depExtensionMergedFields={}",
        goldens.len(),
        rust_cov.dep_bound_record_vars,
        rust_cov.dep_bound_record_ops,
        rust_cov.dep_extension_merged_fields,
    );
    let mut zero_axes: Vec<&str> = Vec::new();
    if rust_cov.dep_bound_record_vars == 0 {
        zero_axes.push("depBoundRecordVars");
    }
    if rust_cov.dep_bound_record_ops == 0 {
        zero_axes.push("depBoundRecordOps");
    }
    if rust_cov.dep_extension_merged_fields == 0 {
        zero_axes.push("depExtensionMergedFields");
    }
    assert!(
        zero_axes.is_empty(),
        "DEGENERATE cross-app coverage matrix: axis/axes {zero_axes:?} are ZERO — \
         the cross-app L3 record-type resolution is NOT firing (a dep table must bind, \
         a dep-extension field must merge). Source-only `opaque==opaque` green is a FAILURE here.",
    );
    // Oracle cross-check: Rust matrix == GOLDEN matrix (al-sem ground truth).
    assert_eq!(
        rust_cov, golden_cov,
        "R2.5b-a cross-app coverage matrix MISMATCH vs golden oracle\n  rust   = {rust_cov:?}\n  golden = {golden_cov:?}",
    );

    // --- Strict divergence gate (direct assert; no allowlist) --------------
    let mut failure = String::new();
    if !all_divergences.is_empty() {
        failure.push_str(&format!(
            "\n{} R2.5b-rt divergence(s) found:\n",
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
        "R2.5b-a cross-app record-types differential FAILED:{failure}"
    );

    eprintln!(
        "R2.5b-a record-types differential: {} fixture(s), 0 divergences.",
        goldens.len(),
    );
}
