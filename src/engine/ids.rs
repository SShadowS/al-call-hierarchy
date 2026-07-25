//! R0 identity encoders — originally Rust ports of al-sem's object/routine
//! identity functions, now **Rust-owned** (CLAUDE.md, "al-sem retirement is
//! COMPLETE"). The regression oracle is `tests/l2_ir/encoder_vectors.rs` against
//! the committed vectors in `tests/r0-vectors/encoder-vectors.json` — Rust-owned
//! baselines, not an al-sem differential.
//!
//! **These no longer reproduce al-sem's output byte-for-byte, and must not be
//! "fixed" back to it.** [`encode_canonical_routine_key`] appends a CONDITIONAL
//! 7th key part — the enclosing-member discriminator (task 3,
//! `feat/l3-substrate-and-parked-items`) — so a member trigger's id is
//! deliberately an id al-sem never minted. Every routine WITHOUT an enclosing
//! member (`enclosing_member: None`: procedures, object-level triggers, and every
//! dependency-ABI routine) still hashes the original six parts and is byte-identical
//! to the pre-discriminator id; the committed vectors contain zero `enclosingMember`
//! entries and therefore all take that branch unchanged.
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
    /// The member (table field / page field / action / dataitem / …) that
    /// ENCLOSES a member trigger — the discriminator that separates the N
    /// `trigger OnAction()` bodies of one page from each other. `None` for
    /// procedures, object-level triggers (`OnRun`/`OnOpenPage`) and every
    /// dependency-ABI routine.
    ///
    /// Two hard contracts, both load-bearing:
    ///
    /// 1. **The string must be the UNESCAPED logical identifier** (inner `""`
    ///    collapsed to `"`), which is exactly what
    ///    [`crate::engine::l2::ir_walk::ir_enclosing_member`] produces — the
    ///    single source every call site uses. The raw IR value is only
    ///    outer-quote-stripped, so feeding it here directly would mint a
    ///    different id for the same routine from a different code path.
    ///    [`encode_canonical_routine_key`] lowercases (AL identifiers are
    ///    case-insensitive) but deliberately does NOT unescape — normalizing
    ///    twice is not idempotent for a member like `"a""""b"`.
    /// 2. **`None` must stay byte-identical to the pre-discriminator id.**
    ///    [`encode_canonical_routine_key`] therefore appends a 7th hash part
    ///    only when this is `Some` — see its doc for why `Some("")` and `None`
    ///    are still distinguishable.
    pub enclosing_member: Option<String>,
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

/// SHA-256 hex of raw bytes (the cli-b snapshot `deriveInputs` file-content hash).
pub fn sha256_bytes_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
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

/// Internal table id: `"{appGuid}/table/{tableNumber}"` (mirrors al-sem
/// `encodeTableId`). Single source of truth — used by both the dependency
/// projection and the L3 extension-field merge, which MUST agree byte-for-byte.
pub(crate) fn encode_table_id(app_guid: &str, table_number: i64) -> String {
    format!("{app_guid}/table/{table_number}")
}

/// Internal field id: `"{tableId}/{fieldNumber}"` (mirrors `encodeFieldId`).
pub(crate) fn encode_field_id(table_id: &str, field_number: i64) -> String {
    format!("{table_id}/{field_number}")
}

/// Internal key id: `"{tableId}/key/{keyIndex}"` (mirrors `encodeKeyId`).
pub(crate) fn encode_key_id(table_id: &str, key_index: usize) -> String {
    format!("{table_id}/key/{key_index}")
}

/// Stable table id: `"{appGuid}:Table:{tableNumber}"` (mirrors `toStableTableId`).
pub(crate) fn to_stable_table_id(app_guid: &str, table_number: i64) -> String {
    format!("{app_guid}:Table:{table_number}")
}

