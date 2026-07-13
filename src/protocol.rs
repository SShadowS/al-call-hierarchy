//! LSP protocol utilities for URI/path conversions

use lsp_types::Uri;
use percent_encoding::{AsciiSet, CONTROLS, percent_decode_str, utf8_percent_encode};
use std::path::{Path, PathBuf};

/// RFC 3986 `pchar`-complement for a URI path segment: everything that is NOT a
/// `pchar` (unreserved / sub-delim / `:` / `@`) must be percent-encoded, plus `%`
/// itself (so an already-percent-looking sequence in the raw filename doesn't get
/// misread as an escape) and `\` (never valid in a path segment; Windows paths are
/// split on it before encoding, but a filename could still contain a literal one).
const PATH_SEGMENT: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'%')
    .add(b'<')
    .add(b'>')
    .add(b'?')
    .add(b'`')
    .add(b'{')
    .add(b'}')
    .add(b'[')
    .add(b']')
    .add(b'^')
    .add(b'|')
    .add(b'\\');

/// Normalize a path for case-insensitive comparison on Windows.
/// On Windows, file paths are case-insensitive but PathBuf comparison is case-sensitive,
/// so we lowercase the entire path to ensure consistent HashMap lookups.
/// On other platforms, returns the path unchanged.
#[cfg(windows)]
pub fn normalize_path(path: &Path) -> PathBuf {
    PathBuf::from(path.to_string_lossy().to_lowercase())
}

#[cfg(not(windows))]
pub fn normalize_path(path: &Path) -> PathBuf {
    path.to_path_buf()
}

/// Convert an LSP URI to a file path
pub fn uri_to_path(uri: &Uri) -> Option<PathBuf> {
    let s = uri.as_str();
    if s.starts_with("file:///") {
        // Windows: file:///C:/path or Unix: file:///path
        let path_str = &s[7..]; // Skip "file://"

        // URL decode percent-encoded sequences (e.g. %3A for ':', %20 for ' ')
        let path_str = percent_decode_str(path_str)
            .decode_utf8_lossy()
            .into_owned();

        #[cfg(windows)]
        {
            // On Windows, skip the leading / before drive letter
            let path_str = path_str.strip_prefix('/').unwrap_or(&path_str);
            Some(normalize_path(Path::new(&path_str.replace('/', "\\"))))
        }
        #[cfg(not(windows))]
        {
            Some(PathBuf::from(path_str))
        }
    } else {
        None
    }
}

