//! Raw grammar layer. Generated vocabulary (`RawKind`/`FieldName`) now; typed CST
//! wrappers + shape facts to follow (Phase 0). During the migration this is `pub`
//! so the dual-run harness can compare `Origin`s; it is NOT a tree-sitter type, and
//! it is tightened to crate-internal at the Phase 5 seal.

pub mod generated;
pub mod node;

pub use generated::{FieldName, GRAMMAR_NODE_TYPES_HASH, NAMED_KIND_COUNT, RawKind};
pub use node::RawNode;

#[cfg(test)]
mod tests {
    use super::{FieldName, GRAMMAR_NODE_TYPES_HASH, NAMED_KIND_COUNT, RawKind};

    #[test]
    fn raw_kind_round_trips() {
        assert_eq!(RawKind::from_raw("procedure"), RawKind::Procedure);
        assert_eq!(RawKind::from_raw("code_block"), RawKind::CodeBlock);
        assert_eq!(
            RawKind::from_raw("statement_block"),
            RawKind::StatementBlock
        );
        assert_eq!(
            RawKind::from_raw("declaration_body"),
            RawKind::DeclarationBody
        );
        assert_eq!(RawKind::Procedure.as_str(), "procedure");
        assert_eq!(RawKind::from_raw("ERROR"), RawKind::Error);
        // Update when the grammar adds/removes a NAMED node kind (the generated
        // `NAMED_KIND_COUNT` const is authoritative; this pins it as a sanity anchor).
        assert_eq!(NAMED_KIND_COUNT, 388);
        assert_eq!(GRAMMAR_NODE_TYPES_HASH.len(), 64);
    }

    #[test]
    #[should_panic(expected = "unknown node kind")]
    fn unknown_kind_panics() {
        let _ = RawKind::from_raw("definitely_not_a_real_kind");
    }

    #[test]
    fn field_round_trips() {
        assert_eq!(FieldName::Name.as_raw(), "name");
        assert_eq!(FieldName::Body.as_raw(), "body");
        assert_eq!(FieldName::Member.as_raw(), "member");
    }
}
