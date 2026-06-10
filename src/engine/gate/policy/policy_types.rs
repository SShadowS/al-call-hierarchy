//! Policy types — port of al-sem `src/policy/policy-types.ts`.
//!
//! The compiled `Predicate` AST + `Rule` / `PolicyDoc` + the per-rule run
//! summary. The AST is serialized (insertion-order) by `policy explain`'s
//! "Normalized AST" block via [`predicate_to_json`].

/// Predicate operator. al-sem `PredicateOperator`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PredicateOperator {
    Eq,
    In,
    Glob,
    GlobIn,
}

impl PredicateOperator {
    pub fn as_str(&self) -> &'static str {
        match self {
            PredicateOperator::Eq => "==",
            PredicateOperator::In => "in",
            PredicateOperator::Glob => "glob",
            PredicateOperator::GlobIn => "glob-in",
        }
    }
}

/// A predicate value: either a single string (`==`/`glob`) or a list (`in`/`glob-in`).
/// Mirrors al-sem's `value: unknown` which is always a `string` or `string[]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PredicateValue {
    Str(String),
    List(Vec<String>),
}

/// The compiled predicate AST. al-sem `Predicate`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Predicate {
    Field {
        field: String,
        operator: PredicateOperator,
        value: PredicateValue,
    },
    All {
        children: Vec<Predicate>,
    },
    Any {
        children: Vec<Predicate>,
    },
    Not {
        child: Box<Predicate>,
    },
}

/// Policy severity (5-level). al-sem `PolicySeverity`.
pub const ALLOWED_SEVERITIES: &[&str] = &["critical", "high", "medium", "low", "info"];
/// al-sem `UnknownPolicy`.
pub const ALLOWED_UNKNOWN: &[&str] = &["fail-open", "fail-closed"];
/// al-sem `CoveragePolicy`.
pub const ALLOWED_COVERAGE: &[&str] = &["complete", "partial", "any"];
/// al-sem `FactOriginFilter`.
pub const ALLOWED_FACTS: &[&str] = &["direct", "inherited", "any"];

/// A compiled rule. al-sem `Rule`.
#[derive(Debug, Clone)]
pub struct Rule {
    pub id: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub message: Option<String>,
    pub severity: String,
    pub when: Predicate,
    pub except: Option<Predicate>,
    pub require_coverage: Option<String>,
    pub on_unknown: Option<String>,
    pub facts: Option<String>,
}

/// Policy defaults block. al-sem `PolicyDoc.defaults`.
#[derive(Debug, Clone, Default)]
pub struct PolicyDefaults {
    pub on_unknown: Option<String>,
    pub require_coverage: Option<String>,
}

/// A fully validated policy document. al-sem `PolicyDoc`.
#[derive(Debug, Clone)]
pub struct PolicyDoc {
    pub version: i64,
    pub description: Option<String>,
    pub defaults: Option<PolicyDefaults>,
    pub rules: Vec<Rule>,
}

/// Per-rule run counters. al-sem `RuleRunSummary` (the subset the engine emits —
/// no `traces`; `errors` is skip-if-none).
#[derive(Debug, Clone)]
pub struct RuleRunSummary {
    pub rule_id: String,
    pub routines_evaluated: usize,
    pub routines_matched: usize,
    pub routines_skipped_coverage: usize,
    pub routines_skipped_unknown: usize,
    pub routines_passed: usize,
    pub findings_emitted: usize,
    pub errors: Option<Vec<String>>,
}

// ---------------------------------------------------------------------------
// Predicate AST → JSON (insertion-order) for `policy explain`.
//
// al-sem renders `JSON.stringify(rule.when, undefined, 2)`. The compiled AST node
// key order is the construction order:
//   field: { kind, field, operator, value }
//   all:   { kind, children }
//   any:   { kind, children }
//   not:   { kind, child }
// We build a tiny ordered JSON value tree and serialize with 2-space indent.
// ---------------------------------------------------------------------------

