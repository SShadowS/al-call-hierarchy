//! Owned-IR L2 walk — the Phase-2 cut foundation.
//!
//! Re-expresses the SPINE of `body_walk` over the owned al-syntax IR rather than
//! the tree-sitter CST: the op/cs DFS numbering (the parity-critical heart), loop
//! tracking, `has_branching`, and the normalized CFN (`statement_tree`) as a REAL
//! `PCFNNode`. The op/cs visit order and the CFN shape were proven at 100% vs the
//! real engine in `tests/ir_dual_run.rs`; this module promotes that proven logic
//! from trace-comparisons to real engine-type production, the first piece of
//! `project_routine_features_ir`. Rich fields (callee/bindings/temp_state) land in
//! subsequent increments, which thread the full `Ctx` scope inputs.

use super::features::{
    PAnchor, PCFNNode, PConditionGuard, PConditionReference, PFieldAccess, PLoop, PVarAssignment,
};
use super::node_util::Utf16Cols;
use super::record_op::record_op_type;
use al_syntax::ir::{
    AlFile, BinaryOp, BlockId, BlockItem, ExprId, ExprKind, ObjectKind, Origin, RoutineDecl,
    StmtKind, UnaryOp, VarDecl,
};
use std::collections::{HashMap, HashSet};

/// Strip surrounding double-quotes from a quoted identifier's raw text.
fn strip_quotes(s: &str) -> String {
    if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}

/// LHS base name of an assignment target (legacy `lhs_identifier_of`, lowercased):
/// a bare identifier's name, or a member_expression's member name; else None.
fn lhs_base_name(file: &AlFile, target: ExprId) -> Option<String> {
    match &file.ir.expr(target).kind {
        ExprKind::Identifier(x) | ExprKind::QuotedIdentifier(x) => Some(x.to_ascii_lowercase()),
        ExprKind::Member { member, .. } => Some(member.to_ascii_lowercase()),
        _ => None,
    }
}

/// The spine result of the IR walk: the op/cs id maps (keyed by the call ExprId,
/// formatted `{routine_id}/op{N}` / `/cs{N}` in DFS order — exactly legacy's
/// numbering), plus `has_branching`. Consumed by the CFN builder.
pub struct IrSpine {
    pub op_id_by_expr: HashMap<ExprId, String>,
    pub cs_id_by_expr: HashMap<ExprId, String>,
    pub has_branching: bool,
    pub loop_count: u32,
    pub loops: Vec<PLoop>,
    pub field_accesses: Vec<PFieldAccess>,
    pub var_assignments: Vec<PVarAssignment>,
    pub condition_references: Vec<PConditionReference>,
    pub identifier_references: Vec<String>,
}

/// Per-routine record-receiver scope, mirroring the harness `ir_op_trace` setup
/// (validated at 100%): `rvars` = record-OP receiver set (Rec/xRec convention +
/// record globals/params/locals); `frvars` = field-access record set (record_var_names
/// semantics — Rec only for table methods).
struct Scope {
    rvars: HashSet<String>,
    frvars: HashSet<String>,
}

fn is_record_ty(v: &VarDecl) -> bool {
    v.ty.as_deref()
        .map(|t| t.to_ascii_lowercase().starts_with("record"))
        .unwrap_or(false)
}

