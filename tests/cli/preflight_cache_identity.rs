//! The preflight verdict cache must never change output.
//!
//! `alsem analyze` output is byte-gated, and the cached `FreshCoverage` payload
//! is formatter-visible (`opaque_apps` flows into the JSON coverage block). So
//! a COLD run, a WARM run (served from cache) and a CACHE-DISABLED run must all
//! produce identical bytes. Nothing else in the suite covers the warm path:
//! every golden family runs with the cache in whatever state the runner left it.

use std::process::Command;

fn run(ws: &str, cache_dir: Option<&std::path::Path>, disabled: bool) -> String {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_alsem"));
    cmd.args(["analyze", ws, "--format", "json", "--deterministic"]);
    match cache_dir {
        Some(d) => {
            cmd.env("ALSEM_PREFLIGHT_CACHE_DIR", d);
        }
        None => {
            cmd.env_remove("ALSEM_PREFLIGHT_CACHE_DIR");
        }
    }
    if disabled {
        cmd.env("ALSEM_NO_PREFLIGHT_CACHE", "1");
    } else {
        cmd.env_remove("ALSEM_NO_PREFLIGHT_CACHE");
    }
    let out = cmd.output().expect("alsem runs");
    assert!(out.status.success(), "alsem exited {:?}", out.status);
    String::from_utf8(out.stdout).expect("stdout is utf-8")
}

#[test]
fn cold_warm_and_disabled_runs_are_byte_identical() {
    let ws = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/r0-corpus/ws-d2-uncertain"
    );
    let cache = tempfile::tempdir().expect("tempdir");

    // COLD: the cache dir is empty, so this run computes and then stores.
    let cold = run(ws, Some(cache.path()), false);

    // Precondition, stated executably: the cold run actually WROTE an entry, so
    // the warm run below is genuinely served and not a second cold run. Without
    // this the test would pass vacuously if caching silently stopped working.
    let entries = std::fs::read_dir(cache.path())
        .expect("cache dir readable")
        .count();
    assert_eq!(
        entries, 1,
        "the cold run must persist exactly one cache entry"
    );

    // WARM: same dir, entry present.
    let warm = run(ws, Some(cache.path()), false);

    // DISABLED: cache bypassed entirely.
    let disabled = run(ws, None, true);

    assert_eq!(
        cold, warm,
        "a warm cache run must be byte-identical to a cold one"
    );
    assert_eq!(
        cold, disabled,
        "a cache-disabled run must be byte-identical to a cached one"
    );
}
