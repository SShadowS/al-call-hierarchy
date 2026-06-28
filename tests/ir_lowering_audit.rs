//! Lowering-audit / resilience gate for the owned IR.
//!
//! Now that NOTHING downstream falls back to tree-sitter, a payload-free
//! `StmtKind::Unknown` / `ExprKind::Unknown` is the only way a real call/var/op
//! fact can silently vanish. This gate locks down two properties of
//! `al_syntax::parse`:
//!
//!  1. **Full coverage on well-formed code** — the whole r0-corpus lowers with ZERO
//!     `Unknown` nodes. A change that drops a construct to `Unknown` (a lowering
//!     gap) fails here instead of silently shrinking the call graph.
//!  2. **No silent drops + no panic** — over adversarial malformed/garbage input the
//!     parser never panics, and every `Unknown` node it DOES emit is accompanied by
//!     a `SyntaxIssue` (the loss is recorded, never silent).

use std::path::{Path, PathBuf};

use al_syntax::ir::{ExprKind, StmtKind};

fn collect_al_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect_al_files(&p, out);
        } else if p.extension().and_then(|s| s.to_str()) == Some("al") {
            out.push(p);
        }
    }
}

fn count_unknown(f: &al_syntax::ir::AlFile) -> (usize, usize) {
    let ustmt =
        f.ir.iter_stmts()
            .filter(|s| matches!(s.kind, StmtKind::Unknown))
            .count();
    let uexpr =
        f.ir.iter_exprs()
            .filter(|e| matches!(e.kind, ExprKind::Unknown))
            .count();
    (ustmt, uexpr)
}

#[test]
fn clean_corpus_lowers_with_zero_unknown_nodes() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("r0-corpus");
    let mut files = Vec::new();
    collect_al_files(&corpus, &mut files);
    assert!(files.len() > 100, "expected a populated corpus");
    files.sort();

    let mut offenders: Vec<String> = Vec::new();
    for path in &files {
        let Ok(src) = std::fs::read_to_string(path) else {
            continue;
        };
        let f = al_syntax::parse(&src);
        let (ustmt, uexpr) = count_unknown(&f);
        if ustmt + uexpr > 0 {
            offenders.push(format!(
                "{}: {ustmt} Unknown stmt, {uexpr} Unknown expr",
                path.strip_prefix(&corpus).unwrap_or(path).display()
            ));
        }
    }
    assert!(
        offenders.is_empty(),
        "the lowerer dropped well-formed constructs to `Unknown` (a lowering gap — \
         each is a silent call/var/op loss):\n{}",
        offenders.join("\n")
    );
}

#[test]
fn malformed_input_never_panics_and_records_every_unknown() {
    // Adversarial inputs: empty/garbage, truncated objects, and structurally-broken
    // AL. None may panic; every `Unknown` node must be matched by a `SyntaxIssue`
    // (the loss is recorded, not silent).
    let fixtures = [
        "",
        "\u{0}\u{1}\u{2}",
        "procedure",
        "}}}{{{",
        "codeunit 50100 \"unterminated",
        "table\n{\nfield(1;Foo;",
        "🦀 procedure begin end",
        "codeunit 9 A { procedure P() begin ",
        "codeunit 1 A{ procedure P() begin @@@ ; ### ; end; }",
        "codeunit 1 A{ procedure P() var x: begin end; }",
        "codeunit 1 A{ procedure P() begin case of end; }",
        "codeunit 1 A{ procedure P() begin if then else end; }",
    ];
    for src in fixtures {
        let f = al_syntax::parse(src);
        // Touch the whole IR so any lazy panic surfaces.
        let (ustmt, uexpr) = count_unknown(&f);
        let unknowns = ustmt + uexpr;
        // Every Unknown is recorded as a SyntaxIssue (one issue per Unknown, plus any
        // recovery issues) — so issues must at least cover the Unknowns. The contract
        // is "Unknown is never a silent drop": its Origin + a SyntaxIssue record it.
        assert!(
            f.issues.len() >= unknowns,
            "{unknowns} Unknown node(s) but only {} SyntaxIssue(s) — a silent drop.\n  src: {src:?}",
            f.issues.len()
        );
    }
}
