//! Dual-run parity harness (owned-syntax-IR migration, Phase 1b).
//!
//! Compares the LEGACY CST walk against the NEW al-syntax IR lowerer on real `.al`
//! corpus files, one feature stream at a time, driving the lowerer to parity. This
//! first stage compares the **routine inventory** (object + procedure/trigger
//! names) — proves the harness + the lowerer's outer-structure fidelity. Deeper
//! streams (call sites, ops, refs) are added as the IR-side extractor grows.
//!
//! Run `cargo test --test ir_dual_run -- --nocapture` to see the parity report.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use al_call_hierarchy::dual_run_support::legacy_routine_names;

/// Case-insensitive, quote-stripped normalization (AL identifiers are
/// case-insensitive; the IR strips quotes, so normalize both sides the same way).
fn norm(s: &str) -> String {
    let s = s.trim();
    let s = s.strip_prefix('"').and_then(|x| x.strip_suffix('"')).unwrap_or(s);
    s.to_ascii_lowercase()
}

fn legacy_routines(source: &str) -> BTreeSet<String> {
    legacy_routine_names(source).iter().map(|n| norm(n)).collect()
}

fn ir_routines(source: &str) -> BTreeSet<String> {
    let file = al_syntax::parse(source);
    file.objects
        .iter()
        .flat_map(|o| o.routines.iter())
        .map(|r| norm(&r.name))
        .collect()
}

/// IR callee method/function names: for each `Call`, the function expr's name
/// (`Identifier` / `Member.member`). Mirrors legacy CALLS `@call.simple` /
/// `@call.method`.
fn ir_call_methods(source: &str) -> Vec<String> {
    use al_syntax::ir::ExprKind;
    let f = al_syntax::parse(source);
    let mut out = Vec::new();
    for e in f.ir.iter_exprs() {
        if let ExprKind::Call { function, .. } = &e.kind {
            match &f.ir.expr(*function).kind {
                ExprKind::Identifier(n) | ExprKind::QuotedIdentifier(n) => out.push(norm(n)),
                ExprKind::Member { member, .. } => out.push(norm(member)),
                _ => {}
            }
        }
    }
    out
}

/// IR member names: every `Member.member`.
fn ir_member_names(source: &str) -> Vec<String> {
    use al_syntax::ir::ExprKind;
    let f = al_syntax::parse(source);
    f.ir.iter_exprs()
        .filter_map(|e| match &e.kind {
            ExprKind::Member { member, .. } => Some(member.clone()),
            _ => None,
        })
        .collect()
}

/// IR variable names: object globals + routine locals (NOT parameters — legacy
/// `variable_declaration` excludes params).
fn ir_variable_names(source: &str) -> Vec<String> {
    let f = al_syntax::parse(source);
    let mut out = Vec::new();
    for o in &f.objects {
        out.extend(o.globals.iter().map(|v| v.name.clone()));
        for r in &o.routines {
            out.extend(r.locals.iter().map(|v| v.name.clone()));
        }
    }
    out
}

/// IR statement-kind multiset (kind strings matching the legacy node kinds).
fn ir_statement_kinds(source: &str) -> Vec<String> {
    use al_syntax::ir::StmtKind;
    let f = al_syntax::parse(source);
    f.ir.iter_stmts()
        .filter_map(|s| {
            Some(match &s.kind {
                StmtKind::If { .. } => "if_statement",
                StmtKind::While { .. } => "while_statement",
                StmtKind::Repeat { .. } => "repeat_statement",
                StmtKind::For { .. } => "for_statement",
                StmtKind::Foreach { .. } => "foreach_statement",
                StmtKind::With { .. } => "with_statement",
                StmtKind::Case { .. } => "case_statement",
                StmtKind::Assignment { .. } => "assignment_statement",
                StmtKind::Exit(_) => "exit_statement",
                StmtKind::Break => "break_statement",
                StmtKind::Continue => "continue_statement",
                StmtKind::AssertError(_) => "asserterror_statement",
                _ => return None,
            }
            .to_string())
        })
        .collect()
}

/// IR temporary-variable names (globals + locals where `temporary`).
fn ir_temporary_var_names(source: &str) -> Vec<String> {
    let f = al_syntax::parse(source);
    let mut out = Vec::new();
    for o in &f.objects {
        out.extend(o.globals.iter().filter(|v| v.temporary).map(|v| v.name.clone()));
        for r in &o.routines {
            out.extend(r.locals.iter().filter(|v| v.temporary).map(|v| v.name.clone()));
        }
    }
    out
}

