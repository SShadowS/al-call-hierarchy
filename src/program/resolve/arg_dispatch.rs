//! Fail-closed argument-type overload dispatch — argtype-dispatch-and-page-catalog
//! plan (v2.1), Task 2.
//!
//! [`resolver::resolve_in_object`]'s `_` arm produces a same-name/same-arity
//! candidate set that has ALREADY survived every existing prevalidation
//! (dedup-shrink / ABI collapse-marker / source-alias / Unknown-evidence) —
//! a proven-exhaustive, CLOSED enumeration with no arg-type evidence to pick
//! between its members. This module adds exactly one thing on top: when the
//! call's arguments are FULLY typed and every candidate's parameter metadata
//! is FULLY known, and EXACTLY ONE candidate's parameter list is
//! DISPATCH-COMPATIBLE with those typed arguments, pick it. Every other
//! outcome is untouched — the set stays `AmbiguousResolved`, which is already
//! an honest answer (see `edge::ObligationOutcome`).
//!
//! # Cardinal rule
//!
//! A WRONG pick is the cardinal sin. Every rule below is written to fail
//! CLOSED: any doubt anywhere degrades the WHOLE call back to no pick, never
//! a partial/best-effort choice. In particular, an unknown-metadata candidate
//! is NEVER filtered out of the competition to let the "provable" remainder
//! resolve — its mere presence degrades the whole call.
//!
//! # The hardened rule set (plan v2.1 Round-1 + Round-2 addenda — BINDING)
//!
//! - **Call-level degradation** ([`pick_candidate`]): a pick requires ALL
//!   supplied args typed (`ArgDispatchInfo::canonical` is `Some`) AND every
//!   candidate's full parameter type+mode metadata known
//!   ([`candidate_param_infos`] returns `Some` for every candidate). ANY
//!   untyped arg / missing candidate metadata / SymbolOnly candidate in the
//!   set / degraded candidate (caller-side prevalidation) → NO PICK.
//! - **Dispatch-canonical identity, not text identity** ([`CanonicalArgType`],
//!   [`dispatch_canonical_type_text`]): Text/Code length brackets are
//!   NON-DISCRIMINATING for by-value compatibility (stripped uniformly by
//!   [`base_keyword`]); object-bearing types (Record/Page/Report/Codeunit/
//!   Query/XmlPort/Enum/Interface) canonicalize via the EXISTING fail-closed
//!   [`ResolveIndex::resolve_object_ref`] semantic identity (`Record "Sales
//!   Header"` == `Record 36` iff they resolve to the SAME table;
//!   unresolvable/ambiguous → that position is untyped); scalar families
//!   compare by exact base keyword only — no implicit-conversion modeling
//!   (`integer` != `decimal` != `biginteger`; `text` != `code`).
//! - **`var` params are ByRef-EXACT identity** ([`positions_compatible`]):
//!   the length-stripping rule applies ONLY to by-value compatibility; a
//!   `var` parameter additionally requires the arg's FULL normalized type
//!   text (length included) to match, and the arg must be
//!   [`ArgDispatchInfo::var_passable`] (a literal/call-result is never
//!   var-passable — a sound elimination, not a degrade).
//! - **`Variant`/`Any` at a discriminating position degrades**
//!   ([`pick_candidate`]): computed from the FULL candidate set BEFORE any
//!   compatibility filtering — a Variant-bearing candidate degrades the call
//!   even if it would otherwise have been "eliminated" by a naive
//!   exclusion-style matcher (no compiler-fixture-proven Variant precedence
//!   exists yet).
//! - **Candidate-set-aware literal typing**: THIS increment types only the
//!   fixture-proven literal families (Integer/Text/Bool/Decimal-with-point —
//!   see [`literal_canonical`]) via ordinary exact-canonical-match
//!   comparison; the additional STRING-vs-Code/Char and INTEGER-vs-
//!   Decimal/BigInteger candidate-set-aware degrade/eliminate rules (C6) are
//!   satisfied by the same exact-match mechanism for every pair EXCEPT the
//!   compiler-proven Integer-literal-vs-`Code[N]` exemplar, which the
//!   dedicated `ws-overload-collision` fixture exercises (an Integer literal
//!   structurally cannot bind `Code[N]`, so ordinary exact-canonical-mismatch
//!   elimination of the `Code[N]` candidate there IS the compiler-proven
//!   answer, not an extra rule).
//! - **Caller-scope-EXACT var lookup** ([`type_one_arg`]): params → locals →
//!   globals, the SAME shadowing order `receiver.rs`'s Step 2 uses for
//!   receiver typing — never a receiver/`with` scope.
//! - **`with`-scope gate for bare-identifier args** ([`type_one_arg`], Task 2
//!   review fix): AL's `with X do` rebinds a bare identifier to the
//!   `with`-receiver's member — this module's caller-scope-EXACT lookup
//!   (params → locals → globals) structurally CANNOT see that rebinding, so
//!   typing a bare-identifier arg from caller scope while inside an
//!   unrepresented `with` risks a WRONG PICK (e.g. `with Rec do
//!   Target.Foo(SomeField)`, where a table field shadows a same-named global
//!   of a DIFFERENT type across two overloads). Mirrors `resolve_bare`'s own
//!   Step 3 with-guard EXACTLY: a bare-identifier arg is typed from caller
//!   scope only when `with_state == WithState::NoWithProven`; any other
//!   state (`InsideWith` or the disagreeing-signals `Unknown`) degrades that
//!   ONE argument position to [`ArgDispatchInfo::untyped`], which in turn
//!   degrades the WHOLE call (module doc's cardinal rule) — never a partial
//!   pick. A LITERAL argument is unaffected (a literal cannot rebind).
//!
//! # SOURCE tier only
//!
//! Candidate parameter metadata comes exclusively from [`BodyMap`]
//! (source-parsed `RoutineDecl`s) — a SymbolOnly (ABI) candidate has no entry
//! there at all, so [`candidate_param_infos`] can never supply metadata for
//! one; callers additionally gate EXPLICITLY on `obj_tier !=
//! TrustTier::SymbolOnly` before attempting a pick (clean skip, not partial —
//! defense-in-depth on top of the BodyMap-miss behavior, not a substitute
//! for it).
//!
//! [`BodyMap`]: crate::program::resolve::body_map::BodyMap
//! [`resolver::resolve_in_object`]: crate::program::resolve::resolver

use al_syntax::ir::{
    AlFile, BinaryOp, Expr, ExprId, ExprKind, Literal, ObjectKind, RoutineDecl, VarDecl,
};

use crate::program::graph::ProgramGraph;
use crate::program::node::ObjectNodeId;
use crate::program::node_extract::{ObjectRef, RoutineNode};
use crate::program::resolve::body_map::BodyMap;
use crate::program::resolve::edge::{BuiltinId, RouteTarget};
use crate::program::resolve::extract::WithState;
use crate::program::resolve::index::{ObjectRefResolution, ResolveIndex};
use crate::program::resolve::receiver::{
    CallerScopeSymbol, ParsedType, caller_scope_symbol, classify_type_text, object_by_id,
    parsed_type_to_receiver, unquote_identifier,
};
use crate::program::resolve::resolver::{
    resolve_bare, resolve_member, routine_node_for_type_query,
};
use crate::program::sig_fp::normalize_type_text;

// ---------------------------------------------------------------------------
// CanonicalArgType
// ---------------------------------------------------------------------------

/// Dispatch-canonical identity of a parameter/argument TYPE — the comparison
/// key the pick uses instead of raw `sig_fp` text (Round-1 addendum,
/// "Dispatch identity != text identity").
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CanonicalArgType {
    /// An object-bearing type resolved to its concrete [`ObjectNodeId`] via
    /// the existing fail-closed [`ResolveIndex::resolve_object_ref`] —
    /// semantic identity, so a name reference and a numeric-id reference to
    /// the SAME declared object compare equal.
    Object(ObjectNodeId),
    /// Any other declared type, canonicalized to its base keyword with a
    /// trailing `[N]` length suffix STRIPPED (`Text[30]`/`Code[20]`/`Text`
    /// canonicalize to `"text"`/`"code"`/`"text"` respectively) — exact
    /// keyword identity only, no implicit-conversion modeling.
    Base(String),
}

impl CanonicalArgType {
    /// Whether this canonical type is the `Variant`/`Any` wildcard — see the
    /// module doc's Variant-wildcard rule.
    fn is_variant_or_any(&self) -> bool {
        matches!(self, CanonicalArgType::Base(s) if s == "variant" || s == "any")
    }
}

/// The literal family an argument expression was proven to be — used ONLY
/// for candidate-set-aware literal elimination (module doc, C6); fixture-
/// proven families only (Round-1 addendum: "Unproven literal shapes → None").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LiteralKind {
    Integer,
    Text,
    Bool,
    Decimal,
}

/// Lowercase the leading whitespace-delimited token of a declared type string
/// and strip a trailing `[N]` length suffix — mirrors
/// `receiver::classify_type_text`'s own tokenization, but returns the raw
/// keyword text directly (never collapsed into a broader family) so
/// Text/Code/Label and Integer/Decimal/BigInteger stay individually
/// distinguishable, which `classify_type_text`'s `ParsedType::Framework`/
/// `Primitive` catch-alls deliberately do NOT preserve (they exist for
/// RECEIVER dispatch, a different, coarser lattice than exact-keyword
/// argument-type identity).
fn base_keyword(ty: &str) -> String {
    let trimmed = ty.trim();
    let first_tok = match trimmed.find(char::is_whitespace) {
        Some(i) => &trimmed[..i],
        None => trimmed,
    };
    let base = match first_tok.find('[') {
        Some(i) => &first_tok[..i],
        None => first_tok,
    };
    base.to_ascii_lowercase()
}

/// Canonicalize a declared type TEXT (a param's or a variable's `ty`) into
/// its [`CanonicalArgType`], as seen from `from`'s app dependency closure —
/// `None` when the text is empty, or names an object-bearing type that does
/// not resolve to EXACTLY one object in that closure (ambiguous/out-of-
/// closure/unresolved — "unresolvable -> position untyped -> degrade",
/// Round-1 addendum).
pub(crate) fn dispatch_canonical_type_text(
    ty_text: &str,
    from: &ObjectNodeId,
    graph: &ProgramGraph,
    index: &ResolveIndex,
) -> Option<CanonicalArgType> {
    if ty_text.trim().is_empty() {
        return None;
    }
    let resolve = |kind: ObjectKind, oref: &ObjectRef| match index.resolve_object_ref(
        graph,
        from.clone(),
        kind,
        oref,
    ) {
        ObjectRefResolution::Unique(id) => Some(CanonicalArgType::Object(id)),
        ObjectRefResolution::Ambiguous
        | ObjectRefResolution::OutOfClosure
        | ObjectRefResolution::Unresolved => None,
    };
    match classify_type_text(ty_text) {
        ParsedType::Record { table_ref } => resolve(ObjectKind::Table, &table_ref),
        ParsedType::Object { kind, object_ref } => resolve(kind, &object_ref),
        ParsedType::Interface { name } => {
            let oref = ObjectRef::Name {
                raw: name.clone(),
                normalized_lc: name,
            };
            resolve(ObjectKind::Interface, &oref)
        }
        ParsedType::EnumType { name } => {
            let oref = ObjectRef::Name {
                raw: name.clone(),
                normalized_lc: name,
            };
            resolve(ObjectKind::Enum, &oref)
        }
        // RecordRef / FieldRef / KeyRef / Framework(_) / Primitive / Dynamic:
        // no object identity to resolve — canonicalize by exact base keyword
        // directly from the raw text (see `base_keyword`'s doc for why this
        // does NOT reuse `classify_type_text`'s own Framework/Primitive
        // grouping).
        _ => Some(CanonicalArgType::Base(base_keyword(ty_text))),
    }
}

/// A literal's proven [`CanonicalArgType`] + [`LiteralKind`], for the
/// fixture-proven families only (Round-1 addendum). `None` for every other
/// literal shape (Date/DateTime/Time/Other) — an unproven literal stays
/// untyped, never eliminates a candidate.
fn literal_canonical(lit: &Literal) -> Option<(CanonicalArgType, LiteralKind)> {
    match lit {
        Literal::Int(_) => Some((
            CanonicalArgType::Base("integer".to_string()),
            LiteralKind::Integer,
        )),
        Literal::Text(_) => Some((
            CanonicalArgType::Base("text".to_string()),
            LiteralKind::Text,
        )),
        Literal::Bool(_) => Some((
            CanonicalArgType::Base("boolean".to_string()),
            LiteralKind::Bool,
        )),
        Literal::Decimal(_) => Some((
            CanonicalArgType::Base("decimal".to_string()),
            LiteralKind::Decimal,
        )),
        Literal::Date(_) | Literal::DateTime(_) | Literal::Time(_) | Literal::Other(_) => None,
    }
}

