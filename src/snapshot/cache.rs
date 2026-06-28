//! Content-addressed source cache for embedded `.app` extraction.
//!
//! The cache key is the blake3 hex of the whole `.app` file, so a changed
//! app is always a different key — stale reads are structurally impossible.

use crate::snapshot::embedded::{SourceFile, app_content_hash, extract_embedded_source};
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// Return the stable cache directory, creating it if necessary.
///
/// Uses the OS cache directory (`dirs::cache_dir`) when available, with
/// `al-ch-snapshot-cache` appended.  Falls back to
/// `<temp_dir>/al-ch-snapshot-cache` when the OS cache dir is unavailable.
pub fn cache_dir() -> PathBuf {
    let base = dirs::cache_dir().unwrap_or_else(std::env::temp_dir);
    let dir = base.join("al-ch-snapshot-cache");
    if let Err(e) = std::fs::create_dir_all(&dir) {
        log::warn!("Failed to create snapshot cache dir {}: {e}", dir.display());
    }
    dir
}

/// Return embedded AL source for `app_path`, using the content-addressed
/// on-disk cache when available.
///
/// On a cache miss the source is extracted, serialised as JSON, and written to
/// `<cache_dir>/<blake3-hex>.json`.  On a cache hit the JSON is deserialised
/// directly — no zip I/O needed.
///
/// Returns `(files, content_hash)`.  `files` is empty for symbol-only apps
/// (same semantics as [`extract_embedded_source`]).  The hash is always the
/// blake3 hex of the whole `.app`, computed once and reused — callers must not
/// call `app_content_hash` a second time.
///
/// Cache I/O failures (unwritable dir, locked file, corrupt entry) are
/// **non-fatal**: a warn is logged and extraction proceeds uncached.
pub fn cached_source(app_path: &Path) -> Result<(Vec<SourceFile>, String)> {
    let hash = app_content_hash(app_path)
        .with_context(|| format!("content hash for {}", app_path.display()))?;

    let cache_file = cache_dir().join(format!("{hash}.json"));

    // Cache hit — deserialise; fall through to re-extract on a corrupt entry.
    if cache_file.exists() {
        match try_read_cache(&cache_file) {
            Ok(files) => return Ok((files, hash)),
            Err(e) => {
                log::warn!(
                    "snapshot cache read failed for {} ({e:#}); re-extracting",
                    cache_file.display()
                );
            }
        }
    }

    // Cache miss (or corrupt hit) — extract then persist best-effort.
    let files = extract_embedded_source(app_path)?;
    if let Err(e) = persist_cache(&cache_file, &hash, &files) {
        log::warn!(
            "snapshot source cache write failed for {} ({e:#}); continuing uncached",
            app_path.display()
        );
    }
    Ok((files, hash))
}

/// Attempt to deserialise a cache entry.  Returns `Err` on any I/O or JSON
/// failure so the caller can fall through to re-extraction.
fn try_read_cache(cache_file: &Path) -> Result<Vec<SourceFile>> {
    let raw = std::fs::read_to_string(cache_file)
        .with_context(|| format!("read cache {}", cache_file.display()))?;
    serde_json::from_str(&raw)
        .with_context(|| format!("deserialise cache {}", cache_file.display()))
}

/// Persist `files` to `cache_file` atomically (temp-write + rename).
///
/// Using per-process temp names lets concurrent processes each write their own
/// file; last rename wins.  Both processes write identical content so the final
/// file is always valid — no torn writes.
fn persist_cache(cache_file: &Path, hash: &str, files: &[SourceFile]) -> Result<()> {
    let json = serde_json::to_string(files).context("serialise source files")?;
    let tmp = cache_dir().join(format!("{hash}-{}.json.tmp", std::process::id()));
    std::fs::write(&tmp, &json).with_context(|| format!("write cache tmp {}", tmp.display()))?;
    std::fs::rename(&tmp, cache_file)
        .with_context(|| format!("rename cache tmp → {}", cache_file.display()))?;
    Ok(())
}
