//! Predicate evaluator — port of al-sem `src/policy/predicate-evaluator.ts`.
//!
//! Tristate (Kleene) evaluation of a compiled predicate over a [`FieldEvalContext`].
//! 4 operators (`==` / `in` / `glob` / `glob-in`) + the 3 Kleene truth tables
//! (AND/OR/NOT). Two modes:
//!   - `full`: every field evaluated normally (fact-scope reads `ctx.fact`).
//!   - `applicability`: fact-scope fields short-circuit to `unknown` (no fact
//!     required) — used to skip routines a rule structurally cannot apply to.

use crate::engine::gate::policy::policy_types::{Predicate, PredicateOperator, PredicateValue};
use crate::engine::gate::policy::predicate_fields::{
    evaluate_field, get_field_def, FieldEvalContext, FieldScope, FieldValue,
};

/// Kleene tristate. al-sem `Tristate`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tristate {
    True,
    False,
    Unknown,
}

/// AND (all): false dominates > unknown > true.
pub fn kleene_and(a: Tristate, b: Tristate) -> Tristate {
    if a == Tristate::False || b == Tristate::False {
        Tristate::False
    } else if a == Tristate::Unknown || b == Tristate::Unknown {
        Tristate::Unknown
    } else {
        Tristate::True
    }
}

/// OR (any): true dominates > unknown > false.
pub fn kleene_or(a: Tristate, b: Tristate) -> Tristate {
    if a == Tristate::True || b == Tristate::True {
        Tristate::True
    } else if a == Tristate::Unknown || b == Tristate::Unknown {
        Tristate::Unknown
    } else {
        Tristate::False
    }
}

/// NOT: swaps true/false, unknown→unknown.
pub fn kleene_not(a: Tristate) -> Tristate {
    match a {
        Tristate::True => Tristate::False,
        Tristate::False => Tristate::True,
        Tristate::Unknown => Tristate::Unknown,
    }
}

/// Evaluation mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvalMode {
    Full,
    Applicability,
}

/// Match an operator against the actual field value. al-sem `matchOperator`.
fn match_operator(op: PredicateOperator, actual: &FieldValue, expected: &PredicateValue) -> bool {
    match op {
        PredicateOperator::Eq => match (actual, expected) {
            // strict eq: actual === expected. actual is a single string here.
            (FieldValue::KnownStr(a), PredicateValue::Str(e)) => a == e,
            _ => false,
        },
        PredicateOperator::In => {
            // membership; if actual is an array, ANY-overlap.
            let PredicateValue::List(list) = expected else {
                return false;
            };
            match actual {
                FieldValue::KnownStr(a) => list.iter().any(|e| e == a),
                FieldValue::KnownList(arr) => arr.iter().any(|a| list.iter().any(|e| e == a)),
                FieldValue::Unknown(_) => false,
            }
        }
        PredicateOperator::Glob => {
            let PredicateValue::Str(pat) = expected else {
                return false;
            };
            glob_match(pat, &actual_to_string(actual))
        }
        PredicateOperator::GlobIn => {
            let PredicateValue::List(pats) = expected else {
                return false;
            };
            match actual {
                FieldValue::KnownList(arr) => {
                    pats.iter().any(|g| arr.iter().any(|a| glob_match(g, a)))
                }
                _ => {
                    let s = actual_to_string(actual);
                    pats.iter().any(|g| glob_match(g, &s))
                }
            }
        }
    }
}

/// al-sem `actualToString`: strings pass through, arrays join with `,`.
fn actual_to_string(v: &FieldValue) -> String {
    match v {
        FieldValue::KnownStr(s) => s.clone(),
        FieldValue::KnownList(arr) => arr.join(","),
        // String(undefined) etc. — never reached for a KNOWN value; defensive.
        FieldValue::Unknown(_) => String::new(),
    }
}

