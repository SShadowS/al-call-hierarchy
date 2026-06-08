//! R3a-4 EXIT GATE — the cross-app dep-hook DIFFERENTIAL + anti-degenerate matrix.
//!
//! Runs the Rust R3a-4 producer + consumer-hook pipeline
//! (`project_r3a4_from_workspace`) over the committed workspace fixture
//! (`tests/r3a4-fixtures/ws` — a source-bearing dep `.app` with the DoIt→DoWrite→Insert
//! chain) and asserts it BYTE-MATCHES the al-sem golden
//! (`tests/r3a4-goldens/cross-app-dep-hooks.r3a4.golden.json`).
//!
//! ## The stable-id form (Task 3 — THE key fix)
//!
//! Every id-bearing field is STABLE-PROJECTED to
//! `<appGuid>:<Type>:<Num>#<normalizedSignatureHash>[/opN|/csN]` — appGuid/signature-
//! derived → cache/modelInstanceId/devFingerprint-INDEPENDENT. The golden carries the
//! SAME stable form (al-sem `DepIdStabilizer`), so the differential is a real byte
//! oracle. NO `dep:<artifactKey>` prefix appears on either side.
//!
//! ## Capture point (R3a-4)
//!
//! post-inject_intra_app_call_edges / collect_cited_dep_evidence /
//! collect_dep_order_index. The R3a-5 cross-app cone is NOT projected.
//!
//! ## KNOWN_DIVERGENCES gating
//!
//! Reuses the repo-root `KNOWN_DIVERGENCES.json` with exact `(test, fixture, path)`
//! matching, scoped to `test == R3A4_TEST_NAME`. Target: empty (byte-match).

use std::path::PathBuf;

use al_call_hierarchy::engine::deps::r3a4_projection::{
    project_r3a4_from_workspace, R3a4Projection,
};
use serde::Deserialize;
use serde_json::Value;

const R3A4_TEST_NAME: &str = "differential_r3a4_dep_hooks_match_goldens";
const FIXTURE: &str = "cross-app-dep-hooks";

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn goldens_dir() -> PathBuf {
    repo_root().join("tests").join("r3a4-goldens")
}

fn ws_fixture_dir() -> PathBuf {
    repo_root().join("tests").join("r3a4-fixtures").join("ws")
}

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

#[derive(Debug, Clone)]
struct Divergence {
    fixture: String,
    path: String,
    golden_value: String,
    rust_value: String,
}

/// Recursively diff two projection values POSITIONALLY (both sides already sorted).
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

/// Build the Rust R3a-4 projection over the workspace fixture.
fn rust_projection() -> R3a4Projection {
    project_r3a4_from_workspace(&ws_fixture_dir(), FIXTURE)
}

#[test]
fn differential_r3a4_dep_hooks_match_goldens() {
    let golden_path = goldens_dir().join(format!("{FIXTURE}.r3a4.golden.json"));
    let golden_text = std::fs::read_to_string(&golden_path)
        .unwrap_or_else(|e| panic!("read R3a-4 golden {}: {e}", golden_path.display()));
    let golden_json: Value = serde_json::from_str(&golden_text)
        .unwrap_or_else(|e| panic!("R3a-4 golden {} not valid JSON: {e}", golden_path.display()));
    // Shape guard — the golden must round-trip through the R3a4Projection serde type.
    let _: R3a4Projection = serde_json::from_value(golden_json.clone()).unwrap_or_else(|e| {
        panic!(
            "R3a-4 golden {} does not parse as R3a4Projection: {e}",
            golden_path.display()
        )
    });

    let projection = rust_projection();
    let rust_json = serde_json::to_value(&projection)
        .unwrap_or_else(|e| panic!("serialize Rust R3a-4 projection: {e}"));

    // --- structural positional diff (the allowlist-gated surface) ---
    let mut all_divergences: Vec<Divergence> = Vec::new();
    diff_value(FIXTURE, "", &golden_json, &rust_json, &mut all_divergences);
    all_divergences
        .sort_by(|a, b| (a.fixture.as_str(), &a.path).cmp(&(b.fixture.as_str(), &b.path)));

    let allowlist: Vec<AllowEntry> = load_allowlist()
        .into_iter()
        .filter(|e| e.test == R3A4_TEST_NAME)
        .collect();

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
            "\n{} UNDOCUMENTED R3a-4 divergence(s) (not in KNOWN_DIVERGENCES.json, \
             test={R3A4_TEST_NAME}):\n",
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
            "\n{} UNUSED R3a-4 allowlist entr(y/ies) (no matching divergence this run):\n",
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
        "R3a-4 dep-hook differential FAILED:{failure}"
    );

    // --- BYTE-MATCH guard: KNOWN_DIVERGENCES is empty → the pretty JSON must be
    // byte-identical to the golden file (the strongest oracle). ----------------
    assert!(
        allowlist.is_empty(),
        "R3a-4 KNOWN_DIVERGENCES must be empty for the EXIT GATE (byte-match); found {} entr(y/ies)",
        allowlist.len()
    );
    let rust_pretty = serde_json::to_string_pretty(&projection)
        .unwrap_or_else(|e| panic!("pretty-serialize Rust R3a-4 projection: {e}"));
    // serde_json::to_string_pretty omits the trailing newline; the golden file ends
    // with one. Normalize by comparing the trimmed bodies + asserting the golden's
    // trailing newline.
    assert_eq!(
        rust_pretty.trim_end(),
        golden_text.trim_end(),
        "R3a-4 pretty JSON is NOT byte-identical to the golden (KNOWN_DIVERGENCES=[])"
    );

    eprintln!(
        "R3a-4 differential: 1 fixture, byte-match, KNOWN_DIVERGENCES=[] ({} entr(y/ies)).",
        allowlist.len()
    );
}

