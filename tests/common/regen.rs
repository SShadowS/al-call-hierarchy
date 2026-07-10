//! Shared `REGEN_TEMP_GOLDENS` value-gate helper for golden-regenerating tests.
//!
//! Doctrine (CLAUDE.md, `al-sem retired / Rust-owned`): every Rust-owned golden
//! regenerates via `REGEN_TEMP_GOLDENS=1 cargo test`. Before Task T0.6, every
//! call site checked PRESENCE (`std::env::var("REGEN_TEMP_GOLDENS").is_ok()` /
//! `.is_err()`), so `REGEN_TEMP_GOLDENS=0 cargo test` silently REWROTE every
//! golden while reporting green — `is_ok()` is true for ANY set value,
//! including `"0"`. This helper checks the VALUE: only the exact string `"1"`
//! regenerates. `"0"`, the empty string, and anything else fall through to the
//! normal assert path — fail-closed toward asserting, never toward silently
//! rewriting goldens.
//!
//! `cargo test` compiles each `tests/*.rs` file as its own separate
//! binary/crate, so a `mod` defined in one cannot be `use`d from another.
//! This file is included via `#[path = "common/regen.rs"] mod regen;` by
//! every golden-asserting test binary (Task T0.6) so there is exactly one
//! implementation. (The single `#[cfg(test)]` unit-test regen check that
//! lives inside the library itself, in `src/parser.rs`, cannot reach across
//! the `src/`/`tests/` boundary via `#[path]` without an awkward relative
//! reference into `tests/`; it mirrors this same value-semantics contract
//! with its own tiny colocated helper instead — see the comment there.)

/// Pure resolution core: given the raw env-var value (if set), decide whether
/// regen mode is active. Only the exact value `"1"` regenerates.
fn resolve_regen_mode(raw: Option<&str>) -> bool {
    raw == Some("1")
}

/// Returns `true` iff `REGEN_TEMP_GOLDENS=1` — the ONLY value that
/// regenerates goldens. Any other value (including `"0"` or an empty string)
/// returns `false`, taking the normal assert path.
///
/// Use this instead of `std::env::var("REGEN_TEMP_GOLDENS").is_ok()` /
/// `.is_err()`, which are presence-based and wrongly treat
/// `REGEN_TEMP_GOLDENS=0` as "regenerate."
#[allow(dead_code)] // not every including binary calls both helpers below
pub fn regen_mode() -> bool {
    resolve_regen_mode(std::env::var("REGEN_TEMP_GOLDENS").ok().as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- pure-helper tests (no env state, never race) ---
    //
    // No test in this module mutates `REGEN_TEMP_GOLDENS`: this file is
    // `#[path]`-included into ~40 golden-asserting test binaries, and a
    // mutating test here would run concurrently with those binaries' own
    // (unlocked) `regen_mode()` reads — racing a real golden gate into its
    // regen branch and silently rewriting a committed golden. The value
    // semantics of `regen_mode()` are a trivial composition over
    // `resolve_regen_mode`, so the pure tests below fully cover it without
    // touching the process environment.

    #[test]
    fn resolve_regen_mode_true_only_for_exact_one() {
        assert!(resolve_regen_mode(Some("1")));
    }

    #[test]
    fn resolve_regen_mode_false_for_zero() {
        assert!(!resolve_regen_mode(Some("0")));
    }

    #[test]
    fn resolve_regen_mode_false_for_empty_string() {
        assert!(!resolve_regen_mode(Some("")));
    }

    #[test]
    fn resolve_regen_mode_false_for_absent() {
        assert!(!resolve_regen_mode(None));
    }

    #[test]
    fn resolve_regen_mode_false_for_other_truthy_looking_values() {
        assert!(!resolve_regen_mode(Some("true")));
        assert!(!resolve_regen_mode(Some("yes")));
        assert!(!resolve_regen_mode(Some("2")));
        assert!(!resolve_regen_mode(Some(" 1")));
        assert!(!resolve_regen_mode(Some("1 ")));
    }
}