// ---------------------------------------------------------------------------
// ArgDispatchInfo / ParamDispatchInfo
// ---------------------------------------------------------------------------

/// Per-call-site-argument-position dispatch info (I7: NOT a bare
/// `Option<String>` — canonical semantic type, literal origin, var-passable
/// flag, all threaded so the pick can apply the full hardened rule set).
#[derive(Debug)]
pub(crate) struct ArgDispatchInfo {
    /// The argument's canonical semantic type, when this increment can
    /// positively type it — `None` (untyped) for any expression shape this
    /// increment defers (call-result / `Rec.Field` / `Enum::Value` / …) OR a
    /// declared var whose type failed canonicalization. An untyped position
    /// degrades the WHOLE call (module doc) — it never eliminates a
    /// candidate.
    pub canonical: Option<CanonicalArgType>,
    /// The argument's FULL normalized type text (length included), when
    /// known — consulted ONLY by the `var`-mode ByRef-EXACT identity check
    /// ([`positions_compatible`]).
    pub exact_text: Option<String>,
    /// Set for a literal argument of a fixture-proven family; `None` for a
    /// declared-var argument (driven by `canonical` alone) or an unproven
    /// literal shape.
    pub literal_kind: Option<LiteralKind>,
    /// `true` when the argument expression is VAR-PASSABLE — a bare
    /// identifier naming a declared param/local/global in the CALLER's own
    /// scope (never a receiver/`with` scope). A literal or any other
    /// expression shape is never var-passable (sound elimination against a
    /// `var` parameter).
    pub var_passable: bool,
}

impl ArgDispatchInfo {
    fn untyped() -> Self {
        ArgDispatchInfo {
            canonical: None,
            exact_text: None,
            literal_kind: None,
            var_passable: false,
        }
    }
}

/// Per-candidate-parameter dispatch info — the param side of I7 ("the param
/// side carries canonical type + mode"). Always fully populated (SOURCE tier
/// `Param::by_ref` is a plain `bool`, never optional) — [`candidate_param_infos`]
/// returns `None` for the WHOLE candidate rather than a partially-populated
/// list when any position cannot be canonicalized.
pub(crate) struct ParamDispatchInfo {
    pub canonical: CanonicalArgType,
    /// FULL normalized type text (length included) — see
    /// [`ArgDispatchInfo::exact_text`]'s doc.
    pub exact_text: String,
    /// `var` (by-reference) parameter modifier.
    pub by_ref: bool,
}

/// Type every argument expression at `args` (in call-site order) — the
/// per-call-site entry point [`resolve::full::resolve_call_site_obligation`]
/// calls ONCE per obligation.
///
/// `routine`/`object_globals` supply the caller-scope-EXACT lookup chain
/// (params -> locals -> globals); `from` is the CALLING object's identity
/// (its app dependency closure is what an object-bearing arg type resolves
/// against). `with_state` is the call site's [`WithState`] (Task 2 review
/// fix) — a bare-identifier arg is typed from caller scope ONLY when this is
/// `NoWithProven`; see the module doc's "`with`-scope gate for
/// bare-identifier args" entry. `body_map` (T3, pageext-merge-and-final-
/// residual plan) is threaded ONLY so the new `Call` arm can re-run
/// `resolve_bare`/`resolve_member` on an INNER call-result expression — the
/// SAME `BodyMap` `resolve_call_site_obligation` already has in scope for
/// the OUTER obligation.
#[allow(clippy::too_many_arguments)] // 7 pre-existing params + `body_map` (T3, pageext-merge-and-final-residual plan).
pub(crate) fn type_call_args(
    args: &[ExprId],
    file: &AlFile,
    routine: &RoutineDecl,
    object_globals: &[VarDecl],
    from: &ObjectNodeId,
    graph: &ProgramGraph,
    index: &ResolveIndex,
    body_map: &BodyMap<'_>,
    with_state: WithState,
) -> Vec<ArgDispatchInfo> {
    args.iter()
        .map(|&id| {
            type_one_arg(
                file,
                file.ir.expr(id),
                routine,
                object_globals,
                from,
                graph,
                index,
                body_map,
                with_state,
            )
        })
        .collect()
}

#[allow(clippy::too_many_arguments)] // 7 pre-existing params + `body_map` (T3, pageext-merge-and-final-residual plan — the new `Call` arm's inner resolve_bare/resolve_member query).
fn type_one_arg(
    file: &AlFile,
    expr: &Expr,
    routine: &RoutineDecl,
    object_globals: &[VarDecl],
    from: &ObjectNodeId,
    graph: &ProgramGraph,
    index: &ResolveIndex,
    body_map: &BodyMap<'_>,
    with_state: WithState,
) -> ArgDispatchInfo {
    match &expr.kind {
        // `QuotedIdentifier` already stores the UNQUOTED name (the lowerer
        // strips quotes at lowering time — same convention `Param::name`/
        // `VarDecl::name` use), so both arms compare directly against the
        // caller's declared names with no extra unquoting.
        ExprKind::Identifier(name) | ExprKind::QuotedIdentifier(name) => {
            // `with`-scope gate (Task 2 review fix, module doc): a BARE
            // IDENTIFIER can be REBOUND by an enclosing `with X do` to the
            // with-receiver's member, which this caller-scope-EXACT lookup
            // structurally cannot see. Mirrors `resolve_bare`'s Step 3
            // with-guard exactly — `InsideWith`/`Unknown` degrade this
            // position to untyped rather than risk typing it against the
            // WRONG (caller-scope) declaration. A literal (the other arm
            // below) is unaffected — it cannot be rebound by `with`.
            if with_state != WithState::NoWithProven {
                return ArgDispatchInfo::untyped();
            }
            // Caller-scope-EXACT lookup: params -> locals -> the routine's
            // own named-return binding -> globals — the SHARED
            // `caller_scope_symbol` helper `receiver.rs`'s Step 2 also uses
            // (T3, receiver-closure-and-arg-increments plan; "one shared
            // helper... must not drift"), never a receiver/`with` scope. A
            // shadowing local/param with no declared type text still shadows
            // a same-named global (module doc) rather than falling through
            // to it; a SAME-SCOPE named-return/param/local collision
            // (malformed AL) degrades this position to untyped exactly like
            // a not-found symbol — never a guess at which one wins.
            match caller_scope_symbol(name, routine, object_globals) {
                CallerScopeSymbol::Found(Some(ty)) => ArgDispatchInfo {
                    canonical: dispatch_canonical_type_text(ty, from, graph, index),
                    exact_text: Some(normalize_type_text(ty)),
                    literal_kind: None,
                    var_passable: true,
                },
                // Found but no declared type text, not found at all in
                // caller scope, or a malformed same-scope duplicate —
                // untyped either way, never a guess.
                CallerScopeSymbol::Found(None)
                | CallerScopeSymbol::NotFound
                | CallerScopeSymbol::MalformedDuplicate => ArgDispatchInfo::untyped(),
            }
        }
        ExprKind::Literal(lit) => match literal_canonical(lit) {
            Some((canonical, kind)) => {
                let text = match &canonical {
                    CanonicalArgType::Base(s) => s.clone(),
                    // `literal_canonical` never produces `Object(..)`.
                    CanonicalArgType::Object(_) => {
                        unreachable!("literal_canonical only ever returns CanonicalArgType::Base")
                    }
                };
                ArgDispatchInfo {
                    canonical: Some(canonical),
                    exact_text: Some(text),
                    literal_kind: Some(kind),
                    var_passable: false,
                }
            }
            None => ArgDispatchInfo::untyped(),
        },
        // Member-field arg (Task 4, receiver-closure-and-arg-increments plan
        // — `Foo(Rec.Field)` / `Foo(Rec."Quoted Field")`): types the field's
        // DECLARED type via the SAME `field_in_table` machinery
        // `receiver.rs`'s record-field arm uses, gated identically:
        // - `with`-scope gate (module doc): the base identifier could be
        //   `with`-rebound, exactly like the bare-identifier arm above.
        // - Multi-hop guard: `object` must be a BARE Identifier/
        //   QuotedIdentifier — `Foo(A.B.Field)` (base itself a Member)
        //   declines, never partially typed.
        // - The base is resolved via caller-scope-EXACT `caller_scope_symbol`
        //   ONLY — deliberately NOT the implicit-Rec fallback
        //   (`receiver.rs`'s Step 3b): an implicit `Rec` with no DECLARED var
        //   in scope declines here (task brief: "implicit-Rec-without-
        //   declared-var base" is an explicit decline).
        // - The base's declared type must classify to `ParsedType::Record`
        //   and resolve to a real table in `from`'s dependency closure — a
        //   non-Record base or an unresolvable table declines.
        // - Routine-shadow guard (mirrors `receiver.rs`'s record-field arm
        //   EXACTLY): a same-named routine anywhere in the table's
        //   visibility-scoped surface declines — AL's parens-optional
        //   zero-arg call makes a bare `Member` structurally ambiguous
        //   between a field access and a parens-less routine call.
        // - `var_passable: false` HARDCODED (round-2 closer, BINDING: AL
        //   requires a VARIABLE for a `var` argument — "A variable is
        //   required" — `Rec.Amount` cannot bind a `var` parameter; a field
        //   expression is never itself a variable).
        ExprKind::Member { object, member, .. } => {
            if with_state != WithState::NoWithProven {
                return ArgDispatchInfo::untyped();
            }
            let base_name = match &file.ir.expr(*object).kind {
                ExprKind::Identifier(n) | ExprKind::QuotedIdentifier(n) => n,
                // Multi-hop base (itself a Member/Call/…) — out of this
                // increment's scope, decline rather than guess.
                _ => return ArgDispatchInfo::untyped(),
            };
            let CallerScopeSymbol::Found(Some(base_ty_text)) =
                caller_scope_symbol(base_name, routine, object_globals)
            else {
                // NotFound / Found(None) / MalformedDuplicate — includes the
                // implicit-Rec-without-declared-var case (task brief):
                // `caller_scope_symbol` never sees the implicit-Rec fallback.
                return ArgDispatchInfo::untyped();
            };
            let ParsedType::Record { table_ref } = classify_type_text(base_ty_text) else {
                return ArgDispatchInfo::untyped();
            };
            let table_id = match index.resolve_object_ref(
                graph,
                from.clone(),
                ObjectKind::Table,
                &table_ref,
            ) {
                ObjectRefResolution::Unique(id) => id,
                ObjectRefResolution::Ambiguous
                | ObjectRefResolution::OutOfClosure
                | ObjectRefResolution::Unresolved => return ArgDispatchInfo::untyped(),
            };
            let Some(from_object) = object_by_id(graph, from) else {
                return ArgDispatchInfo::untyped();
            };
            let field_lc = unquote_identifier(member).to_ascii_lowercase();
            if index.table_scope_has_routine(graph, from_object, &table_id, &field_lc) {
                return ArgDispatchInfo::untyped();
            }
            let Some(field) = index.field_in_table(graph, from_object, &table_id, &field_lc) else {
                return ArgDispatchInfo::untyped();
            };
            ArgDispatchInfo {
                canonical: dispatch_canonical_type_text(&field.type_text, from, graph, index),
                exact_text: Some(normalize_type_text(&field.type_text)),
                literal_kind: None,
                var_passable: false,
            }
        }
        // Call-result arg (T3, pageext-merge-and-final-residual plan): `Foo
        // (GetCount())` / `Foo(X.Method())` — types the INNER call's return
        // value, dispatching on the inner call's OWN `function` shape:
        // - Bare `Identifier`/`QuotedIdentifier` — mirrors Step 5's guards
        //   (`receiver::infer_call_result_receiver`): the local/param/global
        //   SHADOW guard, then a SINGLE-route `resolve_bare` query. See
        //   `type_call_result_arg_bare`'s doc.
        // - `Member{object, member}` — mirrors Step 6's cross-object-chain
        //   base typing (`receiver::infer_cross_object_chain_receiver`): the
        //   base is typed via the SAME caller-scope-EXACT path the plain
        //   `Member` arm above uses (WithState-gated), then a SINGLE-route
        //   `resolve_member` query. See `type_call_result_arg_member`'s doc.
        // - Anything else (a further-nested `Call`/`Index`/… as the
        //   function) — out of this increment's scope, declines.
        ExprKind::Call {
            function,
            args: inner_args,
        } => match &file.ir.expr(*function).kind {
            ExprKind::Identifier(fname) | ExprKind::QuotedIdentifier(fname) => {
                type_call_result_arg_bare(
                    fname,
                    inner_args.len(),
                    routine,
                    object_globals,
                    from,
                    graph,
                    index,
                    body_map,
                    with_state,
                )
            }
            ExprKind::Member {
                object: base_expr,
                member,
                ..
            } => type_call_result_arg_member(
                file,
                *base_expr,
                member,
                inner_args.len(),
                routine,
                object_globals,
                from,
                graph,
                index,
                body_map,
                with_state,
            ),
            _ => ArgDispatchInfo::untyped(),
        },
        // Boolean comparison/logical operators (T3, part b): AL defines
        // Eq/Ne/Lt/Le/Gt/Ge/And/Or/Xor/In UNCONDITIONALLY as Boolean-yielding
        // — no operand inspection needed or wanted (typing them from the
        // OPERATOR alone is exactly as sound as typing a literal, and avoids
        // recursing into arbitrarily complex operand sub-expressions). Every
        // other operator (arithmetic `Add`/`Sub`/`Mul`/`Div`/`IntDiv`/`Mod`,
        // and the catch-all `Other`) stays untyped — including a Text `+`
        // concatenation, which is NOT boolean-typed just because it shares
        // the `Add` variant with numeric addition (module doc's cardinal
        // rule: no guessing). Not itself a literal (`literal_kind: None` —
        // the C6 literal-forbidden-family gate never applies to a computed
        // Boolean) and never var-passable (an operator result is never a
        // variable).
        ExprKind::Binary { op, .. } => match op {
            BinaryOp::Eq
            | BinaryOp::Ne
            | BinaryOp::Lt
            | BinaryOp::Le
            | BinaryOp::Gt
            | BinaryOp::Ge
            | BinaryOp::And
            | BinaryOp::Or
            | BinaryOp::Xor
            | BinaryOp::In => ArgDispatchInfo {
                canonical: Some(CanonicalArgType::Base("boolean".to_string())),
                exact_text: Some("boolean".to_string()),
                literal_kind: None,
                var_passable: false,
            },
            BinaryOp::Add
            | BinaryOp::Sub
            | BinaryOp::Mul
            | BinaryOp::Div
            | BinaryOp::IntDiv
            | BinaryOp::Mod
            | BinaryOp::Other => ArgDispatchInfo::untyped(),
        },
        // Parenthesized unwrap (T3, part b): `Foo((A = B))` types identically
        // to its unwrapped inner expression — parens carry no type
        // information of their own.
        ExprKind::Parenthesized(inner) => type_one_arg(
            file,
            file.ir.expr(*inner),
            routine,
            object_globals,
            from,
            graph,
            index,
            body_map,
            with_state,
        ),
        // Deferred (increment-1 scope, module doc): `Enum::Value` / any
        // other expression shape stays untyped.
        _ => ArgDispatchInfo::untyped(),
    }
}

