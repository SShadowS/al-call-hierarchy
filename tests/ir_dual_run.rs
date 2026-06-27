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

// ---- L2 cutover: ordered op/callsite trace (spine) ----

/// Per-routine ordered traces: `ops` = record-ops + commit/error (the op0..opN
/// sequence), `calls` = call sites (everything else), both in IR DFS visit order.
#[derive(Default)]
struct Trace {
    ops: Vec<String>,
    calls: Vec<String>,
    fields: Vec<String>,
    idents: BTreeSet<String>,
}

/// Per-routine [`Trace`]s in IR DFS visit order (pre-order at each call).
fn ir_op_trace(source: &str) -> Vec<(String, Trace)> {
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
        // `globals` (with rec/xrec convention) is the record-OP receiver set; the
        // FIELD-access set uses record_var_names semantics (rec only for tables).
        let explicit_globals: std::collections::HashSet<String> =
            o.globals.iter().filter(|v| is_rec(v)).map(|v| v.name.to_ascii_lowercase()).collect();
        for r in &o.routines {
            let params_locals = r
                .params
                .iter()
                .filter(|p| p.ty.as_deref().map(|t| t.to_ascii_lowercase().starts_with("record")).unwrap_or(false))
                .map(|p| p.name.to_ascii_lowercase())
                .chain(r.locals.iter().filter(|v| is_rec(v)).map(|v| v.name.to_ascii_lowercase()))
                .collect::<Vec<_>>();
            let mut rvars = globals.clone();
            rvars.extend(params_locals.iter().cloned());
            let mut frvars = explicit_globals.clone();
            if table_method {
                frvars.insert("rec".to_string());
            }
            frvars.extend(params_locals);
            let mut trace = Trace::default();
            // implicit-receiver stack: top = is-current-implicit-receiver-a-record.
            let mut implicit = vec![table_method];
            if let Some(b) = r.body {
                rec_walk_block(&f, b, &rvars, &frvars, &mut implicit, &mut trace);
            }
            out.push((r.name.clone(), trace));
        }
    }
    out
}

fn rec_walk_block(f: &al_syntax::ir::AlFile, bid: al_syntax::ir::BlockId, rvars: &std::collections::HashSet<String>, frvars: &std::collections::HashSet<String>, implicit: &mut Vec<bool>, out: &mut Trace) {
    use al_syntax::ir::BlockItem;
    for item in &f.ir.block(bid).items {
        match item {
            BlockItem::Stmt(s) => rec_walk_stmt(f, *s, rvars, frvars, implicit, out),
            BlockItem::Preproc(g) => for b in &g.branches { rec_walk_block(f, *b, rvars, frvars, implicit, out); },
        }
    }
}

