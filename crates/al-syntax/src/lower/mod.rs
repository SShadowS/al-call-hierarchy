//! CST → IR lowering — the ONLY grammar-aware logic above the raw layer.
//!
//! Phase 1a (this step): outer structure — objects → routines → params / return
//! type / locals / globals. Statement/expression bodies (`RoutineDecl.body`) and
//! `temporary` detection are filled by the next Phase-1 step and validated against
//! the legacy walk under dual-run (spec §5). Unmodelled-but-present nodes are never
//! silently dropped — they surface as `SyntaxIssue` / IR `Unknown`.

use crate::ir::{
    AlFile, BinaryOp, Block, BlockId, BlockItem, CaseBranch, Expr, ExprId, ExprKind, Ir, Literal,
    ObjectDecl, ObjectKind, Origin, Param, ParseStatus, Point, RoutineDecl, RoutineKind, Stmt,
    StmtId, StmtKind, SyntaxIssue, UnaryOp, VarDecl,
};
use crate::raw::{FieldName, RawKind, RawNode};

/// Lower a parsed file root into the owned IR.
pub fn lower_file(root: RawNode, source: &str) -> AlFile {
    let parse_status = if root.has_error() {
        ParseStatus::Recovered
    } else {
        ParseStatus::Clean
    };
    let mut ir = Ir::new();
    let mut issues = Vec::new();
    let mut objects = Vec::new();
    collect_objects(root, source, &mut ir, &mut issues, &mut objects);
    AlFile { objects, ir, issues, parse_status }
}

/// Walk for top-level object declarations, descending namespaces and preproc
/// wrappers (which may enclose objects in BC 24+ / `#if` builds).
fn collect_objects(
    node: RawNode,
    source: &str,
    ir: &mut Ir,
    issues: &mut Vec<crate::ir::SyntaxIssue>,
    out: &mut Vec<ObjectDecl>,
) {
    for child in node.named_children() {
        match object_kind_of(child.kind()) {
            Some(kind) => out.push(lower_object(child, kind, source, ir, issues)),
            None => {
                // Descend containers that may hold objects (namespace, preproc).
                if child.kind() == RawKind::NamespaceDeclaration || is_preproc_wrapper(child) {
                    collect_objects(child, source, ir, issues, out);
                }
            }
        }
    }
}

/// A `preproc_conditional*` wrapper node (`#if`/`#else` region). The lowerer
/// descends BOTH branches (legacy indexes both for BC version-compat).
fn is_preproc_wrapper(n: RawNode) -> bool {
    n.kind_str().starts_with("preproc_conditional")
}

fn object_kind_of(k: RawKind) -> Option<ObjectKind> {
    use ObjectKind as O;
    Some(match k {
        RawKind::CodeunitDeclaration | RawKind::PreprocSplitDeclaration => O::Codeunit,
        RawKind::TableDeclaration => O::Table,
        RawKind::TableextensionDeclaration => O::TableExtension,
        RawKind::PageDeclaration => O::Page,
        RawKind::PageextensionDeclaration => O::PageExtension,
        RawKind::ReportDeclaration => O::Report,
        RawKind::ReportextensionDeclaration => O::ReportExtension,
        RawKind::QueryDeclaration => O::Query,
        RawKind::XmlportDeclaration => O::XmlPort,
        RawKind::EnumDeclaration => O::Enum,
        RawKind::EnumextensionDeclaration => O::EnumExtension,
        RawKind::InterfaceDeclaration => O::Interface,
        RawKind::ControladdinDeclaration => O::ControlAddIn,
        RawKind::EntitlementDeclaration => O::Entitlement,
        RawKind::PermissionsetDeclaration => O::PermissionSet,
        RawKind::PermissionsetextensionDeclaration => O::PermissionSetExtension,
        RawKind::ProfileDeclaration => O::Profile,
        _ => return None,
    })
}

fn lower_object(
    node: RawNode,
    kind: ObjectKind,
    source: &str,
    ir: &mut Ir,
    issues: &mut Vec<crate::ir::SyntaxIssue>,
) -> ObjectDecl {
    let id = node
        .field(FieldName::ObjectId)
        .and_then(|n| n.text(source).trim().parse::<i64>().ok());
    let name = node
        .field(FieldName::ObjectName)
        .map(|n| ident_text(n, source))
        .unwrap_or_default();

    // Routines: every procedure/trigger anywhere in the object subtree (incl. field
    // /action triggers nested in sections, and both #if/#else branches).
    let mut routine_nodes = Vec::new();
    collect_routines(node, &mut routine_nodes);
    let routines = routine_nodes
        .into_iter()
        .map(|r| lower_routine(r, source, ir, issues))
        .collect();

    // Object globals: var_sections under the declaration_body (not inside routines).
    let mut globals = Vec::new();
    if let Some(body) = node.field(FieldName::Body) {
        for member in body.named_children() {
            collect_globals(member, source, &mut globals);
        }
    }

    ObjectDecl { kind, id, name, routines, globals, origin: origin_of(node) }
}