/// (a) bare-Identifier call-result arg (T3, pageext-merge-and-final-residual
/// plan, part a): mirrors `receiver::infer_call_result_receiver`'s (Step 5)
/// guards EXACTLY —
/// 1. **Local-shadowing guard FIRST**: a same-named param/local/global
///    SHADOWS a same-named procedure in AL (a local variable named
///    `GetCount` makes `GetCount()` an indexed/call-adjacent read on the
///    VARIABLE, never a routine call) — checked BEFORE ever consulting
///    `resolve_bare`, which cannot see this shadowing itself.
/// 2. **`resolve_bare` SINGLE-route query** (empty `args` — module doc:
///    "no recursion into `pick_candidate`"; `resolve_bare` is the thin
///    `args = &[]` wrapper, so this is automatic): a genuine same-object
///    overload ambiguity in the INNER call yields >1 routes, which the
///    `[route]` slice pattern declines on (the "2 same-arity inner
///    overloads -> untyped" negative fixture).
/// 3. **`RouteTarget::Routine`/`RouteTarget::AbiSymbol`** — read the
///    resolved routine's return type via [`call_result_arg_from_routine_node`]
///    (the Primitive-decline BYPASS — see that function's doc).
/// 4. **`RouteTarget::Builtin`** (part c) — consult the passive builtin-
///    return catalog ([`builtin_return_base_keyword`]), gated on
///    `resolve_bare` having POSITIVELY reported `Builtin` for this exact
///    name (never a bare name-string match — a source procedure that
///    SHADOWS one of the catalog's names resolves to `RouteTarget::Routine`
///    via Step 1 above, long before `resolve_bare` would ever report
///    `Builtin` for it, so the catalog is structurally unreachable for a
///    shadowed name).
/// 5. **`RouteTarget::Unresolved`** — untyped (name absent / arity mismatch
///    / an unproven builtin-precedence collision).
#[allow(clippy::too_many_arguments)]
fn type_call_result_arg_bare(
    fname: &str,
    inner_arity: usize,
    routine: &RoutineDecl,
    object_globals: &[VarDecl],
    from: &ObjectNodeId,
    graph: &ProgramGraph,
    index: &ResolveIndex,
    body_map: &BodyMap<'_>,
    with_state: WithState,
) -> ArgDispatchInfo {
    let fname_lc = fname.to_ascii_lowercase();
    let shadowed = routine
        .params
        .iter()
        .any(|p| p.name.to_ascii_lowercase() == fname_lc)
        || routine
            .locals
            .iter()
            .any(|v| v.name.to_ascii_lowercase() == fname_lc)
        || object_globals
            .iter()
            .any(|v| v.name.to_ascii_lowercase() == fname_lc);
    if shadowed {
        return ArgDispatchInfo::untyped();
    }
    let Some(from_object) = object_by_id(graph, from) else {
        return ArgDispatchInfo::untyped();
    };
    let (_shape, routes) = resolve_bare(
        from_object,
        &fname_lc,
        inner_arity,
        graph,
        index,
        body_map,
        with_state,
    );
    let [route] = routes.as_slice() else {
        return ArgDispatchInfo::untyped();
    };
    if let RouteTarget::Builtin(BuiltinId(name)) = &route.target {
        return match builtin_return_base_keyword(name) {
            Some(kw) => ArgDispatchInfo {
                canonical: Some(CanonicalArgType::Base(kw.to_string())),
                exact_text: Some(kw.to_string()),
                literal_kind: None,
                var_passable: false,
            },
            None => ArgDispatchInfo::untyped(),
        };
    }
    let Some(node) = routine_node_for_type_query(route, inner_arity, from_object, graph, index)
    else {
        return ArgDispatchInfo::untyped();
    };
    call_result_arg_from_routine_node(node, from, graph, index)
}

/// (b) `Member`-function call-result arg (T3, part b): `Foo(X.Method())` —
/// mirrors `receiver::infer_cross_object_chain_receiver`'s (Step 6) base
/// typing + single-route contract:
/// 1. **`with`-scope gate FIRST** (mirrors the plain `Member`-field arm
///    above EXACTLY): the base identifier could be `with`-rebound, which the
///    caller-scope-EXACT lookup below structurally cannot see.
/// 2. **Bare-identifier base guard**: `object` must be a bare
///    `Identifier`/`QuotedIdentifier` — a multi-hop base (`A.B.Method()`)
///    declines rather than guess.
/// 3. **Caller-scope-EXACT base typing**: the base's declared type, via the
///    SAME `caller_scope_symbol` lookup the plain `Member` arm uses —
///    deliberately NOT the implicit-Rec fallback (same rationale as that
///    arm: an implicit `Rec` with no declared var in scope declines here).
/// 4. **`resolve_member` SINGLE-route query**: `base_receiver.Method()`'s
///    resolved route set must be EXACTLY one route (an interface fan-out or
///    a genuine same-object overload ambiguity — >1 routes — declines,
///    never a guessed pick); a `RouteTarget::Unresolved`/`Builtin` target
///    also declines (no member-builtin return catalog exists in this
///    increment — the passive catalog is bare-global-function-only, part
///    c).
/// 5. **Return-type read** via [`call_result_arg_from_routine_node`] (the
///    Primitive-decline bypass).
#[allow(clippy::too_many_arguments)]
fn type_call_result_arg_member(
    file: &AlFile,
    base_expr: ExprId,
    member: &str,
    inner_arity: usize,
    routine: &RoutineDecl,
    object_globals: &[VarDecl],
    from: &ObjectNodeId,
    graph: &ProgramGraph,
    index: &ResolveIndex,
    body_map: &BodyMap<'_>,
    with_state: WithState,
) -> ArgDispatchInfo {
    if with_state != WithState::NoWithProven {
        return ArgDispatchInfo::untyped();
    }
    let base_name = match &file.ir.expr(base_expr).kind {
        ExprKind::Identifier(n) | ExprKind::QuotedIdentifier(n) => n,
        // Multi-hop base (itself a Member/Call/…) — out of this increment's
        // scope, decline rather than guess.
        _ => return ArgDispatchInfo::untyped(),
    };
    let CallerScopeSymbol::Found(Some(base_ty_text)) =
        caller_scope_symbol(base_name, routine, object_globals)
    else {
        return ArgDispatchInfo::untyped();
    };
    let Some(from_object) = object_by_id(graph, from) else {
        return ArgDispatchInfo::untyped();
    };
    let base_receiver =
        parsed_type_to_receiver(classify_type_text(base_ty_text), from_object, graph, index);
    let member_lc = unquote_identifier(member).to_ascii_lowercase();
    let (_shape, routes) = resolve_member(
        &base_receiver,
        &member_lc,
        inner_arity,
        from_object,
        graph,
        index,
        body_map,
    );
    let [route] = routes.as_slice() else {
        return ArgDispatchInfo::untyped();
    };
    if matches!(
        route.target,
        RouteTarget::Unresolved | RouteTarget::Builtin(_)
    ) {
        return ArgDispatchInfo::untyped();
    }
    let Some(node) = routine_node_for_type_query(route, inner_arity, from_object, graph, index)
    else {
        return ArgDispatchInfo::untyped();
    };
    call_result_arg_from_routine_node(node, from, graph, index)
}

/// Read a resolved call-result routine's return type as an
/// [`ArgDispatchInfo`], applying the SAME structural safety guards
/// [`receiver::receiver_from_routine_node`] (private to that module) applies
/// to a call-result RECEIVER base — the `abi_overload_collapsed` short-
/// circuit AND the `return_type_id` ABI structured cross-validation (Task 2,
/// cross-object-chains plan: the ABI's own declared Subtype `(name, id)`
/// pair must agree with the object the NAME resolves to, or the signal is
/// untrustworthy and declines) — WITHOUT that function's Primitive-decline
/// (T3 plan addendum, BINDING: "the Primitive-decline bypass keeps every
/// other guard verbatim" — an ARGUMENT position WANTS exactly the
/// scalar/primitive shapes a receiver dispatch base would reject, since a
/// primitive has no further members to dispatch on but is exactly what an
/// argument position needs).
///
/// `routine_node_for_type_query` (the caller's own choke point) ALREADY
/// applies the `abi_overload_collapsed` check and the ABI-PREFIX UNIQUENESS
/// GUARD before this function ever sees `node` — the check here is defense-
/// in-depth (mirrors `receiver_from_routine_node`'s own re-check for the
/// `interface_own_routine_node` path it must cover).
fn call_result_arg_from_routine_node(
    node: &RoutineNode,
    from: &ObjectNodeId,
    graph: &ProgramGraph,
    index: &ResolveIndex,
) -> ArgDispatchInfo {
    if node.abi_overload_collapsed {
        return ArgDispatchInfo::untyped();
    }
    let Some(ty_text) = node.return_type.as_deref() else {
        return ArgDispatchInfo::untyped();
    };
    let canonical = dispatch_canonical_type_text(ty_text, from, graph, index);
    if let Some((_name, id)) = &node.return_type_id {
        match &canonical {
            Some(CanonicalArgType::Object(oid)) => match object_by_id(graph, oid) {
                Some(obj) if obj.declared_id == Some(*id) => {}
                _ => return ArgDispatchInfo::untyped(),
            },
            _ => return ArgDispatchInfo::untyped(),
        }
    }
    match canonical {
        Some(c) => ArgDispatchInfo {
            canonical: Some(c),
            exact_text: Some(normalize_type_text(ty_text)),
            literal_kind: None,
            var_passable: false,
        },
        None => ArgDispatchInfo::untyped(),
    }
}

