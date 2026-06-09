//! Stage A2 — `--format json` DocumentEnvelope differential test.
//!
//! For each fixture in the corpus, runs the Rust gate pipeline under
//! `--format json --deterministic` with `AL_SEM_VERSION_OVERRIDE=cli-a-json-v1`
//! and byte-compares the output against the committed al-sem goldens at
//! `U:\Git\al-sem\scripts\cli-a-goldens\json\<fixture>.<slot>.json`.
//!
//! Two goldens per fixture:
//!   - `.default.json` — DEFAULT_DETECTORS (34 detectors).
//!   - `.all.json`     — ALL_DETECTORS (41 detectors, including opt-ins).
//!
//! ## Acceptance gate
//! All 20 × 2 = 40 goldens MUST byte-match. `KNOWN_DIVERGENCES.json` MUST be `[]`.
//!
//! ## Refresh (ignored)
//! `#[ignore]` refresh test shells out to `bun run scripts/dump-analyze-json.ts`
//! (under `AL_SEM_DIR`) to regenerate the goldens from the TS reference.

use std::path::PathBuf;
use std::sync::Mutex;

use al_call_hierarchy::engine::gate::filter::Scope;
use al_call_hierarchy::engine::gate::run::{run_analyze_with_exit, AnalyzeArgs, OutputFormat};

const TEST_NAME: &str = "cli_a_json_differential";

/// The version pin used for all JSON golden captures.
const JSON_VERSION_OVERRIDE: &str = "cli-a-json-v1";

/// Serialises the tests that mutate the process-global env var so they do not
/// race under cargo's parallel test threads.
static ENV_LOCK: Mutex<()> = Mutex::new(());

/// al-sem `DEFAULT_DETECTORS` names, in al-sem's declaration order.
const DEFAULT_DETECTOR_NAMES: &[&str] = &[
    "d1-db-op-in-loop",
    "d2-event-fanout-in-loop",
    "d3-missing-setloadfields",
    "d4-repeated-lookup-in-loop",
    "d5-set-based-opportunity",
    "d7-recursive-event-expansion",
    "d8-commit-in-transaction",
    "d9-transaction-span-summary",
    "d10-self-modifying-loop",
    "d11-modify-without-get",
    "d12-dead-integration-event",
    "d13-cross-app-internal-call",
    "d14-dead-routine",
    "d16-obsolete-routine-call",
    "d17-min-version-drift",
    "d18-constant-filter-in-loop",
    "d19-unused-parameter",
    "d20-unreachable-after-exit",
    "d21-read-without-load",
    "d22-flowfield-without-calcfields",
    "d29-subscriber-modify-on-event-record",
    "d32-constant-boolean-parameter",
    "d33-unfiltered-bulk-write",
    "d34-commit-in-loop",
    "d35-commit-in-event-subscriber",
    "d36-late-setloadfields",
    "d37-validate-without-persist",
    "d38-subscriber-to-obsolete-event",
    "d39-record-left-dirty-across-chain",
    "d41-transitive-filter-loss",
    "d42-cross-call-wrong-setloadfields",
    "d43-event-ishandled-skip",
    "d44-event-multi-subscriber-overlap",
    "d45-event-transitive-table-exposure",
];

/// The 20 fixtures under test (same corpus as stats + SARIF differentials).
const FIXTURES: &[&str] = &[
    "ws-d8-commit-in-tx",
    "ws-d34",
    "ws-d35",
    "ws-txn-d46-pos",
    "ws-txn-d47-pos-http-nocommit",
    "ws-txn-d47-pos-http-commit-after",
    "ws-txn-d47-pos-file",
    "ws-txn-d48-pos",
    "ws-txn-d49-pos-modify-message",
    "ws-txn-d49-pos-modify-runmodal",
    "ws-d51-pos",
    "ws-d51-jobqueue",
    "ws-txn-d46-neg",
    "ws-txn-d47-neg-readonly",
    "ws-txn-d47-neg-temp",
    "ws-txn-d48-neg",
    "ws-txn-d49-neg-no-write",
    "ws-d51-neg",
    "ws-d1-multi-caller",
    "ws-d14-dead-routine",
];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn corpus_dir() -> PathBuf {
    repo_root().join("tests").join("r0-corpus")
}

