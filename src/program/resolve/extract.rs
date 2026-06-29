//! Structured call-site extraction with shape classification — Phase 1 Task 2.
//!
//! Classifies each call expression in every routine body into a [`CalleeShape`]
//! that mirrors L3's `classify_callee` + record-op filter from `ir_walk.rs`.
//! Produces one [`RawSiteV2`] per call site, sorted by `(caller_routine,
//! span.start)`.
//!
//! # Approximations
//! The implicit-Rec bare record-op case (e.g. `Validate(Field)` inside a table
//! trigger where the implicit `Rec` receiver is not explicitly named) is NOT
//! currently classified as `RecordOp` — it emerges as `Bare`. Error() handling
//! and any other classification residual vs L2 are MEASURED by the Phase-1 Task-4
//! site-parity gate (do not assert whether L2 makes a PCallSite for these cases —
//! the gate measures them empirically). The Task-4 gate will quantify the residual
//! from these approximations.

use std::collections::HashSet;

use al_syntax::ir::{AlFile, BlockId, BlockItem, ExprId, ExprKind, RoutineDecl, StmtKind};

use crate::program::resolve::edge::{CanonicalSpan, SourcePos};

/// Classified callee shape for a call expression, mirroring L3's
/// `classify_callee` + record-op filter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CalleeShape {
    /// A bare identifier call: `Foo()`.
    Bare { name: String },
    /// A member call on a non-record, non-keyword receiver: `Helper.Process()`.
    Member {
        receiver_text: String,
        method: String,
    },
    /// An object-run: `Codeunit.Run(...)`, `Page.Run(...)`, `Report.Run(...)`.
    ObjectRun {
        object_kind: String,
        /// Static first argument: the target object name or numeric id, or `None`
        /// when the first argument is a runtime variable (dynamic dispatch).
        /// Derived from `ExprKind::DatabaseReference` only; non-DatabaseReference
        /// arguments produce `None` (mirrors L3's `object_run_callee`).
        target_ref: Option<String>,
        /// `true` when `target_ref` is a name (quoted or bare identifier),
        /// `false` when it is a decimal integer id.
        /// Meaningful only when `target_ref` is `Some`.
        target_is_name: bool,
    },
    /// A record operation on an explicit record-typed receiver: `Rec.SetRange(...)`.
    RecordOp { receiver_text: String, op: String },
    /// A bare `Commit()` call.
    Commit,
    /// Any other call expression that doesn't match a known pattern.
    Unknown,
}

/// One classified call site extracted from a routine body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawSiteV2 {
    /// The enclosing routine's name, lowercased.
    pub caller_routine: String,
    /// Classified callee shape.
    pub shape: CalleeShape,
    /// Number of arguments at the call site.
    pub arity: usize,
    /// Source span of the whole call expression (callee + argument list).
    pub span: CanonicalSpan,
    /// Raw source text of the callee (function) expression — e.g. `"Foo"` or
    /// `"Rec.Insert"`. Does NOT include the argument list. Used to compute
    /// `callee_fp` for the site-parity differential harness (must match L3's
    /// `PCallSite::callee_text` derivation, which is also the raw function-
    /// expression bytes).
    pub callee_text: String,
}

/// The 28 record-operation method names (lowercased), copied verbatim from
/// `src/engine/l2/record_op.rs` (`record_op_type` match arms).
pub fn record_op_names() -> &'static [&'static str] {
    &[
        "findset",
        "findfirst",
        "findlast",
        "find",
        "get",
        "calcfields",
        "calcsums",
        "testfield",
        "modify",
        "modifyall",
        "insert",
        "delete",
        "deleteall",
        "setloadfields",
        "addloadfields",
        "setrange",
        "setfilter",
        "setcurrentkey",
        "reset",
        "copy",
        "transferfields",
        "validate",
        "init",
        "next",
        "count",
        "countapprox",
        "isempty",
        "locktable",
    ]
}

