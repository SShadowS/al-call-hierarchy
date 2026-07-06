//! R2.5b-b — CROSS-APP L3 call-graph differential + the anti-degenerate CROSS-APP
//! transition matrix.
//!
//! For each committed al-sem golden under `tests/r2-5b-cg-goldens/`, run the Rust
//! cross-app L3 build (`build_cross_app_l3_from_workspace`) over the matching
//! `.app`-bearing workspace fixture under `tests/r2-5b-fixtures/<fixture>/`
//! (workspace `.al` source + `.alpackages/*.app` deps), project to the
//! golden-shaped call-graph projection (`project_call_graph`), and assert it
//! BYTE-MATCHES the golden — same projection shape as R2b, but the corpus carries
//! `.alpackages`, so a member call to a PRESENT dep object + name/arity match
//! resolves to the dep StableRoutineId.
//!
//! The `.app` fixtures are the EXACT SAME bytes the al-sem golden generator read
//! (the committed `test/fixtures/r2.5b-deps/` copied into the fixture's
//! `.alpackages/`), so this is a TRUE byte-parity diff. Default `cargo test` runs
//! OFFLINE: everything is committed in-repo.
//!
//! ## Capture point (R2.5b-b)
//!
//! POST-`resolveModel` over the MERGED index (`noDepSummaries:true`) — the resolved
//! `CallEdge[]` + the in-place-UPGRADED `argumentBindings` (Rev 2 #3 anti-stale).
//! modelInstanceId pinned to `r2.5b` on BOTH sides (the operationId/callsiteId carry
//! that prefix; the StableRoutineId keys are modelInstanceId-independent).
//!
//! ## The opaque-vs-external-target split (R3a-0 — real ledger threaded)
//!
//! As of R3a-0 (al-sem `81d538a` Fix 1 + capture fix `93e360d`), both PRODUCTION
//! `analyzeWorkspace` AND the R2.5b capture harness stamp `primaryDependencies` BEFORE
//! `resolveModel`, so the resolver reads the REAL declared deps DURING resolution.
//! `project_call_graph_cross_app` threads the real declared/fetched ledger to mirror this.
//!
//! On the ALL-FETCHED corpus (declared = fetched = {Lib Core, Lib Ext}; the prior
//! `Lib Absent` unfetched dep was removed in `93e360d`) this is BYTE-INVARIANT:
//! `has_unfetched_declared_dependency` is false, so the `gone.M()` member miss into an
//! absent object is `external-target` GENUINELY (preserving the external-target axis), and
//! the ONLY genuine cross-app `opaque` is the ledger-independent OBJECT-RUN form
//! (`Codeunit.Run("Absent Dep Cu")`). The unfetched-declared-dep member-`opaque` branch
//! (Fix 1) is proven out-of-corpus by `tests/r3a0_unfetched_dep_opaque.rs`.
//!
//! ## CROSS-APP anti-degenerate matrix (fail-on-zero, Rev 2 #1/#4)
//!
//! Computed from the RUST output (proves cross-app resolution actually FIRES, not
//! "opaque == opaque"): nonzero (1) edges `resolved` to a DEP StableRoutineId, AND
//! each named transition present: (2) member-not-found, (3) opaque (object-run), (4)
//! external-target, (5) ≥1 upgraded `resolved` argumentBinding. The corpus carries
//! ≥2 dep routines (Dep Mgt Compute/InternalReset/LocalHelper/Recalc/Apply) so a
//! wrong-but-same binding is detectable. An oracle cross-check asserts the Rust
//! matrix equals the al-sem GOLDEN matrix (ground truth).
//!
//! ## Strict comparison
//!
//! The golden and the Rust projection are compared directly: any divergence is a
//! hard failure, with no tolerance mechanism of any kind.

use std::path::PathBuf;

use al_call_hierarchy::engine::deps::cross_app_l3::build_cross_app_l3_from_workspace;
use serde_json::Value;

const R2_5B_MODEL_INSTANCE_ID: &str = "r2.5b";

/// The dep app guids — a StableRoutineId starting with one of these is a cross-app
/// (dep-origin) call target. Mirrors the al-sem capture's `G.core` / `G.other`.
const DEP_CORE: &str = "dddddddd-0000-0000-0000-000000000001";
const DEP_OTHER: &str = "eeeeeeee-0000-0000-0000-000000000002";

