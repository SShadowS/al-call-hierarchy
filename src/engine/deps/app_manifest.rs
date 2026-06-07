//! Rust port of al-sem's `src/symbols/app-manifest.ts` — `parseAppManifestXml`.
//!
//! The dependency app identity used for ENTITY ENCODING (`encode_object_id(app_guid,
//! …)`) comes from the manifest `<App>` element, NOT from `SymbolReference.json`'s
//! `AppId` (R2.5a Rev 2 #2). `includes_source` is read from the manifest's
//! `IncludeSourceInSymbolFile="true"` flag, which drives `sourceKind`.
//!
//! The TS reference uses regexes; we reproduce the SAME semantics with a small,
//! allocation-light manual scanner (no `regex` crate in the engine):
//!   - `<App\b[^>]*>`              → first `<App` followed by a non-word char, up to `>`
//!   - `\b{attr}\s*=\s*"([^"]*)"`  → word-boundary-anchored attr (so `CompatibilityId` ≠ `Id`)
//!   - `<Dependency\b[^>]*>`       → every dependency open-tag
//!   - `/IncludeSourceInSymbolFile\s*=\s*"true"/i`
//!
//! Never panics: every failure path returns a `fail_manifest` carrying `error`
//! with an empty identity (the TS "no silent clean" contract).

/// Dependency-app identity parsed from the manifest `<App>` element.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ManifestAppIdentity {
    pub app_guid: String,
    pub name: String,
    pub publisher: String,
    pub version: String,
}

/// A `<Dependency>` entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestDependency {
    pub app_guid: String,
    pub name: String,
    pub publisher: String,
    pub min_version: String,
}

/// Parsed `NavxManifest.xml`. On any failure, `identity` is empty and `error` is set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppManifest {
    pub identity: ManifestAppIdentity,
    pub dependencies: Vec<ManifestDependency>,
    /// `ResourceExposurePolicy.IncludeSourceInSymbolFile` — whether embedded `.al`
    /// source is present. Drives `sourceKind`.
    pub includes_source: bool,
    /// Set when the manifest could not be parsed; identity is then empty.
    pub error: Option<String>,
}

fn fail_manifest(error: &str) -> AppManifest {
    AppManifest {
        identity: ManifestAppIdentity::default(),
        dependencies: Vec::new(),
        includes_source: false,
        error: Some(error.to_string()),
    }
}

/// True if `c` is an XML "word" char for `\b` purposes (`[A-Za-z0-9_]`).
fn is_word(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// Case-insensitive (ASCII) equality of the `len`-byte window of `bytes` starting
/// at `i` against `needle_lower` (which MUST already be lowercase ASCII).
///
/// Compares on BYTES — never slices `&str` — so it cannot panic on a window whose
/// boundary splits a multi-byte UTF-8 codepoint. A non-ASCII byte simply fails the
/// `eq_ignore_ascii_case` comparison (matching the TS regex, whose tag/attr-name
/// sub-patterns are ASCII and never match inside a non-ASCII run). Returns false
/// when the window runs past the end of `bytes`.
fn window_eq_ignore_ascii_case(bytes: &[u8], i: usize, needle_lower: &[u8]) -> bool {
    let end = match i.checked_add(needle_lower.len()) {
        Some(e) if e <= bytes.len() => e,
        _ => return false,
    };
    bytes[i..end].eq_ignore_ascii_case(needle_lower)
}

/// Find the byte index just past the first opening tag `<{tag}\b[^>]*>` (case
/// -insensitive on the tag name), and return the FULL tag slice `<...>`.
/// Mirrors `xml.match(/<{tag}\b[^>]*>/i)`.
fn find_open_tag<'a>(xml: &'a str, tag: &str) -> Option<&'a str> {
    let bytes = xml.as_bytes();
    let tag_lower = tag.to_ascii_lowercase();
    let tag_lower = tag_lower.as_bytes();
    let tlen = tag_lower.len();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            let start = i;
            let after = i + 1;
            // case-insensitive tag-name match (byte-wise; never slices &str)
            if window_eq_ignore_ascii_case(bytes, after, tag_lower) {
                // `\b`: the char after the tag name must be a NON-word char
                let boundary_idx = after + tlen;
                let boundary_ok = boundary_idx >= bytes.len() || !is_word(bytes[boundary_idx]);
                if boundary_ok {
                    // scan `[^>]*>` — find the closing `>` (no `>` allowed inside)
                    let mut j = boundary_idx;
                    while j < bytes.len() && bytes[j] != b'>' {
                        j += 1;
                    }
                    if j < bytes.len() {
                        return Some(&xml[start..=j]);
                    }
                    // no closing `>` → regex would not match this occurrence
                }
            }
        }
        i += 1;
    }
    None
}