/// Build the per-routine scope (record receiver sets) for `object`/`routine`.
fn build_scope(file: &AlFile, object_idx: usize, routine: &RoutineDecl) -> (Scope, bool) {
    let o = &file.objects[object_idx];
    let mut globals: HashSet<String> = o
        .globals
        .iter()
        .filter(|v| is_record_ty(v))
        .map(|v| v.name.to_ascii_lowercase())
        .collect();
    // Rec/xRec are record receivers by name convention regardless of object type.
    globals.insert("rec".to_string());
    globals.insert("xrec".to_string());
    let explicit_globals: HashSet<String> = o
        .globals
        .iter()
        .filter(|v| is_record_ty(v))
        .map(|v| v.name.to_ascii_lowercase())
        .collect();
    let table_method = matches!(o.kind, ObjectKind::Table | ObjectKind::TableExtension);

    let params_locals: Vec<String> = routine
        .params
        .iter()
        .filter(|p| {
            p.ty.as_deref()
                .map(|t| t.to_ascii_lowercase().starts_with("record"))
                .unwrap_or(false)
        })
        .map(|p| p.name.to_ascii_lowercase())
        .chain(
            routine
                .locals
                .iter()
                .filter(|v| is_record_ty(v))
                .map(|v| v.name.to_ascii_lowercase()),
        )
        .collect();

    let mut rvars = globals;
    rvars.extend(params_locals.iter().cloned());
    let mut frvars = explicit_globals;
    if table_method {
        frvars.insert("rec".to_string());
    }
    frvars.extend(params_locals);
    (Scope { rvars, frvars }, table_method)
}

/// Working state for the spine walk.
struct SpineCtx<'a> {
    file: &'a AlFile,
    scope: &'a Scope,
    routine_id: &'a str,
    source: &'a str,
    cols: &'a Utf16Cols<'a>,
    source_unit_id: &'a str,
    op_index: u32,
    cs_index: u32,
    loop_count: u32,
    has_branching: bool,
    implicit: Vec<bool>,
    op_id_by_expr: HashMap<ExprId, String>,
    cs_id_by_expr: HashMap<ExprId, String>,
    loops: Vec<PLoop>,
    field_accesses: Vec<PFieldAccess>,
    var_assignments: Vec<PVarAssignment>,
    condition_references: Vec<PConditionReference>,
    identifier_ref_set: HashSet<String>,
}

impl<'a> SpineCtx<'a> {
    /// PAnchor for an IR node origin (utf16 columns via `Utf16Cols`, exactly as the
    /// legacy `Ctx::anchor`; `syntax_kind` = the raw grammar kind).
    fn anchor(&self, origin: &Origin) -> PAnchor {
        PAnchor {
            source_unit_id: self.source_unit_id.to_string(),
            start_line: origin.start.row,
            start_column: self
                .cols
                .col(origin.start.row as usize, origin.start.column as usize),
            end_line: origin.end.row,
            end_column: self
                .cols
                .col(origin.end.row as usize, origin.end.column as usize),
            syntax_kind: origin.kind_text.to_string(),
        }
    }

