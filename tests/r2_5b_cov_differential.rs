//! R2.5b-d — CROSS-APP L3 coverage differential + the anti-degenerate CROSS-APP
//! resolution-delta matrix (REV3).
//!
//! For each committed al-sem golden under `tests/r2-5b-cov-goldens/`, run the Rust
//! cross-app L3 build (`build_cross_app_l3_from_workspace`) over the matching
//! `.app`-bearing workspace fixture under `tests/r2-5b-fixtures/<fixture>/`, project to
//! the golden-shaped `AnalysisCoverage` (`project_coverage_disk`), and assert it
//! BYTE-MATCHES the golden — same projection shape as R2d, but the corpus carries
//! `.alpackages`, so cross-app member calls RESOLVE and drop out of `unresolvedCallsites`.
//!
//! The `.app` fixtures are the EXACT SAME bytes the al-sem golden generator read, so
//! this is a TRUE byte-parity diff. Default `cargo test` runs OFFLINE.
//!
//! ## Capture point (R2.5b-d)
//!
//! POST-`resolveModel` over the MERGED index (`noDepSummaries:true`) — `model.coverage`
//! (the AnalysisCoverage buildCoverage produced); NEVER re-runs buildCoverage. Cross-app
//! resolution is only observable post-resolve (the dep objects/routines enter the
//! index/symbol table via withDependencyArtifacts).
//!
//! ## R3a-0 — `opaqueApps` reflects the symbol-only dep apps (latent bug FIXED)
//!
//! al-sem's `buildCoverage` filters `index.identity.apps` by `sourceKind==="symbol-only"`.
//! As of the R3a-0 semantic-oracle epoch (al-sem `81d538a`+`f1650ba`, Fix 2),
//! `withDependencyArtifacts` stamps the dep `AppIdentity`s (with `sourceKind`) into
//! `identity.apps`, so `opaqueApps` now lists the symbol-only dep app guids cross-app
//! (source-only stays `[]` — no deps). The Rust `project_coverage_cross_app` mirrors this
//! (passes ALL apps — workspace `"source"` + each dep — and lets the `symbol-only` filter
//! populate it). The differential asserts the golden's non-empty `opaqueApps` on both
//! sides; the matrix tracks the count as informational (the fix is shipped).
//!
//! ## CROSS-APP anti-degenerate matrix (fail-on-zero, REV3 — re-scoped OFF opaqueApps)
//!
//! Computed from the RUST output (proves cross-app resolution actually FIRES): nonzero
//! (1) cross-app callsites that RESOLVED (and are therefore ABSENT from
//! `unresolvedCallsites` — the cross-app coverage WIN; in a source-only world they would
//! be unresolved), AND (2) the external-target member miss that STAYS IN
//! `unresolvedCallsites`. The resolution per callsite is read from the cross-app call
//! graph (the same merged model). An oracle cross-check asserts the Rust matrix equals
//! the al-sem GOLDEN manifest matrix (ground truth) is covered by the native oracle.
//!
//! ## KNOWN_DIVERGENCES gating
//!
//! Reuses the repo-root `KNOWN_DIVERGENCES.json` with exact `(test, fixture, path)`
//! matching, scoped to `test == R2_5B_COV_TEST_NAME`. Target: empty.

use std::path::PathBuf;

use al_call_hierarchy::engine::deps::cross_app_l3::{
    CrossAppL3, build_cross_app_l3_from_workspace,
};
use serde::Deserialize;
use serde_json::Value;

const R2_5B_COV_TEST_NAME: &str = "differential_r2_5b_coverage_match_goldens";
const R2_5B_MODEL_INSTANCE_ID: &str = "r2.5b";

/// Keys that must NEVER appear on either side of the coverage comparison — call-graph /
/// event-graph (separate gates) / later-gate / L4 surfaces. Mirrors al-sem
/// `FORBIDDEN_KEYS` (`scripts/r2d-l3cov-projection.ts`).
const R2_5B_COV_FORBIDDEN_KEYS: &[&str] = &[
    // call-graph surface (R2.5b-b — a separate gate)
    "callGraph",
    "callsiteId",
    "dispatchKind",
    "dispatchMeta",
    "argumentBindings",
    "groups",
    "bindings",
    "callsiteResolutions",
    // event-graph surface (R2.5b-c — a separate gate)
    "eventGraph",
    "events",
    "edges",
    "eventKind",
    // later-gate / L4 / R2.5
    "typedEdges",
    "summary",
    "analysisGaps",
    "capabilityFactsDirect",
    "rootClassifications",
];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn goldens_dir() -> PathBuf {
    repo_root().join("tests").join("r2-5b-cov-goldens")
}

