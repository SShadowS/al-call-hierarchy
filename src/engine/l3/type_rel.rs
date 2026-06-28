//! AL argument↔parameter type-relation primitives (R2b Task 2) — faithful ports
//! of al-sem's `scalarFamily`, `isObjectish`, `isEnumish`, and `typeRelation`
//! from `src/resolve/call-resolver.ts`.
//!
//! All pure, never panic. `type_relation` returns "definitely-incompatible"
//! ONLY when the two types are provably disjoint (the overload matcher may drop
//! a candidate only on a proof); everything uncertain → "unknown".

use super::al_type::normalize_al_type;

/// Conservative type-relation result. `as_str()` yields the exact al-sem
/// string ("definitely-compatible" | "definitely-incompatible" | "unknown").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeRelation {
    DefinitelyCompatible,
    DefinitelyIncompatible,
    Unknown,
}

impl TypeRelation {
    pub fn as_str(self) -> &'static str {
        match self {
            TypeRelation::DefinitelyCompatible => "definitely-compatible",
            TypeRelation::DefinitelyIncompatible => "definitely-incompatible",
            TypeRelation::Unknown => "unknown",
        }
    }
}

/// Extract the head token: the substring of `norm` before the first space or
/// double-quote. Port of TS `norm.split(/[ "]/, 1)[0] ?? norm` — JS
/// `split(re, 1)` returns the slice up to (excluding) the first delimiter.
fn head(norm: &str) -> &str {
    match norm.find([' ', '"']) {
        Some(idx) => &norm[..idx],
        None => norm,
    }
}

/// Coarse scalar family for a normalized type token; `None` for object/unknown.
///
/// NOTE (preserved from al-sem): "char" and "duration" are intentionally
/// omitted — both are integer-backed in AL and implicitly coerce to numeric, so
/// declaring them disjoint from numeric would be unsound. They fall to `None`
/// (→ typeRelation "unknown").
pub fn scalar_family(norm: &str) -> Option<&'static str> {
    match head(norm) {
        "instream" | "outstream" => Some("stream"),
        "integer" | "decimal" | "biginteger" | "byte" => Some("numeric"),
        "boolean" => Some("boolean"),
        "text" | "code" => Some("text"),
        "date" | "time" | "datetime" => Some("datetime"),
        "guid" => Some("guid"),
        _ => None,
    }
}

/// True for nominal object/reference types whose subtype/implements relations
/// Phase A does not model. Port of al-sem `isObjectish`.
pub fn is_objectish(norm: &str) -> bool {
    matches!(
        head(norm),
        "codeunit"
            | "interface"
            | "record"
            | "report"
            | "page"
            | "query"
            | "xmlport"
            | "enum"
            | "option"
            | "dotnet"
            | "label"
    )
}

/// True for enum/option-typed arguments (integer-backed in AL). Port of
/// al-sem `isEnumish`.
pub fn is_enumish(norm: &str) -> bool {
    matches!(head(norm), "enum" | "option")
}

/// Conservative AL argument↔parameter compatibility. Faithful port of al-sem
/// `typeRelation`: "definitely-incompatible" only on a disjointness proof;
/// Variant/Any, object/interface, and anything uncertain → "unknown".
pub fn type_relation(arg_type: &str, param_type: &str) -> TypeRelation {
    let a = normalize_al_type(arg_type);
    let p = normalize_al_type(param_type);

    if a == p {
        return TypeRelation::DefinitelyCompatible;
    }
    if a == "variant" || p == "variant" || a == "any" || p == "any" {
        return TypeRelation::Unknown;
    }
    // Narrow soundness rule: an enum/option-typed argument can never bind a
    // stream parameter. Only the stream disjointness is provable.
    if (is_enumish(&a) && scalar_family(&p) == Some("stream"))
        || (is_enumish(&p) && scalar_family(&a) == Some("stream"))
    {
        return TypeRelation::DefinitelyIncompatible;
    }
    if is_objectish(&a) || is_objectish(&p) {
        return TypeRelation::Unknown;
    }
    let fa = scalar_family(&a);
    let fp = scalar_family(&p);
    if let (Some(fa), Some(fp)) = (fa, fp)
        && fa != fp
    {
        return TypeRelation::DefinitelyIncompatible;
    }
    TypeRelation::Unknown
}