    /// Exact raw source text of an IR node (via its byte span).
    fn raw(&self, origin: &Origin) -> &'a str {
        &self.source[origin.byte.clone()]
    }

    /// Literal text of an rhs value (legacy `literal_text_of`): boolean → lc,
    /// integer → raw, string → quote-stripped lc; otherwise None.
    fn lit_text(&self, eid: ExprId) -> Option<String> {
        use al_syntax::ir::Literal::*;
        match &self.file.ir.expr(eid).kind {
            ExprKind::Literal(Bool(b)) => Some(b.to_string()),
            ExprKind::Literal(Int(s)) => Some(s.clone()),
            ExprKind::Literal(Text(s)) => {
                let t = s
                    .strip_prefix('\'')
                    .and_then(|x| x.strip_suffix('\''))
                    .unwrap_or(s);
                Some(t.to_ascii_lowercase())
            }
            _ => None,
        }
    }

    /// Collect condition references (the `collect_idents` closure in legacy): a bare
    /// `identifier`, or a `member_expression`'s member identifier, anchored at the
    /// reference; recursing other expression shapes but NOT a member's object.
    fn collect_cond_idents(&mut self, eid: ExprId, kind: &str, stmt: &PAnchor) {
        use ExprKind::*;
        let e = self.file.ir.expr(eid);
        match &e.kind {
            Identifier(name) if e.origin.kind_text == "identifier" => {
                self.condition_references.push(PConditionReference {
                    identifier: name.to_ascii_lowercase(),
                    condition_kind: kind.to_string(),
                    statement_anchor: stmt.clone(),
                    reference_anchor: self.anchor(&e.origin),
                });
            }
            Member {
                member,
                member_origin,
                ..
            } => {
                if member_origin.kind_text == "identifier" {
                    self.condition_references.push(PConditionReference {
                        identifier: member.to_ascii_lowercase(),
                        condition_kind: kind.to_string(),
                        statement_anchor: stmt.clone(),
                        reference_anchor: self.anchor(member_origin),
                    });
                }
                // does NOT recurse into the object.
            }
            Call { function, args } => {
                let (function, args) = (*function, args.clone());
                self.collect_cond_idents(function, kind, stmt);
                for a in args {
                    self.collect_cond_idents(a, kind, stmt);
                }
            }
            Binary { lhs, rhs, .. } => {
                let (lhs, rhs) = (*lhs, *rhs);
                self.collect_cond_idents(lhs, kind, stmt);
                self.collect_cond_idents(rhs, kind, stmt);
            }
            Unary { operand, .. } => {
                let operand = *operand;
                self.collect_cond_idents(operand, kind, stmt);
            }
            Parenthesized(x) => {
                let x = *x;
                self.collect_cond_idents(x, kind, stmt);
            }
            Index { base, index } => {
                let (base, index) = (*base, *index);
                self.collect_cond_idents(base, kind, stmt);
                self.collect_cond_idents(index, kind, stmt);
            }
            QualifiedEnum { enum_type, .. } => {
                let enum_type = *enum_type;
                self.collect_cond_idents(enum_type, kind, stmt);
            }
            RangeExpr { start, end } => {
                let (start, end) = (*start, *end);
                self.collect_cond_idents(start, kind, stmt);
                self.collect_cond_idents(end, kind, stmt);
            }
            _ => {}
        }
    }

    /// Record a loop at its node (legacy creates the PLoop at the loop node, id
    /// `{routine}/loop{N}` with N = discovery order = `loops.len()`).
    fn enter_loop(&mut self, loop_type: &str, origin: &Origin) {
        let id = format!("{}/loop{}", self.routine_id, self.loops.len());
        let source_anchor = self.anchor(origin);
        self.loops.push(PLoop {
            id,
            loop_type: loop_type.to_string(),
            source_anchor,
        });
        self.loop_count += 1;
    }

    fn walk_block(&mut self, bid: BlockId) {
        for item in &self.file.ir.block(bid).items {
            match item {
                BlockItem::Stmt(s) => self.walk_stmt(*s),
                BlockItem::Preproc(g) => {
                    for b in &g.branches {
                        self.walk_block(*b);
                    }
                }
            }
        }
    }

    fn walk_stmt(&mut self, sid: al_syntax::ir::StmtId) {
        use StmtKind::*;
        let st = self.file.ir.stmt(sid);
        match &st.kind {
            Assignment { target, value } => {
                // PVarAssignment: lhs base name (identifier or member name), optional
                // literal rhs, anchored on the assignment statement.
                if let Some(lhs_name) = lhs_base_name(self.file, *target) {
                    let rhs_identifier = match (
                        &self.file.ir.expr(*target).kind,
                        &self.file.ir.expr(*value).kind,
                    ) {
                        (ExprKind::Identifier(_), ExprKind::Identifier(v)) => {
                            Some(v.to_ascii_lowercase())
                        }
                        _ => None,
                    };
                    self.var_assignments.push(PVarAssignment {
                        lhs_name,
                        rhs_literal_value: self.lit_text(*value),
                        source_anchor: self.anchor(&st.origin),
                        rhs_identifier,
                    });
                }
                self.walk_expr(*target);
                self.walk_expr(*value);
            }
            Call(x) => self.walk_expr(*x),
            If {
                cond,
                then_block,
                else_block,
            } => {
                self.has_branching = true;
                let sa = self.anchor(&st.origin);
                self.collect_cond_idents(*cond, "if", &sa);
                self.walk_expr(*cond);
                self.walk_block(*then_block);
                if let Some(b) = else_block {
                    self.walk_block(*b);
                }
            }
            Case {
                scrutinee,
                branches,
                else_block,
            } => {
                self.has_branching = true;
                let sa = self.anchor(&st.origin);
                self.collect_cond_idents(*scrutinee, "case", &sa);
                self.walk_expr(*scrutinee);
                for br in branches {
                    for p in &br.patterns {
                        self.walk_expr(*p);
                    }
                    self.walk_block(br.body);
                }
                if let Some(b) = else_block {
                    self.walk_block(*b);
                }
            }
            While { cond, body } => {
                let sa = self.anchor(&st.origin);
                self.collect_cond_idents(*cond, "while", &sa);
                self.enter_loop("while", &st.origin);
                self.walk_expr(*cond);
                self.walk_block(*body);
            }
            Repeat { body, until } => {
                let sa = self.anchor(&st.origin);
                self.collect_cond_idents(*until, "repeat-until", &sa);
                self.enter_loop("repeat", &st.origin);
                self.walk_block(*body);
                self.walk_expr(*until);
            }
            For {
                var,
                from,
                to,
                body,
                ..
            } => {
                self.enter_loop("for", &st.origin);
                self.walk_expr(*var);
                self.walk_expr(*from);
                self.walk_expr(*to);
                self.walk_block(*body);
            }
            Foreach {
                var,
                iterable,
                body,
            } => {
                self.enter_loop("foreach", &st.origin);
                self.walk_expr(*var);
                self.walk_expr(*iterable);
                self.walk_block(*body);
            }
            With { receiver, body } => {
                self.walk_expr(*receiver);
                let is_rec = match &self.file.ir.expr(*receiver).kind {
                    ExprKind::Identifier(x) | ExprKind::QuotedIdentifier(x) => {
                        self.scope.rvars.contains(&x.to_ascii_lowercase())
                    }
                    _ => false,
                };
                self.implicit.push(is_rec);
                self.walk_block(*body);
                self.implicit.pop();
            }
            Try { body, catch_block } => {
                self.has_branching = true;
                self.walk_block(*body);
                if let Some(c) = catch_block {
                    self.walk_block(*c);
                }
            }
            AssertError(body) => self.walk_block(*body),
            Exit(x) => {
                if let Some(x) = x {
                    self.walk_expr(*x);
                }
            }
            Block(b) => self.walk_block(*b),
            Break | Continue | Unknown => {}
        }
    }

    fn walk_expr(&mut self, eid: ExprId) {
        use ExprKind::*;
        let e = self.file.ir.expr(eid);
        if let Call { function, args } = &e.kind {
            let (function, args) = (*function, args.clone());
            let fe = self.file.ir.expr(function);
            let is_record_op = match &fe.kind {
                Member { object, member, .. } => {
                    let recv = match &self.file.ir.expr(*object).kind {
                        Identifier(x) | QuotedIdentifier(x) => Some(x.to_ascii_lowercase()),
                        _ => None,
                    };
                    recv.map(|r| self.scope.rvars.contains(&r)).unwrap_or(false)
                        && record_op_type(&member.to_ascii_lowercase()).is_some()
                }
                Identifier(m) | QuotedIdentifier(m) => {
                    self.implicit.last().copied().unwrap_or(false)
                        && record_op_type(&m.to_ascii_lowercase()).is_some()
                }
                _ => false,
            };
            let fname = match &fe.kind {
                Identifier(m) | QuotedIdentifier(m) => Some(m.to_ascii_lowercase()),
                _ => None,
            };
            let is_commit = fname.as_deref() == Some("commit");
            let is_error = fname.as_deref() == Some("error");
            if is_record_op || is_commit || is_error {
                // Error advances the op counter but is NOT mapped (legacy
                // op_id_by_node_id omits error — it renders as its cs "error" leaf).
                if !is_error {
                    self.op_id_by_expr
                        .insert(eid, format!("{}/op{}", self.routine_id, self.op_index));
                }
                self.op_index += 1;
            }
            if !is_record_op && !is_commit {
                self.cs_id_by_expr
                    .insert(eid, format!("{}/cs{}", self.routine_id, self.cs_index));
                self.cs_index += 1;
            }
            // Recurse: a member-call RECEIVER is a value ref; a bare callee name is not.
            match &self.file.ir.expr(function).kind {
                Member { object, .. } => {
                    let object = *object;
                    self.walk_expr(object);
                }
                Identifier(_) | QuotedIdentifier(_) => {}
                _ => self.walk_expr(function),
            }
            for a in args {
                self.walk_expr(a);
            }
            return;
        }
        match &e.kind {
            Member {
                object,
                member,
                member_origin,
            } => {
                // Value-position `X.Field` where X is a record var → field access,
                // anchored on the member_expression. Enum-scope refs go through the
                // QualifiedEnum arm (object recursed directly), never here.
                if let Identifier(x) | QuotedIdentifier(x) = &self.file.ir.expr(*object).kind {
                    if self.scope.frvars.contains(&x.to_ascii_lowercase()) {
                        let record_variable_name =
                            self.raw(&self.file.ir.expr(*object).origin).to_string();
                        let field_name = if member_origin.kind_text == "quoted_identifier" {
                            strip_quotes(member)
                        } else {
                            member.clone()
                        };
                        let source_anchor = self.anchor(&e.origin);
                        self.field_accesses.push(PFieldAccess {
                            record_variable_name,
                            field_name,
                            source_anchor,
                        });
                    }
                }
                let object = *object;
                self.walk_expr(object);
            }
            QualifiedEnum { enum_type, .. } => {
                let enum_type = *enum_type;
                match &self.file.ir.expr(enum_type).kind {
                    Member { object, .. } => {
                        let object = *object;
                        self.walk_expr(object);
                    }
                    _ => self.walk_expr(enum_type),
                }
            }
            Binary { lhs, rhs, .. } => {
                let (lhs, rhs) = (*lhs, *rhs);
                self.walk_expr(lhs);
                self.walk_expr(rhs);
            }
            Unary { operand, .. } => {
                let operand = *operand;
                self.walk_expr(operand);
            }
            Parenthesized(x) => {
                let x = *x;
                self.walk_expr(x);
            }
            Index { base, index } => {
                let (base, index) = (*base, *index);
                self.walk_expr(base);
                self.walk_expr(index);
            }
            RangeExpr { start, end } => {
                let (start, end) = (*start, *end);
                self.walk_expr(start);
                self.walk_expr(end);
            }
            // Value-reference identifier (lc, deduped) — legacy counts only plain
            // `identifier` nodes, NOT keyword_identifier/quoted_identifier.
            Identifier(name) => {
                if e.origin.kind_text == "identifier" {
                    self.identifier_ref_set.insert(name.to_ascii_lowercase());
                }
            }
            // `Keyword::Name` (database_reference): the object-type keyword is excluded,
            // but an UNQUOTED table_name identifier is a value ref.
            DatabaseReference(text) => {
                if let Some(last) = text.rsplit("::").next() {
                    let t = last.trim();
                    if !t.starts_with('"') {
                        self.identifier_ref_set.insert(t.to_ascii_lowercase());
                    }
                }
            }
            _ => {}
        }
    }
}

