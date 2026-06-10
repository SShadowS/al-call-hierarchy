//! cli-c/c3 — `cache prune` classification + dry-run byte-parity differential tests.
//!
//! # Coverage
//!
//! (a) NATIVE ORACLES: `classify_artifact_for_prune` byte-for-byte vs
//!     `scripts/cli-c-goldens/cache/classification.json` for all 5 fixture files.
//! (b) DRY-RUN DIFFERENTIAL: run `cache prune --dry-run --dep-cache-dir <fixture-cache>`
//!     and byte-compare the stdout (with the first line normalized from the abs path to
//!     `<CACHE_DIR>`) to `scripts/cli-c-goldens/cache/dry-run.txt`.
//! (c) INTEGRATION: copy the fixture cache to a temp dir, run a real (non-dry-run)
//!     prune, assert the 4 `removed-*` files are deleted and the `kept` file remains.
//!     NOT byte-differentialed — the mutation is filesystem observable only.
//!
//! # Golden source
//! All goldens + fixtures live in the al-sem checkout (default `U:\Git\al-sem`).
//! Override with the `AL_SEM_DIR` environment variable.
//!
//! # Refresh
//! The `#[ignore]` test shells `bun run scripts/dump-cache.ts` under `AL_SEM_DIR`
//! and re-copies the goldens into the engine's test tree (not needed yet — goldens
//! are read directly from the al-sem checkout).

use std::path::PathBuf;