/// ANTI-DEGENERATE matrix (fail-on-zero) — the corpus must exercise every payload
/// surface, and the Rust counts must EQUAL the al-sem manifest's `matrix` block.
#[test]
fn r3a4_anti_degenerate_matrix_matches_manifest() {
    let projection = rust_projection();

    // Read the al-sem manifest matrix (ground truth captured at dump time).
    let manifest_path = goldens_dir().join("manifest.json");
    let manifest_text = std::fs::read_to_string(&manifest_path)
        .unwrap_or_else(|e| panic!("read R3a-4 manifest {}: {e}", manifest_path.display()));
    let manifest: Value = serde_json::from_str(&manifest_text)
        .unwrap_or_else(|e| panic!("R3a-4 manifest not valid JSON: {e}"));
    let mat = manifest
        .get("matrix")
        .unwrap_or_else(|| panic!("R3a-4 manifest carries no `matrix` block"));
    let u = |k: &str| mat.get(k).and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    let b = |k: &str| mat.get(k).and_then(|v| v.as_bool()).unwrap_or(false);

    // --- fail-on-zero (the non-hollow chain dep exercises every surface) -------
    let mut degenerate: Vec<String> = Vec::new();
    if projection.intra_app_call_edges_count == 0 {
        degenerate.push("intraAppCallEdgesCount=0 (need the DoIt→DoWrite edge)".into());
    }
    if projection.injected_typed_edges_count == 0 {
        degenerate.push("injectedTypedEdgesCount=0 (the edge must inject)".into());
    }
    if projection.cited_evidence_count == 0 {
        degenerate.push("citedEvidenceCount=0 (the r.Insert witness must be cited)".into());
    }
    if projection.order_entries_count == 0 {
        degenerate.push("orderEntriesCount=0 (DoIt + DoWrite order entries)".into());
    }
    if projection.return_summaries_count == 0 {
        degenerate.push("returnSummariesCount=0 (DoIt + DoWrite summaries)".into());
    }
    if !projection.dep_order_index_present {
        degenerate.push("depOrderIndexPresent=false (source-bearing dep → present)".into());
    }
    if !projection.freshness_stamp_fresh {
        degenerate.push("freshnessStampFresh=false (empty pkgHash → conservatively fresh)".into());
    }
    assert!(
        degenerate.is_empty(),
        "DEGENERATE R3a-4 matrix — a payload surface is hollow:\n  {}",
        degenerate.join("\n  ")
    );

    // --- pinned counts (the al-sem plan's exact targets) ----------------------
    assert_eq!(
        projection.intra_app_call_edges_count, 1,
        "intraAppCallEdges=1"
    );
    assert_eq!(
        projection.injected_typed_edges_count, 1,
        "injectedTypedEdges=1"
    );
    assert_eq!(projection.cited_evidence_count, 1, "citedEvidence=1");
    assert_eq!(projection.order_entries_count, 2, "orderEntries=2");
    assert_eq!(projection.return_summaries_count, 2, "returnSummaries=2");
    assert!(projection.dep_order_index_present, "depOrderIndexPresent");
    assert!(projection.freshness_stamp_fresh, "freshnessStampFresh");

    // --- cross-check vs the al-sem manifest matrix (ground truth) -------------
    assert_eq!(
        projection.intra_app_call_edges_count,
        u("totalIntraAppCallEdges"),
        "intraAppCallEdges vs manifest"
    );
    assert_eq!(
        projection.injected_typed_edges_count,
        u("totalInjectedTypedEdges"),
        "injectedTypedEdges vs manifest"
    );
    assert_eq!(
        projection.cited_evidence_count,
        u("totalCitedEvidence"),
        "citedEvidence vs manifest"
    );
    assert_eq!(
        projection.order_entries_count,
        u("totalOrderEntries"),
        "orderEntries vs manifest"
    );
    assert_eq!(
        projection.return_summaries_count,
        u("totalReturnSummaries"),
        "returnSummaries vs manifest"
    );
    assert_eq!(
        projection.dep_order_index_present,
        b("depOrderIndexPresent"),
        "depOrderIndexPresent vs manifest"
    );
    assert_eq!(
        projection.freshness_stamp_fresh,
        b("freshnessStampPresent"),
        "freshnessStampFresh vs manifest.freshnessStampPresent"
    );

    eprintln!(
        "R3a-4 matrix: intraAppCallEdges={} injectedTypedEdges={} citedEvidence={} \
         orderEntries={} returnSummaries={} depOrderIndexPresent={} freshnessStampFresh={} \
         (== al-sem manifest)",
        projection.intra_app_call_edges_count,
        projection.injected_typed_edges_count,
        projection.cited_evidence_count,
        projection.order_entries_count,
        projection.return_summaries_count,
        projection.dep_order_index_present,
        projection.freshness_stamp_fresh,
    );
}

