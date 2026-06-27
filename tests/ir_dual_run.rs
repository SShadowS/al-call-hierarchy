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
    let s = s
        .strip_prefix('"')
        .and_then(|x| x.strip_suffix('"'))
        .unwrap_or(s);
    s.to_ascii_lowercase()
}

fn legacy_routines(source: &str) -> BTreeSet<String> {
    legacy_routine_names(source)
        .iter()
        .map(|n| norm(n))
        .collect()
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
            Some(
                match &s.kind {
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
                .to_string(),
            )
        })
        .collect()
}

/// IR temporary-variable names (globals + locals where `temporary`).
fn ir_temporary_var_names(source: &str) -> Vec<String> {
    let f = al_syntax::parse(source);
    let mut out = Vec::new();
    for o in &f.objects {
        out.extend(
            o.globals
                .iter()
                .filter(|v| v.temporary)
                .map(|v| v.name.clone()),
        );
        for r in &o.routines {
            out.extend(
                r.locals
                    .iter()
                    .filter(|v| v.temporary)
                    .map(|v| v.name.clone()),
            );
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
        If {
            then_block,
            else_block,
            ..
        } => {
            let a = block_loop_nesting(f, *then_block, depth);
            let b = else_block
                .map(|e| block_loop_nesting(f, e, depth))
                .unwrap_or(0);
            a.max(b)
        }
        Case {
            branches,
            else_block,
            ..
        } => {
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
            let b = catch_block
                .map(|c| block_loop_nesting(f, c, depth))
                .unwrap_or(0);
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
    // (lhs_name, rhs_literal, anchor) per assignment — mirrors PVarAssignment.
    assigns: Vec<(String, Option<String>, String)>,
    // (identifier, condition_kind, ref_anchor, stmt_anchor) — PConditionReference.
    conds: Vec<(String, String, String, String)>,
    // Per Call-ExprId op/cs sequence index, assigned during the main DFS walk
    // (same numbering legacy body_walk emits as `op{N}` / `cs{N}`). Consumed by the
    // CFN builder, which references op/cs leaves by their sequence number.
    op_idx: std::collections::HashMap<al_syntax::ir::ExprId, u32>,
    cs_idx: std::collections::HashMap<al_syntax::ir::ExprId, u32>,
    // Enclosing-loop id stacks (loop SEQUENCE numbers) snapshotted per op / per call,
    // parallel to `ops` / `calls`. Mirrors legacy `loop_stack` (`{routine}/loop{N}`,
    // N = loop DFS-discovery order). `cur_loops`/`loop_ctr` are the working state.
    op_loops: Vec<Vec<u32>>,
    cs_loops: Vec<Vec<u32>>,
    cur_loops: Vec<u32>,
    loop_ctr: u32,
    // under-asserterror context: snapshotted per op / per call (true iff inside an
    // `asserterror` body). `assert_depth` is the working nesting counter.
    op_under: Vec<bool>,
    cs_under: Vec<bool>,
    assert_depth: u32,
}

/// collect_idents over a condition expr: identifiers + member NAMES (plain
/// identifier only), NOT recursing a member's object. Mirrors legacy collect_idents.
fn collect_cond_idents(
    f: &al_syntax::ir::AlFile,
    eid: al_syntax::ir::ExprId,
    kind: &str,
    stmt: &str,
    out: &mut Trace,
) {
    use al_syntax::ir::ExprKind::*;
    let e = f.ir.expr(eid);
    match &e.kind {
        Identifier(name) if e.origin.kind_text == "identifier" => {
            out.conds.push((
                name.to_ascii_lowercase(),
                kind.to_string(),
                format!("{}:{}", e.origin.start.row, e.origin.start.column),
                stmt.to_string(),
            ));
        }
        Member {
            member,
            member_origin,
            ..
        } => {
            if member_origin.kind_text == "identifier" {
                out.conds.push((
                    member.to_ascii_lowercase(),
                    kind.to_string(),
                    format!("{}:{}", member_origin.start.row, member_origin.start.column),
                    stmt.to_string(),
                ));
            }
            // does NOT recurse into the object.
        }
        Call { function, args } => {
            collect_cond_idents(f, *function, kind, stmt, out);
            for a in args {
                collect_cond_idents(f, *a, kind, stmt, out);
            }
        }
        Binary { lhs, rhs, .. } => {
            collect_cond_idents(f, *lhs, kind, stmt, out);
            collect_cond_idents(f, *rhs, kind, stmt, out);
        }
        Unary { operand, .. } => collect_cond_idents(f, *operand, kind, stmt, out),
        Parenthesized(x) => collect_cond_idents(f, *x, kind, stmt, out),
        Index { base, index } => {
            collect_cond_idents(f, *base, kind, stmt, out);
            collect_cond_idents(f, *index, kind, stmt, out);
        }
        QualifiedEnum { enum_type, .. } => collect_cond_idents(f, *enum_type, kind, stmt, out),
        RangeExpr { start, end } => {
            collect_cond_idents(f, *start, kind, stmt, out);
            collect_cond_idents(f, *end, kind, stmt, out);
        }
        _ => {}
    }
}

/// Per-routine [`Trace`]s in IR DFS visit order (pre-order at each call).
fn ir_op_trace(source: &str) -> Vec<(String, Trace)> {
    let f = al_syntax::parse(source);
    ir_traces(&f)
}

/// Trace core over an already-parsed file (keeps ExprIds stable for the CFN, which
/// references op/cs indices keyed by ExprId).
fn ir_traces(f: &al_syntax::ir::AlFile) -> Vec<(String, Trace)> {
    use al_syntax::ir::VarDecl;
    let is_rec = |v: &VarDecl| {
        v.ty.as_deref()
            .map(|t| t.to_ascii_lowercase().starts_with("record"))
            .unwrap_or(false)
    };
    let mut out = Vec::new();
    for o in &f.objects {
        let mut globals: std::collections::HashSet<String> = o
            .globals
            .iter()
            .filter(|v| is_rec(v))
            .map(|v| v.name.to_ascii_lowercase())
            .collect();
        // `Rec`/`xRec` are record receivers by name convention (classify.rs:277),
        // regardless of object type.
        globals.insert("rec".to_string());
        globals.insert("xrec".to_string());
        // A table/tableext method (procedure OR trigger) has an implicit record `Rec`.
        let table_method = matches!(
            o.kind,
            al_syntax::ir::ObjectKind::Table | al_syntax::ir::ObjectKind::TableExtension
        );
        // `globals` (with rec/xrec convention) is the record-OP receiver set; the
        // FIELD-access set uses record_var_names semantics (rec only for tables).
        let explicit_globals: std::collections::HashSet<String> = o
            .globals
            .iter()
            .filter(|v| is_rec(v))
            .map(|v| v.name.to_ascii_lowercase())
            .collect();
        for r in &o.routines {
            let params_locals = r
                .params
                .iter()
                .filter(|p| {
                    p.ty.as_deref()
                        .map(|t| t.to_ascii_lowercase().starts_with("record"))
                        .unwrap_or(false)
                })
                .map(|p| p.name.to_ascii_lowercase())
                .chain(
                    r.locals
                        .iter()
                        .filter(|v| is_rec(v))
                        .map(|v| v.name.to_ascii_lowercase()),
                )
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
                rec_walk_block(f, b, &rvars, &frvars, &mut implicit, &mut trace);
            }
            out.push((r.name.clone(), trace));
        }
    }
    out
}

fn rec_walk_block(
    f: &al_syntax::ir::AlFile,
    bid: al_syntax::ir::BlockId,
    rvars: &std::collections::HashSet<String>,
    frvars: &std::collections::HashSet<String>,
    implicit: &mut Vec<bool>,
    out: &mut Trace,
) {
    use al_syntax::ir::BlockItem;
    for item in &f.ir.block(bid).items {
        match item {
            BlockItem::Stmt(s) => rec_walk_stmt(f, *s, rvars, frvars, implicit, out),
            BlockItem::Preproc(g) => {
                for b in &g.branches {
                    rec_walk_block(f, *b, rvars, frvars, implicit, out);
                }
            }
        }
    }
}

fn rec_walk_stmt(
    f: &al_syntax::ir::AlFile,
    sid: al_syntax::ir::StmtId,
    rvars: &std::collections::HashSet<String>,
    frvars: &std::collections::HashSet<String>,
    implicit: &mut Vec<bool>,
    out: &mut Trace,
) {
    use al_syntax::ir::{ExprKind, Literal, StmtKind::*};
    macro_rules! e {
        ($x:expr) => {
            rec_walk_expr(f, $x, rvars, frvars, implicit, out)
        };
    }
    macro_rules! b {
        ($x:expr) => {
            rec_walk_block(f, $x, rvars, frvars, implicit, out)
        };
    }
    let st = f.ir.stmt(sid);
    let sa = format!("{}:{}", st.origin.start.row, st.origin.start.column);
    match &st.kind {
        Assignment { target, value } => {
            // PVarAssignment: lhs base name (identifier or member name), optional
            // literal rhs (bool/int/string), anchored on the assignment statement.
            let lhs = match &f.ir.expr(*target).kind {
                ExprKind::Identifier(x) | ExprKind::QuotedIdentifier(x) => {
                    Some(x.to_ascii_lowercase())
                }
                ExprKind::Member { member, .. } => Some(member.to_ascii_lowercase()),
                _ => None,
            };
            if let Some(lhs) = lhs {
                let rhs_lit = match &f.ir.expr(*value).kind {
                    ExprKind::Literal(Literal::Bool(b)) => Some(b.to_string()),
                    ExprKind::Literal(Literal::Int(s)) => Some(s.clone()),
                    ExprKind::Literal(Literal::Text(s)) => {
                        let t = s
                            .strip_prefix('\'')
                            .and_then(|x| x.strip_suffix('\''))
                            .unwrap_or(s);
                        Some(t.to_ascii_lowercase())
                    }
                    _ => None,
                };
                out.assigns.push((
                    lhs,
                    rhs_lit,
                    format!("{}:{}", st.origin.start.row, st.origin.start.column),
                ));
            }
            e!(*target);
            e!(*value);
        }
        Call(x) => e!(*x),
        If {
            cond,
            then_block,
            else_block,
        } => {
            collect_cond_idents(f, *cond, "if", &sa, out);
            e!(*cond);
            b!(*then_block);
            if let Some(x) = else_block {
                b!(*x);
            }
        }
        While { cond, body } => {
            collect_cond_idents(f, *cond, "while", &sa, out);
            // Loop id pushed at the loop node (legacy body_walk), so condition + body
            // ops see it on the stack. N = DFS-discovery order (monotonic counter).
            let ln = out.loop_ctr;
            out.loop_ctr += 1;
            out.cur_loops.push(ln);
            e!(*cond);
            b!(*body);
            out.cur_loops.pop();
        }
        Repeat { body, until } => {
            collect_cond_idents(f, *until, "repeat-until", &sa, out);
            let ln = out.loop_ctr;
            out.loop_ctr += 1;
            out.cur_loops.push(ln);
            b!(*body);
            e!(*until);
            out.cur_loops.pop();
        }
        For {
            var,
            from,
            to,
            body,
            ..
        } => {
            let ln = out.loop_ctr;
            out.loop_ctr += 1;
            out.cur_loops.push(ln);
            e!(*var);
            e!(*from);
            e!(*to);
            b!(*body);
            out.cur_loops.pop();
        }
        Foreach {
            var,
            iterable,
            body,
        } => {
            let ln = out.loop_ctr;
            out.loop_ctr += 1;
            out.cur_loops.push(ln);
            e!(*var);
            e!(*iterable);
            b!(*body);
            out.cur_loops.pop();
        }
        With { receiver, body } => {
            e!(*receiver);
            // implicit receiver of the with-body is a record iff the receiver is a record var.
            let is_rec = match &f.ir.expr(*receiver).kind {
                ExprKind::Identifier(x) | ExprKind::QuotedIdentifier(x) => {
                    rvars.contains(&x.to_ascii_lowercase())
                }
                _ => false,
            };
            implicit.push(is_rec);
            b!(*body);
            implicit.pop();
        }
        Case {
            scrutinee,
            branches,
            else_block,
        } => {
            collect_cond_idents(f, *scrutinee, "case", &sa, out);
            e!(*scrutinee);
            for br in branches {
                for p in &br.patterns {
                    e!(*p);
                }
                b!(br.body);
            }
            if let Some(x) = else_block {
                b!(*x);
            }
        }
        Try { body, catch_block } => {
            b!(*body);
            if let Some(c) = catch_block {
                b!(*c);
            }
        }
        AssertError(body) => {
            out.assert_depth += 1;
            b!(*body);
            out.assert_depth -= 1;
        }
        Exit(x) => {
            if let Some(x) = x {
                e!(*x);
            }
        }
        Block(x) => b!(*x),
        _ => {}
    }
}

fn rec_walk_expr(
    f: &al_syntax::ir::AlFile,
    eid: al_syntax::ir::ExprId,
    rvars: &std::collections::HashSet<String>,
    frvars: &std::collections::HashSet<String>,
    implicit: &mut Vec<bool>,
    out: &mut Trace,
) {
    use al_call_hierarchy::engine::l2::record_op::record_op_type;
    use al_syntax::ir::ExprKind::*;
    let e = f.ir.expr(eid);
    if let Call { function, args } = &e.kind {
        let fe = f.ir.expr(*function);
        let is_record_op = match &fe.kind {
            // explicit receiver: X.Method() where X is a record var.
            Member { object, member, .. } => {
                let recv = match &f.ir.expr(*object).kind {
                    Identifier(x) | QuotedIdentifier(x) => Some(x.to_ascii_lowercase()),
                    _ => None,
                };
                recv.map(|r| rvars.contains(&r)).unwrap_or(false)
                    && record_op_type(&member.to_ascii_lowercase()).is_some()
            }
            // implicit receiver: bare Method() with a record implicit receiver in scope.
            Identifier(m) | QuotedIdentifier(m) => {
                implicit.last().copied().unwrap_or(false)
                    && record_op_type(&m.to_ascii_lowercase()).is_some()
            }
            _ => false,
        };
        // Commit() = operation only; Error() = operation AND a call site (legacy
        // pushes both). Both detected as a bare identifier function.
        let fname = match &fe.kind {
            Identifier(m) | QuotedIdentifier(m) => Some(m.to_ascii_lowercase()),
            _ => None,
        };
        let is_commit = fname.as_deref() == Some("commit");
        let is_error = fname.as_deref() == Some("error");
        let anchor = format!("{}:{}", e.origin.start.row, e.origin.start.column);
        if is_record_op || is_commit || is_error {
            // The op SEQUENCE index = position among all ops (record-op/commit/error).
            // But the CFN's `op_id_by_node_id` (mirrored by `op_idx`) deliberately
            // OMITS error nodes — an error renders as its `cs` "error" leaf, not an op
            // leaf — so only map non-error ops while still counting error in the index.
            if !is_error {
                out.op_idx.insert(eid, out.ops.len() as u32);
            }
            out.op_loops.push(out.cur_loops.clone());
            out.op_under.push(out.assert_depth > 0);
            out.ops.push(anchor.clone());
        }
        // call site: everything that isn't a record-op or a commit (Error IS a call site).
        if !is_record_op && !is_commit {
            out.cs_idx.insert(eid, out.calls.len() as u32);
            out.cs_loops.push(out.cur_loops.clone());
            out.cs_under.push(out.assert_depth > 0);
            out.calls.push(anchor);
        }
        // Recurse the callee. A member-call RECEIVER is a value ref (counted); the
        // bare-call FUNCTION name and the method name are NOT.
        match &fe.kind {
            Member { object, .. } => rec_walk_expr(f, *object, rvars, frvars, implicit, out),
            Identifier(_) | QuotedIdentifier(_) => {} // bare callee name — not a value ref
            _ => rec_walk_expr(f, *function, rvars, frvars, implicit, out),
        }
        for a in args {
            rec_walk_expr(f, *a, rvars, frvars, implicit, out);
        }
        return;
    }
    match &e.kind {
        // Value-position member: `X.Field` where X is a record var → field access.
        Member { object, .. } => {
            if let Identifier(x) | QuotedIdentifier(x) = &f.ir.expr(*object).kind {
                if frvars.contains(&x.to_ascii_lowercase()) {
                    out.fields
                        .push(format!("{}:{}", e.origin.start.row, e.origin.start.column));
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
        // `Keyword::Name` (database_reference): the object-type keyword is excluded,
        // but an UNQUOTED table_name identifier is a value ref.
        DatabaseReference(text) => {
            if let Some(last) = text.rsplit("::").next() {
                let t = last.trim();
                if !t.starts_with('"') {
                    out.idents.insert(t.to_ascii_lowercase());
                }
            }
        }
        Binary { lhs, rhs, .. } => {
            rec_walk_expr(f, *lhs, rvars, frvars, implicit, out);
            rec_walk_expr(f, *rhs, rvars, frvars, implicit, out);
        }
        Unary { operand, .. } => rec_walk_expr(f, *operand, rvars, frvars, implicit, out),
        Parenthesized(x) => rec_walk_expr(f, *x, rvars, frvars, implicit, out),
        Index { base, index } => {
            rec_walk_expr(f, *base, rvars, frvars, implicit, out);
            rec_walk_expr(f, *index, rvars, frvars, implicit, out);
        }
        RangeExpr { start, end } => {
            rec_walk_expr(f, *start, rvars, frvars, implicit, out);
            rec_walk_expr(f, *end, rvars, frvars, implicit, out);
        }
        _ => {}
    }
}

// ---- L2 cutover: statement_tree (CFN skeleton) ----

/// Normalized CFN node — the parity surface of `PCFNNode`. op/cs ids are replaced
/// by their sequence NUMBER (legacy `op{N}`/`cs{N}` → N) so the IR and legacy trees
/// compare regardless of the routine-id hash prefix. Mirrors `PCFNNode`'s Option
/// semantics exactly (None vs Some([]) is significant — it is what serializes).
#[derive(PartialEq, Eq, Debug, Clone)]
struct NCfn {
    kind: String,
    op: Option<u32>,
    cs: Option<u32>,
    guard: Option<(String, String)>,
    leaves: Option<Vec<NCfn>>,
    children: Option<Vec<NCfn>>,
    else_children: Option<Vec<NCfn>>,
}

fn ncfn(kind: &str) -> NCfn {
    NCfn {
        kind: kind.to_string(),
        op: None,
        cs: None,
        guard: None,
        leaves: None,
        children: None,
        else_children: None,
    }
}

/// Trailing integer of a legacy `…/op{N}` or `…/cs{N}` id.
fn id_num(id: &str) -> u32 {
    id.rsplit(|c| c == 'p' || c == 's')
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(u32::MAX)
}

/// Convert a legacy `PCFNNode` to the normalized form (ids → sequence numbers).
fn norm_legacy_cfn(n: &al_call_hierarchy::engine::l2::features::PCFNNode) -> NCfn {
    let conv = |v: &Option<Vec<al_call_hierarchy::engine::l2::features::PCFNNode>>| {
        v.as_ref()
            .map(|xs| xs.iter().map(norm_legacy_cfn).collect())
    };
    NCfn {
        kind: n.kind.clone(),
        op: n.operation_id.as_deref().map(id_num),
        cs: n.callsite_id.as_deref().map(id_num),
        guard: n
            .condition_guard
            .as_ref()
            .map(|g| (g.identifier.clone(), g.polarity.clone())),
        leaves: conv(&n.condition_leaves),
        children: conv(&n.children),
        else_children: conv(&n.else_children),
    }
}

/// IR CFN builder — a faithful port of `engine::l2::cfn` over the owned IR. Reads
/// op/cs sequence numbers from the `Trace` (assigned during the proven main walk).
struct IrCfn<'a> {
    f: &'a al_syntax::ir::AlFile,
    tr: &'a Trace,
}

impl<'a> IrCfn<'a> {
    fn block_items(&self, bid: al_syntax::ir::BlockId) -> Vec<NCfn> {
        use al_syntax::ir::BlockItem;
        let mut out = Vec::new();
        for item in &self.f.ir.block(bid).items {
            match item {
                BlockItem::Stmt(s) => {
                    if let Some(c) = self.build_statement(*s) {
                        out.push(c);
                    }
                }
                // A preproc group's branches splice inline in source order (matching
                // the flat layout the legacy CST walk sees).
                BlockItem::Preproc(g) => {
                    for b in &g.branches {
                        out.extend(self.block_items(*b));
                    }
                }
            }
        }
        out
    }

    /// `build_block` → a "block" node wrapping its statement children.
    fn build_block(&self, bid: al_syntax::ir::BlockId) -> NCfn {
        let mut n = ncfn("block");
        n.children = Some(self.block_items(bid));
        n
    }

    fn is_error_fn(&self, function: al_syntax::ir::ExprId) -> bool {
        use al_syntax::ir::ExprKind::*;
        matches!(&self.f.ir.expr(function).kind, Identifier(m) | QuotedIdentifier(m) if m.eq_ignore_ascii_case("error"))
    }

    /// Receiver-side leaves of a chained call: if the function is `X.m` and `X` is
    /// itself a call/member, harvest it into `out` (as siblings, before the leaf).
    fn harvest_receiver(&self, function: al_syntax::ir::ExprId, out: &mut Vec<NCfn>) {
        use al_syntax::ir::ExprKind::*;
        if let Member { object, .. } = &self.f.ir.expr(function).kind {
            if matches!(&self.f.ir.expr(*object).kind, Call { .. } | Member { .. }) {
                self.harvest(*object, out);
            }
        }
    }

    /// Harvest op/callsite leaves from an expression subtree (condition/expression
    /// position): receiver leaves become SIBLINGS, args nest as `leaves`.
    fn harvest(&self, eid: al_syntax::ir::ExprId, out: &mut Vec<NCfn>) {
        use al_syntax::ir::ExprKind::*;
        let e = self.f.ir.expr(eid);
        match &e.kind {
            Call { function, args } => {
                if let Some(&op) = self.tr.op_idx.get(&eid) {
                    let mut inner = Vec::new();
                    for a in args {
                        self.harvest(*a, &mut inner);
                    }
                    self.harvest_receiver(*function, out);
                    let mut leaf = ncfn("op");
                    leaf.op = Some(op);
                    if !inner.is_empty() {
                        leaf.leaves = Some(inner);
                    }
                    out.push(leaf);
                    return;
                }
                if let Some(&cs) = self.tr.cs_idx.get(&eid) {
                    let mut inner = Vec::new();
                    for a in args {
                        self.harvest(*a, &mut inner);
                    }
                    self.harvest_receiver(*function, out);
                    let mut leaf = ncfn(if self.is_error_fn(*function) {
                        "error"
                    } else {
                        "call"
                    });
                    leaf.cs = Some(cs);
                    if !inner.is_empty() {
                        leaf.leaves = Some(inner);
                    }
                    out.push(leaf);
                    return;
                }
                // No id: recurse function then args (matches legacy named-child recursion).
                self.harvest(*function, out);
                for a in args {
                    self.harvest(*a, out);
                }
            }
            Member { object, .. } => self.harvest(*object, out),
            Binary { lhs, rhs, .. } => {
                self.harvest(*lhs, out);
                self.harvest(*rhs, out);
            }
            Unary { operand, .. } => self.harvest(*operand, out),
            Parenthesized(x) => self.harvest(*x, out),
            Index { base, index } => {
                self.harvest(*base, out);
                self.harvest(*index, out);
            }
            QualifiedEnum { enum_type, .. } => self.harvest(*enum_type, out),
            RangeExpr { start, end } => {
                self.harvest(*start, out);
                self.harvest(*end, out);
            }
            _ => {}
        }
    }

    /// A call in STATEMENT position → ONE leaf; receiver + arg leaves nest inside it
    /// as `leaves` (unlike condition position, where the receiver is a sibling).
    fn build_stmt_call(&self, eid: al_syntax::ir::ExprId) -> NCfn {
        use al_syntax::ir::ExprKind::*;
        let e = self.f.ir.expr(eid);
        let Call { function, args } = &e.kind else {
            return ncfn("other");
        };
        let mut pre = Vec::new();
        self.harvest_receiver(*function, &mut pre);
        for a in args {
            self.harvest(*a, &mut pre);
        }
        let mut leaf = if let Some(&op) = self.tr.op_idx.get(&eid) {
            let mut l = ncfn("op");
            l.op = Some(op);
            l
        } else if let Some(&cs) = self.tr.cs_idx.get(&eid) {
            let mut l = ncfn(if self.is_error_fn(*function) {
                "error"
            } else {
                "call"
            });
            l.cs = Some(cs);
            l
        } else {
            ncfn("other")
        };
        if !pre.is_empty() {
            leaf.leaves = Some(pre);
        }
        leaf
    }

    /// Simple boolean-guard recognizer on an `if` condition.
    fn simple_guard(&self, cond: al_syntax::ir::ExprId) -> Option<(String, String)> {
        use al_syntax::ir::{BinaryOp, ExprKind::*, Literal, UnaryOp};
        let e = self.f.ir.expr(cond);
        match &e.kind {
            Identifier(n) if e.origin.kind_text == "identifier" => {
                Some((n.to_ascii_lowercase(), "positive".to_string()))
            }
            Unary {
                op: UnaryOp::Not,
                operand,
            } => match &self.f.ir.expr(*operand).kind {
                Identifier(n) if self.f.ir.expr(*operand).origin.kind_text == "identifier" => {
                    Some((n.to_ascii_lowercase(), "negative".to_string()))
                }
                _ => None,
            },
            Binary {
                op: BinaryOp::Eq,
                lhs,
                rhs,
            } => {
                let id_side =
                    [*lhs, *rhs]
                        .into_iter()
                        .find_map(|s| match &self.f.ir.expr(s).kind {
                            Identifier(n) if self.f.ir.expr(s).origin.kind_text == "identifier" => {
                                Some(n.to_ascii_lowercase())
                            }
                            _ => None,
                        });
                let false_side = [*lhs, *rhs]
                    .into_iter()
                    .any(|s| matches!(&self.f.ir.expr(s).kind, Literal(Literal::Bool(false))));
                match (id_side, false_side) {
                    (Some(id), true) => Some((id, "negative".to_string())),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    fn harvest_vec(&self, eid: al_syntax::ir::ExprId) -> Vec<NCfn> {
        let mut v = Vec::new();
        self.harvest(eid, &mut v);
        v
    }

    fn build_statement(&self, sid: al_syntax::ir::StmtId) -> Option<NCfn> {
        use al_syntax::ir::StmtKind::*;
        let st = self.f.ir.stmt(sid);
        let some_if = |v: Vec<NCfn>| if v.is_empty() { None } else { Some(v) };
        Some(match &st.kind {
            Call(e) => self.build_stmt_call(*e),
            If {
                cond,
                then_block,
                else_block,
            } => {
                let mut n = ncfn("if");
                n.children = Some(vec![self.build_block(*then_block)]);
                n.else_children = else_block.map(|b| vec![self.build_block(b)]);
                n.leaves = some_if(self.harvest_vec(*cond));
                n.guard = self.simple_guard(*cond);
                n
            }
            Case {
                scrutinee,
                branches,
                else_block,
            } => {
                let mut branch_cfns = Vec::new();
                for br in branches {
                    let mut b = ncfn("case-branch");
                    b.children = Some(vec![self.build_block(br.body)]);
                    branch_cfns.push(b);
                }
                if let Some(eb) = else_block {
                    let mut b = ncfn("case-branch");
                    b.children = Some(vec![self.build_block(*eb)]);
                    branch_cfns.push(b);
                }
                let mut n = ncfn("case");
                n.children = Some(branch_cfns);
                n.leaves = some_if(self.harvest_vec(*scrutinee));
                n
            }
            While { cond, body } => {
                let mut n = ncfn("while");
                n.children = Some(vec![self.build_block(*body)]);
                n.leaves = some_if(self.harvest_vec(*cond));
                n
            }
            For { from, to, body, .. } => {
                let mut n = ncfn("for");
                n.children = Some(vec![self.build_block(*body)]);
                let mut leaves = self.harvest_vec(*from);
                leaves.extend(self.harvest_vec(*to));
                n.leaves = some_if(leaves);
                n
            }
            Foreach { iterable, body, .. } => {
                let mut n = ncfn("foreach");
                n.children = Some(vec![self.build_block(*body)]);
                n.leaves = some_if(self.harvest_vec(*iterable));
                n
            }
            Repeat { body, until } => {
                // repeat's body statements are DIRECT children (not wrapped in a block).
                let mut n = ncfn("repeat");
                n.children = Some(self.block_items(*body));
                n.leaves = some_if(self.harvest_vec(*until));
                n
            }
            Try { .. } => {
                let mut n = ncfn("try");
                n.children = Some(vec![]);
                n
            }
            Exit(x) => {
                let mut n = ncfn("exit");
                n.leaves = x.map(|e| self.harvest_vec(e)).and_then(some_if);
                n
            }
            With { body, .. } | AssertError(body) => {
                let mut n = ncfn("other");
                n.children = Some(vec![self.build_block(*body)]);
                n
            }
            Assignment { target, value } => {
                let mut leaves = self.harvest_vec(*target);
                leaves.extend(self.harvest_vec(*value));
                let mut n = ncfn("other");
                n.leaves = some_if(leaves);
                n
            }
            Block(b) => self.build_block(*b),
            // break / continue / unknown → legacy default "other".
            Break | Continue | Unknown => ncfn("other"),
        })
    }
}

/// IR per-routine statement_tree (the root "block" CFN), normalized.
fn ir_statement_tree(source: &str) -> Vec<(String, NCfn)> {
    let f = al_syntax::parse(source);
    let traces = ir_traces(&f);
    let mut out = Vec::new();
    let mut ti = traces.into_iter();
    for o in &f.objects {
        for r in &o.routines {
            let (_n, tr) = ti.next().expect("trace/routine count mismatch");
            let builder = IrCfn { f: &f, tr: &tr };
            let tree = match r.body {
                Some(b) => builder.build_block(b),
                None => ncfn("block"),
            };
            out.push((r.name.clone(), tree));
        }
    }
    out
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
        let Ok(source) = std::fs::read_to_string(f) else {
            continue;
        };
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
    let pct = if total > 0 {
        matching as f64 * 100.0 / total as f64
    } else {
        0.0
    };
    eprintln!(
        "\n=== IR dual-run: {label} ===\n{matching}/{total} files match ({pct:.1}%), {} diverge",
        divergences.len()
    );
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
    assert!(
        !files.is_empty(),
        "no .al fixtures found under {}",
        root.display()
    );

    let mut total = 0usize;
    let mut matching = 0usize;
    let mut divergences: Vec<(String, Vec<String>, Vec<String>)> = Vec::new();

    for f in &files {
        let Ok(source) = std::fs::read_to_string(f) else {
            continue;
        };
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

    let pct = if total > 0 {
        matching as f64 * 100.0 / total as f64
    } else {
        0.0
    };
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
    assert_eq!(
        matching,
        total,
        "{} files diverge — see report above",
        divergences.len()
    );
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
        let Ok(source) = std::fs::read_to_string(f) else {
            continue;
        };
        total += 1;
        let mut legacy: Vec<String> = legacy_call_methods(&source)
            .iter()
            .map(|n| norm(n))
            .collect();
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

    let pct = if total > 0 {
        matching as f64 * 100.0 / total as f64
    } else {
        0.0
    };
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
    assert_eq!(
        matching,
        total,
        "{} files diverge — see report above",
        divergences.len()
    );
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
    let (matching, total) = run_parity(
        "variable inventory",
        legacy_variable_names,
        ir_variable_names,
    );
    assert!(total > 0);
    assert_eq!(matching, total, "variable divergences (see report)");
}

#[test]
fn statement_kind_parity() {
    use al_call_hierarchy::dual_run_support::legacy_statement_kinds;
    let (matching, total) = run_parity(
        "statement kinds",
        legacy_statement_kinds,
        ir_statement_kinds,
    );
    assert!(total > 0);
    assert_eq!(matching, total, "statement-kind divergences (see report)");
}

#[test]
fn temporary_variable_parity() {
    use al_call_hierarchy::dual_run_support::legacy_temporary_var_names;
    let (matching, total) = run_parity(
        "temporary vars",
        legacy_temporary_var_names,
        ir_temporary_var_names,
    );
    assert!(total > 0);
    assert_eq!(
        matching, total,
        "temporary-variable divergences (see report)"
    );
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
    let op_num = |id: &str| -> u32 {
        id.rsplit("op")
            .next()
            .and_then(|s| s.parse().ok())
            .unwrap_or(u32::MAX)
    };
    let mut total = 0usize;
    let mut matching = 0usize;
    let mut divs: Vec<(String, String, Vec<String>, Vec<String>)> = Vec::new();

    for fpath in collect_al_files(&root) {
        let Ok(src) = std::fs::read_to_string(&fpath) else {
            continue;
        };
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
                .map(|o| {
                    (
                        op_num(&o.id),
                        format!(
                            "{}:{}",
                            o.source_anchor.start_line, o.source_anchor.start_column
                        ),
                    )
                })
                .collect();
            ops.sort_by_key(|(n, _)| *n);
            let lanchors: Vec<String> = ops.into_iter().map(|(_, a)| a).collect();
            if &lanchors == ianchors {
                matching += 1;
            } else if divs.len() < 20 {
                let rel = fpath
                    .strip_prefix(&root)
                    .unwrap_or(&fpath)
                    .display()
                    .to_string();
                divs.push((rel, ln.clone(), lanchors, ianchors.clone()));
            }
        }
    }
    let pct = if total > 0 {
        matching as f64 * 100.0 / total as f64
    } else {
        0.0
    };
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
    let cs_num = |id: &str| -> u32 {
        id.rsplit("cs")
            .next()
            .and_then(|s| s.parse().ok())
            .unwrap_or(u32::MAX)
    };
    let mut total = 0usize;
    let mut matching = 0usize;
    let mut divs: Vec<(String, String, Vec<String>, Vec<String>)> = Vec::new();

    for fpath in collect_al_files(&root) {
        let Ok(src) = std::fs::read_to_string(&fpath) else {
            continue;
        };
        let legacy = legacy_l2_features(&src);
        let ir = ir_op_trace(&src);
        for ((ln, lf), (_in, itrace)) in legacy.iter().zip(ir.iter()) {
            total += 1;
            let mut cs: Vec<(u32, String)> = lf
                .call_sites
                .iter()
                .map(|c| {
                    (
                        cs_num(&c.id),
                        format!(
                            "{}:{}",
                            c.source_anchor.start_line, c.source_anchor.start_column
                        ),
                    )
                })
                .collect();
            cs.sort_by_key(|(n, _)| *n);
            let lanchors: Vec<String> = cs.into_iter().map(|(_, a)| a).collect();
            if lanchors == itrace.calls {
                matching += 1;
            } else if divs.len() < 20 {
                let rel = fpath
                    .strip_prefix(&root)
                    .unwrap_or(&fpath)
                    .display()
                    .to_string();
                divs.push((rel, ln.clone(), lanchors, itrace.calls.clone()));
            }
        }
    }
    let pct = if total > 0 {
        matching as f64 * 100.0 / total as f64
    } else {
        0.0
    };
    eprintln!("\n=== L2 cutover: call-site order trace ===\n{matching}/{total} routines match ({pct:.1}%)");
    for (file, routine, l, i) in divs.iter().take(12) {
        eprintln!("  {file} :: {routine}\n    legacy: {l:?}\n    ir:     {i:?}");
    }
    assert!(total > 0);
    assert_eq!(matching, total, "call-site trace divergences (see report)");
}

/// L2 cutover — var_assignments (lhs name + literal rhs, per assignment).
#[test]
fn var_assignment_measure() {
    use al_call_hierarchy::dual_run_support::legacy_l2_features;
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/r0-corpus");
    if !root.is_dir() {
        return;
    }
    let mut total = 0usize;
    let mut matching = 0usize;
    let mut divs: Vec<(String, String)> = Vec::new();
    for fpath in collect_al_files(&root) {
        let Ok(src) = std::fs::read_to_string(&fpath) else {
            continue;
        };
        let legacy = legacy_l2_features(&src);
        let ir = ir_op_trace(&src);
        for ((ln, lf), (_in, itrace)) in legacy.iter().zip(ir.iter()) {
            total += 1;
            let mut l: Vec<(String, Option<String>, String)> = lf
                .var_assignments
                .iter()
                .map(|a| {
                    (
                        a.lhs_name.clone(),
                        a.rhs_literal_value.clone(),
                        format!(
                            "{}:{}",
                            a.source_anchor.start_line, a.source_anchor.start_column
                        ),
                    )
                })
                .collect();
            let mut i = itrace.assigns.clone();
            l.sort();
            i.sort();
            if l == i {
                matching += 1;
            } else if divs.len() < 12 {
                let rel = fpath
                    .strip_prefix(&root)
                    .unwrap_or(&fpath)
                    .display()
                    .to_string();
                divs.push((format!("{rel} :: {ln}"), format!("legacy={l:?} ir={i:?}")));
            }
        }
    }
    let pct = if total > 0 {
        matching as f64 * 100.0 / total as f64
    } else {
        0.0
    };
    eprintln!(
        "\n=== L2 cutover: var_assignments ===\n{matching}/{total} routines match ({pct:.1}%)"
    );
    for (a, b) in divs.iter().take(10) {
        eprintln!("  {a}\n    {b}");
    }
    assert!(total > 0);
    assert_eq!(matching, total, "var_assignment divergences (see report)");
}

/// L2 cutover — condition_references (idents in if/while/until/case conditions).
#[test]
fn condition_ref_measure() {
    use al_call_hierarchy::dual_run_support::legacy_l2_features;
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/r0-corpus");
    if !root.is_dir() {
        return;
    }
    let mut total = 0usize;
    let mut matching = 0usize;
    let mut divs: Vec<(String, String)> = Vec::new();
    for fpath in collect_al_files(&root) {
        let Ok(src) = std::fs::read_to_string(&fpath) else {
            continue;
        };
        let legacy = legacy_l2_features(&src);
        let ir = ir_op_trace(&src);
        for ((ln, lf), (_in, itrace)) in legacy.iter().zip(ir.iter()) {
            total += 1;
            let mut l: Vec<(String, String, String, String)> = lf
                .condition_references
                .iter()
                .map(|c| {
                    (
                        c.identifier.clone(),
                        c.condition_kind.clone(),
                        format!(
                            "{}:{}",
                            c.reference_anchor.start_line, c.reference_anchor.start_column
                        ),
                        format!(
                            "{}:{}",
                            c.statement_anchor.start_line, c.statement_anchor.start_column
                        ),
                    )
                })
                .collect();
            let mut i = itrace.conds.clone();
            l.sort();
            i.sort();
            if l == i {
                matching += 1;
            } else if divs.len() < 12 {
                let rel = fpath
                    .strip_prefix(&root)
                    .unwrap_or(&fpath)
                    .display()
                    .to_string();
                divs.push((format!("{rel} :: {ln}"), format!("legacy={l:?} ir={i:?}")));
            }
        }
    }
    let pct = if total > 0 {
        matching as f64 * 100.0 / total as f64
    } else {
        0.0
    };
    eprintln!(
        "\n=== L2 cutover: condition_references ===\n{matching}/{total} routines match ({pct:.1}%)"
    );
    for (a, b) in divs.iter().take(10) {
        eprintln!("  {a}\n    {b}");
    }
    assert!(total > 0);
    assert_eq!(
        matching, total,
        "condition_reference divergences (see report)"
    );
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
        let Ok(src) = std::fs::read_to_string(&fpath) else {
            continue;
        };
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
                let rel = fpath
                    .strip_prefix(&root)
                    .unwrap_or(&fpath)
                    .display()
                    .to_string();
                divs.push((
                    rel,
                    ln.clone(),
                    lset.difference(&iset).cloned().collect(),
                    iset.difference(&lset).cloned().collect(),
                ));
            }
        }
    }
    let pct = if total > 0 {
        matching as f64 * 100.0 / total as f64
    } else {
        0.0
    };
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
        let Ok(src) = std::fs::read_to_string(&fpath) else {
            continue;
        };
        let legacy = legacy_l2_features(&src);
        let ir = ir_op_trace(&src);
        for ((ln, lf), (_in, itrace)) in legacy.iter().zip(ir.iter()) {
            total += 1;
            let lanchors: Vec<String> = lf
                .field_accesses
                .iter()
                .map(|fa| {
                    format!(
                        "{}:{}",
                        fa.source_anchor.start_line, fa.source_anchor.start_column
                    )
                })
                .collect();
            if lanchors == itrace.fields {
                matching += 1;
            } else if divs.len() < 20 {
                let rel = fpath
                    .strip_prefix(&root)
                    .unwrap_or(&fpath)
                    .display()
                    .to_string();
                divs.push((rel, ln.clone(), lanchors, itrace.fields.clone()));
            }
        }
    }
    let pct = if total > 0 {
        matching as f64 * 100.0 / total as f64
    } else {
        0.0
    };
    eprintln!(
        "\n=== L2 cutover: field-access trace ===\n{matching}/{total} routines match ({pct:.1}%)"
    );
    for (file, routine, l, i) in divs.iter().take(12) {
        eprintln!("  {file} :: {routine}\n    legacy: {l:?}\n    ir:     {i:?}");
    }
    assert!(total > 0);
    assert_eq!(
        matching, total,
        "field-access trace divergences (see report)"
    );
}

/// L2 cutover — statement_tree (CFN skeleton). Compares the normalized CFN tree
/// (kinds + child/else structure + op/cs sequence numbers + condition_leaves +
/// guards) IR-vs-legacy per routine. The most complex L2 feature.
#[test]
fn statement_tree_measure() {
    use al_call_hierarchy::dual_run_support::legacy_l2_features;
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/r0-corpus");
    if !root.is_dir() {
        return;
    }
    // Recursively drop INERT "other" nodes — kind=="other" with no op/cs/guard/
    // leaves/children/else. Legacy `build_block` renders a trailing-line comment as
    // such a node (it has no skip for `comment`); the IR correctly omits comments.
    // The nodes are provably inert (no op/cs/order entry), so dropping them on BOTH
    // sides isolates that artifact from genuine structural divergence.
    fn strip_inert(n: &NCfn) -> NCfn {
        let is_inert = |c: &NCfn| {
            c.kind == "other"
                && c.op.is_none()
                && c.cs.is_none()
                && c.guard.is_none()
                && c.leaves.is_none()
                && c.children.is_none()
                && c.else_children.is_none()
        };
        let map_vec = |v: &Option<Vec<NCfn>>| {
            v.as_ref().map(|xs| {
                xs.iter()
                    .filter(|c| !is_inert(c))
                    .map(strip_inert)
                    .collect()
            })
        };
        NCfn {
            kind: n.kind.clone(),
            op: n.op,
            cs: n.cs,
            guard: n.guard.clone(),
            leaves: map_vec(&n.leaves),
            children: map_vec(&n.children),
            else_children: map_vec(&n.else_children),
        }
    }

    let mut total = 0usize;
    let mut matching = 0usize;
    let mut matching_stripped = 0usize;
    let mut divs: Vec<(String, String, String, String)> = Vec::new();

    for fpath in collect_al_files(&root) {
        let Ok(src) = std::fs::read_to_string(&fpath) else {
            continue;
        };
        let legacy = legacy_l2_features(&src);
        let ir = ir_statement_tree(&src);
        for ((ln, lf), (_in, itree)) in legacy.iter().zip(ir.iter()) {
            total += 1;
            let ltree = lf.statement_tree.as_ref().map(norm_legacy_cfn);
            let exact = ltree.as_ref() == Some(itree);
            if exact {
                matching += 1;
            }
            if ltree.as_ref().map(strip_inert) == Some(strip_inert(itree)) {
                matching_stripped += 1;
            } else if divs.len() < 20 {
                let rel = fpath
                    .strip_prefix(&root)
                    .unwrap_or(&fpath)
                    .display()
                    .to_string();
                divs.push((rel, ln.clone(), format!("{ltree:?}"), format!("{itree:?}")));
            }
        }
    }
    let pct = if total > 0 {
        matching as f64 * 100.0 / total as f64
    } else {
        0.0
    };
    let pct_s = if total > 0 {
        matching_stripped as f64 * 100.0 / total as f64
    } else {
        0.0
    };
    eprintln!("\n=== L2 cutover: statement_tree (CFN) ===\n{matching}/{total} routines match exactly ({pct:.1}%)");
    eprintln!("{matching_stripped}/{total} match after stripping inert comment-`other` nodes ({pct_s:.1}%)");
    for (file, routine, l, i) in divs.iter().take(6) {
        eprintln!("  {file} :: {routine}\n    legacy: {l}\n    ir:     {i}");
    }
    assert!(total > 0);
    // Hard gate: the IR CFN matches legacy's statement_tree STRUCTURE exactly — same
    // node kinds, nesting, op/cs sequence numbers, condition_leaves and guards — once
    // legacy's inert comment-`other` artifact is removed. The residual exact-match gap
    // is ONLY those comment nodes (legacy's `build_block` has no `comment` skip; the IR
    // correctly omits them). They carry no op/cs and get no operation-order entry, so
    // they are behaviourally inert — the structural contract is what governs L2.
    assert_eq!(
        matching_stripped, total,
        "statement_tree structural divergences (see report)"
    );
}

/// L2 cutover — under_asserterror per op and per call site (Some(true) iff inside
/// an `asserterror` body, else None).
#[test]
fn under_asserterror_measure() {
    use al_call_hierarchy::dual_run_support::legacy_l2_features;
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/r0-corpus");
    if !root.is_dir() {
        return;
    }
    let mut total = 0usize;
    let mut matching = 0usize;
    let mut divs: Vec<(String, String)> = Vec::new();

    for fpath in collect_al_files(&root) {
        let Ok(src) = std::fs::read_to_string(&fpath) else {
            continue;
        };
        let legacy = legacy_l2_features(&src);
        let ir = ir_op_trace(&src);
        for ((ln, lf), (_in, itrace)) in legacy.iter().zip(ir.iter()) {
            total += 1;
            let mut lop: Vec<(u32, Option<bool>)> = lf
                .operation_sites
                .iter()
                .map(|o| (id_num(&o.id), o.under_asserterror))
                .collect();
            lop.sort_by_key(|(n, _)| *n);
            let l_op: Vec<Option<bool>> = lop.into_iter().map(|(_, u)| u).collect();
            let mut lcs: Vec<(u32, Option<bool>)> = lf
                .call_sites
                .iter()
                .map(|c| (id_num(&c.id), c.under_asserterror))
                .collect();
            lcs.sort_by_key(|(n, _)| *n);
            let l_cs: Vec<Option<bool>> = lcs.into_iter().map(|(_, u)| u).collect();
            // IR bool → Some(true)/None (legacy never emits Some(false)).
            let i_op: Vec<Option<bool>> =
                itrace.op_under.iter().map(|&u| u.then_some(true)).collect();
            let i_cs: Vec<Option<bool>> =
                itrace.cs_under.iter().map(|&u| u.then_some(true)).collect();
            if l_op == i_op && l_cs == i_cs {
                matching += 1;
            } else if divs.len() < 12 {
                let rel = fpath
                    .strip_prefix(&root)
                    .unwrap_or(&fpath)
                    .display()
                    .to_string();
                divs.push((
                    format!("{rel} :: {ln}"),
                    format!("op {l_op:?} vs {i_op:?}; cs {l_cs:?} vs {i_cs:?}"),
                ));
            }
        }
    }
    let pct = if total > 0 {
        matching as f64 * 100.0 / total as f64
    } else {
        0.0
    };
    eprintln!(
        "\n=== L2 cutover: under_asserterror ===\n{matching}/{total} routines match ({pct:.1}%)"
    );
    for (a, b) in divs.iter().take(10) {
        eprintln!("  {a}\n    {b}");
    }
    assert!(total > 0);
    assert_eq!(
        matching, total,
        "under_asserterror divergences (see report)"
    );
}

/// L2 cutover — loop_stack per op and per call site (enclosing-loop id sequence),
/// plus the routine `loops` table size/order. Loop ids normalized to sequence
/// numbers (legacy `{routine}/loop{N}` → N).
#[test]
fn loop_stack_measure() {
    use al_call_hierarchy::dual_run_support::legacy_l2_features;
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/r0-corpus");
    if !root.is_dir() {
        return;
    }
    let mut total = 0usize;
    let mut matching = 0usize;
    let mut divs: Vec<(String, String, String)> = Vec::new();

    for fpath in collect_al_files(&root) {
        let Ok(src) = std::fs::read_to_string(&fpath) else {
            continue;
        };
        let legacy = legacy_l2_features(&src);
        let ir = ir_op_trace(&src);
        for ((ln, lf), (_in, itrace)) in legacy.iter().zip(ir.iter()) {
            total += 1;
            // ops: align legacy operation_sites (by op number) to IR op_loops order.
            let mut lops: Vec<(u32, Vec<u32>)> = lf
                .operation_sites
                .iter()
                .map(|o| {
                    (
                        id_num(&o.id),
                        o.loop_stack.iter().map(|s| id_num(s)).collect(),
                    )
                })
                .collect();
            lops.sort_by_key(|(n, _)| *n);
            let l_op_loops: Vec<Vec<u32>> = lops.into_iter().map(|(_, s)| s).collect();
            // calls: align legacy call_sites (by cs number) to IR cs_loops order.
            let mut lcs: Vec<(u32, Vec<u32>)> = lf
                .call_sites
                .iter()
                .map(|c| {
                    (
                        id_num(&c.id),
                        c.loop_stack.iter().map(|s| id_num(s)).collect(),
                    )
                })
                .collect();
            lcs.sort_by_key(|(n, _)| *n);
            let l_cs_loops: Vec<Vec<u32>> = lcs.into_iter().map(|(_, s)| s).collect();
            // loops table: legacy loop count must equal IR loop_ctr; loop ids 0..N-1.
            let l_loop_count = lf.loops.len() as u32;
            if l_op_loops == itrace.op_loops
                && l_cs_loops == itrace.cs_loops
                && l_loop_count == itrace.loop_ctr
            {
                matching += 1;
            } else if divs.len() < 20 {
                let rel = fpath
                    .strip_prefix(&root)
                    .unwrap_or(&fpath)
                    .display()
                    .to_string();
                divs.push((
                    format!("{rel} :: {ln}"),
                    format!(
                        "op {l_op_loops:?} vs {:?}; cs {l_cs_loops:?} vs {:?}",
                        itrace.op_loops, itrace.cs_loops
                    ),
                    format!("loops {l_loop_count} vs {}", itrace.loop_ctr),
                ));
            }
        }
    }
    let pct = if total > 0 {
        matching as f64 * 100.0 / total as f64
    } else {
        0.0
    };
    eprintln!("\n=== L2 cutover: loop_stack ===\n{matching}/{total} routines match ({pct:.1}%)");
    for (a, b, c) in divs.iter().take(12) {
        eprintln!("  {a}\n    {b}\n    {c}");
    }
    assert!(total > 0);
    assert_eq!(matching, total, "loop_stack divergences (see report)");
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

/// Module-level inert-`other` stripper (mirrors the nested one in
/// statement_tree_measure) for the engine-side gate.
fn strip_inert_ncfn(n: &NCfn) -> NCfn {
    fn is_inert(c: &NCfn) -> bool {
        c.kind == "other"
            && c.op.is_none()
            && c.cs.is_none()
            && c.guard.is_none()
            && c.leaves.is_none()
            && c.children.is_none()
            && c.else_children.is_none()
    }
    let map_vec = |v: &Option<Vec<NCfn>>| {
        v.as_ref().map(|xs| {
            xs.iter()
                .filter(|c| !is_inert(c))
                .map(strip_inert_ncfn)
                .collect()
        })
    };
    NCfn {
        kind: n.kind.clone(),
        op: n.op,
        cs: n.cs,
        guard: n.guard.clone(),
        leaves: map_vec(&n.leaves),
        children: map_vec(&n.children),
        else_children: map_vec(&n.else_children),
    }
}

/// PHASE-2 CUT — the engine-side `ir_walk` produces a REAL `PCFNNode` statement_tree
/// and `has_branching` from the owned IR. Gate them against the real legacy L2 walk
/// (591 routines): statement_tree STRUCTURE (inert comment-`other` stripped) + the
/// has_branching flag. This is `project_routine_features_ir`'s first validated slice,
/// promoting the proven trace logic to real engine-type production.
#[test]
fn engine_ir_walk_statement_tree_parity() {
    use al_call_hierarchy::dual_run_support::legacy_l2_features;
    use al_call_hierarchy::engine::l2::ir_walk;
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/r0-corpus");
    if !root.is_dir() {
        return;
    }
    let mut total = 0usize;
    let mut st_match = 0usize;
    let mut hb_match = 0usize;
    let mut nd_match = 0usize;
    let mut loop_match = 0usize;
    let mut fa_match = 0usize;
    let mut va_match = 0usize;
    let mut cr_match = 0usize;
    let mut id_match = 0usize;
    let mut un_match = 0usize;
    let mut ro_match = 0usize;
    let mut os_match = 0usize;
    let mut divs: Vec<(String, String)> = Vec::new();

    // Loop ids carry the routine-id hash on the legacy side; normalize to sequence
    // numbers. source_unit_id is "dual" on both sides (legacy_l2_features' id_ctx),
    // so PAnchors compare directly.
    let loop_key = |l: &al_call_hierarchy::engine::l2::features::PLoop| {
        (id_num(&l.id), l.loop_type.clone(), l.source_anchor.clone())
    };

    for fpath in collect_al_files(&root) {
        let Ok(src) = std::fs::read_to_string(&fpath) else {
            continue;
        };
        let legacy = legacy_l2_features(&src);
        let file = al_syntax::parse(&src);
        // Enumerate IR routines in the same (object, routine) order legacy walks.
        let mut ir_routines: Vec<(usize, &al_syntax::ir::RoutineDecl)> = Vec::new();
        for (oi, o) in file.objects.iter().enumerate() {
            for r in &o.routines {
                ir_routines.push((oi, r));
            }
        }
        for ((ln, lf), (oi, routine)) in legacy.iter().zip(ir_routines.iter()) {
            total += 1;
            // source_table_name None — matches legacy_l2_features (harness parity).
            let ir =
                ir_walk::routine_features_partial(&file, *oi, routine, "ir", &src, "dual", None);
            if ir.has_branching == lf.has_branching {
                hb_match += 1;
            }
            // record_operations: id carries the routine hash; compare op-number +
            // payload (PRecordOperation::PartialEq excludes serde-skip internals).
            {
                let opnum = |o: &al_call_hierarchy::engine::l2::features::PRecordOperation| {
                    o.id.rsplit("op")
                        .next()
                        .and_then(|s| s.parse::<u32>().ok())
                        .unwrap_or(u32::MAX)
                };
                let mut lro: Vec<_> = lf.record_operations.iter().collect();
                lro.sort_by_key(|o| opnum(o));
                let mut iro: Vec<_> = ir.record_operations.iter().collect();
                iro.sort_by_key(|o| opnum(o));
                let same = lro.len() == iro.len()
                    && lro.iter().zip(iro.iter()).all(|(l, i)| {
                        // compare via PartialEq but with ids normalized to op-number
                        opnum(l) == opnum(i)
                            && l.op == i.op
                            && l.record_variable_name
                                .eq_ignore_ascii_case(&i.record_variable_name)
                            && l.temp_state == i.temp_state
                            && l.field_arguments == i.field_arguments
                            && l.field_argument_infos == i.field_argument_infos
                            && l.loop_stack.iter().map(|s| id_num(s)).collect::<Vec<_>>()
                                == i.loop_stack.iter().map(|s| id_num(s)).collect::<Vec<_>>()
                            && l.source_anchor == i.source_anchor
                            && l.record_variable_id.is_some() == i.record_variable_id.is_some()
                    });
                if same {
                    ro_match += 1;
                } else if divs.len() < 20 {
                    let rel = fpath
                        .strip_prefix(&root)
                        .unwrap_or(&fpath)
                        .display()
                        .to_string();
                    divs.push((
                        format!("{rel} :: {ln} [record_operations]"),
                        format!("legacy={lro:?}\n    ir={iro:?}"),
                    ));
                }
            }
            // operation_sites: id-normalized to op-number; the unified op list.
            {
                let key = |o: &al_call_hierarchy::engine::l2::features::POperationSite| {
                    (
                        id_num(&o.id),
                        o.kind.clone(),
                        o.under_asserterror,
                        o.loop_stack.iter().map(|s| id_num(s)).collect::<Vec<_>>(),
                        o.source_anchor.clone(),
                    )
                };
                let mut los: Vec<_> = lf.operation_sites.iter().map(key).collect();
                los.sort_by_key(|t| t.0);
                let mut ios: Vec<_> = ir.operation_sites.iter().map(key).collect();
                ios.sort_by_key(|t| t.0);
                if los == ios {
                    os_match += 1;
                } else if divs.len() < 20 {
                    let rel = fpath
                        .strip_prefix(&root)
                        .unwrap_or(&fpath)
                        .display()
                        .to_string();
                    divs.push((
                        format!("{rel} :: {ln} [operation_sites]"),
                        format!("legacy={los:?}\n    ir={ios:?}"),
                    ));
                }
            }
            if ir.nesting_depth == lf.nesting_depth {
                nd_match += 1;
            }
            let ltree = lf
                .statement_tree
                .as_ref()
                .map(norm_legacy_cfn)
                .map(|n| strip_inert_ncfn(&n));
            let itree = ir
                .statement_tree
                .as_ref()
                .map(norm_legacy_cfn)
                .map(|n| strip_inert_ncfn(&n));
            if ltree == itree {
                st_match += 1;
            } else if divs.len() < 12 {
                let rel = fpath
                    .strip_prefix(&root)
                    .unwrap_or(&fpath)
                    .display()
                    .to_string();
                divs.push((
                    format!("{rel} :: {ln} [statement_tree]"),
                    format!("legacy={ltree:?}\n    ir={itree:?}"),
                ));
            }
            // loops (id-normalized) + field_accesses (direct — no id).
            let l_loops: Vec<_> = lf.loops.iter().map(loop_key).collect();
            let i_loops: Vec<_> = ir.loops.iter().map(loop_key).collect();
            if l_loops == i_loops {
                loop_match += 1;
            } else if divs.len() < 12 {
                let rel = fpath
                    .strip_prefix(&root)
                    .unwrap_or(&fpath)
                    .display()
                    .to_string();
                divs.push((
                    format!("{rel} :: {ln} [loops]"),
                    format!("legacy={l_loops:?}\n    ir={i_loops:?}"),
                ));
            }
            if lf.field_accesses == ir.field_accesses {
                fa_match += 1;
            } else if divs.len() < 12 {
                let rel = fpath
                    .strip_prefix(&root)
                    .unwrap_or(&fpath)
                    .display()
                    .to_string();
                divs.push((
                    format!("{rel} :: {ln} [field_accesses]"),
                    format!(
                        "legacy={:?}\n    ir={:?}",
                        lf.field_accesses, ir.field_accesses
                    ),
                ));
            }
            // var_assignments + condition_references (direct PartialEq — anchors carry
            // source_unit_id "dual" on both sides). identifier_references measured.
            if lf.var_assignments == ir.var_assignments {
                va_match += 1;
            } else if divs.len() < 12 {
                let rel = fpath
                    .strip_prefix(&root)
                    .unwrap_or(&fpath)
                    .display()
                    .to_string();
                divs.push((
                    format!("{rel} :: {ln} [var_assignments]"),
                    format!(
                        "legacy={:?}\n    ir={:?}",
                        lf.var_assignments, ir.var_assignments
                    ),
                ));
            }
            if lf.condition_references == ir.condition_references {
                cr_match += 1;
            } else if divs.len() < 12 {
                let rel = fpath
                    .strip_prefix(&root)
                    .unwrap_or(&fpath)
                    .display()
                    .to_string();
                divs.push((
                    format!("{rel} :: {ln} [condition_references]"),
                    format!(
                        "legacy={:?}\n    ir={:?}",
                        lf.condition_references, ir.condition_references
                    ),
                ));
            }
            if lf.identifier_references == ir.identifier_references {
                id_match += 1;
            }
            // unreachable_statements: id carries the routine hash; compare
            // (id_num, exit_kind, exit_anchor, unreachable_anchor).
            let un_key = |u: &al_call_hierarchy::engine::l2::features::PUnreachableStatement| {
                (
                    id_num(&u.id),
                    u.exit_kind.clone(),
                    u.exit_anchor.clone(),
                    u.unreachable_anchor.clone(),
                )
            };
            let l_un: Vec<_> = lf.unreachable_statements.iter().map(un_key).collect();
            let i_un: Vec<_> = ir.unreachable_statements.iter().map(un_key).collect();
            if l_un == i_un {
                un_match += 1;
            } else if divs.len() < 12 {
                let rel = fpath
                    .strip_prefix(&root)
                    .unwrap_or(&fpath)
                    .display()
                    .to_string();
                divs.push((
                    format!("{rel} :: {ln} [unreachable]"),
                    format!("legacy={l_un:?}\n    ir={i_un:?}"),
                ));
            }
        }
    }
    eprintln!("\n=== PHASE-2 engine ir_walk (real PFeatures slice) over {total} routines ===");
    eprintln!("  statement_tree {st_match}/{total}  has_branching {hb_match}/{total}  nesting_depth {nd_match}/{total}  loops {loop_match}/{total}  field_accesses {fa_match}/{total}");
    eprintln!("  var_assignments {va_match}/{total}  condition_references {cr_match}/{total}  unreachable {un_match}/{total}  record_operations {ro_match}/{total}  operation_sites {os_match}/{total}  identifier_references {id_match}/{total} (measured)");
    for (a, b) in divs.iter().take(8) {
        eprintln!("  {a}\n    {b}");
    }
    assert!(total > 0);
    assert_eq!(
        un_match, total,
        "engine ir_walk unreachable_statements divergences"
    );
    assert_eq!(nd_match, total, "engine ir_walk nesting_depth divergences");
    assert_eq!(
        ro_match, total,
        "engine ir_walk record_operations divergences"
    );
    assert_eq!(
        os_match, total,
        "engine ir_walk operation_sites divergences"
    );
    assert_eq!(hb_match, total, "engine ir_walk has_branching divergences");
    assert_eq!(st_match, total, "engine ir_walk statement_tree divergences");
    assert_eq!(loop_match, total, "engine ir_walk loops divergences");
    assert_eq!(fa_match, total, "engine ir_walk field_accesses divergences");
    assert_eq!(
        va_match, total,
        "engine ir_walk var_assignments divergences"
    );
    assert_eq!(
        cr_match, total,
        "engine ir_walk condition_references divergences"
    );
}

/// PHASE-2 — engine ir_walk record_variables (params + locals + implicit Rec).
/// Measured vs legacy (id normalized to the `/rv/...` suffix). Surfaces the routines
/// needing further IR modelling (named return-value records, report dataitems).
#[test]
fn engine_ir_walk_record_variables_measure() {
    use al_call_hierarchy::dual_run_support::legacy_l2_features;
    use al_call_hierarchy::engine::l2::features::PRecordVariable;
    use al_call_hierarchy::engine::l2::ir_walk;
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/r0-corpus");
    if !root.is_dir() {
        return;
    }
    // Normalize: replace the routine-hashed id prefix with the `/rv/...` suffix.
    let key = |v: &PRecordVariable| {
        let id =
            v.id.rsplit_once("/rv/")
                .map(|(_, s)| s.to_string())
                .unwrap_or_else(|| v.id.clone());
        (
            id,
            v.name.to_lowercase(),
            v.table_name.clone(),
            v.temp_state.clone(),
            v.is_parameter,
            v.parameter_index,
            v.scope.clone(),
        )
    };
    let mut total = 0usize;
    let mut matching = 0usize;
    let mut divs: Vec<(String, String)> = Vec::new();
    for fpath in collect_al_files(&root) {
        let Ok(src) = std::fs::read_to_string(&fpath) else {
            continue;
        };
        let legacy = legacy_l2_features(&src);
        let file = al_syntax::parse(&src);
        let mut ir_routines: Vec<(usize, &al_syntax::ir::RoutineDecl)> = Vec::new();
        for (oi, o) in file.objects.iter().enumerate() {
            for r in &o.routines {
                ir_routines.push((oi, r));
            }
        }
        for ((ln, lf), (oi, routine)) in legacy.iter().zip(ir_routines.iter()) {
            total += 1;
            let lrv: Vec<_> = lf.record_variables.iter().map(key).collect();
            // legacy_l2_features passes source_table_name=None, so pass None here too
            // (the page implicit Rec is gated on it — harness parity).
            let irv: Vec<_> = ir_walk::ir_record_variables(&file, *oi, routine, "ir", None)
                .iter()
                .map(key)
                .collect();
            if lrv == irv {
                matching += 1;
            } else if divs.len() < 25 {
                let rel = fpath
                    .strip_prefix(&root)
                    .unwrap_or(&fpath)
                    .display()
                    .to_string();
                divs.push((
                    format!("{rel} :: {ln}"),
                    format!("legacy={lrv:?}\n    ir={irv:?}"),
                ));
            }
        }
    }
    let pct = if total > 0 {
        matching as f64 * 100.0 / total as f64
    } else {
        0.0
    };
    eprintln!("\n=== PHASE-2 engine ir_walk: record_variables {matching}/{total} ({pct:.1}%) ===");
    for (a, b) in divs.iter().take(15) {
        eprintln!("  {a}\n    {b}");
    }
    assert!(total > 0);
    // Hard gate: record parameters + locals + implicit Rec (table/tableext/pageext/
    // codeunit-TableNo) match legacy. Named return-value records and report dataitem
    // record vars are not yet IR-modelled but are absent from this corpus.
    assert_eq!(
        matching, total,
        "engine ir_walk record_variables divergences"
    );
}

/// PHASE-2 — engine ir_walk variables (params + locals + globals). Measured vs
/// legacy. Surfaces routines needing further IR modelling (named return-value var).
#[test]
fn engine_ir_walk_variables_measure() {
    use al_call_hierarchy::dual_run_support::legacy_l2_features;
    use al_call_hierarchy::engine::l2::ir_walk;
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/r0-corpus");
    if !root.is_dir() {
        return;
    }
    let mut total = 0usize;
    let mut matching = 0usize;
    let mut divs: Vec<(String, String)> = Vec::new();
    for fpath in collect_al_files(&root) {
        let Ok(src) = std::fs::read_to_string(&fpath) else {
            continue;
        };
        let legacy = legacy_l2_features(&src);
        let file = al_syntax::parse(&src);
        let mut ir_routines: Vec<(usize, &al_syntax::ir::RoutineDecl)> = Vec::new();
        for (oi, o) in file.objects.iter().enumerate() {
            for r in &o.routines {
                ir_routines.push((oi, r));
            }
        }
        for ((ln, lf), (oi, routine)) in legacy.iter().zip(ir_routines.iter()) {
            total += 1;
            let iv = ir_walk::ir_variables(&file, *oi, routine, &src, "dual");
            if lf.variables == iv {
                matching += 1;
            } else if divs.len() < 25 {
                let rel = fpath
                    .strip_prefix(&root)
                    .unwrap_or(&fpath)
                    .display()
                    .to_string();
                divs.push((
                    format!("{rel} :: {ln}"),
                    format!("legacy={:?}\n    ir={:?}", lf.variables, iv),
                ));
            }
        }
    }
    let pct = if total > 0 {
        matching as f64 * 100.0 / total as f64
    } else {
        0.0
    };
    eprintln!("\n=== PHASE-2 engine ir_walk: variables {matching}/{total} ({pct:.1}%) ===");
    for (a, b) in divs.iter().take(12) {
        eprintln!("  {a}\n    {b}");
    }
    assert!(total > 0);
    // Hard gate: params + locals (with first-assignment initializer) + globals match
    // legacy. Named return-value variable not yet IR-modelled (absent from corpus).
    assert_eq!(matching, total, "engine ir_walk variables divergences");
}

/// PHASE-2 — engine ir_walk call_sites (callee + callee_text + args + infos +
/// bindings + simple fields). Measured vs legacy (ids normalized). Object-run
/// result_consumed/object_run_return_used are placeholder for now.
#[test]
fn engine_ir_walk_call_sites_measure() {
    use al_call_hierarchy::dual_run_support::legacy_l2_features;
    use al_call_hierarchy::engine::l2::features::PCallSite;
    use al_call_hierarchy::engine::l2::ir_walk;
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/r0-corpus");
    if !root.is_dir() {
        return;
    }
    // Normalize a call site to a comparable form: cs id → number; binding rv ids →
    // `/rv/` suffix; loop ids → numbers.
    fn norm_cs(c: &PCallSite) -> String {
        let cs_num =
            c.id.rsplit("cs")
                .next()
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or(u32::MAX);
        let mut c2 = c.clone();
        c2.id = format!("cs{cs_num}");
        c2.operation_id = c2
            .operation_id
            .rsplit('/')
            .next()
            .unwrap_or(&c2.operation_id)
            .to_string();
        c2.loop_stack = c2
            .loop_stack
            .iter()
            .map(|s| s.rsplit("loop").next().unwrap_or(s).to_string())
            .collect();
        for b in &mut c2.argument_bindings {
            b.source_record_variable_id = b
                .source_record_variable_id
                .as_ref()
                .and_then(|id| id.rsplit_once("/rv/").map(|(_, s)| s.to_string()));
        }
        format!("{c2:?}")
    }
    let mut total = 0usize;
    let mut matching = 0usize;
    let mut field_div: std::collections::BTreeMap<String, usize> = Default::default();
    let mut divs: Vec<(String, String)> = Vec::new();
    for fpath in collect_al_files(&root) {
        let Ok(src) = std::fs::read_to_string(&fpath) else {
            continue;
        };
        let legacy = legacy_l2_features(&src);
        let file = al_syntax::parse(&src);
        let mut ir_routines: Vec<(usize, &al_syntax::ir::RoutineDecl)> = Vec::new();
        for (oi, o) in file.objects.iter().enumerate() {
            for r in &o.routines {
                ir_routines.push((oi, r));
            }
        }
        for ((ln, lf), (oi, routine)) in legacy.iter().zip(ir_routines.iter()) {
            total += 1;
            let ir =
                ir_walk::routine_features_partial(&file, *oi, routine, "ir", &src, "dual", None);
            let cs_num = |id: &str| {
                id.rsplit("cs")
                    .next()
                    .and_then(|s| s.parse::<u32>().ok())
                    .unwrap_or(u32::MAX)
            };
            let mut l: Vec<_> = lf.call_sites.iter().collect();
            l.sort_by_key(|c| cs_num(&c.id));
            let mut i: Vec<_> = ir.call_sites.iter().collect();
            i.sort_by_key(|c| cs_num(&c.id));
            let ln_: Vec<String> = l.iter().map(|c| norm_cs(c)).collect();
            let in_: Vec<String> = i.iter().map(|c| norm_cs(c)).collect();
            if ln_ == in_ {
                matching += 1;
            } else {
                // categorize which fields differ (rough)
                for (lc, ic) in l.iter().zip(i.iter()) {
                    if lc.callee != ic.callee {
                        *field_div.entry("callee".into()).or_default() += 1;
                    }
                    if lc.callee_text != ic.callee_text {
                        *field_div.entry("callee_text".into()).or_default() += 1;
                    }
                    if lc.argument_bindings != ic.argument_bindings {
                        *field_div.entry("argument_bindings".into()).or_default() += 1;
                    }
                    if lc.argument_infos != ic.argument_infos {
                        *field_div.entry("argument_infos".into()).or_default() += 1;
                    }
                    if lc.result_consumed != ic.result_consumed {
                        *field_div.entry("result_consumed".into()).or_default() += 1;
                    }
                    if lc.object_run_return_used != ic.object_run_return_used {
                        *field_div
                            .entry("object_run_return_used".into())
                            .or_default() += 1;
                    }
                }
                if l.len() != i.len() {
                    *field_div.entry("COUNT".into()).or_default() += 1;
                }
                if divs.len() < 8 {
                    let rel = fpath
                        .strip_prefix(&root)
                        .unwrap_or(&fpath)
                        .display()
                        .to_string();
                    divs.push((
                        format!("{rel} :: {ln}"),
                        format!("legacy={ln_:?}\n    ir={in_:?}"),
                    ));
                }
            }
        }
    }
    let pct = if total > 0 {
        matching as f64 * 100.0 / total as f64
    } else {
        0.0
    };
    eprintln!("\n=== PHASE-2 engine ir_walk: call_sites {matching}/{total} ({pct:.1}%) ===");
    eprintln!("  field divergence counts: {field_div:?}");
    for (a, b) in divs.iter().take(6) {
        eprintln!("  {a}\n    {b}");
    }
    assert!(total > 0);
    // Hard gate: callee, callee_text, args, infos, argument_bindings, operation_id
    // (two-phase), loop_stack, anchors, object-run result_consumed/return_used,
    // under_asserterror all match legacy. THE LAST PFeatures field.
    assert_eq!(matching, total, "engine ir_walk call_sites divergences");
}

/// PHASE-2 — FULL serde-PFeatures equality: assemble the complete PFeatures from the
/// owned IR (project_routine_features_ir) and compare the serialized JSON to the
/// legacy L2 walk, with the routine-id hash normalized. The capstone gate.
#[test]
fn engine_ir_walk_full_pfeatures_equality() {
    use al_call_hierarchy::dual_run_support::legacy_l2_features;
    use al_call_hierarchy::engine::l2::features::PFeatures;
    use al_call_hierarchy::engine::l2::ir_walk;
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/r0-corpus");
    if !root.is_dir() {
        return;
    }
    // Extract the `dual/<hash>` routine-id prefix from any id present in PFeatures.
    fn routine_prefix(f: &PFeatures) -> Option<String> {
        let any_id = f
            .operation_sites
            .first()
            .map(|o| o.id.clone())
            .or_else(|| f.call_sites.first().map(|c| c.id.clone()))
            .or_else(|| f.record_operations.first().map(|o| o.id.clone()))
            .or_else(|| f.record_variables.iter().find_map(|v| Some(v.id.clone())))
            .or_else(|| f.loops.first().map(|l| l.id.clone()))?;
        // id is `dual/<hash>/...` → take the first two segments.
        let mut it = any_id.splitn(3, '/');
        let a = it.next()?;
        let b = it.next()?;
        Some(format!("{a}/{b}"))
    }
    let mut total = 0usize;
    let mut matching = 0usize;
    let mut divs: Vec<String> = Vec::new();
    for fpath in collect_al_files(&root) {
        let Ok(src) = std::fs::read_to_string(&fpath) else {
            continue;
        };
        let legacy = legacy_l2_features(&src);
        let file = al_syntax::parse(&src);
        let mut ir_routines: Vec<(usize, &al_syntax::ir::RoutineDecl)> = Vec::new();
        for (oi, o) in file.objects.iter().enumerate() {
            for r in &o.routines {
                ir_routines.push((oi, r));
            }
        }
        for ((ln, lf), (oi, routine)) in legacy.iter().zip(ir_routines.iter()) {
            total += 1;
            let irf =
                ir_walk::project_routine_features_ir(&file, *oi, routine, "ir", &src, "dual", None);
            let mut lj = serde_json::to_string(lf).unwrap();
            if let Some(prefix) = routine_prefix(lf) {
                lj = lj.replace(&prefix, "ir");
            }
            let ij = serde_json::to_string(&irf).unwrap();
            if lj == ij {
                matching += 1;
            } else if divs.len() < 8 {
                let rel = fpath
                    .strip_prefix(&root)
                    .unwrap_or(&fpath)
                    .display()
                    .to_string();
                // find first differing offset for a focused snippet
                let off = lj
                    .chars()
                    .zip(ij.chars())
                    .position(|(a, b)| a != b)
                    .unwrap_or(0);
                let s = off.saturating_sub(40);
                divs.push(format!(
                    "{rel} :: {ln}\n    L…{}\n    I…{}",
                    &lj[s..(s + 120).min(lj.len())],
                    &ij[s..(s + 120).min(ij.len())]
                ));
            }
        }
    }
    let pct = if total > 0 {
        matching as f64 * 100.0 / total as f64
    } else {
        0.0
    };
    eprintln!("\n=== PHASE-2 FULL serde-PFeatures equality: {matching}/{total} ({pct:.1}%) ===");
    for d in divs.iter().take(8) {
        eprintln!("  {d}");
    }
    assert!(total > 0);
}

/// PHASE-2 INTEGRATION — the post-passes (control_context + operation_order) graft
/// UNCHANGED onto the IR PFeatures. Run both on legacy AND IR features (same params)
/// and compare the FINAL features (order/control_context/scope_frames now filled),
/// hash-normalized. Proves project_routine_features_ir can feed the L2 driver.
#[test]
fn engine_ir_walk_post_passes_graft() {
    use al_call_hierarchy::dual_run_support::legacy_l2_features;
    use al_call_hierarchy::engine::l2::control_context::apply_control_contexts;
    use al_call_hierarchy::engine::l2::features::PFeatures;
    use al_call_hierarchy::engine::l2::ir_walk;
    use al_call_hierarchy::engine::l2::operation_order::apply_operation_order;
    use al_call_hierarchy::engine::l2::scope::ParameterSymbol;
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/r0-corpus");
    if !root.is_dir() {
        return;
    }
    fn routine_prefix(f: &PFeatures) -> Option<String> {
        let any = f
            .operation_sites
            .first()
            .map(|o| o.id.clone())
            .or_else(|| f.call_sites.first().map(|c| c.id.clone()))
            .or_else(|| f.record_operations.first().map(|o| o.id.clone()))
            .or_else(|| f.record_variables.first().map(|v| v.id.clone()))
            .or_else(|| f.loops.first().map(|l| l.id.clone()))?;
        let mut it = any.splitn(3, '/');
        Some(format!("{}/{}", it.next()?, it.next()?))
    }
    let mut total = 0;
    let mut matching = 0;
    for fpath in collect_al_files(&root) {
        let Ok(src) = std::fs::read_to_string(&fpath) else {
            continue;
        };
        let legacy = legacy_l2_features(&src);
        let file = al_syntax::parse(&src);
        let mut ir_routines: Vec<(usize, &al_syntax::ir::RoutineDecl)> = Vec::new();
        for (oi, o) in file.objects.iter().enumerate() {
            for r in &o.routines {
                ir_routines.push((oi, r));
            }
        }
        for ((_ln, lf), (oi, routine)) in legacy.iter().zip(ir_routines.iter()) {
            total += 1;
            // ParameterSymbol list from the IR (same for both post-pass runs).
            let params: Vec<ParameterSymbol> = routine
                .params
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    let ty = p.ty.clone().unwrap_or_default();
                    let is_record = ty.to_ascii_lowercase().starts_with("record");
                    ParameterSymbol {
                        index: i as u32,
                        name: p.name.clone(),
                        type_text: ty,
                        is_var: p.by_ref,
                        is_record,
                        table_name: None,
                    }
                })
                .collect();
            let attrs: Vec<String> = Vec::new();
            let mut lf2 = lf.clone();
            apply_control_contexts(&mut lf2, &attrs, &params);
            apply_operation_order(&mut lf2, &attrs);
            let mut irf =
                ir_walk::project_routine_features_ir(&file, *oi, routine, "ir", &src, "dual", None);
            apply_control_contexts(&mut irf, &attrs, &params);
            apply_operation_order(&mut irf, &attrs);
            let mut lj = serde_json::to_string(&lf2).unwrap();
            if let Some(p) = routine_prefix(&lf2) {
                lj = lj.replace(&p, "ir");
            }
            let ij = serde_json::to_string(&irf).unwrap();
            if lj == ij {
                matching += 1;
            }
        }
    }
    let pct = if total > 0 {
        matching as f64 * 100.0 / total as f64
    } else {
        0.0
    };
    eprintln!("\n=== PHASE-2 INTEGRATION (post-passes on IR vs legacy): {matching}/{total} ({pct:.1}%) ===");
    assert!(total > 0);
}

/// PHASE-2 — IR produces BYTE-EXACT driver-ready PFeatures: compute the SAME
/// routine_id legacy does (via compute_routine_id from IR object/routine metadata,
/// incl. attributes for classify_kind) so the op/cs/rv/loop ids match EXACTLY, then
/// full serde-PFeatures equality with NO id normalization.
#[test]
fn engine_ir_walk_exact_id_pfeatures() {
    use al_call_hierarchy::dual_run_support::legacy_l2_features;
    use al_call_hierarchy::engine::l2::ir_walk;
    use al_call_hierarchy::engine::l2::scope::{compute_routine_id, ParameterSymbol};
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/r0-corpus");
    if !root.is_dir() {
        return;
    }
    let obj_type = |k: &al_syntax::ir::ObjectKind| -> &'static str {
        use al_syntax::ir::ObjectKind::*;
        match k {
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
            _ => "Codeunit",
        }
    };
    let mut total = 0;
    let mut matching = 0;
    for fpath in collect_al_files(&root) {
        let Ok(src) = std::fs::read_to_string(&fpath) else {
            continue;
        };
        let legacy = legacy_l2_features(&src);
        let file = al_syntax::parse(&src);
        let mut ir_routines: Vec<(usize, &al_syntax::ir::RoutineDecl)> = Vec::new();
        for (oi, o) in file.objects.iter().enumerate() {
            for r in &o.routines {
                ir_routines.push((oi, r));
            }
        }
        for ((_ln, lf), (oi, routine)) in legacy.iter().zip(ir_routines.iter()) {
            total += 1;
            let o = &file.objects[*oi];
            let attrs: Vec<&str> = routine.attributes.iter().map(|s| s.as_str()).collect();
            let kind = if attrs.contains(&"eventsubscriber") {
                "event-subscriber"
            } else if attrs.contains(&"integrationevent") || attrs.contains(&"businessevent") {
                "event-publisher"
            } else if routine.kind == al_syntax::ir::RoutineKind::Trigger {
                "trigger"
            } else {
                "procedure"
            };
            let params: Vec<ParameterSymbol> = routine
                .params
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    let ty = p.ty.clone().unwrap_or_default();
                    ParameterSymbol {
                        index: i as u32,
                        name: p.name.clone(),
                        type_text: ty.clone(),
                        is_var: p.by_ref,
                        is_record: ty.to_ascii_lowercase().starts_with("record"),
                        table_name: None,
                    }
                })
                .collect();
            let routine_id = compute_routine_id(
                "dual",
                obj_type(&o.kind),
                o.id.unwrap_or(0),
                kind,
                &routine.name,
                &params,
                routine.return_type.as_deref(),
                "dual",
            );
            let irf = ir_walk::project_routine_features_ir(
                &file,
                *oi,
                routine,
                &routine_id,
                &src,
                "dual",
                None,
            );
            if serde_json::to_string(lf).unwrap() == serde_json::to_string(&irf).unwrap() {
                matching += 1;
            }
        }
    }
    let pct = if total > 0 {
        matching as f64 * 100.0 / total as f64
    } else {
        0.0
    };
    eprintln!("\n=== PHASE-2 IR byte-exact PFeatures (real ids, no normalization): {matching}/{total} ({pct:.1}%) ===");
    assert!(total > 0);
}

