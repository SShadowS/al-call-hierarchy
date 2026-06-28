//! Page-control extraction + CurrPage.<Part> resolution (Rust-owned).
use al_call_hierarchy::engine::deps::symbol_reference::parse_symbol_reference;
use al_call_hierarchy::engine::l3::l3_workspace::{
    PageControlKind, assemble_and_resolve_default, assemble_workspace_units,
};
use al_call_hierarchy::engine::l3::symbol_table::SymbolTable;

fn page_with_part() -> &'static str {
    // The part "Lines" sources page 50100 "My List Part", which carries the `Bar`
    // procedure that `Foo`'s `CurrPage.Lines.Page.Bar()` callsite must resolve to.
    r#"page 50100 "My List Part" { SourceTable = "Item"; layout { area(Content) { } } procedure Bar() begin end; }
page 50101 "My Card" {
    SourceTable = "Item";
    layout { area(Content) { part(Lines; "My List Part") { } } }
    procedure Foo() begin CurrPage.Lines.Page.Bar(); end;
}
"#
}

// Same fixture but the member call uses NON-canonical casing
// (`CURRPAGE.Lines.PAGE.Bar()`). AL is case-insensitive, so this must resolve
// identically — guards the case-insensitive prefix/suffix matching in
// `currpage_control_receiver`.
fn page_with_part_mixed_case() -> &'static str {
    r#"page 50100 "My List Part" { SourceTable = "Item"; layout { area(Content) { } } procedure Bar() begin end; }
page 50101 "My Card" {
    SourceTable = "Item";
    layout { area(Content) { part(Lines; "My List Part") { } } }
    procedure Foo() begin CURRPAGE.Lines.PAGE.Bar(); end;
}
"#
}

/// Hand-written SymbolReference.json with one Page containing a Kind-6 (Part) control
/// and a Kind-10 (UserControl) control. Verifies that `parse_symbol_reference` populates
/// `AbiObject.page_controls` from dep `.app` symbol data.
#[test]
fn dep_page_controls_extracted_from_symbol_reference() {
    let json = r#"{
        "AppId": "test-guid",
        "Name": "TestApp",
        "Publisher": "Test",
        "Version": "1.0.0.0",
        "Pages": [
            {
                "Id": 50100,
                "Name": "My Card Page",
                "Controls": [
                    {
                        "Kind": 6,
                        "Name": "Data Migration Status",
                        "RelatedPagePartId": { "Name": "", "Id": 1795 },
                        "Properties": []
                    },
                    {
                        "Kind": 10,
                        "Name": "MyAddin",
                        "RelatedControlAddIn": "Microsoft.Dynamics.Nav.Client.MyAddin",
                        "Properties": []
                    },
                    {
                        "Kind": 1,
                        "Name": "SomeField",
                        "Controls": [
                            {
                                "Kind": 6,
                                "Name": "NestedPart",
                                "RelatedPagePartId": { "Name": "", "Id": 9999 },
                                "Properties": []
                            }
                        ]
                    }
                ],
                "Methods": [],
                "Properties": []
            }
        ]
    }"#;

    let abi = parse_symbol_reference(json);
    assert!(abi.error.is_none(), "parse error: {:?}", abi.error);

    let page = abi
        .objects
        .iter()
        .find(|o| o.name == "My Card Page")
        .expect("page not found in ABI objects");

    // Should have exactly 3 controls: the top-level Kind-6, Kind-10, and the nested Kind-6
    assert_eq!(
        page.page_controls.len(),
        3,
        "expected 3 page_controls, got: {:?}",
        page.page_controls
    );

    // Kind-6 Part: target is the page NUMBER as a string
    let part = page
        .page_controls
        .iter()
        .find(|(name, _, _)| name == "Data Migration Status")
        .expect("Data Migration Status part not found");
    assert_eq!(part.1, "part");
    assert_eq!(part.2, "1795");
    // target parses as integer (the Page object number)
    assert!(
        part.2.parse::<i64>().is_ok(),
        "part target should be parseable as integer: {}",
        part.2
    );

    // Kind-10 UserControl: target is the control add-in name
    let uc = page
        .page_controls
        .iter()
        .find(|(name, _, _)| name == "MyAddin")
        .expect("MyAddin usercontrol not found");
    assert_eq!(uc.1, "usercontrol");
    assert_eq!(uc.2, "Microsoft.Dynamics.Nav.Client.MyAddin");

    // Nested Kind-6 Part: recursion works
    let nested = page
        .page_controls
        .iter()
        .find(|(name, _, _)| name == "NestedPart")
        .expect("NestedPart not found");
    assert_eq!(nested.1, "part");
    assert_eq!(nested.2, "9999");
}

#[test]
fn native_part_control_extracted() {
    let ws = assemble_workspace_units(
        &[("u".to_string(), page_with_part().to_string())],
        "app",
        "mi",
    );
    let card = ws.objects.iter().find(|o| o.name == "My Card").unwrap();
    let lines = card
        .page_controls
        .iter()
        .find(|c| c.name == "Lines")
        .unwrap();
    assert_eq!(lines.kind, PageControlKind::Part);
    assert_eq!(lines.target, "My List Part");
}

