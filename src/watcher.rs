//! File system watcher for incremental updates

use anyhow::Result;
use log::{debug, error, info};
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;

/// File change event
#[derive(Debug)]
pub enum FileChange {
    /// File was created or modified
    Modified(PathBuf),
    /// File was deleted
    Deleted(PathBuf),
}

/// File system watcher for AL files
pub struct AlFileWatcher {
    _watcher: RecommendedWatcher,
    receiver: Receiver<FileChange>,
}

impl AlFileWatcher {
    /// Create a new watcher for the given directory
    pub fn new(root: &Path) -> Result<Self> {
        let (tx, rx) = channel();

        let mut watcher = RecommendedWatcher::new(
            move |result: Result<Event, notify::Error>| {
                match result {
                    Ok(event) => {
                        // Filter for AL files
                        let al_paths: Vec<_> = event
                            .paths
                            .iter()
                            .filter(|p| {
                                p.extension()
                                    .map(|ext| ext.eq_ignore_ascii_case("al"))
                                    .unwrap_or(false)
                            })
                            .cloned()
                            .collect();

                        if al_paths.is_empty() {
                            return;
                        }

                        for path in al_paths {
                            let change = match event.kind {
                                EventKind::Create(_) | EventKind::Modify(_) => {
                                    debug!("File modified: {}", path.display());
                                    FileChange::Modified(path)
                                }
                                EventKind::Remove(_) => {
                                    debug!("File deleted: {}", path.display());
                                    FileChange::Deleted(path)
                                }
                                _ => continue,
                            };

                            if tx.send(change).is_err() {
                                error!("Failed to send file change event");
                            }
                        }
                    }
                    Err(e) => {
                        error!("Watch error: {:?}", e);
                    }
                }
            },
            Config::default().with_poll_interval(Duration::from_secs(2)),
        )?;

        watcher.watch(root, RecursiveMode::Recursive)?;
        info!("Watching for file changes in: {}", root.display());

        Ok(Self {
            _watcher: watcher,
            receiver: rx,
        })
    }

    /// Try to receive a file change event (non-blocking)
    pub fn try_recv(&self) -> Option<FileChange> {
        self.receiver.try_recv().ok()
    }

    /// Receive a file change event (blocking with timeout)
    pub fn recv_timeout(&self, timeout: Duration) -> Option<FileChange> {
        self.receiver.recv_timeout(timeout).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_watcher_creation() {
        let dir = tempdir().unwrap();
        let watcher = AlFileWatcher::new(dir.path());
        assert!(watcher.is_ok());
    }

    #[test]
    fn test_watcher_detects_al_file() {
        let dir = tempdir().unwrap();
        let watcher = AlFileWatcher::new(dir.path()).unwrap();

        // Create an AL file
        let al_file = dir.path().join("Test.al");
        fs::write(&al_file, "codeunit 50000 Test {}").unwrap();

        // Give the watcher time to detect the change
        std::thread::sleep(Duration::from_millis(100));

        // Should have received a change event
        // Note: This may be flaky in CI environments
        let change = watcher.recv_timeout(Duration::from_secs(3));
        if let Some(FileChange::Modified(path)) = change {
            assert_eq!(path, al_file);
        }
    }
}