/// Stable field id: `"{stableTableId}#{fieldNumber}"` (mirrors `toStableFieldId`).
pub(crate) fn to_stable_field_id(app_guid: &str, table_number: i64, field_number: i64) -> String {
    format!(
        "{}#{}",
        to_stable_table_id(app_guid, table_number),
        field_number
    )
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
/// normalizedSignatureHash]`, plus a CONDITIONAL 7th part —
/// `enclosing_member_lower` — appended only when the routine has an enclosing
/// member ([`CanonicalRoutineKey::enclosing_member`]).
///
/// **Conditional, not unconditional, and that is the whole design.** Without a
/// member discriminator two `trigger OnAction()` bodies in one page hash to one
/// id (measured: 23.9 % of DO routines, 16.7 % of BC Base App routines collapse
/// onto a shared id). Appending the part only when a member exists means every
/// routine that never collides — procedures, object-level triggers, and the
/// whole dependency-ABI side, which passes `None` — keeps a byte-identical id,
/// so the cross-app join stays symmetric by construction and the committed
/// encoder vectors do not move.
///
/// `sha256_of_strings` length-prefixes every part, so `[…6 parts]` and
/// `[…6 parts, ""]` already hash differently: an empty-named member is still
/// distinguishable from no member at all, and the conditional append is
/// unambiguous rather than merely conventional.
///
/// This changes the hash INPUT only. The id's SHAPE
/// (`{modelInstanceId}/{64 lowercase hex}`) is load-bearing far downstream —
/// `l5::fingerprint`'s `substitute_stable_ids` locates ids by scanning for
/// exactly 64 lowercase-hex bytes and `l4::summary`'s `stable_sub_id` splits on
/// exactly two `/`-parts — so a `#member` suffix or an extra `/`-segment would
/// silently break both and move every fingerprint in the product. Pinned by
/// `routine_id_shape_is_two_parts_with_64_hex_regardless_of_member`.
pub fn encode_canonical_routine_key(key: &CanonicalRoutineKey) -> String {
    let mut parts = vec![
        key.app_guid.clone(),
        key.object_type.clone(),
        key.object_number.to_string(),
        key.routine_kind.clone(),
        key.routine_name.to_lowercase(),
        key.normalized_signature_hash.clone(),
    ];
    // CONDITIONAL: no member → the 6-part hash, byte-identical to the
    // pre-discriminator schema. The value is lowercased (AL identifiers are
    // case-insensitive, exactly like `routine_name` above) but NOT unescaped —
    // callers pass the already-unescaped logical identifier, see
    // `CanonicalRoutineKey::enclosing_member`.
    if let Some(member) = &key.enclosing_member {
        parts.push(member.to_lowercase());
    }
    sha256_of_strings(&parts)
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

// ---------------------------------------------------------------------------
// localeCompare-faithful collation for snapshot sort keys (cli-b binding rule).
//
// al-sem's snapshot derivers sort with `String.localeCompare` (ICU DUCET default
// collation), NOT ordinal byte order. The two diverge for the snapshot alphabet:
//   - punctuation: `:` collates BEFORE `#` BEFORE `|` (ICU), but byte order is the
//     opposite (`#`=0x23 < `:`=0x3a < `|`=0x7c). So `a:` < `a#` in ICU but `a#` <
//     `a:` by bytes — affecting stableId order (`Table:N` vs `Table:N#field`).
//   - letters are CASE-INSENSITIVE at the primary level (lowercase before uppercase
//     only as a tertiary tiebreak), so a mixed-case `.alpackages` filename `A.app`
//     collates among the lowercase `a..z`, not before all of them — shifting both
//     `inputs` order AND the workspaceFingerprint hash built over it.
//
// [`locale_compare`] reproduces ICU's order for the printable-ASCII alphabet that
// AL identifiers / workspace paths use. The PRIMARY rank table below is the EXACT
// `[...printableAscii].sort((a,b)=>a.localeCompare(b))` order from Bun
// (oracle-pinned in `tests/encoder_vectors.rs`). Case is a TERTIARY tiebreak
// (lowercase before uppercase), matching ICU multi-level collation.
//
// Characters outside printable ASCII (rare in AL identifiers / paths) fall back to
// codepoint order shifted ABOVE the known alphabet, so they sort last
// deterministically — a documented, conservative approximation.
// ---------------------------------------------------------------------------

/// Primary collation rank for one char (lower index = sorts earlier). The order is
/// ICU's printable-ASCII DUCET order; each letter's lower/upper pair shares the SAME
/// primary rank (case handled at the tertiary level).
fn locale_primary_rank(c: char) -> u32 {
    // ICU printable-ASCII order (from Bun's localeCompare); each entry is the
    // primary-equivalence class. Letters pair lower+upper at one rank.
    const PUNCT: &[char] = &[
        ' ', '_', '-', ',', ';', ':', '!', '?', '.', '\'', '"', '(', ')', '[', ']', '{', '}', '@',
        '*', '/', '\\', '&', '#', '%', '`', '^', '+', '<', '=', '>', '|', '~', '$',
    ];
    if let Some(i) = PUNCT.iter().position(|&p| p == c) {
        return i as u32; // 0..=32
    }
    let punct_len = PUNCT.len() as u32; // 33
    match c {
        '0'..='9' => punct_len + (c as u32 - '0' as u32), // 33..=42
        'a'..='z' => punct_len + 10 + (c as u32 - 'a' as u32), // 43..=68
        'A'..='Z' => punct_len + 10 + (c as u32 - 'A' as u32), // same primary as lowercase
        // Unknown char: sort last, deterministically, by codepoint above the table.
        _ => 1_000_000 + c as u32,
    }
}

/// Tertiary (case) weight — lowercase before uppercase (ICU default). Non-letters 0.
fn locale_case_weight(c: char) -> u8 {
    match c {
        'A'..='Z' => 1,
        _ => 0,
    }
}

/// `a.localeCompare(b)` for the snapshot sort-key alphabet — two-level ICU collation
/// (primary rank, then case tertiary), matching Bun's `String.localeCompare`. Use
/// this (NOT `str::cmp`) at every snapshot deriver sort site.
pub fn locale_compare(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let ac: Vec<char> = a.chars().collect();
    let bc: Vec<char> = b.chars().collect();
    let n = ac.len().min(bc.len());
    // Primary level.
    for i in 0..n {
        let pa = locale_primary_rank(ac[i]);
        let pb = locale_primary_rank(bc[i]);
        if pa != pb {
            return pa.cmp(&pb);
        }
    }
    if ac.len() != bc.len() {
        return ac.len().cmp(&bc.len());
    }
    // Tertiary (case) level — only on a full primary tie.
    for i in 0..n {
        let ta = locale_case_weight(ac[i]);
        let tb = locale_case_weight(bc[i]);
        if ta != tb {
            return ta.cmp(&tb);
        }
    }
    Ordering::Equal
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

    // -- the enclosing-member discriminator (Task 3) --------------------------

    fn member_key(member: Option<&str>) -> CanonicalRoutineKey {
        CanonicalRoutineKey {
            app_guid: "11111111-2222-3333-4444-555555555555".to_string(),
            object_type: "Page".to_string(),
            object_number: 50811,
            routine_kind: "trigger".to_string(),
            routine_name: "OnAction".to_string(),
            normalized_signature_hash: sha256_hex("onaction():"),
            enclosing_member: member.map(|m| m.to_string()),
        }
    }

    /// THE TRAP, pinned. The discriminator changes the hash INPUT; it must never
    /// change the id's SHAPE. `l5::fingerprint::substitute_stable_ids` finds ids by
    /// scanning for `"{mid}/"` followed by EXACTLY 64 lowercase-hex bytes, and
    /// `l4::summary::stable_sub_id` splits an id into EXACTLY two `/`-parts — a
    /// `#member` suffix or a 7th `/`-segment silently defeats both, and the failure
    /// mode is every fingerprint in the product moving.
    ///
    /// This asserts the shape INDEPENDENTLY of the content, so it holds for any
    /// future key part too.
    #[test]
    fn routine_id_shape_is_two_parts_with_64_hex_regardless_of_member() {
        for member in [None, Some(""), Some("No."), Some(r#"a"b"#), Some("😀")] {
            let id = encode_routine_id(&member_key(member), "r0");
            let parts: Vec<&str> = id.split('/').collect();
            assert_eq!(
                parts.len(),
                2,
                "id must stay exactly two `/`-separated parts (member={member:?}): {id}"
            );
            assert_eq!(parts[0], "r0", "first part is the modelInstanceId verbatim");
            assert_eq!(
                parts[1].len(),
                64,
                "hash part must stay 64 bytes (member={member:?}): {id}"
            );
            assert!(
                parts[1]
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
                "hash part must stay LOWERCASE hex (member={member:?}): {id}"
            );
        }
    }

    /// The closure the discriminator buys, at the encoder level: two routines
    /// identical in all six legacy key parts and differing ONLY in their enclosing
    /// member now get distinct ids. Before Task 3 these were the same id.
    #[test]
    fn distinct_enclosing_members_yield_distinct_routine_ids() {
        let first = encode_canonical_routine_key(&member_key(Some("First")));
        let second = encode_canonical_routine_key(&member_key(Some("Second")));
        assert_ne!(
            first, second,
            "two same-name triggers under different members must not collide"
        );
        // AL identifiers are case-insensitive: the SAME member differing only in
        // case is the same routine and must keep one id.
        assert_eq!(
            first,
            encode_canonical_routine_key(&member_key(Some("FIRST")))
        );
    }

    /// The conditional append, both directions: no member reproduces the exact
    /// 6-part hash (so every non-member routine's id is byte-identical to the
    /// pre-discriminator schema), and `Some("")` is still distinguishable from
    /// `None` because `sha256_of_strings` length-prefixes each part.
    #[test]
    fn absent_member_reproduces_the_six_part_hash_and_is_distinct_from_empty() {
        let key = member_key(None);
        let six_part = sha256_of_strings(&[
            key.app_guid.clone(),
            key.object_type.clone(),
            key.object_number.to_string(),
            key.routine_kind.clone(),
            key.routine_name.to_lowercase(),
            key.normalized_signature_hash.clone(),
        ]);
        assert_eq!(
            encode_canonical_routine_key(&key),
            six_part,
            "a routine with no enclosing member must keep the legacy 6-part hash"
        );
        assert_ne!(
            encode_canonical_routine_key(&member_key(None)),
            encode_canonical_routine_key(&member_key(Some(""))),
            "length-prefixed framing keeps `None` and `Some(\"\")` apart"
        );
    }

    #[test]
    fn sha256_hex_empty_is_known_digest() {
        assert_eq!(
            sha256_hex(""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    // -- localeCompare oracle: ICU order for the snapshot sort-key alphabet --
    //
    // Each `assert_lt` mirrors a Bun `"a".localeCompare("b") < 0` result
    // (oracle-pinned). The key divergences from ordinal `str::cmp`:
    //   - `:` collates BEFORE `#` (ICU) though `#`(0x23) < `:`(0x3a) by bytes;
    //   - letters are case-insensitive at the primary level (lowercase before
    //     uppercase only as a tertiary tiebreak).

    fn assert_lt(a: &str, b: &str) {
        assert_eq!(
            locale_compare(a, b),
            std::cmp::Ordering::Less,
            "expected localeCompare({a:?}, {b:?}) == Less"
        );
        assert_eq!(
            locale_compare(b, a),
            std::cmp::Ordering::Greater,
            "expected localeCompare({b:?}, {a:?}) == Greater"
        );
    }

    #[test]
    fn locale_compare_colon_sorts_before_hash() {
        // Bun: "a:".localeCompare("a#") < 0  (the StableTableId vs field-id case).
        assert_lt("a:", "a#");
        // The empty suffix sorts before the `#`-suffixed field id.
        assert_lt("g:Table:50101", "g:Table:50101#1");
        assert_lt("g:Table:50101#1", "g:Table:50101#2");
        assert_lt("g:Table:50101#2", "g:Table:50101#K0");
    }

    #[test]
    fn locale_compare_is_case_insensitive_primary() {
        // Bun: a mixed-case `.alpackages` filename `A.app` collates AMONG the
        // lowercase a..z (case is tertiary), NOT before all uppercase by byte order.
        // Order: a.app < A.app < m.app < z.app.
        assert_lt("a.app", "A.app");
        assert_lt("A.app", "m.app");
        assert_lt("m.app", "z.app");
        // Tertiary: same primary, lowercase before uppercase.
        assert_eq!(
            locale_compare("codeunit", "CODEUNIT"),
            std::cmp::Ordering::Less
        );
    }

    #[test]
    fn locale_compare_punct_and_digits_order() {
        // ICU: `-` < `:` < `.` < `/` < `#` < `|`, all before digits, digits before letters.
        assert_lt("a-", "a:");
        assert_lt("a:", "a.");
        assert_lt("a.", "a/");
        assert_lt("a/", "a#");
        assert_lt("a#", "a|");
        assert_lt("a|", "a0");
        assert_lt("a9", "aa");
    }

    #[test]
    fn locale_compare_inputs_kind_path_order() {
        // The `kind|path` join keys for the cli-b inputs deriver, ICU-sorted.
        assert_lt("app-json|app.json", "dep-package|.alpackages/a.app");
        assert_lt(
            "dep-package|.alpackages/a.app",
            "policy|al-sem.coverage.yaml",
        );
        assert_lt(
            "policy|al-sem.coverage.yaml",
            "roots-config|roots.config.json",
        );
    }
}
