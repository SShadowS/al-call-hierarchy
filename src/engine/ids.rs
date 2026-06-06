//! R0 identity encoders — Rust ports of al-sem's object/routine identity
//! functions. These MUST reproduce al-sem's output byte-for-byte; the
//! differential oracle is `tests/encoder_vectors.rs` against the committed
//! vectors in `tests/r0-vectors/encoder-vectors.json`.
//!
//! Cross-port invariants worth knowing before touching anything here:
//! - `sha256_of_strings` length-prefixes each part with its **UTF-16 code-unit
//!   count** (JS `String.length`), NOT byte length and NOT Unicode scalar
//!   count. `"😀"` → prefix `"2"`, `"é"` → prefix `"1"`. See [`utf16_len`].
//! - object/routine IDs are built from `/`-separated internal forms; the
//!   "stable" forms swap `/` → `:` (object) or append `#hash` (routine).

use sha2::{Digest, Sha256};

/// A routine parameter as it feeds the canonical signature: the raw type text
/// and whether it is passed by reference (`var`).
#[derive(Debug, Clone)]
pub struct ParamSpec {
    pub type_text: String,
    pub is_var: bool,
}

/// The cross-app-stable key for a routine, mirroring al-sem's
/// `CanonicalRoutineKey`.
#[derive(Debug, Clone)]
pub struct CanonicalRoutineKey {
    pub app_guid: String,
    pub object_type: String,
    pub object_number: i64,
    pub routine_kind: String,
    pub routine_name: String,
    pub normalized_signature_hash: String,
}

/// Count of UTF-16 code units in `s` — equal to JavaScript's `String.length`.
///
/// This is the prefix length [`sha256_of_strings`] feeds before each part, so
/// it deliberately counts surrogate pairs as 2 (e.g. `"😀"` → 2) rather than
/// counting scalars (1) or bytes (4).
fn utf16_len(s: &str) -> usize {
    s.encode_utf16().count()
}

/// Lowercase hex of the SHA-256 of `s` interpreted as UTF-8 bytes.
pub fn sha256_hex(s: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(s.as_bytes());
    hex_lower(&hasher.finalize())
}

/// Hash an ordered list of strings with an unambiguous, JS-`String.length`
/// based framing: for each part feed `"<utf16_len>:" + part_utf8_bytes`.
///
/// The length prefix is the UTF-16 code-unit count, NOT the byte length — this
/// is the JS `String.length` contract and getting it wrong silently breaks
/// every RoutineId. See [`utf16_len`].
pub fn sha256_of_strings(parts: &[String]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(utf16_len(part).to_string().as_bytes());
        hasher.update(b":");
        hasher.update(part.as_bytes());
    }
    hex_lower(&hasher.finalize())
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// Internal object id: `"{appGuid}/{objectType}/{objectNumber}"`, no
/// normalization (appGuid casing kept verbatim).
pub fn encode_object_id(app_guid: &str, object_type: &str, object_number: i64) -> String {
    format!("{app_guid}/{object_type}/{object_number}")
}

/// Stable object id: replace every `/` in the internal id with `:`.
pub fn to_stable_object_id(internal_object_id: &str) -> String {
    internal_object_id.replace('/', ":")
}

/// Canonical, case-insensitive routine signature string.
///
/// `"{name_lower}({param_specs_joined_by_';'}):{return_lower}"` where each param
/// spec is `(var )?{type_lower_trimmed}` and `return` defaults to empty when
/// absent. Only the ends of type/return text are trimmed — inner whitespace
/// (e.g. inside `Record "Sales Line"`) is preserved.
pub fn canonical_routine_signature(
    name: &str,
    parameters: &[ParamSpec],
    return_type_text: Option<&str>,
) -> String {
    let params = parameters
        .iter()
        .map(|p| {
            let prefix = if p.is_var { "var " } else { "" };
            format!("{prefix}{}", p.type_text.trim().to_lowercase())
        })
        .collect::<Vec<_>>()
        .join(";");

    let ret = return_type_text.unwrap_or("").trim().to_lowercase();

    format!("{}({}):{}", name.to_lowercase(), params, ret)
}

/// SHA-256 hex of the canonical routine signature — the return-type-aware
/// normalized signature hash.
pub fn normalized_signature_hash(
    name: &str,
    parameters: &[ParamSpec],
    return_type_text: Option<&str>,
) -> String {
    sha256_hex(&canonical_routine_signature(
        name,
        parameters,
        return_type_text,
    ))
}

/// Return-type-aware routine fingerprint — identical computation to
/// [`normalized_signature_hash`] (al-sem unified these).
pub fn routine_signature_fingerprint(
    name: &str,
    parameters: &[ParamSpec],
    return_type_text: Option<&str>,
) -> String {
    sha256_hex(&canonical_routine_signature(
        name,
        parameters,
        return_type_text,
    ))
}

/// Canonical routine key hash: `sha256_of_strings` over the 6 ordered parts
/// `[appGuid, objectType, objectNumber, routineKind, routineName_lower,
/// normalizedSignatureHash]`.
pub fn encode_canonical_routine_key(key: &CanonicalRoutineKey) -> String {
    sha256_of_strings(&[
        key.app_guid.clone(),
        key.object_type.clone(),
        key.object_number.to_string(),
        key.routine_kind.clone(),
        key.routine_name.to_lowercase(),
        key.normalized_signature_hash.clone(),
    ])
}

/// Full RoutineId: `"{modelInstanceId}/{canonicalRoutineKeyHash}"`.
pub fn encode_routine_id(key: &CanonicalRoutineKey, model_instance_id: &str) -> String {
    format!("{model_instance_id}/{}", encode_canonical_routine_key(key))
}

/// Stable routine id from its parts: `"{stableObjectId}#{normalizedSignatureHash}"`.
pub fn to_stable_routine_id_from_parts(
    stable_object_id: &str,
    normalized_signature_hash: &str,
) -> String {
    format!("{stable_object_id}#{normalized_signature_hash}")
}

/// Object signature fingerprint: `sha256("{objectType}|{objectNumber}|{name}")`.
pub fn object_signature_fingerprint(object_type: &str, object_number: i64, name: &str) -> String {
    sha256_hex(&format!("{object_type}|{object_number}|{name}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Locks the UTF-16 length-prefix landmine independently of the generated
    // vectors: an emoji is a surrogate pair → JS String.length == 2, so the
    // single-part framing is "2:" + utf8(😀).
    #[test]
    fn sha256_of_strings_uses_utf16_length_prefix_for_emoji() {
        assert_eq!(utf16_len("😀"), 2);
        // Hand-derived: SHA-256 of bytes "2:" followed by the 4 UTF-8 bytes of 😀.
        let mut h = Sha256::new();
        h.update(b"2:");
        h.update("😀".as_bytes());
        let expected = hex_lower(&h.finalize());
        assert_eq!(sha256_of_strings(&["😀".to_string()]), expected);
    }

    #[test]
    fn utf16_len_counts_code_units_not_scalars_or_bytes() {
        assert_eq!(utf16_len(""), 0);
        assert_eq!(utf16_len("a"), 1);
        assert_eq!(utf16_len("é"), 1); // 1 UTF-16 unit, 2 UTF-8 bytes
        assert_eq!(utf16_len("café"), 4); // NOT byte length 5
        assert_eq!(utf16_len("😀"), 2); // surrogate pair, NOT 1 scalar / 4 bytes
        assert_eq!(utf16_len("a😀b"), 4); // 1 + 2 + 1
    }

    #[test]
    fn sha256_hex_empty_is_known_digest() {
        assert_eq!(
            sha256_hex(""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }
}
