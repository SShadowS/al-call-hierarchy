//! Single-hop framework-property compound receivers (Feature C1).
//!
//! `HttpClient.DefaultRequestHeaders.Add('k','v')` — the base `Client` is an
//! `HttpClient` framework receiver, `DefaultRequestHeaders` is a framework-returning
//! property (→ `HttpHeaders`), and `.Add(...)` is an `HttpHeaders` builtin. The
//! compound receiver must therefore classify the callsite as a `builtin` edge, not a
//! `CompoundReceiver` unknown.
use al_call_hierarchy::engine::l3::l3_workspace::assemble_and_resolve_default;

fn codeunit_with_http_compound() -> &'static str {
    r#"codeunit 50120 "Http Caller"
{
    var Client: HttpClient;

    procedure Foo()
    begin
        Client.DefaultRequestHeaders.Add('k', 'v');
    end;
}
"#
}

/// `Client.DefaultRequestHeaders.Add('k','v')` where `Client : HttpClient` must
/// classify as a `builtin` edge (HttpHeaders.Add), not a `CompoundReceiver` unknown.
#[test]
fn http_default_request_headers_add_classifies_builtin() {
    const APP_GUID: &str = "3c000000-0000-0000-0000-0000000003cc";
    let owned = vec![(
        "u.al".to_string(),
        codeunit_with_http_compound().to_string(),
    )];
    let resolved = assemble_and_resolve_default(&owned, APP_GUID);

    let proj = resolved.project_call_graph();
    let edges: Vec<_> = proj.groups.iter().flat_map(|g| g.edges.iter()).collect();

    // The `Foo` routine's only member call is `Client.DefaultRequestHeaders.Add(...)`.
    let builtin_edges: Vec<_> = edges.iter().filter(|e| e.resolution == "builtin").collect();

    assert!(
        !builtin_edges.is_empty(),
        "Client.DefaultRequestHeaders.Add('k','v') must classify as a builtin edge; all edges: {:#?}",
        edges
    );

    // And NO edge from this fixture remains a CompoundReceiver `unknown`.
    assert!(
        !edges.iter().any(|e| e.resolution == "unknown"),
        "no edge should remain unknown; all edges: {:#?}",
        edges
    );
}

/// Method-form framework chain: `JTok.AsValue().AsInteger()` — `JTok : JsonToken`,
/// `AsValue()` returns a `JsonValue` (deterministic framework conversion), and
/// `.AsInteger()` is a `JsonValue` builtin. Must classify `builtin`, not
/// CompoundReceiver unknown.
fn codeunit_with_json_chain() -> &'static str {
    r#"codeunit 50121 "Json Caller"
{
    var JTok: JsonToken;

    procedure Foo()
    begin
        JTok.AsValue().AsInteger();
    end;
}
"#
}

#[test]
fn json_astoken_chain_classifies_builtin() {
    const APP_GUID: &str = "3c000000-0000-0000-0000-0000000003cc";
    let owned = vec![("u.al".to_string(), codeunit_with_json_chain().to_string())];
    let resolved = assemble_and_resolve_default(&owned, APP_GUID);
    let proj = resolved.project_call_graph();
    let edges: Vec<_> = proj.groups.iter().flat_map(|g| g.edges.iter()).collect();
    assert!(
        edges.iter().any(|e| e.resolution == "builtin"),
        "JTok.AsValue().AsInteger() must classify builtin; edges: {:#?}",
        edges
    );
    assert!(
        !edges.iter().any(|e| e.resolution == "unknown"),
        "no edge should remain unknown; edges: {:#?}",
        edges
    );
}
