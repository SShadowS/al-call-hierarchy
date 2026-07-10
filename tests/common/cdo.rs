//! Shared CDO-workspace gating helper for CDO-env-gated tests.
//!
//! `CDO_WS` points at a real Business Central workspace tree that only
//! exists on machines with access to it (CI cannot reach it). Tests gated on
//! it must skip silently in the common case (no `CDO_WS`, e.g. any CI run or
//! a contributor's machine without the tree) but fail LOUDLY when
//! `ENFORCE_CDO_WS=1` is set and the workspace is missing — that combination
//! marks a gated/scheduled run (see `scripts/cdo-gate`) where losing
//! `CDO_WS` must never silently pass.
//!
//! `cargo test` compiles each `tests/*.rs` file as its own separate
//! binary/crate, so a `mod` defined in one cannot be `use`d from another.
//! This file is included via `#[path = "common/cdo.rs"] mod cdo;` by every
//! test binary that needs the helper (`program_resolve_harness.rs`,
//! `program_graph.rs`, `snapshot_robustness.rs`) so there is exactly one
//! implementation (Task T0.2).

/// Returns `CDO_WS` as a `PathBuf` when set and pointing at an existing
/// path; otherwise `None` — UNLESS `ENFORCE_CDO_WS=1`, in which case a
/// missing/invalid `CDO_WS` panics (naming the current test via its thread
/// name, which libtest sets to the test's path) instead of returning `None`.
///
/// Callers use the standard skip idiom:
/// ```ignore
/// let Some(ws) = cdo_ws_or_enforce() else { return };
/// ```
/// Under `ENFORCE_CDO_WS=1` with a missing workspace, that `else` arm is
/// unreachable because this function panics first — no fail-open.
pub fn cdo_ws_or_enforce() -> Option<std::path::PathBuf> {
    let ws = std::env::var_os("CDO_WS")
        .map(std::path::PathBuf::from)
        .filter(|p| p.exists());
    if ws.is_none() {
        assert!(
            std::env::var("ENFORCE_CDO_WS").as_deref() != Ok("1"),
            "ENFORCE_CDO_WS=1 but CDO_WS is unset or does not point at an existing path (test: {})",
            std::thread::current().name().unwrap_or("<unknown test>")
        );
    }
    ws
}
