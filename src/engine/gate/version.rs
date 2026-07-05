//! The engine's own driver-identity version string.
//!
//! This is the value the CLI reports as its version in output envelopes that carry
//! one (the JSON/events display field, SARIF `driver.version`): normally the crate's
//! own release version, overridable at runtime for byte-stable golden tests.
//!
//! The dependency-cache header's `analyzer` stamp is a SEPARATE, deliberately
//! decoupled concern — see [`crate::engine::gate::cache_prune::CACHE_ANALYZER_VERSION`].
//!
//! A handful of CLI subcommands (`policy check`, `digest`, `prove`, `fingerprint`,
//! `events fanout`/`chains`) still thread the legacy `DEFAULT_ALSEM_VERSION` const
//! directly rather than this module's [`driver_version`] — tracked separately, out
//! of scope for this pass.

/// Legacy fallback version string still used directly by a handful of CLI
/// subcommands (`policy check`, `digest`, `prove`, `fingerprint`, `events
/// fanout`/`chains`) that have not yet been migrated onto [`driver_version`].
/// Deliberately left as a literal (not `env!("CARGO_PKG_VERSION")`) so those
/// call sites are untouched by this decoupling; migrating them is tracked
/// separately.
pub const DEFAULT_ALSEM_VERSION: &str = "0.0.12";

/// The env-var name carrying the driver-version override.
const OVERRIDE_ENV_VAR: &str = "ALCH_DRIVER_VERSION_OVERRIDE";

/// Pure resolution: given the (optional) override value, return the effective version.
///   - `Some(v)` (the env var was set) → `v`.
///   - `None` → the crate's own release version (`CARGO_PKG_VERSION`).
///
/// This is the unit-testable core — no process-global env state, so tests do not race.
fn resolve_version(override_value: Option<&str>) -> String {
    match override_value {
        Some(v) => v.to_string(),
        None => env!("CARGO_PKG_VERSION").to_string(),
    }
}

/// Return the engine's driver-identity version string.
///
/// Resolution order:
///   1. `ALCH_DRIVER_VERSION_OVERRIDE` env var — if set, return its value.
///   2. `CARGO_PKG_VERSION` (the crate's own release version) — the honest default.
///
/// The env-var override exists so differential test harnesses can pin a stable
/// version in a golden envelope without baking a specific release version into the
/// golden file.
pub fn driver_version() -> String {
    resolve_version(std::env::var(OVERRIDE_ENV_VAR).ok().as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Serializes the tests that mutate the process-global env var so they do not race
    /// under cargo's parallel test threads.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    // --- pure-helper tests (no env state, never race) ---

    #[test]
    fn resolve_version_uses_override_when_present() {
        assert_eq!(
            resolve_version(Some("test-override-99.9.9")),
            "test-override-99.9.9"
        );
    }

    #[test]
    fn resolve_version_defaults_when_absent() {
        assert_eq!(resolve_version(None), env!("CARGO_PKG_VERSION"));
    }

    // --- thin integration tests of the real env read (serialized via ENV_LOCK) ---

    #[test]
    fn driver_version_reads_override_env_var() {
        let _guard = ENV_LOCK.lock().unwrap();
        // TODO: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::set_var(OVERRIDE_ENV_VAR, "test-override-99.9.9") };
        let v = driver_version();
        // TODO: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::remove_var(OVERRIDE_ENV_VAR) };
        assert_eq!(v, "test-override-99.9.9");
    }

    #[test]
    fn driver_version_defaults_when_env_unset() {
        let _guard = ENV_LOCK.lock().unwrap();
        // TODO: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::remove_var(OVERRIDE_ENV_VAR) };
        let v = driver_version();
        assert_eq!(v, env!("CARGO_PKG_VERSION"));
    }
}