/// Returns `true` if the raw AL type string `ty` denotes a record type.
///
/// Mirrors `ir_walk.rs::is_record_receiver_ty`: the string must start with
/// `"record"` (case-insensitive), INCLUSIVE of `RecordRef`. This ensures that
/// a `RecordRef`-typed var's record-op calls classify as `RecordOp` (matching L3,
/// which makes no PCallSite for them).
fn is_record_ty(ty: &str) -> bool {
    ty.trim().to_ascii_lowercase().starts_with("record")
}

/// Compute the set of lowercased record-receiver variable names in scope for a
/// routine.
///
/// Always includes `"rec"` and `"xrec"` (the AL implicit record vars). Any param
/// or local whose declared type is a Record type (via [`is_record_ty`]) is also
/// included.
///
/// Object-level globals are NOT included here — pass them via `object_globals` in
/// [`extract_sites`].
pub fn routine_rvars(routine: &RoutineDecl) -> HashSet<String> {
    let mut rvars = HashSet::new();
    rvars.insert("rec".to_string());
    rvars.insert("xrec".to_string());
    for p in &routine.params {
        if p.ty.as_deref().map(is_record_ty).unwrap_or(false) {
            rvars.insert(p.name.to_ascii_lowercase());
        }
    }
    for v in &routine.locals {
        if v.ty.as_deref().map(is_record_ty).unwrap_or(false) {
            rvars.insert(v.name.to_ascii_lowercase());
        }
    }
    rvars
}

/// Convert a byte offset into a 0-based `(line, col)` source position by
/// counting newlines in the prefix `src[..byte]`. Mirrors `extract_min.rs`.
fn byte_to_pos(src: &str, byte: usize) -> SourcePos {
    let byte = byte.min(src.len());
    let prefix = &src[..byte];
    let line = prefix.bytes().filter(|&b| b == b'\n').count() as u32;
    let col = match prefix.rfind('\n') {
        Some(nl) => (byte - nl - 1) as u32,
        None => byte as u32,
    };
    SourcePos { line, col }
}

