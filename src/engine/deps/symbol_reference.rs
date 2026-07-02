//! Rust port of al-sem's `src/symbols/symbol-reference-parser.ts` —
//! `parseSymbolReference` and its helpers.
//!
//! Produces the neutral `SymbolReferenceAbi` DTO from raw `SymbolReference.json`
//! text: object-array dispatch (ROUTINE_BEARING / EXTENSION_ROUTINE_BEARING /
//! BARE / Tables), `parse_method` (event-kind from attributes), `parse_field`,
//! `classify_abi_arg`, `abi_attribute_info`, `parse_abi_interface_name`,
//! `unquote_abi_name`, `raw_object_property`, and InherentCommitBehavior parse.
//!
//! Reuses the shared `AttributeInfo`/`AttributeArg` shape from
//! `crate::engine::l3::al_attributes` — the ABI path is a SECOND producer of the
//! SAME shape (the native AST path is the first), so the event-graph resolver and
//! attribute consumers traverse one normalized representation.
//!
//! Never panics: a JSON parse failure yields a DTO with empty objects/tables and
//! an `error` string (the TS "never throws" / catch posture).

use crate::engine::l3::al_attributes::{AttributeArg, AttributeInfo};
use serde::Deserialize;
use serde_json::Value;

/// "integration" | "business" | "unknown" — only meaningful when the routine kind
/// is `event-publisher`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AbiEventKind {
    Integration,
    Business,
    Unknown,
}

impl AbiEventKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            AbiEventKind::Integration => "integration",
            AbiEventKind::Business => "business",
            AbiEventKind::Unknown => "unknown",
        }
    }
}

/// A parameter signature from `SymbolReference.json` — no per-run ids.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbiParameter {
    pub name: String,
    pub type_text: String,
    pub is_var: bool,
    /// True when the parameter is declared `temporary` (e.g. `var Rec: Record T
    /// temporary`). Additive — no consumers yet; populated by later tasks (Task 6).
    pub is_temporary: bool,
}

/// A routine signature from `SymbolReference.json`. No body, no anchor, no per-run id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbiRoutine {
    pub name: String,
    /// One of the al-sem `RoutineKind` strings: "procedure" | "event-publisher" |
    /// "event-subscriber".
    pub kind: String,
    pub event_kind: AbiEventKind,
    pub parameters: Vec<AbiParameter>,
    /// Whether `parameters` reflects a genuinely-parsed `Parameters` JSON array
    /// (present in the source JSON, even if empty for a true 0-arg procedure) vs.
    /// the field being absent/unparseable — arity is TRI-STATE (Task 1, round-2
    /// hardening): `false` here means the candidate's arity is UNKNOWN, never
    /// zero. Consumers (`abi_ingest`) must never treat an unknown arity as a
    /// concrete `0` — see `abi_ingest::UNKNOWN_ARITY`.
    pub parameters_known: bool,
    /// Reconstructed SOURCE-SHAPED return-type text (Task 2) — see
    /// [`reconstruct_return_type_text`] for the full fail-closed rule set.
    /// `None` covers both "no return type declared" AND "a return type was
    /// declared but could not be safely reconstructed" (Id-only Subtype, a
    /// Subtype Name containing a quote character) — this field alone cannot
    /// distinguish the two; that distinction does not matter to any consumer
    /// (both mean "do not treat this as a known scalar type").
    pub return_type_text: Option<String>,
    /// The raw `(name, id)` pair from the return type's `Subtype`, present
    /// ONLY when BOTH `Subtype.Name` and `Subtype.Id` were declared in the
    /// source JSON (Task 2 enabling primitive for Task 3's cross-object chain
    /// cross-validation: when a return type's declared Subtype carries both a
    /// Name and an Id, the object the Name resolves to must ALSO carry that
    /// declared Id, or the candidate route declines — name-or-id alone is not
    /// proof of object identity, round-1 C4). Deliberately INDEPENDENT of
    /// `return_type_text`'s fail-closed TEXT reconstruction rules: a Subtype
    /// Name containing a `"` still yields `return_type_text == None` (never
    /// synthesize unescaped text), but the raw identity pair is still carried
    /// here — cross-validation is a structured `==` comparison, never a
    /// text-synthesis operation, so the quote landmine does not apply to it.
    pub return_type_id: Option<(String, i64)>,
    pub is_local: bool,
    pub is_internal: bool,
    /// `protected` visibility modifier (`"IsProtected":true` in
    /// `SymbolReference.json` — verified against real Microsoft System App data:
    /// 10 occurrences, matching its embedded source's 10 `protected procedure`s
    /// 1:1). UNLIKE `is_local`/`is_internal` (dropped entirely at ingestion — AL
    /// never lets an outside caller reach them), a `protected` ABI routine is
    /// KEPT and carried as `Access::Protected`: AL lets a workspace extension of
    /// the declaring object call its `protected` members (Task 1).
    pub is_protected: bool,
    /// Reconstructed attribute strings (back-compat / display).
    pub attributes: Vec<String>,
    /// Structured attributes — same shape as the native AL path's `AttributeInfo`.
    pub attributes_parsed: Vec<AttributeInfo>,
}

/// An ABI field — table column metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbiField {
    pub field_number: i64,
    pub name: String,
    pub data_type: String,
    /// "Normal" | "FlowField" | "FlowFilter".
    pub field_class: String,
    pub is_blob_like: bool,
}

/// An ABI key — references fields by name; index is array position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbiKey {
    pub name: String,
    pub field_names: Vec<String>,
}

/// An ABI object — codeunit/page/table/etc.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AbiObject {
    pub object_type: String,
    pub object_number: i64,
    pub name: String,
    pub routines: Vec<AbiRoutine>,
    pub object_subtype: Option<String>,
    pub page_type: Option<String>,
    pub source_table_name: Option<String>,
    pub extends_target_name: Option<String>,
    /// Unquoted, `#guid#`-stripped interface names. `None` = field absent.
    pub implemented_interfaces: Option<Vec<String>>,
    /// Canonical lower-case member: "ignore" | "error" | "allow".
    pub inherent_commit_behavior: Option<String>,
    /// Page controls (name, kind, target). kind ∈ {"part","usercontrol"}.
    /// target = subpage Page NUMBER (string) for parts, control-add-in NAME for usercontrols.
    pub page_controls: Vec<(String, String, String)>,
}

