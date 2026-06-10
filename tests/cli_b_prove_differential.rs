//! cli-b/b2 — the PROVE CLI differential.
//!
//! For each (fixture, routine, question) in the hardcoded `PROVE_CORPUS` const (which
//! mirrors the al-sem `scripts/cli-b-goldens/prove/manifest.json` entries), runs
//! `run_prove_pipeline` with `deterministic:true` and version `cli-b-v1`, and
//! byte-compares the `.json` and `.human.txt` goldens from
//! `U:\Git\al-sem\scripts\cli-b-goldens\prove\`.
//!
//! Additionally, verifies the dummy-doc case (ws-d8-commit-in-tx, NonExistentRoutineXYZ)
//! exits with code 2.
//!
//! ## Acceptance gate
//!
//! All 18 × 2 = 36 regular goldens + the 2 dummy-doc goldens (= 38 total) MUST
//! byte-match. This is ungated: divergence is either a Rust bug to fix or a model
//! difference to BLOCK — never a KNOWN_DIVERGENCES entry.
//!
//! ## Refresh (ignored)
//!
//! `#[ignore] refresh_goldens` shells `bun run scripts/dump-prove.ts` under
//! `AL_SEM_DIR` to regenerate the goldens.

use std::path::PathBuf;

use al_call_hierarchy::engine::l5::prove::run_prove_pipeline;

const VERSION_OVERRIDE: &str = "cli-b-v1";

/// al-sem repo root (override with `AL_SEM_DIR`).
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
        .join("prove")
}

/// (fixture, golden_slug, routine, question, expected_exit_code)
const PROVE_CORPUS: &[(&str, &str, &str, &str, u8)] = &[
    // may-commit
    (
        "ws-d34",
        "ws-d34.may-commit",
        "DirectCommitInLoop",
        "may-commit",
        0,
    ),
    (
        "ws-d14-dead-routine",
        "ws-d14-dead-routine.may-commit",
        "OnRun",
        "may-commit",
        0,
    ),
    (
        "ws-d35",
        "ws-d35.may-commit",
        "OnBeforePrintSafe",
        "may-commit",
        0,
    ),
    // commits-on-success-path
    (
        "ws-d8-commit-in-tx",
        "ws-d8-commit-in-tx.commits-on-success-path",
        "HandlePosted",
        "commits-on-success-path",
        0,
    ),
    (
        "ws-d14-dead-routine",
        "ws-d14-dead-routine.commits-on-success-path",
        "OnRun",
        "commits-on-success-path",
        0,
    ),
    (
        "ws-d34",
        "ws-d34.commits-on-success-path",
        "DirectCommitInLoop",
        "commits-on-success-path",
        0,
    ),
    // writes-table
    (
        "ws-d1-multi-caller",
        "ws-d1-multi-caller.writes-table-mc-customer",
        "CallerA",
        "writes-table:MC Customer",
        0,
    ),
    (
        "ws-d34",
        "ws-d34.writes-table-customer",
        "DirectCommitInLoop",
        "writes-table:Customer",
        0,
    ),
    (
        "ws-d35",
        "ws-d35.writes-table-customer",
        "OnBeforePrintSafe",
        "writes-table:Customer",
        0,
    ),
    // publishes-event
    (
        "ws-d8-commit-in-tx",
        "ws-d8-commit-in-tx.publishes-event-onafterpostsalesdoc",
        "PostSalesDoc",
        "publishes-event:onafterpostsalesdoc",
        0,
    ),
    (
        "ws-d34",
        "ws-d34.publishes-event-onafterpost",
        "DirectCommitInLoop",
        "publishes-event:OnAfterPost",
        0,
    ),
    (
        "ws-d35",
        "ws-d35.publishes-event-onafterpost",
        "OnBeforePrintSafe",
        "publishes-event:OnAfterPost",
        0,
    ),
    // reaches-ui
    (
        "ws-d35",
        "ws-d35.reaches-ui",
        "OnBeforePrintSafe",
        "reaches-ui",
        0,
    ),
    (
        "ws-d34",
        "ws-d34.reaches-ui",
        "DirectCommitInLoop",
        "reaches-ui",
        0,
    ),
    (
        "ws-txn-d47-pos-http-nocommit",
        "ws-txn-d47-pos-http-nocommit.reaches-ui",
        "SendAfterModify",
        "reaches-ui",
        0,
    ),
    // throws-error
    (
        "ws-d51-pos",
        "ws-d51-pos.throws-error",
        "PostThenError",
        "throws-error",
        0,
    ),
    (
        "ws-d34",
        "ws-d34.throws-error",
        "DirectCommitInLoop",
        "throws-error",
        0,
    ),
    (
        "ws-d35",
        "ws-d35.throws-error",
        "OnBeforePrintSafe",
        "throws-error",
        0,
    ),
    // dummy-doc case (exit 2)
    (
        "ws-d8-commit-in-tx",
        "dummy-doc.may-commit",
        "NonExistentRoutineXYZ",
        "may-commit",
        2,
    ),
];

