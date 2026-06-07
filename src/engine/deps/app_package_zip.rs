//! Rust port of al-sem's `src/symbols/app-package-zip.ts` +
//! `src/symbols/symbol-reference-reader.ts` (entry pick + BOM strip).
//!
//! A BC `.app` file is a ZIP archive that may carry a binary header before the
//! ZIP local-file signature `PK\x03\x04`. We strip that header (scanning at most
//! the first 4096 bytes — the TS `Math.min(len-4, 4096)` bound), then select the
//! `SymbolReference.json` / `NavxManifest.xml` entry by normalizing entry names
//! (`\` → `/`), lowercasing, and matching `ends_with(...)`. The FIRST matching
//! entry in archive iteration order wins — mirroring TS `Object.keys(entries)[0]`
//! over `fflate`'s entry map.
//!
//! Never panics: a malformed archive / missing entry yields `None`, matching the
//! TS "never throws" posture.

use std::io::{Cursor, Read};

/// Scan the first `≤4096` bytes of `bytes` for the ZIP local-file signature
/// `PK\x03\x04` and return the slice starting there. If no signature is found in
/// the bound, assume it is already a plain ZIP and return the input unchanged.
///
/// Mirrors al-sem `stripAppHeader`: `limit = min(len - 4, 4096)`; on a match at
/// `i == 0` the input is returned verbatim, otherwise the tail from `i`.
pub fn strip_app_header(bytes: &[u8]) -> &[u8] {
    if bytes.len() < 4 {
        return bytes;
    }
    // limit = min(len - 4, 4096); loop is `for i in 0..limit` (exclusive),
    // and we index i..i+4 so the last inspected window is [limit-1 .. limit+2].
    let limit = std::cmp::min(bytes.len() - 4, 4096);
    for i in 0..limit {
        if bytes[i] == 0x50 && bytes[i + 1] == 0x4b && bytes[i + 2] == 0x03 && bytes[i + 3] == 0x04
        {
            return &bytes[i..];
        }
    }
    bytes
}

/// Normalize a ZIP entry key: backslashes → forward slashes. Mirrors al-sem
/// `normalizeZipEntryName`.
pub fn normalize_zip_entry_name(key: &str) -> String {
    key.replace('\\', "/")
}

/// Extract the bytes of the FIRST entry whose normalized, lowercased name ends
/// with `suffix_lower`. Iterates entries in archive order (the `zip` crate's
/// `by_index`, which preserves the central-directory order — the analogue of
/// `fflate`'s insertion-ordered entry map). Returns `None` when the archive is
/// unreadable or no entry matches. Never panics.
fn extract_entry_bytes(app_bytes: &[u8], suffix_lower: &str) -> Option<Vec<u8>> {
    let zip = strip_app_header(app_bytes);
    let cursor = Cursor::new(zip.to_vec());
    let mut archive = match zip::ZipArchive::new(cursor) {
        Ok(a) => a,
        Err(_) => return None,
    };
    let len = archive.len();
    for i in 0..len {
        // Read the name first (immutable view), then re-borrow to read bytes.
        let name = match archive.by_index(i) {
            Ok(f) => f.name().to_string(),
            Err(_) => continue,
        };
        let normalized = normalize_zip_entry_name(&name).to_lowercase();
        if normalized.ends_with(suffix_lower) {
            let mut file = match archive.by_index(i) {
                Ok(f) => f,
                Err(_) => return None,
            };
            let mut out = Vec::new();
            if file.read_to_end(&mut out).is_err() {
                return None;
            }
            return Some(out);
        }
    }
    None
}

/// Decode bytes as UTF-8 (lossy — engine never panics on bad input) and strip a
/// leading UTF-8 BOM if present. Mirrors al-sem `decodeText`.
fn decode_text(bytes: &[u8]) -> String {
    // Strip a UTF-8 BOM (EF BB BF) at the byte level first, matching the TS
    // behaviour of dropping the U+FEFF code unit after decode.
    let body = if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        &bytes[3..]
    } else {
        bytes
    };
    String::from_utf8_lossy(body).into_owned()
}

/// Extract the `SymbolReference.json` text from raw `.app` bytes. Returns `None`
/// if absent. Never panics. Mirrors al-sem `extractSymbolReferenceJson`.
pub fn extract_symbol_reference_json(app_bytes: &[u8]) -> Option<String> {
    extract_entry_bytes(app_bytes, "symbolreference.json").map(|b| decode_text(&b))
}

/// Extract the `NavxManifest.xml` text from raw `.app` bytes. Returns `None` if
/// absent. Never panics. Mirrors the manifest entry pick in al-sem
/// `readAppManifest` (UTF-8 decode; no BOM strip in TS for the manifest, but a
/// leading BOM is harmless to the regex scan — we leave the XML bytes as-is via
/// lossy UTF-8).
pub fn extract_navx_manifest_xml(app_bytes: &[u8]) -> Option<String> {
    extract_entry_bytes(app_bytes, "navxmanifest.xml")
        .map(|b| String::from_utf8_lossy(&b).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_header_no_header_returns_input() {
        let plain = b"PK\x03\x04rest-of-zip";
        assert_eq!(strip_app_header(plain), plain);
    }

    #[test]
    fn strip_header_skips_binary_prefix() {
        let mut bytes = vec![0x00, 0xAA, 0xBB];
        bytes.extend_from_slice(b"PK\x03\x04tail");
        assert_eq!(strip_app_header(&bytes), b"PK\x03\x04tail");
    }

    #[test]
    fn strip_header_short_input_is_safe() {
        assert_eq!(strip_app_header(b"PK"), b"PK");
        assert_eq!(strip_app_header(b""), b"");
    }

    #[test]
    fn strip_header_signature_beyond_4096_not_found() {
        // Header longer than the 4096 scan bound → signature not found → input
        // returned unchanged (TS behaviour: assume plain ZIP).
        let mut bytes = vec![0u8; 5000];
        bytes.extend_from_slice(b"PK\x03\x04tail");
        assert_eq!(strip_app_header(&bytes), &bytes[..]);
    }

    #[test]
    fn normalize_backslashes() {
        assert_eq!(normalize_zip_entry_name("a\\b\\c"), "a/b/c");
        assert_eq!(normalize_zip_entry_name("a/b"), "a/b");
    }

    #[test]
    fn decode_text_strips_bom() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(b"{}");
        assert_eq!(decode_text(&bytes), "{}");
        assert_eq!(decode_text(b"{}"), "{}");
    }
}