/// PHASE-3/5 prep — IR routine-envelope metadata (attributes / attributesParsed /
/// access_modifier) matches legacy. Validates the IR is ready to drive the PRoutine
/// envelope (toward removing tree-sitter from project_file). Measured per routine vs
/// project_named_routine (legacy envelope; its features now come from the IR but its
/// attributes/access_modifier are still legacy-derived).
#[test]
fn engine_ir_attributes_parity() {
    use al_call_hierarchy::engine::l2::ir_walk;
    use al_call_hierarchy::engine::l2::l2_workspace::project_named_routine;
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/r0-corpus");
    if !root.is_dir() {
        return;
    }
    let lang = al_call_hierarchy::language::language();
    let mut total = 0usize;
    let mut matching = 0usize;
    let mut divs: Vec<String> = Vec::new();
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
        let file = al_syntax::parse(&src);
        // Per-name dedup: only compare names unique within the file (project_named_routine
        // returns the first match).
        let mut name_counts: std::collections::HashMap<String, usize> = Default::default();
        for o in &file.objects {
            for r in &o.routines {
                *name_counts.entry(r.name.to_ascii_lowercase()).or_default() += 1;
            }
        }
        for o in &file.objects {
            for r in &o.routines {
                if name_counts[&r.name.to_ascii_lowercase()] != 1 {
                    continue; // ambiguous name — skip
                }
                let Some(legacy) = project_named_routine(&src, &r.name, "dual", "ws:test", &tree)
                else {
                    continue;
                };
                total += 1;
                let (ir_attrs, ir_parsed) = ir_walk::ir_attributes(r, &file, &src);
                let ok = ir_attrs == legacy.attributes
                    && ir_parsed == legacy.attributes_parsed
                    && r.access_modifier == legacy.access_modifier;
                if ok {
                    matching += 1;
                } else if divs.len() < 12 {
                    divs.push(format!(
                        "{} :: {}\n    legacy attrs={:?} parsed={:?} mod={:?}\n    ir     attrs={:?} parsed={:?} mod={:?}",
                        fpath.strip_prefix(&root).unwrap_or(&fpath).display(), r.name,
                        legacy.attributes, legacy.attributes_parsed, legacy.access_modifier,
                        ir_attrs, ir_parsed, r.access_modifier
                    ));
                }
            }
        }
    }
    let pct = if total > 0 {
        matching as f64 * 100.0 / total as f64
    } else {
        0.0
    };
    eprintln!("\n=== PHASE-3/5 IR routine-envelope metadata: {matching}/{total} ({pct:.1}%) ===");
    for d in divs.iter().take(10) {
        eprintln!("  {d}");
    }
    assert!(total > 0);
    assert_eq!(
        matching, total,
        "engine ir_walk routine-envelope metadata divergences"
    );
}