// ---- per-routine IR traversal (for real-L2 PFeatures parity) ----

fn block_branches(f: &al_syntax::ir::AlFile, bid: al_syntax::ir::BlockId) -> bool {
    use al_syntax::ir::BlockItem;
    for item in &f.ir.block(bid).items {
        match item {
            BlockItem::Stmt(sid) => {
                if stmt_branches(f, *sid) {
                    return true;
                }
            }
            BlockItem::Preproc(g) => {
                for b in &g.branches {
                    if block_branches(f, *b) {
                        return true;
                    }
                }
            }
        }
    }
    false
}

fn stmt_branches(f: &al_syntax::ir::AlFile, sid: al_syntax::ir::StmtId) -> bool {
    use al_syntax::ir::StmtKind::*;
    match &f.ir.stmt(sid).kind {
        // legacy has_branching = if / case / try present (NOT loops).
        If { .. } | Case { .. } | Try { .. } => true,
        While { body, .. }
        | Repeat { body, .. }
        | For { body, .. }
        | Foreach { body, .. }
        | With { body, .. }
        | AssertError(body)
        | Block(body) => block_branches(f, *body),
        _ => false,
    }
}

/// IR max loop-nesting depth of a block (loops by containment; if/case transparent).
fn block_loop_nesting(f: &al_syntax::ir::AlFile, bid: al_syntax::ir::BlockId, depth: u32) -> u32 {
    use al_syntax::ir::BlockItem;
    let mut m = 0;
    for item in &f.ir.block(bid).items {
        match item {
            BlockItem::Stmt(sid) => m = m.max(stmt_loop_nesting(f, *sid, depth)),
            BlockItem::Preproc(g) => {
                for b in &g.branches {
                    m = m.max(block_loop_nesting(f, *b, depth));
                }
            }
        }
    }
    m
}

fn stmt_loop_nesting(f: &al_syntax::ir::AlFile, sid: al_syntax::ir::StmtId, depth: u32) -> u32 {
    use al_syntax::ir::StmtKind::*;
    match &f.ir.stmt(sid).kind {
        While { body, .. } | Repeat { body, .. } | For { body, .. } | Foreach { body, .. } => {
            let d = depth + 1;
            d.max(block_loop_nesting(f, *body, d))
        }
        If { then_block, else_block, .. } => {
            let a = block_loop_nesting(f, *then_block, depth);
            let b = else_block.map(|e| block_loop_nesting(f, e, depth)).unwrap_or(0);
            a.max(b)
        }
        Case { branches, else_block, .. } => {
            let mut m = 0;
            for br in branches {
                m = m.max(block_loop_nesting(f, br.body, depth));
            }
            if let Some(e) = else_block {
                m = m.max(block_loop_nesting(f, *e, depth));
            }
            m
        }
        Try { body, catch_block } => {
            let a = block_loop_nesting(f, *body, depth);
            let b = catch_block.map(|c| block_loop_nesting(f, c, depth)).unwrap_or(0);
            a.max(b)
        }
        With { body, .. } | AssertError(body) | Block(body) => block_loop_nesting(f, *body, depth),
        _ => 0,
    }
}

/// IR per-routine `name=nesting_depth` pairs (mirrors PFeatures.nesting_depth).
fn ir_nesting_pairs(source: &str) -> Vec<String> {
    let f = al_syntax::parse(source);
    let mut out = Vec::new();
    for o in &f.objects {
        for r in &o.routines {
            let d = r.body.map(|b| block_loop_nesting(&f, b, 0)).unwrap_or(0);
            out.push(format!("{}={}", r.name, d));
        }
    }
    out
}

/// IR: names of routines whose body contains branching (mirrors PFeatures.has_branching).
fn ir_branching_routines(source: &str) -> Vec<String> {
    let f = al_syntax::parse(source);
    let mut out = Vec::new();
    for o in &f.objects {
        for r in &o.routines {
            if let Some(b) = r.body {
                if block_branches(&f, b) {
                    out.push(r.name.clone());
                }
            }
        }
    }
    out
}

// ---- L2 cutover: record-op ordered trace (spine step 1) ----

