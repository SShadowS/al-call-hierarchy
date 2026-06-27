//! PHASE-2/3 precondition — the IR object/routine SET (by start byte) must match
//! the tree-sitter decl/routine set the legacy emitter iterates. If they match,
//! the L2 emitter can iterate IR objects directly (drop the live tree-sitter
//! parse). Divergences are the only thing that would change the emitted object set.

use std::path::Path;

fn collect_al_files(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().map(|x| x == "al").unwrap_or(false) {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

fn object_type_for(kind: &str) -> bool {
    al_call_hierarchy::engine::l2::scope::object_type_for(kind).is_some()
}

fn routine_nodes<'t>(n: tree_sitter::Node<'t>, out: &mut Vec<tree_sitter::Node<'t>>) {
    let mut c = n.walk();
    for ch in n.named_children(&mut c) {
        if ch.kind() == "procedure" || ch.kind() == "trigger_declaration" {
            out.push(ch);
        } else {
            routine_nodes(ch, out);
        }
    }
}

#[test]
fn ir_object_and_routine_set_parity() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/r0-corpus");
    if !root.is_dir() {
        return;
    }
    let lang = al_call_hierarchy::language::language();
    let mut obj_total = 0usize;
    let mut obj_match = 0usize;
    let mut rout_total = 0usize;
    let mut rout_match = 0usize;
    let mut obj_div: Vec<String> = Vec::new();
    let mut rout_div: Vec<String> = Vec::new();
    for fpath in collect_al_files(&root) {
        let Ok(src) = std::fs::read_to_string(&fpath) else {
            continue;
        };
        let mut parser = tree_sitter::Parser::new();
        if parser.set_language(&lang).is_err() {
            continue;
        }
        let Some(tree) = parser.parse(&src, None) else {
            continue;
        };

        // tree-sitter object decls (top-level + recursive object_type_for)
        let mut ts_obj_bytes: std::collections::HashSet<usize> = Default::default();
        let mut c = tree.root_node().walk();
        for decl in tree.root_node().named_children(&mut c) {
            if object_type_for(decl.kind()) {
                ts_obj_bytes.insert(decl.start_byte());
            }
        }
        let mut ts_rout: Vec<tree_sitter::Node> = Vec::new();
        routine_nodes(tree.root_node(), &mut ts_rout);
        let ts_rout_bytes: std::collections::HashSet<usize> =
            ts_rout.iter().map(|n| n.start_byte()).collect();

        let file = al_syntax::parse(&src);
        let ir_obj_bytes: std::collections::HashSet<usize> =
            file.objects.iter().map(|o| o.origin.byte.start).collect();
        let ir_rout_bytes: std::collections::HashSet<usize> = file
            .objects
            .iter()
            .flat_map(|o| o.routines.iter().map(|r| r.origin.byte.start))
            .collect();

        obj_total += ts_obj_bytes.len();
        obj_match += ts_obj_bytes.intersection(&ir_obj_bytes).count();
        if ts_obj_bytes != ir_obj_bytes {
            obj_div.push(format!(
                "{}: ts={} ir={}",
                fpath.display(),
                ts_obj_bytes.len(),
                ir_obj_bytes.len()
            ));
        }
        rout_total += ts_rout_bytes.len();
        rout_match += ts_rout_bytes.intersection(&ir_rout_bytes).count();
        if ts_rout_bytes != ir_rout_bytes {
            let missing: Vec<_> = ts_rout_bytes.difference(&ir_rout_bytes).collect();
            let extra: Vec<_> = ir_rout_bytes.difference(&ts_rout_bytes).collect();
            rout_div.push(format!(
                "{}: ts_only={:?} ir_only={:?}",
                fpath.display(),
                missing,
                extra
            ));
        }
    }
    eprintln!(
        "\n=== OBJECT set: {obj_match}/{obj_total} ; ROUTINE set: {rout_match}/{rout_total} ==="
    );
    for d in &obj_div {
        eprintln!("OBJ DIV {d}");
    }
    for d in &rout_div {
        eprintln!("ROUT DIV {d}");
    }
    eprintln!(
        "obj_div files={} rout_div files={}",
        obj_div.len(),
        rout_div.len()
    );
}