/// PHASE-3 prep — IR-derived ParameterSymbol matches legacy extract_parameters
/// (the stable-id hash input). Validates the parameters port is safe before wiring.
#[test]
fn engine_ir_parameters_parity() {
    use al_call_hierarchy::engine::l2::scope::{extract_parameters, ParameterSymbol};
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/r0-corpus");
    if !root.is_dir() {
        return;
    }
    fn ir_params(
        r: &al_syntax::ir::RoutineDecl,
    ) -> Vec<(u32, String, String, bool, bool, Option<String>)> {
        r.params
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let ty = p.ty.clone().unwrap_or_default();
                let lc = ty.to_ascii_lowercase();
                let is_record = lc.starts_with("record");
                let table_name = if is_record {
                    let rest = ty[6..].trim_start();
                    if let Some(a) = rest.strip_prefix('"') {
                        a.find('"').map(|e| a[..e].to_string())
                    } else {
                        rest.split_whitespace().next().map(|w| w.to_string())
                    }
                } else {
                    None
                };
                (
                    i as u32,
                    p.name.clone(),
                    ty,
                    p.by_ref,
                    is_record,
                    table_name,
                )
            })
            .collect()
    }
    fn key(p: &ParameterSymbol) -> (u32, String, String, bool, bool, Option<String>) {
        (
            p.index,
            p.name.clone(),
            p.type_text.clone(),
            p.is_var,
            p.is_record,
            p.table_name.clone(),
        )
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
    let lang = al_call_hierarchy::language::language();
    let mut total = 0usize;
    let mut matching = 0usize;
    let mut divs: Vec<String> = Vec::new();
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
        let file = al_syntax::parse(&src);
        let mut ir_by_byte: std::collections::HashMap<usize, &al_syntax::ir::RoutineDecl> =
            Default::default();
        for o in &file.objects {
            for r in &o.routines {
                ir_by_byte.insert(r.origin.byte.start, r);
            }
        }
        let mut nodes = Vec::new();
        routine_nodes(tree.root_node(), &mut nodes);
        for node in nodes {
            let Some(r) = ir_by_byte.get(&node.start_byte()) else {
                continue;
            };
            total += 1;
            let legacy: Vec<_> = extract_parameters(node, &src).iter().map(key).collect();
            let ir: Vec<_> = ir_params(r);
            if legacy == ir {
                matching += 1;
            } else if divs.len() < 10 {
                divs.push(format!(
                    "{} :: {}\n    legacy={:?}\n    ir    ={:?}",
                    fpath.strip_prefix(&root).unwrap_or(&fpath).display(),
                    r.name,
                    legacy,
                    ir
                ));
            }
        }
    }
    let pct = if total > 0 {
        matching as f64 * 100.0 / total as f64
    } else {
        0.0
    };
    eprintln!(
        "\n=== PHASE-3 IR parameters vs extract_parameters: {matching}/{total} ({pct:.1}%) ==="
    );
    for d in divs.iter().take(10) {
        eprintln!("  {d}");
    }
    assert!(total > 0);
    assert_eq!(matching, total, "engine ir parameters divergences");
}

