//! Where per-user state lives, and how the pre-rename locations are still honoured.
//!
//! The crate was called `al-call-hierarchy` until the 2026-08-07 rename, and its
//! per-user state lived in `~/.al-call-hierarchy/` with per-workspace overrides in
//! `<workspace>/.al-call-hierarchy.json`. The analyzer, meanwhile, had always used
//! `~/.al-sem/cache/`. This module is the single place that knows both spellings, so
//! the convergence on `~/.al-sem/` happens once rather than in each consumer.
//!
//! **The rule: WRITE the current path, READ the current path first and the legacy one
//! second.** An existing install therefore keeps its thresholds and its anonymous
//! installation id across the rename instead of silently reverting to defaults. Nothing
//! is ever written back to a legacy location.
//!
//! [`resolve_for_read`] deliberately returns the CURRENT path when neither file exists,
//! so a caller that reports "no config found" names the location a user should create
//! rather than the retired one.
//!
//! The legacy constants are load-bearing until the fallback is retired, which needs a
//! deprecation window long enough that no live install still holds only the old paths.

use std::path::{Path, PathBuf};

/// The per-user state directory under `$HOME`, shared with the analyzer's `cache/`.
pub const STATE_DIR: &str = ".al-sem";

/// The pre-rename state directory. Read-only — see the module doc.
pub const LEGACY_STATE_DIR: &str = ".al-call-hierarchy";

/// The per-workspace config file name.
pub const WORKSPACE_CONFIG: &str = ".al-sem.json";

/// The pre-rename per-workspace config file name. Read-only.
pub const LEGACY_WORKSPACE_CONFIG: &str = ".al-call-hierarchy.json";

/// `~/.al-sem/<leaf>` — the path to WRITE. `None` when the home directory is unknown.
pub fn state_path(leaf: &str) -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(STATE_DIR).join(leaf))
}

/// `~/.al-call-hierarchy/<leaf>` — the legacy path, for reads only.
pub fn legacy_state_path(leaf: &str) -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(LEGACY_STATE_DIR).join(leaf))
}

/// The state path to READ: current if it exists, else legacy if THAT exists, else
/// current.
pub fn state_path_for_read(leaf: &str) -> Option<PathBuf> {
    let current = state_path(leaf)?;
    let legacy = legacy_state_path(leaf);
    Some(resolve_for_read(current, legacy))
}

/// The per-workspace config path to READ, under the same current-then-legacy rule.
pub fn workspace_config_for_read(workspace_root: &Path) -> PathBuf {
    resolve_for_read(
        workspace_root.join(WORKSPACE_CONFIG),
        Some(workspace_root.join(LEGACY_WORKSPACE_CONFIG)),
    )
}

/// The current-then-legacy choice itself, taking paths rather than reading `$HOME` so
/// it is testable without touching the real home directory.
///
/// Returns `current` unless `current` is absent AND `legacy` is present.
pub fn resolve_for_read(current: PathBuf, legacy: Option<PathBuf>) -> PathBuf {
    if current.exists() {
        return current;
    }
    match legacy {
        Some(l) if l.exists() => l,
        _ => current,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // The preconditions below are hand-built (a file is created, or deliberately not
    // created) rather than produced by another part of the engine, so these tests
    // survive any later change to how the real paths are computed.

    #[test]
    fn current_wins_when_both_exist() {
        let dir = TempDir::new().unwrap();
        let current = dir.path().join("current.json");
        let legacy = dir.path().join("legacy.json");
        std::fs::write(&current, b"{}").unwrap();
        std::fs::write(&legacy, b"{}").unwrap();

        assert_eq!(resolve_for_read(current.clone(), Some(legacy)), current);
    }

    #[test]
    fn legacy_is_used_when_current_is_absent() {
        let dir = TempDir::new().unwrap();
        let current = dir.path().join("current.json");
        let legacy = dir.path().join("legacy.json");
        std::fs::write(&legacy, b"{}").unwrap();
        assert!(!current.exists(), "precondition: no current-path file");

        assert_eq!(
            resolve_for_read(current, Some(legacy.clone())),
            legacy,
            "an install that predates the rename must keep being read"
        );
    }

    #[test]
    fn current_is_returned_when_neither_exists() {
        let dir = TempDir::new().unwrap();
        let current = dir.path().join("current.json");
        let legacy = dir.path().join("legacy.json");
        assert!(
            !current.exists() && !legacy.exists(),
            "precondition: neither"
        );

        assert_eq!(
            resolve_for_read(current.clone(), Some(legacy)),
            current,
            "a 'not found' message must name the path a user should create"
        );
    }

    #[test]
    fn missing_legacy_candidate_is_not_an_error() {
        let dir = TempDir::new().unwrap();
        let current = dir.path().join("current.json");
        assert_eq!(resolve_for_read(current.clone(), None), current);
    }

    #[test]
    fn workspace_config_prefers_the_current_name() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join(WORKSPACE_CONFIG), b"{}").unwrap();
        std::fs::write(dir.path().join(LEGACY_WORKSPACE_CONFIG), b"{}").unwrap();

        assert_eq!(
            workspace_config_for_read(dir.path()),
            dir.path().join(WORKSPACE_CONFIG)
        );
    }

    #[test]
    fn workspace_config_falls_back_to_the_legacy_name() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join(LEGACY_WORKSPACE_CONFIG), b"{}").unwrap();

        assert_eq!(
            workspace_config_for_read(dir.path()),
            dir.path().join(LEGACY_WORKSPACE_CONFIG)
        );
    }

    #[test]
    fn the_two_names_are_distinct() {
        // Guards the copy-paste failure where both constants end up the same string,
        // which would make every fallback test above pass vacuously.
        assert_ne!(STATE_DIR, LEGACY_STATE_DIR);
        assert_ne!(WORKSPACE_CONFIG, LEGACY_WORKSPACE_CONFIG);
    }
}