/// Passive builtin-return catalog (T3, part c): the return TYPE (base
/// keyword, length-independent) of a small, high-value set of ubiquitous AL
/// GLOBAL builtin functions. Consulted ONLY after `resolve_bare` has
/// POSITIVELY reported `RouteTarget::Builtin` for the SAME name (never a
/// bare name-string match — see [`type_call_result_arg_bare`]'s doc for why
/// a shadowed name never reaches this catalog at all). Per-entry cited
/// against the `methods-auto` Microsoft Learn reference (mirrors
/// `framework_returns.rs`'s per-entry citation discipline); an uncataloged
/// builtin name (`RouteTarget::Builtin` for a name not listed here) stays
/// untyped, never guessed.
///
/// - `StrSubstNo`: <https://learn.microsoft.com/en-us/dynamics365/business-central/dev-itpro/developer/methods-auto/text/text-strsubstno-method>
/// - `Format`: <https://learn.microsoft.com/en-us/dynamics365/business-central/dev-itpro/developer/methods-auto/system/system-format-joker-integer-integer-method>
/// - `CopyStr`: <https://learn.microsoft.com/en-us/dynamics365/business-central/dev-itpro/developer/methods-auto/text/text-copystr-method>
/// - `LowerCase`: <https://learn.microsoft.com/en-us/dynamics365/business-central/dev-itpro/developer/methods-auto/text/text-lowercase-method>
/// - `UpperCase`: <https://learn.microsoft.com/en-us/dynamics365/business-central/dev-itpro/developer/methods-auto/text/text-uppercase-method>
/// - `Round`: <https://learn.microsoft.com/en-us/dynamics365/business-central/dev-itpro/developer/methods-auto/system/system-round-method>
/// - `StrLen`: <https://learn.microsoft.com/en-us/dynamics365/business-central/dev-itpro/developer/methods-auto/text/text-strlen-method>
const BUILTIN_RETURN_TEXT_CATALOG: &[(&str, &str)] = &[
    ("strsubstno", "text"),
    ("format", "text"),
    ("copystr", "text"),
    ("lowercase", "text"),
    ("uppercase", "text"),
    ("round", "decimal"),
    ("strlen", "integer"),
];

/// Look up `name_lc` in [`BUILTIN_RETURN_TEXT_CATALOG`] — `None` for any
/// name not listed (fail-closed: absence is untyped, never a guess).
fn builtin_return_base_keyword(name_lc: &str) -> Option<&'static str> {
    BUILTIN_RETURN_TEXT_CATALOG
        .iter()
        .find(|(n, _)| *n == name_lc)
        .map(|(_, t)| *t)
}

/// Build the full [`ParamDispatchInfo`] list for one candidate's parameters,
/// as seen from `from` (the CANDIDATE's OWN declaring object identity — an
/// object-bearing param type resolves against the object that DECLARED the
/// routine, not the caller). Returns `None` — "missing candidate metadata",
/// degrading the WHOLE call per the module doc — when ANY parameter has no
/// declared type text, or its declared type fails canonicalization
/// (unresolvable object reference).
///
/// # `parse_incomplete` gate (Task 2 review, Finding 3)
///
/// `decl.params` is trusted verbatim below — but a `parse_incomplete` decl
/// means the parser already recovered from a syntax error somewhere INSIDE
/// this routine's own declaration, and a param TYPE is the very first place
/// this module lets candidate metadata adjudicate between overloads. A
/// recovery artifact masquerading as a legitimate declared type there could
/// feed a confident (and possibly WRONG) pick. Fail closed uniformly with
/// every other codebase consumer of this flag (`engine::l5`'s detectors,
/// `l3_workspace`'s coverage report, etc. all skip a `parse_incomplete`
/// routine rather than trust its recovered shape): treat it exactly like
/// missing candidate metadata, degrading the WHOLE call, never a partial or
/// best-effort read of the recovered params.
pub(crate) fn candidate_param_infos(
    decl: &RoutineDecl,
    from: &ObjectNodeId,
    graph: &ProgramGraph,
    index: &ResolveIndex,
) -> Option<Vec<ParamDispatchInfo>> {
    if decl.parse_incomplete {
        return None;
    }
    let mut out = Vec::with_capacity(decl.params.len());
    for p in &decl.params {
        let ty = p.ty.as_deref()?;
        let canonical = dispatch_canonical_type_text(ty, from, graph, index)?;
        out.push(ParamDispatchInfo {
            canonical,
            exact_text: normalize_type_text(ty),
            by_ref: p.by_ref,
        });
    }
    Some(out)
}

// ---------------------------------------------------------------------------
// Compatibility + pick
// ---------------------------------------------------------------------------

/// Base keywords AL genuinely lets flow into one another via implicit
/// conversion at the language level (Text<->Code<->Char<->Label narrowing/
/// widening; Integer<->Decimal<->BigInteger numeric widening) — two DIFFERENT
/// keywords in the SAME family can never be PICKED-between (still exact-
/// keyword-only for equality, per the module doc), but a mismatch there is
/// NOT a proven-incompatible elimination either: it is UNDECIDED, and an
/// undecided candidate blocks a confident pick of anything else exactly like
/// an unmatched twin would (see [`position_provably_incompatible`]). A
/// mismatch OUTSIDE these families (e.g. `InStream` vs `Text`, or `Integer`
/// vs a Framework/object type) IS a proven, structural incompatibility — two
/// fundamentally disjoint runtime representations that no AL conversion
/// bridges — and eliminates the candidate.
///
/// This grouping applies UNIFORMLY to literal AND declared-var arguments — a
/// conservative superset of the plan's C6 addendum, which frames the
/// STRING-vs-Code/Char and INTEGER-vs-Decimal/BigInteger degrade rules as
/// literal-specific. The same real-world AL assignability reasoning applies
/// identically to a declared var of that family (a `Text`-typed variable is
/// exactly as "text-ish-adjacent" to `Code[20]` as a text literal is), and
/// erring toward the STRICTER (never-picks-when-undecided) reading is the
/// sound direction per the module doc's cardinal rule — it can only ever
/// produce FEWER picks than a literal-only scoping would, never a wrong one.
/// [`literal_forbidden_families`] additionally encodes the plan's literal
/// wording VERBATIM (redundant with this grouping for the two named pairs,
/// kept for direct traceability to the addendum text).
const TEXT_ISH_FAMILY: &[&str] = &["text", "code", "char", "label"];
const NUMERIC_FAMILY: &[&str] = &["integer", "decimal", "biginteger"];

/// Whether two DIFFERENT Base keywords belong to the same soft (non-
/// eliminating) family — see [`TEXT_ISH_FAMILY`]/[`NUMERIC_FAMILY`]'s doc.
/// Callers only invoke this after already establishing `a != b`.
fn same_soft_family(a: &str, b: &str) -> bool {
    (TEXT_ISH_FAMILY.contains(&a) && TEXT_ISH_FAMILY.contains(&b))
        || (NUMERIC_FAMILY.contains(&a) && NUMERIC_FAMILY.contains(&b))
}

/// The plan's C6 literal-typing wording, verbatim: the Base families a
/// literal of this [`LiteralKind`] is "contextually usable as but unproven"
/// — their mere PRESENCE in the candidate set at that position degrades the
/// call (see [`pick_candidate`]'s literal gate). Empty for a family with no
/// documented pair (Bool/Decimal — no fixture-proven cross-family literal
/// rule exists for them in this increment).
fn literal_forbidden_families(kind: LiteralKind) -> &'static [&'static str] {
    match kind {
        LiteralKind::Text => &["code", "char"],
        LiteralKind::Integer => &["decimal", "biginteger"],
        LiteralKind::Bool | LiteralKind::Decimal => &[],
    }
}

/// Whether `arg` EXACTLY matches `param` at one position — canonical-
/// identity EQUALITY (the ONLY basis for a pick — see the module doc's
/// cardinal rule), plus the `var`-mode ByRef-EXACT tightening (Round-2
/// closer C5).
fn position_exact_match(arg: &ArgDispatchInfo, param: &ParamDispatchInfo) -> bool {
    let Some(arg_canonical) = &arg.canonical else {
        return false;
    };
    if arg_canonical != &param.canonical {
        return false;
    }
    if param.by_ref {
        // A literal/call-result argument can never bind a `var` parameter —
        // sound elimination (module doc).
        if !arg.var_passable {
            return false;
        }
        // Length-EXACT identity for `var`, Base (scalar/Text/Code) types
        // only — object-bearing types have no "length" concept; the
        // canonical Object(..) equality above already IS their exact
        // identity.
        if matches!(param.canonical, CanonicalArgType::Base(_)) {
            match &arg.exact_text {
                Some(a_text) if *a_text == param.exact_text => {}
                _ => return false,
            }
        }
    }
    true
}

/// Whether `param` is PROVEN incompatible with `arg` at one position — the
/// ELIMINATION test a non-picked candidate must satisfy at some position for
/// its presence to NOT block the pick (see [`pick_candidate`]). Distinct from
/// [`position_exact_match`]: a candidate can be neither an exact match NOR
/// provably incompatible (e.g. `Text` vs `Code[20]` — see [`same_soft_
/// family`]'s doc) — that UNDECIDED middle ground blocks a confident pick of
/// anything else, exactly like a genuine second exact match would.
fn position_provably_incompatible(arg: &ArgDispatchInfo, param: &ParamDispatchInfo) -> bool {
    let Some(arg_canonical) = &arg.canonical else {
        return false;
    };
    if arg_canonical != &param.canonical {
        return !matches!(
            (arg_canonical, &param.canonical),
            (CanonicalArgType::Base(a), CanonicalArgType::Base(b)) if same_soft_family(a, b)
        );
    }
    // Canonical types match — still provably incompatible when `var` mode
    // requires exact length (or var-passability) and it doesn't hold (C5):
    // a `var Text[30]` argument LITERALLY cannot bind a `var Text[50]`
    // parameter, and a literal/call-result argument literally cannot bind
    // any `var` parameter.
    if param.by_ref {
        if !arg.var_passable {
            return true;
        }
        if matches!(param.canonical, CanonicalArgType::Base(_))
            && arg.exact_text.as_deref() != Some(param.exact_text.as_str())
        {
            return true;
        }
    }
    false
}

