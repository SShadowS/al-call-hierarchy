//! Page-control extraction + CurrPage.<Part> resolution (Rust-owned).
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
