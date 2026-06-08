//! R4 — L5 DETECTOR FINDINGS differential over the SOURCE-ONLY smoke corpus.
//!
//! For each committed al-sem golden under `tests/r4-goldens/<fixture>.r4.golden.json`,
//! run the Rust source-only L0→L3→L5 pass (`assemble_and_resolve_workspace_default(...)`
//! → `engine::l5::finding::project_r4_findings(...)` over the REGISTERED detectors)
//! over the matching `tests/r0-corpus/<fixture>` workspace.
//!
//! ## Wave gating (the ACCEPTANCE GATE)
//!
//! Only d4 is ported in R4-0 Task 2b. So:
//!   - `ws-d4-repeated-get` (wave R4-A) MUST byte-match its golden END-TO-END
//!     (the projection serialized pretty + trailing newline == the golden file).
//!   - The other 6 smoke fixtures carry findings from NOT-YET-PORTED detectors.
//!     For those we assert the fixture runs cleanly to the L5 boundary and, with
//!     only the registered (d4) detectors, produces the d4-SUBSET of the golden
//!     (empty for non-d4 fixtures), logging "deferred to wave X". Each wave flips
//!     its fixture to a full byte-match as its detector lands.
//!
//! ## Anti-degenerate (fail-on-zero)
//!
//! `ws-d4-repeated-get` MUST produce ≥1 finding AND byte-match — a regression that
//! zeroed d4 would otherwise pass the "empty subset == empty subset" path.
//!
//! ## KNOWN_DIVERGENCES gating
//!
//! Reuses the repo-root `KNOWN_DIVERGENCES.json` with exact `(test, fixture, path)`
//! matching, scoped to `test == R4_TEST_NAME`. Target: empty for the ported subset.

use std::collections::BTreeSet;
use std::path::PathBuf;

use al_call_hierarchy::engine::l3::l3_workspace::assemble_and_resolve_workspace_default;
use al_call_hierarchy::engine::l5::detectors::registered_detectors;
use al_call_hierarchy::engine::l5::finding::{project_r4_findings, R4FindingsProjection};
use serde::Deserialize;
use serde_json::Value;

const R4_TEST_NAME: &str = "differential_r4_findings_match_goldens";

/// The smoke set: ≥1 fixture per substrate wave (mirrors al-sem's SMOKE_FIXTURES).
/// `(fixture, wave, detectors)`. `detectors` is the GOLDEN envelope's detector
/// list (NOT the registered set) — passed to `project_r4_findings` so the envelope
/// matches the al-sem golden. `ported` flips true as each wave's detector lands.
struct Smoke {
    fixture: &'static str,
    wave: &'static str,
    detectors: &'static [&'static str],
    ported: bool,
}

const SMOKE: &[Smoke] = &[
    Smoke {
        fixture: "ws-d4-repeated-get",
        wave: "R4-A",
        detectors: &["d4-repeated-lookup-in-loop"],
        ported: true,
    },
    Smoke {
        fixture: "ws-d22",
        wave: "R4-B",
        detectors: &["d22-flowfield-without-calcfields"],
        ported: false,
    },
    Smoke {
        fixture: "ws-d12-dead-event",
        wave: "R4-C",
        detectors: &["d12-dead-integration-event"],
        ported: false,
    },
    Smoke {
        fixture: "ws-d8-commit-in-tx",
        wave: "R4-D",
        detectors: &["d8-commit-in-transaction"],
        ported: false,
    },
    Smoke {
        fixture: "ws-d3",
        wave: "R4-E",
        detectors: &["d3-missing-setloadfields"],
        ported: false,
    },
    Smoke {
        fixture: "ws-txn-d47-pos-http-nocommit",
        wave: "R4-F",
        detectors: &["d47-io-unsafe-txn"],
        ported: false,
    },
    Smoke {
        fixture: "ws-d14-dead-routine",
        wave: "R4-G",
        detectors: &["d14-dead-routine"],
        ported: false,
    },
];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn goldens_dir() -> PathBuf {
    repo_root().join("tests").join("r4-goldens")
}