/// Walk the routine body, producing the op/cs id maps + has_branching + the
/// anchored `loops` and `field_accesses`.
#[allow(clippy::too_many_arguments)]
pub fn walk_spine(
    file: &AlFile,
    object_idx: usize,
    routine: &RoutineDecl,
    routine_id: &str,
    source: &str,
    cols: &Utf16Cols,
    source_unit_id: &str,
) -> IrSpine {
    let (scope, table_method) = build_scope(file, object_idx, routine);
    let mut ctx = SpineCtx {
        file,
        scope: &scope,
        routine_id,
        source,
        cols,
        source_unit_id,
        op_index: 0,
        cs_index: 0,
        loop_count: 0,
        has_branching: false,
        implicit: vec![table_method],
        op_id_by_expr: HashMap::new(),
        cs_id_by_expr: HashMap::new(),
        loops: Vec::new(),
        field_accesses: Vec::new(),
        var_assignments: Vec::new(),
        condition_references: Vec::new(),
        identifier_ref_set: HashSet::new(),
    };
    if let Some(b) = routine.body {
        ctx.walk_block(b);
    }
    // Post-pass ordering to match legacy: var_assignments by source anchor,
    // condition_references by reference anchor, identifier_references sorted+deduped.
    let mut var_assignments = ctx.var_assignments;
    var_assignments.sort_by(|a, b| {
        (a.source_anchor.start_line, a.source_anchor.start_column)
            .cmp(&(b.source_anchor.start_line, b.source_anchor.start_column))
    });
    let mut condition_references = ctx.condition_references;
    condition_references.sort_by(|a, b| {
        (
            a.reference_anchor.start_line,
            a.reference_anchor.start_column,
        )
            .cmp(&(
                b.reference_anchor.start_line,
                b.reference_anchor.start_column,
            ))
    });
    let mut identifier_references: Vec<String> = ctx.identifier_ref_set.into_iter().collect();
    identifier_references.sort();
    IrSpine {
        op_id_by_expr: ctx.op_id_by_expr,
        cs_id_by_expr: ctx.cs_id_by_expr,
        has_branching: ctx.has_branching,
        loop_count: ctx.loop_count,
        loops: ctx.loops,
        field_accesses: ctx.field_accesses,
        var_assignments,
        condition_references,
        identifier_references,
    }
}