fn ir_object_type(k: &al_syntax::ir::ObjectKind) -> Option<&'static str> {
    use al_syntax::ir::ObjectKind::*;
    Some(match k {
        Codeunit => "Codeunit",
        Table => "Table",
        TableExtension => "TableExtension",
        Page => "Page",
        PageExtension => "PageExtension",
        Report => "Report",
        ReportExtension => "ReportExtension",
        Query => "Query",
        XmlPort => "XMLport",
        Enum => "Enum",
        EnumExtension => "EnumExtension",
        Interface => "Interface",
        ControlAddIn => "ControlAddIn",
        PermissionSet => "PermissionSet",
        // legacy object_type_for returns None for these → skipped
        PermissionSetExtension | Profile | Entitlement | Other => return None,
    })
}

#[test]
fn ir_object_id_inputs_parity() {
    use al_call_hierarchy::engine::l2::node_util::{named_children, node_text, strip_quotes};
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/r0-corpus");
    if !root.is_dir() {
        return;
    }
    let lang = al_call_hierarchy::language::language();
    let mut total = 0usize;
    let mut matching = 0usize;
    let mut div: Vec<String> = Vec::new();
    for fpath in collect_al_files(&root) {
        let Ok(src) = std::fs::read_to_string(&fpath) else {
            continue;
        };
        let mut parser = tree_sitter::Parser::new();
        if parser.set_language(&lang).is_err() {
            continue;
        }
        let Some(tree) = parser.parse(&src, None) else {
            continue;
        };
        // tree-sitter: (byte) -> (type, number, name)
        let mut ts: std::collections::HashMap<usize, (String, i64, String)> = Default::default();
        let mut c = tree.root_node().walk();
        for decl in tree.root_node().named_children(&mut c) {
            if let Some(ot) = al_call_hierarchy::engine::l2::scope::object_type_for(decl.kind()) {
                let num = named_children(decl)
                    .into_iter()
                    .find(|ch| ch.kind() == "integer")
                    .and_then(|ch| node_text(ch, &src).trim().parse::<i64>().ok())
                    .unwrap_or(0);
                let mut name = String::new();
                for ch in named_children(decl) {
                    match ch.kind() {
                        "quoted_identifier" => {
                            name = strip_quotes(node_text(ch, &src)).to_string();
                            break;
                        }
                        "identifier" => {
                            name = node_text(ch, &src).to_string();
                            break;
                        }
                        _ => {}
                    }
                }
                ts.insert(decl.start_byte(), (ot.to_string(), num, name));
            }
        }
        let file = al_syntax::parse(&src);
        let mut ir: std::collections::HashMap<usize, (String, i64, String)> = Default::default();
        for o in &file.objects {
            if let Some(ot) = ir_object_type(&o.kind) {
                ir.insert(
                    o.origin.byte.start,
                    (ot.to_string(), o.id.unwrap_or(0), o.name.clone()),
                );
            }
        }
        // compare keysets + values
        let all: std::collections::HashSet<usize> = ts.keys().chain(ir.keys()).copied().collect();
        for b in all {
            total += 1;
            match (ts.get(&b), ir.get(&b)) {
                (Some(a), Some(c)) if a == c => matching += 1,
                (a, c) => div.push(format!("{} @{}: ts={:?} ir={:?}", fpath.display(), b, a, c)),
            }
        }
    }
    eprintln!("\n=== OBJECT ID inputs (type,number,name): {matching}/{total} ===");
    for d in div.iter().take(30) {
        eprintln!("DIV {d}");
    }
    eprintln!("div count={}", div.len());
    assert_eq!(matching, total, "object id-input divergences");
}
