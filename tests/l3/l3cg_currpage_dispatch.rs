//! Phase-2 fixture tests: CurrPage / CurrReport singletons are reclassified from
//! `unknown { UntrackedReceiver }` to `builtin` via the PageInstance /
//! ReportInstance member catalog.
//!
//! These are language singletons (not declared variables) that refer to the
//! current page / report instance inside their triggers. They must dispatch
//! through `dispatch_framework` exactly like other framework receivers — catalog
//! hit ⇒ `builtin`, catalog miss ⇒ `Unknown { FrameworkMethodNotInCatalog }`.

use al_call_hierarchy::engine::l3::call_graph_projection::{L3CallGraphProjection, PCallEdge};
use al_call_hierarchy::engine::l3::l3_workspace::assemble_and_resolve_default;

const APP_GUID: &str = "3c000000-0000-0000-0000-0000000003cc";

fn project_ws(files: &[(&str, &str)]) -> L3CallGraphProjection {
    let owned: Vec<(String, String)> = files
        .iter()
        .map(|(n, s)| (n.to_string(), s.to_string()))
        .collect();
    assemble_and_resolve_default(&owned, APP_GUID).project_call_graph()
}

fn all_edges(p: &L3CallGraphProjection) -> Vec<&PCallEdge> {
    p.groups.iter().flat_map(|g| g.edges.iter()).collect()
}

// A minimal page with a trigger that calls CurrPage.Update(false).
const PAGE_UPDATE_SRC: &str = r#"
table 50000 TestTab {
    fields { field(1; Fld; Integer) { } }
    keys { key(PK; Fld) { } }
}
page 50000 TestPage {
    SourceTable = TestTab;
    layout {
        area(content) {
            group(g) {
                field(Fld; Rec.Fld) { }
            }
        }
    }
    trigger OnAfterGetRecord()
    begin
        CurrPage.Update(false);
    end;
}
"#;

// A minimal page with a trigger that calls CurrPage.SetSelectionFilter(Rec).
const PAGE_SSF_SRC: &str = r#"
table 50001 TestTab2 {
    fields { field(1; Fld; Integer) { } }
    keys { key(PK; Fld) { } }
}
page 50001 TestPage2 {
    SourceTable = TestTab2;
    layout {
        area(content) {
            group(g) {
                field(Fld; Rec.Fld) { }
            }
        }
    }
    trigger OnAfterGetRecord()
    var
        FilterRec: Record TestTab2;
    begin
        CurrPage.SetSelectionFilter(FilterRec);
    end;
}
"#;

// A minimal report with a trigger that calls CurrReport.Skip().
const REPORT_SKIP_SRC: &str = r#"
table 50002 TestTab3 {
    fields { field(1; Fld; Integer) { } }
    keys { key(PK; Fld) { } }
}
report 50000 TestReport {
    dataset {
        dataitem(TestTab3; TestTab3) {
            trigger OnAfterGetRecord()
            begin
                CurrReport.Skip();
            end;
        }
    }
}
"#;

// A page calling a method that is NOT in the PageInstance catalog — stays unknown.
const PAGE_UNKNOWN_METHOD_SRC: &str = r#"
table 50003 TestTab4 {
    fields { field(1; Fld; Integer) { } }
    keys { key(PK; Fld) { } }
}
page 50002 TestPage3 {
    SourceTable = TestTab4;
    layout {
        area(content) {
            group(g) {
                field(Fld; Rec.Fld) { }
            }
        }
    }
    trigger OnAfterGetRecord()
    begin
        CurrPage.NoSuchZzz();
    end;
}
"#;

#[test]
fn currpage_builtin_method_is_builtin() {
    let p = project_ws(&[("src/page_update.al", PAGE_UPDATE_SRC)]);
    let edges = all_edges(&p);
    // CurrPage.Update must produce a builtin edge.
    let builtin_edges: Vec<_> = edges
        .iter()
        .filter(|e| e.resolution == "builtin" && e.dispatch_kind == "builtin")
        .collect();
    assert!(
        !builtin_edges.is_empty(),
        "CurrPage.Update() must produce a builtin edge; all edges: {:#?}",
        edges
    );
    // None of the builtin edges should have a `to` target.
    for e in &builtin_edges {
        assert!(
            e.to.is_none(),
            "builtin edges must not have a `to` target; edge: {:#?}",
            e
        );
    }
    // No unknown edges for the CurrPage call.
    let unknown_edges: Vec<_> = edges.iter().filter(|e| e.resolution == "unknown").collect();
    assert!(
        unknown_edges.is_empty(),
        "CurrPage.Update() must not produce unknown edges; unknowns: {:#?}",
        unknown_edges
    );
}

#[test]
fn currpage_setselectionfilter_is_builtin() {
    let p = project_ws(&[("src/page_ssf.al", PAGE_SSF_SRC)]);
    let edges = all_edges(&p);
    let builtin_edges: Vec<_> = edges
        .iter()
        .filter(|e| e.resolution == "builtin" && e.dispatch_kind == "builtin")
        .collect();
    assert!(
        !builtin_edges.is_empty(),
        "CurrPage.SetSelectionFilter() must produce a builtin edge; all edges: {:#?}",
        edges
    );
    let unknown_edges: Vec<_> = edges.iter().filter(|e| e.resolution == "unknown").collect();
    assert!(
        unknown_edges.is_empty(),
        "CurrPage.SetSelectionFilter() must not produce unknown edges; unknowns: {:#?}",
        unknown_edges
    );
}

#[test]
fn currreport_method_is_builtin() {
    let p = project_ws(&[("src/report_skip.al", REPORT_SKIP_SRC)]);
    let edges = all_edges(&p);
    let builtin_edges: Vec<_> = edges
        .iter()
        .filter(|e| e.resolution == "builtin" && e.dispatch_kind == "builtin")
        .collect();
    assert!(
        !builtin_edges.is_empty(),
        "CurrReport.Skip() must produce a builtin edge; all edges: {:#?}",
        edges
    );
    let unknown_edges: Vec<_> = edges.iter().filter(|e| e.resolution == "unknown").collect();
    assert!(
        unknown_edges.is_empty(),
        "CurrReport.Skip() must not produce unknown edges; unknowns: {:#?}",
        unknown_edges
    );
}

#[test]
fn currpage_unknown_method_stays_unknown() {
    let p = project_ws(&[("src/page_unknown.al", PAGE_UNKNOWN_METHOD_SRC)]);
    let edges = all_edges(&p);
    // The method is not in the PageInstance catalog — must stay unknown.
    let unknown_edges: Vec<_> = edges.iter().filter(|e| e.resolution == "unknown").collect();
    assert!(
        !unknown_edges.is_empty(),
        "CurrPage.NoSuchZzz() (not in catalog) must stay unknown; all edges: {:#?}",
        edges
    );
    // Must NOT produce a spurious builtin.
    let builtin_edges: Vec<_> = edges.iter().filter(|e| e.resolution == "builtin").collect();
    assert!(
        builtin_edges.is_empty(),
        "CurrPage.NoSuchZzz() must not be a false builtin; edges: {:#?}",
        builtin_edges
    );
}