// ---- CFN (statement_tree) as a real PCFNNode ----

fn cfn_node(kind: &str) -> PCFNNode {
    PCFNNode {
        kind: kind.to_string(),
        operation_id: None,
        callsite_id: None,
        condition_guard: None,
        condition_leaves: None,
        children: None,
        else_children: None,
        is_case_else: false,
        source_range: None,
    }
}

/// Builds the real `PCFNNode` statement_tree from the IR + the op/cs id maps.
pub struct IrCfn<'a> {
    pub file: &'a AlFile,
    pub spine: &'a IrSpine,
}

impl<'a> IrCfn<'a> {
    fn ir(&self) -> &al_syntax::ir::Ir {
        &self.file.ir
    }

    fn block_items(&self, bid: BlockId) -> Vec<PCFNNode> {
        let mut out = Vec::new();
        for item in &self.file.ir.block(bid).items {
            match item {
                BlockItem::Stmt(s) => {
                    if let Some(c) = self.build_statement(*s) {
                        out.push(c);
                    }
                }
                BlockItem::Preproc(g) => {
                    for b in &g.branches {
                        out.extend(self.block_items(*b));
                    }
                }
            }
        }
        out
    }

    pub fn build_block(&self, bid: BlockId) -> PCFNNode {
        let mut n = cfn_node("block");
        n.children = Some(self.block_items(bid));
        n
    }