/// Convert a file path to an LSP URI.
/// Note: On Windows, paths stored in the call graph are normalized to lowercase
/// (via `normalize_path`), so URIs produced from graph paths will have lowercase
/// drive letters and path segments. LSP clients on Windows handle this correctly.
pub fn path_to_uri(path: &Path) -> Uri {
    let path_str = path.to_string_lossy();
    #[cfg(windows)]
    let path_normalized = path_str.replace('\\', "/");
    #[cfg(not(windows))]
    let path_normalized = path_str.to_string();

    // Percent-encode each path segment independently (RFC 3986), rather than the
    // previous hand-picked handful of characters (space, parens, brackets) — that
    // subset left non-ASCII text and other reserved bytes (#, %, +, @, ...) raw in
    // the URI, which lsp-types' URI parser then rejected outright (H-13).
    let path_encoded = path_normalized
        .split('/')
        .map(|segment| utf8_percent_encode(segment, PATH_SEGMENT).to_string())
        .collect::<Vec<_>>()
        .join("/");

    #[cfg(windows)]
    let uri_str = format!("file:///{}", path_encoded);
    #[cfg(not(windows))]
    let uri_str = format!("file://{}", path_encoded);

    match uri_str.parse() {
        Ok(uri) => uri,
        Err(e) => {
            // Log the problematic path for debugging
            log::warn!(
                "Failed to parse URI '{}' from path '{}': {}. Using fallback.",
                uri_str,
                path.display(),
                e
            );
            "file:///unknown".parse().unwrap()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_uri_to_path_windows() {
        let uri: Uri = "file:///C:/Users/test/file.al".parse().unwrap();
        let path = uri_to_path(&uri);
        #[cfg(windows)]
        assert_eq!(path, Some(PathBuf::from("c:\\users\\test\\file.al")));
        #[cfg(not(windows))]
        assert_eq!(path, Some(PathBuf::from("/C:/Users/test/file.al")));
    }

    #[test]
    fn test_uri_to_path_with_spaces() {
        let uri: Uri = "file:///C:/My%20Project/file.al".parse().unwrap();
        let path = uri_to_path(&uri);
        #[cfg(windows)]
        assert_eq!(path, Some(PathBuf::from("c:\\my project\\file.al")));
        // On non-Windows the path keeps its case and its leading slash; the
        // percent-decoding of `%20` is the platform-independent contract here.
        #[cfg(not(windows))]
        assert_eq!(path, Some(PathBuf::from("/C:/My Project/file.al")));
    }

    #[test]
    fn test_path_to_uri() {
        #[cfg(windows)]
        {
            let path = PathBuf::from("C:\\Users\\test\\file.al");
            let uri = path_to_uri(&path);
            assert_eq!(uri.as_str(), "file:///C:/Users/test/file.al");
        }
    }

    #[test]
    fn test_normalize_path_case_insensitive() {
        #[cfg(windows)]
        {
            let path1 = normalize_path(Path::new("C:\\Git\\Project\\AL\\File.al"));
            let path2 = normalize_path(Path::new("C:\\Git\\Project\\al\\file.al"));
            let path3 = normalize_path(Path::new("c:\\git\\project\\AL\\FILE.AL"));
            assert_eq!(path1, path2);
            assert_eq!(path2, path3);
        }
        #[cfg(not(windows))]
        {
            // On non-Windows, paths are preserved as-is
            let path = normalize_path(Path::new("/home/user/Project/AL/File.al"));
            assert_eq!(path, PathBuf::from("/home/user/Project/AL/File.al"));
        }
    }

    #[test]
    fn test_uri_to_path_percent_encoded_colon() {
        // Issue #9: VS Code encodes drive letter colon as %3A in file URIs
        // e.g. rootUri: "file:///d%3A/Repos/Clone/..."
        let uri: Uri = "file:///d%3A/Repos/MyProject/src/file.al".parse().unwrap();
        let path = uri_to_path(&uri);
        #[cfg(windows)]
        assert_eq!(
            path,
            Some(PathBuf::from("d:\\repos\\myproject\\src\\file.al"))
        );
        #[cfg(not(windows))]
        assert_eq!(path, Some(PathBuf::from("/d:/Repos/MyProject/src/file.al")));
    }

    #[test]
    fn test_uri_to_path_all_percent_encoded() {
        // Ensure all standard percent-encoded characters are decoded, not just a hardcoded subset
        // %23 = #, %25 = %, %40 = @, %2B = +, %3D = =, %26 = &
        let uri: Uri = "file:///C:/dir%23name/file%40v2.al".parse().unwrap();
        let path = uri_to_path(&uri);
        #[cfg(windows)]
        assert_eq!(path, Some(PathBuf::from("c:\\dir#name\\file@v2.al")));
        #[cfg(not(windows))]
        assert_eq!(path, Some(PathBuf::from("/C:/dir#name/file@v2.al")));
    }

    #[cfg(windows)]
    #[test]
    fn uri_roundtrip_non_ascii_path() {
        // H-13: Løsninger previously produced file:///unknown via fluent-uri rejection
        // because the hand-rolled encoder only escaped a ~5-char subset (space, (, ),
        // [, ]) and left raw non-ASCII / reserved bytes in the URI, which lsp-types'
        // fluent-uri-backed parser then rejected.
        //
        // Windows-only: these literals use `\` as the path separator (real Windows
        // paths), which `path_to_uri` only rewrites to `/` under `#[cfg(windows)]` —
        // see the `#[cfg(not(windows))]` sibling below for the native-Unix coverage
        // of the same character classes.
        for p in [
            r"C:\Løsninger\App\Fil æøå.al",
            r"C:\repo\100%\a#b\c+d @e\f.al",
            r"C:\repo\emoji 🚀\file.al",
        ] {
            let uri = path_to_uri(Path::new(p));
            let uri_str = uri.as_str();
            assert_ne!(uri_str, "file:///unknown", "must not hit fallback for {p}");
            // path_to_uri preserves case and leaves the drive-letter colon literal
            // (see test_path_to_uri above) — it does not lowercase; that's normalize_path's job.
            assert!(uri_str.starts_with("file:///C:/"), "{uri_str}");
            let back = uri_to_path(&uri).expect("must decode");
            assert_eq!(back, normalize_path(Path::new(p)), "roundtrip {p}");
        }
    }

    #[cfg(not(windows))]
    #[test]
    fn uri_roundtrip_non_ascii_path() {
        // H-13, Unix sibling: same character classes as the Windows test above
        // (non-ASCII text, %, #, +, @, space, emoji), exercised with native
        // Unix absolute paths (forward-slash separators, no drive letter) so the
        // round trip is genuinely driven through the `#[cfg(not(windows))]` arms
        // of both path_to_uri and uri_to_path — CI (ubuntu-latest) runs this arm.
        for p in [
            "/repo/Løsninger/Fil æøå.al",
            "/repo/100%/a#b/c+d @e/f.al",
            "/repo/emoji 🚀/file.al",
        ] {
            let uri = path_to_uri(Path::new(p));
            let uri_str = uri.as_str();
            assert_ne!(uri_str, "file:///unknown", "must not hit fallback for {p}");
            assert!(uri_str.starts_with("file:///repo/"), "{uri_str}");
            let back = uri_to_path(&uri).expect("must decode");
            assert_eq!(back, normalize_path(Path::new(p)), "roundtrip {p}");
        }
    }

    #[test]
    fn test_uri_to_path_case_normalized_on_windows() {
        // Different URI casings should produce the same path on Windows
        let uri1: Uri = "file:///U:/Git/Project/AL/Codeunit/File.al"
            .parse()
            .unwrap();
        let uri2: Uri = "file:///U:/Git/Project/Al/Codeunit/File.al"
            .parse()
            .unwrap();
        let path1 = uri_to_path(&uri1);
        let path2 = uri_to_path(&uri2);
        #[cfg(windows)]
        assert_eq!(path1, path2);
        // On a case-SENSITIVE filesystem these are genuinely different paths —
        // pinning that here is what makes the Windows arm above a statement
        // about normalization rather than an accident. The LSP surface's own
        // case-insensitive fallbacks (`resolve_virtual_path`, `classify_path`)
        // exist precisely because this divergence is real.
        #[cfg(not(windows))]
        assert_ne!(path1, path2);
    }
}