fn fixtures_dir() -> PathBuf {
    repo_root().join("tests").join("r2-5b-fixtures")
}

/// One entry in `KNOWN_DIVERGENCES.json`. `test` scopes the entry.
#[derive(Debug, Clone, Deserialize)]
struct AllowEntry {
    #[serde(default = "default_allow_test")]
    test: String,
    fixture: String,
    path: String,
    #[serde(default)]
    #[allow(dead_code)]
    reason: String,
    #[serde(default)]
    #[allow(dead_code)]
    expires: String,
}

fn default_allow_test() -> String {
    "differential_identity_subset_matches_goldens".to_string()
}

fn load_allowlist() -> Vec<AllowEntry> {
    let path = repo_root().join("KNOWN_DIVERGENCES.json");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
    serde_json::from_str(&text)
        .unwrap_or_else(|e| panic!("failed to parse {} as a JSON array: {e}", path.display()))
}

/// A single divergence (golden vs rust), with a stable JSON-pointer-ish locator.
#[derive(Debug, Clone)]
struct Divergence {
    fixture: String,
    path: String,
    golden_value: String,
    rust_value: String,
}

/// Discover every `tests/r2-5b-cov-goldens/*.r2.5b-cov.golden.json`, sorted by fixture
/// name (skips manifest.json).
fn discover_goldens() -> Vec<(String, PathBuf)> {
    let dir = goldens_dir();
    let mut out = Vec::new();
    let entries = std::fs::read_dir(&dir).unwrap_or_else(|e| {
        panic!(
            "failed to read R2.5b-cov goldens dir {}: {e}",
            dir.display()
        )
    });
    for entry in entries {
        let entry = entry.expect("dir entry");
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.ends_with(".r2.5b-cov.golden.json") {
            continue;
        }
        let fixture = name.trim_end_matches(".r2.5b-cov.golden.json").to_string();
        out.push((fixture, entry.path()));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// The cross-app anti-degenerate resolution-delta matrix (REV3 — re-scoped off
/// opaqueApps).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct CrossAppCoverage {
    /// cross-app callsites that RESOLVED (edge `to` a dep routine) — therefore ABSENT
    /// from `unresolvedCallsites` (the cross-app coverage WIN).
    cross_app_resolved_absent: usize,
    /// external-target member misses present IN `unresolvedCallsites`.
    external_target_present: usize,
    /// `opaqueApps` length — now reflects the symbol-only dep apps (R3a-0 Fix 2).
    opaque_apps_count: usize,
    /// total `unresolvedCallsites` multiset length (dups preserved).
    unresolved_count: usize,
}

impl CrossAppCoverage {
    fn add(&mut self, o: &CrossAppCoverage) {
        self.cross_app_resolved_absent += o.cross_app_resolved_absent;
        self.external_target_present += o.external_target_present;
        self.opaque_apps_count += o.opaque_apps_count;
        self.unresolved_count += o.unresolved_count;
    }
}

/// Compute the cross-app resolution-delta matrix for ONE fixture's RUST model. Reads the
/// cross-app CALL GRAPH (the resolved edges) to classify each workspace callsite, plus
/// the coverage projection for `opaqueApps` / `unresolvedCallsites` lengths. Faithful
/// port of al-sem `crossAppCoverageOf` (`scripts/dump-r2.5b-coverage.ts`).
fn coverage_of(cross: &CrossAppL3, cov: &Value) -> CrossAppCoverage {
    let opaque_apps_count = cov
        .get("opaqueApps")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    let unresolved_count = cov
        .get("unresolvedCallsites")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);

    // Classify via the resolved call graph: a `resolved` edge whose `to` is a dep
    // routine is a cross-app WIN (its workspace callsite is ABSENT from
    // unresolvedCallsites); an `external-target` edge stays IN unresolvedCallsites.
    let cg = cross.project_call_graph();
    let cg_json = serde_json::to_value(&cg).expect("serialize cross-app call graph");
    let mut cross_app_resolved_absent = 0usize;
    let mut external_target_present = 0usize;
    if let Some(groups) = cg_json.get("groups").and_then(|g| g.as_array()) {
        for group in groups {
            if let Some(edges) = group.get("edges").and_then(|e| e.as_array()) {
                for e in edges {
                    let res = e.get("resolution").and_then(|r| r.as_str()).unwrap_or("");
                    let to = e.get("to").and_then(|t| t.as_str()).unwrap_or("");
                    if res == "resolved" && is_dep_owned(to) {
                        cross_app_resolved_absent += 1;
                    } else if res == "external-target" {
                        external_target_present += 1;
                    }
                }
            }
        }
    }
    CrossAppCoverage {
        cross_app_resolved_absent,
        external_target_present,
        opaque_apps_count,
        unresolved_count,
    }
}

