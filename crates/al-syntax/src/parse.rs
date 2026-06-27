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

    #[test]
    fn lowers_outer_structure() {
        use crate::ir::{ObjectKind, RoutineKind};
        let src = "codeunit 50000 Foo\n{\n    var\n        G: Integer;\n    \
                   procedure Bar(var X: Integer; Y: Code[20]): Boolean\n    var\n        \
                   L: Text;\n    begin\n    end;\n\n    trigger OnRun()\n    begin\n    end;\n}\n";
        let f = parse(src);
        assert_eq!(f.parse_status, ParseStatus::Clean);
        assert_eq!(f.objects.len(), 1);
        let o = &f.objects[0];
        assert_eq!(o.kind, ObjectKind::Codeunit);
        assert_eq!(o.id, Some(50000));
        assert_eq!(o.name, "Foo");
        assert_eq!(o.globals.len(), 1, "object global G");
        assert_eq!(o.globals[0].name, "G");
        assert_eq!(o.routines.len(), 2, "Bar + OnRun");
        let bar = o.routines.iter().find(|r| r.name == "Bar").expect("Bar");
        assert_eq!(bar.kind, RoutineKind::Procedure);
        assert_eq!(bar.params.len(), 2);
        assert!(bar.params[0].by_ref, "var X");
        assert!(!bar.params[1].by_ref, "Y");
        assert_eq!(bar.return_type.as_deref(), Some("Boolean"));
        assert_eq!(bar.locals.len(), 1);
        assert_eq!(bar.locals[0].name, "L");
        let onrun = o.routines.iter().find(|r| r.name == "OnRun").expect("OnRun");
        assert_eq!(onrun.kind, RoutineKind::Trigger);
    }

    #[test]
    fn lowers_statement_body() {
        use crate::ir::{BlockItem, StmtKind};
        let src = "codeunit 1 A\n{\n    procedure P()\n    var\n        i: Integer;\n    begin\n        \
                   i := 1;\n        if i > 0 then\n            Message('x')\n        else\n            \
                   Clear(i);\n        while i < 10 do\n            i += 1;\n    end;\n}\n";
        let f = parse(src);
        assert_eq!(f.parse_status, ParseStatus::Clean);
        let r = &f.objects[0].routines[0];
        let body = r.body.expect("body");
        let blk = f.ir.block(body);
        assert_eq!(blk.items.len(), 3, "assignment, if, while");
        match blk.items[0] {
            BlockItem::Stmt(sid) => {
                assert!(matches!(f.ir.stmt(sid).kind, StmtKind::Assignment { .. }));
            }
            _ => panic!("expected stmt"),
        }
        match blk.items[1] {
            BlockItem::Stmt(sid) => {
                assert!(matches!(f.ir.stmt(sid).kind, StmtKind::If { .. }));
            }
            _ => panic!("expected if"),
        }
        let msgs: Vec<&String> = f.issues.iter().map(|i| &i.message).collect();
        assert!(f.issues.is_empty(), "unexpected unlowered nodes: {:?}", msgs);
    }
}
