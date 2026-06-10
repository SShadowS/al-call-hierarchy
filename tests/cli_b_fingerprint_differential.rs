//! cli-b/b3 — FINGERPRINT CLI differential.
//!
//! For each fixture in `FINGERPRINT_CORPUS` (20 fixtures), runs the Rust fingerprint
//! pipeline and byte-compares the output against the committed al-sem goldens under
//! `U:\Git\al-sem\scripts\cli-b-goldens\fingerprint\`.
//!
//! ## Goldens checked per fixture
//!
//! - `<fixture>.json`          — fingerprint-query JSON (query flags: includeInherited=true, witnessLimit=3)
//! - `<fixture>.human.txt`     — compact human text (same query flags)
//! - `<fixture>.cbor`          — CBOR snapshot (B0; no query flags)
//! - `<fixture>.cbor.gz`       — CBOR.gz snapshot (B0; no query flags)
//!
//! ## Extra goldens for `ws-d8-commit-in-tx`
//!
//! - `.witness-all.json`       — witnessLimit="all"
//! - `.witness-0.json`         — witnessLimit=0
//! - `.witness-false.json`     — witnessLimit=false (disabled)
//! - `.selector-error.json`    — selector error path (exit 2); valid doc in payload.diagnostics
//! - `.human-full.txt`         — full verbosity human text
//! - `shards/manifest.json`    — shard manifest (B0)
//! - `shards/primary.json`     — primary shard (B0)
//!
//! ## Acceptance gate
//!
//! All goldens MUST byte-match. Divergences are bugs to fix, not KNOWN_DIVERGENCES entries.
//!
//! ## Refresh (ignored)
//!
//! `#[ignore] refresh_goldens` shells `bun run scripts/dump-fingerprint.ts` under
//! `AL_SEM_DIR` to regenerate the goldens.

use std::path::{Path, PathBuf};

use al_call_hierarchy::engine::gate::model_instance_id::compute_gate_model_instance_id;
use al_call_hierarchy::engine::gate::run::compute_analyzer_diagnostics;
use al_call_hierarchy::engine::l3::l3_workspace::assemble_and_resolve_workspace;
use al_call_hierarchy::engine::l5::detectors::registered_detectors;
use al_call_hierarchy::engine::l5::fingerprint_cli::{
    run_fingerprint_pipeline, FingerprintFormat, FingerprintOptions, FingerprintOutput,
};
use al_call_hierarchy::engine::l5::fingerprint_query::WitnessLimit;
use al_call_hierarchy::engine::l5::snapshot_full::{
    compose_full_snapshot, serialize_cbor, serialize_cbor_gz, serialize_envelope,
    serialize_sharded, EnvelopeDiagnostic, FullSnapshotOptions,
};

const VERSION_OVERRIDE: &str = "cli-b-v1";

/// The fingerprint corpus (same 20 fixtures as the snapshot corpus).
const FINGERPRINT_CORPUS: &[&str] = &[
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

const WITNESS_FIXTURE: &str = "ws-d8-commit-in-tx";
const SHARD_FIXTURE: &str = "ws-d8-commit-in-tx";
const ERROR_FIXTURE: &str = "ws-d8-commit-in-tx";
const ERROR_SELECTOR: &str = "THIS_ROUTINE_DOES_NOT_EXIST_FOR_ERROR_TEST";

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
    "d16-blob-in-loop",
    "d17-non-setbased-on-large-table",
    "d18-event-subscriber-heavy",
    "d19-flowfield-in-loop",
    "d20-unbounded-result-set",
    "d21-temp-table-misuse",
    "d22-deprecated-api-use",
    "d29-onaftergetrecord-heavy",
    "d32-internal-event-publisher",
    "d33-event-without-subscribers",
    "d34-page-source-heavy",
    "d35-implicit-transaction-scope",
    "d36-redundant-calcfields",
    "d37-record-passed-by-value",
    "d38-page-trigger-heavy",
    "d39-codeunit-instantiation-in-loop",
    "d41-unindexed-filter",
    "d42-locktable-late",
    "d43-event-ishandled-skip",
    "d44-event-recursive-publish",
    "d45-text-encoding-mismatch",
];