/// Record-op anchors per routine, in IR DFS visit order (pre-order at the call).
/// `row:col` of each `X.Method()` where X is a Record var and Method is a record
/// builtin. Mirrors legacy record_operations order.
fn ir_record_op_trace(source: &str) -> Vec<(String, Vec<String>)> {
    use al_syntax::ir::VarDecl;
    let f = al_syntax::parse(source);
    let is_rec = |v: &VarDecl| v.ty.as_deref().map(|t| t.to_ascii_lowercase().starts_with("record")).unwrap_or(false);
    let mut out = Vec::new();
    for o in &f.objects {
        let mut globals: std::collections::HashSet<String> =
            o.globals.iter().filter(|v| is_rec(v)).map(|v| v.name.to_ascii_lowercase()).collect();
        // `Rec`/`xRec` are record receivers by name convention (classify.rs:277),
        // regardless of object type.
        globals.insert("rec".to_string());
        globals.insert("xrec".to_string());
        // A table/tableext method (procedure OR trigger) has an implicit record `Rec`.
        let table_method = matches!(o.kind, al_syntax::ir::ObjectKind::Table | al_syntax::ir::ObjectKind::TableExtension);
        for r in &o.routines {
            let mut rvars = globals.clone();
            rvars.extend(r.params.iter().filter(|p| p.ty.as_deref().map(|t| t.to_ascii_lowercase().starts_with("record")).unwrap_or(false)).map(|p| p.name.to_ascii_lowercase()));
            rvars.extend(r.locals.iter().filter(|v| is_rec(v)).map(|v| v.name.to_ascii_lowercase()));
            let mut anchors = Vec::new();
            // implicit-receiver stack: top = is-current-implicit-receiver-a-record.
            let mut implicit = vec![table_method];
            if let Some(b) = r.body {
                rec_walk_block(&f, b, &rvars, &mut implicit, &mut anchors);
            }
            out.push((r.name.clone(), anchors));
        }
    }
    out
}

fn rec_walk_block(f: &al_syntax::ir::AlFile, bid: al_syntax::ir::BlockId, rvars: &std::collections::HashSet<String>, implicit: &mut Vec<bool>, out: &mut Vec<String>) {
    use al_syntax::ir::BlockItem;
    for item in &f.ir.block(bid).items {
        match item {
            BlockItem::Stmt(s) => rec_walk_stmt(f, *s, rvars, implicit, out),
            BlockItem::Preproc(g) => for b in &g.branches { rec_walk_block(f, *b, rvars, implicit, out); },
        }
    }
}

fn rec_walk_stmt(f: &al_syntax::ir::AlFile, sid: al_syntax::ir::StmtId, rvars: &std::collections::HashSet<String>, implicit: &mut Vec<bool>, out: &mut Vec<String>) {
    use al_syntax::ir::{ExprKind, StmtKind::*};
    macro_rules! e { ($x:expr) => { rec_walk_expr(f, $x, rvars, implicit, out) }; }
    macro_rules! b { ($x:expr) => { rec_walk_block(f, $x, rvars, implicit, out) }; }
    match &f.ir.stmt(sid).kind {
        Assignment { target, value } => { e!(*target); e!(*value); }
        Call(x) => e!(*x),
        If { cond, then_block, else_block } => { e!(*cond); b!(*then_block); if let Some(x)=else_block { b!(*x); } }
        While { cond, body } => { e!(*cond); b!(*body); }
        Repeat { body, until } => { b!(*body); e!(*until); }
        For { var, from, to, body, .. } => { e!(*var); e!(*from); e!(*to); b!(*body); }
        Foreach { var, iterable, body } => { e!(*var); e!(*iterable); b!(*body); }
        With { receiver, body } => {
            e!(*receiver);
            // implicit receiver of the with-body is a record iff the receiver is a record var.
            let is_rec = match &f.ir.expr(*receiver).kind {
                ExprKind::Identifier(x) | ExprKind::QuotedIdentifier(x) => rvars.contains(&x.to_ascii_lowercase()),
                _ => false,
            };
            implicit.push(is_rec);
            b!(*body);
            implicit.pop();
        }
        Case { scrutinee, branches, else_block } => { e!(*scrutinee); for br in branches { for p in &br.patterns { e!(*p); } b!(br.body); } if let Some(x)=else_block { b!(*x); } }
        Try { body, catch_block } => { b!(*body); if let Some(c)=catch_block { b!(*c); } }
        AssertError(body) => b!(*body),
        Exit(x) => { if let Some(x)=x { e!(*x); } }
        Block(x) => b!(*x),
        _ => {}
    }
}

