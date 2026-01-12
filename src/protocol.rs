//! LSP protocol utilities for URI/path conversions

use lsp_types::Uri;
use std::path::{Path, PathBuf};

/// Convert an LSP URI to a file path
pub fn uri_to_path(uri: &Uri) -> Option<PathBuf> {
    let s = uri.as_str();
    if s.starts_with("file:///") {
        // Windows: file:///C:/path or Unix: file:///path
        let path_str = &s[7..]; // Skip "file://"

        // URL decode common sequences
        let path_str = path_str
            .replace("%20", " ")
            .replace("%28", "(")
            .replace("%29", ")")
            .replace("%5B", "[")
            .replace("%5D", "]");

        #[cfg(windows)]
        {
            // On Windows, skip the leading / before drive letter
            let path_str = path_str.strip_prefix('/').unwrap_or(&path_str);
            Some(PathBuf::from(path_str.replace('/', "\\")))
        }
        #[cfg(not(windows))]
        {
            Some(PathBuf::from(path_str))
        }
    } else {
        None
    }
}

/// Convert a file path to an LSP URI
pub fn path_to_uri(path: &Path) -> Uri {
    let path_str = path.to_string_lossy();
    #[cfg(windows)]
    let uri_str = format!("file:///{}", path_str.replace('\\', "/"));
    #[cfg(not(windows))]
    let uri_str = format!("file://{}", path_str);
    uri_str
        .parse()
        .unwrap_or_else(|_| "file:///unknown".parse().unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_uri_to_path_windows() {
        let uri: Uri = "file:///C:/Users/test/file.al".parse().unwrap();
        let path = uri_to_path(&uri);
        #[cfg(windows)]
        assert_eq!(path, Some(PathBuf::from("C:\\Users\\test\\file.al")));
        #[cfg(not(windows))]
        assert_eq!(path, Some(PathBuf::from("/C:/Users/test/file.al")));
    }

    #[test]
    fn test_uri_to_path_with_spaces() {
        let uri: Uri = "file:///C:/My%20Project/file.al".parse().unwrap();
        let path = uri_to_path(&uri);
        #[cfg(windows)]
        assert_eq!(path, Some(PathBuf::from("C:\\My Project\\file.al")));
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
}
