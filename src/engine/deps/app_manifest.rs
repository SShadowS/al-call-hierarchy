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

/// Find the byte index just past the first opening tag `<{tag}\b[^>]*>` (case
/// -insensitive on the tag name), and return the FULL tag slice `<...>`.
/// Mirrors `xml.match(/<{tag}\b[^>]*>/i)`.
fn find_open_tag<'a>(xml: &'a str, tag: &str) -> Option<&'a str> {
    let bytes = xml.as_bytes();
    let tag_lower = tag.to_ascii_lowercase();
    let tlen = tag_lower.len();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            let start = i;
            let after = i + 1;
            // case-insensitive tag-name match
            if after + tlen <= bytes.len()
                && xml[after..after + tlen].eq_ignore_ascii_case(&tag_lower)
            {
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
    let tlen = tag_lower.len();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            let start = i;
            let after = i + 1;
            if after + tlen <= bytes.len()
                && xml[after..after + tlen].eq_ignore_ascii_case(&tag_lower)
            {
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
    let alen = attr_lower.len();
    let mut i = 0;
    while i + alen <= bytes.len() {
        // `\b` before attr: previous char must be a non-word char (or BOL).
        let boundary_before = i == 0 || !is_word(bytes[i - 1]);
        if boundary_before && tag[i..i + alen].eq_ignore_ascii_case(&attr_lower) {
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
    const NEEDLE: &str = "includesourceinsymbolfile";
    let bytes = xml.as_bytes();
    let lower = xml.to_ascii_lowercase();
    let lbytes = lower.as_bytes();
    let nlen = NEEDLE.len();
    let mut i = 0;
    while i + nlen <= lbytes.len() {
        if &lower[i..i + nlen] == NEEDLE {
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
                    if j + 4 <= bytes.len()
                        && lower[j..j + 4] == *"true"
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
    let tlen = tag_lower.len();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            let after = i + 1;
            if after + tlen <= bytes.len()
                && xml[after..after + tlen].eq_ignore_ascii_case(&tag_lower)
            {
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
}