/// Keys that must NEVER appear on either side of the call-graph comparison —
/// later-gate / L4 surfaces. Mirrors al-sem `FORBIDDEN_KEYS` + the manifest's
/// `forbiddenKeys`.
const R2_5B_CG_FORBIDDEN_KEYS: &[&str] = &[
    "typedEdges",
    "summary",
    "coverage",
    "eventGraph",
    "callsiteResolutions",
    "openWorld",
    "capabilityFactsDirect",
    "rootClassifications",
];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn goldens_dir() -> PathBuf {
    repo_root().join("tests").join("r2-5b-cg-goldens")
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

/// Discover every `tests/r2-5b-cg-goldens/*.r2.5b-cg.golden.json`, sorted by
/// fixture name (skips manifest.json).
fn discover_goldens() -> Vec<(String, PathBuf)> {
    let dir = goldens_dir();
    let mut out = Vec::new();
    let entries = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("failed to read R2.5b-cg goldens dir {}: {e}", dir.display()));
    for entry in entries {
        let entry = entry.expect("dir entry");
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.ends_with(".r2.5b-cg.golden.json") {
            continue;
        }
        let fixture = name.trim_end_matches(".r2.5b-cg.golden.json").to_string();
        out.push((fixture, entry.path()));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// The cross-app anti-degenerate transition matrix (Rev 2 #1/#4).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct CrossAppCoverage {
    /// edges with `resolution == "resolved"` AND `to` = a DEP StableRoutineId.
    resolved_to_dep_routine: usize,
    /// member-not-found edges (present dep object, missing member).
    member_not_found: usize,
    /// opaque edges (the object-run form into an absent declared dep).
    opaque: usize,
    /// external-target edges (member call, object absent, all deps fetched).
    external_target: usize,
    /// upgraded argumentBindings with bindingResolution "resolved" (anti-stale).
    upgraded_resolved_bindings: usize,
}

impl CrossAppCoverage {
    fn add(&mut self, o: &CrossAppCoverage) {
        self.resolved_to_dep_routine += o.resolved_to_dep_routine;
        self.member_not_found += o.member_not_found;
        self.opaque += o.opaque;
        self.external_target += o.external_target;
        self.upgraded_resolved_bindings += o.upgraded_resolved_bindings;
    }
}

/// True iff the StableRoutineId is owned by a dep app.
fn is_dep_owned(stable_id: &str) -> bool {
    stable_id.starts_with(&format!("{DEP_CORE}:"))
        || stable_id.starts_with(&format!("{DEP_OTHER}:"))
}

/// Compute the cross-app transition matrix for ONE projection `Value` (golden OR
/// rust). The shapes are identical, so the same walker serves both. Faithful port
/// of al-sem `crossAppCoverageOf` (`scripts/dump-r2.5b-call-graph.ts`).
fn coverage_of(proj: &Value) -> CrossAppCoverage {
    let mut m = CrossAppCoverage::default();
    let str_of = |v: &Value, k: &str| -> String {
        v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string()
    };
    if let Some(groups) = proj.get("groups").and_then(|g| g.as_array()) {
        for g in groups {
            if let Some(edges) = g.get("edges").and_then(|e| e.as_array()) {
                for e in edges {
                    let res = str_of(e, "resolution");
                    let to = e.get("to").and_then(|t| t.as_str());
                    if res == "resolved" && to.map(is_dep_owned).unwrap_or(false) {
                        m.resolved_to_dep_routine += 1;
                    } else if res == "member-not-found" {
                        m.member_not_found += 1;
                    } else if res == "opaque" {
                        m.opaque += 1;
                    } else if res == "external-target" {
                        m.external_target += 1;
                    }
                }
            }
        }
    }
    if let Some(bindings) = proj.get("bindings").and_then(|b| b.as_array()) {
        for site in bindings {
            if let Some(bs) = site.get("bindings").and_then(|b| b.as_array()) {
                for ab in bs {
                    if str_of(ab, "bindingResolution") == "resolved" {
                        m.upgraded_resolved_bindings += 1;
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
                if R2_5B_CG_FORBIDDEN_KEYS.contains(&k.as_str()) {
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
/// canonically sorted by the projection — groups by callsiteId, edges by sort key,
/// bindings by callsiteId). The edge MULTISET is preserved (Vec, never Set; no
/// dedup — Rev 2 #6): positional comparison over the sorted Vecs keeps duplicates.
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

/// The R2.5b-b cross-app call-graph differential pass + cross-app transition
/// matrix. The corpus is the single committed `.app`-bearing fixture; both sides
/// read byte-identical `.app`s.
#[test]
fn differential_r2_5b_call_graph_match_goldens() {
    let goldens = discover_goldens();
    assert!(
        !goldens.is_empty(),
        "no R2.5b-cg goldens discovered under {} — corpus missing?",
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
            "R2.5b-cg golden {} has no matching `.app`-bearing fixture at {} (offline corpus incomplete)",
            golden_path.display(),
            fixture_dir.display()
        );

        // Golden side: parse as JSON.
        let golden_text = std::fs::read_to_string(golden_path)
            .unwrap_or_else(|e| panic!("read R2.5b-cg golden {}: {e}", golden_path.display()));
        let golden_json: Value = serde_json::from_str(&golden_text).unwrap_or_else(|e| {
            panic!(
                "R2.5b-cg golden {} is not valid JSON: {e}",
                golden_path.display()
            )
        });

        // Rust side: cross-app assemble+resolve → project_call_graph → JSON. The
        // cross-app build reads the workspace `.al` source + the `.alpackages/*.app`
        // deps; fail-closed layouts yield an empty projection (never throws).
        let projection =
            match build_cross_app_l3_from_workspace(&fixture_dir, R2_5B_MODEL_INSTANCE_ID) {
                Some(cross) => cross.project_call_graph(),
                None => {
                    al_call_hierarchy::engine::l3::call_graph_projection::L3CallGraphProjection {
                        groups: vec![],
                        bindings: vec![],
                    }
                }
            };
        let rust_json = serde_json::to_value(&projection)
            .unwrap_or_else(|e| panic!("serialize Rust R2.5b-cg projection for {fixture}: {e}"));

        // REGEN path (mirrors `r2_5b_rt_differential.rs`). When `REGEN_TEMP_GOLDENS`
        // is set, write the ENGINE projection to the golden file (matching the
        // on-disk pretty form) instead of comparing — the goldens are Rust-owned
        // baselines (TS oracle retired).
        if std::env::var("REGEN_TEMP_GOLDENS").is_ok() {
            let mut pretty = serde_json::to_string_pretty(&projection)
                .unwrap_or_else(|e| panic!("regen serialize R2.5b-cg {fixture}: {e}"));
            pretty.push('\n');
            std::fs::write(golden_path, pretty)
                .unwrap_or_else(|e| panic!("regen write {}: {e}", golden_path.display()));
            eprintln!("REGEN r2.5b-cg golden: {}", golden_path.display());
            continue;
        }

        // Forbidden later-gate / L4 field scan on BOTH sides.
        scan_forbidden(
            &golden_json,
            &format!("{fixture}:golden"),
            &mut forbidden_hits,
        );
        scan_forbidden(&rust_json, &format!("{fixture}:rust"), &mut forbidden_hits);

        // Cross-app transition matrices (Rust drives the anti-degenerate gate;
        // golden is the oracle cross-check).
        rust_cov.add(&coverage_of(&rust_json));
        golden_cov.add(&coverage_of(&golden_json));

        // Positional structural diff (both sides already canonically sorted).
        diff_value(fixture, "", &golden_json, &rust_json, &mut all_divergences);
    }

    // REGEN mode wrote every golden above and asserts nothing.
    if std::env::var("REGEN_TEMP_GOLDENS").is_ok() {
        eprintln!("REGEN r2.5b-cg: wrote {} golden(s)", goldens.len());
        return;
    }

    all_divergences
        .sort_by(|a, b| (a.fixture.as_str(), &a.path).cmp(&(b.fixture.as_str(), &b.path)));

    // --- Forbidden-field guard (hard fail, never allowlistable) -------------
    assert!(
        forbidden_hits.is_empty(),
        "FORBIDDEN later-gate/L4 field(s) leaked into the R2.5b-cg comparison \
         (golden or rust):\n  {}",
        forbidden_hits.join("\n  ")
    );

    // --- CROSS-APP transition matrix gate (fail-on-zero, Rev 2 #1/#4) -------
    eprintln!(
        "R2.5b-b cross-app transition matrix ({} fixture(s)): \
         resolvedToDepRoutine={} memberNotFound={} opaque={} externalTarget={} \
         upgradedResolvedBindings={}",
        goldens.len(),
        rust_cov.resolved_to_dep_routine,
        rust_cov.member_not_found,
        rust_cov.opaque,
        rust_cov.external_target,
        rust_cov.upgraded_resolved_bindings,
    );
    let mut zero_axes: Vec<&str> = Vec::new();
    if rust_cov.resolved_to_dep_routine == 0 {
        zero_axes.push("resolvedToDepRoutine");
    }
    if rust_cov.member_not_found == 0 {
        zero_axes.push("memberNotFound");
    }
    if rust_cov.opaque == 0 {
        zero_axes.push("opaque");
    }
    if rust_cov.external_target == 0 {
        zero_axes.push("externalTarget");
    }
    if rust_cov.upgraded_resolved_bindings == 0 {
        zero_axes.push("upgradedResolvedBindings");
    }
    assert!(
        zero_axes.is_empty(),
        "DEGENERATE cross-app transition matrix: axis/axes {zero_axes:?} are ZERO — \
         the cross-app L3 call-graph resolution is NOT firing (a member call must resolve \
         to a dep StableRoutineId; the named transitions member-not-found / opaque / \
         external-target / a binding upgrade must each appear). Source-only `opaque==opaque` \
         green is a FAILURE here.",
    );
    // Oracle cross-check: Rust matrix == GOLDEN matrix (al-sem ground truth).
    assert_eq!(
        rust_cov, golden_cov,
        "R2.5b-b cross-app transition matrix MISMATCH vs golden oracle\n  rust   = {rust_cov:?}\n  golden = {golden_cov:?}",
    );

    // --- Strict divergence assert --------------------------------------------
    let mut failure = String::new();
    if !all_divergences.is_empty() {
        failure.push_str(&format!(
            "\n{} R2.5b-cg divergence(s) found:\n",
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
        "R2.5b-b cross-app call-graph differential FAILED:{failure}"
    );

    eprintln!(
        "R2.5b-b call-graph differential: {} fixture(s), 0 divergences.",
        goldens.len()
    );
}