    fn is_error_fn(&self, function: ExprId) -> bool {
        use ExprKind::*;
        matches!(&self.ir().expr(function).kind, Identifier(m) | QuotedIdentifier(m) if m.eq_ignore_ascii_case("error"))
    }

    fn harvest_receiver(&self, function: ExprId, out: &mut Vec<PCFNNode>) {
        use ExprKind::*;
        if let Member { object, .. } = &self.ir().expr(function).kind {
            if matches!(&self.ir().expr(*object).kind, Call { .. } | Member { .. }) {
                self.harvest(*object, out);
            }
        }
    }

    fn harvest(&self, eid: ExprId, out: &mut Vec<PCFNNode>) {
        use ExprKind::*;
        let e = self.ir().expr(eid);
        match &e.kind {
            Call { function, args } => {
                if let Some(op) = self.spine.op_id_by_expr.get(&eid) {
                    let mut inner = Vec::new();
                    for a in args {
                        self.harvest(*a, &mut inner);
                    }
                    self.harvest_receiver(*function, out);
                    let mut leaf = cfn_node("op");
                    leaf.operation_id = Some(op.clone());
                    if !inner.is_empty() {
                        leaf.condition_leaves = Some(inner);
                    }
                    out.push(leaf);
                    return;
                }
                if let Some(cs) = self.spine.cs_id_by_expr.get(&eid) {
                    let mut inner = Vec::new();
                    for a in args {
                        self.harvest(*a, &mut inner);
                    }
                    self.harvest_receiver(*function, out);
                    let mut leaf = cfn_node(if self.is_error_fn(*function) {
                        "error"
                    } else {
                        "call"
                    });
                    leaf.callsite_id = Some(cs.clone());
                    if !inner.is_empty() {
                        leaf.condition_leaves = Some(inner);
                    }
                    out.push(leaf);
                    return;
                }
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

    fn build_stmt_call(&self, eid: ExprId) -> PCFNNode {
        use ExprKind::*;
        let e = self.ir().expr(eid);
        let Call { function, args } = &e.kind else {
            return cfn_node("other");
        };
        let mut pre = Vec::new();
        self.harvest_receiver(*function, &mut pre);
        for a in args {
            self.harvest(*a, &mut pre);
        }
        let mut leaf = if let Some(op) = self.spine.op_id_by_expr.get(&eid) {
            let mut l = cfn_node("op");
            l.operation_id = Some(op.clone());
            l
        } else if let Some(cs) = self.spine.cs_id_by_expr.get(&eid) {
            let mut l = cfn_node(if self.is_error_fn(*function) {
                "error"
            } else {
                "call"
            });
            l.callsite_id = Some(cs.clone());
            l
        } else {
            cfn_node("other")
        };
        if !pre.is_empty() {
            leaf.condition_leaves = Some(pre);
        }
        leaf
    }

    fn simple_guard(&self, cond: ExprId) -> Option<PConditionGuard> {
        use ExprKind::*;
        let e = self.ir().expr(cond);
        match &e.kind {
            Identifier(n) if e.origin.kind_text == "identifier" => Some(PConditionGuard {
                identifier: n.to_ascii_lowercase(),
                polarity: "positive".to_string(),
            }),
            Unary {
                op: UnaryOp::Not,
                operand,
            } => match &self.ir().expr(*operand).kind {
                Identifier(n) if self.ir().expr(*operand).origin.kind_text == "identifier" => {
                    Some(PConditionGuard {
                        identifier: n.to_ascii_lowercase(),
                        polarity: "negative".to_string(),
                    })
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
                        .find_map(|s| match &self.ir().expr(s).kind {
                            Identifier(n) if self.ir().expr(s).origin.kind_text == "identifier" => {
                                Some(n.to_ascii_lowercase())
                            }
                            _ => None,
                        });
                let false_side = [*lhs, *rhs].into_iter().any(|s| {
                    matches!(
                        &self.ir().expr(s).kind,
                        Literal(al_syntax::ir::Literal::Bool(false))
                    )
                });
                match (id_side, false_side) {
                    (Some(id), true) => Some(PConditionGuard {
                        identifier: id,
                        polarity: "negative".to_string(),
                    }),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    fn harvest_vec(&self, eid: ExprId) -> Vec<PCFNNode> {
        let mut v = Vec::new();
        self.harvest(eid, &mut v);
        v
    }

    fn build_statement(&self, sid: al_syntax::ir::StmtId) -> Option<PCFNNode> {
        use StmtKind::*;
        let st = self.ir().stmt(sid);
        let some_if = |v: Vec<PCFNNode>| if v.is_empty() { None } else { Some(v) };
        Some(match &st.kind {
            Call(e) => self.build_stmt_call(*e),
            If {
                cond,
                then_block,
                else_block,
            } => {
                let mut n = cfn_node("if");
                n.children = Some(vec![self.build_block(*then_block)]);
                n.else_children = else_block.map(|b| vec![self.build_block(b)]);
                n.condition_leaves = some_if(self.harvest_vec(*cond));
                n.condition_guard = self.simple_guard(*cond);
                n
            }
            Case {
                scrutinee,
                branches,
                else_block,
            } => {
                let mut branch_cfns = Vec::new();
                for br in branches {
                    let mut b = cfn_node("case-branch");
                    b.children = Some(vec![self.build_block(br.body)]);
                    branch_cfns.push(b);
                }
                if let Some(eb) = else_block {
                    let mut b = cfn_node("case-branch");
                    b.is_case_else = true;
                    b.children = Some(vec![self.build_block(*eb)]);
                    branch_cfns.push(b);
                }
                let mut n = cfn_node("case");
                n.children = Some(branch_cfns);
                n.condition_leaves = some_if(self.harvest_vec(*scrutinee));
                n
            }
            While { cond, body } => {
                let mut n = cfn_node("while");
                n.children = Some(vec![self.build_block(*body)]);
                n.condition_leaves = some_if(self.harvest_vec(*cond));
                n
            }
            For { from, to, body, .. } => {
                let mut n = cfn_node("for");
                n.children = Some(vec![self.build_block(*body)]);
                let mut leaves = self.harvest_vec(*from);
                leaves.extend(self.harvest_vec(*to));
                n.condition_leaves = some_if(leaves);
                n
            }
            Foreach { iterable, body, .. } => {
                let mut n = cfn_node("foreach");
                n.children = Some(vec![self.build_block(*body)]);
                n.condition_leaves = some_if(self.harvest_vec(*iterable));
                n
            }
            Repeat { body, until } => {
                let mut n = cfn_node("repeat");
                n.children = Some(self.block_items(*body));
                n.condition_leaves = some_if(self.harvest_vec(*until));
                n
            }
            Try { .. } => {
                let mut n = cfn_node("try");
                n.children = Some(vec![]);
                n
            }
            Exit(x) => {
                let mut n = cfn_node("exit");
                n.condition_leaves = x.map(|e| self.harvest_vec(e)).and_then(some_if);
                n
            }
            With { body, .. } | AssertError(body) => {
                let mut n = cfn_node("other");
                n.children = Some(vec![self.build_block(*body)]);
                n
            }
            Assignment { target, value } => {
                let mut leaves = self.harvest_vec(*target);
                leaves.extend(self.harvest_vec(*value));
                let mut n = cfn_node("other");
                n.condition_leaves = some_if(leaves);
                n
            }
            Block(b) => self.build_block(*b),
            Break | Continue | Unknown => cfn_node("other"),
        })
    }
}

/// The slice of `PFeatures` the IR walk currently produces as REAL engine types.
/// Grows toward full `PFeatures` as the rich call_site/record_operation payloads
/// are threaded in subsequent increments.
pub struct IrPartialFeatures {
    pub statement_tree: Option<PCFNNode>,
    pub has_branching: bool,
    pub nesting_depth: u32,
    pub loops: Vec<PLoop>,
    pub field_accesses: Vec<PFieldAccess>,
    pub var_assignments: Vec<PVarAssignment>,
    pub condition_references: Vec<PConditionReference>,
    pub identifier_references: Vec<String>,
}

/// Build the validated slice of `PFeatures` from the owned IR for one routine.
pub fn routine_features_partial(
    file: &AlFile,
    object_idx: usize,
    routine: &RoutineDecl,
    routine_id: &str,
    source: &str,
    source_unit_id: &str,
) -> IrPartialFeatures {
    let cols = Utf16Cols::new(source);
    let spine = walk_spine(
        file,
        object_idx,
        routine,
        routine_id,
        source,
        &cols,
        source_unit_id,
    );
    let statement_tree = routine.body.map(|b| {
        let cfn = IrCfn {
            file,
            spine: &spine,
        };
        cfn.build_block(b)
    });
    let nesting_depth = super::compute_nesting_depth(&spine.loops);
    IrPartialFeatures {
        statement_tree,
        has_branching: spine.has_branching,
        nesting_depth,
        loops: spine.loops,
        field_accesses: spine.field_accesses,
        var_assignments: spine.var_assignments,
        condition_references: spine.condition_references,
        identifier_references: spine.identifier_references,
    }
}
