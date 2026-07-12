//! Byte<->UTF-16 column conversion (H-12) + LSP `positionEncoding` negotiation.
//!
//! The engine is UTF-8-byte-column-native throughout (`al-syntax`'s IR, the
//! call graph, everything downstream). LSP's mandatory fallback encoding is
//! UTF-16 code units (every client MUST support it per the 3.17 spec; UTF-8
//! is opt-in via `general.positionEncodings`). This module bridges the two
//! without depending on `lsp_types`, so it stays trivially unit-testable.

/// Which column encoding an LSP session negotiated with the client.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PositionEncoding {
    Utf8,
    Utf16,
}

/// Pick a [`PositionEncoding`] from the client's advertised
/// `general.positionEncodings` capability values (LSP 3.17, `general`
/// client capabilities).
///
/// Per spec, UTF-16 is the mandatory fallback: a client that omits the
/// capability entirely, or omits `"utf-8"` from the offered list, gets
/// UTF-16. UTF-8 is negotiated only when the client explicitly lists it.
pub fn negotiate(client_encodings: Option<&[String]>) -> PositionEncoding {
    let offers_utf8 = client_encodings.is_some_and(|encs| encs.iter().any(|e| e == "utf-8"));
    if offers_utf8 {
        PositionEncoding::Utf8
    } else {
        PositionEncoding::Utf16
    }
}

/// Lazy per-line byte<->UTF-16 conversion table for one file's text.
///
/// Lines are split once at construction (cheap: no per-line work happens
/// yet); the actual byte<->UTF-16 unit walk happens on demand inside
/// `col_out`/`col_in`. AL source lines are short, so a per-call
/// `char_indices()` walk is plenty fast — memoize nothing fancier unless
/// profiling ever says otherwise.
pub struct LineTable<'t> {
    lines: Vec<&'t str>,
}

impl<'t> LineTable<'t> {
    pub fn new(text: &'t str) -> Self {
        let lines = text
            .split('\n')
            .map(|l| l.strip_suffix('\r').unwrap_or(l))
            .collect();
        LineTable { lines }
    }

    /// Out-of-range `line` (or a file with no trailing newline) resolves to
    /// an empty line rather than panicking — fail-closed clamp.
    fn line_text(&self, line: u32) -> &'t str {
        self.lines.get(line as usize).copied().unwrap_or("")
    }

    /// UTF-8 byte column (engine-native) -> column in `enc` for LSP output.
    /// Out-of-range `byte_col` clamps to the line's end (fail-closed, never panics).
    pub fn col_out(&self, line: u32, byte_col: u32, enc: PositionEncoding) -> u32 {
        let text = self.line_text(line);
        if enc == PositionEncoding::Utf8 {
            return byte_col.min(text.len() as u32);
        }
        let byte_col = byte_col as usize;
        let mut consumed_bytes = 0usize;
        let mut units = 0u32;
        for c in text.chars() {
            if consumed_bytes >= byte_col {
                break;
            }
            consumed_bytes += c.len_utf8();
            units += c.len_utf16() as u32;
        }
        units
    }

    /// Inbound LSP column in `enc` -> UTF-8 byte column for engine lookups.
    /// Out-of-range `enc_col` clamps to the line's end (fail-closed, never panics).
    pub fn col_in(&self, line: u32, enc_col: u32, enc: PositionEncoding) -> u32 {
        let text = self.line_text(line);
        if enc == PositionEncoding::Utf8 {
            return enc_col.min(text.len() as u32);
        }
        let mut units_seen = 0u32;
        let mut bytes = 0u32;
        for c in text.chars() {
            if units_seen >= enc_col {
                break;
            }
            units_seen += c.len_utf16() as u32;
            bytes += c.len_utf8() as u32;
        }
        bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_conversion_danish_and_emoji() {
        let t = LineTable::new("æøå x\n🚀 y\nplain\n");
        assert_eq!(t.col_out(0, 6, PositionEncoding::Utf16), 3); // after æøå = 6 bytes = 3 UTF-16 units
        assert_eq!(t.col_in(0, 3, PositionEncoding::Utf16), 6);
        assert_eq!(t.col_out(1, 4, PositionEncoding::Utf16), 2); // after 🚀 = 4 bytes = 2 units (surrogate pair)
        assert_eq!(t.col_out(2, 3, PositionEncoding::Utf8), 3); // utf-8 mode: identity
        assert_eq!(t.col_out(0, 999, PositionEncoding::Utf16), 5); // clamp to line end (5 chars -> 5 units)
    }

    #[test]
    fn ascii_passthrough_is_identity_in_both_encodings() {
        let t = LineTable::new("hello world\n");
        assert_eq!(t.col_out(0, 5, PositionEncoding::Utf16), 5);
        assert_eq!(t.col_in(0, 5, PositionEncoding::Utf16), 5);
        assert_eq!(t.col_out(0, 5, PositionEncoding::Utf8), 5);
        assert_eq!(t.col_in(0, 5, PositionEncoding::Utf8), 5);
    }

    #[test]
    fn out_of_range_column_clamps_to_line_end_never_panics() {
        let t = LineTable::new("abc\n");
        assert_eq!(t.col_out(0, 9999, PositionEncoding::Utf8), 3);
        assert_eq!(t.col_in(0, 9999, PositionEncoding::Utf16), 3);
        assert_eq!(t.col_in(0, 9999, PositionEncoding::Utf8), 3);
    }

    #[test]
    fn out_of_range_line_index_clamps_to_empty_line_never_panics() {
        let t = LineTable::new("abc\n");
        assert_eq!(t.col_out(50, 0, PositionEncoding::Utf16), 0);
        assert_eq!(t.col_in(50, 0, PositionEncoding::Utf16), 0);
    }

    #[test]
    fn negotiate_returns_utf8_only_when_client_offers_it() {
        assert_eq!(negotiate(None), PositionEncoding::Utf16);
        assert_eq!(negotiate(Some(&[])), PositionEncoding::Utf16);
        assert_eq!(
            negotiate(Some(&["utf-16".to_string()])),
            PositionEncoding::Utf16
        );
        assert_eq!(
            negotiate(Some(&["utf-32".to_string()])),
            PositionEncoding::Utf16
        );
        assert_eq!(
            negotiate(Some(&["utf-8".to_string()])),
            PositionEncoding::Utf8
        );
        assert_eq!(
            negotiate(Some(&["utf-16".to_string(), "utf-8".to_string()])),
            PositionEncoding::Utf8
        );
    }
}
