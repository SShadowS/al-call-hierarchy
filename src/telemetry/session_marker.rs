//! Crash-detection marker.
//!
//! Created at startup, deleted on graceful shutdown. Presence at startup
//! signals that the previous session terminated abnormally (SIGKILL, OS crash,
//! power loss, exporter hang past shutdown budget).

use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

fn default_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".al-call-hierarchy").join("session.lock"))
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
        Some(p) => record_at(&p),
        None => MarkerStatus {
            previous_session_unclean: false,
            created: false,
        },
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
}
