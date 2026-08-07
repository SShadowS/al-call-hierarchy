//! Crash-detection marker.
//!
//! Created at startup, deleted on graceful shutdown. Presence at startup
//! signals that the previous session terminated abnormally (SIGKILL, OS crash,
//! power loss, exporter hang past shutdown budget).

use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

const LEAF: &str = "session.lock";

/// `~/.al-sem/session.lock` — where this session's marker is written.
fn default_path() -> Option<PathBuf> {
    crate::state_paths::state_path(LEAF)
}

/// `~/.al-call-hierarchy/session.lock` — a marker left by a pre-rename session.
fn legacy_path() -> Option<PathBuf> {
    crate::state_paths::legacy_state_path(LEAF)
}

/// Result of writing the marker at startup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MarkerStatus {
    /// `true` if the marker file existed before this session started.
    pub previous_session_unclean: bool,
    /// `true` if we successfully wrote the marker for this session.
    pub created: bool,
}

/// Record this session's marker. Returns whether the previous session was unclean.
pub fn record_session_start() -> MarkerStatus {
    match default_path() {
        Some(p) => record_start_at(&p, legacy_path().as_deref()),
        None => MarkerStatus {
            previous_session_unclean: false,
            created: false,
        },
    }
}

/// [`record_session_start`] with both paths injected.
///
/// A marker at the pre-rename location means a session crashed before the 2026-08-07
/// rename and never got to clean up. That is exactly the signal this marker exists to
/// carry, so it counts as an unclean previous session — and the stale file is then
/// removed, because it is OUR marker under a name we retired, not a user's data. Unlike
/// the installation id, nothing is migrated: the marker's whole meaning is "the previous
/// session of this install", so once consumed it has no further use.
pub fn record_start_at(path: &Path, legacy: Option<&Path>) -> MarkerStatus {
    let legacy_marker = legacy.is_some_and(|l| l.exists());
    if legacy_marker && let Some(l) = legacy {
        clean_shutdown_at(l);
    }

    let status = record_at(path);
    MarkerStatus {
        previous_session_unclean: status.previous_session_unclean || legacy_marker,
        created: status.created,
    }
}

pub fn record_at(path: &Path) -> MarkerStatus {
    let previously_existed = path.exists();
    let created = match write_marker(path) {
        Ok(()) => true,
        Err(e) => {
            log::warn!(
                "telemetry: failed to write session marker at {}: {}",
                path.display(),
                e
            );
            false
        }
    };
    MarkerStatus {
        previous_session_unclean: previously_existed,
        created,
    }
}

/// Delete the marker. Called on graceful shutdown after summary export.
pub fn record_clean_shutdown() {
    if let Some(p) = default_path() {
        clean_shutdown_at(&p);
    }
}

pub fn clean_shutdown_at(path: &Path) {
    if let Err(e) = fs::remove_file(path)
        && e.kind() != std::io::ErrorKind::NotFound
    {
        log::warn!(
            "telemetry: failed to remove session marker at {}: {}",
            path.display(),
            e
        );
    }
}

fn write_marker(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating dir {}", parent.display()))?;
    }
    fs::write(path, b"").with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn first_session_no_previous_marker() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("session.lock");
        let status = record_at(&path);
        assert!(!status.previous_session_unclean);
        assert!(status.created);
        assert!(path.exists());
    }

    #[test]
    fn second_session_without_clean_shutdown_detects_unclean() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("session.lock");
        let _ = record_at(&path);
        // Simulated crash: no clean_shutdown_at call.
        let status = record_at(&path);
        assert!(status.previous_session_unclean);
        assert!(status.created);
    }

    #[test]
    fn clean_shutdown_removes_marker() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("session.lock");
        let _ = record_at(&path);
        clean_shutdown_at(&path);
        assert!(!path.exists());
    }

    #[test]
    fn clean_shutdown_is_idempotent() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("session.lock");
        clean_shutdown_at(&path); // file doesn't exist; must not panic
        clean_shutdown_at(&path);
    }

    // Each precondition below is a file created (or not created) literally in the test,
    // so none of them depends on `record_at` to produce the state under test.

    #[test]
    fn a_marker_left_by_a_pre_rename_session_still_reports_the_crash() {
        let dir = TempDir::new().unwrap();
        let current = dir.path().join("current.lock");
        let legacy = dir.path().join("legacy.lock");
        fs::write(&legacy, b"").unwrap();
        assert!(!current.exists(), "precondition: no marker at the new path");

        let status = record_start_at(&current, Some(&legacy));

        assert!(
            status.previous_session_unclean,
            "a crash that happened before the rename must not be swallowed by it"
        );
        assert!(status.created);
        assert!(
            !legacy.exists(),
            "our own retired marker is consumed, not left to report forever"
        );
    }

    #[test]
    fn no_marker_anywhere_reports_a_clean_previous_session() {
        let dir = TempDir::new().unwrap();
        let current = dir.path().join("current.lock");
        let legacy = dir.path().join("legacy.lock");
        assert!(
            !current.exists() && !legacy.exists(),
            "precondition: neither"
        );

        let status = record_start_at(&current, Some(&legacy));

        assert!(!status.previous_session_unclean);
        assert!(status.created);
        assert!(current.exists());
    }

    #[test]
    fn a_current_marker_alone_still_reports_the_crash() {
        let dir = TempDir::new().unwrap();
        let current = dir.path().join("current.lock");
        let legacy = dir.path().join("legacy.lock");
        fs::write(&current, b"").unwrap();

        let status = record_start_at(&current, Some(&legacy));

        assert!(status.previous_session_unclean);
    }
}
