//! Receiver-name parsing (R2b Task 2) — faithful port of al-sem's
//! `simpleReceiverName` from `src/index/receiver-classification.ts` (re-exported
//! by `src/resolve/type-ref.ts`).
//!
//! Pure string function, never panics. Returns the lowercased, quote-stripped
//! receiver name if the receiver expression is a simple identifier; `None` for
//! compound expressions.
//!
//! Accepts:
//!   - `Identifier`          → lowercased identifier text
//!   - `"Quoted Identifier"` → inner text, lowercased
//!
//! Rejects (→ None):
//!   - empty / whitespace-only
//!   - anything containing `.`, `(`, `[`
//!   - unquoted identifiers containing whitespace (space or tab)
//!   - a quoted identifier whose inner text contains `(` or `[`

/// JS `String.prototype.trim()` whitespace set (see al_type.rs for the spec
/// rationale). al-sem's `simpleReceiverName` calls `.trim()` then makes its
/// later structural decisions on the trimmed string, so we trim identically.
fn js_trim(s: &str) -> &str {
    fn is_ws(c: char) -> bool {
        matches!(
            c,
            '\u{0009}'
                | '\u{000A}'
                | '\u{000B}'
                | '\u{000C}'
                | '\u{000D}'
                | '\u{0020}'
                | '\u{00A0}'
                | '\u{1680}'
                | '\u{2000}'
                ..='\u{200A}'
                    | '\u{2028}'
                    | '\u{2029}'
                    | '\u{202F}'
                    | '\u{205F}'
                    | '\u{3000}'
                    | '\u{FEFF}'
        )
    }
    let start = s.char_indices().find(|&(_, c)| !is_ws(c)).map(|(i, _)| i);
    let Some(start) = start else { return "" };
    let end = s
        .char_indices()
        .rev()
        .find(|&(_, c)| !is_ws(c))
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or(s.len());
    &s[start..end]
}

/// Returns the lowercased, quote-stripped receiver name if `receiver_text` is a
/// simple identifier; `None` for compound expressions. Faithful port of
/// al-sem `simpleReceiverName`.
pub fn simple_receiver_name(receiver_text: &str) -> Option<String> {
    if receiver_text.is_empty() {
        return None;
    }

    let trimmed = js_trim(receiver_text);
    if trimmed.is_empty() {
        return None;
    }

    // Quoted identifier: must start AND end with a double-quote.
    if trimmed.starts_with('"') {
        // TS: `if (!trimmed.endsWith('"') || trimmed.length < 2) return undefined;`
        if !trimmed.ends_with('"') || trimmed.chars().count() < 2 {
            return None;
        }
        let inner = &trimmed[1..trimmed.len() - 1];
        // A quoted identifier containing compound-expression chars is malformed.
        if inner.contains('(') || inner.contains('[') {
            return None;
        }
        return Some(inner.to_lowercase());
    }

    // Unquoted identifier: must not contain compound-expression chars or
    // whitespace (TS checks `.`, `(`, `[`, space, tab explicitly).
    if trimmed.contains('.')
        || trimmed.contains('(')
        || trimmed.contains('[')
        || trimmed.contains(' ')
        || trimmed.contains('\t')
    {
        return None;
    }

    Some(trimmed.to_lowercase())
}