/// Iterate every opening tag `<{tag}\b[^>]*>` (case-insensitive). Mirrors
/// `xml.matchAll(/<{tag}\b[^>]*>/gi)`.
fn find_all_open_tags<'a>(xml: &'a str, tag: &str) -> Vec<&'a str> {
    let bytes = xml.as_bytes();
    let tag_lower = tag.to_ascii_lowercase();
    let tag_lower = tag_lower.as_bytes();
    let tlen = tag_lower.len();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            let start = i;
            let after = i + 1;
            if window_eq_ignore_ascii_case(bytes, after, tag_lower) {
                let boundary_idx = after + tlen;
                let boundary_ok = boundary_idx >= bytes.len() || !is_word(bytes[boundary_idx]);
                if boundary_ok {
                    let mut j = boundary_idx;
                    while j < bytes.len() && bytes[j] != b'>' {
                        j += 1;
                    }
                    if j < bytes.len() {
                        out.push(&xml[start..=j]);
                        i = j + 1;
                        continue;
                    }
                }
            }
        }
        i += 1;
    }
    out
}

/// Read an attribute value out of an opening tag, anchored on a word boundary so
/// a longer attribute name (`CompatibilityId`) cannot match a shorter one (`Id`).
/// Mirrors `tag.match(/\b{attr}\s*=\s*"([^"]*)"/i)?.[1] ?? ""`.
fn read_tag_attr(tag: &str, attr: &str) -> String {
    let bytes = tag.as_bytes();
    let attr_lower = attr.to_ascii_lowercase();
    let attr_lower = attr_lower.as_bytes();
    let alen = attr_lower.len();
    let mut i = 0;
    while i + alen <= bytes.len() {
        // `\b` before attr: previous char must be a non-word char (or BOL).
        let boundary_before = i == 0 || !is_word(bytes[i - 1]);
        // byte-wise name compare; never slices &str at a non-boundary.
        if boundary_before && window_eq_ignore_ascii_case(bytes, i, attr_lower) {
            let mut j = i + alen;
            // `\s*`
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'=' {
                j += 1;
                while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                    j += 1;
                }
                if j < bytes.len() && bytes[j] == b'"' {
                    j += 1;
                    let val_start = j;
                    while j < bytes.len() && bytes[j] != b'"' {
                        j += 1;
                    }
                    if j < bytes.len() {
                        return tag[val_start..j].to_string();
                    }
                }
            }
        }
        i += 1;
    }
    String::new()
}

/// Read one attribute from the first occurrence of `<{tag} ...>`, tolerant of
/// attribute order. Mirrors `readAttr`.
fn read_attr(xml: &str, tag: &str, attr: &str) -> String {
    match find_open_tag(xml, tag) {
        Some(t) => read_tag_attr(t, attr),
        None => String::new(),
    }
}