/// DFS collecting `procedure` / `trigger_declaration` nodes. AL has no nested
/// routines, so we do not descend into a routine once found.
fn collect_routines<'t>(node: RawNode<'t>, out: &mut Vec<RawNode<'t>>) {
    for child in node.named_children() {
        match child.kind() {
            RawKind::Procedure | RawKind::TriggerDeclaration => out.push(child),
            _ => collect_routines(child, out),
        }
    }
}

/// Collect object-level var declarations, descending preproc wrappers (both
/// branches) but NOT routines/sections-with-their-own-scope.
fn collect_globals(node: RawNode, source: &str, out: &mut Vec<VarDecl>) {
    match node.kind() {
        RawKind::VarSection => extract_var_section(node, source, out),
        _ if is_preproc_wrapper(node) => {
            for c in node.named_children() {
                collect_globals(c, source, out);
            }
        }
        _ => {}
    }
}

fn lower_routine(
    node: RawNode,
    source: &str,
    ir: &mut Ir,
    issues: &mut Vec<SyntaxIssue>,
) -> RoutineDecl {
    let kind = if node.kind() == RawKind::TriggerDeclaration {
        RoutineKind::Trigger
    } else {
        RoutineKind::Procedure
    };
    let name = node
        .field(FieldName::Name)
        .map(|n| ident_text(n, source))
        .unwrap_or_default();

    let params = node
        .field(FieldName::Parameters)
        .map(|pl| {
            pl.named_children()
                .into_iter()
                .filter(|p| p.kind() == RawKind::Parameter)
                .map(|p| lower_param(p, source))
                .collect()
        })
        .unwrap_or_default();

    let return_type = node
        .field(FieldName::ReturnType)
        .map(|n| n.text(source).trim().to_string());

    // Locals: var_section child(ren) of the routine (+ preproc-wrapped).
    let mut locals = Vec::new();
    for child in node.named_children() {
        collect_globals(child, source, &mut locals);
    }

    let body = node
        .field(FieldName::Body)
        .filter(|b| b.kind() == RawKind::CodeBlock)
        .map(|cb| lower_code_block(cb, ir, issues, source));

    RoutineDecl { kind, name, params, return_type, locals, body, origin: origin_of(node) }
}

fn lower_param(node: RawNode, source: &str) -> Param {
    let by_ref = node.field(FieldName::Modifier).is_some();
    let name = node
        .field(FieldName::Name)
        .map(|n| ident_text(n, source))
        .unwrap_or_default();
    let ty = node
        .field(FieldName::Type)
        .map(|n| n.text(source).trim().to_string());
    Param { name, by_ref, ty, origin: origin_of(node) }
}

