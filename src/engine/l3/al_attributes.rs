//! L3 structured-attribute model + accessors — Rust port of al-sem's
//! `src/model/attributes.ts` (`AttributeInfo` / `AttributeArg` +
//! `findAttribute` / `qualifiedArg` / `stringArg` / `boolArg`).
//!
//! The grammar-derived `AttributeArg` shape (kind / text / value / qualifier /
//! member) is ALREADY produced verbatim by R1's L2 attribute indexing
//! (`src/engine/l2/l2_workspace.rs::attr_arg_from_node`, which mirrors al-sem's
//! `attribute-from-node.ts`). This module mirrors that JSON shape as a typed
//! struct so the event-graph resolver can query attributes structurally, and so
//! the R2c parity vectors can round-trip the al-sem serialization arg-for-arg.
//!
//! `value`, `qualifier`, `member` are OMITTED (None → skipped) when absent — the
//! exact serde contract that makes the R2c attribute-shape vectors round-trip
//! byte-for-byte against al-sem's serialized `attributesParsed`.

use serde::{Deserialize, Serialize};

/// One positional argument inside an attribute's argument list. Mirrors al-sem's
/// `AttributeArg`. `value`/`qualifier`/`member` are skipped when absent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AttributeArg {
    /// Grammar-aligned argument kind (`boolean`, `string_literal`,
    /// `qualified_enum_value`, `database_reference`, …, else `unknown`).
    pub kind: String,
    /// Raw, source-faithful slice (includes quotes for string_literal /
    /// quoted_identifier).
    pub text: String,
    /// Unquoted / derived value:
    /// - string_literal / quoted_identifier: contents between the quotes
    /// - qualified_enum_value / database_reference: RHS of `::` (unquoted)
    /// - boolean / integer / identifier: the text verbatim
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub value: Option<String>,
    /// LHS of `::` for qualified_enum_value / database_reference.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub qualifier: Option<String>,
    /// RHS of `::` for qualified_enum_value / database_reference (UNQUOTED).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub member: Option<String>,
}

/// One `[Name(args...)]` attribute on a routine. Mirrors al-sem's `AttributeInfo`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AttributeInfo {
    /// Case-preserved name as written.
    pub name: String,
    /// Positional args.
    pub args: Vec<AttributeArg>,
    /// The full `[…]` source slice.
    pub raw: String,
}

/// The qualifier/member halves of a `::`-qualified arg.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QualifiedArg {
    pub qualifier: String,
    pub member: String,
}

/// Case-insensitive lookup of the FIRST attribute matching `name`. Returns None
/// when absent — call sites branch on that, never panic. Mirrors `findAttribute`.
pub fn find_attribute<'a>(attrs: &'a [AttributeInfo], name: &str) -> Option<&'a AttributeInfo> {
    let lc = name.to_lowercase();
    attrs.iter().find(|a| a.name.to_lowercase() == lc)
}

/// True when at least one attribute matches `name` (case-insensitive).
pub fn has_attribute(attrs: &[AttributeInfo], name: &str) -> bool {
    find_attribute(attrs, name).is_some()
}

/// Read the `value` of arg at `index` when it is a `string_literal`. Returns None
/// when the arg is absent, of the wrong kind, or value is unset. Mirrors `stringArg`.
pub fn string_arg(attr: &AttributeInfo, index: usize) -> Option<String> {
    let a = attr.args.get(index)?;
    if a.kind != "string_literal" {
        return None;
    }
    a.value.clone()
}

/// Read a boolean positional arg at `index`. Returns Some(true)/Some(false) for the
/// literal `true`/`false` (case-insensitive), None when the arg is absent, of a
/// non-boolean kind, or has no value. Mirrors `boolArg`.
pub fn bool_arg(attr: &AttributeInfo, index: usize) -> Option<bool> {
    let a = attr.args.get(index)?;
    if a.kind != "boolean" {
        return None;
    }
    let v = a.value.as_ref()?;
    match v.to_lowercase().as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

/// Read a `qualified_enum_value` / `database_reference` arg's qualifier + member.
/// Returns None when the arg is absent or of a non-qualified kind. Mirrors
/// `qualifiedArg`.
pub fn qualified_arg(attr: &AttributeInfo, index: usize) -> Option<QualifiedArg> {
    let a = attr.args.get(index)?;
    if a.kind != "qualified_enum_value" && a.kind != "database_reference" {
        return None;
    }
    let qualifier = a.qualifier.clone()?;
    let member = a.member.clone()?;
    Some(QualifiedArg { qualifier, member })
}