fn rec_walk_expr(f: &al_syntax::ir::AlFile, eid: al_syntax::ir::ExprId, rvars: &std::collections::HashSet<String>, implicit: &mut Vec<bool>, out: &mut Vec<String>) {
    use al_call_hierarchy::engine::l2::record_op::record_op_type;
    use al_syntax::ir::ExprKind::*;
    let e = f.ir.expr(eid);
    if let Call { function, args } = &e.kind {
        let fe = f.ir.expr(*function);
        let is_record_op = match &fe.kind {
            // explicit receiver: X.Method() where X is a record var.
            Member { object, member } => {
                let recv = match &f.ir.expr(*object).kind { Identifier(x) | QuotedIdentifier(x) => Some(x.to_ascii_lowercase()), _ => None };
                recv.map(|r| rvars.contains(&r)).unwrap_or(false) && record_op_type(&member.to_ascii_lowercase()).is_some()
            }
            // implicit receiver: bare Method() with a record implicit receiver in scope.
            Identifier(m) | QuotedIdentifier(m) => {
                implicit.last().copied().unwrap_or(false) && record_op_type(&m.to_ascii_lowercase()).is_some()
            }
            _ => false,
        };
        if is_record_op {
            out.push(format!("{}:{}", e.origin.start.row, e.origin.start.column));
        }
        rec_walk_expr(f, *function, rvars, implicit, out);
        for a in args { rec_walk_expr(f, *a, rvars, implicit, out); }
        return;
    }
    match &e.kind {
        Member { object, .. } => rec_walk_expr(f, *object, rvars, implicit, out),
        Binary { lhs, rhs, .. } => { rec_walk_expr(f, *lhs, rvars, implicit, out); rec_walk_expr(f, *rhs, rvars, implicit, out); }
        Unary { operand, .. } => rec_walk_expr(f, *operand, rvars, implicit, out),
        Parenthesized(x) => rec_walk_expr(f, *x, rvars, implicit, out),
        Index { base, index } => { rec_walk_expr(f, *base, rvars, implicit, out); rec_walk_expr(f, *index, rvars, implicit, out); }
        QualifiedEnum { enum_type, .. } => rec_walk_expr(f, *enum_type, rvars, implicit, out),
        RangeExpr { start, end } => { rec_walk_expr(f, *start, rvars, implicit, out); rec_walk_expr(f, *end, rvars, implicit, out); }
        _ => {}
    }
}

/// Shared corpus parity runner: norm+sort both sides per file, report, return
/// (matching, total).
fn run_parity(
    label: &str,
    legacy: impl Fn(&str) -> Vec<String>,
    ir: impl Fn(&str) -> Vec<String>,
) -> (usize, usize) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/r0-corpus");
    if !root.is_dir() {
        return (0, 0);
    }
    let mut total = 0usize;
    let mut matching = 0usize;
    let mut divergences: Vec<(String, Vec<String>, Vec<String>)> = Vec::new();
    for f in &collect_al_files(&root) {
        let Ok(source) = std::fs::read_to_string(f) else { continue };
        total += 1;
        let mut l: Vec<String> = legacy(&source).iter().map(|s| norm(s)).collect();
        let mut i: Vec<String> = ir(&source).iter().map(|s| norm(s)).collect();
        l.sort();
        i.sort();
        if l == i {
            matching += 1;
        } else {
            let lset: BTreeSet<_> = l.iter().cloned().collect();
            let iset: BTreeSet<_> = i.iter().cloned().collect();
            let rel = f.strip_prefix(&root).unwrap_or(f).display().to_string();
            divergences.push((
                rel,
                lset.difference(&iset).cloned().collect(),
                iset.difference(&lset).cloned().collect(),
            ));
        }
    }
    let pct = if total > 0 { matching as f64 * 100.0 / total as f64 } else { 0.0 };
    eprintln!("\n=== IR dual-run: {label} ===\n{matching}/{total} files match ({pct:.1}%), {} diverge", divergences.len());
    for (file, only_legacy, only_ir) in divergences.iter().take(25) {
        eprintln!("  {file}\n    legacy-only: {only_legacy:?}\n    ir-only:     {only_ir:?}");
    }
    if divergences.len() > 25 {
        eprintln!("  ... {} more", divergences.len() - 25);
    }
    (matching, total)
}

