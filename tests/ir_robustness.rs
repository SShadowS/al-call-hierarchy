//! Owned-IR L2 cutover — robustness / anti-regression suite.
//!
//! The L2 emitter is now driven entirely by the owned `al_syntax` IR (no tree-sitter
//! CST walk, no legacy fallback). Byte-for-byte parity over the well-formed corpus
//! proves the happy path; these tests lock down the edge cases that corpus parity
//! structurally CANNOT (external review, gpt-5.5 + gemini-3.1-pro, flagged each as a
//! potential silent-correctness hole):
//!
//!   1. Malformed routines must NOT be dropped from the emitted set, and a malformed
//!      routine must not swallow following routines (the binary trust-boundary risk).
//!   2. Quoted-identifier names with embedded doubled quotes (`""`) must normalize
//!      IDENTICALLY to the legacy `strip_quotes` (outer-quote strip only) — the
//!      stable-routine-id hash depends on it.
//!   3. An extension object's number is the extension's OWN id, never a numeric target.
//!   4. `ir_object_type` must equal the legacy `object_type_for` for every object kind.
//!   5. `al_syntax::parse` + the L2 entry points must not panic on garbage input.

use al_call_hierarchy::engine::l2::ir_walk::ir_object_type;
use al_call_hierarchy::engine::l2::l2_workspace::project_named_routine;
use al_call_hierarchy::engine::l2::scope::object_type_for;

/// Recursively collect tree-sitter routine nodes (matching the legacy emitter's
/// prune-at-match descent).
fn ts_routine_nodes<'t>(n: tree_sitter::Node<'t>, out: &mut Vec<tree_sitter::Node<'t>>) {
    let mut c = n.walk();
    for ch in n.named_children(&mut c) {
        if ch.kind() == "procedure" || ch.kind() == "trigger_declaration" {
            out.push(ch);
        } else {
            ts_routine_nodes(ch, out);
        }
    }
}

fn parse_ts(src: &str) -> tree_sitter::Tree {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&al_call_hierarchy::language::language())
        .expect("set AL language");
    parser.parse(src, None).expect("parse")
}

/// A malformed routine must still be EMITTED (by start byte), and a parse error in
/// one routine must not drop a following routine. The invariant: every tree-sitter
/// routine node carrying a recoverable name has an IR routine at the same start byte.
#[test]
fn malformed_routines_are_not_dropped() {
    // Adversarial malformed bodies, each followed by a well-formed routine that must
    // survive. Families: stray token, unterminated call, missing inner end, missing
    // semicolon, malformed case branch, malformed param list.
    let fixtures = [
        // stray token mid-body
        "codeunit 50100 A\n{\n procedure Broken() begin Foo(); @@@ end;\n procedure After() begin Bar(); end;\n}\n",
        // unterminated call argument list + missing inner end
        "codeunit 50101 B\n{\n procedure P() begin Foo(); if X then begin Bar(  Baz(); end;\n procedure Q() begin Ok(); end;\n}\n",
        // missing semicolon between two calls
        "codeunit 50102 C\n{\n procedure P() begin Foo() Bar(); end;\n procedure R() begin Fine(); end;\n}\n",
        // malformed case branch
        "codeunit 50103 D\n{\n procedure P() begin case X of 1: ; 2 Bad(); end; end;\n procedure S() begin Good(); end;\n}\n",
        // malformed parameter list after a valid name
        "codeunit 50104 E\n{\n procedure P(var : ) begin end;\n procedure T() begin Last(); end;\n}\n",
    ];

    for src in fixtures {
        let tree = parse_ts(src);
        let mut ts_nodes = Vec::new();
        ts_routine_nodes(tree.root_node(), &mut ts_nodes);
        let ts_named: std::collections::HashMap<usize, String> = ts_nodes
            .iter()
            .filter_map(|n| {
                let nm = n.child_by_field_name("name")?;
                let t = nm.utf8_text(src.as_bytes()).ok()?;
                let t = t.trim().trim_matches('"');
                if t.is_empty() {
                    None
                } else {
                    Some((n.start_byte(), t.to_string()))
                }
            })
            .collect();

        let file = al_syntax::parse(src);
        let ir_bytes: std::collections::HashSet<usize> = file
            .objects
            .iter()
            .flat_map(|o| o.routines.iter().map(|r| r.origin.byte.start))
            .collect();

        for (byte, name) in &ts_named {
            assert!(
                ir_bytes.contains(byte),
                "malformed-input routine `{name}` (start byte {byte}) was DROPPED by the IR.\n  src: {src:?}"
            );
        }
    }
}

