//! Versioned framework property/method RETURN-TYPE table (beyond-1B.3b Task 4).
//!
//! Distinct from [`crate::program::resolve::member_catalog`] (which answers "does
//! this framework kind have a member named X?" for LEAF Phase-B dispatch): this
//! module answers "what framework kind does `<Framework>.<member>` (or
//! `<Framework>.<method>(...)`) ITSELF evaluate to?" — enabling single-hop
//! compound-receiver chains like `Response.Content().ReadAs(...)` or
//! `JToken.AsObject().Get(...)` to type their INTERMEDIATE receiver before Phase B
//! ever looks up the leaf method.
//!
//! # Clean-room note
//!
//! Ported (APPROACH, not code) from L3's `framework_property_type` /
//! `framework_method_return_type` (`src/engine/l3/member_builtins.rs:380-458`).
//! L3's table is NOT copied verbatim: cross-checking it against the fresh-native,
//! already-validated membership catalog (`member_catalog.rs`, sourced from AL
//! extension `ms-dynamics-smb.al-18.0.2293710`) revealed L3 claims `JsonObject`/
//! `JsonArray`/`JsonValue` also carry `AsValue`/`AsObject`/`AsArray` — they do not
//! (verified against both the membership catalog AND the primary MS Learn
//! `methods-auto` reference, see per-entry provenance below); those broader L3
//! entries are correctly OMITTED here, and Rust-owned is the more accurate
//! baseline per this project's testing philosophy (al-sem retired; no byte-parity
//! chase).
//!
//! # Per-entry validation (not a header comment)
//!
//! Every entry below is verified against the **primary source** — the
//! `methods-auto` reference tables at
//! `learn.microsoft.com/.../dev-itpro/developer/methods-auto/<type>/<type>-data-type`
//! (which state the exact signature AND an explicit `"Available or changed with
//! runtime version …"` line) — cross-checked for member EXISTENCE against the
//! independently-generated [`crate::program::resolve::member_catalog`] phf sets
//! (`ms-dynamics-smb.al-18.0.2293710`). An entry is included ONLY when both agree
//! and the return type is unambiguous (no same-`(kind, member_lc, is_method,
//! arity)` overload with a different return kind — see [`MIN_SUPPORTED_RUNTIME`]
//! for the version-gating policy this implies).
//!
//! `is_method` encodes real AL syntax, not a caller choice: AL procedures ALWAYS
//! require parens (even zero-arg — `Response.Content()`, not `Response.Content`),
//! so a source site's parenthesization alone determines which table row applies;
//! there is no "property vs method" ambiguity to resolve at the call site.

use crate::program::resolve::receiver::FrameworkKind;

// ---------------------------------------------------------------------------
// Supported-runtime pin
// ---------------------------------------------------------------------------

/// The minimum AL/BC runtime version every table entry below is validated
/// against, as `(major, minor)`.
///
/// Every entry in [`framework_return_kind`] is documented by Microsoft Learn as
/// `"Available or changed with runtime version 1.0"` — i.e. present since the
/// AL runtime's inception, with no later-version gating or removal. Because the
/// table's validated floor (1.0) is satisfied by every BC runtime that can run
/// this engine's target workspaces (BC ships runtime 1.0+ universally; nothing
/// in the current table requires a HIGHER floor), there is no real workspace for
/// which an entry would need to be dynamically disabled today — so this module
/// does not thread a per-workspace runtime-version check into
/// [`framework_return_kind`]'s call sites (that would require plumbing the
/// parsed `app.json` `"runtime"` field through `infer_receiver_type`, a
/// meaningfully larger change with no entry currently exercising the disabled
/// path — untestable dead code). This constant is the documented POLICY pin: if
/// a FUTURE entry is added whose validated floor is higher than `(1, 0)` (e.g. a
/// runtime-24+-only type), that entry's addition MUST come with the dynamic
/// gate wired in at the same time, using this constant as the comparison
/// baseline — do not add a higher-floor entry without it.
pub const MIN_SUPPORTED_RUNTIME: (u32, u32) = (1, 0);

// ---------------------------------------------------------------------------
// framework_return_kind
// ---------------------------------------------------------------------------

