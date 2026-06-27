//! IR expressions. Names/literal text are stored inline so the IR is
//! self-contained (no source slice needed to read it); the engine interns as it
//! consumes. `Unknown` carries no payload — its `Origin` + a `SyntaxIssue` record
//! the failure (never a silent drop).

use super::{ExprId, Origin};

pub struct Expr {
    pub kind: ExprKind,
    pub origin: Origin,
}

pub enum ExprKind {
    Identifier(String),
    QuotedIdentifier(String),
    /// `object.member`
    Member { object: ExprId, member: String },
    /// `function(args...)`
    Call { function: ExprId, args: Vec<ExprId> },
    /// `base[index]` (subscript)
    Index { base: ExprId, index: ExprId },
    Literal(Literal),
    Unary { op: UnaryOp, operand: ExprId },
    Binary { op: BinaryOp, lhs: ExprId, rhs: ExprId },
    Parenthesized(ExprId),
    /// `Enum::Value` — `enum_type` is lowered (it can be a `member_expression`
    /// like `Rec.Status::Open`), `value` is the member text.
    QualifiedEnum { enum_type: ExprId, value: String },
    /// `Database::"Customer"` and similar object references.
    DatabaseReference(String),
    /// `a..b`
    RangeExpr { start: ExprId, end: ExprId },
    /// A syntactically present expression the lowerer does not yet model. The kind
    /// is preserved via `Origin.kind_text`; a `SyntaxIssue` is recorded.
    Unknown,
}

pub enum Literal {
    Int(String),
    Decimal(String),
    Bool(bool),
    /// String / verbatim string content (raw text, quotes included).
    Text(String),
    Date(String),
    DateTime(String),
    Time(String),
    /// Any other literal kind, raw text preserved.
    Other(String),
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum UnaryOp {
    Not,
    Neg,
    Plus,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    /// integer division `DIV`
    IntDiv,
    /// modulo `MOD`
    Mod,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    Xor,
    /// `IN` membership
    In,
    /// anything not in the set above, operator text preserved on the node origin.
    Other,
}