/// A `var_section` → its `var_body` → one `VarDecl` per declared name (`A, B: T`
/// yields two). `temporary` detection is refined in the parity step (false here).
fn extract_var_section(section: RawNode, source: &str, out: &mut Vec<VarDecl>) {
    let Some(body) = section.field(FieldName::Body) else {
        return;
    };
    for decl in body.named_children() {
        match decl.kind() {
            RawKind::VariableDeclaration => {
                let ty = decl
                    .field(FieldName::Type)
                    .map(|n| n.text(source).trim().to_string());
                let names = decl.children_by_field(FieldName::Name);
                if names.is_empty() {
                    // single unnamed-by-field fallback: skip (no name to record)
                    continue;
                }
                for nm in names {
                    out.push(VarDecl {
                        name: ident_text(nm, source),
                        ty: ty.clone(),
                        temporary: false,
                        origin: origin_of(decl),
                    });
                }
            }
            _ if is_preproc_wrapper(decl) => {
                for c in decl.named_children() {
                    if c.kind() == RawKind::VariableDeclaration {
                        // shallow: flatten preproc-wrapped declarations (both branches)
                        let ty = c
                            .field(FieldName::Type)
                            .map(|n| n.text(source).trim().to_string());
                        for nm in c.children_by_field(FieldName::Name) {
                            out.push(VarDecl {
                                name: ident_text(nm, source),
                                ty: ty.clone(),
                                temporary: false,
                                origin: origin_of(c),
                            });
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

// ---- body lowering (statements + expressions) ----
//
// First cut: preproc-wrapped statements are FLATTENED in document order (legacy
// recursively descends; the structured-vs-flat choice is settled by Phase 1
// dual-run). Unmodelled nodes become `Unknown` + a `SyntaxIssue` — never dropped.

/// `code_block` → its `statement_block` (v3) → a `Block`.
fn lower_code_block(cb: RawNode, ir: &mut Ir, issues: &mut Vec<SyntaxIssue>, source: &str) -> BlockId {
    let inner = cb.field(FieldName::Body).unwrap_or(cb);
    lower_stmt_seq(inner, origin_of(cb), ir, issues, source)
}

/// A branch position (then/else/loop body): a `code_block`, a bare `statement_block`,
/// or a single statement. Always normalized to a `Block`.
fn lower_branch(node: RawNode, ir: &mut Ir, issues: &mut Vec<SyntaxIssue>, source: &str) -> BlockId {
    match node.kind() {
        RawKind::CodeBlock => lower_code_block(node, ir, issues, source),
        RawKind::StatementBlock => lower_stmt_seq(node, origin_of(node), ir, issues, source),
        _ => {
            let mut items = Vec::new();
            lower_block_child(node, ir, issues, source, &mut items);
            ir.add_block(Block { items, origin: origin_of(node) })
        }
    }
}

fn lower_stmt_seq(
    container: RawNode,
    origin: Origin,
    ir: &mut Ir,
    issues: &mut Vec<SyntaxIssue>,
    source: &str,
) -> BlockId {
    let mut items = Vec::new();
    for child in container.named_children() {
        lower_block_child(child, ir, issues, source, &mut items);
    }
    ir.add_block(Block { items, origin })
}

fn lower_block_child(
    node: RawNode,
    ir: &mut Ir,
    issues: &mut Vec<SyntaxIssue>,
    source: &str,
    items: &mut Vec<BlockItem>,
) {
    if is_preproc_wrapper(node) {
        for c in node.named_children() {
            lower_block_child(c, ir, issues, source, items);
        }
        return;
    }
    if node.kind() == RawKind::EmptyStatement {
        return;
    }
    let sid = lower_stmt(node, ir, issues, source);
    items.push(BlockItem::Stmt(sid));
}

fn lower_stmt(node: RawNode, ir: &mut Ir, issues: &mut Vec<SyntaxIssue>, source: &str) -> StmtId {
    let origin = origin_of(node);
    let kind = match node.kind() {
        RawKind::AssignmentStatement => {
            let target = lower_opt_field(node, FieldName::Left, ir, issues, source);
            let value = lower_opt_field(node, FieldName::Right, ir, issues, source);
            StmtKind::Assignment { target, value }
        }
        RawKind::CallExpression | RawKind::MemberExpression => StmtKind::Call(lower_expr(node, ir, issues, source)),
        RawKind::IfStatement => StmtKind::If {
            cond: lower_opt_field(node, FieldName::Condition, ir, issues, source),
            then_block: lower_branch_field(node, FieldName::ThenBranch, ir, issues, source),
            else_block: node
                .field(FieldName::ElseBranch)
                .map(|b| lower_branch(b, ir, issues, source)),
        },
        RawKind::WhileStatement => StmtKind::While {
            cond: lower_opt_field(node, FieldName::Condition, ir, issues, source),
            body: lower_branch_field(node, FieldName::Body, ir, issues, source),
        },
        RawKind::RepeatStatement => StmtKind::Repeat {
            body: lower_branch_field(node, FieldName::Body, ir, issues, source),
            until: lower_opt_field(node, FieldName::Condition, ir, issues, source),
        },
        RawKind::ForStatement => {
            let down = node
                .field(FieldName::Direction)
                .map(|d| d.text(source).eq_ignore_ascii_case("downto"))
                .unwrap_or(false);
            StmtKind::For {
                var: lower_opt_field(node, FieldName::Variable, ir, issues, source),
                from: lower_opt_field(node, FieldName::Start, ir, issues, source),
                to: lower_opt_field(node, FieldName::End, ir, issues, source),
                down,
                body: lower_branch_field(node, FieldName::Body, ir, issues, source),
            }
        }
        RawKind::ForeachStatement => StmtKind::Foreach {
            var: lower_opt_field(node, FieldName::Variable, ir, issues, source),
            iterable: lower_opt_field(node, FieldName::Iterable, ir, issues, source),
            body: lower_branch_field(node, FieldName::Body, ir, issues, source),
        },
        RawKind::WithStatement => StmtKind::With {
            receiver: lower_opt_field(node, FieldName::Record, ir, issues, source),
            body: lower_branch_field(node, FieldName::Body, ir, issues, source),
        },
        RawKind::CaseStatement => {
            let scrutinee = lower_opt_field(node, FieldName::Expression, ir, issues, source);
            let (branches, else_block) = lower_case_body(node, ir, issues, source);
            StmtKind::Case { scrutinee, branches, else_block }
        }
        RawKind::AsserterrorStatement => {
            StmtKind::AssertError(lower_branch_field(node, FieldName::Body, ir, issues, source))
        }
        RawKind::ExitStatement => StmtKind::Exit(
            node.field(FieldName::ReturnValue)
                .map(|e| lower_expr(e, ir, issues, source)),
        ),
        RawKind::BreakStatement => StmtKind::Break,
        RawKind::ContinueStatement => StmtKind::Continue,
        RawKind::CodeBlock => StmtKind::Block(lower_code_block(node, ir, issues, source)),
        _ => {
            issues.push(SyntaxIssue {
                message: format!("unlowered statement `{}`", node.kind_str()),
                origin: origin.clone(),
            });
            StmtKind::Unknown
        }
    };
    ir.add_stmt(Stmt { kind, origin })
}

/// Lower `case_body` → (branches, else block).
fn lower_case_body(
    case_node: RawNode,
    ir: &mut Ir,
    issues: &mut Vec<SyntaxIssue>,
    source: &str,
) -> (Vec<CaseBranch>, Option<BlockId>) {
    let mut branches = Vec::new();
    let mut else_block = None;
    let Some(body) = case_node.field(FieldName::Body) else {
        return (branches, else_block);
    };
    for child in body.named_children() {
        match child.kind() {
            RawKind::CaseBranch => {
                let patterns = child
                    .children_by_field(FieldName::Pattern)
                    .into_iter()
                    .map(|p| lower_expr(p, ir, issues, source))
                    .collect();
                let body = lower_branch_field(child, FieldName::Body, ir, issues, source);
                branches.push(CaseBranch { patterns, body, origin: origin_of(child) });
            }
            RawKind::CaseElseBranch => {
                else_block = Some(lower_branch_field(child, FieldName::Body, ir, issues, source));
            }
            _ => {}
        }
    }
    (branches, else_block)
}

/// Lower a required-expression field; missing → `Unknown` placeholder (recorded).
fn lower_opt_field(
    node: RawNode,
    f: FieldName,
    ir: &mut Ir,
    issues: &mut Vec<SyntaxIssue>,
    source: &str,
) -> ExprId {
    match node.field(f) {
        Some(e) => lower_expr(e, ir, issues, source),
        None => {
            let origin = origin_of(node);
            issues.push(SyntaxIssue {
                message: format!("missing `{:?}` on `{}`", f, node.kind_str()),
                origin: origin.clone(),
            });
            ir.add_expr(Expr { kind: ExprKind::Unknown, origin })
        }
    }
}

fn lower_branch_field(
    node: RawNode,
    f: FieldName,
    ir: &mut Ir,
    issues: &mut Vec<SyntaxIssue>,
    source: &str,
) -> BlockId {
    match node.field(f) {
        Some(b) => lower_branch(b, ir, issues, source),
        None => ir.add_block(Block { items: Vec::new(), origin: origin_of(node) }),
    }
}

fn lower_expr(node: RawNode, ir: &mut Ir, issues: &mut Vec<SyntaxIssue>, source: &str) -> ExprId {
    let origin = origin_of(node);
    let kind = match node.kind() {
        RawKind::Identifier | RawKind::KeywordIdentifier => ExprKind::Identifier(node.text(source).to_string()),
        RawKind::QuotedIdentifier => ExprKind::QuotedIdentifier(ident_text(node, source)),
        RawKind::MemberExpression => {
            let object = lower_opt_field(node, FieldName::Object, ir, issues, source);
            let member = node
                .field(FieldName::Member)
                .map(|m| ident_text(m, source))
                .unwrap_or_default();
            ExprKind::Member { object, member }
        }
        RawKind::CallExpression => {
            let function = lower_opt_field(node, FieldName::Function, ir, issues, source);
            let args = node
                .field(FieldName::Arguments)
                .map(|al| {
                    al.named_children()
                        .into_iter()
                        .map(|a| lower_expr(a, ir, issues, source))
                        .collect()
                })
                .unwrap_or_default();
            ExprKind::Call { function, args }
        }
        RawKind::SubscriptExpression => ExprKind::Index {
            base: lower_opt_field(node, FieldName::Object, ir, issues, source),
            index: lower_opt_field(node, FieldName::Index, ir, issues, source),
        },
        RawKind::ParenthesizedExpression => {
            match node.named_children().into_iter().next() {
                Some(inner) => ExprKind::Parenthesized(lower_expr(inner, ir, issues, source)),
                None => ExprKind::Unknown,
            }
        }
        RawKind::UnaryExpression => ExprKind::Unary {
            op: unary_op(node, source),
            operand: lower_opt_field(node, FieldName::Operand, ir, issues, source),
        },
        RawKind::AdditiveExpression
        | RawKind::MultiplicativeExpression
        | RawKind::ComparisonExpression
        | RawKind::LogicalExpression => ExprKind::Binary {
            op: binary_op(node, source),
            lhs: lower_opt_field(node, FieldName::Left, ir, issues, source),
            rhs: lower_opt_field(node, FieldName::Right, ir, issues, source),
        },
        RawKind::RangeExpression => ExprKind::RangeExpr {
            start: lower_opt_field(node, FieldName::Left, ir, issues, source),
            end: lower_opt_field(node, FieldName::Right, ir, issues, source),
        },
        RawKind::QualifiedEnumValue => ExprKind::QualifiedEnum {
            enum_type: node.field(FieldName::EnumType).map(|e| e.text(source).to_string()).unwrap_or_default(),
            value: node.field(FieldName::Value).map(|v| ident_text(v, source)).unwrap_or_default(),
        },
        RawKind::DatabaseReference => ExprKind::DatabaseReference(node.text(source).to_string()),
        RawKind::Boolean => ExprKind::Literal(Literal::Bool(node.text(source).eq_ignore_ascii_case("true"))),
        RawKind::Integer => ExprKind::Literal(Literal::Int(node.text(source).to_string())),
        RawKind::Decimal => ExprKind::Literal(Literal::Decimal(node.text(source).to_string())),
        RawKind::StringLiteral | RawKind::VerbatimString => {
            ExprKind::Literal(Literal::Text(node.text(source).to_string()))
        }
        _ => {
            issues.push(SyntaxIssue {
                message: format!("unlowered expression `{}`", node.kind_str()),
                origin: origin.clone(),
            });
            ExprKind::Unknown
        }
    };
    ir.add_expr(Expr { kind, origin })
}

fn binary_op(node: RawNode, source: &str) -> BinaryOp {
    let t = node.field(FieldName::Operator).map(|o| o.text(source).to_string()).unwrap_or_default();
    match t.to_ascii_lowercase().as_str() {
        "+" => BinaryOp::Add,
        "-" => BinaryOp::Sub,
        "*" => BinaryOp::Mul,
        "/" => BinaryOp::Div,
        "div" => BinaryOp::IntDiv,
        "mod" => BinaryOp::Mod,
        "=" => BinaryOp::Eq,
        "<>" => BinaryOp::Ne,
        "<" => BinaryOp::Lt,
        "<=" => BinaryOp::Le,
        ">" => BinaryOp::Gt,
        ">=" => BinaryOp::Ge,
        "and" => BinaryOp::And,
        "or" => BinaryOp::Or,
        "xor" => BinaryOp::Xor,
        "in" => BinaryOp::In,
        _ => BinaryOp::Other,
    }
}

fn unary_op(node: RawNode, source: &str) -> UnaryOp {
    let t = node.field(FieldName::Operator).map(|o| o.text(source).to_string()).unwrap_or_default();
    match t.to_ascii_lowercase().as_str() {
        "not" => UnaryOp::Not,
        "-" => UnaryOp::Neg,
        _ => UnaryOp::Plus,
    }
}

/// Strip one layer of surrounding double quotes from a (quoted) identifier.
fn ident_text(n: RawNode, source: &str) -> String {
    let t = n.text(source);
    let bytes = t.as_bytes();
    if bytes.len() >= 2 && bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"' {
        t[1..t.len() - 1].to_string()
    } else {
        t.to_string()
    }
}

/// Build an [`Origin`] from a raw node (used pervasively by lowering).
pub(crate) fn origin_of(n: RawNode) -> Origin {
    let s = n.start_position();
    let e = n.end_position();
    Origin {
        kind_text: n.kind_str(),
        ts_id: n.id(),
        byte: n.byte_range(),
        start: Point { row: s.row as u32, column: s.column as u32 },
        end: Point { row: e.row as u32, column: e.column as u32 },
    }
}