use al_call_hierarchy::engine::gate::cache_prune::{
    classify_artifact_for_prune, format_prune_report, prune_cache, PruneStatus,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// The al-sem checkout root.  Override via `AL_SEM_DIR`.
fn al_sem_dir() -> PathBuf {
    PathBuf::from(std::env::var("AL_SEM_DIR").unwrap_or_else(|_| r"U:\Git\al-sem".to_string()))
}

fn cache_goldens_dir() -> PathBuf {
    al_sem_dir()
        .join("scripts")
        .join("cli-c-goldens")
        .join("cache")
}

fn fixture_cache_dir() -> PathBuf {
    cache_goldens_dir().join("fixture-cache")
}

/// Skip the test if the al-sem checkout / fixture cache is not present.
fn corpus_available() -> bool {
    fixture_cache_dir().is_dir()
}

fn load_golden(name: &str) -> String {
    let path = cache_goldens_dir().join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read golden {}: {e}", path.display()))
}

// ---------------------------------------------------------------------------
// (a) Native oracles — classify_artifact_for_prune per fixture file
// ---------------------------------------------------------------------------

/// The 5 fixture files and their expected classification statuses.
const FIXTURE_CLASSIFICATIONS: &[(&str, PruneStatus)] = &[
    (
        "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef.json",
        PruneStatus::RemovedContentHashMismatch,
    ),
    (
        "babebabebabebabebabebabebabebabebabebabebabebabebabebabebabebabe.json",
        PruneStatus::RemovedVersionMismatch,
    ),
    (
        "cafecafecafecafecafecafecafecafecafecafecafecafecafecafecafecafe.json",
        PruneStatus::Kept,
    ),
    (
        "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef.json",
        PruneStatus::RemovedUnreadable,
    ),
    ("nothex.json", PruneStatus::RemovedBadName),
];

#[test]
fn classification_oracle_content_hash_mismatch() {
    if !corpus_available() {
        return;
    }
    let file = "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef.json";
    let path = fixture_cache_dir().join(file);
    assert_eq!(
        classify_artifact_for_prune(&path),
        PruneStatus::RemovedContentHashMismatch,
        "expected removed-content-hash-mismatch for {file}"
    );
}

#[test]
fn classification_oracle_version_mismatch() {
    if !corpus_available() {
        return;
    }
    let file = "babebabebabebabebabebabebabebabebabebabebabebabebabebabebabebabe.json";
    let path = fixture_cache_dir().join(file);
    assert_eq!(
        classify_artifact_for_prune(&path),
        PruneStatus::RemovedVersionMismatch,
        "expected removed-version-mismatch for {file}"
    );
}

#[test]
fn classification_oracle_kept() {
    if !corpus_available() {
        return;
    }
    let file = "cafecafecafecafecafecafecafecafecafecafecafecafecafecafecafecafe.json";
    let path = fixture_cache_dir().join(file);
    assert_eq!(
        classify_artifact_for_prune(&path),
        PruneStatus::Kept,
        "expected kept for {file}"
    );
}

#[test]
fn classification_oracle_unreadable() {
    if !corpus_available() {
        return;
    }
    let file = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef.json";
    let path = fixture_cache_dir().join(file);
    assert_eq!(
        classify_artifact_for_prune(&path),
        PruneStatus::RemovedUnreadable,
        "expected removed-unreadable for {file}"
    );
}

#[test]
fn classification_oracle_bad_name() {
    if !corpus_available() {
        return;
    }
    let file = "nothex.json";
    let path = fixture_cache_dir().join(file);
    assert_eq!(
        classify_artifact_for_prune(&path),
        PruneStatus::RemovedBadName,
        "expected removed-bad-name for {file}"
    );
}

/// Aggregate oracle: verify ALL 5 statuses against classification.json.
#[test]
fn classification_oracle_all_vs_golden_json() {
    if !corpus_available() {
        return;
    }

    // Load and parse the classification golden.
    let golden_text = load_golden("classification.json");
    let golden: serde_json::Value =
        serde_json::from_str(&golden_text).expect("classification.json must be valid JSON");
    let golden_arr = golden
        .as_array()
        .expect("classification.json must be an array");

    // Build expected map: file → status string.
    let expected: std::collections::HashMap<String, String> = golden_arr
        .iter()
        .map(|entry| {
            let file = entry["file"].as_str().expect("file field").to_string();
            let status = entry["status"].as_str().expect("status field").to_string();
            (file, status)
        })
        .collect();

    // Classify each fixture and compare.
    for (file, _) in FIXTURE_CLASSIFICATIONS {
        let path = fixture_cache_dir().join(file);
        let actual = classify_artifact_for_prune(&path);
        let actual_str = actual.as_str();
        let expected_str = expected
            .get(*file)
            .map(|s| s.as_str())
            .unwrap_or_else(|| panic!("file {file} not found in classification.json"));
        assert_eq!(
            actual_str, expected_str,
            "classification mismatch for {file}: got {actual_str:?}, want {expected_str:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// (b) Dry-run differential — stdout byte-matches dry-run.txt (with normalization)
// ---------------------------------------------------------------------------

#[test]
fn dry_run_differential_vs_golden() {
    if !corpus_available() {
        return;
    }

    let cache_dir = fixture_cache_dir();
    let cache_dir_str = cache_dir.to_string_lossy();

    // Run the dry-run prune.
    let result = prune_cache(Some(&cache_dir_str), true);
    let report = format_prune_report(&result, true);

    // Normalize: replace the abs cache dir path in the first line with `<CACHE_DIR>`.
    // al-sem golden: `al-sem cache: <CACHE_DIR>`
    let normalized = report.replacen(cache_dir_str.as_ref(), "<CACHE_DIR>", 1);

    // Load the committed golden.
    let golden = load_golden("dry-run.txt");

    assert_eq!(
        normalized, golden,
        "dry-run output does not match dry-run.txt golden.\n\
         --- normalized ---\n{normalized}\n--- golden ---\n{golden}"
    );
}

// ---------------------------------------------------------------------------
// (c) Integration test — real (non-dry-run) prune deletes removed-* files
// ---------------------------------------------------------------------------

#[test]
fn integration_real_prune_deletes_removed_files() {
    if !corpus_available() {
        return;
    }

    // Copy the fixture cache to a temp dir so we can mutate it.
    let tmp_dir = std::env::temp_dir().join(format!(
        "al-sem-cache-prune-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    std::fs::create_dir_all(&tmp_dir).expect("create temp dir");

    // Copy all fixture files into the temp dir.
    let src_dir = fixture_cache_dir();
    for entry in std::fs::read_dir(&src_dir).expect("read fixture-cache") {
        let entry = entry.expect("read entry");
        let src = entry.path();
        if !src.is_file() {
            continue;
        }
        let dst = tmp_dir.join(entry.file_name());
        std::fs::copy(&src, &dst).expect("copy fixture file");
    }

    let tmp_str = tmp_dir.to_string_lossy();

    // Run a REAL prune (not dry-run).
    let result = prune_cache(Some(&tmp_str), false);

    // 4 files should be removed, 1 kept.
    assert_eq!(
        result.files_removed, 4,
        "expected 4 files removed; got {}",
        result.files_removed
    );

    // The `kept` file must still exist.
    let kept_file = "cafecafecafecafecafecafecafecafecafecafecafecafecafecafecafecafe.json";
    assert!(
        tmp_dir.join(kept_file).exists(),
        "kept file {kept_file} should still exist after real prune"
    );

    // The 4 `removed-*` files must NOT exist.
    let removed_files = [
        "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef.json",
        "babebabebabebabebabebabebabebabebabebabebabebabebabebabebabebabe.json",
        "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef.json",
        "nothex.json",
    ];
    for f in &removed_files {
        assert!(
            !tmp_dir.join(f).exists(),
            "removed file {f} should be deleted after real prune"
        );
    }

    // Clean up.
    let _ = std::fs::remove_dir_all(&tmp_dir);
}

// ---------------------------------------------------------------------------
// #[ignore] refresh test — re-generates goldens by running dump-cache.ts
// ---------------------------------------------------------------------------

/// Refresh the cache goldens by running `bun run scripts/dump-cache.ts` in the
/// al-sem checkout. This test is marked `#[ignore]` — run manually with:
///   `cargo test -p al-call-hierarchy refresh_cache_goldens -- --ignored`
///
/// The dump script regenerates the fixture-cache files + classification.json +
/// dry-run.txt + manifest.json under `scripts/cli-c-goldens/cache/`.
#[test]
#[ignore]
fn refresh_cache_goldens() {
    let al_sem = al_sem_dir();
    assert!(
        al_sem.is_dir(),
        "AL_SEM_DIR not found at {}",
        al_sem.display()
    );

    let status = std::process::Command::new("bun")
        .args(["run", "scripts/dump-cache.ts"])
        .current_dir(&al_sem)
        .status()
        .expect("failed to run bun");

    assert!(
        status.success(),
        "dump-cache.ts failed with exit code: {:?}",
        status.code()
    );

    println!(
        "Cache goldens refreshed. Check for changes in {}",
        al_sem
            .join("scripts")
            .join("cli-c-goldens")
            .join("cache")
            .display()
    );
}
