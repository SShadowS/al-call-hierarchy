//! Stage A4 — `--format html` byte-parity differential test.
//!
//! For each fixture in the corpus, runs the Rust gate pipeline under
//! `--format html --deterministic` with `AL_SEM_VERSION_OVERRIDE=cli-a-json-v1`
//! and byte-compares the output against the committed al-sem goldens at
//! `U:\Git\al-sem\scripts\cli-a-goldens\html\<fixture>.<slot>.html`.
//!
//! Slots:
//!   - `.default.html` — DEFAULT_DETECTORS (34 detectors).
//!   - `.all.html`     — ALL_DETECTORS (only for ws-d8-commit-in-tx and ws-d1-multi-caller).
//!
//! Total: 22 goldens (20 default + 2 all).
//!
//! ## Notable fixtures
//!   - `ws-d8-commit-in-tx` — event-graph fixture, produces an inline `<svg>`.
//!   - `ws-txn-d46-neg` — canonical 0-findings fixture.
//!
//! ## Acceptance gate
//! All 22 goldens MUST byte-match. `KNOWN_DIVERGENCES.json` MUST be `[]`.
//!
//! ## Refresh (ignored)
//! `#[ignore]` refresh test shells out to `bun run scripts/dump-analyze-html.ts`
//! (under `AL_SEM_DIR`) to regenerate the goldens from the TS reference.

use std::path::PathBuf;
use std::sync::Mutex;

use al_call_hierarchy::engine::gate::filter::Scope;
use al_call_hierarchy::engine::gate::run::{run_analyze_with_exit, AnalyzeArgs, OutputFormat};

const TEST_NAME: &str = "cli_a_html_differential";

/// The version pin used for all HTML golden captures (same as JSON / terminal / stats).
const HTML_VERSION_OVERRIDE: &str = "cli-a-json-v1";

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

/// The 20 fixtures under test (same corpus as JSON / stats / terminal differentials).
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

/// Fixtures that have an `all` slot golden in addition to `default`.
const ALL_SLOT_FIXTURES: &[&str] = &["ws-d8-commit-in-tx", "ws-d1-multi-caller"];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn corpus_dir() -> PathBuf {
    repo_root().join("tests").join("r0-corpus")
}

fn al_sem_html_dir() -> PathBuf {
    repo_root()
        .parent()
        .expect("CARGO_MANIFEST_DIR has a parent")
        .join("al-sem")
        .join("scripts")
        .join("cli-a-goldens")
        .join("html")
}

/// Build detector string for `--detector` flag from a names slice.
fn detector_arg(names: &[&str]) -> String {
    names.join(",")
}

/// Obtain the all-detectors CSV (all registered detectors in registry order).
fn all_detector_csv() -> String {
    use al_call_hierarchy::engine::l5::detectors::registered_detectors;
    registered_detectors()
        .into_iter()
        .map(|d| d.name)
        .collect::<Vec<_>>()
        .join(",")
}

/// Run the Rust HTML pipeline for one fixture with the given detector list.
/// The env var `AL_SEM_VERSION_OVERRIDE` MUST be set by the caller before
/// this function is called (it reads the env at call time via `alsem_version()`).
fn run_html(fixture: &str, detector_csv: &str) -> String {
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
        format: OutputFormat::Html,
        sarif_version_override: None,
        fail_on: None,
        require_dependencies: false,
        baseline: None,
        update_baseline: false,
        disable_inline_suppression: false,
        group_by: None,
        deterministic: true,
        with_evidence: false,
    };
    // The pipeline returns (output, exit_code, warning); we only need the output.
    // The trailing newline is appended by the bin — add it here to match the golden.
    match run_analyze_with_exit(&args, "engine-default") {
        Ok((out, _, _)) => format!("{out}\n"),
        Err(e) => panic!("{TEST_NAME}: run_analyze failed for {fixture}: {e}"),
    }
}

/// Simple line-by-line diff helper for HTML.
fn html_diff(fixture: &str, slot: &str, golden: &str, rust: &str) -> String {
    if golden == rust {
        return String::new();
    }
    let gl: Vec<&str> = golden.lines().collect();
    let rl: Vec<&str> = rust.lines().collect();
    let max = gl.len().max(rl.len());
    for i in 0..max {
        let g = gl.get(i).copied().unwrap_or("<missing>");
        let r = rl.get(i).copied().unwrap_or("<missing>");
        if g != r {
            // Show context: up to 2 lines before the diff
            let ctx_start = i.saturating_sub(2);
            let mut ctx = String::new();
            for j in ctx_start..i {
                ctx.push_str(&format!(
                    "   [{}] {}\n",
                    j + 1,
                    gl.get(j).copied().unwrap_or("<missing>")
                ));
            }
            return format!(
                "[{fixture}/{slot}] first diff at line {}:\n{ctx}  golden[{}]: {g}\n  rust  [{}]: {r}",
                i + 1,
                i + 1,
                i + 1,
            );
        }
    }
    format!(
        "[{fixture}/{slot}] byte mismatch (golden {} lines, rust {} lines)",
        gl.len(),
        rl.len()
    )
}

// ---------------------------------------------------------------------------
// Main byte-match test
// ---------------------------------------------------------------------------

