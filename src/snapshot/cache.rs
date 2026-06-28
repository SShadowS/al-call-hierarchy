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
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Return embedded AL source for `app_path`, using the content-addressed
/// on-disk cache when available.
///
/// On a cache miss the source is extracted, serialised as JSON, and written to
/// `<cache_dir>/<blake3-hex>.json`.  On a cache hit the JSON is deserialised
/// directly — no zip I/O needed.
///
/// Returns an empty `Vec` for symbol-only apps (same semantics as
/// [`extract_embedded_source`]).
pub fn cached_source(app_path: &Path) -> Result<Vec<SourceFile>> {
    let hash = app_content_hash(app_path)
        .with_context(|| format!("content hash for {}", app_path.display()))?;

    let cache_file = cache_dir().join(format!("{hash}.json"));

    // Cache hit.
    if cache_file.exists() {
        let raw = std::fs::read_to_string(&cache_file)
            .with_context(|| format!("read cache {}", cache_file.display()))?;
        let files: Vec<SourceFile> = serde_json::from_str(&raw)
            .with_context(|| format!("deserialise cache {}", cache_file.display()))?;
        return Ok(files);
    }

    // Cache miss — extract then persist.
    let files = extract_embedded_source(app_path)?;
    let json = serde_json::to_string(&files).context("serialise source files")?;
    // Write atomically: temp file beside the target, then rename.
    let tmp = cache_file.with_extension("json.tmp");
    std::fs::write(&tmp, &json).with_context(|| format!("write cache tmp {}", tmp.display()))?;
    std::fs::rename(&tmp, &cache_file)
        .with_context(|| format!("rename cache tmp → {}", cache_file.display()))?;

    Ok(files)
}
