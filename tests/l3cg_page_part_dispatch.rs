//! Page-control extraction + CurrPage.<Part> resolution (Rust-owned).
use al_call_hierarchy::engine::deps::symbol_reference::parse_symbol_reference;
use al_call_hierarchy::engine::l3::l3_workspace::{assemble_workspace_units, PageControlKind};

fn page_with_part() -> &'static str {
    r#"page 50100 "My List Part" { SourceTable = "Item"; layout { area(Content) { } } }
page 50101 "My Card" {
    SourceTable = "Item";
    layout { area(Content) { part(Lines; "My List Part") { } } }
    procedure Foo() begin CurrPage.Lines.Page.Bar(); end;
}
page 50102 "X" { procedure Bar() begin end; }
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
