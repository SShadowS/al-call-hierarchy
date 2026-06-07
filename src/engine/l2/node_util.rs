//! Shared tree-sitter node helpers for the L2 walker.
//!
//! These mirror `src/parser/ast.ts` + the column semantics al-sem inherits from
//! web-tree-sitter. The single subtlety the Rust tree-sitter crate forces on us
//! is UTF-16 column normalization: tree-sitter reports byte offsets within a
//! line, while web-tree-sitter (and therefore every al-sem anchor) uses UTF-16
//! code-unit columns. [`Utf16Cols`] converts.

use tree_sitter::Node;

/// Iterate a node's NAMED children (mirrors al-sem `namedChildren`).
///
/// NOTE: this tree-sitter version's `named_child` takes a `u32` index.
pub fn named_children<'a>(node: Node<'a>) -> Vec<Node<'a>> {
    let n = node.named_child_count() as u32;
    (0..n).filter_map(|i| node.named_child(i)).collect()
}

/// Source text of a node (UTF-8 byte slice).
pub fn node_text<'a>(node: Node, source: &'a str) -> &'a str {
    &source[node.byte_range()]
}

/// First named child of a given kind, or None.
pub fn child_of_kind<'a>(node: Node<'a>, kind: &str) -> Option<Node<'a>> {
    named_children(node).into_iter().find(|c| c.kind() == kind)
}

/// Strip surrounding double quotes — matches al-sem `stripQuotes`: only strips
/// when text is >= 2 chars AND starts with `"` AND ends with `"`.
pub fn strip_quotes(text: &str) -> &str {
    let mut chars = text.chars();
    let first = chars.next();
    let last = chars.next_back();
    if first == Some('"') && last == Some('"') {
        &text[1..text.len() - 1]
    } else {
        text
    }
}

/// Strip a single layer of surrounding double OR single quotes (al-sem
/// `stripQuoteChars` in expression-from-node.ts / callee-from-node.ts).
pub fn strip_quote_chars(text: &str) -> &str {
    let mut chars = text.chars();
    let first = chars.next();
    let last = chars.next_back();
    if (first == Some('"') && last == Some('"')) || (first == Some('\'') && last == Some('\'')) {
        // first/last are ASCII quotes (1 byte each).
        &text[1..text.len() - 1]
    } else {
        text
    }
}

/// Column converter, keyed per source.
///
/// EMPIRICAL FINDING (R1a Task 2, vector families `j`/`k`): the R1a plan assumed
/// al-sem's anchors use UTF-16 code-unit columns and the Rust tree-sitter crate
/// uses UTF-8 byte columns, requiring conversion. That is NOT the case for this
/// grammar + binding: the committed oracle vectors' non-ASCII columns
/// (`Message('é'); Cust.FindSet()` → FindSet startColumn 23; `Cust.SetFilter("Naïve
/// Field", …)` → endColumn 47) match the Rust tree-sitter `start_position().column`
/// (a UTF-8 byte offset within the line) EXACTLY — web-tree-sitter reports byte
/// columns too. Converting to UTF-16 would shift those columns DOWN by 1 per
/// non-ASCII char and break parity. So `col` is an identity pass-through over the
/// tree-sitter byte column. The type is retained as the single choke point for
/// column emission in case a future grammar/binding diverges. See the Task 2
/// report for the full rationale.
pub struct Utf16Cols<'a> {
    _source: &'a str,
}

impl<'a> Utf16Cols<'a> {
    pub fn new(source: &'a str) -> Self {
        Self { _source: source }
    }

    /// Return the tree-sitter byte column verbatim (matches al-sem's anchors).
    pub fn col(&self, _row: usize, byte_col: usize) -> u32 {
        byte_col as u32
    }
}