fn corpus_dir() -> PathBuf {
    repo_root().join("tests").join("r0-corpus")
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

fn compact(v: &Value) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| format!("{v:?}"))
}

/// Recursively diff two values POSITIONALLY (both sides already canonically sorted).
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

/// Run the Rust source-only L5 pass for one fixture over the REGISTERED detectors,
/// projecting the envelope with the golden's declared detector list.
fn run_rust(fixture: &str, detector_names: &[&str]) -> R4FindingsProjection {
    let fixture_dir = corpus_dir().join(fixture);
    assert!(
        fixture_dir.is_dir(),
        "R4 golden for {fixture} has no matching in-repo fixture at {} (offline corpus incomplete)",
        fixture_dir.display()
    );
    let names: Vec<String> = detector_names.iter().map(|s| s.to_string()).collect();
    let detectors = registered_detectors();
    match assemble_and_resolve_workspace_default(&fixture_dir) {
        Some(resolved) => project_r4_findings(&resolved, &detectors, fixture, &names),
        None => R4FindingsProjection {
            fixture_name: fixture.to_string(),
            detectors: names,
            finding_count: 0,
            findings: vec![],
        },
    }
}

/// Pretty-serialize + trailing newline — the exact on-disk golden form.
fn pretty_with_newline(proj: &R4FindingsProjection) -> String {
    let mut s = serde_json::to_string_pretty(proj).expect("serialize R4 projection");
    s.push('\n');
    s
}