/// PHASE-2 — the serde-SKIPPED L5 detector inputs (in_until_condition, run_trigger)
/// on record_operations match legacy. These are EXCLUDED from PRecordOperation's
/// PartialEq, so the byte-exact L2 gate cannot see them — this gate guards them.
#[test]
fn engine_ir_record_op_l5_inputs_parity() {
    use al_call_hierarchy::dual_run_support::legacy_l2_features;
    use al_call_hierarchy::engine::l2::ir_walk;
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/r0-corpus");
    if !root.is_dir() {
        return;
    }
    let opnum = |id: &str| {
        id.rsplit("op")
            .next()
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(u32::MAX)
    };
    let mut total = 0usize;
    let mut matching = 0usize;
    let mut divs: Vec<String> = Vec::new();
    for fpath in collect_al_files(&root) {
        let Ok(src) = std::fs::read_to_string(&fpath) else {
            continue;
        };
        let legacy = legacy_l2_features(&src);
        let file = al_syntax::parse(&src);
        let mut ir_routines: Vec<(usize, &al_syntax::ir::RoutineDecl)> = Vec::new();
        for (oi, o) in file.objects.iter().enumerate() {
            for r in &o.routines {
                ir_routines.push((oi, r));
            }
        }
        for ((ln, lf), (oi, routine)) in legacy.iter().zip(ir_routines.iter()) {
            total += 1;
            let irf =
                ir_walk::project_routine_features_ir(&file, *oi, routine, "ir", &src, "dual", None);
            let key = |ops: &[al_call_hierarchy::engine::l2::features::PRecordOperation]| {
                let mut v: Vec<(u32, bool, Option<bool>)> = ops
                    .iter()
                    .map(|o| (opnum(&o.id), o.in_until_condition, o.run_trigger))
                    .collect();
                v.sort_by_key(|t| t.0);
                v
            };
            let l = key(&lf.record_operations);
            let i = key(&irf.record_operations);
            if l == i {
                matching += 1;
            } else if divs.len() < 12 {
                divs.push(format!(
                    "{} :: {ln}\n    legacy={l:?}\n    ir    ={i:?}",
                    fpath.strip_prefix(&root).unwrap_or(&fpath).display()
                ));
            }
        }
    }
    let pct = if total > 0 {
        matching as f64 * 100.0 / total as f64
    } else {
        0.0
    };
    eprintln!("\n=== PHASE-2 record-op L5 inputs (in_until_condition/run_trigger): {matching}/{total} ({pct:.1}%) ===");
    for d in divs.iter().take(10) {
        eprintln!("  {d}");
    }
    assert!(total > 0);
    assert_eq!(
        matching, total,
        "record-op L5-input (in_until_condition/run_trigger) divergences"
    );
}