/// Quoted identifiers with embedded doubled quotes must match the legacy
/// `strip_quotes` (outer strip only) — NOT a semantic unescape. If the IR unescaped
/// `"A ""B"""` to `A "B"`, the stable-routine-id hash would silently drift.
#[test]
fn quoted_identifier_doubled_quotes_match_legacy_strip() {
    let src = concat!(
        "codeunit 50100 \"My \"\"Obj\"\"\"\n",
        "{\n",
        "    procedure \"A \"\"B\"\"\"()\n",
        "    begin\n",
        "    end;\n",
        "}\n",
    );
    let file = al_syntax::parse(src);
    assert_eq!(file.objects.len(), 1);
    let o = &file.objects[0];
    // Legacy strip_quotes strips only the OUTER pair, preserving doubled inner quotes.
    assert_eq!(o.name, "My \"\"Obj\"\"", "object name outer-strip only");
    assert_eq!(o.routines.len(), 1);
    assert_eq!(
        o.routines[0].name, "A \"\"B\"\"",
        "routine name outer-strip only"
    );

    // And the routine is resolvable by that exact (legacy-form) name through the
    // production entry point — i.e. the stable id is computed from the legacy form.
    let r = project_named_routine(src, "A \"\"B\"\"", "app", "ws:t.al");
    assert!(r.is_some(), "routine not found by its legacy-stripped name");
}

/// An extension object's number is the extension's OWN id, even when the target
/// could be read as numeric — never the target.
#[test]
fn extension_object_number_is_declaration_id() {
    let cases = [
        (
            "tableextension 50100 \"My Ext\" extends Customer\n{\n}\n",
            50100,
        ),
        (
            "pageextension 50111 PExt extends \"Customer Card\"\n{\n}\n",
            50111,
        ),
        (
            "enumextension 50122 EExt extends \"My Enum\"\n{\n}\n",
            50122,
        ),
    ];
    for (src, want) in cases {
        let file = al_syntax::parse(src);
        assert_eq!(file.objects.len(), 1, "one object: {src:?}");
        assert_eq!(
            file.objects[0].id,
            Some(want),
            "extension number must be its own id: {src:?}"
        );
    }
}

/// `ir_object_type(ObjectKind)` must equal the legacy node-kind `object_type_for`
/// for every emitted object kind — and skip (None) the same set. Drives a per-kind
/// snippet through BOTH paths and asserts equality.
#[test]
fn ir_object_type_matches_legacy_object_type_for() {
    // (source declaring exactly one object, tree-sitter decl node kind)
    let cases: &[&str] = &[
        "codeunit 50100 X\n{\n}\n",
        "table 50100 X\n{\n}\n",
        "tableextension 50100 X extends Customer\n{\n}\n",
        "page 50100 X\n{\n}\n",
        "pageextension 50100 X extends \"Customer Card\"\n{\n}\n",
        "report 50100 X\n{\n}\n",
        "reportextension 50100 X extends \"Customer List\"\n{\n}\n",
        "query 50100 X\n{\n}\n",
        "xmlport 50100 X\n{\n}\n",
        "enum 50100 X\n{\n}\n",
        "enumextension 50100 X extends \"My Enum\"\n{\n}\n",
        "interface X\n{\n}\n",
        "controladdin X\n{\n}\n",
        "permissionset 50100 X\n{\n}\n",
        "permissionsetextension 50100 X extends Y\n{\n}\n",
        "profile X\n{\n}\n",
        "entitlement X\n{\n}\n",
    ];
    for src in cases {
        let tree = parse_ts(src);
        let mut c = tree.root_node().walk();
        let top: Vec<tree_sitter::Node> = tree.root_node().named_children(&mut c).collect();
        let decl = top
            .iter()
            .find(|n| n.kind().ends_with("_declaration"))
            .copied()
            .or_else(|| top.first().copied())
            .expect("a top-level decl");
        let legacy = object_type_for(decl.kind());

        let file = al_syntax::parse(src);
        assert_eq!(file.objects.len(), 1, "one object: {src:?}");
        let ir = ir_object_type(&file.objects[0].kind);

        assert_eq!(
            legacy.map(|s| s.to_string()),
            ir.map(|s| s.to_string()),
            "object-type mapping diverges for {src:?}: legacy={legacy:?} ir={ir:?} (ts kind {})",
            decl.kind()
        );
    }
}