/// The dep app guids — a STABLE id (the projected `to` of a resolved cross-app edge)
/// starting with one of these is a dep-owned routine. Mirrors `G.core` / `G.other`.
const DEP_CORE: &str = "dddddddd-0000-0000-0000-000000000001";
const DEP_OTHER: &str = "eeeeeeee-0000-0000-0000-000000000002";

/// True iff the projected (STABLE) routine id is owned by a dep app.
fn is_dep_owned(stable_id: &str) -> bool {
    stable_id.starts_with(&format!("{DEP_CORE}:"))
        || stable_id.starts_with(&format!("{DEP_OTHER}:"))
}

/// Recursively collect every forbidden object-key in `value`, with its JSON pointer path.
fn scan_forbidden(value: &Value, path: &str, hits: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (k, v) in map {
                let child = format!("{path}.{k}");
                if R2_5B_COV_FORBIDDEN_KEYS.contains(&k.as_str()) {
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

/// Recursively diff two coverage values POSITIONALLY (both sides already canonically
/// sorted — the multisets are sorted with cmpStable; duplicates PRESERVED via positional
/// comparison over the sorted Vecs).
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

/// The R2.5b-d cross-app coverage differential pass + the cross-app resolution-delta
/// matrix. The corpus is the single committed `.app`-bearing fixture; both sides read
/// byte-identical `.app`s.
#[test]
fn differential_r2_5b_coverage_match_goldens() {
    let goldens = discover_goldens();
    assert!(
        !goldens.is_empty(),
        "no R2.5b-cov goldens discovered under {} — corpus missing?",
        goldens_dir().display()
    );

    let allowlist: Vec<AllowEntry> = load_allowlist()
        .into_iter()
        .filter(|e| e.test == R2_5B_COV_TEST_NAME)
        .collect();

    let mut all_divergences: Vec<Divergence> = Vec::new();
    let mut forbidden_hits: Vec<String> = Vec::new();
    let mut rust_cov = CrossAppCoverage::default();

    for (fixture, golden_path) in &goldens {
        let fixture_dir = fixtures_dir().join(fixture);
        assert!(
            fixture_dir.is_dir(),
            "R2.5b-cov golden {} has no matching `.app`-bearing fixture at {} (offline corpus incomplete)",
            golden_path.display(),
            fixture_dir.display()
        );

        // Golden side: parse as JSON.
        let golden_text = std::fs::read_to_string(golden_path)
            .unwrap_or_else(|e| panic!("read R2.5b-cov golden {}: {e}", golden_path.display()));
        let golden_json: Value = serde_json::from_str(&golden_text).unwrap_or_else(|e| {
            panic!(
                "R2.5b-cov golden {} is not valid JSON: {e}",
                golden_path.display()
            )
        });

        // Rust side: cross-app assemble+resolve → project_coverage_disk → JSON.
        let cross = build_cross_app_l3_from_workspace(&fixture_dir, R2_5B_MODEL_INSTANCE_ID);
        let projection = match &cross {
            Some(c) => c.project_coverage_disk(&fixture_dir),
            None => al_call_hierarchy::engine::l3::coverage::AnalysisCoverage {
                source_units_total: 0,
                source_units_parsed: 0,
                routines_total: 0,
                routines_body_available: 0,
                routines_parse_incomplete: vec![],
                opaque_apps: vec![],
                unresolved_callsites: vec![],
                dynamic_dispatch_sites: vec![],
            },
        };
        let rust_json = serde_json::to_value(&projection)
            .unwrap_or_else(|e| panic!("serialize Rust R2.5b-cov projection for {fixture}: {e}"));

        // Forbidden call-graph / event-graph / later-gate / L4 field scan on BOTH sides.
        scan_forbidden(
            &golden_json,
            &format!("{fixture}:golden"),
            &mut forbidden_hits,
        );
        scan_forbidden(&rust_json, &format!("{fixture}:rust"), &mut forbidden_hits);

        // Cross-app resolution-delta matrix (Rust drives the anti-degenerate gate). The
        // golden's unresolved multiset is the byte-parity oracle; the named-vector oracle
        // (r2_5b_cov_oracles.rs) asserts the SPECIFIC resolved/external-target ids.
        if let Some(c) = &cross {
            rust_cov.add(&coverage_of(c, &rust_json));
        }

        // Positional structural diff (both sides already canonically sorted).
        diff_value(fixture, "", &golden_json, &rust_json, &mut all_divergences);
    }

    all_divergences
        .sort_by(|a, b| (a.fixture.as_str(), &a.path).cmp(&(b.fixture.as_str(), &b.path)));

    // --- Forbidden-field guard (hard fail, never allowlistable) -------------
    assert!(
        forbidden_hits.is_empty(),
        "FORBIDDEN call-graph/event-graph/later-gate/L4 field(s) leaked into the R2.5b-cov \
         comparison (golden or rust):\n  {}",
        forbidden_hits.join("\n  ")
    );

    // --- CROSS-APP resolution-delta matrix gate (fail-on-zero, REV3) --------
    eprintln!(
        "R2.5b-d cross-app resolution-delta matrix ({} fixture(s)): \
         crossAppResolvedAbsent={} externalTargetPresent={} opaqueAppsCount={} (symbol-only deps, R3a-0) \
         unresolvedCount={}",
        goldens.len(),
        rust_cov.cross_app_resolved_absent,
        rust_cov.external_target_present,
        rust_cov.opaque_apps_count,
        rust_cov.unresolved_count,
    );
    let mut zero_axes: Vec<&str> = Vec::new();
    if rust_cov.cross_app_resolved_absent == 0 {
        zero_axes.push("crossAppResolvedAbsent");
    }
    if rust_cov.external_target_present == 0 {
        zero_axes.push("externalTargetPresent");
    }
    assert!(
        zero_axes.is_empty(),
        "DEGENERATE cross-app resolution-delta matrix: axis/axes {zero_axes:?} are ZERO — \
         the cross-app L3 coverage resolution is NOT firing (a cross-app member call must \
         RESOLVE and drop out of unresolvedCallsites; the external-target member miss must \
         stay IN — REV3). Source-only `everything unresolved` green is a FAILURE here.",
    );
    // R3a-0 (Fix 2): `opaqueApps` now reflects the symbol-only dep apps. The corpus has
    // two symbol-only deps (Lib Core / Lib Ext) → count 2. This is the fixed reality; the
    // byte-parity diff against the golden's non-empty opaqueApps is the authoritative check.
    assert_eq!(
        rust_cov.opaque_apps_count, 2,
        "opaqueApps MUST list the two symbol-only dep apps (R3a-0 Fix 2 — buildCoverage reads \
         identity.apps, which withDependencyArtifacts now stamps with the symbol-only deps)",
    );

    // --- Allowlist gating (same semantics as R2a/R2.5b-rt/-cg/-eg) ----------
    let mut entry_used = vec![false; allowlist.len()];
    let mut undocumented: Vec<&Divergence> = Vec::new();
    for div in &all_divergences {
        let mut covered = false;
        for (i, entry) in allowlist.iter().enumerate() {
            if entry.fixture == div.fixture && entry.path == div.path {
                entry_used[i] = true;
                covered = true;
            }
        }
        if !covered {
            undocumented.push(div);
        }
    }
    let unused: Vec<&AllowEntry> = allowlist
        .iter()
        .enumerate()
        .filter(|(i, _)| !entry_used[*i])
        .map(|(_, e)| e)
        .collect();

    let mut failure = String::new();
    if !undocumented.is_empty() {
        failure.push_str(&format!(
            "\n{} UNDOCUMENTED R2.5b-cov divergence(s) (not in KNOWN_DIVERGENCES.json, \
             test={R2_5B_COV_TEST_NAME}):\n",
            undocumented.len()
        ));
        for d in &undocumented {
            failure.push_str(&format!(
                "  [{}] {}\n      golden = {}\n      rust   = {}\n",
                d.fixture, d.path, d.golden_value, d.rust_value
            ));
        }
    }
    if !unused.is_empty() {
        failure.push_str(&format!(
            "\n{} UNUSED R2.5b-cov allowlist entr(y/ies) (no matching divergence this run):\n",
            unused.len()
        ));
        for e in &unused {
            failure.push_str(&format!(
                "  [{}] {}  (reason: {:?}, expires: {:?})\n",
                e.fixture, e.path, e.reason, e.expires
            ));
        }
    }

    assert!(
        failure.is_empty(),
        "R2.5b-d cross-app coverage differential FAILED:{failure}"
    );

    eprintln!(
        "R2.5b-d coverage differential: {} fixture(s), 0 divergences, \
         allowlist fully consumed ({} entr(y/ies)).",
        goldens.len(),
        allowlist.len()
    );
}