/// Case-insensitive anchored glob. al-sem `globMatch` / `compileGlob`: `*` → match
/// any chars, `?` → match single char, other regex metachars escaped, `^...$`
/// anchored, ASCII-insensitive, `.` excludes newline.
///
/// We compile to a hand-rolled matcher (no regex dep). Semantics match the JS
/// `RegExp("^"+escaped+"$","i")` with `*`→`.*`, `?`→`.` where `.` matches any char
/// EXCEPT newline. Case-insensitivity is full Unicode simple case folding via
/// `to_lowercase`, matching JS `i`-flag for the BMP table/event names compared here.
pub fn glob_match(pattern: &str, value: &str) -> bool {
    // Tokenize the pattern into a sequence of matcher ops.
    enum Tok {
        Star,          // .*  (any run, including empty; excludes newline per char)
        AnyNonNewline, // .   (exactly one non-newline char)
        Lit(char),     // a literal char (case-insensitive compare)
    }
    let mut toks: Vec<Tok> = Vec::with_capacity(pattern.chars().count());
    for c in pattern.chars() {
        match c {
            '*' => toks.push(Tok::Star),
            '?' => toks.push(Tok::AnyNonNewline),
            other => toks.push(Tok::Lit(other)),
        }
    }
    let value_chars: Vec<char> = value.chars().collect();

    // Backtracking matcher (anchored). Patterns are short (policy globs), so the
    // recursive star-backtrack is fine.
    fn matches(toks: &[Tok], ti: usize, val: &[char], vi: usize) -> bool {
        if ti == toks.len() {
            return vi == val.len();
        }
        match &toks[ti] {
            Tok::Star => {
                // `.*` — `.` excludes newline, so a `*` cannot span a newline.
                // Try consuming 0..k non-newline chars.
                // First try zero-width:
                if matches(toks, ti + 1, val, vi) {
                    return true;
                }
                let mut j = vi;
                while j < val.len() && val[j] != '\n' {
                    j += 1;
                    if matches(toks, ti + 1, val, j) {
                        return true;
                    }
                }
                false
            }
            Tok::AnyNonNewline => {
                if vi < val.len() && val[vi] != '\n' {
                    matches(toks, ti + 1, val, vi + 1)
                } else {
                    false
                }
            }
            Tok::Lit(p) => {
                if vi < val.len() && chars_eq_ci(*p, val[vi]) {
                    matches(toks, ti + 1, val, vi + 1)
                } else {
                    false
                }
            }
        }
    }

    matches(&toks, 0, &value_chars, 0)
}

/// Case-insensitive char equality (JS RegExp `i` flag, simple case folding).
fn chars_eq_ci(a: char, b: char) -> bool {
    if a == b {
        return true;
    }
    // Compare simple lowercase folds (handles ASCII + BMP letters).
    a.to_lowercase().eq(b.to_lowercase())
}

/// Evaluate a single field predicate to a tristate. `mode` controls fact-scope
/// short-circuiting.
fn field_tristate(
    field: &str,
    operator: PredicateOperator,
    value: &PredicateValue,
    ctx: &FieldEvalContext,
    mode: EvalMode,
) -> Tristate {
    let Some(def) = get_field_def(field) else {
        return Tristate::Unknown;
    };
    // Applicability: a fact-scoped field cannot be decided without a fact.
    if mode == EvalMode::Applicability && def.scope == FieldScope::Fact {
        return Tristate::Unknown;
    }
    let fv = evaluate_field(def, ctx);
    match fv {
        FieldValue::Unknown(_) => Tristate::Unknown,
        known => {
            if match_operator(operator, &known, value) {
                Tristate::True
            } else {
                Tristate::False
            }
        }
    }
}

/// Tristate-only evaluation (no trace). al-sem `evalTristate`.
pub fn eval_tristate(p: &Predicate, ctx: &FieldEvalContext, mode: EvalMode) -> Tristate {
    match p {
        Predicate::Field {
            field,
            operator,
            value,
        } => field_tristate(field, *operator, value, ctx, mode),
        Predicate::All { children } => {
            if children.is_empty() {
                return Tristate::True; // empty conjunction → true
            }
            let mut acc = Tristate::True;
            for c in children {
                acc = kleene_and(acc, eval_tristate(c, ctx, mode));
                if acc == Tristate::False {
                    return Tristate::False; // short-circuit
                }
            }
            acc
        }
        Predicate::Any { children } => {
            if children.is_empty() {
                return Tristate::False; // empty disjunction → false
            }
            let mut acc = Tristate::False;
            for c in children {
                acc = kleene_or(acc, eval_tristate(c, ctx, mode));
                if acc == Tristate::True {
                    return Tristate::True;
                }
            }
            acc
        }
        Predicate::Not { child } => kleene_not(eval_tristate(child, ctx, mode)),
    }
}

/// Full evaluation (fact present). al-sem `evaluateResult`.
pub fn evaluate_result(p: &Predicate, ctx: &FieldEvalContext) -> Tristate {
    eval_tristate(p, ctx, EvalMode::Full)
}

/// Applicability evaluation (fact fields ⇒ unknown). al-sem `evaluateApplicability`.
/// `false` ⇒ no fact can satisfy P for this routine ⇒ rule not applicable.
pub fn evaluate_applicability(p: &Predicate, ctx: &FieldEvalContext) -> Tristate {
    eval_tristate(p, ctx, EvalMode::Applicability)
}