/// A minimal insertion-order JSON value tree (mirrors the events serializer's `Jv`).
enum AstJv {
    Str(String),
    Obj(Vec<(&'static str, AstJv)>),
    Arr(Vec<AstJv>),
}

fn predicate_to_jv(p: &Predicate) -> AstJv {
    match p {
        Predicate::Field {
            field,
            operator,
            value,
        } => {
            let value_jv = match value {
                PredicateValue::Str(s) => AstJv::Str(s.clone()),
                PredicateValue::List(xs) => {
                    AstJv::Arr(xs.iter().map(|x| AstJv::Str(x.clone())).collect())
                }
            };
            AstJv::Obj(vec![
                ("kind", AstJv::Str("field".to_string())),
                ("field", AstJv::Str(field.clone())),
                ("operator", AstJv::Str(operator.as_str().to_string())),
                ("value", value_jv),
            ])
        }
        Predicate::All { children } => AstJv::Obj(vec![
            ("kind", AstJv::Str("all".to_string())),
            (
                "children",
                AstJv::Arr(children.iter().map(predicate_to_jv).collect()),
            ),
        ]),
        Predicate::Any { children } => AstJv::Obj(vec![
            ("kind", AstJv::Str("any".to_string())),
            (
                "children",
                AstJv::Arr(children.iter().map(predicate_to_jv).collect()),
            ),
        ]),
        Predicate::Not { child } => AstJv::Obj(vec![
            ("kind", AstJv::Str("not".to_string())),
            ("child", predicate_to_jv(child)),
        ]),
    }
}

fn ast_escape(s: &str, buf: &mut String) {
    buf.push('"');
    for c in s.chars() {
        match c {
            '"' => buf.push_str("\\\""),
            '\\' => buf.push_str("\\\\"),
            '\n' => buf.push_str("\\n"),
            '\r' => buf.push_str("\\r"),
            '\t' => buf.push_str("\\t"),
            '\u{0008}' => buf.push_str("\\b"),
            '\u{000C}' => buf.push_str("\\f"),
            c if (c as u32) < 0x20 => buf.push_str(&format!("\\u{:04x}", c as u32)),
            c => buf.push(c),
        }
    }
    buf.push('"');
}

fn write_ast(v: &AstJv, buf: &mut String, indent: usize) {
    match v {
        AstJv::Str(s) => ast_escape(s, buf),
        AstJv::Obj(pairs) => {
            if pairs.is_empty() {
                buf.push_str("{}");
                return;
            }
            buf.push('{');
            let inner = indent + 2;
            for (i, (k, val)) in pairs.iter().enumerate() {
                buf.push('\n');
                for _ in 0..inner {
                    buf.push(' ');
                }
                buf.push('"');
                buf.push_str(k);
                buf.push_str("\": ");
                write_ast(val, buf, inner);
                if i + 1 < pairs.len() {
                    buf.push(',');
                }
            }
            buf.push('\n');
            for _ in 0..indent {
                buf.push(' ');
            }
            buf.push('}');
        }
        AstJv::Arr(items) => {
            if items.is_empty() {
                buf.push_str("[]");
                return;
            }
            buf.push('[');
            let inner = indent + 2;
            for (i, val) in items.iter().enumerate() {
                buf.push('\n');
                for _ in 0..inner {
                    buf.push(' ');
                }
                write_ast(val, buf, inner);
                if i + 1 < items.len() {
                    buf.push(',');
                }
            }
            buf.push('\n');
            for _ in 0..indent {
                buf.push(' ');
            }
            buf.push(']');
        }
    }
}

/// Serialize the predicate AST to `JSON.stringify(p, undefined, 2)` form (no
/// trailing newline). Used by `policy explain`'s "Normalized AST" block.
pub fn predicate_to_json(p: &Predicate) -> String {
    let jv = predicate_to_jv(p);
    let mut buf = String::new();
    write_ast(&jv, &mut buf, 0);
    buf
}