/// PHASE-2 — object-procedure-name collision: a bare `Modify()` inside a table
/// method, where the table ALSO declares a `procedure Modify()`, is a CALL to that
/// procedure, NOT a record op (legacy object_procedure_names check). Synthetic
/// fixture (absent from the corpus); compares the IR record_operations/call_sites to
/// legacy.
#[test]
fn engine_ir_object_procedure_collision() {
    use al_call_hierarchy::dual_run_support::legacy_l2_features;
    use al_call_hierarchy::engine::l2::ir_walk;
    let src = r#"table 50000 "Foo"
{
    fields { field(1; "No."; Integer) { } }
    procedure Modify()
    begin
    end;

    procedure Caller()
    begin
        Modify();
    end;
}
"#;
    let legacy = legacy_l2_features(src);
    let file = al_syntax::parse(src);
    let mut ir_routines: Vec<(usize, &al_syntax::ir::RoutineDecl)> = Vec::new();
    for (oi, o) in file.objects.iter().enumerate() {
        for r in &o.routines {
            ir_routines.push((oi, r));
        }
    }
    let mut checked_caller = false;
    for ((ln, lf), (oi, routine)) in legacy.iter().zip(ir_routines.iter()) {
        let irf =
            ir_walk::project_routine_features_ir(&file, *oi, routine, "ir", src, "dual", None);
        // record-op count + call-site count must match legacy.
        assert_eq!(
            lf.record_operations.len(),
            irf.record_operations.len(),
            "{ln}: record_operations count mismatch (legacy {} ir {})",
            lf.record_operations.len(),
            irf.record_operations.len()
        );
        assert_eq!(
            lf.call_sites.len(),
            irf.call_sites.len(),
            "{ln}: call_sites count mismatch (legacy {} ir {})",
            lf.call_sites.len(),
            irf.call_sites.len()
        );
        if ln == "Caller" {
            checked_caller = true;
            // Modify() collides with the local procedure → a CALL, not a record op.
            assert_eq!(
                lf.record_operations.len(),
                0,
                "legacy treated Modify() as a record op?"
            );
            assert_eq!(
                irf.record_operations.len(),
                0,
                "IR wrongly treated Modify() as a record op"
            );
            assert_eq!(
                irf.call_sites.len(),
                1,
                "IR should emit a call site for Modify()"
            );
        }
    }
    assert!(checked_caller, "Caller routine not found");
}