/// An ABI table — physical table layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbiTable {
    pub object_number: i64,
    pub name: String,
    pub fields: Vec<AbiField>,
    pub keys: Vec<AbiKey>,
    /// True when the table is declared with `TableType = Temporary`. Additive —
    /// no consumers yet; populated by later tasks (Task 6).
    pub is_temporary: bool,
}

/// The neutral DTO `parse_symbol_reference` produces — no model entities/ids.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SymbolReferenceAbi {
    pub app_guid: String,
    pub name: String,
    pub publisher: String,
    pub version: String,
    pub objects: Vec<AbiObject>,
    pub tables: Vec<AbiTable>,
    /// Set when the JSON could not be parsed; objects/tables are then empty.
    pub error: Option<String>,
}

// --- Raw serde shapes (mirror the TS `Raw*` interfaces) ---------------------

#[derive(Debug, Clone, Deserialize, Default)]
struct RawArg {
    #[serde(rename = "Value")]
    value: Option<Value>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct RawAttr {
    #[serde(rename = "Name")]
    name: Option<String>,
    #[serde(rename = "Arguments")]
    arguments: Option<Vec<RawArg>>,
}

/// The nested `Subtype` object `SymbolReference.json` attaches to a
/// `TypeDefinition` for a database/framework type (e.g.
/// `{"Name":"Codeunit","Subtype":{"Name":"Http Content","Id":2354}}`) —
/// carries the AL object's declared name and/or numeric id (Task 2). Either
/// field may be absent independently: a real dependency ABI can declare only
/// a `Name` (id genuinely unknown to the compiler at symbol-export time) or
/// only an `Id` (name unavailable) — see [`reconstruct_return_type_text`] for
/// how each combination is handled fail-closed.
#[derive(Debug, Clone, Deserialize, Default)]
struct RawSubtype {
    #[serde(rename = "Name")]
    name: Option<String>,
    #[serde(rename = "Id")]
    id: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct RawTypeDef {
    #[serde(rename = "Name")]
    name: Option<String>,
    /// Present when the parameter / return type carries a `temporary` modifier in
    /// the AL source (e.g. `var Rec: Record T temporary`). Additive — populated
    /// by later tasks (Task 6).
    #[serde(rename = "Temporary")]
    temporary: Option<bool>,
    /// The nested database/framework subtype (Task 2) — see [`RawSubtype`].
    /// `None` for a scalar/bare type (`Integer`, `HttpHeaders`) that carries
    /// no nested object identity.
    #[serde(rename = "Subtype")]
    subtype: Option<RawSubtype>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct RawParam {
    #[serde(rename = "Name")]
    name: Option<String>,
    #[serde(rename = "IsVar")]
    is_var: Option<bool>,
    #[serde(rename = "TypeDefinition")]
    type_definition: Option<RawTypeDef>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct RawMethod {
    #[serde(rename = "Name")]
    name: Option<String>,
    #[serde(rename = "Parameters")]
    parameters: Option<Vec<RawParam>>,
    #[serde(rename = "ReturnTypeDefinition")]
    return_type_definition: Option<RawTypeDef>,
    #[serde(rename = "Attributes")]
    attributes: Option<Vec<RawAttr>>,
    #[serde(rename = "IsLocal")]
    is_local: Option<bool>,
    #[serde(rename = "IsInternal")]
    is_internal: Option<bool>,
    #[serde(rename = "IsProtected")]
    is_protected: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct RawProperty {
    #[serde(rename = "Name")]
    name: Option<String>,
    #[serde(rename = "Value")]
    value: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct RawField {
    #[serde(rename = "Id")]
    id: Option<i64>,
    #[serde(rename = "Name")]
    name: Option<String>,
    #[serde(rename = "TypeDefinition")]
    type_definition: Option<RawTypeDef>,
    #[serde(rename = "Properties")]
    properties: Option<Vec<RawProperty>>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct RawKey {
    #[serde(rename = "Name")]
    name: Option<String>,
    #[serde(rename = "FieldNames")]
    field_names: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct RawRelatedId {
    #[serde(rename = "Id")]
    id: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct RawControl {
    #[serde(rename = "Kind")]
    kind: Option<i64>,
    #[serde(rename = "Name")]
    name: Option<String>,
    #[serde(rename = "RelatedPagePartId")]
    related_page_part_id: Option<RawRelatedId>,
    #[serde(rename = "RelatedControlAddIn")]
    related_control_addin: Option<String>,
    #[serde(rename = "Controls")]
    controls: Option<Vec<RawControl>>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct RawObject {
    #[serde(rename = "Id")]
    id: Option<i64>,
    #[serde(rename = "Name")]
    name: Option<String>,
    #[serde(rename = "Methods")]
    methods: Option<Vec<RawMethod>>,
    #[serde(rename = "Fields")]
    fields: Option<Vec<RawField>>,
    #[serde(rename = "Keys")]
    keys: Option<Vec<RawKey>>,
    #[serde(rename = "Properties")]
    properties: Option<Vec<RawProperty>>,
    #[serde(rename = "TargetObject")]
    target_object: Option<String>,
    #[serde(rename = "ImplementedInterfaces")]
    implemented_interfaces: Option<Vec<String>>,
    #[serde(rename = "Controls")]
    controls: Option<Vec<RawControl>>,
}

// --- Helpers (mirror the TS module-level functions) -------------------------

fn blob_like(token: &str) -> bool {
    matches!(token, "blob" | "media" | "mediaset")
}

/// Read a named property value from a `Properties` array. Case-insensitive.
fn raw_object_property(
    properties: &Option<Vec<RawProperty>>,
    property_name: &str,
) -> Option<String> {
    let props = properties.as_ref()?;
    let lc = property_name.to_lowercase();
    for p in props {
        if p.name.clone().unwrap_or_default().to_lowercase() == lc {
            return p.value.clone();
        }
    }
    None
}

/// Task 6 (G7, RV-4): a table is declared temporary when it carries the property
/// `{"Name":"TableType","Value":"Temporary"}` (case-insensitive value match). Mirror
/// how `parse_field` reads the `fieldclass` property — structural read, NO
/// string-sniffing. Verified against a real Continia Core 29.0 SymbolReference.json.
fn raw_table_is_temporary(properties: &Option<Vec<RawProperty>>) -> bool {
    raw_object_property(properties, "TableType")
        .map(|v| v.eq_ignore_ascii_case("Temporary"))
        .unwrap_or(false)
}

/// Strip surrounding double-quotes (AL quoted-identifier syntax).
fn unquote_abi_name(raw: &str) -> String {
    let chars: Vec<char> = raw.chars().collect();
    if chars.len() >= 2 && chars[0] == '"' && chars[chars.len() - 1] == '"' {
        chars[1..chars.len() - 1].iter().collect()
    } else {
        raw.to_string()
    }
}

/// Parse a raw `ImplementedInterfaces` value into an unquoted interface name:
/// strip a leading `#<...>#` cross-app prefix, then strip surrounding quotes.
/// Mirrors `parseAbiInterfaceName` (regex `/^#[^#]*#(.+)$/`).
fn parse_abi_interface_name(raw: &str) -> String {
    // `/^#[^#]*#(.+)$/` — `#`, then zero-or-more non-`#`, then `#`, then `.+`.
    let name = if let Some(stripped) = raw.strip_prefix('#') {
        if let Some(hash_pos) = stripped.find('#') {
            let rest = &stripped[hash_pos + 1..];
            // `.+` requires at least one char after the closing `#`.
            if !rest.is_empty() { rest } else { raw }
        } else {
            raw
        }
    } else {
        raw
    };
    let chars: Vec<char> = name.chars().collect();
    if chars.len() >= 2 && chars[0] == '"' && chars[chars.len() - 1] == '"' {
        chars[1..chars.len() - 1].iter().collect()
    } else {
        name.to_string()
    }
}

/// Stringify a serde_json `Value` the way JS `String(x.Value ?? "")` would for the
/// values that appear in attribute argument lists. Only used for the back-compat
/// `attributes` display strings.
fn js_string_of(v: &Option<Value>) -> String {
    match v {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Bool(b)) => b.to_string(),
        Some(Value::Number(n)) => n.to_string(),
        Some(other) => other.to_string(),
    }
}

/// Reconstruct the back-compat attribute string, e.g. "[IntegrationEvent(False, False)]".
fn attr_string(a: &RawAttr) -> String {
    let name = a.name.clone().unwrap_or_default();
    let args: Vec<String> = a
        .arguments
        .as_ref()
        .map(|args| args.iter().map(|x| js_string_of(&x.value)).collect())
        .unwrap_or_default();
    if args.is_empty() {
        format!("[{name}]")
    } else {
        format!("[{}({})]", name, args.join(", "))
    }
}

/// Object-type keywords distinguishing a `database_reference` from a generic
/// `qualified_enum_value`. Mirrors the TS `OBJECT_TYPE_KEYWORDS` set.
fn is_object_type_keyword(s: &str) -> bool {
    matches!(
        s,
        "database" | "page" | "report" | "codeunit" | "xmlport" | "query" | "table"
    )
}

/// True when every char of `s` is `0-9` (non-empty, optional leading `-`).
fn is_integer_text(s: &str) -> bool {
    let b = s.as_bytes();
    if b.is_empty() {
        return false;
    }
    let mut i = 0;
    if b[0] == b'-' {
        i = 1;
        if i == b.len() {
            return false;
        }
    }
    while i < b.len() {
        if !b[i].is_ascii_digit() {
            return false;
        }
        i += 1;
    }
    true
}

/// Classify a single `RawArg.Value` into a typed `AttributeArg`. Mirrors
/// `classifyAbiArg`.
fn classify_abi_arg(raw: &Option<Value>) -> AttributeArg {
    match raw {
        Some(Value::Bool(b)) => {
            let text = if *b { "true" } else { "false" }.to_string();
            return AttributeArg {
                kind: "boolean".to_string(),
                text: text.clone(),
                value: Some(text),
                qualifier: None,
                member: None,
            };
        }
        Some(Value::Number(n)) => {
            // `typeof raw === "number" && Number.isFinite && Number.isInteger`
            if let Some(i) = n.as_i64() {
                let text = i.to_string();
                return AttributeArg {
                    kind: "integer".to_string(),
                    text: text.clone(),
                    value: Some(text),
                    qualifier: None,
                    member: None,
                };
            }
            if let Some(u) = n.as_u64() {
                let text = u.to_string();
                return AttributeArg {
                    kind: "integer".to_string(),
                    text: text.clone(),
                    value: Some(text),
                    qualifier: None,
                    member: None,
                };
            }
            // Non-integer number → falls through to String(raw) path below.
        }
        _ => {}
    }

    // text = String(raw ?? "")
    let text = js_string_of(raw);
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return AttributeArg {
            kind: "unknown".to_string(),
            text,
            value: None,
            qualifier: None,
            member: None,
        };
    }

    let lower = trimmed.to_lowercase();
    if lower == "true" || lower == "false" {
        return AttributeArg {
            kind: "boolean".to_string(),
            text: trimmed.to_string(),
            value: Some(lower),
            qualifier: None,
            member: None,
        };
    }
    if is_integer_text(trimmed) {
        return AttributeArg {
            kind: "integer".to_string(),
            text: trimmed.to_string(),
            value: Some(trimmed.to_string()),
            qualifier: None,
            member: None,
        };
    }
    let tchars: Vec<char> = trimmed.chars().collect();
    if tchars.len() >= 2 && tchars[0] == '\'' && tchars[tchars.len() - 1] == '\'' {
        let inner: String = tchars[1..tchars.len() - 1].iter().collect();
        return AttributeArg {
            kind: "string_literal".to_string(),
            text: trimmed.to_string(),
            value: Some(inner),
            qualifier: None,
            member: None,
        };
    }
    if tchars.len() >= 2 && tchars[0] == '"' && tchars[tchars.len() - 1] == '"' {
        let inner: String = tchars[1..tchars.len() - 1].iter().collect();
        return AttributeArg {
            kind: "quoted_identifier".to_string(),
            text: trimmed.to_string(),
            value: Some(inner),
            qualifier: None,
            member: None,
        };
    }

    // `::`-qualified
    if let Some(colon_idx) = trimmed.find("::")
        && colon_idx > 0
    {
        let qualifier = trimmed[..colon_idx].trim().to_string();
        let member_raw = trimmed[colon_idx + 2..].trim();
        let mraw_chars: Vec<char> = member_raw.chars().collect();
        let member = if mraw_chars.len() >= 2
            && mraw_chars[0] == '"'
            && mraw_chars[mraw_chars.len() - 1] == '"'
        {
            mraw_chars[1..mraw_chars.len() - 1].iter().collect()
        } else {
            member_raw.to_string()
        };
        let kind = if is_object_type_keyword(&qualifier.to_lowercase()) {
            "database_reference"
        } else {
            "qualified_enum_value"
        };
        return AttributeArg {
            kind: kind.to_string(),
            text: trimmed.to_string(),
            value: Some(member.clone()),
            qualifier: Some(qualifier),
            member: Some(member),
        };
    }

    // Bare token → identifier.
    AttributeArg {
        kind: "identifier".to_string(),
        text: trimmed.to_string(),
        value: Some(trimmed.to_string()),
        qualifier: None,
        member: None,
    }
}

/// Build an `AttributeInfo` from a structured `RawAttr`. Mirrors `abiAttributeInfo`.
fn abi_attribute_info(a: &RawAttr) -> AttributeInfo {
    let name = a.name.clone().unwrap_or_default();
    let args: Vec<AttributeArg> = a
        .arguments
        .as_ref()
        .map(|args| args.iter().map(|x| classify_abi_arg(&x.value)).collect())
        .unwrap_or_default();
    let raw = if args.is_empty() {
        format!("[{name}]")
    } else {
        let joined: Vec<String> = args.iter().map(|x| x.text.clone()).collect();
        format!("[{}({})]", name, joined.join(", "))
    };
    AttributeInfo { name, args, raw }
}

/// Reconstruct AL SOURCE-SHAPED return-type text from a parsed `RawTypeDef`
/// (Task 2) — fail-closed per the task's round-1/round-2 landmines. Declining
/// to `None` is always preferred over synthesizing a plausible-looking but
/// possibly-wrong type reference; this function NEVER escapes, truncates, or
/// approximates.
///
/// - No `Name` at all → `None` (nothing to reconstruct).
/// - `Name` present, no `Subtype` → the bare `Name` text UNCHANGED (e.g.
///   `HttpHeaders`, or a generic/container shape like `List of [Codeunit
///   "X"]` — passed through as-is; a downstream scalar-typed consumer
///   declines a non-scalar shape itself, this function never approximates a
///   container into a scalar).
/// - `Subtype.Name` present and quote-free → `"{Name} \"{Subtype.Name}\""`
///   (source-shaped, quoted — e.g. `Codeunit "Http Content"`). A
///   namespace/dot-qualified `Subtype.Name` is carried verbatim, never
///   truncated.
/// - `Subtype.Name` present but CONTAINS a `"` → `None`. Re-quoting a name
///   that already carries a quote character would require escaping, and this
///   function must never synthesize escaped text a downstream text
///   classifier could misparse (round-1 M2 / round-2 gemini landmine) —
///   decline rather than guess at an escaping convention.
/// - `Subtype.Id` present but NO `Subtype.Name` → `None`. AL object ids are
///   NOT cross-app unique — a bare numeric reconstruction could resolve to
///   the WRONG app's object. Never synthesize a numeric type reference
///   (round-1 critical).
/// - `Subtype` present but carries neither `Name` nor `Id` → `None`
///   (defensive; nothing usable to reconstruct).
fn reconstruct_return_type_text(t: &RawTypeDef) -> Option<String> {
    let outer_name = t.name.as_deref()?;
    match &t.subtype {
        None => Some(outer_name.to_string()),
        Some(sub) => match &sub.name {
            Some(sub_name) if !sub_name.contains('"') => {
                Some(format!("{outer_name} \"{sub_name}\""))
            }
            _ => None,
        },
    }
}

/// Extract the raw `(name, id)` cross-validation pair from a `RawTypeDef`'s
/// `Subtype` — `Some` ONLY when both fields are present (Task 2; see
/// [`AbiRoutine::return_type_id`]'s doc for why this is independent of
/// [`reconstruct_return_type_text`]'s fail-closed TEXT rules).
fn return_type_subtype_id(t: &RawTypeDef) -> Option<(String, i64)> {
    let sub = t.subtype.as_ref()?;
    Some((sub.name.clone()?, sub.id?))
}

/// Classify a method as a routine, deriving event-publisher kind from attributes.
/// Mirrors `parseMethod`.
fn parse_method(m: &RawMethod) -> AbiRoutine {
    let raw_attrs = m.attributes.clone().unwrap_or_default();
    let attributes: Vec<String> = raw_attrs.iter().map(attr_string).collect();
    let attributes_parsed: Vec<AttributeInfo> = raw_attrs.iter().map(abi_attribute_info).collect();

    let mut kind = "procedure".to_string();
    let mut event_kind = AbiEventKind::Unknown;
    for info in &attributes_parsed {
        match info.name.to_lowercase().as_str() {
            "integrationevent" => {
                kind = "event-publisher".to_string();
                event_kind = AbiEventKind::Integration;
            }
            "businessevent" => {
                kind = "event-publisher".to_string();
                event_kind = AbiEventKind::Business;
            }
            "eventsubscriber" => {
                kind = "event-subscriber".to_string();
            }
            _ => {}
        }
    }

    // Tri-state arity (Task 1, round-2 hardening): `Some(_)` — even `Some(vec![])`
    // for a genuine 0-arg procedure — means the JSON carried a real `Parameters`
    // array; `None` means the field was absent/unparseable, so the arity is
    // UNKNOWN, never zero. `abi_ingest` maps `!parameters_known` to a sentinel
    // that can never arity-match a real call site.
    let parameters_known = m.parameters.is_some();

    AbiRoutine {
        name: m.name.clone().unwrap_or_default(),
        kind,
        event_kind,
        parameters: m
            .parameters
            .clone()
            .unwrap_or_default()
            .iter()
            .map(|p| AbiParameter {
                name: p.name.clone().unwrap_or_default(),
                type_text: p
                    .type_definition
                    .as_ref()
                    .and_then(|t| t.name.clone())
                    .unwrap_or_default(),
                is_var: p.is_var == Some(true),
                // Task 6 (G7, RV-4): a record param carries `TypeDefinition.Temporary`
                // when declared `temporary` in source (verified against a real Continia
                // Core 29.0 SymbolReference.json). Read it so the ABI→L3 projection can
                // model the same Known(true) temp shape the native path produces.
                is_temporary: p.type_definition.as_ref().and_then(|t| t.temporary) == Some(true),
            })
            .collect(),
        parameters_known,
        return_type_text: m
            .return_type_definition
            .as_ref()
            .and_then(reconstruct_return_type_text),
        return_type_id: m
            .return_type_definition
            .as_ref()
            .and_then(return_type_subtype_id),
        is_local: m.is_local == Some(true),
        is_internal: m.is_internal == Some(true),
        is_protected: m.is_protected == Some(true),
        attributes,
        attributes_parsed,
    }
}

/// Mirror `parseField`.
fn parse_field(f: &RawField) -> AbiField {
    let data_type = f
        .type_definition
        .as_ref()
        .and_then(|t| t.name.clone())
        .unwrap_or_default();
    let mut field_class = "Normal".to_string();
    for p in f.properties.clone().unwrap_or_default().iter() {
        if p.name.clone().unwrap_or_default().to_lowercase() == "fieldclass" {
            let v = p.value.clone().unwrap_or_default().to_lowercase();
            if v.contains("flowfield") {
                field_class = "FlowField".to_string();
            } else if v.contains("flowfilter") {
                field_class = "FlowFilter".to_string();
            }
        }
    }
    // isBlobLike = BLOB_LIKE.has((dataType.split("[")[0] ?? dataType).toLowerCase())
    let base_token = data_type
        .split('[')
        .next()
        .unwrap_or(&data_type)
        .to_lowercase();
    AbiField {
        field_number: f.id.unwrap_or(0),
        name: f.name.clone().unwrap_or_default(),
        data_type,
        field_class,
        is_blob_like: blob_like(&base_token),
    }
}

fn parse_key(k: &RawKey) -> AbiKey {
    AbiKey {
        name: k.name.clone().unwrap_or_default(),
        field_names: k.field_names.clone().unwrap_or_default(),
    }
}

/// Recursively walk a page's control tree and collect Kind-6 (Part) and Kind-10
/// (UserControl) entries into `out` as `(name, kind_str, target)` tuples.
fn collect_page_controls(controls: &[RawControl], out: &mut Vec<(String, String, String)>) {
    for c in controls {
        let name = c.name.clone().unwrap_or_default();
        match c.kind {
            Some(6) => {
                if let Some(id) = c.related_page_part_id.as_ref().and_then(|r| r.id) {
                    out.push((name, "part".into(), id.to_string()));
                }
            }
            Some(10) => {
                if let Some(addin) = &c.related_control_addin {
                    out.push((name, "usercontrol".into(), addin.clone()));
                }
            }
            _ => {}
        }
        if let Some(sub) = &c.controls {
            collect_page_controls(sub, out);
        }
    }
}

/// Extract a typed array of `RawObject` from the top-level JSON map at `key`.
/// Missing / wrong-typed → empty (mirrors `(raw[key] as RawObject[]) ?? []`).
/// Collect every object array under `key` — at the top level AND recursively inside
/// every `Namespaces[]` node. BC 24+ symbol files nest objects under namespace nodes
/// (e.g. `Namespaces[].Microsoft.Sales.Document.Pages`); older files keep them flat
/// at the root. Both are gathered so a modern dependency `.app` projects its FULL
/// object set — without namespace recursion ~99% of a modern Base Application's
/// objects (and every routine/table they carry) are silently dropped, which is the
/// dominant cross-app resolution hole.
fn raw_objects(map: &serde_json::Map<String, Value>, key: &str) -> Vec<RawObject> {
    let mut out = Vec::new();
    collect_raw_objects(map, key, &mut out);
    out
}

fn collect_raw_objects(map: &serde_json::Map<String, Value>, key: &str, out: &mut Vec<RawObject>) {
    if let Some(v) = map.get(key)
        && let Ok(objs) = serde_json::from_value::<Vec<RawObject>>(v.clone())
    {
        out.extend(objs);
    }
    if let Some(Value::Array(namespaces)) = map.get("Namespaces") {
        for ns in namespaces {
            if let Value::Object(ns_map) = ns {
                collect_raw_objects(ns_map, key, out);
            }
        }
    }
}

/// Parse the raw `SymbolReference.json` text into the neutral ABI DTO. Never panics.
/// Mirrors `parseSymbolReference`.
pub fn parse_symbol_reference(json: &str) -> SymbolReferenceAbi {
    let parsed: Result<Value, _> = serde_json::from_str(json);
    let root = match parsed {
        Ok(Value::Object(m)) => m,
        Ok(_) => {
            // Non-object JSON: TS would treat property access as undefined and emit
            // empty collections with empty identity (no error). Reproduce that.
            return SymbolReferenceAbi::default();
        }
        Err(e) => {
            return SymbolReferenceAbi {
                error: Some(format!("SymbolReference.json parse failed: {e}")),
                ..Default::default()
            };
        }
    };

    let mut objects: Vec<AbiObject> = Vec::new();
    let mut tables: Vec<AbiTable> = Vec::new();

    // ROUTINE_BEARING
    const ROUTINE_BEARING: [(&str, &str); 6] = [
        ("Codeunits", "Codeunit"),
        ("Pages", "Page"),
        ("Reports", "Report"),
        ("XmlPorts", "XMLport"),
        ("Queries", "Query"),
        ("Interfaces", "Interface"),
    ];
    for (key, object_type) in ROUTINE_BEARING {
        for o in raw_objects(&root, key) {
            let mut abi_object = AbiObject {
                object_type: object_type.to_string(),
                object_number: o.id.unwrap_or(0),
                name: o.name.clone().unwrap_or_default(),
                routines: o
                    .methods
                    .clone()
                    .unwrap_or_default()
                    .iter()
                    .map(parse_method)
                    .collect(),
                ..Default::default()
            };
            if object_type == "Codeunit"
                && let Some(subtype) = raw_object_property(&o.properties, "Subtype")
            {
                abi_object.object_subtype = Some(subtype);
            }
            if object_type == "Page" {
                if let Some(pt) = raw_object_property(&o.properties, "PageType") {
                    abi_object.page_type = Some(pt);
                }
                if let Some(st) = raw_object_property(&o.properties, "SourceTable") {
                    abi_object.source_table_name = Some(unquote_abi_name(&st));
                }
                let mut page_controls: Vec<(String, String, String)> = Vec::new();
                if let Some(controls) = &o.controls {
                    collect_page_controls(controls, &mut page_controls);
                }
                abi_object.page_controls = page_controls;
            }
            if let Some(ifaces) = &o.implemented_interfaces {
                abi_object.implemented_interfaces =
                    Some(ifaces.iter().map(|s| parse_abi_interface_name(s)).collect());
            }
            if let Some(icb_raw) = raw_object_property(&o.properties, "InherentCommitBehavior") {
                let member = match icb_raw.rfind("::") {
                    Some(sep) => icb_raw[sep + 2..].to_lowercase(),
                    None => icb_raw.to_lowercase(),
                };
                match member.as_str() {
                    "ignore" => abi_object.inherent_commit_behavior = Some("ignore".to_string()),
                    "error" => abi_object.inherent_commit_behavior = Some("error".to_string()),
                    "allow" => abi_object.inherent_commit_behavior = Some("allow".to_string()),
                    _ => {}
                }
            }
            objects.push(abi_object);
        }
    }

    // EXTENSION_ROUTINE_BEARING
    const EXTENSION_ROUTINE_BEARING: [(&str, &str); 2] = [
        ("TableExtensions", "TableExtension"),
        ("PageExtensions", "PageExtension"),
    ];
    for (key, object_type) in EXTENSION_ROUTINE_BEARING {
        for o in raw_objects(&root, key) {
            let mut abi_object = AbiObject {
                object_type: object_type.to_string(),
                object_number: o.id.unwrap_or(0),
                name: o.name.clone().unwrap_or_default(),
                routines: o
                    .methods
                    .clone()
                    .unwrap_or_default()
                    .iter()
                    .map(parse_method)
                    .collect(),
                ..Default::default()
            };
            if let Some(target) = &o.target_object {
                abi_object.extends_target_name = Some(unquote_abi_name(target));
            }
            objects.push(abi_object);
            if object_type == "TableExtension" {
                tables.push(AbiTable {
                    object_number: o.id.unwrap_or(0),
                    name: o.name.clone().unwrap_or_default(),
                    fields: o
                        .fields
                        .clone()
                        .unwrap_or_default()
                        .iter()
                        .map(parse_field)
                        .collect(),
                    keys: o
                        .keys
                        .clone()
                        .unwrap_or_default()
                        .iter()
                        .map(parse_key)
                        .collect(),
                    // A TableExtension never declares TableType, but read it
                    // structurally for uniformity (yields false absent the property).
                    is_temporary: raw_table_is_temporary(&o.properties),
                });
            }
        }
    }

    // Tables → both an AbiTable and an AbiObject.
    for t in raw_objects(&root, "Tables") {
        let object_number = t.id.unwrap_or(0);
        tables.push(AbiTable {
            object_number,
            name: t.name.clone().unwrap_or_default(),
            fields: t
                .fields
                .clone()
                .unwrap_or_default()
                .iter()
                .map(parse_field)
                .collect(),
            keys: t
                .keys
                .clone()
                .unwrap_or_default()
                .iter()
                .map(parse_key)
                .collect(),
            // Task 6 (G7, RV-4): read the table-level `TableType = Temporary` marker.
            is_temporary: raw_table_is_temporary(&t.properties),
        });
        objects.push(AbiObject {
            object_type: "Table".to_string(),
            object_number,
            name: t.name.clone().unwrap_or_default(),
            routines: t
                .methods
                .clone()
                .unwrap_or_default()
                .iter()
                .map(parse_method)
                .collect(),
            ..Default::default()
        });
    }

    // BARE
    const BARE: [(&str, &str); 6] = [
        ("EnumTypes", "Enum"),
        ("ControlAddIns", "ControlAddIn"),
        ("PermissionSets", "PermissionSet"),
        ("PermissionSetExtensions", "PermissionSetExtension"),
        ("ReportExtensions", "ReportExtension"),
        ("DotNetPackages", "DotNetPackage"),
    ];
    for (key, object_type) in BARE {
        for o in raw_objects(&root, key) {
            let mut abi_object = AbiObject {
                object_type: object_type.to_string(),
                object_number: o.id.unwrap_or(0),
                name: o.name.clone().unwrap_or_default(),
                routines: Vec::new(),
                ..Default::default()
            };
            if let Some(ifaces) = &o.implemented_interfaces {
                abi_object.implemented_interfaces =
                    Some(ifaces.iter().map(|s| parse_abi_interface_name(s)).collect());
            }
            objects.push(abi_object);
        }
    }

    let str_prop = |k: &str| -> String {
        match root.get(k) {
            Some(Value::String(s)) => s.clone(),
            Some(Value::Null) | None => String::new(),
            Some(other) => other.to_string(),
        }
    };

    SymbolReferenceAbi {
        app_guid: str_prop("AppId"),
        name: str_prop("Name"),
        publisher: str_prop("Publisher"),
        version: str_prop("Version"),
        objects,
        tables,
        error: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_boolean_primitive() {
        let a = classify_abi_arg(&Some(Value::Bool(false)));
        assert_eq!(a.kind, "boolean");
        assert_eq!(a.text, "false");
        assert_eq!(a.value.as_deref(), Some("false"));
    }

    #[test]
    fn classify_database_reference() {
        let a = classify_abi_arg(&Some(Value::String("Codeunit::\"X\"".to_string())));
        assert_eq!(a.kind, "database_reference");
        assert_eq!(a.qualifier.as_deref(), Some("Codeunit"));
        assert_eq!(a.member.as_deref(), Some("X"));
        assert_eq!(a.value.as_deref(), Some("X"));
    }

    #[test]
    fn classify_qualified_enum_value() {
        let a = classify_abi_arg(&Some(Value::String("ObjectType::Codeunit".to_string())));
        assert_eq!(a.kind, "qualified_enum_value");
        assert_eq!(a.qualifier.as_deref(), Some("ObjectType"));
        assert_eq!(a.member.as_deref(), Some("Codeunit"));
    }

    #[test]
    fn classify_string_literal_and_empty() {
        let a = classify_abi_arg(&Some(Value::String("'Event'".to_string())));
        assert_eq!(a.kind, "string_literal");
        assert_eq!(a.value.as_deref(), Some("Event"));
        let b = classify_abi_arg(&Some(Value::String("''".to_string())));
        assert_eq!(b.kind, "string_literal");
        assert_eq!(b.value.as_deref(), Some(""));
    }

    #[test]
    fn interface_name_strips_guid_prefix_and_quotes() {
        assert_eq!(parse_abi_interface_name("\"IFoo\""), "IFoo");
        assert_eq!(parse_abi_interface_name("IBar"), "IBar");
        assert_eq!(
            parse_abi_interface_name("#63ca2fa4#\"Telemetry Logger\""),
            "Telemetry Logger"
        );
    }

    #[test]
    fn bad_json_yields_error_not_panic() {
        let abi = parse_symbol_reference("{ not json");
        assert!(abi.error.is_some());
        assert!(abi.objects.is_empty());
    }

    // -- Task 1: `IsProtected` parsing + tri-state arity ---------------------

    #[test]
    fn parse_method_reads_is_protected() {
        let json = r##"{
            "RuntimeVersion":"11.0",
            "Tables":[],
            "Codeunits":[{"Id":50100,"Name":"ProtTest","Methods":[
                {"Name":"P","IsProtected":true,"Parameters":[]},
                {"Name":"Pub","Parameters":[]}
            ]}],
            "Pages":[],"Reports":[],"XmlPorts":[],"Queries":[],"Interfaces":[],
            "EnumTypes":[],"TableExtensions":[],
            "AppId":"x","Name":"x","Publisher":"x","Version":"1.0.0.0"
        }"##;
        let abi = parse_symbol_reference(json);
        let cu = abi.objects.first().expect("Codeunit must parse");
        let p = cu
            .routines
            .iter()
            .find(|r| r.name == "P")
            .expect("P must exist");
        assert!(p.is_protected, "IsProtected:true must map to is_protected");
        let pub_r = cu
            .routines
            .iter()
            .find(|r| r.name == "Pub")
            .expect("Pub must exist");
        assert!(
            !pub_r.is_protected,
            "absent IsProtected must default to false"
        );
    }

    #[test]
    fn parse_method_arity_is_tri_state() {
        // "Parameters":[] (present, empty) → known arity 0.
        let json_present = r##"{"Name":"Foo","Parameters":[]}"##;
        let raw_present: RawMethod = serde_json::from_str(json_present).unwrap();
        let m_present = parse_method(&raw_present);
        assert!(
            m_present.parameters_known,
            "an explicit empty Parameters array is a KNOWN 0-arity, not unknown"
        );
        assert_eq!(m_present.parameters.len(), 0);

        // No "Parameters" key at all → unknown arity, never zero.
        let json_absent = r##"{"Name":"Foo"}"##;
        let raw_absent: RawMethod = serde_json::from_str(json_absent).unwrap();
        let m_absent = parse_method(&raw_absent);
        assert!(
            !m_absent.parameters_known,
            "an absent Parameters field must be UNKNOWN arity, never zero"
        );
        assert_eq!(
            m_absent.parameters.len(),
            0,
            "the parameters Vec is still empty for display purposes, but \
             parameters_known=false is what callers must gate on"
        );
    }

    // -- Task 2: structured ABI return types (`Subtype`) ---------------------
    //
    // `reconstruct_return_type_text` / `return_type_subtype_id` — source-shaped
    // reconstruction, fail-closed per the task brief's round-1/round-2
    // landmines. Every case below drives the FULL `parse_method` path from raw
    // JSON (never calls the private helpers directly), matching the style of
    // `parse_method_reads_is_protected` above.

    // (a) Name + Subtype{Name, Id} both present → quoted source-shaped text,
    // AND the structured (name, id) pair is retained for downstream
    // cross-validation.
    #[test]
    fn parse_method_return_type_subtype_reconstructs_quoted_source_shape() {
        let json = r##"{
            "Name":"GetHttpContent",
            "ReturnTypeDefinition":{"Name":"Codeunit","Subtype":{"Name":"Http Content","Id":2354}}
        }"##;
        let raw: RawMethod = serde_json::from_str(json).unwrap();
        let m = parse_method(&raw);
        assert_eq!(
            m.return_type_text.as_deref(),
            Some("Codeunit \"Http Content\""),
            "Name-preferred quoted source shape"
        );
        assert_eq!(
            m.return_type_id,
            Some(("Http Content".to_string(), 2354)),
            "the structured (name, id) pair must be retained for Task 3's \
             cross-object chain cross-validation"
        );
    }

    // (b) bare Name, no Subtype at all → unchanged pass-through.
    #[test]
    fn parse_method_return_type_bare_name_passthrough() {
        let json = r##"{"Name":"Foo","ReturnTypeDefinition":{"Name":"HttpHeaders"}}"##;
        let raw: RawMethod = serde_json::from_str(json).unwrap();
        let m = parse_method(&raw);
        assert_eq!(m.return_type_text.as_deref(), Some("HttpHeaders"));
        assert_eq!(
            m.return_type_id, None,
            "no Subtype at all means no (name, id) pair to carry"
        );
    }

    // (c) Subtype carries an Id but NO Name → DECLINE to `None`. AL object ids
    // are NOT cross-app unique; a bare numeric reconstruction could resolve to
    // the WRONG app's object (round-1 critical — fail closed, never
    // synthesize).
    #[test]
    fn parse_method_return_type_id_only_declines() {
        let json = r##"{
            "Name":"Foo",
            "ReturnTypeDefinition":{"Name":"Codeunit","Subtype":{"Id":2354}}
        }"##;
        let raw: RawMethod = serde_json::from_str(json).unwrap();
        let m = parse_method(&raw);
        assert_eq!(
            m.return_type_text, None,
            "Id-only Subtype must decline the WHOLE reconstruction, not fall \
             back to the bare outer Name"
        );
        assert_eq!(
            m.return_type_id, None,
            "no Name means no valid (name, id) pair — a lone id is not proof \
             of identity"
        );
    }

    // (f) FORMAT LANDMINE: Subtype.Name contains a quote character → the TEXT
    // reconstruction must strictly decline (never escape — downstream text
    // classification must never see synthesized escaping), but the raw
    // (name, id) identity pair is STILL carried: cross-validation is a
    // structured `==` comparison, never text synthesis, so the quote landmine
    // does not apply to it.
    #[test]
    fn parse_method_return_type_subtype_name_with_quote_declines_text_only() {
        let json = r##"{
            "Name":"Foo",
            "ReturnTypeDefinition":{"Name":"Codeunit","Subtype":{"Name":"Weird\"Name","Id":1}}
        }"##;
        let raw: RawMethod = serde_json::from_str(json).unwrap();
        let m = parse_method(&raw);
        assert_eq!(
            m.return_type_text, None,
            "a Subtype Name containing a quote must never be synthesized/escaped \
             into source-shaped text"
        );
        assert_eq!(
            m.return_type_id,
            Some(("Weird\"Name".to_string(), 1)),
            "the raw identity pair is independent of the TEXT landmine — it is \
             never used to synthesize text"
        );
    }

    // (f) FORMAT LANDMINE: a namespace/dot-qualified Subtype.Name must be
    // carried verbatim — never truncated.
    #[test]
    fn parse_method_return_type_namespace_qualified_subtype_name_not_truncated() {
        let json = r##"{
            "Name":"Foo",
            "ReturnTypeDefinition":{"Name":"Codeunit","Subtype":{"Name":"My.Namespace.Http Content","Id":9}}
        }"##;
        let raw: RawMethod = serde_json::from_str(json).unwrap();
        let m = parse_method(&raw);
        assert_eq!(
            m.return_type_text.as_deref(),
            Some("Codeunit \"My.Namespace.Http Content\""),
            "a dot-qualified name must be carried verbatim, never truncated at \
             the first `.`"
        );
        assert_eq!(
            m.return_type_id,
            Some(("My.Namespace.Http Content".to_string(), 9))
        );
    }

    // (f) FORMAT LANDMINE: a generic/container return (`List of [...]`, no
    // Subtype) passes through as-is — never approximated into a scalar.
    #[test]
    fn parse_method_return_type_generic_container_passthrough() {
        let json = r##"{
            "Name":"Foo",
            "ReturnTypeDefinition":{"Name":"List of [Codeunit \"Http Content\"]"}
        }"##;
        let raw: RawMethod = serde_json::from_str(json).unwrap();
        let m = parse_method(&raw);
        assert_eq!(
            m.return_type_text.as_deref(),
            Some("List of [Codeunit \"Http Content\"]"),
            "a generic/container return with no Subtype passes through as-is — \
             scalar-declined downstream, never approximated here"
        );
        assert_eq!(m.return_type_id, None);
    }

    // Defensive: a Subtype object present but carrying NEITHER Name nor Id →
    // decline (nothing usable to reconstruct or cross-validate).
    #[test]
    fn parse_method_return_type_empty_subtype_declines() {
        let json = r##"{
            "Name":"Foo",
            "ReturnTypeDefinition":{"Name":"Codeunit","Subtype":{}}
        }"##;
        let raw: RawMethod = serde_json::from_str(json).unwrap();
        let m = parse_method(&raw);
        assert_eq!(m.return_type_text, None);
        assert_eq!(m.return_type_id, None);
    }

    // No ReturnTypeDefinition at all → both fields `None` (control).
    #[test]
    fn parse_method_no_return_type_definition_yields_none() {
        let json = r##"{"Name":"Foo"}"##;
        let raw: RawMethod = serde_json::from_str(json).unwrap();
        let m = parse_method(&raw);
        assert_eq!(m.return_type_text, None);
        assert_eq!(m.return_type_id, None);
    }
}
