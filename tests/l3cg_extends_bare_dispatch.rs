//! Phase-3 fixture tests: a bare call inside a PageExtension / TableExtension
//! / ReportExtension resolves to the procedure defined on the EXTENDED base
//! object when own-object lookup fails.
//!
//! AL extensions may bare-call procedures defined on their base object — the
//! compiler injects the base-object scope for them. Before this change the
//! bare-call resolver only looked in the caller's own object, so these calls
//! fell through to `Unknown{BareUnresolved}`. The fix adds a second lookup
//! against the extends-target base object.
//!
//! Positive tests:
//!   1. PageExtension bare-calling a base Page procedure → `resolved`.
//!   2. TableExtension bare-calling a base Table procedure → `resolved`.
//!
//! Negative / regression tests:
//!   3. A bare call to a name in NEITHER the ext NOR the base → stays `unknown`.
//!   4. A PageExt calling its OWN procedure still resolves own-first (regression
//!      guard; must not require the base-object fallback to work).

use al_call_hierarchy::engine::l3::call_graph_projection::{L3CallGraphProjection, PCallEdge};
use al_call_hierarchy::engine::l3::l3_workspace::assemble_and_resolve_default;

const APP_GUID: &str = "2b000000-0000-0000-0000-0000000002cc";

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

/// Test 1: PageExtension bare-calls a procedure defined on the base Page.
///
/// The `CallPostDocument()` bare call inside the PageExtension trigger must
/// resolve to the base page's `CallPostDocument` procedure — not stay unknown.
#[test]
fn pageext_bare_call_resolves_to_base_page_proc() {
    let base_page = r#"page 50000 BaseP {
    SourceTable = "Item";
    procedure CallPostDocument()
    begin
    end
}"#;
    let ext = r#"pageextension 50001 Ext extends BaseP
{
    trigger OnOpenPage()
    begin
        CallPostDocument();
    end
}"#;
    let p = project_ws(&[("src/base.al", base_page), ("src/ext.al", ext)]);
    let edges = all_edges(&p);

    // The bare call to CallPostDocument() must resolve.
    let resolved: Vec<&&PCallEdge> = edges
        .iter()
        .filter(|e| e.resolution == "resolved" && e.dispatch_kind == "direct")
        .collect();
    assert!(
        !resolved.is_empty(),
        "CallPostDocument() bare-called from PageExtension must resolve to base page proc; got edges: {:#?}",
        edges
    );
    assert!(
        resolved.iter().all(|e| e.to.is_some()),
        "resolved edge must have a non-None `to`; edges: {:#?}",
        resolved
    );

    // No bare-unresolved edges at all.
    let unknown: Vec<&&PCallEdge> = edges.iter().filter(|e| e.resolution == "unknown").collect();
    assert!(
        unknown.is_empty(),
        "no unknown edges should remain; unknowns: {:#?}",
        unknown
    );
}

/// Test 2: TableExtension bare-calls a procedure defined on the base Table.
#[test]
fn tableext_bare_call_resolves_to_base_table_proc() {
    let base_table = r#"table 50000 BaseT {
    fields { field(1; "No."; Code[20]) { } }
    keys { key(PK; "No.") { } }
    procedure BaseProc()
    begin
    end
}"#;
    let ext = r#"tableextension 50001 TExt extends BaseT
{
    trigger OnInsert()
    begin
        BaseProc();
    end
}"#;
    let p = project_ws(&[("src/base.al", base_table), ("src/ext.al", ext)]);
    let edges = all_edges(&p);

    let resolved: Vec<&&PCallEdge> = edges
        .iter()
        .filter(|e| e.resolution == "resolved" && e.dispatch_kind == "direct")
        .collect();
    assert!(
        !resolved.is_empty(),
        "BaseProc() bare-called from TableExtension must resolve to base table proc; got edges: {:#?}",
        edges
    );
    assert!(
        resolved.iter().all(|e| e.to.is_some()),
        "resolved edge must have a non-None `to`"
    );

    let unknown: Vec<&&PCallEdge> = edges.iter().filter(|e| e.resolution == "unknown").collect();
    assert!(
        unknown.is_empty(),
        "no unknown edges should remain; unknowns: {:#?}",
        unknown
    );
}

/// Test 3: A bare call to a name in NEITHER the extension NOR the base stays unknown.
#[test]
fn pageext_bare_unknown_stays_unknown() {
    let base_page = r#"page 50000 BaseP {
    SourceTable = "Item";
    procedure KnownProc()
    begin
    end
}"#;
    let ext = r#"pageextension 50001 Ext extends BaseP
{
    trigger OnOpenPage()
    begin
        TotallyMissingProc();
    end
}"#;
    let p = project_ws(&[("src/base.al", base_page), ("src/ext.al", ext)]);
    let edges = all_edges(&p);

    let unknown: Vec<&&PCallEdge> = edges.iter().filter(|e| e.resolution == "unknown").collect();
    assert_eq!(
        unknown.len(),
        1,
        "a bare call to a name not in ext OR base must stay unknown; edges: {:#?}",
        edges
    );

    // No spurious resolution.
    let resolved: Vec<&&PCallEdge> = edges
        .iter()
        .filter(|e| e.resolution == "resolved")
        .collect();
    assert!(
        resolved.is_empty(),
        "must not spuriously resolve a missing procedure; resolved: {:#?}",
        resolved
    );
}

/// Test 4: A PageExtension calling its OWN procedure resolves via own-object lookup
/// (regression guard — the base-object fallback must not interfere).
#[test]
fn own_object_bare_still_resolves() {
    let base_page = r#"page 50000 BaseP {
    SourceTable = "Item";
}"#;
    let ext = r#"pageextension 50001 Ext extends BaseP
{
    procedure OwnProc()
    begin
    end

    trigger OnOpenPage()
    begin
        OwnProc();
    end
}"#;
    let p = project_ws(&[("src/base.al", base_page), ("src/ext.al", ext)]);
    let edges = all_edges(&p);

    let resolved: Vec<&&PCallEdge> = edges
        .iter()
        .filter(|e| e.resolution == "resolved" && e.dispatch_kind == "direct")
        .collect();
    assert!(
        !resolved.is_empty(),
        "OwnProc() bare-called from the same PageExtension must resolve via own-object; got edges: {:#?}",
        edges
    );
    assert!(
        resolved.iter().all(|e| e.to.is_some()),
        "resolved edge must have a non-None `to`"
    );

    let unknown: Vec<&&PCallEdge> = edges.iter().filter(|e| e.resolution == "unknown").collect();
    assert!(
        unknown.is_empty(),
        "no unknown edges should remain; unknowns: {:#?}",
        unknown
    );
}
