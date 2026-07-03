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

use al_syntax::ir::{AlFile, Expr, ExprId, ExprKind, Literal, ObjectKind, RoutineDecl, VarDecl};

use crate::program::graph::ProgramGraph;
use crate::program::node::ObjectNodeId;
use crate::program::node_extract::ObjectRef;
use crate::program::resolve::index::{ObjectRefResolution, ResolveIndex};
use crate::program::resolve::receiver::{ParsedType, classify_type_text};
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
/// against).
pub(crate) fn type_call_args(
    args: &[ExprId],
    file: &AlFile,
    routine: &RoutineDecl,
    object_globals: &[VarDecl],
    from: &ObjectNodeId,
    graph: &ProgramGraph,
    index: &ResolveIndex,
) -> Vec<ArgDispatchInfo> {
    args.iter()
        .map(|&id| {
            type_one_arg(
                file.ir.expr(id),
                routine,
                object_globals,
                from,
                graph,
                index,
            )
        })
        .collect()
}

fn type_one_arg(
    expr: &Expr,
    routine: &RoutineDecl,
    object_globals: &[VarDecl],
    from: &ObjectNodeId,
    graph: &ProgramGraph,
    index: &ResolveIndex,
) -> ArgDispatchInfo {
    match &expr.kind {
        // `QuotedIdentifier` already stores the UNQUOTED name (the lowerer
        // strips quotes at lowering time — same convention `Param::name`/
        // `VarDecl::name` use), so both arms compare directly against the
        // caller's declared names with no extra unquoting.
        ExprKind::Identifier(name) | ExprKind::QuotedIdentifier(name) => {
            // Caller-scope-EXACT lookup: params -> locals -> globals (the
            // SAME Step-2 shadowing order `receiver.rs`'s `infer_receiver_
            // type` uses) — never a receiver/`with` scope. `.find` stops at
            // the FIRST matching declaration; a shadowing local/param with no
            // declared type text still shadows a same-named global (module
            // doc) rather than falling through to it.
            let declared_ty: Option<Option<&str>> = routine
                .params
                .iter()
                .find(|p| p.name.eq_ignore_ascii_case(name))
                .map(|p| p.ty.as_deref())
                .or_else(|| {
                    routine
                        .locals
                        .iter()
                        .find(|v| v.name.eq_ignore_ascii_case(name))
                        .map(|v| v.ty.as_deref())
                })
                .or_else(|| {
                    object_globals
                        .iter()
                        .find(|v| v.name.eq_ignore_ascii_case(name))
                        .map(|v| v.ty.as_deref())
                });
            match declared_ty {
                Some(Some(ty)) => ArgDispatchInfo {
                    canonical: dispatch_canonical_type_text(ty, from, graph, index),
                    exact_text: Some(normalize_type_text(ty)),
                    literal_kind: None,
                    var_passable: true,
                },
                // Found but no declared type text, or not found at all in
                // caller scope — untyped either way, never a guess.
                _ => ArgDispatchInfo::untyped(),
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
        // Deferred (increment-1 scope, module doc): call-result / `Rec.Field`
        // / `Enum::Value` / any other expression shape stays untyped.
        _ => ArgDispatchInfo::untyped(),
    }
}

/// Build the full [`ParamDispatchInfo`] list for one candidate's parameters,
/// as seen from `from` (the CANDIDATE's OWN declaring object identity — an
/// object-bearing param type resolves against the object that DECLARED the
/// routine, not the caller). Returns `None` — "missing candidate metadata",
/// degrading the WHOLE call per the module doc — when ANY parameter has no
/// declared type text, or its declared type fails canonicalization
/// (unresolvable object reference).
pub(crate) fn candidate_param_infos(
    decl: &RoutineDecl,
    from: &ObjectNodeId,
    graph: &ProgramGraph,
    index: &ResolveIndex,
) -> Option<Vec<ParamDispatchInfo>> {
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
        let from = test_object_id();

        let e = ident_expr("X");
        let info = type_one_arg(&e, &routine, &globals, &from, &graph, &index);
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
        let from = test_object_id();

        let e = ident_expr("X");
        let info = type_one_arg(&e, &routine, &globals, &from, &graph, &index);
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
        let from = test_object_id();

        let e = Expr {
            kind: ExprKind::Literal(Literal::Int("5".to_string())),
            origin: test_origin(),
        };
        let info = type_one_arg(&e, &routine, &[], &from, &graph, &index);
        assert_eq!(
            info.canonical,
            Some(CanonicalArgType::Base("integer".into()))
        );
        assert_eq!(info.literal_kind, Some(LiteralKind::Integer));
        assert!(!info.var_passable);
    }
}