#[test]
fn cli_a_html_byte_match() {
    let html_dir = al_sem_html_dir();
    if !html_dir.is_dir() {
        eprintln!(
            "{TEST_NAME}: al-sem html directory not found at {}, SKIPPING",
            html_dir.display()
        );
        return;
    }

    let all_csv = all_detector_csv();
    let default_csv = detector_arg(DEFAULT_DETECTOR_NAMES);
    let all_slot_set: std::collections::HashSet<&str> = ALL_SLOT_FIXTURES.iter().copied().collect();

    let mut divergences: Vec<String> = Vec::new();

    // Serialize env access across all sub-runs (AL_SEM_VERSION_OVERRIDE is process-global).
    let _guard = ENV_LOCK.lock().unwrap();
    std::env::set_var("AL_SEM_VERSION_OVERRIDE", HTML_VERSION_OVERRIDE);

    for &fixture in FIXTURES {
        // Always run default slot.
        {
            let golden_path = html_dir.join(format!("{fixture}.default.html"));
            if !golden_path.exists() {
                divergences.push(format!(
                    "[{fixture}/default] golden file missing: {}",
                    golden_path.display()
                ));
            } else {
                let golden = std::fs::read_to_string(&golden_path).unwrap_or_else(|e| {
                    panic!("{TEST_NAME}: failed to read {}: {e}", golden_path.display())
                });
                let rust_out = run_html(fixture, &default_csv);
                if rust_out != golden {
                    let diff = html_diff(fixture, "default", &golden, &rust_out);
                    divergences.push(diff);
                }
            }
        }

        // Run all slot only for fixtures that have it.
        if all_slot_set.contains(fixture) {
            let golden_path = html_dir.join(format!("{fixture}.all.html"));
            if !golden_path.exists() {
                divergences.push(format!(
                    "[{fixture}/all] golden file missing: {}",
                    golden_path.display()
                ));
            } else {
                let golden = std::fs::read_to_string(&golden_path).unwrap_or_else(|e| {
                    panic!("{TEST_NAME}: failed to read {}: {e}", golden_path.display())
                });
                let rust_out = run_html(fixture, &all_csv);
                if rust_out != golden {
                    let diff = html_diff(fixture, "all", &golden, &rust_out);
                    divergences.push(diff);
                }
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

/// ws-txn-d46-neg (canonical 0-findings fixture) must render the "No findings."
/// empty body and the app masthead from app.json.
#[test]
fn zero_findings_fixture_renders_correctly() {
    let html_dir = al_sem_html_dir();
    if !html_dir.is_dir() {
        eprintln!("{TEST_NAME}: al-sem html directory not found, SKIPPING zero-findings oracle");
        return;
    }
    let default_csv = detector_arg(DEFAULT_DETECTOR_NAMES);

    let _guard = ENV_LOCK.lock().unwrap();
    std::env::set_var("AL_SEM_VERSION_OVERRIDE", HTML_VERSION_OVERRIDE);
    let out = run_html("ws-txn-d46-neg", &default_csv);
    std::env::remove_var("AL_SEM_VERSION_OVERRIDE");

    assert!(
        out.contains(r#"<p class="empty">No findings."#),
        "zero-findings fixture must contain the empty-body paragraph"
    );
    assert!(
        out.contains("0 finding(s)"),
        "zero-findings fixture must show 0 finding(s) in footer"
    );
}

/// ws-d8-commit-in-tx (event-graph fixture) must render an inline `<svg>` with
/// bezier paths and column headers.
#[test]
fn event_graph_fixture_renders_svg() {
    let fixture_dir = corpus_dir().join("ws-d8-commit-in-tx");
    if !fixture_dir.is_dir() {
        eprintln!("{TEST_NAME}: ws-d8-commit-in-tx fixture missing, SKIPPING event-graph oracle");
        return;
    }
    let default_csv = detector_arg(DEFAULT_DETECTOR_NAMES);

    let _guard = ENV_LOCK.lock().unwrap();
    std::env::set_var("AL_SEM_VERSION_OVERRIDE", HTML_VERSION_OVERRIDE);
    let out = run_html("ws-d8-commit-in-tx", &default_csv);
    std::env::remove_var("AL_SEM_VERSION_OVERRIDE");

    assert!(
        out.contains(r#"class="evgraph""#),
        "event-graph fixture must contain an evgraph SVG element"
    );
    assert!(
        out.contains("<path d=\"M"),
        "event-graph fixture must contain bezier path(s)"
    );
    assert!(
        out.contains("PUBLISHER"),
        "event-graph SVG must have PUBLISHER column header"
    );
    assert!(
        out.contains("SUBSCRIBER"),
        "event-graph SVG must have SUBSCRIBER column header"
    );
    assert!(
        out.contains("stroke-width=\"1.5\""),
        "bezier paths must use stroke-width=1.5"
    );
    assert!(
        out.contains("opacity=\"0.7\""),
        "bezier paths must use opacity=0.7"
    );
}

// ---------------------------------------------------------------------------
// Refresh test (ignored — only run explicitly)
// ---------------------------------------------------------------------------

/// Regenerate the al-sem html goldens by running the TS reference.
///
/// Run with:
///   cargo test --test cli_a_html_differential refresh_goldens -- --ignored
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
        .args(["run", "scripts/dump-analyze-html.ts"])
        .current_dir(&al_sem_dir)
        .env("AL_SEM_VERSION_OVERRIDE", HTML_VERSION_OVERRIDE)
        .status()
        .expect("failed to run bun");
    assert!(
        status.success(),
        "bun run scripts/dump-analyze-html.ts failed"
    );
    eprintln!("refresh_goldens: goldens refreshed at {al_sem_dir}/scripts/cli-a-goldens/html/");
}