/// Neither `al_syntax::parse` nor the L2 entry points may panic on garbage input.
#[test]
fn no_panic_on_garbage() {
    let garbage = [
        "",
        "\u{0}\u{1}\u{2}",
        "procedure",
        "codeunit",
        "}}}{{{",
        "codeunit 50100 \"unterminated",
        "table\n{\nfield(1;Foo;",
        "🦀 procedure begin end",
        "codeunit 9 A { procedure P() begin ",
    ];
    for src in garbage {
        let f = al_syntax::parse(src);
        // Touch the routines so any lazy panic surfaces.
        let _ = f.objects.iter().map(|o| o.routines.len()).sum::<usize>();
        let _ = project_named_routine(src, "P", "app", "ws:g.al");
    }
}

/// Report dataitems are modelled in the owned IR (Phase-3 prep). A dataitem trigger's
/// implicit `Rec` is typed to its enclosing dataitem's source table, and every report
/// routine sees the dataitem NAMES as record vars typed to their source tables. This
/// exercises `project_routine_features_ir` directly (the IR-driven L2 path), so the
/// modeling is validated independent of the L3 emitter.
#[test]
fn report_dataitem_implicit_rec_and_name_vars_seeded() {
    use al_call_hierarchy::engine::l2::ir_walk;
    use al_call_hierarchy::engine::l2::scope::{compute_routine_id, ParameterSymbol};
    let src = concat!(
        "report 50100 \"My Report\"\n{\n",
        "    dataset\n    {\n",
        "        dataitem(Cust; Customer)\n        {\n",
        "            trigger OnAfterGetRecord() begin Rec.TableCaption(); end;\n",
        "        }\n    }\n",
        "    procedure Helper() begin end;\n",
        "}\n",
    );
    let f = al_syntax::parse(src);
    let o = &f.objects[0];
    // The report carries its dataitem (name, source-table) in the IR.
    assert_eq!(
        o.report_dataitems,
        vec![("Cust".to_string(), "Customer".to_string())]
    );

    let oi = 0usize;
    let no_params: Vec<ParameterSymbol> = vec![];
    // The dataitem trigger: implicit `Rec` typed to Customer + a `Cust` dataitem-name var.
    let trig = o
        .routines
        .iter()
        .find(|r| r.name == "OnAfterGetRecord")
        .expect("trigger");
    assert_eq!(trig.dataitem_source_table.as_deref(), Some("Customer"));
    let rid = compute_routine_id(
        "a", "Report", 50100, "trigger", &trig.name, &no_params, None, "a",
    );
    let feats = ir_walk::project_routine_features_ir(&f, oi, trig, &rid, src, "a", None);
    let rec = feats
        .record_variables
        .iter()
        .find(|rv| rv.name.eq_ignore_ascii_case("Rec"))
        .expect("implicit Rec seeded in the dataitem trigger");
    assert_eq!(
        rec.table_name.as_deref(),
        Some("Customer"),
        "dataitem trigger Rec typed to the dataitem source table"
    );
    assert!(
        feats
            .record_variables
            .iter()
            .any(|rv| rv.name == "Cust" && rv.table_name.as_deref() == Some("Customer")),
        "dataitem name `Cust` seeded as a record var typed to Customer"
    );

    // A report-level procedure (NOT a dataitem trigger): no implicit dataitem Rec, but
    // the dataitem NAME var is still in scope across the whole report.
    let helper = o
        .routines
        .iter()
        .find(|r| r.name == "Helper")
        .expect("Helper");
    assert_eq!(helper.dataitem_source_table, None);
    let rid2 = compute_routine_id(
        "a",
        "Report",
        50100,
        "procedure",
        &helper.name,
        &no_params,
        None,
        "a",
    );
    let feats2 = ir_walk::project_routine_features_ir(&f, oi, helper, &rid2, src, "a", None);
    assert!(
        !feats2
            .record_variables
            .iter()
            .any(|rv| rv.name.eq_ignore_ascii_case("Rec")),
        "a report-level procedure has no implicit dataitem Rec"
    );
    assert!(
        feats2.record_variables.iter().any(|rv| rv.name == "Cust"),
        "the dataitem name var is in scope in report-level procedures too"
    );
}