/// Look up the [`FrameworkKind`] returned by accessing `member_lc` on a
/// `Framework(kind)` receiver, given its AL syntactic FORM (`is_method`: does
/// the source call it with parens?) and `arity` (arg count when `is_method`).
///
/// `None` — fail closed — for: an unlisted `(kind, member_lc)` pair (table
/// miss); the right member but the WRONG form (a property-form entry invoked as
/// a method-with-parens, or vice versa); or the right member/form but a
/// mismatched arity. Every listed entry is a single-hop, DETERMINISTIC AL
/// framework conversion — the return kind never varies by argument VALUE (only
/// by the argument's absence entirely, which is a different arity/overload and
/// thus a different table row or an intentional omission).
pub fn framework_return_kind(
    kind: &FrameworkKind,
    member_lc: &str,
    is_method: bool,
    arity: usize,
) -> Option<FrameworkKind> {
    use FrameworkKind::*;
    match (kind, member_lc, is_method, arity) {
        // ---------------------------------------------------------------
        // JSON conversions — all zero-arg METHODS (parens required in AL;
        // e.g. `JToken.AsObject()`, never `JToken.AsObject`).
        //
        // Provenance: methods-auto/jsontoken, /jsonobject, /jsonarray,
        // /jsonvalue (Microsoft Learn, fetched 2026-07-02) — each page states
        // "Available or changed with runtime version 1.0". Cross-checked
        // against `member_catalog.rs`'s JSONTOKEN/JSONOBJECT/JSONARRAY/
        // JSONVALUE phf sets (ms-dynamics-smb.al-18.0.2293710): all four
        // method names are present on their respective kind, confirming
        // real-world membership independent of this table.
        //
        // Deliberately NOT ported from L3 (`member_builtins.rs:408-413`),
        // which claims `AsValue`/`AsObject`/`AsArray` ALSO exist on
        // `JsonObject`/`JsonArray`/`JsonValue` — neither the MS Learn
        // methods-auto pages nor `member_catalog.rs`'s validated membership
        // sets list those methods on those three kinds (only `AsToken`
        // exists on them); that broader L3 claim is unvalidated/wrong and is
        // correctly omitted here (Rust-owned > al-sem parity).
        // ---------------------------------------------------------------
        (JsonToken, "asobject", true, 0) => Some(JsonObject),
        (JsonToken, "asarray", true, 0) => Some(JsonArray),
        (JsonToken, "asvalue", true, 0) => Some(JsonValue),
        (JsonObject, "astoken", true, 0) => Some(JsonToken),
        (JsonArray, "astoken", true, 0) => Some(JsonToken),
        (JsonValue, "astoken", true, 0) => Some(JsonToken),

        // ---------------------------------------------------------------
        // HTTP chain — all zero-arg METHODS (parens required; e.g.
        // `Response.Content()`, never `Response.Content`).
        //
        // Provenance: methods-auto/httpresponsemessage, /httprequestmessage,
        // /httpclient (Microsoft Learn, fetched 2026-07-02) — each page
        // states "Available or changed with runtime version 1.0" and lists
        // the zero-arg forms explicitly (`Content()`, `Headers()`,
        // `DefaultRequestHeaders()`). Cross-checked against
        // `member_catalog.rs`'s HTTPRESPONSE/HTTPREQUEST/HTTPCLIENT phf sets:
        // "content"/"headers"/"defaultrequestheaders" are all present.
        //
        // `HttpRequestMessage.Content` is OVERLOADED — `Content()` (arity 0,
        // getter, returns `HttpContent`) and `Content(HttpContent)` (arity 1,
        // setter, no chainable return) — per the brief's disambiguation rule
        // ("can't disambiguate without arg typing → not tabled" only applies
        // when the SAME arity has conflicting returns; here the two arities
        // are DISTINCT rows, and the arity-1 setter simply has no table entry
        // since it returns nothing to chain onto — not a conflict).
        //
        // `HttpRequestMessage` has NO zero-arg `Headers()` — only
        // `GetHeaders(var HttpHeaders)` (an out-param method, no chainable
        // return) — deliberately NOT tabled (would be a fabricated entry).
        // `HttpContent.GetHeaders(var HttpHeaders)` is the same out-param
        // shape — also not tabled for the same reason.
        // ---------------------------------------------------------------
        (HttpResponseMessage, "content", true, 0) => Some(HttpContent),
        (HttpResponseMessage, "headers", true, 0) => Some(HttpHeaders),
        (HttpRequestMessage, "content", true, 0) => Some(HttpContent),
        (HttpClient, "defaultrequestheaders", true, 0) => Some(HttpHeaders),

        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_token_conversions_resolve() {
        assert_eq!(
            framework_return_kind(&FrameworkKind::JsonToken, "asobject", true, 0),
            Some(FrameworkKind::JsonObject)
        );
        assert_eq!(
            framework_return_kind(&FrameworkKind::JsonToken, "asarray", true, 0),
            Some(FrameworkKind::JsonArray)
        );
        assert_eq!(
            framework_return_kind(&FrameworkKind::JsonToken, "asvalue", true, 0),
            Some(FrameworkKind::JsonValue)
        );
    }

    #[test]
    fn json_container_astoken_resolves() {
        assert_eq!(
            framework_return_kind(&FrameworkKind::JsonObject, "astoken", true, 0),
            Some(FrameworkKind::JsonToken)
        );
        assert_eq!(
            framework_return_kind(&FrameworkKind::JsonArray, "astoken", true, 0),
            Some(FrameworkKind::JsonToken)
        );
        assert_eq!(
            framework_return_kind(&FrameworkKind::JsonValue, "astoken", true, 0),
            Some(FrameworkKind::JsonToken)
        );
    }

    /// L3's table (WRONGLY, per the module doc) claims `JsonObject.AsObject()`
    /// exists — the fresh table must NOT ports that error.
    #[test]
    fn json_object_has_no_asobject_asarray_asvalue() {
        assert_eq!(
            framework_return_kind(&FrameworkKind::JsonObject, "asobject", true, 0),
            None
        );
        assert_eq!(
            framework_return_kind(&FrameworkKind::JsonObject, "asarray", true, 0),
            None
        );
        assert_eq!(
            framework_return_kind(&FrameworkKind::JsonValue, "asobject", true, 0),
            None
        );
    }

    #[test]
    fn http_response_content_and_headers_resolve() {
        assert_eq!(
            framework_return_kind(&FrameworkKind::HttpResponseMessage, "content", true, 0),
            Some(FrameworkKind::HttpContent)
        );
        assert_eq!(
            framework_return_kind(&FrameworkKind::HttpResponseMessage, "headers", true, 0),
            Some(FrameworkKind::HttpHeaders)
        );
    }

    #[test]
    fn http_request_content_resolves_but_setter_arity_does_not() {
        assert_eq!(
            framework_return_kind(&FrameworkKind::HttpRequestMessage, "content", true, 0),
            Some(FrameworkKind::HttpContent)
        );
        // The arity-1 SETTER form has no chainable return — must NOT resolve.
        assert_eq!(
            framework_return_kind(&FrameworkKind::HttpRequestMessage, "content", true, 1),
            None
        );
        // `HttpRequestMessage` has no zero-arg `Headers()` — only
        // `GetHeaders(var HttpHeaders)` — must NOT be fabricated.
        assert_eq!(
            framework_return_kind(&FrameworkKind::HttpRequestMessage, "headers", true, 0),
            None
        );
    }

    #[test]
    fn http_client_default_request_headers_resolves() {
        assert_eq!(
            framework_return_kind(&FrameworkKind::HttpClient, "defaultrequestheaders", true, 0),
            Some(FrameworkKind::HttpHeaders)
        );
    }

    /// Property-form (`is_method: false`) never matches — every table entry is
    /// a real AL zero-arg METHOD (parens required), never a no-parens property.
    #[test]
    fn property_form_never_matches_method_entries() {
        assert_eq!(
            framework_return_kind(&FrameworkKind::HttpResponseMessage, "content", false, 0),
            None
        );
        assert_eq!(
            framework_return_kind(&FrameworkKind::JsonToken, "asobject", false, 0),
            None
        );
    }

    #[test]
    fn wrong_arity_declines() {
        assert_eq!(
            framework_return_kind(&FrameworkKind::JsonToken, "asobject", true, 1),
            None
        );
    }

    #[test]
    fn unlisted_member_declines() {
        assert_eq!(
            framework_return_kind(&FrameworkKind::JsonObject, "notamember", true, 0),
            None
        );
    }

    #[test]
    fn unlisted_kind_declines() {
        let other = FrameworkKind::Other("sometype".to_string());
        assert_eq!(framework_return_kind(&other, "content", true, 0), None);
    }
}
