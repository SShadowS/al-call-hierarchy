//! perf_trace Task 4 acceptance gate — Jacobi per-pass counters vs per-SCC span.
//!
//! Root-cause regression: `run_one_scc` (`src/engine/l4/summary_runner.rs`)
//! originally gated its `jacobi.pass` per-pass counters behind the SAME
//! `ALSEM_TRACE_SCC_MIN` threshold as the `jacobi.scc.*` span. That threshold
//! exists to bound the SPAN's verbosity (a real corpus can have many small
//! recursive SCCs, e.g. plain self-recursion, and a span per one would bloat
//! the trace) — it was never meant to also suppress the cheap per-pass
//! counters, which are the population data the decisive-experiment plan reads
//! (spec `docs/superpowers/specs/2026-07-18-tracing-infra.md`: "the population
//! instrumentation already lands with Detail::Jacobi"). Discovered via a REAL
//! `ALSEM_TRACE_DETAIL=jacobi` run on the DO workspace (Continia Document
//! Output): DO's largest SCC has exactly 1 member (self-recursion; 8 such
//! recursive singletons, `scc_stats.max_scc == 1`), so with the default
//! `ALSEM_TRACE_SCC_MIN=100` every span AND every counter was silently absent
//! — not just the span, contradicting the spec's stated Jacobi-tier population
//! guarantee. Fixed by splitting the single `trace_jacobi` gate into
//! `trace_jacobi` (tier-enabled, still gates the per-pass counters) and a
//! separate `trace_scc_span` (tier-enabled AND size >= SCC_MIN, gates ONLY the
//! span).
//!
//! `ws-recursive` (`tests/r0-corpus/ws-recursive/src/cycle.al`) is a real
//! 2-member mutually-recursive SCC (`Ping` <-> `Pong`). Setting
//! `ALSEM_TRACE_SCC_MIN=3` (above the SCC's size) proves the decoupling: the
//! per-SCC span must be ABSENT (2 < 3) while `jacobi.pass` counters must be
//! PRESENT (the SCC is genuinely recursive, tier is enabled).

use std::path::PathBuf;
use std::process::Command;

fn alsem_bin() -> &'static str {
    env!("CARGO_BIN_EXE_alsem")
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("r0-corpus")
        .join(name)
}

fn trace_events(path: &std::path::Path) -> Vec<serde_json::Value> {
    let txt = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("read trace file {}: {e}", path.display()));
    let v: serde_json::Value = serde_json::from_str(&txt).unwrap_or_else(|e| {
        panic!(
            "trace file {} is not valid JSON: {e}\n{txt}",
            path.display()
        )
    });
    v.as_array()
        .unwrap_or_else(|| panic!("trace root must be an array: {}", path.display()))
        .clone()
}

#[test]
fn jacobi_pass_counters_fire_below_scc_min_while_span_stays_gated() {
    let ws = fixture("ws-recursive");
    assert!(ws.is_dir(), "fixture missing: {}", ws.display());

    let dir = tempfile::tempdir().expect("tempdir");
    let trace_path = dir.path().join("ws-recursive-jacobi.trace.json");

    let out = Command::new(alsem_bin())
        .arg("analyze")
        .arg(&ws)
        .arg("--format")
        .arg("json")
        .arg("--deterministic")
        .env("ALSEM_TRACE", "1")
        .env("ALSEM_TRACE_DETAIL", "jacobi")
        .env("ALSEM_TRACE_FILE", &trace_path)
        // Above the Ping/Pong SCC's 2-member size: the span must NOT open, but
        // (per the fix) the per-pass counters must still fire.
        .env("ALSEM_TRACE_SCC_MIN", "3")
        .output()
        .expect("spawn alsem analyze");
    assert!(
        out.status.success(),
        "alsem analyze ws-recursive must exit 0: stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        trace_path.is_file(),
        "ALSEM_TRACE=1 must create the trace file at {}",
        trace_path.display()
    );

    let events = trace_events(&trace_path);
    let names: Vec<&str> = events.iter().filter_map(|e| e["name"].as_str()).collect();

    assert!(
        names.contains(&"jacobi.pass"),
        "jacobi.pass counters must fire for a genuinely recursive 2-member SCC \
         even when it is below ALSEM_TRACE_SCC_MIN=3 (population data must not \
         depend on SCC size); event names: {names:?}"
    );
    assert!(
        !names.iter().any(|n| n.starts_with("jacobi.scc.")),
        "the per-SCC span must stay gated on ALSEM_TRACE_SCC_MIN (verbosity \
         control only): the 2-member SCC is below the 3-member floor; event \
         names: {names:?}"
    );
}

/// Companion: at `ALSEM_TRACE_SCC_MIN=1` (at/below the SCC's size) BOTH the
/// span and the per-pass counters must fire — the size gate only ever removes
/// the span, never the population data.
#[test]
fn jacobi_span_also_fires_once_scc_min_is_at_or_below_actual_size() {
    let ws = fixture("ws-recursive");
    assert!(ws.is_dir(), "fixture missing: {}", ws.display());

    let dir = tempfile::tempdir().expect("tempdir");
    let trace_path = dir.path().join("ws-recursive-jacobi-min1.trace.json");

    let out = Command::new(alsem_bin())
        .arg("analyze")
        .arg(&ws)
        .arg("--format")
        .arg("json")
        .arg("--deterministic")
        .env("ALSEM_TRACE", "1")
        .env("ALSEM_TRACE_DETAIL", "jacobi")
        .env("ALSEM_TRACE_FILE", &trace_path)
        .env("ALSEM_TRACE_SCC_MIN", "1")
        .output()
        .expect("spawn alsem analyze");
    assert!(
        out.status.success(),
        "alsem analyze ws-recursive must exit 0: stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );

    let events = trace_events(&trace_path);
    let names: Vec<&str> = events.iter().filter_map(|e| e["name"].as_str()).collect();

    assert!(
        names.contains(&"jacobi.pass"),
        "jacobi.pass counters must fire: {names:?}"
    );
    assert!(
        names.iter().any(|n| n.starts_with("jacobi.scc.")),
        "the per-SCC span must fire once SCC_MIN <= the SCC's actual size: {names:?}"
    );
}