fn collect_al_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in walkdir::WalkDir::new(root).into_iter().flatten() {
        let p = entry.path();
        if p.extension().map(|e| e == "al").unwrap_or(false) {
            out.push(p.to_path_buf());
        }
    }
    out.sort();
    out
}

#[test]
fn routine_inventory_parity() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/r0-corpus");
    if !root.is_dir() {
        eprintln!("r0-corpus absent; skipping");
        return;
    }
    let files = collect_al_files(&root);
    assert!(!files.is_empty(), "no .al fixtures found under {}", root.display());

    let mut total = 0usize;
    let mut matching = 0usize;
    let mut divergences: Vec<(String, Vec<String>, Vec<String>)> = Vec::new();

    for f in &files {
        let Ok(source) = std::fs::read_to_string(f) else { continue };
        total += 1;
        let legacy = legacy_routines(&source);
        let ir = ir_routines(&source);
        if legacy == ir {
            matching += 1;
        } else {
            let only_legacy: Vec<String> = legacy.difference(&ir).cloned().collect();
            let only_ir: Vec<String> = ir.difference(&legacy).cloned().collect();
            let rel = f.strip_prefix(&root).unwrap_or(f).display().to_string();
            divergences.push((rel, only_legacy, only_ir));
        }
    }

    let pct = if total > 0 { matching as f64 * 100.0 / total as f64 } else { 0.0 };
    eprintln!(
        "\n=== IR dual-run: routine inventory ===\n{matching}/{total} files match ({pct:.1}%), {} diverge",
        divergences.len()
    );
    for (file, only_legacy, only_ir) in divergences.iter().take(25) {
        eprintln!("  {file}");
        if !only_legacy.is_empty() {
            eprintln!("    legacy-only: {only_legacy:?}");
        }
        if !only_ir.is_empty() {
            eprintln!("    ir-only:     {only_ir:?}");
        }
    }
    if divergences.len() > 25 {
        eprintln!("  ... {} more", divergences.len() - 25);
    }

    // Hard parity gate: the IR routine inventory must match legacy on every file.
    assert_eq!(matching, total, "{} files diverge — see report above", divergences.len());
}

#[test]
fn call_inventory_parity() {
    use al_call_hierarchy::dual_run_support::legacy_call_methods;
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/r0-corpus");
    if !root.is_dir() {
        return;
    }
    let files = collect_al_files(&root);
    let mut total = 0usize;
    let mut matching = 0usize;
    let mut divergences: Vec<(String, Vec<String>, Vec<String>)> = Vec::new();

    for f in &files {
        let Ok(source) = std::fs::read_to_string(f) else { continue };
        total += 1;
        let mut legacy: Vec<String> = legacy_call_methods(&source).iter().map(|n| norm(n)).collect();
        let mut ir = ir_call_methods(&source);
        legacy.sort();
        ir.sort();
        if legacy == ir {
            matching += 1;
        } else {
            // report multiset difference
            let lset: BTreeSet<_> = legacy.iter().cloned().collect();
            let iset: BTreeSet<_> = ir.iter().cloned().collect();
            let rel = f.strip_prefix(&root).unwrap_or(f).display().to_string();
            divergences.push((
                rel,
                lset.difference(&iset).cloned().collect(),
                iset.difference(&lset).cloned().collect(),
            ));
        }
    }

    let pct = if total > 0 { matching as f64 * 100.0 / total as f64 } else { 0.0 };
    eprintln!(
        "\n=== IR dual-run: call inventory ===\n{matching}/{total} files match ({pct:.1}%), {} diverge",
        divergences.len()
    );
    for (file, only_legacy, only_ir) in divergences.iter().take(25) {
        eprintln!("  {file}\n    legacy-only: {only_legacy:?}\n    ir-only:     {only_ir:?}");
    }
    if divergences.len() > 25 {
        eprintln!("  ... {} more", divergences.len() - 25);
    }
    // Hard parity gate: IR call inventory must match legacy on every file.
    assert_eq!(matching, total, "{} files diverge — see report above", divergences.len());
}

#[test]
fn member_access_parity() {
    use al_call_hierarchy::dual_run_support::legacy_body_member_names;
    let (matching, total) = run_parity("member access", legacy_body_member_names, ir_member_names);
    assert!(total > 0);
    assert_eq!(matching, total, "member-access divergences (see report)");
}

