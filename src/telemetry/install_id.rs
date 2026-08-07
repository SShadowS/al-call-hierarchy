//! Per-installation salt management.
//!
//! Stored at `~/.al-sem/installation-id` (32 random bytes). Generated on first use;
//! persists across runs.
//!
//! An install that predates the 2026-08-07 rename has its salt at
//! `~/.al-call-hierarchy/installation-id`. That file is read and then MIGRATED to the
//! current location, so the anonymous identity survives the rename rather than a fresh
//! salt making one install look like two. The legacy copy is left in place — deleting a
//! user's file to tidy up our own rename is not this code's business.

use crate::telemetry::hash::Salt;
use anyhow::{Context, Result};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

const SALT_BYTES: usize = 32;

const LEAF: &str = "installation-id";

/// Resolve `~/.al-sem/installation-id` — the path this code WRITES.
fn default_path() -> Option<PathBuf> {
    crate::state_paths::state_path(LEAF)
}

/// Resolve `~/.al-call-hierarchy/installation-id` — the pre-rename path, read only.
fn legacy_path() -> Option<PathBuf> {
    crate::state_paths::legacy_state_path(LEAF)
}

/// Load existing salt, or generate-and-persist a fresh one.
/// Falls back to an in-memory salt if the filesystem is unwritable.
pub fn load_or_create() -> (Salt, bool /* persisted */) {
    let Some(path) = default_path() else {
        return (random_salt(), false);
    };
    load_or_create_migrating(&path, legacy_path().as_deref())
}

/// [`load_or_create`] with both paths injected, so the migration is testable without
/// touching the real home directory.
///
/// Reads `path`; failing that, reads `legacy` and writes the salt it found to `path`;
/// failing both, mints a fresh salt. The `persisted` flag reports whether the salt in
/// use is now backed by `path`.
pub fn load_or_create_migrating(path: &Path, legacy: Option<&Path>) -> (Salt, bool) {
    if let Ok(salt) = read_salt(path) {
        return (salt, true);
    }
    if let Some(legacy) = legacy
        && let Ok(salt) = read_salt(legacy)
    {
        log::info!(
            "telemetry: carrying the installation id forward from {} to {}",
            legacy.display(),
            path.display()
        );
        if let Err(e) = persist_salt(path, &salt) {
            log::warn!(
                "telemetry: failed to migrate installation-id to {}: {}. Continuing with the existing id, unpersisted.",
                path.display(),
                e
            );
            return (salt, false);
        }
        return (salt, true);
    }
    load_or_create_at(path)
}

pub fn load_or_create_at(path: &Path) -> (Salt, bool) {
    if let Ok(salt) = read_salt(path) {
        return (salt, true);
    }
    let salt = random_salt();
    if let Err(e) = persist_salt(path, &salt) {
        log::warn!(
            "telemetry: failed to persist installation-id at {}: {}. Using in-memory salt for this session.",
            path.display(),
            e
        );
        return (salt, false);
    }
    (salt, true)
}

fn read_salt(path: &Path) -> Result<Salt> {
    let mut f = fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut buf = [0u8; SALT_BYTES];
    f.read_exact(&mut buf)
        .with_context(|| format!("reading 32 bytes from {}", path.display()))?;
    Ok(buf)
}

fn persist_salt(path: &Path, salt: &Salt) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating dir {}", parent.display()))?;
    }
    fs::write(path, salt).with_context(|| format!("writing {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

fn random_salt() -> Salt {
    let mut salt = [0u8; SALT_BYTES];
    getrandom_compat(&mut salt);
    salt
}

#[cfg(not(test))]
fn getrandom_compat(buf: &mut [u8]) {
    // Fall back to time-based weak entropy if blake3's RNG isn't desired.
    // We pull from std with a small mix; for stronger entropy use a real RNG.
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut h = blake3::Hasher::new();
    h.update(&nanos.to_le_bytes());
    h.update(&std::process::id().to_le_bytes());
    let mut reader = h.finalize_xof();
    reader.fill(buf);
}

#[cfg(test)]
fn getrandom_compat(buf: &mut [u8]) {
    // Tests need determinism; use the address of `buf` for variability.
    let seed = buf.as_ptr() as usize;
    let mut h = blake3::Hasher::new();
    h.update(&seed.to_le_bytes());
    let mut reader = h.finalize_xof();
    reader.fill(buf);
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn first_call_creates_and_persists_salt() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("salt");
        let (salt, persisted) = load_or_create_at(&path);
        assert!(persisted);
        assert!(path.exists());
        assert_eq!(salt.len(), 32);
    }

    #[test]
    fn second_call_reuses_existing_salt() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("salt");
        let (s1, _) = load_or_create_at(&path);
        let (s2, _) = load_or_create_at(&path);
        assert_eq!(s1, s2);
    }

    #[test]
    fn corrupt_short_file_falls_back_to_new_salt() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("salt");
        fs::write(&path, b"too short").unwrap();
        let (salt, persisted) = load_or_create_at(&path);
        // We could not parse the existing file; we generate fresh and overwrite.
        assert_eq!(salt.len(), 32);
        assert!(persisted);
    }

    // The salt below is written literally, not obtained from `load_or_create_at`, so
    // these tests state their own precondition and cannot be invalidated by a change to
    // how salts are minted.
    const KNOWN_SALT: [u8; SALT_BYTES] = [7u8; SALT_BYTES];

    #[test]
    fn a_pre_rename_salt_is_carried_forward_and_written_to_the_new_path() {
        let dir = TempDir::new().unwrap();
        let current = dir.path().join("current");
        let legacy = dir.path().join("legacy");
        fs::write(&legacy, KNOWN_SALT).unwrap();
        assert!(!current.exists(), "precondition: nothing at the new path");

        let (salt, persisted) = load_or_create_migrating(&current, Some(&legacy));

        assert_eq!(
            salt, KNOWN_SALT,
            "the anonymous identity must survive the rename — a fresh salt would make \
             one install look like two"
        );
        assert!(persisted);
        assert!(
            current.exists(),
            "the salt must be migrated, not re-read forever"
        );
        assert_eq!(fs::read(&current).unwrap(), KNOWN_SALT);
        assert!(legacy.exists(), "the user's old file is not ours to delete");
    }

    #[test]
    fn the_current_salt_wins_over_a_pre_rename_one() {
        let dir = TempDir::new().unwrap();
        let current = dir.path().join("current");
        let legacy = dir.path().join("legacy");
        fs::write(&current, KNOWN_SALT).unwrap();
        fs::write(&legacy, [9u8; SALT_BYTES]).unwrap();

        let (salt, persisted) = load_or_create_migrating(&current, Some(&legacy));

        assert_eq!(salt, KNOWN_SALT);
        assert!(persisted);
    }

    #[test]
    fn no_salt_anywhere_mints_a_fresh_one() {
        let dir = TempDir::new().unwrap();
        let current = dir.path().join("current");
        let legacy = dir.path().join("legacy");
        assert!(
            !current.exists() && !legacy.exists(),
            "precondition: neither"
        );

        let (salt, persisted) = load_or_create_migrating(&current, Some(&legacy));

        assert_eq!(salt.len(), SALT_BYTES);
        assert_ne!(salt, KNOWN_SALT);
        assert!(persisted);
        assert!(current.exists());
    }
}