/// Case-insensitive search for `IncludeSourceInSymbolFile\s*=\s*"true"`. Mirrors
/// `/IncludeSourceInSymbolFile\s*=\s*"true"/i.test(xml)`.
fn includes_source_flag(xml: &str) -> bool {
    const NEEDLE: &[u8] = b"includesourceinsymbolfile";
    // ASCII-lowercasing preserves byte length and leaves non-ASCII bytes intact,
    // so `lbytes` indices align with `bytes` and all comparisons stay byte-wise
    // (never slicing `&str` at a possibly-non-boundary window).
    let bytes = xml.as_bytes();
    let lower = xml.to_ascii_lowercase();
    let lbytes = lower.as_bytes();
    let nlen = NEEDLE.len();
    let mut i = 0;
    while i + nlen <= lbytes.len() {
        if &lbytes[i..i + nlen] == NEEDLE {
            let mut j = i + nlen;
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'=' {
                j += 1;
                while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                    j += 1;
                }
                if j < bytes.len() && bytes[j] == b'"' {
                    j += 1;
                    // expect `true` (case-insensitive) then `"`
                    if j + 4 <= lbytes.len()
                        && &lbytes[j..j + 4] == b"true"
                        && j + 4 < bytes.len()
                        && bytes[j + 4] == b'"'
                    {
                        return true;
                    }
                }
            }
        }
        i += 1;
    }
    false
}

/// Parse the text of a `NavxManifest.xml`. Never panics — failure is reported via
/// `error`. Mirrors al-sem `parseAppManifestXml`.
pub fn parse_app_manifest_xml(xml: &str) -> AppManifest {
    // `/<App\b/i.test(xml)` — require an `<App` followed by a non-word char.
    if find_open_tag_present(xml, "App") {
        // continue
    } else {
        return fail_manifest("no <App> element found in manifest");
    }

    let identity = ManifestAppIdentity {
        app_guid: read_attr(xml, "App", "Id"),
        name: read_attr(xml, "App", "Name"),
        publisher: read_attr(xml, "App", "Publisher"),
        version: read_attr(xml, "App", "Version"),
    };
    if identity.app_guid.is_empty() {
        return fail_manifest("<App> element missing Id attribute");
    }

    let mut dependencies = Vec::new();
    for tag in find_all_open_tags(xml, "Dependency") {
        dependencies.push(ManifestDependency {
            app_guid: read_tag_attr(tag, "Id"),
            name: read_tag_attr(tag, "Name"),
            publisher: read_tag_attr(tag, "Publisher"),
            min_version: read_tag_attr(tag, "MinVersion"),
        });
    }

    AppManifest {
        identity,
        dependencies,
        includes_source: includes_source_flag(xml),
        error: None,
    }
}

/// `/<App\b/i.test(xml)` — there exists an `<App` followed by a non-word char.
/// Distinct from `find_open_tag` because the presence test does NOT require a
/// closing `>` (the TS guard regex is just `/<App\b/i`).
fn find_open_tag_present(xml: &str, tag: &str) -> bool {
    let bytes = xml.as_bytes();
    let tag_lower = tag.to_ascii_lowercase();
    let tag_lower = tag_lower.as_bytes();
    let tlen = tag_lower.len();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            let after = i + 1;
            if window_eq_ignore_ascii_case(bytes, after, tag_lower) {
                let boundary_idx = after + tlen;
                if boundary_idx >= bytes.len() || !is_word(bytes[boundary_idx]) {
                    return true;
                }
            }
        }
        i += 1;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<?xml version="1.0"?>
<Package>
  <App Id="11111111-2222-3333-4444-555555555555" Name="My Dep" Publisher="Acme" Version="1.2.3.4" CompatibilityId="0.0.0.0"/>
  <Dependency Id="aaaa" Name="Base" Publisher="Microsoft" MinVersion="22.0.0.0"/>
  <ResourceExposurePolicy IncludeSourceInSymbolFile="true"/>