/// The subset of a golden `Value`'s findings whose `detector` is in `names`.
fn finding_subset(golden: &Value, names: &BTreeSet<String>) -> Vec<Value> {
    golden
        .get("findings")
        .and_then(|f| f.as_array())
        .map(|arr| {
            arr.iter()
                .filter(|f| {
                    f.get("detector")
                        .and_then(|d| d.as_str())
                        .map(|d| names.contains(d))
                        .unwrap_or(false)
                })
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn differential_r4_findings_match_goldens() {
    let allowlist: Vec<AllowEntry> = load_allowlist()
        .into_iter()
        .filter(|e| e.test == R4_TEST_NAME)
        .collect();

    let registered_names: BTreeSet<String> = registered_detectors()
        .iter()
        .map(|d| d.name.clone())
        .collect();

    let mut all_divergences: Vec<Divergence> = Vec::new();
    let mut d4_byte_matched = false;
    let mut d4_finding_count = 0usize;

    for smoke in SMOKE {
        let golden_path = goldens_dir().join(format!("{}.r4.golden.json", smoke.fixture));
        assert!(
            golden_path.is_file(),
            "missing R4 golden: {}",
            golden_path.display()
        );
        let golden_text = std::fs::read_to_string(&golden_path)
            .unwrap_or_else(|e| panic!("read R4 golden {}: {e}", golden_path.display()));
        let golden_json: Value = serde_json::from_str(&golden_text)
            .unwrap_or_else(|e| panic!("R4 golden {} not valid JSON: {e}", golden_path.display()));
        // Shape guard — the golden parses as R4FindingsProjection.
        let _: R4FindingsProjection =
            serde_json::from_value(golden_json.clone()).unwrap_or_else(|e| {
                panic!("R4 golden {} not R4FindingsProjection: {e}", smoke.fixture)
            });

        let rust = run_rust(smoke.fixture, smoke.detectors);

        if smoke.ported {
            // ACCEPTANCE GATE: full END-TO-END byte-match of the serialized doc.
            let rust_text = pretty_with_newline(&rust);
            if rust_text == golden_text {
                d4_byte_matched = true;
            } else {
                // Surface a structural diff to aid debugging, then fall through to
                // the assertion below.
                let rust_json = serde_json::to_value(&rust).expect("rust → value");
                diff_value(
                    smoke.fixture,
                    "",
                    &golden_json,
                    &rust_json,
                    &mut all_divergences,
                );
            }
            d4_finding_count = rust.finding_count;
            assert_eq!(
                rust_text, golden_text,
                "R4 ACCEPTANCE GATE: {} ({}) did NOT byte-match its golden",
                smoke.fixture, smoke.wave
            );
        } else {
            // DEFERRED wave: the fixture must run cleanly to the L5 boundary, and
            // the d4-SUBSET of the golden (filtered to the REGISTERED detectors)
            // must match the Rust output (empty == empty for non-d4 fixtures).
            let golden_subset = finding_subset(&golden_json, &registered_names);
            let rust_json = serde_json::to_value(&rust).expect("rust → value");
            let rust_subset = finding_subset(&rust_json, &registered_names);
            diff_value(
                smoke.fixture,
                ".findings(registered-subset)",
                &Value::Array(golden_subset.clone()),
                &Value::Array(rust_subset.clone()),
                &mut all_divergences,
            );
            eprintln!(
                "R4 {} ({}): deferred to wave {} — registered-subset findings: {} (golden subset: {})",
                smoke.fixture,
                smoke.wave,
                smoke.wave,
                rust_subset.len(),
                golden_subset.len(),
            );
        }
    }

    // --- Anti-degenerate (fail-on-zero on the ported detector) --------------
    assert!(
        d4_byte_matched,
        "R4 anti-degenerate: ws-d4-repeated-get did NOT byte-match (acceptance gate failed)"
    );
    assert!(
        d4_finding_count >= 1,
        "R4 anti-degenerate: ws-d4-repeated-get produced {d4_finding_count} findings — expected ≥1"
    );

    // --- Allowlist gating ---------------------------------------------------
    all_divergences
        .sort_by(|a, b| (a.fixture.as_str(), &a.path).cmp(&(b.fixture.as_str(), &b.path)));
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
            "\n{} UNDOCUMENTED R4 divergence(s) (not in KNOWN_DIVERGENCES.json, test={R4_TEST_NAME}):\n",
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
            "\n{} UNUSED R4 allowlist entr(y/ies) (no matching divergence this run):\n",
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
        "R4 findings differential FAILED:{failure}"
    );

    eprintln!(
        "R4 differential: {} smoke fixture(s); ws-d4-repeated-get byte-matched ({} finding(s)); \
         6 fixtures deferred to later waves; allowlist fully consumed ({} entr(y/ies)).",
        SMOKE.len(),
        d4_finding_count,
        allowlist.len(),
    );
}

/// Refresh the R4 goldens + manifest from a local al-sem checkout. Gated on
/// `AL_SEM_DIR`; does NOT auto-commit. Mirrors `refresh_r3a5_goldens_from_al_sem`.
#[test]
#[ignore]
fn refresh_r4_goldens_from_al_sem() {
    let al_sem = match std::env::var("AL_SEM_DIR") {
        Ok(d) => PathBuf::from(d),
        Err(_) => {
            eprintln!("AL_SEM_DIR not set — skipping R4 refresh");
            return;
        }
    };
    let src = al_sem.join("scripts").join("r4-goldens");
    let dst = goldens_dir();
    std::fs::create_dir_all(&dst).expect("mk r4 goldens dir");
    let mut copied = 0usize;
    for smoke in SMOKE {
        let name = format!("{}.r4.golden.json", smoke.fixture);
        let s = src.join(&name);
        if s.exists() {
            std::fs::copy(&s, dst.join(&name)).unwrap_or_else(|e| panic!("copy {name}: {e}"));
            copied += 1;
        }
    }
    let manifest = src.join("manifest.json");
    if manifest.exists() {
        std::fs::copy(&manifest, dst.join("manifest.json")).expect("copy manifest");
    }
    eprintln!(
        "R4: refreshed {copied} golden(s) + manifest from {} → {}",
        src.display(),
        dst.display()
    );
}
