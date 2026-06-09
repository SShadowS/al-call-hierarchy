//! al-sem version string for the Rust engine — mirrors the TS `ANALYZER_VERSION`
//! in `src/providers/discover.ts`.
//!
//! We deliberately do NOT use `env!("CARGO_PKG_VERSION")` — that's the Rust crate
//! version (which tracks the engine release cycle, not the al-sem semantic version).
//! The canonical al-sem version is pinned to the `al-sem` package.json `version`
//! field at build time and overridable at runtime via `AL_SEM_VERSION_OVERRIDE`.

/// The pinned al-sem version this Rust engine is byte-parity with.
/// Must match al-sem's `package.json` `"version"` field.
pub const DEFAULT_ALSEM_VERSION: &str = "0.0.12";

/// The env-var name carrying the al-sem version override (mirrors the TS env var).
const OVERRIDE_ENV_VAR: &str = "AL_SEM_VERSION_OVERRIDE";

/// Pure resolution: given the (optional) override value, return the effective version.
///   - `Some(v)` (the env var was set) → `v`.
///   - `None` → `DEFAULT_ALSEM_VERSION`.
/// This is the unit-testable core — no process-global env state, so tests do not race.
fn resolve_version(override_value: Option<&str>) -> String {
    match override_value {
        Some(v) => v.to_string(),
        None => DEFAULT_ALSEM_VERSION.to_string(),
    }
}

/// Return the al-sem version string.
///
/// Resolution order (mirrors the TS `ANALYZER_VERSION` / env-override convention):
///   1. `AL_SEM_VERSION_OVERRIDE` env var — if set, return its value.
///   2. `DEFAULT_ALSEM_VERSION` constant.
///
/// The env-var override is used by the differential test harness to pin a stable
/// `alsemVersion` in the JSON envelope without baking a specific version into the
/// golden file.
pub fn alsem_version() -> String {
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
        assert_eq!(resolve_version(None), DEFAULT_ALSEM_VERSION);
    }

    // --- thin integration tests of the real env read (serialized via ENV_LOCK) ---

    #[test]
    fn alsem_version_reads_override_env_var() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var(OVERRIDE_ENV_VAR, "test-override-99.9.9");
        let v = alsem_version();
        std::env::remove_var(OVERRIDE_ENV_VAR);
        assert_eq!(v, "test-override-99.9.9");
    }

    #[test]
    fn alsem_version_defaults_when_env_unset() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var(OVERRIDE_ENV_VAR);
        let v = alsem_version();
        assert_eq!(v, DEFAULT_ALSEM_VERSION);
    }
}