fn al_sem_json_dir() -> PathBuf {
    repo_root()
        .parent()
        .expect("CARGO_MANIFEST_DIR has a parent")
        .join("al-sem")
        .join("scripts")
        .join("cli-a-goldens")
        .join("json")
}

/// Build detector string for `--detector` flag from a names slice.
fn detector_arg(names: &[&str]) -> String {
    names.join(",")
}

/// Run the Rust JSON pipeline for one fixture with the given detector list.
/// The env var `AL_SEM_VERSION_OVERRIDE` MUST be set by the caller before
/// this function is called (it reads the env at call time via `alsem_version()`).
fn run_json(fixture: &str, detector_csv: &str) -> String {
    let fixture_dir = corpus_dir().join(fixture);
    assert!(
        fixture_dir.is_dir(),
        "{TEST_NAME}: fixture {fixture} not found at {}",
        fixture_dir.display()
    );
    let args = AnalyzeArgs {
        workspace: fixture_dir.to_string_lossy().to_string(),
        min_severity: None,
        detector: Some(detector_csv.to_string()),
        preset: None,
        scope: Scope::Primary,
        limit: None,
        format: OutputFormat::Json,
        sarif_version_override: None,
        fail_on: None,
        require_dependencies: false,
        baseline: None,
        update_baseline: false,
        disable_inline_suppression: false,
        group_by: None,
        deterministic: true,
    };
    // The pipeline returns (output, exit_code, warning); we only need the output.
    // The trailing newline is appended by the bin — add it here to match the golden.
    match run_analyze_with_exit(&args, "engine-default") {
        Ok((out, _, _)) => format!("{out}\n"),
        Err(e) => panic!("{TEST_NAME}: run_analyze failed for {fixture}: {e}"),
    }
}

/// Obtain the all-detectors CSV (all 41 detectors in registry order).
fn all_detector_csv() -> String {
    use al_call_hierarchy::engine::l5::detectors::registered_detectors;
    registered_detectors()
        .into_iter()
        .map(|d| d.name)
        .collect::<Vec<_>>()
        .join(",")
}

/// Diff helper: produce a human-readable delta between two JSON strings
/// (golden vs rust). Tries to parse both; if either fails, falls back to
/// first-differing-line.
fn json_diff(fixture: &str, slot: &str, golden: &str, rust: &str) -> String {
    let gv: serde_json::Value = match serde_json::from_str(golden) {
        Ok(v) => v,
        Err(e) => return format!("golden JSON parse error: {e}"),
    };
    let rv: serde_json::Value = match serde_json::from_str(rust) {
        Ok(v) => v,
        Err(e) => return format!("rust JSON parse error: {e}"),
    };

    if gv == rv {
        // Byte mismatch but same JSON — whitespace/ordering issue only.
        let gl: Vec<&str> = golden.lines().collect();
        let rl: Vec<&str> = rust.lines().collect();
        for (i, (g, r)) in gl.iter().zip(rl.iter()).enumerate() {
            if g != r {
                return format!(
                    "[{fixture}/{slot}] byte mismatch at line {} (JSON values equal):\n  golden: {g}\n  rust:   {r}",
                    i + 1
                );
            }
        }
        format!("[{fixture}/{slot}] byte mismatch (different lengths, JSON values equal)")
    } else {
        // JSON values differ — produce a path-level diff.
        diff_values(fixture, slot, "", &gv, &rv)
    }
}

