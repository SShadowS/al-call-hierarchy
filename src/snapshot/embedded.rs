//! Extract embedded ShowMyCode `.al` source from a `.app` package.

use anyhow::{Context, Result};
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

/// `.app` files start with a 40-byte NAVX header, then a standard zip.
///
/// NOTE: `src/app_package.rs` (declared in the *binary* `main.rs` scope) has
/// an identical `NAVX_HEADER_SIZE` and a factored `open_app_zip` helper.
/// That helper cannot be referenced here because `app_package` is not part of
/// the *library* crate (`lib.rs` / `snapshot`). The 4-line zip-open logic is
/// therefore duplicated; a future task should move `app_package` into `lib.rs`
/// so both callers can share it.
const NAVX_HEADER_SIZE: u64 = 40;

/// One embedded source file recovered from a `.app`.
#[derive(Clone, Debug)]
pub struct SourceFile {
    pub virtual_path: String,
    pub text: String,
}

/// Open a `.app`'s embedded zip by seeking past the NAVX header.
///
/// Returns `None` if the file cannot be opened or is not a valid zip
/// (e.g. a truncated / corrupt `.app`).
fn open_zip(path: &Path) -> Result<Option<zip::ZipArchive<BufReader<std::fs::File>>>> {
    let file =
        std::fs::File::open(path).with_context(|| format!("open .app: {}", path.display()))?;
    let mut reader = BufReader::new(file);
    reader.seek(SeekFrom::Start(NAVX_HEADER_SIZE))?;
    match zip::ZipArchive::new(reader) {
        Ok(a) => Ok(Some(a)),
        Err(_) => Ok(None),
    }
}

/// blake3 hex of the whole `.app` file (artifact identity).
pub fn app_content_hash(app_path: &Path) -> Result<String> {
    let bytes =
        std::fs::read(app_path).with_context(|| format!("read .app: {}", app_path.display()))?;
    Ok(blake3::hash(&bytes).to_hex().to_string())
}

/// Extract every `*.al` entry from the `.app`'s embedded zip. Returns an empty
/// `Vec` if the app ships no source (symbol-only / runtime app).
///
/// Entry names are percent-decoded (the AL compiler URL-encodes them) and BOM
/// is stripped before the text is decoded as UTF-8 (lossy).
pub fn extract_embedded_source(app_path: &Path) -> Result<Vec<SourceFile>> {
    let Some(mut archive) = open_zip(app_path)? else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let name = entry.name().to_string();
        if !name.to_ascii_lowercase().ends_with(".al") {
            continue;
        }
        let mut raw = Vec::new();
        entry.read_to_end(&mut raw)?;
        let text = String::from_utf8_lossy(strip_bom(&raw)).into_owned();
        let virtual_path = percent_encoding::percent_decode_str(&name)
            .decode_utf8_lossy()
            .into_owned();
        out.push(SourceFile { virtual_path, text });
    }
    Ok(out)
}

fn strip_bom(b: &[u8]) -> &[u8] {
    if b.starts_with(&[0xEF, 0xBB, 0xBF]) {
        &b[3..]
    } else {
        b
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    // Set CDO_APP to a real ShowMyCode .app to exercise extraction.
    fn cdo_app() -> Option<std::path::PathBuf> {
        std::env::var_os("CDO_APP")
            .map(std::path::PathBuf::from)
            .filter(|p| p.exists())
    }

    #[test]
    fn extracts_al_source_from_showmycode_app() {
        let Some(app) = cdo_app() else {
            return;
        };
        let files = extract_embedded_source(&app).expect("extract");
        assert!(
            files.len() > 100,
            "ShowMyCode app should yield many .al files, got {}",
            files.len()
        );
        assert!(files.iter().all(|f| f.virtual_path.ends_with(".al")));
        assert!(
            files
                .iter()
                .any(|f| f.text.contains("codeunit") || f.text.contains("table"))
        );
    }

    #[test]
    fn content_hash_is_stable() {
        let Some(app) = cdo_app() else {
            return;
        };
        assert_eq!(
            app_content_hash(&app).unwrap(),
            app_content_hash(&app).unwrap()
        );
        assert_eq!(app_content_hash(Path::new(&app)).unwrap().len(), 64);
    }
}