</Package>"#;

    #[test]
    fn parses_app_identity_word_boundary() {
        let m = parse_app_manifest_xml(SAMPLE);
        assert!(m.error.is_none());
        // CompatibilityId must NOT be read as Id.
        assert_eq!(m.identity.app_guid, "11111111-2222-3333-4444-555555555555");
        assert_eq!(m.identity.name, "My Dep");
        assert_eq!(m.identity.publisher, "Acme");
        assert_eq!(m.identity.version, "1.2.3.4");
        assert!(m.includes_source);
        assert_eq!(m.dependencies.len(), 1);
        assert_eq!(m.dependencies[0].app_guid, "aaaa");
        assert_eq!(m.dependencies[0].min_version, "22.0.0.0");
    }

    #[test]
    fn no_app_element_fails() {
        let m = parse_app_manifest_xml("<Package></Package>");
        assert_eq!(
            m.error.as_deref(),
            Some("no <App> element found in manifest")
        );
        assert!(m.identity.app_guid.is_empty());
    }

    #[test]
    fn missing_id_fails() {
        let m = parse_app_manifest_xml(r#"<App Name="x"/>"#);
        assert_eq!(
            m.error.as_deref(),
            Some("<App> element missing Id attribute")
        );
    }

    #[test]
    fn includes_source_false_when_absent() {
        let m = parse_app_manifest_xml(r#"<App Id="g"/>"#);
        assert!(!m.includes_source);
    }

    #[test]
    fn includes_source_false_when_value_not_true() {
        let m = parse_app_manifest_xml(r#"<App Id="g"/><X IncludeSourceInSymbolFile = "false"/>"#);
        assert!(!m.includes_source);
    }

    /// Regression: non-ASCII publisher/name (common for real BC `.app`s, e.g.
    /// `Ægir`, `Lønn`) must parse without panicking. Before the byte-wise scanner
    /// fix, the hand-rolled `&str` slices split a UTF-8 codepoint and panicked
    /// (`byte index N is not a char boundary`). The TS regex oracle never throws —
    /// it just matches the ASCII attribute names and passes non-ASCII through the
    /// captured value verbatim. Assert that exact behaviour here.
    #[test]
    fn parses_internationalized_manifest_without_panic() {
        let xml = r#"<?xml version="1.0"?>
<Package>
  <App Id="22222222-3333-4444-5555-666666666666" Name="Lønn Pålegg" Publisher="Ægir Software ÆØÅ" Version="9.8.7.6" CompatibilityId="0.0.0.0"/>
  <Dependency Id="dep-é" Name="Naïve Modül" Publisher="Société Générale" MinVersion="22.0.0.0"/>
  <ResourceExposurePolicy IncludeSourceInSymbolFile="true"/>
</Package>"#;
        let m = parse_app_manifest_xml(xml);
        assert!(m.error.is_none(), "should parse, got error: {:?}", m.error);
        assert_eq!(m.identity.app_guid, "22222222-3333-4444-5555-666666666666");
        // Non-ASCII attribute values are passed through verbatim (UTF-8 intact).
        assert_eq!(m.identity.name, "Lønn Pålegg");
        assert_eq!(m.identity.publisher, "Ægir Software ÆØÅ");
        assert_eq!(m.identity.version, "9.8.7.6");
        // CompatibilityId still must NOT be read as Id.
        assert!(m.includes_source);
        assert_eq!(m.dependencies.len(), 1);
        assert_eq!(m.dependencies[0].app_guid, "dep-é");
        assert_eq!(m.dependencies[0].name, "Naïve Modül");
        assert_eq!(m.dependencies[0].publisher, "Société Générale");
        assert_eq!(m.dependencies[0].min_version, "22.0.0.0");
    }

    /// The reviewer's direct reproductions: these exact inputs panicked at a
    /// non-char-boundary slice before the fix. Now they must return cleanly.
    #[test]
    fn reviewer_repros_do_not_panic() {
        // read_tag_attr: non-ASCII value BEFORE the target ASCII attribute.
        assert_eq!(
            read_tag_attr("<App Publisher=\"Æøå-Corp\" Id=\"g\">", "Id"),
            "g"
        );
        // find_open_tag: a non-ASCII byte where the tag-name window would land.
        assert!(find_open_tag("<Apé Id=\"x\">", "App").is_none());
        // find_open_tag_present over the same kind of input must also be panic-free.
        assert!(!find_open_tag_present("<Apé Id=\"x\">", "App"));
        // includes_source_flag scanning past a non-ASCII run must be panic-free.
        assert!(!includes_source_flag("<X Name=\"Ægir\"/>"));
    }
}