fn diff_values(
    fixture: &str,
    slot: &str,
    path: &str,
    gv: &serde_json::Value,
    rv: &serde_json::Value,
) -> String {
    match (gv, rv) {
        (serde_json::Value::Object(go), serde_json::Value::Object(ro)) => {
            let mut parts = Vec::new();
            for (k, gval) in go {
                let p = if path.is_empty() {
                    k.clone()
                } else {
                    format!("{path}.{k}")
                };
                match ro.get(k) {
                    Some(rval) if rval != gval => {
                        parts.push(diff_values(fixture, slot, &p, gval, rval));
                    }
                    None => parts.push(format!(
                        "[{fixture}/{slot}] .{p}: in golden, missing in rust"
                    )),
                    _ => {}
                }
            }
            for k in ro.keys() {
                if !go.contains_key(k) {
                    let p = if path.is_empty() {
                        k.clone()
                    } else {
                        format!("{path}.{k}")
                    };
                    parts.push(format!(
                        "[{fixture}/{slot}] .{p}: not in golden, present in rust"
                    ));
                }
            }
            if parts.is_empty() {
                format!("[{fixture}/{slot}] .{path}: objects differ (no field-level diff found)")
            } else {
                parts.join("\n")
            }
        }
        (serde_json::Value::Array(ga), serde_json::Value::Array(ra)) => {
            if ga.len() != ra.len() {
                return format!(
                    "[{fixture}/{slot}] .{path}[]: golden len={} rust len={}",
                    ga.len(),
                    ra.len()
                );
            }
            let mut parts = Vec::new();
            for (i, (gval, rval)) in ga.iter().zip(ra.iter()).enumerate() {
                if gval != rval {
                    let p = format!("{path}[{i}]");
                    parts.push(diff_values(fixture, slot, &p, gval, rval));
                }
            }
            if parts.is_empty() {
                format!("[{fixture}/{slot}] .{path}[]: arrays differ (no element-level diff)")
            } else {
                parts.join("\n")
            }
        }
        _ => format!(
            "[{fixture}/{slot}] .{path}: golden={} rust={}",
            serde_json::to_string(gv).unwrap_or_default(),
            serde_json::to_string(rv).unwrap_or_default()
        ),
    }
}

// ---------------------------------------------------------------------------
// Main byte-match test
// ---------------------------------------------------------------------------

#[test]
fn cli_a_json_byte_match() {
    let json_dir = al_sem_json_dir();
    if !json_dir.is_dir() {
        eprintln!(
            "{TEST_NAME}: al-sem json directory not found at {}, SKIPPING",
            json_dir.display()
        );
        return;
    }

    let all_csv = all_detector_csv();
    let default_csv = detector_arg(DEFAULT_DETECTOR_NAMES);

    let mut divergences: Vec<String> = Vec::new();

    // Serialize env access across all sub-runs (AL_SEM_VERSION_OVERRIDE is process-global).
    let _guard = ENV_LOCK.lock().unwrap();
    std::env::set_var("AL_SEM_VERSION_OVERRIDE", JSON_VERSION_OVERRIDE);

    for &fixture in FIXTURES {
        for (slot, csv) in &[("default", default_csv.as_str()), ("all", all_csv.as_str())] {
            let golden_path = json_dir.join(format!("{fixture}.{slot}.json"));
            if !golden_path.exists() {
                divergences.push(format!(
                    "[{fixture}/{slot}] golden file missing: {}",
                    golden_path.display()
                ));
                continue;
            }
            let golden = std::fs::read_to_string(&golden_path).unwrap_or_else(|e| {
                panic!("{TEST_NAME}: failed to read {}: {e}", golden_path.display())
            });
            let rust_out = run_json(fixture, csv);

            if rust_out != golden {
                let diff = json_diff(fixture, slot, &golden, &rust_out);
                divergences.push(diff);
            }
        }
    }

    std::env::remove_var("AL_SEM_VERSION_OVERRIDE");

    if !divergences.is_empty() {
        let mut msg = format!("{TEST_NAME}: {} divergence(s) found:\n", divergences.len());
        for d in &divergences {
            msg.push_str(&format!("  {d}\n"));
        }
        panic!("{msg}");
    }
}

// ---------------------------------------------------------------------------
// Anti-degenerate oracles
// ---------------------------------------------------------------------------

