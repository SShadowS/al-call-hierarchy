//! Parse entry point: AL source → owned [`AlFile`]. The tree-sitter `Tree` lives
//! only for the duration of lowering; everything the engine needs is copied into
//! the owned IR before it drops.

use crate::ir::AlFile;
use crate::lower;
use crate::raw::RawNode;

/// Parse + lower one AL source file.
pub fn parse(source: &str) -> AlFile {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&crate::language::language())
        .expect("load AL grammar");
    let tree = parser
        .parse(source, None)
        .expect("tree-sitter parse returned None");
    lower::lower_file(RawNode::new(tree.root_node()), source)
}

#[cfg(test)]
mod tests {
    use super::parse;
    use crate::ir::ParseStatus;

    #[test]
    fn parses_minimal_codeunit() {
        let f = parse("codeunit 50000 Foo\n{\n    procedure Bar()\n    begin\n    end;\n}\n");
        assert_eq!(f.parse_status, ParseStatus::Clean);
    }

    #[test]
    fn flags_recovery_on_broken_source() {
        let f = parse("codeunit 50000 Foo\n{\n    procedure Bar(  @@@ \n");
        assert_eq!(f.parse_status, ParseStatus::Recovered);
    }
}
