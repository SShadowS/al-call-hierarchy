//! Per-installation salt management.
//!
//! Stored at `~/.al-call-hierarchy/installation-id` (32 random bytes).
//! Generated on first use; persists across runs.

use crate::telemetry::hash::Salt;
use anyhow::{Context, Result};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

const SALT_BYTES: usize = 32;

/// Resolve `~/.al-call-hierarchy/installation-id`.
fn default_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".al-call-hierarchy").join("installation-id"))
}

/// Load existing salt, or generate-and-persist a fresh one.
/// Falls back to an in-memory salt if the filesystem is unwritable.
pub fn load_or_create() -> (Salt, bool /* persisted */) {
    let Some(path) = default_path() else {
        return (random_salt(), false);
    };
    load_or_create_at(&path)
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
}