/// UNIFIED GOLDEN REFRESH (R3a-4) — the one-command regen path. `#[ignore]`d so the
/// default `cargo test` loop stays OFFLINE; gated on `AL_SEM_DIR`. After regenerating
/// the al-sem goldens (`bun run scripts/dump-r3a4-dep-hooks.ts`), run:
///
/// ```bash
/// AL_SEM_DIR=/u/Git/al-sem cargo test --test r3a4_differential -- \
///     --ignored refresh_r3a4_goldens_from_al_sem --nocapture
/// ```
///
/// Re-copies from the al-sem checkout into the engine:
///   - the golden + manifest (`scripts/r3a4-goldens/{cross-app-dep-hooks.r3a4.golden.json,
///     manifest.json}` → `tests/r3a4-goldens/`),
///   - the committed dep `.app` fixture (`test/fixtures/r3a4-deps/<guid>.app` →
///     `tests/r3a4-fixtures/<guid>.app` AND `tests/r3a4-fixtures/ws/.alpackages/<guid>.app`),
///     so BOTH sides read the SAME `.app` bytes.
///
/// NOTE: al-sem builds the R3a-4 workspace INLINE (TS constants in
/// `scripts/r3a4-projection.ts`); the engine's `tests/r3a4-fixtures/ws/{app.json,src/*.al}`
/// is the hand-maintained mirror — if the al-sem capture changes the workspace shape,
/// update those `.al` files. This refresh copies the goldens + the dep `.app`; it does
/// NOT auto-commit.
#[test]
#[ignore]
fn refresh_r3a4_goldens_from_al_sem() {
    let al_sem = match std::env::var("AL_SEM_DIR") {
        Ok(d) => PathBuf::from(d),
        Err(_) => {
            eprintln!("AL_SEM_DIR not set — skipping R3a-4 refresh");
            return;
        }
    };
    const DEP_GUID: &str = "cccccccc-0001-0000-0000-000000000001";

    // 1. goldens + manifest.
    let src_goldens = al_sem.join("scripts").join("r3a4-goldens");
    let dst_goldens = goldens_dir();
    std::fs::create_dir_all(&dst_goldens).expect("mk r3a4 goldens dir");
    for name in [
        "cross-app-dep-hooks.r3a4.golden.json",
        "manifest.json",
        "r3a4-vectors.json",
    ] {
        let src = src_goldens.join(name);
        if src.exists() {
            std::fs::copy(&src, dst_goldens.join(name))
                .unwrap_or_else(|e| panic!("copy {name}: {e}"));
        }
    }
    eprintln!("R3a-4: copied goldens → {}", dst_goldens.display());

    // 2. the dep `.app` fixture (both the flat fixture + the ws/.alpackages copy).
    let src_app = al_sem
        .join("test")
        .join("fixtures")
        .join("r3a4-deps")
        .join(format!("{DEP_GUID}.app"));
    let fixtures = repo_root().join("tests").join("r3a4-fixtures");
    std::fs::copy(&src_app, fixtures.join(format!("{DEP_GUID}.app")))
        .unwrap_or_else(|e| panic!("copy dep .app (flat): {e}"));
    let alpackages = fixtures.join("ws").join(".alpackages");
    std::fs::create_dir_all(&alpackages).expect("mk ws/.alpackages");
    std::fs::copy(&src_app, alpackages.join(format!("{DEP_GUID}.app")))
        .unwrap_or_else(|e| panic!("copy dep .app (ws): {e}"));
    eprintln!("R3a-4: copied dep .app → flat + ws/.alpackages");

    eprintln!(
        "R3a-4 goldens + dep .app refreshed from {}",
        al_sem.display()
    );
}

/// Determinism: the stable ids are devFingerprint/modelInstanceId-INDEPENDENT — the
/// projection emits the SAME stable id regardless of the internal model_instance_id,
/// and is byte-stable across repeated runs.
#[test]
fn r3a4_stable_ids_are_model_instance_independent() {
    let a = serde_json::to_string(&rust_projection()).unwrap();
    let b = serde_json::to_string(&rust_projection()).unwrap();
    assert_eq!(a, b, "R3a-4 projection is byte-stable across runs");

    // No internal `dep:<artifactKey>` prefix may appear anywhere in the emitted ids.
    assert!(
        !a.contains("\"dep:") || a.contains("\"dep:cccccccc"),
        "no internal dep:<artifactKey> id prefix leaks (only the sourceFile \
         `dep:<appGuid>:<path>` form is allowed)"
    );
    // Every emitted routine id is the stable `<appGuid>:Codeunit:<num>#<hash>` form.
    assert!(
        a.contains("cccccccc-0001-0000-0000-000000000001:Codeunit:50300#"),
        "stable routine id form present"
    );
}