/// `page_controls_for` on a PageExtension returns its OWN controls PLUS the base
/// page's controls.
#[test]
fn symbol_table_page_controls_for_merges_base_page() {
    let src = r#"
page 50200 "My Card" {
    SourceTable = "Item";
    layout { area(Content) { part(BasePart; "My List Part") { } } }
}
pageextension 50201 "My Card Ext" extends "My Card"
{
    layout { addlast(Content) { part(Extra; "My List Part") { } } }
}
"#;
    let ws = assemble_workspace_units(&[("u".to_string(), src.to_string())], "app", "mi");
    let symbols = SymbolTable::build(&ws.objects, &ws.tables, &ws.routines);

    // Find the PageExtension's object id.
    let ext = ws
        .objects
        .iter()
        .find(|o| o.name == "My Card Ext")
        .expect("pageextension not found");

    let controls = symbols.page_controls_for(&ext.id);

    // Should contain the extension's own "Extra" AND the base page's "BasePart".
    let names: Vec<&str> = controls.iter().map(|c| c.name.as_str()).collect();
    assert!(
        names.contains(&"Extra"),
        "expected 'Extra' in controls, got: {:?}",
        names
    );
    assert!(
        names.contains(&"BasePart"),
        "expected 'BasePart' from base page in controls, got: {:?}",
        names
    );
    assert_eq!(
        controls.len(),
        2,
        "expected exactly 2 controls (1 own + 1 base), got: {:?}",
        names
    );
}

/// `CurrPage.Lines.Page.Bar()` inside page 50101 "My Card" must RESOLVE to the
/// `Bar` procedure on page 50100 "My List Part" (the part's source page), as a
/// `Resolved` edge — not a `CompoundReceiver` unknown.
#[test]
fn currpage_part_page_member_resolves_to_subpage_procedure() {
    const APP_GUID: &str = "3c000000-0000-0000-0000-0000000003cc";
    let owned = vec![("u.al".to_string(), page_with_part().to_string())];
    let resolved = assemble_and_resolve_default(&owned, APP_GUID);

    // Bar's internal routine id (object 50100 "My List Part").
    let bar = resolved
        .workspace
        .routines
        .iter()
        .find(|r| r.name == "Bar")
        .expect("Bar procedure not found in workspace routines");
    let bar_stable = bar.stable_routine_id.clone();

    let proj = resolved.project_call_graph();
    let edges: Vec<_> = proj.groups.iter().flat_map(|g| g.edges.iter()).collect();

    // Find the edge originating from Foo's CurrPage.Lines.Page.Bar() callsite.
    let resolved_to_bar: Vec<_> = edges
        .iter()
        .filter(|e| e.resolution == "resolved" && e.to.as_deref() == Some(bar_stable.as_str()))
        .collect();

    assert!(
        !resolved_to_bar.is_empty(),
        "CurrPage.Lines.Page.Bar() must resolve to Bar (stable id {}); all edges: {:#?}",
        bar_stable,
        edges
    );
}

#[test]
fn currpage_part_resolution_is_case_insensitive() {
    const APP_GUID: &str = "3c000000-0000-0000-0000-0000000003cc";
    let owned = vec![("u.al".to_string(), page_with_part_mixed_case().to_string())];
    let resolved = assemble_and_resolve_default(&owned, APP_GUID);

    let bar_stable = resolved
        .workspace
        .routines
        .iter()
        .find(|r| r.name == "Bar")
        .expect("Bar procedure not found")
        .stable_routine_id
        .clone();

    let proj = resolved.project_call_graph();
    let resolved_to_bar = proj
        .groups
        .iter()
        .flat_map(|g| g.edges.iter())
        .any(|e| e.resolution == "resolved" && e.to.as_deref() == Some(bar_stable.as_str()));

    assert!(
        resolved_to_bar,
        "CURRPAGE.Lines.PAGE.Bar() (non-canonical casing) must resolve to Bar — AL is case-insensitive"
    );
}

fn page_with_usercontrol() -> &'static str {
    // The page carries a `usercontrol(Body; "Some AddIn")` control-add-in and a
    // procedure calling `CurrPage.Body.SetContent('x')`. A control-add-in method is
    // a platform/JS call with no in-AL target, so the edge must classify `builtin`.
    r#"page 50110 "Addin Card" {
    SourceTable = "Item";
    layout { area(Content) { usercontrol(Body; "Some AddIn") { } } }
    procedure Drive() begin CurrPage.Body.SetContent('x'); end;
}
"#
}

/// `CurrPage.Body.SetContent('x')` where `Body` is a `usercontrol` (control add-in)
/// must classify as a `builtin` edge — a control-add-in method is a platform/JS call
/// with no in-AL target, not a resolution failure (`unknown`) and not `dynamic`.
#[test]
fn currpage_usercontrol_member_classifies_builtin() {
    const APP_GUID: &str = "3c000000-0000-0000-0000-0000000003cc";
    let owned = vec![("u.al".to_string(), page_with_usercontrol().to_string())];
    let resolved = assemble_and_resolve_default(&owned, APP_GUID);

    let proj = resolved.project_call_graph();
    let edges: Vec<_> = proj.groups.iter().flat_map(|g| g.edges.iter()).collect();

    // The `Drive` routine's only member call is `CurrPage.Body.SetContent(...)`.
    // There is exactly one builtin edge and it must carry that classification.
    let builtin_edges: Vec<_> = edges.iter().filter(|e| e.resolution == "builtin").collect();

    assert!(
        !builtin_edges.is_empty(),
        "CurrPage.Body.SetContent('x') must classify as a builtin edge; all edges: {:#?}",
        edges
    );

    // And NO edge from this fixture is a CompoundReceiver `unknown` (the pre-Task-7
    // behavior) — every edge resolved to something concrete.
    assert!(
        !edges.iter().any(|e| e.resolution == "unknown"),
        "no edge should remain unknown; all edges: {:#?}",
        edges
    );
}
