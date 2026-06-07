//! Type-reference parsing (R2b Task 2) — faithful port of al-sem's
//! `parseObjectTypeRef` from `src/resolve/type-ref.ts`.
//!
//! Pure string function, never panics. Input is a normalized `declaredType`
//! string (whitespace already collapsed, quoted identifiers preserved verbatim),
//! e.g. `Codeunit "Sales-Post"`, `Page "Customer Card"`, `XmlPort Foo`,
//! `Interface IFoo`, `Enum 50100`.
//!
//! Returns `None` for records, primitives, and unrecognized types.

/// Canonical object-type kind. `as_str()` yields the exact al-sem spelling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectKind {
    Codeunit,
    Page,
    Report,
    Query,
    XmlPort,
    Interface,
    Enum,
}

impl ObjectKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ObjectKind::Codeunit => "Codeunit",
            ObjectKind::Page => "Page",
            ObjectKind::Report => "Report",
            ObjectKind::Query => "Query",
            ObjectKind::XmlPort => "XmlPort",
            ObjectKind::Interface => "Interface",
            ObjectKind::Enum => "Enum",
        }
    }
}

/// A parsed object type reference: kind + unquoted, original-casing name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectTypeRef {
    pub kind: ObjectKind,
    pub name: String,
}

/// Lowercased keyword → canonical kind. Port of al-sem `KEYWORD_TO_KIND`.
fn keyword_to_kind(lc: &str) -> Option<ObjectKind> {
    match lc {
        "codeunit" => Some(ObjectKind::Codeunit),
        "page" => Some(ObjectKind::Page),
        "report" => Some(ObjectKind::Report),
        "query" => Some(ObjectKind::Query),
        "xmlport" => Some(ObjectKind::XmlPort),
        "interface" => Some(ObjectKind::Interface),
        "enum" => Some(ObjectKind::Enum),
        _ => None,
    }
}

/// Parse a normalized `declaredType` into an object type reference.
///
/// Port of al-sem `parseObjectTypeRef`:
///   - empty string → None
///   - no space (bare primitive like "Integer") → None
///   - keyword (before first space) looked up case-insensitively; unrecognized
///     ("Record", "Text", …) → None
///   - the name portion (after first space) is unquoted if wrapped in
///     double-quotes; an empty name portion → None
pub fn parse_object_type_ref(declared_type: &str) -> Option<ObjectTypeRef> {
    if declared_type.is_empty() {
        return None;
    }

    // Split into at most two tokens: keyword and the rest (name portion), on the
    // FIRST space. TS uses `indexOf(" ")`, i.e. an ASCII U+0020 space only.
    let space_idx = declared_type.find(' ')?;

    let keyword = &declared_type[..space_idx];
    let name_portion = &declared_type[space_idx + 1..];

    let kind = keyword_to_kind(&keyword.to_lowercase())?;

    let name = unquote_name(name_portion)?;
    Some(ObjectTypeRef { kind, name })
}

/// Strip surrounding double-quotes if present; `None` only when the token is
/// empty. Port of al-sem `unquoteName`.
fn unquote_name(token: &str) -> Option<String> {
    if token.is_empty() {
        return None;
    }
    if token.starts_with('"') && token.ends_with('"') && token.len() >= 2 {
        // `len >= 2` guards the single-`"` case (starts==ends on the same char):
        // a lone `"` has len 1, so it falls through to the verbatim return.
        // For a real pair, strip exactly one leading + one trailing byte (both
        // are the ASCII `"`, 1 byte each).
        return Some(token[1..token.len() - 1].to_string());
    }
    Some(token.to_string())
}
