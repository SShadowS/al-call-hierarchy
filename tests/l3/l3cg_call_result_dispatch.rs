//! Single-hop call-result compound receiver resolution (`Func().Method()`) —
//! Feature C2 (Rust-owned).
//!
//! A member call ON the RESULT of a bare own-object procedure with a KNOWN return
//! type (`GetClient().Get(...)` where `GetClient(): HttpClient`) must type the
//! receiver as that return type and dispatch the method on it — here classifying
//! `GetClient().Get(...)` as the `HttpClient.Get` `builtin`.
//!
//! PRECISION GUARD: a call-result whose function returns a PRIMITIVE scalar
//! (`GetText(): Text`) must NEVER falsely resolve a bogus method on it — the
//! receiver stays an honest `unknown` (no false resolution that masks a real hole).
use al_call_hierarchy::engine::l3::l3_workspace::assemble_and_resolve_default;

const APP_GUID: &str = "3c000000-0000-0000-0000-0000000003cc";

/// `GetClient(): HttpClient` + `GetClient().Get('http://x')` — the call-result
/// receiver types as `HttpClient`, so `.Get` is the platform `HttpClient.Get`
/// `builtin` (not a `compound-receiver::call-result` unknown).
fn codeunit_call_result_http() -> &'static str {
    r#"codeunit 50100 "C2 Http" {
    procedure GetClient(): HttpClient begin end;
    procedure Foo() begin GetClient().Get('http://x'); end;
}
"#
}

/// `GetText(): Text` + `GetText().BogusMethodXyz()` — the return type is a
/// primitive scalar; the helper DECLINES, so the receiver stays compound/unknown
/// and `BogusMethodXyz` is NEVER falsely resolved.
fn codeunit_call_result_primitive() -> &'static str {
    r#"codeunit 50101 "C2 Prim" {
    procedure GetText(): Text begin end;
    procedure Foo() begin GetText().BogusMethodXyz(); end;
}
"#
}

#[test]
fn call_result_http_client_classifies_builtin() {
    let owned = vec![("u.al".to_string(), codeunit_call_result_http().to_string())];
    let resolved = assemble_and_resolve_default(&owned, APP_GUID);

    let proj = resolved.project_call_graph();
    let edges: Vec<_> = proj.groups.iter().flat_map(|g| g.edges.iter()).collect();

    // `GetClient().Get('http://x')` must classify `builtin` (HttpClient.Get) on the
    // MEMBER-dispatch edge. (The inner bare `GetClient()` is a separate
    // `direct`/`resolved` edge.)
    let builtin_member: Vec<_> = edges
        .iter()
        .filter(|e| e.resolution == "builtin" && e.dispatch_kind == "builtin")
        .collect();
    assert!(
        !builtin_member.is_empty(),
        "GetClient().Get('http://x') must classify as a builtin member edge (HttpClient.Get); all edges: {:#?}",
        edges
    );

    // And NO edge from this fixture remains `unknown` — the only member callsite
    // (`GetClient().Get(...)`) is now the HttpClient.Get builtin, so nothing is left
    // as the pre-C2 `compound-receiver::call-result` unknown.
    assert!(
        !edges.iter().any(|e| e.resolution == "unknown"),
        "GetClient().Get(...) must NOT remain an unknown edge; all edges: {:#?}",
        edges
    );
}

#[test]
fn call_result_primitive_does_not_falsely_resolve() {
    let owned = vec![(
        "u.al".to_string(),
        codeunit_call_result_primitive().to_string(),
    )];
    let resolved = assemble_and_resolve_default(&owned, APP_GUID);

    let proj = resolved.project_call_graph();
    let edges: Vec<_> = proj.groups.iter().flat_map(|g| g.edges.iter()).collect();

    // The MEMBER edge for `GetText().BogusMethodXyz()` (dispatch_kind "method")
    // must NOT be `resolved`/`builtin` — a primitive (Text) return is DECLINED, so
    // there is no false resolution. (The inner BARE call `GetText()` is correctly a
    // separate `direct`/`resolved` edge — that is the real call to GetText and is
    // NOT what this guards.) `BogusMethodXyz` is not a Text builtin and not a real
    // procedure, so the method edge must stay an honest `unknown` call-result hole.
    let member_edges: Vec<_> = edges
        .iter()
        .filter(|e| e.dispatch_kind == "method")
        .collect();
    assert!(
        !member_edges.is_empty(),
        "expected a member-dispatch edge for GetText().BogusMethodXyz(); all edges: {:#?}",
        edges
    );
    assert!(
        member_edges
            .iter()
            .all(|e| e.resolution != "resolved" && e.resolution != "builtin"),
        "GetText().BogusMethodXyz() member call must NOT falsely resolve/builtin (primitive return declined); all edges: {:#?}",
        edges
    );
    assert!(
        member_edges.iter().all(|e| e.resolution == "unknown"),
        "GetText().BogusMethodXyz() member call must stay an unknown edge (primitive return declined); all edges: {:#?}",
        edges
    );
}