/// ws-txn-d46-neg (canonical 0-findings fixture) must produce diagnostics:[],
/// payload.findings:[], and bySeverity:{}, byDetector:{}.
#[test]
fn zero_findings_fixture_has_empty_maps() {
    let json_dir = al_sem_json_dir();
    if !json_dir.is_dir() {
        eprintln!("{TEST_NAME}: al-sem json directory not found, SKIPPING zero-findings oracle");
        return;
    }
    let default_csv = detector_arg(DEFAULT_DETECTOR_NAMES);

    let _guard = ENV_LOCK.lock().unwrap();
    std::env::set_var("AL_SEM_VERSION_OVERRIDE", JSON_VERSION_OVERRIDE);
    let out = run_json("ws-txn-d46-neg", &default_csv);
    std::env::remove_var("AL_SEM_VERSION_OVERRIDE");

    let v: serde_json::Value =
        serde_json::from_str(&out).expect("zero-findings output must be valid JSON");

    assert_eq!(
        v["diagnostics"].as_array().map(|a| a.len()).unwrap_or(99),
        0,
        "ws-txn-d46-neg must have 0 diagnostics"
    );
    assert_eq!(
        v["payload"]["findings"]
            .as_array()
            .map(|a| a.len())
            .unwrap_or(99),
        0,
        "ws-txn-d46-neg must have 0 findings"
    );
    let by_sev = &v["payload"]["summary"]["bySeverity"];
    assert_eq!(
        by_sev,
        &serde_json::json!({}),
        "ws-txn-d46-neg bySeverity must be empty object"
    );
    let by_det = &v["payload"]["summary"]["byDetector"];
    assert_eq!(
        by_det,
        &serde_json::json!({}),
        "ws-txn-d46-neg byDetector must be empty object"
    );
}

/// Envelope fields for any fixture must match the pinned contract values.
#[test]
fn envelope_fields_are_correct() {
    let fixture_dir = corpus_dir().join("ws-txn-d46-neg");
    if !fixture_dir.is_dir() {
        eprintln!("{TEST_NAME}: ws-txn-d46-neg fixture missing, SKIPPING envelope oracle");
        return;
    }
    let default_csv = detector_arg(DEFAULT_DETECTOR_NAMES);

    let _guard = ENV_LOCK.lock().unwrap();
    std::env::set_var("AL_SEM_VERSION_OVERRIDE", JSON_VERSION_OVERRIDE);
    let out = run_json("ws-txn-d46-neg", &default_csv);
    std::env::remove_var("AL_SEM_VERSION_OVERRIDE");

    let v: serde_json::Value = serde_json::from_str(&out).expect("output must be valid JSON");

    assert_eq!(v["kind"], "analyze-report");
    assert_eq!(v["schemaVersion"], "1.0.0");
    assert_eq!(v["alsemVersion"], JSON_VERSION_OVERRIDE);
    assert_eq!(v["deterministic"], true);
    assert_eq!(v["generatedAt"], "1970-01-01T00:00:00Z");
}

// ---------------------------------------------------------------------------
// Refresh test (ignored — only run explicitly)
// ---------------------------------------------------------------------------

/// Regenerate the al-sem json goldens by running the TS reference.
///
/// Run with:
///   cargo test --test cli_a_json_differential refresh_goldens -- --ignored
///
/// Requires `AL_SEM_DIR` env var pointing to the al-sem repo root (or the
/// sibling `../al-sem` path is used as a fallback).
#[test]
#[ignore]
fn refresh_goldens() {
    let al_sem_dir = std::env::var("AL_SEM_DIR").unwrap_or_else(|_| {
        repo_root()
            .parent()
            .expect("parent")
            .join("al-sem")
            .to_string_lossy()
            .to_string()
    });
    let status = std::process::Command::new("bun")
        .args(["run", "scripts/dump-analyze-json.ts"])
        .current_dir(&al_sem_dir)
        .env("AL_SEM_VERSION_OVERRIDE", JSON_VERSION_OVERRIDE)
        .status()
        .expect("failed to run bun");
    assert!(
        status.success(),
        "bun run scripts/dump-analyze-json.ts failed"
    );
    eprintln!("refresh_goldens: goldens refreshed at {al_sem_dir}/scripts/cli-a-goldens/json/");
}
