//! Dual-run parity support (owned-syntax-IR migration). Exposes the LEGACY
//! tree-sitter extraction the engine relies on, reachable from integration tests,
//! so the IR lowerer can be diffed against it. Removed at the Phase 5 seal.

use crate::language;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Parser, Query, QueryCursor};

/// Legacy callee method/function names in a source file, via the engine's `CALLS`
/// query: `@call.simple` (`Foo()`) + `@call.method` (`Rec.SetRange()`).
pub fn legacy_call_methods(source: &str) -> Vec<String> {
    capture_texts(source, language::queries::CALLS, &["call.simple", "call.method"])
}

/// Legacy routine names in a source file: every `procedure` / `trigger`
/// definition, via the same `DEFINITIONS` query the engine uses.
pub fn legacy_routine_names(source: &str) -> Vec<String> {
    capture_texts(source, language::queries::DEFINITIONS, &["proc.name", "trigger.name"])
}

/// Run a query and collect the source text of the named captures, in match order.
fn capture_texts(source: &str, query_src: &str, wanted: &[&str]) -> Vec<String> {
    let lang = language::language();
    let mut parser = Parser::new();
    if parser.set_language(&lang).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source, None) else {
        return Vec::new();
    };
    let Ok(query) = Query::new(&lang, query_src) else {
        return Vec::new();
    };
    let names = query.capture_names();
    let mut out = Vec::new();
    let mut cursor = QueryCursor::new();
    let mut it = cursor.matches(&query, tree.root_node(), source.as_bytes());
    while let Some(m) = it.next() {
        for cap in m.captures {
            if wanted.contains(&names[cap.index as usize]) {
                out.push(source[cap.node.byte_range()].to_string());
            }
        }
    }
    out
}