// ---------------------------------------------------------------------------
// Diagnostic helpers
// ---------------------------------------------------------------------------

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

fn context_around(bytes: &[u8], pos: usize, radius: usize) -> String {
    let start = pos.saturating_sub(radius);
    let end = (pos + radius).min(bytes.len());
    String::from_utf8_lossy(&bytes[start..end])
        .replace('\n', "↵")
        .replace('\r', "↩")
}

// ---------------------------------------------------------------------------
// Per-entry runner
// ---------------------------------------------------------------------------

fn run_one(fixture: &str, routine: &str, question: &str) -> Result<(String, String, u8), String> {
    let fixture_dir = fixtures_dir().join(fixture);
    if !fixture_dir.is_dir() {
        return Err(format!(
            "fixture {fixture} not found at {}",
            fixture_dir.display()
        ));
    }

    match run_prove_pipeline(
        &fixture_dir,
        routine,
        question,
        VERSION_OVERRIDE,
        true, // deterministic
    ) {
        Ok(result) => Ok((result.json_text, result.human_text, result.exit_code)),
        Err(msg) => Err(msg),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn json_matches_goldens() {
    let gdir = goldens_dir();
    for (fixture, golden_slug, routine, question, expected_exit) in PROVE_CORPUS {
        let (got_json, _, actual_exit) = run_one(fixture, routine, question).unwrap_or_else(|e| {
            panic!("{fixture}/{routine}/{question}: run_prove_pipeline failed: {e}")
        });

        assert_eq!(
            actual_exit, *expected_exit,
            "{fixture}/{routine}/{question}: exit code mismatch: got {actual_exit}, want {expected_exit}"
        );

        let golden_path = gdir.join(format!("{golden_slug}.json"));
        let want = std::fs::read_to_string(&golden_path).unwrap_or_else(|_| {
            panic!(
                "{fixture}/{routine}/{question}: golden not found at {}",
                golden_path.display()
            )
        });

        if got_json != want {
            let got_b = got_json.as_bytes();
            let want_b = want.as_bytes();
            let pos = first_diff(got_b, want_b).unwrap_or(0);
            let got_ctx = context_around(got_b, pos, 120);
            let want_ctx = context_around(want_b, pos, 120);
            panic!(
                "{fixture}/{routine}/{question}: JSON golden mismatch at byte {pos}\n  got  (±120): {got_ctx:?}\n  want (±120): {want_ctx:?}\n  got len={}, want len={}",
                got_b.len(),
                want_b.len()
            );
        }
    }
}

#[test]
fn human_matches_goldens() {
    let gdir = goldens_dir();
    for (fixture, golden_slug, routine, question, _expected_exit) in PROVE_CORPUS {
        let (_, got_human, _) = run_one(fixture, routine, question).unwrap_or_else(|e| {
            panic!("{fixture}/{routine}/{question}: run_prove_pipeline failed: {e}")
        });

        let golden_path = gdir.join(format!("{golden_slug}.human.txt"));
        let want = std::fs::read_to_string(&golden_path).unwrap_or_else(|_| {
            panic!(
                "{fixture}/{routine}/{question}: human golden not found at {}",
                golden_path.display()
            )
        });

        if got_human != want {
            let got_b = got_human.as_bytes();
            let want_b = want.as_bytes();
            let pos = first_diff(got_b, want_b).unwrap_or(0);
            let got_ctx = context_around(got_b, pos, 120);
            let want_ctx = context_around(want_b, pos, 120);
            panic!(
                "{fixture}/{routine}/{question}: human golden mismatch at byte {pos}\n  got  (±120): {got_ctx:?}\n  want (±120): {want_ctx:?}\n  got len={}, want len={}",
                got_b.len(),
                want_b.len()
            );
        }
    }
}

#[test]
fn dummy_doc_exits_2() {
    let fixture = "ws-d8-commit-in-tx";
    let routine = "NonExistentRoutineXYZ";
    let question = "may-commit";

    let (_, _, exit) = run_one(fixture, routine, question)
        .unwrap_or_else(|e| panic!("dummy-doc: run_prove_pipeline failed: {e}"));

    assert_eq!(exit, 2, "dummy-doc case must exit 2 (routine not resolved)");
}

/// Refresh helper — regenerate the prove goldens via `bun run scripts/dump-prove.ts`.
/// Only run via `cargo test -- --ignored refresh_goldens`.
#[test]
#[ignore]
fn refresh_goldens() {
    let al_sem = al_sem_dir();
    let status = std::process::Command::new("bun")
        .arg("run")
        .arg("scripts/dump-prove.ts")
        .current_dir(&al_sem)
        .env("AL_SEM_VERSION_OVERRIDE", VERSION_OVERRIDE)
        .status()
        .expect("bun not found on PATH");
    assert!(
        status.success(),
        "bun run scripts/dump-prove.ts failed with status {status}"
    );
}