/// Strip one layer of surrounding double-quotes or single-quotes (mirrors L2's
/// `strip_quote_chars`).
fn strip_quote_chars(s: &str) -> String {
    let s = s.trim();
    if s.len() >= 2
        && ((s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')))
    {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}

/// Object-run object kind for a `keyword_identifier` receiver (mirrors L2's
/// `object_run_kind`).
fn object_run_kind(text: &str) -> Option<&'static str> {
    match text.trim().to_ascii_lowercase().as_str() {
        "codeunit" => Some("Codeunit"),
        "page" => Some("Page"),
        "report" => Some("Report"),
        _ => None,
    }
}

/// Classify a single call's callee into a [`CalleeShape`].
///
/// Classification precedence (mirrors L3's `classify_callee` + record-op filter
/// from `ir_walk.rs`):
///
/// 1. `Member` with `Identifier`/`QuotedIdentifier` receiver in `rvars` AND
///    method in [`record_op_names`] → `RecordOp`.
/// 2. `Member` with `keyword_identifier` receiver (`codeunit`/`page`/`report`)
///    AND method `"run"` → `ObjectRun`.
/// 3. Any other `Member` → `Member`.
/// 4. Bare `Identifier("commit")` / `QuotedIdentifier("commit")` → `Commit`.
/// 5. Any other bare `Identifier` / `QuotedIdentifier` → `Bare`.
/// 6. Everything else → `Unknown`.
fn classify_call(
    file: &AlFile,
    src: &str,
    function: ExprId,
    args: &[ExprId],
    rvars: &HashSet<String>,
) -> CalleeShape {
    let fe = file.ir.expr(function);
    match &fe.kind {
        ExprKind::Member { object, member, .. } => {
            let obj = file.ir.expr(*object);
            // Strip quotes + lowercase for matching.
            let method_lc = strip_quote_chars(member).to_ascii_lowercase();

            // --- Check 1: RecordOp ------------------------------------------------
            // Receiver must be a simple Identifier/QuotedIdentifier in rvars AND
            // method must be in record_op_names (mirrors L2 record-op filter).
            let recv_lc = match &obj.kind {
                ExprKind::Identifier(r) | ExprKind::QuotedIdentifier(r) => {
                    Some(r.to_ascii_lowercase())
                }
                _ => None,
            };
            if let Some(ref r_lc) = recv_lc
                && rvars.contains(r_lc)
                && record_op_names().contains(&method_lc.as_str())
            {
                let receiver_text = src[obj.origin.byte.clone()].to_string();
                return CalleeShape::RecordOp {
                    receiver_text,
                    op: method_lc,
                };
            }

            // --- Check 2: ObjectRun -----------------------------------------------
            // Mirrors L2's `object_run_callee`: receiver is `keyword_identifier`
            // (Codeunit/Page/Report) AND method is "run".
            // Target extraction: only `ExprKind::DatabaseReference` arguments carry
            // a static target; all other argument kinds (variables, integer literals
            // NOT wrapped in DatabaseReference) produce `target_ref = None` (dynamic
            // dispatch — mirrors L3's behaviour).
            if obj.origin.kind_text == "keyword_identifier" && method_lc == "run" {
                let obj_text = &src[obj.origin.byte.clone()];
                if let Some(okind) = object_run_kind(obj_text) {
                    let mut target_ref: Option<String> = None;
                    let mut target_is_name = false;
                    if let Some(&first_arg) = args.first()
                        && let ExprKind::DatabaseReference(text) = &file.ir.expr(first_arg).kind
                        && let Some((_, tn)) = text.split_once("::")
                    {
                        let tn = tn.trim();
                        if tn.starts_with('"') {
                            // Quoted name: strip surrounding quotes.
                            target_ref = Some(strip_quote_chars(tn));
                            target_is_name = true;
                        } else if tn.parse::<i64>().is_ok() {
                            // Decimal integer id.
                            target_ref = Some(tn.to_string());
                            // target_is_name stays false
                        } else {
                            // Bare (unquoted) name.
                            target_ref = Some(tn.to_string());
                            target_is_name = true;
                        }
                        // Non-DatabaseReference first arg (variable, expression, etc.)
                        // → let-chain falls through; target_ref stays None (dynamic).
                    }
                    return CalleeShape::ObjectRun {
                        object_kind: okind.to_string(),
                        target_ref,
                        target_is_name,
                    };
                }
            }

            // --- Check 3: General Member ------------------------------------------
            let receiver_text = src[obj.origin.byte.clone()].to_string();
            let method = strip_quote_chars(member);
            CalleeShape::Member {
                receiver_text,
                method,
            }
        }

        ExprKind::Identifier(name) => {
            if name.eq_ignore_ascii_case("commit") {
                CalleeShape::Commit
            } else {
                CalleeShape::Bare { name: name.clone() }
            }
        }
        ExprKind::QuotedIdentifier(name) => {
            // QuotedIdentifier stores the already-unquoted name (lowerer strips quotes).
            if name.eq_ignore_ascii_case("commit") {
                CalleeShape::Commit
            } else {
                CalleeShape::Bare { name: name.clone() }
            }
        }

        _ => CalleeShape::Unknown,
    }
}

/// Recursively collect every [`RawSiteV2`] reachable from `eid`, including
/// calls nested inside arguments or chained receivers.
fn collect_calls_v2(
    file: &AlFile,
    src: &str,
    eid: ExprId,
    unit: &str,
    caller: &str,
    rvars: &HashSet<String>,
    out: &mut Vec<RawSiteV2>,
) {
    let e = file.ir.expr(eid);
    match &e.kind {
        ExprKind::Call { function, args } => {
            let fn_id = *function;
            let arg_ids = args.to_vec();

            // Classify and emit this call site.
            let shape = classify_call(file, src, fn_id, &arg_ids, rvars);
            // callee_text = raw source bytes of the function expression (not the
            // arg list).  Mirrors extract_min.rs and L3's ir_walk classify_callee
            // so callee_fp agrees between the two sides of the harness.
            let callee_text = src[file.ir.expr(fn_id).origin.byte.clone()].to_string();
            let span = CanonicalSpan {
                unit: unit.to_string(),
                start: byte_to_pos(src, e.origin.byte.start),
                end: byte_to_pos(src, e.origin.byte.end),
            };
            out.push(RawSiteV2 {
                caller_routine: caller.to_string(),
                shape,
                arity: arg_ids.len(),
                span,
                callee_text,
            });

            // Recurse: function expression (catches chained calls), then args.
            collect_calls_v2(file, src, fn_id, unit, caller, rvars, out);
            for a in arg_ids {
                collect_calls_v2(file, src, a, unit, caller, rvars, out);
            }
        }
        ExprKind::Member { object, .. } => {
            let obj = *object;
            collect_calls_v2(file, src, obj, unit, caller, rvars, out);
        }
        ExprKind::Binary { lhs, rhs, .. } => {
            let (l, r) = (*lhs, *rhs);
            collect_calls_v2(file, src, l, unit, caller, rvars, out);
            collect_calls_v2(file, src, r, unit, caller, rvars, out);
        }
        ExprKind::Unary { operand, .. } => {
            let op = *operand;
            collect_calls_v2(file, src, op, unit, caller, rvars, out);
        }
        ExprKind::Parenthesized(x) => {
            let x = *x;
            collect_calls_v2(file, src, x, unit, caller, rvars, out);
        }
        ExprKind::Index { base, index } => {
            let (b, i) = (*base, *index);
            collect_calls_v2(file, src, b, unit, caller, rvars, out);
            collect_calls_v2(file, src, i, unit, caller, rvars, out);
        }
        ExprKind::RangeExpr { start, end } => {
            let (s, e2) = (*start, *end);
            collect_calls_v2(file, src, s, unit, caller, rvars, out);
            collect_calls_v2(file, src, e2, unit, caller, rvars, out);
        }
        ExprKind::QualifiedEnum { enum_type, .. } => {
            let et = *enum_type;
            collect_calls_v2(file, src, et, unit, caller, rvars, out);
        }
        // Identifier / QuotedIdentifier / Literal / DatabaseReference / Unknown:
        // no nested calls.
        _ => {}
    }
}

fn walk_block_v2(
    file: &AlFile,
    src: &str,
    bid: BlockId,
    unit: &str,
    caller: &str,
    rvars: &HashSet<String>,
    out: &mut Vec<RawSiteV2>,
) {
    for item in &file.ir.block(bid).items {
        match item {
            BlockItem::Stmt(sid) => {
                let st = file.ir.stmt(*sid);
                walk_stmt_v2(file, src, &st.kind, unit, caller, rvars, out);
            }
            BlockItem::Preproc(g) => {
                for b in &g.branches {
                    walk_block_v2(file, src, *b, unit, caller, rvars, out);
                }
            }
        }
    }
}

fn walk_stmt_v2(
    file: &AlFile,
    src: &str,
    kind: &StmtKind,
    unit: &str,
    caller: &str,
    rvars: &HashSet<String>,
    out: &mut Vec<RawSiteV2>,
) {
    match kind {
        StmtKind::Assignment { target, value } => {
            collect_calls_v2(file, src, *target, unit, caller, rvars, out);
            collect_calls_v2(file, src, *value, unit, caller, rvars, out);
        }
        StmtKind::Call(eid) => {
            collect_calls_v2(file, src, *eid, unit, caller, rvars, out);
        }
        StmtKind::If {
            cond,
            then_block,
            else_block,
        } => {
            collect_calls_v2(file, src, *cond, unit, caller, rvars, out);
            walk_block_v2(file, src, *then_block, unit, caller, rvars, out);
            if let Some(b) = else_block {
                walk_block_v2(file, src, *b, unit, caller, rvars, out);
            }
        }
        StmtKind::Case {
            scrutinee,
            branches,
            else_block,
        } => {
            collect_calls_v2(file, src, *scrutinee, unit, caller, rvars, out);
            for br in branches {
                for &p in &br.patterns {
                    collect_calls_v2(file, src, p, unit, caller, rvars, out);
                }
                walk_block_v2(file, src, br.body, unit, caller, rvars, out);
            }
            if let Some(b) = else_block {
                walk_block_v2(file, src, *b, unit, caller, rvars, out);
            }
        }
        StmtKind::While { cond, body } => {
            collect_calls_v2(file, src, *cond, unit, caller, rvars, out);
            walk_block_v2(file, src, *body, unit, caller, rvars, out);
        }
        StmtKind::Repeat { body, until } => {
            walk_block_v2(file, src, *body, unit, caller, rvars, out);
            collect_calls_v2(file, src, *until, unit, caller, rvars, out);
        }
        StmtKind::For {
            var,
            from,
            to,
            body,
            ..
        } => {
            collect_calls_v2(file, src, *var, unit, caller, rvars, out);
            collect_calls_v2(file, src, *from, unit, caller, rvars, out);
            collect_calls_v2(file, src, *to, unit, caller, rvars, out);
            walk_block_v2(file, src, *body, unit, caller, rvars, out);
        }
        StmtKind::Foreach {
            var,
            iterable,
            body,
        } => {
            collect_calls_v2(file, src, *var, unit, caller, rvars, out);
            collect_calls_v2(file, src, *iterable, unit, caller, rvars, out);
            walk_block_v2(file, src, *body, unit, caller, rvars, out);
        }
        StmtKind::With { receiver, body } => {
            collect_calls_v2(file, src, *receiver, unit, caller, rvars, out);
            walk_block_v2(file, src, *body, unit, caller, rvars, out);
        }
        StmtKind::Try { body, catch_block } => {
            walk_block_v2(file, src, *body, unit, caller, rvars, out);
            if let Some(c) = catch_block {
                walk_block_v2(file, src, *c, unit, caller, rvars, out);
            }
        }
        StmtKind::AssertError(body) => {
            walk_block_v2(file, src, *body, unit, caller, rvars, out);
        }
        StmtKind::Exit(x) => {
            if let Some(e) = x {
                collect_calls_v2(file, src, *e, unit, caller, rvars, out);
            }
        }
        StmtKind::Block(b) => {
            walk_block_v2(file, src, *b, unit, caller, rvars, out);
        }
        StmtKind::Break | StmtKind::Continue | StmtKind::Unknown => {}
    }
}

/// Walk every routine body in `file` and return one [`RawSiteV2`] per call
/// expression, classified into a [`CalleeShape`].
///
/// `src` is the original AL source text; `unit` names the file (e.g. `"C.al"`);
/// `object_globals` is the set of lowercased record-typed global variable names
/// from the enclosing object (the caller is responsible for filtering to
/// record-typed globals before passing in). Per routine, `rvars = routine_rvars(routine)
/// ∪ object_globals`. The result is sorted by `(caller_routine, span.start)`.
///
/// **Multi-object limitation:** when a file contains more than one object and
/// two objects share a routine name, the returned list will contain sites from
/// BOTH routines under the same `caller_routine` label.  Callers that need to
/// attribute sites to a single object should use [`extract_sites_for_object`]
/// instead.
pub fn extract_sites(
    file: &AlFile,
    src: &str,
    unit: &str,
    object_globals: &HashSet<String>,
) -> Vec<RawSiteV2> {
    let mut out = Vec::new();
    for obj in &file.objects {
        for routine in &obj.routines {
            if let Some(body) = routine.body {
                let caller = routine.name.to_ascii_lowercase();
                let mut rvars = routine_rvars(routine);
                rvars.extend(object_globals.iter().cloned());
                walk_block_v2(file, src, body, unit, &caller, &rvars, &mut out);
            }
        }
    }
    out.sort_by(|a, b| {
        a.caller_routine
            .cmp(&b.caller_routine)
            .then_with(|| a.span.start.cmp(&b.span.start))
    });
    out
}

/// Walk only the routines of `file.objects[obj_idx]` and return one
/// [`RawSiteV2`] per call expression.
///
/// Unlike [`extract_sites`] (which processes ALL objects in the file),
/// this variant is scoped to a single object so that sites are unambiguously
/// attributed to that object even when multiple objects in the same file share
/// routine names.
///
/// `object_globals` should contain only the record-typed global variable names
/// declared in `file.objects[obj_idx]`.  The result is sorted by
/// `(caller_routine, span.start)`.
///
/// Returns an empty `Vec` if `obj_idx` is out of range.
pub fn extract_sites_for_object(
    file: &AlFile,
    src: &str,
    unit: &str,
    object_globals: &HashSet<String>,
    obj_idx: usize,
) -> Vec<RawSiteV2> {
    let Some(obj) = file.objects.get(obj_idx) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for routine in &obj.routines {
        if let Some(body) = routine.body {
            let caller = routine.name.to_ascii_lowercase();
            let mut rvars = routine_rvars(routine);
            rvars.extend(object_globals.iter().cloned());
            walk_block_v2(file, src, body, unit, &caller, &rvars, &mut out);
        }
    }
    out.sort_by(|a, b| {
        a.caller_routine
            .cmp(&b.caller_routine)
            .then_with(|| a.span.start.cmp(&b.span.start))
    });
    out
}

/// Walk only `file.objects[obj_idx].routines[routine_idx]` and return one
/// [`RawSiteV2`] per call expression in that single routine body.
///
/// This is the per-routine companion to [`extract_sites_for_object`].  Use it
/// when iterating over `obj.routines` by index in the calling code, to avoid
/// attributing sites to the wrong routine instance when two routines in the
/// same object share a name (e.g. multiple `OnValidate` field triggers in a
/// TableExtension).
///
/// `object_globals` should be the record-typed global variable names from
/// `file.objects[obj_idx]` only.
///
/// Returns an empty `Vec` if either index is out of range or the routine has
/// no body.
pub fn extract_sites_for_routine(
    file: &AlFile,
    src: &str,
    unit: &str,
    object_globals: &HashSet<String>,
    obj_idx: usize,
    routine_idx: usize,
) -> Vec<RawSiteV2> {
    let Some(obj) = file.objects.get(obj_idx) else {
        return Vec::new();
    };
    let Some(routine) = obj.routines.get(routine_idx) else {
        return Vec::new();
    };
    let Some(body) = routine.body else {
        return Vec::new();
    };
    let caller = routine.name.to_ascii_lowercase();
    let mut rvars = routine_rvars(routine);
    rvars.extend(object_globals.iter().cloned());
    let mut out = Vec::new();
    walk_block_v2(file, src, body, unit, &caller, &rvars, &mut out);
    out.sort_by_key(|a| a.span.start);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_call_shapes() {
        let src = r#"
codeunit 50100 "C"
{
    procedure Run()
    var
        Rec: Record Item;
        Json: JsonObject;
    begin
        Foo();
        Helper.Process();
        Rec.SetRange(Status);
        Codeunit.Run(Codeunit::"Other");
        Json.Add('a');
        Commit();
    end;
    procedure Foo() begin end;
}
"#;
        let file = al_syntax::parse(src);
        let sites = extract_sites(&file, src, "C.al", &std::collections::HashSet::new());
        let run: Vec<_> = sites.iter().filter(|s| s.caller_routine == "run").collect();
        assert_eq!(run.len(), 6, "Expected 6 call sites in Run procedure");
        assert!(run.iter().any(
            |s| matches!(&s.shape, CalleeShape::Bare { name } if name.eq_ignore_ascii_case("foo"))
        ));
        assert!(
            run.iter()
                .any(|s| matches!(&s.shape, CalleeShape::Member { method, .. } if method.eq_ignore_ascii_case("process")))
        );
        assert!(
            run.iter()
                .any(|s| matches!(&s.shape, CalleeShape::RecordOp { op, .. } if op.eq_ignore_ascii_case("setrange")))
        );
        assert!(
            run.iter()
                .any(|s| matches!(&s.shape, CalleeShape::ObjectRun { .. }))
        );
        assert!(run.iter().any(|s| matches!(&s.shape, CalleeShape::Commit)));
        // Json.Add is a Member call, NOT a RecordOp (Json is not a record).
        assert!(
            run.iter()
                .any(|s| matches!(&s.shape, CalleeShape::Member { receiver_text, method } if receiver_text.eq_ignore_ascii_case("json") && method.eq_ignore_ascii_case("add")))
        );
        assert!(
            !run.iter()
                .any(|s| matches!(&s.shape, CalleeShape::RecordOp { receiver_text, .. } if receiver_text.eq_ignore_ascii_case("json")))
        );
    }
}