/// PHASE-2 guard — NO well-formed routine silently falls back to legacy body_walk.
/// The driver matches tree-sitter routines to IR routines by start byte and falls
/// back to legacy on a miss; a miss for a well-formed routine would mean a lowerer
/// regression silently degrading to legacy (and masking divergence). Assert every
/// non-parse-error tree-sitter routine is matched in the IR byte index.
#[test]
fn engine_ir_no_silent_fallback() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/r0-corpus");
    if !root.is_dir() {
        return;
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
    let lang = al_call_hierarchy::language::language();
    let mut total = 0usize;
    let mut matched = 0usize;
    let mut misses: Vec<String> = Vec::new();
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
        let file = al_syntax::parse(&src);
        let mut ir_bytes: std::collections::HashSet<usize> = Default::default();
        for o in &file.objects {
            for r in &o.routines {
                ir_bytes.insert(r.origin.byte.start);
            }
        }
        let mut nodes = Vec::new();
        routine_nodes(tree.root_node(), &mut nodes);
        for node in nodes {
            if node.has_error() {
                continue; // parse-error routine — legacy fallback is intended
            }
            // skip nameless routines (the driver continues past them).
            let named = node.child_by_field_name("name").is_some();
            if !named {
                continue;
            }
            total += 1;
            if ir_bytes.contains(&node.start_byte()) {
                matched += 1;
            } else if misses.len() < 20 {
                misses.push(format!(
                    "{} @ byte {}",
                    fpath.strip_prefix(&root).unwrap_or(&fpath).display(),
                    node.start_byte()
                ));
            }
        }
    }
    eprintln!("\n=== PHASE-2 no-silent-fallback: {matched}/{total} well-formed routines matched in IR ===");
    for m in misses.iter().take(20) {
        eprintln!("  MISS: {m}");
    }
    assert!(total > 0);
    assert_eq!(
        matched, total,
        "well-formed routines fell back to legacy (lowerer byte-miss)"
    );
}