/// Attempt the Task 2 fail-closed pick over a prevalidated, same-name/
/// same-arity, all-CONCRETE candidate set (every entry of `candidates` is
/// parallel — by index — to the caller's own candidate `RoutineNodeId` list).
///
/// Returns `Some(index)` iff EXACTLY ONE candidate EXACTLY matches `args`
/// AND every OTHER candidate is PROVABLY INCOMPATIBLE with `args` at some
/// position — an "undecided" (same-soft-family, non-exact) competitor blocks
/// the pick just like a second exact match would, since its presence means
/// the closed candidate set is not provably narrowed to one. `None` for
/// every other outcome (any untyped arg position, a Variant/Any param at a
/// discriminating position, a literal-forbidden-family candidate present, 0
/// or >1 exact matches, an undecided non-picked candidate) — the caller's
/// existing `AmbiguousOverload` construction is UNCHANGED whenever this
/// returns `None`.
pub(crate) fn pick_candidate(
    args: &[ArgDispatchInfo],
    candidates: &[Vec<ParamDispatchInfo>],
) -> Option<usize> {
    if args.is_empty() || candidates.len() < 2 {
        return None;
    }
    // Call-level degradation: EVERY supplied arg must be typed.
    if args.iter().any(|a| a.canonical.is_none()) {
        return None;
    }
    for pos in 0..args.len() {
        if candidates.iter().any(|c| pos >= c.len()) {
            // Arity mismatch inside the candidate set — should not happen
            // (every candidate here was already arity-filtered by the
            // caller), but fail closed rather than index out of bounds.
            return None;
        }
        // Variant/Any wildcard gate — "discriminating position" computed
        // from the FULL candidate set BEFORE any compatibility filtering
        // (I9).
        let types_at_pos: Vec<&CanonicalArgType> =
            candidates.iter().map(|c| &c[pos].canonical).collect();
        let by_ref_at_pos: Vec<bool> = candidates.iter().map(|c| c[pos].by_ref).collect();
        let discriminating = types_at_pos.windows(2).any(|w| w[0] != w[1])
            || by_ref_at_pos.windows(2).any(|w| w[0] != w[1]);
        if discriminating && types_at_pos.iter().any(|t| t.is_variant_or_any()) {
            return None;
        }
        // C6 literal-forbidden-family gate, stated verbatim (module doc):
        // the MERE PRESENCE of a "contextually usable but unproven" target
        // family at this position degrades the whole call.
        if let Some(lk) = args[pos].literal_kind {
            let forbidden = literal_forbidden_families(lk);
            if !forbidden.is_empty()
                && types_at_pos.iter().any(
                    |t| matches!(t, CanonicalArgType::Base(b) if forbidden.contains(&b.as_str())),
                )
            {
                return None;
            }
        }
    }

    let mut exact_idx: Option<usize> = None;
    for (i, params) in candidates.iter().enumerate() {
        if args.len() == params.len()
            && args
                .iter()
                .zip(params.iter())
                .all(|(a, p)| position_exact_match(a, p))
        {
            if exact_idx.is_some() {
                // A second exact match: ordinary ambiguity, never pick.
                return None;
            }
            exact_idx = Some(i);
        }
    }
    let picked = exact_idx?;

    // Every OTHER candidate must be PROVEN incompatible at some position —
    // an undecided competitor blocks the pick (doc above).
    for (i, params) in candidates.iter().enumerate() {
        if i == picked {
            continue;
        }
        let eliminated = args
            .iter()
            .zip(params.iter())
            .any(|(a, p)| position_provably_incompatible(a, p));
        if !eliminated {
            return None;
        }
    }
    Some(picked)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::program::node::{AppRef, ObjKey};

    /// A minimal, arbitrary `ObjectNodeId` for tests that only need `from`'s
    /// identity as an opaque scope parameter (no object actually resolved
    /// against it — every test graph below is empty).
    fn test_object_id() -> ObjectNodeId {
        ObjectNodeId {
            app: AppRef(0),
            kind: al_syntax::ir::ObjectKind::Codeunit,
            key: ObjKey::Id(1),
        }
    }

    fn base_arg(kw: &str) -> ArgDispatchInfo {
        ArgDispatchInfo {
            canonical: Some(CanonicalArgType::Base(kw.to_string())),
            exact_text: Some(kw.to_string()),
            literal_kind: None,
            var_passable: true,
        }
    }

    fn base_param(kw: &str, by_ref: bool) -> ParamDispatchInfo {
        ParamDispatchInfo {
            canonical: CanonicalArgType::Base(kw.to_string()),
            exact_text: kw.to_string(),
            by_ref,
        }
    }

    // -- base_keyword -------------------------------------------------------

    #[test]
    fn base_keyword_strips_length_and_lowercases() {
        assert_eq!(base_keyword("Text[30]"), "text");
        assert_eq!(base_keyword("Code[20]"), "code");
        assert_eq!(base_keyword("Integer"), "integer");
        assert_eq!(base_keyword("  Decimal  "), "decimal");
    }

    // -- pick_candidate: positive shapes -------------------------------------

    /// Two candidates, one discriminating position, arg matches candidate 1
    /// exactly — a clean pick.
    #[test]
    fn pick_candidate_selects_the_sole_exact_match() {
        let args = vec![base_arg("instream")];
        let candidates = vec![
            vec![base_param("text", false)],
            vec![base_param("instream", false)],
        ];
        assert_eq!(pick_candidate(&args, &candidates), Some(1));
    }

    /// A non-discriminating position (both candidates share it) must not
    /// block a pick driven by a genuinely discriminating position.
    #[test]
    fn pick_candidate_ignores_non_discriminating_position() {
        let args = vec![base_arg("instream"), base_arg("integer")];
        let candidates = vec![
            vec![base_param("text", false), base_param("integer", false)],
            vec![base_param("instream", false), base_param("integer", false)],
        ];
        assert_eq!(pick_candidate(&args, &candidates), Some(1));
    }

    // -- pick_candidate: negative shapes (mandatory per plan addenda) -------

    /// Same-family scalars that BOTH exactly equal neither... here both
    /// candidates are canonically IDENTICAL post length-stripping
    /// (`Text[30]` and `Text[50]` -> `"text"`) — "can never be picked
    /// between" (Round-1 addendum (a)): any arg compatible with one is
    /// compatible with both, so >1 survive.
    #[test]
    fn pick_candidate_degrades_when_length_stripping_collapses_candidates_to_identical() {
        let args = vec![base_arg("text")];
        let candidates = vec![
            vec![ParamDispatchInfo {
                canonical: CanonicalArgType::Base("text".into()),
                exact_text: "text[30]".into(),
                by_ref: false,
            }],
            vec![ParamDispatchInfo {
                canonical: CanonicalArgType::Base("text".into()),
                exact_text: "text[50]".into(),
                by_ref: false,
            }],
        ];
        assert_eq!(pick_candidate(&args, &candidates), None);
    }

    /// `Code[20]` vs `Code[30]` — same shape as the Text case above, the
    /// Code-family sibling.
    #[test]
    fn pick_candidate_degrades_for_code_length_collapse() {
        let args = vec![base_arg("code")];
        let candidates = vec![
            vec![ParamDispatchInfo {
                canonical: CanonicalArgType::Base("code".into()),
                exact_text: "code[20]".into(),
                by_ref: false,
            }],
            vec![ParamDispatchInfo {
                canonical: CanonicalArgType::Base("code".into()),
                exact_text: "code[30]".into(),
                by_ref: false,
            }],
        ];
        assert_eq!(pick_candidate(&args, &candidates), None);
    }

    /// `var Text[30]` arg vs `var Text[50]` param at a DISCRIMINATING
    /// position (a sibling candidate has a different, non-var param there) —
    /// Round-2 closer C5: length is INCLUDED for `var` exact identity, so
    /// the length-mismatched candidate is eliminated (not degraded) while
    /// the other survives alone.
    #[test]
    fn pick_candidate_var_param_requires_exact_length() {
        let arg = ArgDispatchInfo {
            canonical: Some(CanonicalArgType::Base("text".into())),
            exact_text: Some("text[30]".into()),
            literal_kind: None,
            var_passable: true,
        };
        let candidates = vec![
            // by_ref length-mismatched sibling — eliminated by the ByRef-
            // EXACT check.
            vec![ParamDispatchInfo {
                canonical: CanonicalArgType::Base("text".into()),
                exact_text: "text[50]".into(),
                by_ref: true,
            }],
            // by-value candidate: length is NOT discriminating for by-value,
            // canonical "text" == "text" -> compatible.
            vec![ParamDispatchInfo {
                canonical: CanonicalArgType::Base("text".into()),
                exact_text: "text[99]".into(),
                by_ref: false,
            }],
        ];
        assert_eq!(pick_candidate(&[arg], &candidates), Some(1));
    }

    /// A literal argument is never var-passable — a `var` parameter is
    /// UNCONDITIONALLY incompatible with it, regardless of type match
    /// (mandatory negative: "var-param-with-literal").
    #[test]
    fn pick_candidate_var_param_rejects_literal_argument() {
        let args = vec![ArgDispatchInfo {
            canonical: Some(CanonicalArgType::Base("integer".into())),
            exact_text: Some("integer".into()),
            literal_kind: Some(LiteralKind::Integer),
            var_passable: false,
        }];
        let candidates = vec![
            vec![base_param("integer", true)],
            vec![base_param("text", false)],
        ];
        assert_eq!(pick_candidate(&args, &candidates), None);
    }

    /// Variant wildcard: a Variant param at a discriminating position
    /// degrades the WHOLE call, even though a naive exclusion-style matcher
    /// would have eliminated the OTHER (non-Variant) candidate and left
    /// Variant as the sole "survivor" — that survivor-by-elimination is
    /// UNPROVEN, not a confident pick (Round-1 addendum I5).
    #[test]
    fn pick_candidate_degrades_on_variant_at_discriminating_position() {
        let args = vec![base_arg("instream")];
        let candidates = vec![
            vec![base_param("variant", false)],
            vec![base_param("integer", false)],
        ];
        assert_eq!(pick_candidate(&args, &candidates), None);
    }

    /// An untyped argument position degrades the whole call, never merely
    /// "skips" that position.
    #[test]
    fn pick_candidate_degrades_on_untyped_argument() {
        let args = vec![ArgDispatchInfo::untyped()];
        let candidates = vec![
            vec![base_param("text", false)],
            vec![base_param("instream", false)],
        ];
        assert_eq!(pick_candidate(&args, &candidates), None);
    }

    /// Zero compatible candidates (arg canonically matches neither) is a
    /// stay-ambiguous outcome, never an error.
    #[test]
    fn pick_candidate_none_compatible_stays_ambiguous() {
        let args = vec![base_arg("variant")];
        let candidates = vec![
            vec![base_param("text", false)],
            vec![base_param("integer", false)],
        ];
        assert_eq!(pick_candidate(&args, &candidates), None);
    }

    /// Mandatory negative ("same-family scalars -> no pick", `ws-overload-
    /// negatives`' `CallIndistinct`): a DECLARED-VAR `Text` argument exactly
    /// matches an `(Integer, Text)` candidate, but a sibling `(Integer,
    /// Code[20])` candidate is UNDECIDED (Text/Code same soft family, module
    /// doc) rather than eliminated — the undecided competitor blocks the
    /// pick even though it is not itself an exact match.
    #[test]
    fn pick_candidate_declared_var_text_vs_code_stays_undecided() {
        let args = vec![base_arg("integer"), base_arg("text")];
        let candidates = vec![
            vec![base_param("integer", false), base_param("text", false)],
            vec![base_param("integer", false), base_param("code", false)],
        ];
        assert_eq!(pick_candidate(&args, &candidates), None);
    }

    /// C6, stated verbatim: a STRING literal degrades the call whenever the
    /// candidate set contains a Code/Char candidate at that position — even
    /// though the OTHER candidate (Text) would otherwise be a clean sole
    /// exact match.
    #[test]
    fn pick_candidate_text_literal_degrades_on_code_candidate_present() {
        let args = vec![ArgDispatchInfo {
            canonical: Some(CanonicalArgType::Base("text".into())),
            exact_text: Some("text".into()),
            literal_kind: Some(LiteralKind::Text),
            var_passable: false,
        }];
        let candidates = vec![
            vec![base_param("text", false)],
            vec![base_param("code", false)],
        ];
        assert_eq!(pick_candidate(&args, &candidates), None);
    }

    /// C6's compiler-proven exemplar: an INTEGER literal is NOT in the
    /// literal-forbidden-family list for `Code` (only Decimal/BigInteger
    /// are), so ordinary exact-canonical-mismatch elimination applies — the
    /// `ws-overload-collision` flip's underlying mechanism, pinned directly
    /// here at the unit level.
    #[test]
    fn pick_candidate_integer_literal_eliminates_code_candidate() {
        let args = vec![ArgDispatchInfo {
            canonical: Some(CanonicalArgType::Base("integer".into())),
            exact_text: Some("integer".into()),
            literal_kind: Some(LiteralKind::Integer),
            var_passable: false,
        }];
        let candidates = vec![
            vec![base_param("integer", false)],
            vec![base_param("code", false)],
        ];
        assert_eq!(pick_candidate(&args, &candidates), Some(0));
    }

    /// C6's OTHER named pair: an INTEGER literal degrades when a Decimal/
    /// BigInteger candidate is present.
    #[test]
    fn pick_candidate_integer_literal_degrades_on_decimal_candidate_present() {
        let args = vec![ArgDispatchInfo {
            canonical: Some(CanonicalArgType::Base("integer".into())),
            exact_text: Some("integer".into()),
            literal_kind: Some(LiteralKind::Integer),
            var_passable: false,
        }];
        let candidates = vec![
            vec![base_param("integer", false)],
            vec![base_param("decimal", false)],
        ];
        assert_eq!(pick_candidate(&args, &candidates), None);
    }

    // -- type_one_arg: caller-scope-EXACT shadowing --------------------------

    fn test_origin() -> al_syntax::ir::Origin {
        al_syntax::ir::Origin {
            kind_text: "",
            ts_id: 0,
            byte: 0..0,
            start: al_syntax::ir::Point { row: 0, column: 0 },
            end: al_syntax::ir::Point { row: 0, column: 0 },
        }
    }

    fn param(name: &str, ty: &str) -> al_syntax::ir::Param {
        al_syntax::ir::Param {
            name: name.to_string(),
            by_ref: false,
            ty: Some(ty.to_string()),
            origin: test_origin(),
        }
    }

    fn var(name: &str, ty: &str) -> VarDecl {
        VarDecl {
            name: name.to_string(),
            ty: Some(ty.to_string()),
            temporary: false,
            origin: test_origin(),
        }
    }

    fn empty_routine() -> RoutineDecl {
        RoutineDecl {
            kind: al_syntax::ir::RoutineKind::Procedure,
            name: "Test".to_string(),
            name_origin: test_origin(),
            params: vec![],
            return_type: None,
            return_name: None,
            locals: vec![],
            attributes: vec![],
            attributes_parsed: vec![],
            access_modifier: None,
            parse_incomplete: false,
            dataitem_source_table: None,
            enclosing_member: None,
            in_dataset_modify_context: false,
            body: None,
            origin: test_origin(),
        }
    }

    fn ident_expr(name: &str) -> Expr {
        Expr {
            kind: ExprKind::Identifier(name.to_string()),
            origin: test_origin(),
        }
    }

    /// A minimal empty `AlFile` for `type_one_arg` tests that don't exercise
    /// the Member arm (Task 4 — `type_one_arg` now takes `file: &AlFile` so
    /// that arm can dereference the base `ExprId`; every OTHER arm never
    /// touches `file` at all, so an empty one is behavior-neutral here).
    fn empty_file() -> AlFile {
        AlFile {
            objects: vec![],
            ir: al_syntax::ir::Ir::new(),
            issues: vec![],
            parse_status: al_syntax::ir::ParseStatus::Clean,
        }
    }

    /// A local var of the same name shadows a same-named global — the
    /// caller-scope lookup must resolve to the LOCAL's declared type, never
    /// fall through to the global.
    #[test]
    fn type_one_arg_local_shadows_global() {
        let mut routine = empty_routine();
        routine.locals.push(var("X", "Integer"));
        let globals = vec![var("X", "Text")];
        let graph = ProgramGraph::default();
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &[]);
        let from = test_object_id();

        let e = ident_expr("X");
        let info = type_one_arg(
            &empty_file(),
            &e,
            &routine,
            &globals,
            &from,
            &graph,
            &index,
            &body_map,
            WithState::NoWithProven,
        );
        assert_eq!(
            info.canonical,
            Some(CanonicalArgType::Base("integer".into())),
            "a local must shadow a same-named global"
        );
        assert!(info.var_passable);
    }

    /// A param of the same name shadows both a local and a global.
    #[test]
    fn type_one_arg_param_shadows_local_and_global() {
        let mut routine = empty_routine();
        routine.params.push(param("X", "Boolean"));
        routine.locals.push(var("X", "Integer"));
        let globals = vec![var("X", "Text")];
        let graph = ProgramGraph::default();
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &[]);
        let from = test_object_id();

        let e = ident_expr("X");
        let info = type_one_arg(
            &empty_file(),
            &e,
            &routine,
            &globals,
            &from,
            &graph,
            &index,
            &body_map,
            WithState::NoWithProven,
        );
        assert_eq!(
            info.canonical,
            Some(CanonicalArgType::Base("boolean".into())),
            "a param must shadow both a same-named local and a same-named global"
        );
    }

    /// A literal is never var-passable.
    #[test]
    fn type_one_arg_literal_is_not_var_passable() {
        let routine = empty_routine();
        let graph = ProgramGraph::default();
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &[]);
        let from = test_object_id();

        let e = Expr {
            kind: ExprKind::Literal(Literal::Int("5".to_string())),
            origin: test_origin(),
        };
        let info = type_one_arg(
            &empty_file(),
            &e,
            &routine,
            &[],
            &from,
            &graph,
            &index,
            &body_map,
            WithState::NoWithProven,
        );
        assert_eq!(
            info.canonical,
            Some(CanonicalArgType::Base("integer".into()))
        );
        assert_eq!(info.literal_kind, Some(LiteralKind::Integer));
        assert!(!info.var_passable);
    }

    // -- type_one_arg: `with`-scope gate (Task 2 review fix, Finding 1) -----

    /// The dormant wrong-pick vector, directly at the `type_one_arg` unit
    /// level: a bare identifier that resolves cleanly in caller scope
    /// (`WithState::NoWithProven`) must degrade to UNTYPED the moment the
    /// call site is known to sit inside a `with` block — the caller-scope
    /// lookup cannot see the with-receiver's own rebinding, so trusting the
    /// caller-scope type here would risk typing the arg against the WRONG
    /// declaration.
    #[test]
    fn type_one_arg_bare_identifier_degrades_inside_with() {
        let mut routine = empty_routine();
        routine.locals.push(var("SomeField", "Integer"));
        let graph = ProgramGraph::default();
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &[]);
        let from = test_object_id();

        let e = ident_expr("SomeField");
        let info = type_one_arg(
            &empty_file(),
            &e,
            &routine,
            &[],
            &from,
            &graph,
            &index,
            &body_map,
            WithState::InsideWith,
        );
        assert_eq!(
            info.canonical, None,
            "a bare identifier inside a proven `with` must degrade to \
             untyped, even though caller scope resolves it cleanly; got {info:?}",
        );
        assert!(!info.var_passable);
    }

    /// The disagreeing-signals `WithState::Unknown` case must ALSO degrade
    /// (fail closed) — not just the proven `InsideWith` case.
    #[test]
    fn type_one_arg_bare_identifier_degrades_on_with_signal_disagreement() {
        let mut routine = empty_routine();
        routine.locals.push(var("SomeField", "Integer"));
        let graph = ProgramGraph::default();
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &[]);
        let from = test_object_id();

        let e = ident_expr("SomeField");
        let info = type_one_arg(
            &empty_file(),
            &e,
            &routine,
            &[],
            &from,
            &graph,
            &index,
            &body_map,
            WithState::Unknown,
        );
        assert_eq!(
            info.canonical, None,
            "the with-detection-signal-disagreement case must also fail \
             closed to untyped; got {info:?}",
        );
    }

    /// Control: a LITERAL argument is unaffected by `with_state` — a literal
    /// cannot be rebound by a `with` block, so it stays typed even
    /// `InsideWith`.
    #[test]
    fn type_one_arg_literal_typed_regardless_of_with_state() {
        let routine = empty_routine();
        let graph = ProgramGraph::default();
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &[]);
        let from = test_object_id();

        let e = Expr {
            kind: ExprKind::Literal(Literal::Int("5".to_string())),
            origin: test_origin(),
        };
        let info = type_one_arg(
            &empty_file(),
            &e,
            &routine,
            &[],
            &from,
            &graph,
            &index,
            &body_map,
            WithState::InsideWith,
        );
        assert_eq!(
            info.canonical,
            Some(CanonicalArgType::Base("integer".into())),
            "a literal must type regardless of with_state; got {info:?}",
        );
    }

    // -- candidate_param_infos: `parse_incomplete` gate (Finding 3) ---------

    /// A `parse_incomplete` candidate declaration must degrade to `None`
    /// (missing candidate metadata) even when every param otherwise carries
    /// a syntactically well-formed declared type — the recovered shape is
    /// never trusted as pick-adjudicating evidence.
    #[test]
    fn candidate_param_infos_degrades_on_parse_incomplete() {
        let mut decl = empty_routine();
        decl.params.push(param("X", "Integer"));
        decl.parse_incomplete = true;
        let graph = ProgramGraph::default();
        let index = ResolveIndex::build(&graph);
        let from = test_object_id();

        assert!(
            candidate_param_infos(&decl, &from, &graph, &index).is_none(),
            "a parse_incomplete candidate must yield no param metadata at all"
        );
    }

    /// Control: the same declaration with `parse_incomplete = false` yields
    /// full metadata — proves the gate is specific to the flag, not a
    /// blanket regression.
    #[test]
    fn candidate_param_infos_populates_when_parse_complete() {
        let mut decl = empty_routine();
        decl.params.push(param("X", "Integer"));
        let graph = ProgramGraph::default();
        let index = ResolveIndex::build(&graph);
        let from = test_object_id();

        let infos = candidate_param_infos(&decl, &from, &graph, &index)
            .expect("a parse-complete candidate must yield param metadata");
        assert_eq!(infos.len(), 1);
        assert_eq!(infos[0].canonical, CanonicalArgType::Base("integer".into()));
    }

    // -- type_one_arg: named-return-value binding (T3) -----------------------

    /// The routine's own named-return binding types a bare-identifier ARG
    /// exactly like a local — this is the mechanism behind the #9/#10
    /// ambiguous-flip fixture below: BEFORE this task, `ReturnValue` had no
    /// way to be found in caller scope at all (no `return_name` field
    /// existed), so this position was always untyped.
    #[test]
    fn type_one_arg_named_return_binding_types_like_a_local() {
        let mut routine = empty_routine();
        routine.return_name = Some("ReturnValue".to_string());
        routine.return_type = Some("JsonValue".to_string());
        let graph = ProgramGraph::default();
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &[]);
        let from = test_object_id();

        let e = ident_expr("ReturnValue");
        let info = type_one_arg(
            &empty_file(),
            &e,
            &routine,
            &[],
            &from,
            &graph,
            &index,
            &body_map,
            WithState::NoWithProven,
        );
        assert_eq!(
            info.canonical,
            Some(CanonicalArgType::Base("jsonvalue".into())),
            "the named-return binding must type the arg exactly like a local"
        );
        assert!(
            info.var_passable,
            "the binding behaves like an ordinary local — var-passable"
        );
    }

    /// A QUOTED binding name types an arg referenced via `QuotedIdentifier`
    /// identically to the unquoted form (both already store the UNQUOTED
    /// name at the IR level).
    #[test]
    fn type_one_arg_quoted_named_return_binding_types_like_a_local() {
        let mut routine = empty_routine();
        routine.return_name = Some("My Result".to_string());
        routine.return_type = Some("Text".to_string());
        let graph = ProgramGraph::default();
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &[]);
        let from = test_object_id();

        let e = Expr {
            kind: ExprKind::QuotedIdentifier("My Result".to_string()),
            origin: test_origin(),
        };
        let info = type_one_arg(
            &empty_file(),
            &e,
            &routine,
            &[],
            &from,
            &graph,
            &index,
            &body_map,
            WithState::NoWithProven,
        );
        assert_eq!(
            info.canonical,
            Some(CanonicalArgType::Base("text".into())),
            "a quoted reference to the binding must still type via caller_scope_symbol"
        );
    }

    /// SHADOW: the named-return binding shadows a same-named global exactly
    /// like `receiver.rs`'s Step 2 (the shared helper) — mirrors that
    /// module's `step2_named_return_binding_shadows_global` fixture.
    #[test]
    fn type_one_arg_named_return_binding_shadows_global() {
        let mut routine = empty_routine();
        routine.return_name = Some("Ret".to_string());
        routine.return_type = Some("Integer".to_string());
        let globals = vec![var("Ret", "Text")];
        let graph = ProgramGraph::default();
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &[]);
        let from = test_object_id();

        let e = ident_expr("Ret");
        let info = type_one_arg(
            &empty_file(),
            &e,
            &routine,
            &globals,
            &from,
            &graph,
            &index,
            &body_map,
            WithState::NoWithProven,
        );
        assert_eq!(
            info.canonical,
            Some(CanonicalArgType::Base("integer".into())),
            "the binding must shadow a same-named global"
        );
    }

    /// SAME-SCOPE malformed duplicate (round-2 closer): a named-return
    /// binding colliding with a same-named LOCAL degrades to untyped —
    /// never a guess at which one wins.
    #[test]
    fn type_one_arg_named_return_duplicate_with_local_degrades_to_untyped() {
        let mut routine = empty_routine();
        routine.return_name = Some("Ret".to_string());
        routine.return_type = Some("Integer".to_string());
        routine.locals.push(var("Ret", "Text"));
        let graph = ProgramGraph::default();
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &[]);
        let from = test_object_id();

        let e = ident_expr("Ret");
        let info = type_one_arg(
            &empty_file(),
            &e,
            &routine,
            &[],
            &from,
            &graph,
            &index,
            &body_map,
            WithState::NoWithProven,
        );
        assert_eq!(
            info.canonical, None,
            "a malformed same-scope duplicate must degrade to untyped, never guess"
        );
        assert!(!info.var_passable);
    }

    /// THE #9/#10 AMBIGUOUS-FLIP SHAPE (T3): a `GetJsonAttribute(.., \
    /// ReturnValue)`-style 2-overload call where the SECOND arg is the
    /// caller routine's OWN named-return binding. Before this task the
    /// binding could never be found in caller scope (no `return_name` field
    /// existed at all), so this position was always untyped and
    /// `pick_candidate` could never pick — this fixture proves the fix
    /// supplies exactly the missing evidence.
    #[test]
    fn named_return_binding_types_arg_and_flips_overload_pick() {
        let mut routine = empty_routine();
        routine.return_name = Some("ReturnValue".to_string());
        routine.return_type = Some("JsonValue".to_string());
        let graph = ProgramGraph::default();
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &[]);
        let from = test_object_id();

        // Position 0 ("AttrName"): identical across both candidates — never
        // discriminating. Position 1 (`ReturnValue`): typed via the binding.
        let attr_arg = base_arg("text");
        let return_value_arg = type_one_arg(
            &empty_file(),
            &ident_expr("ReturnValue"),
            &routine,
            &[],
            &from,
            &graph,
            &index,
            &body_map,
            WithState::NoWithProven,
        );
        let args = vec![attr_arg, return_value_arg];

        let candidates = vec![
            vec![base_param("text", false), base_param("jsonvalue", false)],
            vec![base_param("text", false), base_param("integer", false)],
        ];

        assert_eq!(
            pick_candidate(&args, &candidates),
            Some(0),
            "the binding's type (JsonValue) must exact-match candidate 0 and \
             provably eliminate candidate 1 (Integer), flipping AmbiguousResolved to a pick"
        );
    }

    /// CONTRAST control: with NO named-return binding declared (an anonymous
    /// return, or none at all), the SAME `ReturnValue` identifier is just an
    /// ordinary unbound name — untyped, so the SAME 2-candidate set stays
    /// unpicked (`AmbiguousResolved`, unchanged) — proves the fix in the
    /// test above is genuinely load-bearing, not incidental.
    #[test]
    fn without_named_return_binding_the_same_overload_set_stays_unpicked() {
        let routine = empty_routine(); // no return_name at all
        let graph = ProgramGraph::default();
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &[]);
        let from = test_object_id();

        let attr_arg = base_arg("text");
        let return_value_arg = type_one_arg(
            &empty_file(),
            &ident_expr("ReturnValue"),
            &routine,
            &[],
            &from,
            &graph,
            &index,
            &body_map,
            WithState::NoWithProven,
        );
        assert_eq!(
            return_value_arg.canonical, None,
            "with no binding declared, `ReturnValue` must be an untyped bare identifier"
        );
        let args = vec![attr_arg, return_value_arg];

        let candidates = vec![
            vec![base_param("text", false), base_param("jsonvalue", false)],
            vec![base_param("text", false), base_param("integer", false)],
        ];
        assert_eq!(
            pick_candidate(&args, &candidates),
            None,
            "an untyped arg position must degrade the WHOLE call — no pick"
        );
    }

    // -- type_one_arg: member-field args (Task 4, receiver-closure-and-arg-
    // increments plan) -------------------------------------------------------

    /// Builds a graph with one Table (`Customer`, id 18, carrying the given
    /// field) and one Codeunit (`CallerCu`, id 999) in the SAME app — the
    /// minimal shape the new Member arm needs (`object_by_id`/
    /// `field_in_table`/`resolve_object_ref` all require a real graph, unlike
    /// every OTHER `type_one_arg` arm tested above, which never touched one).
    fn build_member_arg_graph(
        field_name_lc: &str,
        field_type_text: &str,
    ) -> (ProgramGraph, ObjectNodeId) {
        use crate::program::graph::ObjectIndex;
        use crate::program::node_extract::{FieldNode, ObjectNode};
        use crate::program::topology::DependencyGraph;
        use crate::snapshot::{AppId, TrustTier};

        let mut apps = crate::program::node::AppRegistry::default();
        let app = apps.intern(&AppId {
            guid: String::new(),
            name: "TestApp".into(),
            publisher: "Test".into(),
            version: "1.0.0.0".into(),
        });
        let table = ObjectNode {
            id: ObjectNodeId {
                app,
                kind: al_syntax::ir::ObjectKind::Table,
                key: ObjKey::Id(18),
            },
            name: "Customer".to_string(),
            declared_id: Some(18),
            extends_target: None,
            implements: vec![],
            tier: TrustTier::Workspace,
            source_table: None,
            table_no: None,
            source_table_temporary: false,
            page_controls: vec![],
            fields: vec![FieldNode {
                name_lc: field_name_lc.to_string(),
                type_text: field_type_text.to_string(),
            }],
            dataitems: vec![],
            parse_incomplete: false,
        };
        let caller = ObjectNode {
            id: ObjectNodeId {
                app,
                kind: al_syntax::ir::ObjectKind::Codeunit,
                key: ObjKey::Id(999),
            },
            name: "CallerCu".to_string(),
            declared_id: Some(999),
            extends_target: None,
            implements: vec![],
            tier: TrustTier::Workspace,
            source_table: None,
            table_no: None,
            source_table_temporary: false,
            page_controls: vec![],
            fields: vec![],
            dataitems: vec![],
            parse_incomplete: false,
        };
        let from_id = caller.id.clone();
        let mut objects = vec![table, caller];
        objects.sort_by(|a, b| a.id.cmp(&b.id));
        let obj_index = ObjectIndex::build(&objects);
        let graph = ProgramGraph {
            apps,
            topology: DependencyGraph::default(),
            objects,
            routines: vec![],
            obj_index,
            ..Default::default()
        };
        (graph, from_id)
    }

    /// Parses `src`, finds the call site whose raw callee text matches
    /// `callee_lc` (case-insensitive), and returns the parsed file plus its
    /// argument `ExprId`s and `WithState` — the real-AST fixture builder
    /// `type_one_arg`'s Member arm needs (it dereferences `object: ExprId`
    /// via `file.ir`, unlike every other arm's hand-built `Expr`).
    fn parse_call_args(
        src: &str,
        callee_lc: &str,
    ) -> (al_syntax::ir::AlFile, Vec<ExprId>, WithState) {
        use crate::program::resolve::extract::extract_sites;
        let file = al_syntax::parse(src);
        let sites = extract_sites(&file, src, "T.al", &std::collections::HashSet::new());
        let site = sites
            .iter()
            .find(|s| s.callee_text.eq_ignore_ascii_case(callee_lc))
            .unwrap_or_else(|| panic!("no call site with callee {callee_lc:?} found"));
        (file, site.args.clone(), site.with_state)
    }

    /// POSITIVE: `Foo(Rec.Blob)` — a bare-var-based member-field arg types via
    /// the field's declared type, exactly like `receiver.rs`'s record-field
    /// receiver arm.
    #[test]
    fn type_one_arg_member_field_bare_var_resolves_declared_field_type() {
        let src = r#"
codeunit 50100 "C"
{
    procedure Run()
    var
        Rec: Record Customer;
    begin
        Foo(Rec.Blob);
    end;
}
"#;
        let (file, args, with_state) = parse_call_args(src, "Foo");
        assert_eq!(args.len(), 1);
        assert_eq!(with_state, WithState::NoWithProven);

        let (graph, from_id) = build_member_arg_graph("blob", "Blob");
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &[]);
        let mut routine = empty_routine();
        routine.locals.push(var("Rec", "Record Customer"));

        let info = type_one_arg(
            &file,
            file.ir.expr(args[0]),
            &routine,
            &[],
            &from_id,
            &graph,
            &index,
            &body_map,
            with_state,
        );
        assert_eq!(
            info.canonical,
            Some(CanonicalArgType::Base("blob".to_string()))
        );
        assert!(
            !info.var_passable,
            "member-field args are never var-passable (round-2 closer, hardcoded)"
        );
    }

    /// POSITIVE: `Foo(X."Quoted Field")` — the quoted-field spelling resolves
    /// identically to the unquoted one.
    #[test]
    fn type_one_arg_member_quoted_field_resolves_declared_field_type() {
        let src = r#"
codeunit 50100 "C"
{
    procedure Run()
    var
        X: Record Customer;
    begin
        Foo(X."Quoted Field");
    end;
}
"#;
        let (file, args, with_state) = parse_call_args(src, "Foo");
        let (graph, from_id) = build_member_arg_graph("quoted field", "Text[50]");
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &[]);
        let mut routine = empty_routine();
        routine.locals.push(var("X", "Record Customer"));

        let info = type_one_arg(
            &file,
            file.ir.expr(args[0]),
            &routine,
            &[],
            &from_id,
            &graph,
            &index,
            &body_map,
            with_state,
        );
        assert_eq!(
            info.canonical,
            Some(CanonicalArgType::Base("text".to_string()))
        );
        assert!(!info.var_passable);
    }

    /// NEGATIVE: an implicit `Rec` with NO declared var in scope declines —
    /// this arm deliberately does NOT use `receiver.rs`'s Step 3b implicit-Rec
    /// identity fallback (task brief: "implicit-Rec-without-declared-var
    /// base" is an explicit decline).
    #[test]
    fn type_one_arg_member_field_implicit_rec_without_declared_var_declines() {
        let src = r#"
codeunit 50100 "C"
{
    procedure Run()
    begin
        Foo(Rec.Blob);
    end;
}
"#;
        let (file, args, with_state) = parse_call_args(src, "Foo");
        let (graph, from_id) = build_member_arg_graph("blob", "Blob");
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &[]);
        let routine = empty_routine(); // no `Rec` declared at all

        let info = type_one_arg(
            &file,
            file.ir.expr(args[0]),
            &routine,
            &[],
            &from_id,
            &graph,
            &index,
            &body_map,
            with_state,
        );
        assert_eq!(
            info.canonical, None,
            "an implicit Rec with no DECLARED var in scope must decline"
        );
    }

    /// NEGATIVE: a multi-hop base (`A.B.Field`, the base itself a Member) —
    /// out of this increment's scope, declines rather than partially type.
    #[test]
    fn type_one_arg_member_field_multi_hop_base_declines() {
        let src = r#"
codeunit 50100 "C"
{
    procedure Run()
    var
        Rec: Record Customer;
    begin
        Foo(Rec.Something.Blob);
    end;
}
"#;
        let (file, args, with_state) = parse_call_args(src, "Foo");
        let (graph, from_id) = build_member_arg_graph("blob", "Blob");
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &[]);
        let mut routine = empty_routine();
        routine.locals.push(var("Rec", "Record Customer"));

        let info = type_one_arg(
            &file,
            file.ir.expr(args[0]),
            &routine,
            &[],
            &from_id,
            &graph,
            &index,
            &body_map,
            with_state,
        );
        assert_eq!(
            info.canonical, None,
            "a multi-hop base (itself a Member) must decline, never partially type"
        );
    }

    /// NEGATIVE: a non-Record base (`SomeText.Blob` where `SomeText: Text`) —
    /// declines rather than guess at a field on a non-Record type.
    #[test]
    fn type_one_arg_member_field_non_record_base_declines() {
        let src = r#"
codeunit 50100 "C"
{
    procedure Run()
    var
        SomeText: Text;
    begin
        Foo(SomeText.Blob);
    end;
}
"#;
        let (file, args, with_state) = parse_call_args(src, "Foo");
        let (graph, from_id) = build_member_arg_graph("blob", "Blob");
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &[]);
        let mut routine = empty_routine();
        routine.locals.push(var("SomeText", "Text"));

        let info = type_one_arg(
            &file,
            file.ir.expr(args[0]),
            &routine,
            &[],
            &from_id,
            &graph,
            &index,
            &body_map,
            with_state,
        );
        assert_eq!(
            info.canonical, None,
            "a non-Record base must decline, never guess at a field"
        );
    }

    /// NEGATIVE: an unresolvable field name (`Rec.NoSuchField`) — the base IS
    /// a Record, but `field_in_table` misses, so this declines exactly like
    /// `receiver.rs`'s record-field arm does for the same miss.
    #[test]
    fn type_one_arg_member_field_unresolvable_field_declines() {
        let src = r#"
codeunit 50100 "C"
{
    procedure Run()
    var
        Rec: Record Customer;
    begin
        Foo(Rec.NoSuchField);
    end;
}
"#;
        let (file, args, with_state) = parse_call_args(src, "Foo");
        let (graph, from_id) = build_member_arg_graph("blob", "Blob");
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &[]);
        let mut routine = empty_routine();
        routine.locals.push(var("Rec", "Record Customer"));

        let info = type_one_arg(
            &file,
            file.ir.expr(args[0]),
            &routine,
            &[],
            &from_id,
            &graph,
            &index,
            &body_map,
            with_state,
        );
        assert_eq!(
            info.canonical, None,
            "an unresolvable field name must decline, never guess"
        );
    }

    /// THE OVERLOAD FIXTURE (round-2 closer, BINDING): a member-field arg's
    /// canonical type EXACT-MATCHES a by-value overload while its
    /// `var_passable: false` ELIMINATES the sibling `var`-mode overload of the
    /// identical type — proves the hardcoded `false` is load-bearing, not
    /// inert (a wrong `true` here would make BOTH candidates exact-match,
    /// degrading this to an unpicked ambiguity instead of a clean pick).
    #[test]
    fn pick_candidate_member_field_arg_eliminates_var_param_candidate() {
        let args = vec![ArgDispatchInfo {
            canonical: Some(CanonicalArgType::Base("text".into())),
            exact_text: Some("text".into()),
            literal_kind: None,
            var_passable: false,
        }];
        let candidates = vec![
            vec![base_param("text", false)], // by-value Text overload
            vec![base_param("text", true)],  // var Text overload
        ];
        assert_eq!(
            pick_candidate(&args, &candidates),
            Some(0),
            "the var Text overload must be ELIMINATED (a member-field expr is \
             never var-passable) while the by-value Text overload is picked"
        );
    }

    // -- type_one_arg: call-result args (T3, pageext-merge-and-final-residual
    // plan) — Call/Binary/Parenthesized arms ---------------------------------

    /// Every lowered `BinaryOp` token class AL defines as UNCONDITIONALLY
    /// Boolean-typed (Eq/Ne/Lt/Le/Gt/Ge/And/Or/Xor/In) types cleanly
    /// regardless of operand shape; the arithmetic family (`Add`) declines —
    /// including a TEXT `+` concatenation (the SAME `Add` variant, non-
    /// numeric operands), proving the decline is OPERATOR-driven, never
    /// "looks numeric"-driven.
    #[test]
    fn type_one_arg_binary_operators_type_boolean_or_decline_per_token_class() {
        let cases: &[(&str, bool)] = &[
            ("1 = 1", true),          // Eq
            ("1 <> 1", true),         // Ne
            ("1 < 2", true),          // Lt
            ("1 <= 2", true),         // Le
            ("2 > 1", true),          // Gt
            ("2 >= 1", true),         // Ge
            ("TRUE AND FALSE", true), // And
            ("TRUE OR FALSE", true),  // Or
            ("TRUE XOR FALSE", true), // Xor
            ("1 IN [1, 2, 3]", true), // In
            ("1 + 1", false),         // Add (arithmetic decline)
            ("'a' + 'b'", false),     // Add (text-concat decline — same
                                      // variant as arithmetic; proves the decline is per-OPERATOR, not
                                      // per-operand-"numeric-ness").
        ];
        for (expr_src, expect_boolean) in cases {
            let src = format!(
                r#"
codeunit 50100 "C"
{{
    procedure Run()
    begin
        Foo({expr_src});
    end;
}}
"#
            );
            let (file, args, with_state) = parse_call_args(&src, "Foo");
            assert_eq!(args.len(), 1, "case {expr_src:?}");
            let routine = empty_routine();
            let graph = ProgramGraph::default();
            let index = ResolveIndex::build(&graph);
            let body_map = BodyMap::build(&graph, &[]);
            let from = test_object_id();
            let info = type_one_arg(
                &file,
                file.ir.expr(args[0]),
                &routine,
                &[],
                &from,
                &graph,
                &index,
                &body_map,
                with_state,
            );
            if *expect_boolean {
                assert_eq!(
                    info.canonical,
                    Some(CanonicalArgType::Base("boolean".into())),
                    "case {expr_src:?} must type Boolean; got {info:?}"
                );
                assert!(
                    !info.var_passable,
                    "case {expr_src:?}: a computed Boolean is never var-passable"
                );
                assert_eq!(
                    info.literal_kind, None,
                    "case {expr_src:?}: a computed Boolean is not itself a literal"
                );
            } else {
                assert_eq!(
                    info.canonical, None,
                    "case {expr_src:?} must stay untyped; got {info:?}"
                );
            }
        }
    }

    /// Parenthesized unwrap: `Foo((1 = 1))` types identically to its
    /// unwrapped inner expression — parens carry no type information of
    /// their own.
    #[test]
    fn type_one_arg_parenthesized_unwraps_to_inner_typing() {
        let src = r#"
codeunit 50100 "C"
{
    procedure Run()
    begin
        Foo((1 = 1));
    end;
}
"#;
        let (file, args, with_state) = parse_call_args(src, "Foo");
        let routine = empty_routine();
        let graph = ProgramGraph::default();
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &[]);
        let from = test_object_id();
        let info = type_one_arg(
            &file,
            file.ir.expr(args[0]),
            &routine,
            &[],
            &from,
            &graph,
            &index,
            &body_map,
            with_state,
        );
        assert_eq!(
            info.canonical,
            Some(CanonicalArgType::Base("boolean".into())),
            "a parenthesized comparison must unwrap to its inner Boolean typing; got {info:?}"
        );
    }

    /// Shadow guard (bare-Call arm, mirrors Step 5's guard EXACTLY): a LOCAL
    /// var named `GetCount` shadows a same-named procedure — `GetCount()` is
    /// then a variable reference, never a routine call, so this position
    /// must decline rather than guess. No real graph dispatch is ever
    /// attempted (the guard fires first).
    #[test]
    fn type_one_arg_call_result_bare_shadowed_by_local_declines() {
        let src = r#"
codeunit 50100 "C"
{
    procedure Run()
    var
        GetCount: Integer;
    begin
        Foo(GetCount());
    end;
}
"#;
        let (file, args, with_state) = parse_call_args(src, "Foo");
        let mut routine = empty_routine();
        routine.locals.push(var("GetCount", "Integer"));
        let graph = ProgramGraph::default();
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &[]);
        let from = test_object_id();
        let info = type_one_arg(
            &file,
            file.ir.expr(args[0]),
            &routine,
            &[],
            &from,
            &graph,
            &index,
            &body_map,
            with_state,
        );
        assert_eq!(
            info.canonical, None,
            "a local shadowing the called name must decline, never guess; got {info:?}"
        );
    }

    /// `with`-scope gate (Member-Call arm, mirrors the plain `Member`-field
    /// arm's gate EXACTLY): the base identifier could be `with`-rebound,
    /// which the caller-scope-EXACT lookup structurally cannot see — the
    /// SAME technique `type_one_arg_bare_identifier_degrades_inside_with`
    /// uses (manually overriding `with_state` on an otherwise-clean parse,
    /// since the gate must fire on the PROVEN with-state signal alone,
    /// independent of whether this particular snippet has a real `with`
    /// block).
    #[test]
    fn type_call_result_arg_member_degrades_inside_with() {
        let src = r#"
codeunit 50100 "C"
{
    procedure Run()
    var
        Rec: Record Customer;
    begin
        Foo(Rec.ToBase64String());
    end;
}
"#;
        let (file, args, _with_state) = parse_call_args(src, "Foo");
        let (graph, from_id) = build_member_arg_graph("blob", "Blob");
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &[]);
        let mut routine = empty_routine();
        routine.locals.push(var("Rec", "Record Customer"));

        let info = type_one_arg(
            &file,
            file.ir.expr(args[0]),
            &routine,
            &[],
            &from_id,
            &graph,
            &index,
            &body_map,
            WithState::InsideWith,
        );
        assert_eq!(
            info.canonical, None,
            "a Member-function call-result base must degrade to untyped inside \
             a proven `with` block; got {info:?}"
        );
    }

    /// Multi-hop base decline (Member-Call arm): `Foo(Rec.Sub.Method())` —
    /// the outer Call's function is `Member{object: Member{..}, ..}`, not a
    /// bare identifier — out of this increment's scope, declines rather
    /// than guess.
    #[test]
    fn type_call_result_arg_member_multi_hop_base_declines() {
        let src = r#"
codeunit 50100 "C"
{
    procedure Run()
    var
        Rec: Record Customer;
    begin
        Foo(Rec.Sub.Method());
    end;
}
"#;
        let (file, args, with_state) = parse_call_args(src, "Foo");
        let (graph, from_id) = build_member_arg_graph("blob", "Blob");
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &[]);
        let mut routine = empty_routine();
        routine.locals.push(var("Rec", "Record Customer"));

        let info = type_one_arg(
            &file,
            file.ir.expr(args[0]),
            &routine,
            &[],
            &from_id,
            &graph,
            &index,
            &body_map,
            with_state,
        );
        assert_eq!(
            info.canonical, None,
            "a multi-hop base (itself a Member) must decline, never partially type; got {info:?}"
        );
    }

    /// Builds a cross-app graph: a Workspace-tier `Caller` Codeunit + a
    /// SymbolOnly-tier dependency `Dep Worker` Codeunit declaring one
    /// arity-0 routine `GetValue` whose `return_type` is `None` — the common
    /// real-world ABI-ingestion gap (a SymbolReference entry that did not
    /// capture a return type). Mirrors `resolver.rs`'s
    /// `entry_trigger_marker_guard_fixture` pattern.
    fn build_symbol_only_member_call_graph() -> (ProgramGraph, ObjectNodeId) {
        use crate::program::graph::ObjectIndex;
        use crate::program::node::{AppRegistry, RoutineNodeId};
        use crate::program::node_extract::{Access, ObjectNode};
        use crate::program::resolve::edge::{AbiEventKind, AbiRoutineKind};
        use crate::program::topology::DependencyGraph;
        use crate::snapshot::{AppId, TrustTier};

        let mut apps = AppRegistry::default();
        let ws_ref = apps.intern(&AppId {
            guid: String::new(),
            name: "WS".into(),
            publisher: "Test".into(),
            version: "1.0.0.0".into(),
        });
        let dep_ref = apps.intern(&AppId {
            guid: String::new(),
            name: "DepApp".into(),
            publisher: "Test".into(),
            version: "1.0.0.0".into(),
        });

        let caller_id = ObjectNodeId {
            app: ws_ref,
            kind: al_syntax::ir::ObjectKind::Codeunit,
            key: ObjKey::Id(50700),
        };
        let dep_id = ObjectNodeId {
            app: dep_ref,
            kind: al_syntax::ir::ObjectKind::Codeunit,
            key: ObjKey::Id(60700),
        };

        let mut objects = vec![
            ObjectNode {
                id: caller_id.clone(),
                name: "Caller".into(),
                declared_id: Some(50700),
                extends_target: None,
                implements: vec![],
                tier: TrustTier::Workspace,
                source_table: None,
                table_no: None,
                source_table_temporary: false,
                page_controls: vec![],
                fields: vec![],
                dataitems: vec![],
                parse_incomplete: false,
            },
            ObjectNode {
                id: dep_id.clone(),
                name: "Dep Worker".into(),
                declared_id: Some(60700),
                extends_target: None,
                implements: vec![],
                tier: TrustTier::SymbolOnly,
                source_table: None,
                table_no: None,
                source_table_temporary: false,
                page_controls: vec![],
                fields: vec![],
                dataitems: vec![],
                parse_incomplete: false,
            },
        ];
        objects.sort_by(|a, b| a.id.cmp(&b.id));

        let routines = vec![RoutineNode {
            id: RoutineNodeId {
                object: dep_id.clone(),
                name_lc: "getvalue".into(),
                enclosing_member_lc: None,
                params_count: 0,
                sig_fp: 0,
            },
            name: "GetValue".into(),
            is_trigger: false,
            access: Access::Public,
            tier: TrustTier::SymbolOnly,
            event_subscribers: vec![],
            subscriber_instance_manual: false,
            publisher_kind: None,
            include_sender: None,
            abi_routine_kind: Some(AbiRoutineKind::Procedure),
            abi_event_kind: Some(AbiEventKind::None),
            param_sig_key: String::new(),
            return_type: None,
            return_type_id: None,
            abi_overload_collapsed: false,
            source_overload_aliased: false,
        }];

        let mut topology = DependencyGraph::default();
        topology.add_dependency(ws_ref, dep_ref);

        let obj_index = ObjectIndex::build(&objects);
        let graph = ProgramGraph {
            apps,
            topology,
            objects,
            routines,
            obj_index,
            ..Default::default()
        };
        (graph, caller_id)
    }

    /// SymbolOnly inner (T3 negative): the call-result base resolves to a
    /// cross-app SymbolOnly (ABI) object whose target routine carries NO
    /// captured return type (`return_type: None`) — a real-world ABI-
    /// ingestion gap. Must decline to untyped, never guess/panic.
    #[test]
    fn type_call_result_arg_member_symbol_only_inner_with_no_return_type_declines() {
        let src = r#"
codeunit 50700 "Caller"
{
    procedure Run()
    var
        DepVar: Codeunit "Dep Worker";
    begin
        Foo(DepVar.GetValue());
    end;
}
"#;
        let (file, args, with_state) = parse_call_args(src, "Foo");
        assert_eq!(with_state, WithState::NoWithProven);
        let (graph, from_id) = build_symbol_only_member_call_graph();
        let index = ResolveIndex::build(&graph);
        let body_map = BodyMap::build(&graph, &[]);
        let mut routine = empty_routine();
        routine
            .locals
            .push(var("DepVar", "Codeunit \"Dep Worker\""));

        let info = type_one_arg(
            &file,
            file.ir.expr(args[0]),
            &routine,
            &[],
            &from_id,
            &graph,
            &index,
            &body_map,
            with_state,
        );
        assert_eq!(
            info.canonical, None,
            "a SymbolOnly inner call with no captured return type must decline; got {info:?}"
        );
    }

    // -- builtin_return_base_keyword (T3, part c) ----------------------------

    /// Every catalog entry resolves to its documented base keyword; an
    /// uncataloged builtin name (e.g. `Message`, which returns nothing) is
    /// `None` — fail-closed, never a guess.
    #[test]
    fn builtin_return_base_keyword_catalog_lookup() {
        assert_eq!(builtin_return_base_keyword("strsubstno"), Some("text"));
        assert_eq!(builtin_return_base_keyword("format"), Some("text"));
        assert_eq!(builtin_return_base_keyword("copystr"), Some("text"));
        assert_eq!(builtin_return_base_keyword("lowercase"), Some("text"));
        assert_eq!(builtin_return_base_keyword("uppercase"), Some("text"));
        assert_eq!(builtin_return_base_keyword("round"), Some("decimal"));
        assert_eq!(builtin_return_base_keyword("strlen"), Some("integer"));
        assert_eq!(
            builtin_return_base_keyword("message"),
            None,
            "an uncataloged builtin must be None, never a guess"
        );
    }
}