fn al_sem_dir() -> PathBuf {
    std::env::var("AL_SEM_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(r"U:\Git\al-sem"))
}

fn fixtures_dir() -> PathBuf {
    al_sem_dir().join("test").join("fixtures")
}

fn goldens_dir() -> PathBuf {
    al_sem_dir()
        .join("scripts")
        .join("cli-b-goldens")
        .join("fingerprint")
}

fn fixture_dir(fixture: &str) -> PathBuf {
    fixtures_dir().join(fixture)
}

fn first_diff(a: &[u8], b: &[u8]) -> Option<usize> {
    let n = a.len().min(b.len());
    for i in 0..n {
        if a[i] != b[i] {
            return Some(i);
        }
    }
    if a.len() != b.len() {
        Some(n)
    } else {
        None
    }
}

/// Build envelope diagnostics for a fixture workspace.
fn envelope_diagnostics(
    ws_path: &Path,
    resolved: &al_call_hierarchy::engine::l3::l3_workspace::L3Resolved,
) -> Vec<EnvelopeDiagnostic> {
    let default_detectors: Vec<_> = registered_detectors()
        .into_iter()
        .filter(|d| DEFAULT_DETECTOR_NAMES.contains(&d.name.as_str()))
        .collect();
    compute_analyzer_diagnostics(ws_path, resolved, &default_detectors)
        .into_iter()
        .map(|d| EnvelopeDiagnostic {
            code: format!("DIAG-{}", d.stage),
            severity: d.severity,
            message: d.message,
        })
        .collect()
}

/// Helper: compose full snapshot tree for the B0 path (cbor / cbor.gz / envelope).
fn compose_full_for(
    fixture: &str,
) -> (
    al_call_hierarchy::engine::gate::cbor::CborValue,
    al_call_hierarchy::engine::l3::l3_workspace::L3Resolved,
    PathBuf,
) {
    let ws = fixture_dir(fixture);
    assert!(
        ws.is_dir(),
        "fixture {fixture} not found at {}",
        ws.display()
    );
    let model_id = compute_gate_model_instance_id(&ws)
        .unwrap_or_else(|| panic!("{fixture}: could not compute modelInstanceId"));
    let resolved = assemble_and_resolve_workspace(&ws, &model_id)
        .unwrap_or_else(|| panic!("{fixture}: workspace did not resolve"));
    let opts = FullSnapshotOptions {
        workspace_dir: &ws,
        alsem_version: VERSION_OVERRIDE,
        deterministic: true,
        roots_config_ignored: false,
    };
    let tree = compose_full_snapshot(&resolved, &opts);
    (tree, resolved, ws)
}

/// Helper: run the fingerprint query pipeline for a fixture with default query options
/// (includeInherited=true, witnessLimit=3).
fn run_query_for(
    fixture: &str,
    witness_limit: Option<WitnessLimit>,
    verbosity: &str,
) -> (String, String) {
    let ws = fixture_dir(fixture);
    let opts = FingerprintOptions {
        workspace: &ws,
        alsem_version: VERSION_OVERRIDE,
        format: FingerprintFormat::Json,
        out: None,
        shard: false,
        witness_limit,
        roots: None,
        routine_selectors: Vec::new(),
        include_inherited: true,
        is_query_requested: true,
        deterministic: true,
        verbosity,
    };
    let result = run_fingerprint_pipeline(&opts)
        .unwrap_or_else(|e| panic!("{fixture}: fingerprint pipeline error: {e}"));
    let json_text = match result.output {
        FingerprintOutput::Text(t) => t,
        _ => panic!("{fixture}: expected Text output from json query"),
    };

    // Also get human text.
    let human_opts = FingerprintOptions {
        workspace: &ws,
        alsem_version: VERSION_OVERRIDE,
        format: FingerprintFormat::Human,
        out: None,
        shard: false,
        witness_limit,
        roots: None,
        routine_selectors: Vec::new(),
        include_inherited: true,
        is_query_requested: true,
        deterministic: true,
        verbosity,
    };
    let human_result = run_fingerprint_pipeline(&human_opts)
        .unwrap_or_else(|e| panic!("{fixture}: fingerprint human pipeline error: {e}"));
    let human_text = match human_result.output {
        FingerprintOutput::Text(t) => t,
        _ => panic!("{fixture}: expected Text output from human query"),
    };

    (json_text, human_text)
}

// ===========================================================================
// B0 tests: CBOR and CBOR.gz (no query flags)
// ===========================================================================

#[test]
fn cbor_matches_goldens() {
    for fixture in FINGERPRINT_CORPUS {
        let (tree, _, _) = compose_full_for(fixture);
        let got = serialize_cbor(&tree);
        let golden_path = goldens_dir().join(format!("{fixture}.cbor"));
        let want = std::fs::read(&golden_path)
            .unwrap_or_else(|e| panic!("read {}: {e}", golden_path.display()));
        if let Some(off) = first_diff(&got, &want) {
            panic!(
                "cli-b/b3 CBOR: {fixture}.cbor diverges at byte {off} (got={} want={})",
                got.len(),
                want.len()
            );
        }
    }
}

#[test]
fn cbor_gz_matches_goldens() {
    for fixture in FINGERPRINT_CORPUS {
        let (tree, _, _) = compose_full_for(fixture);
        let got = serialize_cbor_gz(&tree);
        let golden_path = goldens_dir().join(format!("{fixture}.cbor.gz"));
        let want = std::fs::read(&golden_path)
            .unwrap_or_else(|e| panic!("read {}: {e}", golden_path.display()));
        if let Some(off) = first_diff(&got, &want) {
            panic!(
                "cli-b/b3 CBOR.GZ: {fixture}.cbor.gz diverges at byte {off} (got={} want={})",
                got.len(),
                want.len()
            );
        }
    }
}

// ===========================================================================
// Query JSON tests (with query flags: includeInherited=true, witnessLimit=3)
// ===========================================================================

#[test]
fn query_json_matches_goldens() {
    for fixture in FINGERPRINT_CORPUS {
        let ws = fixture_dir(fixture);
        let opts = FingerprintOptions {
            workspace: &ws,
            alsem_version: VERSION_OVERRIDE,
            format: FingerprintFormat::Json,
            out: None,
            shard: false,
            witness_limit: Some(WitnessLimit::Capped(3)),
            roots: None,
            routine_selectors: Vec::new(),
            include_inherited: true,
            is_query_requested: true,
            deterministic: true,
            verbosity: "compact",
        };
        let result = run_fingerprint_pipeline(&opts)
            .unwrap_or_else(|e| panic!("{fixture}: fingerprint json pipeline error: {e}"));
        let got = match result.output {
            FingerprintOutput::Text(t) => t,
            _ => panic!("{fixture}: expected Text from json query"),
        };
        let golden_path = goldens_dir().join(format!("{fixture}.json"));
        let want = std::fs::read_to_string(&golden_path)
            .unwrap_or_else(|e| panic!("read {}: {e}", golden_path.display()));
        assert_eq!(
            got, want,
            "cli-b/b3 QUERY JSON: {fixture}.json did NOT byte-match"
        );
    }
}

// ===========================================================================
// Human compact text tests
// ===========================================================================

#[test]
fn human_compact_matches_goldens() {
    for fixture in FINGERPRINT_CORPUS {
        let ws = fixture_dir(fixture);
        let opts = FingerprintOptions {
            workspace: &ws,
            alsem_version: VERSION_OVERRIDE,
            format: FingerprintFormat::Human,
            out: None,
            shard: false,
            witness_limit: Some(WitnessLimit::Capped(3)),
            roots: None,
            routine_selectors: Vec::new(),
            include_inherited: true,
            is_query_requested: true,
            deterministic: true,
            verbosity: "compact",
        };
        let result = run_fingerprint_pipeline(&opts)
            .unwrap_or_else(|e| panic!("{fixture}: fingerprint human pipeline error: {e}"));
        let got = match result.output {
            FingerprintOutput::Text(t) => t,
            _ => panic!("{fixture}: expected Text from human query"),
        };
        let golden_path = goldens_dir().join(format!("{fixture}.human.txt"));
        let want = std::fs::read_to_string(&golden_path)
            .unwrap_or_else(|e| panic!("read {}: {e}", golden_path.display()));
        assert_eq!(
            got, want,
            "cli-b/b3 HUMAN COMPACT: {fixture}.human.txt did NOT byte-match"
        );
    }
}

// ===========================================================================
// Witness mode variants for ws-d8-commit-in-tx
// ===========================================================================

#[test]
fn witness_all_matches_golden() {
    let ws = fixture_dir(WITNESS_FIXTURE);
    let opts = FingerprintOptions {
        workspace: &ws,
        alsem_version: VERSION_OVERRIDE,
        format: FingerprintFormat::Json,
        out: None,
        shard: false,
        witness_limit: Some(WitnessLimit::All),
        roots: None,
        routine_selectors: Vec::new(),
        include_inherited: true,
        is_query_requested: true,
        deterministic: true,
        verbosity: "compact",
    };
    let result = run_fingerprint_pipeline(&opts)
        .unwrap_or_else(|e| panic!("{WITNESS_FIXTURE}: witness-all pipeline error: {e}"));
    let got = match result.output {
        FingerprintOutput::Text(t) => t,
        _ => panic!("expected Text"),
    };
    let golden_path = goldens_dir().join(format!("{WITNESS_FIXTURE}.witness-all.json"));
    let want = std::fs::read_to_string(&golden_path)
        .unwrap_or_else(|e| panic!("read {}: {e}", golden_path.display()));
    assert_eq!(
        got, want,
        "cli-b/b3: {WITNESS_FIXTURE}.witness-all.json did NOT byte-match"
    );
}

#[test]
fn witness_zero_matches_golden() {
    let ws = fixture_dir(WITNESS_FIXTURE);
    let opts = FingerprintOptions {
        workspace: &ws,
        alsem_version: VERSION_OVERRIDE,
        format: FingerprintFormat::Json,
        out: None,
        shard: false,
        witness_limit: Some(WitnessLimit::Capped(0)),
        roots: None,
        routine_selectors: Vec::new(),
        include_inherited: true,
        is_query_requested: true,
        deterministic: true,
        verbosity: "compact",
    };
    let result = run_fingerprint_pipeline(&opts)
        .unwrap_or_else(|e| panic!("{WITNESS_FIXTURE}: witness-0 pipeline error: {e}"));
    let got = match result.output {
        FingerprintOutput::Text(t) => t,
        _ => panic!("expected Text"),
    };
    let golden_path = goldens_dir().join(format!("{WITNESS_FIXTURE}.witness-0.json"));
    let want = std::fs::read_to_string(&golden_path)
        .unwrap_or_else(|e| panic!("read {}: {e}", golden_path.display()));
    assert_eq!(
        got, want,
        "cli-b/b3: {WITNESS_FIXTURE}.witness-0.json did NOT byte-match"
    );
}

#[test]
fn witness_false_matches_golden() {
    let ws = fixture_dir(WITNESS_FIXTURE);
    let opts = FingerprintOptions {
        workspace: &ws,
        alsem_version: VERSION_OVERRIDE,
        format: FingerprintFormat::Json,
        out: None,
        shard: false,
        witness_limit: Some(WitnessLimit::Disabled),
        roots: None,
        routine_selectors: Vec::new(),
        include_inherited: true,
        is_query_requested: true,
        deterministic: true,
        verbosity: "compact",
    };
    let result = run_fingerprint_pipeline(&opts)
        .unwrap_or_else(|e| panic!("{WITNESS_FIXTURE}: witness-false pipeline error: {e}"));
    let got = match result.output {
        FingerprintOutput::Text(t) => t,
        _ => panic!("expected Text"),
    };
    let golden_path = goldens_dir().join(format!("{WITNESS_FIXTURE}.witness-false.json"));
    let want = std::fs::read_to_string(&golden_path)
        .unwrap_or_else(|e| panic!("read {}: {e}", golden_path.display()));
    assert_eq!(
        got, want,
        "cli-b/b3: {WITNESS_FIXTURE}.witness-false.json did NOT byte-match"
    );
}

// ===========================================================================
// Selector error path (exit code 2 + valid doc in payload.diagnostics)
// ===========================================================================

#[test]
fn selector_error_json_matches_golden_and_exits_2() {
    let ws = fixture_dir(ERROR_FIXTURE);
    let opts = FingerprintOptions {
        workspace: &ws,
        alsem_version: VERSION_OVERRIDE,
        format: FingerprintFormat::Json,
        out: None,
        shard: false,
        witness_limit: Some(WitnessLimit::Capped(3)),
        roots: None,
        routine_selectors: vec![ERROR_SELECTOR.to_string()],
        include_inherited: true,
        is_query_requested: true,
        deterministic: true,
        verbosity: "compact",
    };
    let result = run_fingerprint_pipeline(&opts)
        .unwrap_or_else(|e| panic!("{ERROR_FIXTURE}: selector-error pipeline error: {e}"));
    assert_eq!(result.exit_code, 2, "selector error must exit 2");
    let got = match result.output {
        FingerprintOutput::Text(t) => t,
        _ => panic!("expected Text for selector error json"),
    };
    let golden_path = goldens_dir().join(format!("{ERROR_FIXTURE}.selector-error.json"));
    let want = std::fs::read_to_string(&golden_path)
        .unwrap_or_else(|e| panic!("read {}: {e}", golden_path.display()));
    assert_eq!(
        got, want,
        "cli-b/b3: {ERROR_FIXTURE}.selector-error.json did NOT byte-match"
    );
}

// ===========================================================================
// Human full verbosity
// ===========================================================================

#[test]
fn human_full_matches_golden() {
    let ws = fixture_dir(WITNESS_FIXTURE);
    let opts = FingerprintOptions {
        workspace: &ws,
        alsem_version: VERSION_OVERRIDE,
        format: FingerprintFormat::Human,
        out: None,
        shard: false,
        witness_limit: Some(WitnessLimit::Capped(3)),
        roots: None,
        routine_selectors: Vec::new(),
        include_inherited: true,
        is_query_requested: true,
        deterministic: true,
        verbosity: "full",
    };
    let result = run_fingerprint_pipeline(&opts)
        .unwrap_or_else(|e| panic!("{WITNESS_FIXTURE}: human-full pipeline error: {e}"));
    let got = match result.output {
        FingerprintOutput::Text(t) => t,
        _ => panic!("expected Text from human-full query"),
    };
    let golden_path = goldens_dir().join(format!("{WITNESS_FIXTURE}.human-full.txt"));
    let want = std::fs::read_to_string(&golden_path)
        .unwrap_or_else(|e| panic!("read {}: {e}", golden_path.display()));
    assert_eq!(
        got, want,
        "cli-b/b3: {WITNESS_FIXTURE}.human-full.txt did NOT byte-match"
    );
}

// ===========================================================================
// Shard output
// ===========================================================================

#[test]
fn shard_output_matches_goldens() {
    let ws = fixture_dir(SHARD_FIXTURE);
    let (tree, resolved, ws_path) = compose_full_for(SHARD_FIXTURE);

    let shards = serialize_sharded(&tree, VERSION_OVERRIDE, false);
    // Goldens live under shards/<fixture-name>/<shard-name>.
    let shard_dir = goldens_dir().join("shards").join(SHARD_FIXTURE);

    for shard in &shards {
        let golden_path = shard_dir.join(&shard.name);
        let want = std::fs::read(&golden_path)
            .unwrap_or_else(|e| panic!("read shard {}: {e}", golden_path.display()));
        if let Some(off) = first_diff(&shard.bytes, &want) {
            panic!(
                "cli-b/b3 SHARD: shards/{} diverges at byte {off} (got={} want={})",
                shard.name,
                shard.bytes.len(),
                want.len()
            );
        }
    }
    drop((resolved, ws_path)); // keep compiler happy
    drop(ws);
}

// ===========================================================================
// Refresh (ignored)
// ===========================================================================

/// Regenerate all fingerprint goldens by running the al-sem TS dump script.
/// Run with: `cargo test -p al-call-hierarchy --test cli_b_fingerprint_differential -- refresh_goldens --ignored --nocapture`
#[test]
#[ignore]
fn refresh_goldens() {
    let al_sem = al_sem_dir();
    let status = std::process::Command::new("bun")
        .args(["run", "scripts/dump-fingerprint.ts"])
        .current_dir(&al_sem)
        .status()
        .expect("bun run scripts/dump-fingerprint.ts");
    assert!(status.success(), "dump-fingerprint.ts failed: {status}");
}
