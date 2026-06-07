//! AL type-string normalization (R2b Task 2) — a faithful port of al-sem's
//! `normalizeAlType` from `src/resolve/call-resolver.ts`.
//!
//! Pure string function, never panics. Used for overload-disambiguation type
//! comparison: AL type names are case-insensitive, and length specifiers
//! (`Text[100]`, `Code[20]`) are assignment-compatible with their unbounded
//! forms, so they must not defeat a structural match.
//!
//! TS source (ported char-for-char):
//! ```ts
//! function normalizeAlType(t: string): string {
//!   return t
//!     .toLowerCase()
//!     .replace(/\[[^\]]*\]/g, "") // drop length specifiers: Text[100] -> Text
//!     .replace(/\s+/g, " ")
//!     .trim();
//! }
//! ```
//!
//! The two `replace` calls are hand-scanned (no `regex` crate dependency) to
//! reproduce JS `String.prototype.replace(/.../g, ...)` byte-for-byte:
//!   1. `/\[[^\]]*\]/g` — remove every `[`…`]` run (the inner class `[^\]]*`
//!      matches any char that is NOT `]`, so the match ends at the first `]`).
//!      A `[` with no following `]` is NOT a match in JS (the regex requires a
//!      closing `]`), so an unterminated `[` is left intact.
//!   2. `/\s+/g` — collapse every maximal run of JS-whitespace to a single
//!      U+0020 space.
//!   3. `.trim()` — JS String.trim() strips leading/trailing whitespace.
//!
//! Ordering matters: bracket removal happens BEFORE whitespace collapse, so
//! `array[10] of Integer` → `array of integer` and `Text [ 50 ]` → `text`.

/// JS `\s` (and the chars stripped by `String.prototype.trim()`) per the
/// ECMAScript spec: the Unicode "White_Space" set plus the line terminators
/// (LF, CR, U+2028, U+2029). This list mirrors V8's WhiteSpace + LineTerminator.
fn is_js_whitespace(c: char) -> bool {
    matches!(
        c,
        '\u{0009}' // tab
        | '\u{000A}' // LF
        | '\u{000B}' // vertical tab
        | '\u{000C}' // form feed
        | '\u{000D}' // CR
        | '\u{0020}' // space
        | '\u{00A0}' // no-break space
        | '\u{1680}'
        | '\u{2000}'
            ..='\u{200A}'
        | '\u{2028}' // line separator
        | '\u{2029}' // paragraph separator
        | '\u{202F}'
        | '\u{205F}'
        | '\u{3000}'
        | '\u{FEFF}' // BOM / ZWNBSP (JS treats as whitespace)
    )
}

/// Remove every `[`…`]` run, matching JS `replace(/\[[^\]]*\]/g, "")`.
///
/// Scans left to right: on a `[`, look ahead for the next `]`; if found, drop
/// the whole `[`…`]` span (inclusive) and resume after it. If a `[` has no
/// following `]`, it is NOT a regex match in JS, so emit it verbatim and
/// continue (subsequent `[`…`]` pairs can still match).
fn strip_bracket_runs(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];
        if c == '[' {
            // Look for the next ']' at or after i+1.
            if let Some(off) = chars[i + 1..].iter().position(|&ch| ch == ']') {
                // Drop the entire `[`…`]` span; resume after the `]`.
                i = i + 1 + off + 1;
                continue;
            }
            // No closing `]` → not a match; emit `[` verbatim.
        }
        out.push(c);
        i += 1;
    }
    out
}

/// Collapse every maximal run of JS-whitespace into a single space, matching
/// JS `replace(/\s+/g, " ")`.
fn collapse_whitespace(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_ws = false;
    for c in s.chars() {
        if is_js_whitespace(c) {
            if !in_ws {
                out.push(' ');
                in_ws = true;
            }
        } else {
            out.push(c);
            in_ws = false;
        }
    }
    out
}

/// Strip leading/trailing JS-whitespace, matching JS `String.prototype.trim()`.
fn js_trim(s: &str) -> &str {
    let start = s
        .char_indices()
        .find(|&(_, c)| !is_js_whitespace(c))
        .map(|(i, _)| i);
    let Some(start) = start else { return "" };
    let end = s
        .char_indices()
        .rev()
        .find(|&(_, c)| !is_js_whitespace(c))
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or(s.len());
    &s[start..end]
}

/// Normalize an AL type string for overload-disambiguation comparison.
///
/// Faithful port of al-sem's `normalizeAlType`: lowercase → drop `[…]` length
/// specifiers → collapse whitespace → trim.
pub fn normalize_al_type(t: &str) -> String {
    // JS `.toLowerCase()` is Unicode-aware; Rust `to_lowercase()` matches it
    // for the ASCII type keywords + quoted identifiers AL produces.
    let lowered = t.to_lowercase();
    let no_brackets = strip_bracket_runs(&lowered);
    let collapsed = collapse_whitespace(&no_brackets);
    js_trim(&collapsed).to_string()
}