fn rec_walk_stmt(f: &al_syntax::ir::AlFile, sid: al_syntax::ir::StmtId, rvars: &std::collections::HashSet<String>, frvars: &std::collections::HashSet<String>, implicit: &mut Vec<bool>, out: &mut Trace) {
    use al_syntax::ir::{ExprKind, StmtKind::*};
    macro_rules! e { ($x:expr) => { rec_walk_expr(f, $x, rvars, frvars, implicit, out) }; }
    macro_rules! b { ($x:expr) => { rec_walk_block(f, $x, rvars, frvars, implicit, out) }; }
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

fn rec_walk_expr(f: &al_syntax::ir::AlFile, eid: al_syntax::ir::ExprId, rvars: &std::collections::HashSet<String>, frvars: &std::collections::HashSet<String>, implicit: &mut Vec<bool>, out: &mut Trace) {
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
        // Commit() = operation only; Error() = operation AND a call site (legacy
        // pushes both). Both detected as a bare identifier function.
        let fname = match &fe.kind { Identifier(m) | QuotedIdentifier(m) => Some(m.to_ascii_lowercase()), _ => None };
        let is_commit = fname.as_deref() == Some("commit");
        let is_error = fname.as_deref() == Some("error");
        let anchor = format!("{}:{}", e.origin.start.row, e.origin.start.column);
        if is_record_op || is_commit || is_error {
            out.ops.push(anchor.clone());
        }
        // call site: everything that isn't a record-op or a commit (Error IS a call site).
        if !is_record_op && !is_commit {
            out.calls.push(anchor);
        }
        // Recurse the callee. A member-call RECEIVER is a value ref (counted); the
        // bare-call FUNCTION name and the method name are NOT.
        match &fe.kind {
            Member { object, .. } => rec_walk_expr(f, *object, rvars, frvars, implicit, out),
            Identifier(_) | QuotedIdentifier(_) => {} // bare callee name — not a value ref
            _ => rec_walk_expr(f, *function, rvars, frvars, implicit, out),
        }
        for a in args { rec_walk_expr(f, *a, rvars, frvars, implicit, out); }
        return;
    }
    match &e.kind {
        // Value-position member: `X.Field` where X is a record var → field access.
        Member { object, .. } => {
            if let Identifier(x) | QuotedIdentifier(x) = &f.ir.expr(*object).kind {
                if frvars.contains(&x.to_ascii_lowercase()) {
                    out.fields.push(format!("{}:{}", e.origin.start.row, e.origin.start.column));
                }
            }
            rec_walk_expr(f, *object, rvars, frvars, implicit, out);
        }
        // Enum scope (`Rec.Status::Open`): the enum_type member is NOT a field access;
        // recurse its receiver only.
        QualifiedEnum { enum_type, .. } => match &f.ir.expr(*enum_type).kind {
            Member { object, .. } => rec_walk_expr(f, *object, rvars, frvars, implicit, out),
            _ => rec_walk_expr(f, *enum_type, rvars, frvars, implicit, out),
        },
        // value-reference identifier (lc, deduped) — legacy identifier_references
        // counts only plain `identifier` nodes, NOT keyword_identifier.
        Identifier(name) => {
            if e.origin.kind_text == "identifier" {
                out.idents.insert(name.to_ascii_lowercase());
            }
        }
        Binary { lhs, rhs, .. } => { rec_walk_expr(f, *lhs, rvars, frvars, implicit, out); rec_walk_expr(f, *rhs, rvars, frvars, implicit, out); }
        Unary { operand, .. } => rec_walk_expr(f, *operand, rvars, frvars, implicit, out),
        Parenthesized(x) => rec_walk_expr(f, *x, rvars, frvars, implicit, out),
        Index { base, index } => { rec_walk_expr(f, *base, rvars, frvars, implicit, out); rec_walk_expr(f, *index, rvars, frvars, implicit, out); }
        RangeExpr { start, end } => { rec_walk_expr(f, *start, rvars, frvars, implicit, out); rec_walk_expr(f, *end, rvars, frvars, implicit, out); }
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
    // id is `<routine_id>/opN`; extract the trailing op number (hex routine_id has no "op").
    let op_num = |id: &str| -> u32 { id.rsplit("op").next().and_then(|s| s.parse().ok()).unwrap_or(u32::MAX) };
    let mut total = 0usize;
    let mut matching = 0usize;
    let mut divs: Vec<(String, String, Vec<String>, Vec<String>)> = Vec::new();

    for fpath in collect_al_files(&root) {
        let Ok(src) = std::fs::read_to_string(&fpath) else { continue };
        let legacy = legacy_l2_features(&src);
        let ir = ir_op_trace(&src);
        for ((ln, lf), (_in, itrace)) in legacy.iter().zip(ir.iter()) {
            total += 1;
            let ianchors = &itrace.ops;
            // operation_sites is the COMPLETE unified op list: every record_operation
            // mirrors into operation_sites (kind "record-op"/"lock") with the same
            // op_id, plus genuine commit/error-call ops. So this alone is the op0..opN
            // sequence.
            let mut ops: Vec<(u32, String)> = lf
                .operation_sites
                .iter()
                .map(|o| (op_num(&o.id), format!("{}:{}", o.source_anchor.start_line, o.source_anchor.start_column)))
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
    eprintln!("\n=== L2 cutover: op-counter trace (record-ops + commit/error) ===\n{matching}/{total} routines match ({pct:.1}%)");
    for (file, routine, l, i) in divs.iter().take(12) {
        eprintln!("  {file} :: {routine}\n    legacy: {l:?}\n    ir:     {i:?}");
    }
    assert!(total > 0);
    // Hard gate: record-op classification + visit order match the real engine L2.
    assert_eq!(matching, total, "record-op trace divergences (see report)");
}

/// L2 cutover — call-site ORDER trace measurement. Per routine, compares the
/// ordered call-site anchors (IR DFS vs legacy call_sites in cs-id order).
#[test]
fn callsite_trace_measure() {
    use al_call_hierarchy::dual_run_support::legacy_l2_features;
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/r0-corpus");
    if !root.is_dir() {
        return;
    }
    let cs_num = |id: &str| -> u32 { id.rsplit("cs").next().and_then(|s| s.parse().ok()).unwrap_or(u32::MAX) };
    let mut total = 0usize;
    let mut matching = 0usize;
    let mut divs: Vec<(String, String, Vec<String>, Vec<String>)> = Vec::new();

    for fpath in collect_al_files(&root) {
        let Ok(src) = std::fs::read_to_string(&fpath) else { continue };
        let legacy = legacy_l2_features(&src);
        let ir = ir_op_trace(&src);
        for ((ln, lf), (_in, itrace)) in legacy.iter().zip(ir.iter()) {
            total += 1;
            let mut cs: Vec<(u32, String)> = lf
                .call_sites
                .iter()
                .map(|c| (cs_num(&c.id), format!("{}:{}", c.source_anchor.start_line, c.source_anchor.start_column)))
                .collect();
            cs.sort_by_key(|(n, _)| *n);
            let lanchors: Vec<String> = cs.into_iter().map(|(_, a)| a).collect();
            if lanchors == itrace.calls {
                matching += 1;
            } else if divs.len() < 20 {
                let rel = fpath.strip_prefix(&root).unwrap_or(&fpath).display().to_string();
                divs.push((rel, ln.clone(), lanchors, itrace.calls.clone()));
            }
        }
    }
    let pct = if total > 0 { matching as f64 * 100.0 / total as f64 } else { 0.0 };
    eprintln!("\n=== L2 cutover: call-site order trace ===\n{matching}/{total} routines match ({pct:.1}%)");
    for (file, routine, l, i) in divs.iter().take(12) {
        eprintln!("  {file} :: {routine}\n    legacy: {l:?}\n    ir:     {i:?}");
    }
    assert!(total > 0);
    assert_eq!(matching, total, "call-site trace divergences (see report)");
}

/// L2 cutover — identifier_references (deduped/sorted value-ref identifiers).
#[test]
fn identifier_refs_measure() {
    use al_call_hierarchy::dual_run_support::legacy_l2_features;
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/r0-corpus");
    if !root.is_dir() {
        return;
    }
    let mut total = 0usize;
    let mut matching = 0usize;
    let mut divs: Vec<(String, String, Vec<String>, Vec<String>)> = Vec::new();
    for fpath in collect_al_files(&root) {
        let Ok(src) = std::fs::read_to_string(&fpath) else { continue };
        let legacy = legacy_l2_features(&src);
        let ir = ir_op_trace(&src);
        for ((ln, lf), (_in, itrace)) in legacy.iter().zip(ir.iter()) {
            total += 1;
            let l: Vec<String> = lf.identifier_references.clone();
            let i: Vec<String> = itrace.idents.iter().cloned().collect();
            if l == i {
                matching += 1;
            } else if divs.len() < 20 {
                let lset: BTreeSet<_> = l.iter().cloned().collect();
                let iset: BTreeSet<_> = i.iter().cloned().collect();
                let rel = fpath.strip_prefix(&root).unwrap_or(&fpath).display().to_string();
                divs.push((rel, ln.clone(), lset.difference(&iset).cloned().collect(), iset.difference(&lset).cloned().collect()));
            }
        }
    }
    let pct = if total > 0 { matching as f64 * 100.0 / total as f64 } else { 0.0 };
    eprintln!("\n=== L2 cutover: identifier_references ===\n{matching}/{total} routines match ({pct:.1}%)");
    for (file, routine, l, i) in divs.iter().take(12) {
        eprintln!("  {file} :: {routine}\n    legacy-only: {l:?}\n    ir-only:     {i:?}");
    }
    assert!(total > 0);
}

/// L2 cutover — field-access ORDER trace. `X.Field` in expression position where X
/// is a record var (legacy field_accesses, visit order).
#[test]
fn field_access_trace_measure() {
    use al_call_hierarchy::dual_run_support::legacy_l2_features;
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/r0-corpus");
    if !root.is_dir() {
        return;
    }
    let mut total = 0usize;
    let mut matching = 0usize;
    let mut divs: Vec<(String, String, Vec<String>, Vec<String>)> = Vec::new();

    for fpath in collect_al_files(&root) {
        let Ok(src) = std::fs::read_to_string(&fpath) else { continue };
        let legacy = legacy_l2_features(&src);
        let ir = ir_op_trace(&src);
        for ((ln, lf), (_in, itrace)) in legacy.iter().zip(ir.iter()) {
            total += 1;
            let lanchors: Vec<String> = lf
                .field_accesses
                .iter()
                .map(|fa| format!("{}:{}", fa.source_anchor.start_line, fa.source_anchor.start_column))
                .collect();
            if lanchors == itrace.fields {
                matching += 1;
            } else if divs.len() < 20 {
                let rel = fpath.strip_prefix(&root).unwrap_or(&fpath).display().to_string();
                divs.push((rel, ln.clone(), lanchors, itrace.fields.clone()));
            }
        }
    }
    let pct = if total > 0 { matching as f64 * 100.0 / total as f64 } else { 0.0 };
    eprintln!("\n=== L2 cutover: field-access trace ===\n{matching}/{total} routines match ({pct:.1}%)");
    for (file, routine, l, i) in divs.iter().take(12) {
        eprintln!("  {file} :: {routine}\n    legacy: {l:?}\n    ir:     {i:?}");
    }
    assert!(total > 0);
    assert_eq!(matching, total, "field-access trace divergences (see report)");
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