#[test]
fn variable_inventory_parity() {
    use al_call_hierarchy::dual_run_support::legacy_variable_names;
    let (matching, total) = run_parity("variable inventory", legacy_variable_names, ir_variable_names);
    assert!(total > 0);
    assert_eq!(matching, total, "variable divergences (see report)");
}

#[test]
fn statement_kind_parity() {
    use al_call_hierarchy::dual_run_support::legacy_statement_kinds;
    let (matching, total) = run_parity("statement kinds", legacy_statement_kinds, ir_statement_kinds);
    assert!(total > 0);
    assert_eq!(matching, total, "statement-kind divergences (see report)");
}

#[test]
fn temporary_variable_parity() {
    use al_call_hierarchy::dual_run_support::legacy_temporary_var_names;
    let (matching, total) = run_parity("temporary vars", legacy_temporary_var_names, ir_temporary_var_names);
    assert!(total > 0);
    assert_eq!(matching, total, "temporary-variable divergences (see report)");
}

/// First REAL-L2 PFeatures parity: has_branching, diffed against the actual engine
/// L2 walk (not a query proxy). Proves the legacy_l2_features gate.
#[test]
fn has_branching_parity() {
    use al_call_hierarchy::dual_run_support::legacy_l2_features;
    let legacy = |src: &str| -> Vec<String> {
        legacy_l2_features(src)
            .into_iter()
            .filter(|(_, f)| f.has_branching)
            .map(|(n, _)| n)
            .collect()
    };
    let (matching, total) = run_parity("has_branching (real L2)", legacy, ir_branching_routines);
    assert!(total > 0);
    assert_eq!(matching, total, "has_branching divergences (see report)");
}

/// L2 cutover spine — record-op ORDER trace measurement (not yet a hard gate).
/// Per routine, compares the ordered sequence of record-op anchors (IR DFS vs
/// legacy record_operations in op-id order). Surfaces the first order divergence
/// per the reviewers' trace-first methodology.
#[test]
fn record_op_trace_measure() {
    use al_call_hierarchy::dual_run_support::legacy_l2_features;
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/r0-corpus");
    if !root.is_dir() {
        return;
    }
    let op_num = |id: &str| -> u32 { id.trim_start_matches("op").parse().unwrap_or(u32::MAX) };
    let mut total = 0usize;
    let mut matching = 0usize;
    let mut divs: Vec<(String, String, Vec<String>, Vec<String>)> = Vec::new();

    for fpath in collect_al_files(&root) {
        let Ok(src) = std::fs::read_to_string(&fpath) else { continue };
        let legacy = legacy_l2_features(&src);
        let ir = ir_record_op_trace(&src);
        for ((ln, lf), (_in, ianchors)) in legacy.iter().zip(ir.iter()) {
            total += 1;
            let mut ops: Vec<(u32, String)> = lf
                .record_operations
                .iter()
                .map(|r| (op_num(&r.id), format!("{}:{}", r.source_anchor.start_line, r.source_anchor.start_column)))
                .collect();
            ops.sort_by_key(|(n, _)| *n);
            let lanchors: Vec<String> = ops.into_iter().map(|(_, a)| a).collect();
            if &lanchors == ianchors {
                matching += 1;
            } else if divs.len() < 20 {
                let rel = fpath.strip_prefix(&root).unwrap_or(&fpath).display().to_string();
                divs.push((rel, ln.clone(), lanchors, ianchors.clone()));
            }
        }
    }
    let pct = if total > 0 { matching as f64 * 100.0 / total as f64 } else { 0.0 };
    eprintln!("\n=== L2 cutover: record-op order trace ===\n{matching}/{total} routines match ({pct:.1}%)");
    for (file, routine, l, i) in divs.iter().take(12) {
        eprintln!("  {file} :: {routine}\n    legacy: {l:?}\n    ir:     {i:?}");
    }
    assert!(total > 0);
    // Hard gate: record-op classification + visit order match the real engine L2.
    assert_eq!(matching, total, "record-op trace divergences (see report)");
}

#[test]
fn nesting_depth_parity() {
    use al_call_hierarchy::dual_run_support::legacy_l2_features;
    let legacy = |src: &str| -> Vec<String> {
        legacy_l2_features(src)
            .into_iter()
            .map(|(n, f)| format!("{}={}", n, f.nesting_depth))
            .collect()
    };
    let (matching, total) = run_parity("nesting_depth (real L2)", legacy, ir_nesting_pairs);
    assert!(total > 0);
    assert_eq!(matching, total, "nesting_depth divergences (see report)");
}
