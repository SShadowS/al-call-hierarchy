//! Phase-3 fixture tests: implicit `Rec`/`xRec` receiver resolution (Task 6a).
//!
//! For Table/Page/TableExtension/PageExtension objects, the implicit `Rec` and
//! `xRec` receivers are registered in `routine.record_variables` (with `table_id`
//! set by `record_types::resolve_routine_record_types` pass 3) but NOT in
//! `routine.variables`. Before Task 6a, Step 2 of `infer_receiver_type` failed
//! with `UntrackedReceiver` for these receivers, preventing table-procedure dispatch.
//!
//! Task 6a (Step 2b) adds a check in `infer_receiver_type` that, BEFORE yielding
//! `UntrackedReceiver`, looks up the receiver name in `record_variables`. When the
//! entry has a resolved `table_id`, the table object id is derived and a
//! `ReceiverType::Record { table_object_id: Some(..) }` is returned so Phase B can
//! dispatch both catalog builtins and real table procedures.
//!
//! Negative control: a codeunit with an undeclared `Rec` variable (no effective own
//! table → `table_id` is None) stays `Unknown { UntrackedReceiver }`.

use al_call_hierarchy::engine::l3::call_graph_projection::{L3CallGraphProjection, PCallEdge};
use al_call_hierarchy::engine::l3::l3_workspace::assemble_and_resolve_default;

const APP_GUID: &str = "3a000000-0000-0000-0000-0000000003dd";

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

// ---------------------------------------------------------------------------
// Test 1: Rec.Helper() in a table trigger resolves to the table procedure.
// ---------------------------------------------------------------------------

/// An `OnInsert` trigger calling `Rec.Helper()` where `Helper` is a procedure
/// on the same table. With Task 6a, `Rec` in `record_variables` has
/// `table_id == Some(own_table_id)` so Phase A resolves it to
/// `ReceiverType::Record { table_object_id: Some(..) }` and Phase B dispatches
/// `Helper` as a resolved table procedure.
#[test]
fn implicit_rec_table_procedure_resolves() {
    let tbl = r#"table 50010 Item {
    fields { field(1; "No."; Code[20]) { } }
    keys { key(PK; "No.") { } }
    trigger OnInsert()
    begin
        Rec.Helper();
    end
    procedure Helper()
    begin
    end
}"#;
    let p = project_ws(&[("src/item.al", tbl)]);
    let edges = all_edges(&p);

    let resolved_method: Vec<&&PCallEdge> = edges
        .iter()
        .filter(|e| e.resolution == "resolved" && e.dispatch_kind == "method")
        .collect();

    assert!(
        !resolved_method.is_empty(),
        "Rec.Helper() in OnInsert must resolve to 'resolved'/'method'; got edges: {:#?}",
        edges
    );
    assert!(
        resolved_method.iter().all(|e| e.to.is_some()),
        "resolved edge must have a non-None `to`; edges: {:#?}",
        resolved_method
    );

    // No unknown method edges remain.
    let unknown_method_edges: Vec<&&PCallEdge> = edges
        .iter()
        .filter(|e| e.resolution == "unknown" && e.dispatch_kind == "method")
        .collect();
    assert!(
        unknown_method_edges.is_empty(),
        "no method edge should stay unknown after Task 6a; unknowns: {:#?}",
        unknown_method_edges
    );
}

// ---------------------------------------------------------------------------
// Test 2: Rec.TableCaption() stays builtin; Rec.Helper() resolves.
// ---------------------------------------------------------------------------

/// In a table trigger, `Rec.TableCaption()` is a Record catalog builtin and must
/// emit `builtin`. `Rec.Helper()` is a real table procedure and must emit
/// `resolved`. Both can coexist in the same trigger.
///
/// Note: Record DML operations (`Modify`, `Insert`, `FindFirst`, …) are captured
/// at L2 as `PRecordOperation`, NOT as `PCallSite`, so they never reach the call
/// resolver and cannot be used here. `TableCaption` and `FieldNo` ARE `PCallSite`s
/// and are in the catalog.
#[test]
fn implicit_rec_builtin_stays_builtin() {
    let tbl = r#"table 50011 Order {
    fields { field(1; "No."; Code[20]) { } }
    keys { key(PK; "No.") { } }
    trigger OnModify()
    begin
        Rec.TableCaption();
        Rec.Helper();
    end
    procedure Helper()
    begin
    end
}"#;
    let p = project_ws(&[("src/order.al", tbl)]);
    let edges = all_edges(&p);

    // TableCaption is a Record builtin — must be `builtin`.
    let builtin_edges: Vec<&&PCallEdge> =
        edges.iter().filter(|e| e.resolution == "builtin").collect();
    assert!(
        !builtin_edges.is_empty(),
        "Rec.TableCaption() must be 'builtin'; got edges: {:#?}",
        edges
    );

    // Helper is a real table procedure — must be `resolved`.
    let resolved_method: Vec<&&PCallEdge> = edges
        .iter()
        .filter(|e| e.resolution == "resolved" && e.dispatch_kind == "method")
        .collect();
    assert!(
        !resolved_method.is_empty(),
        "Rec.Helper() must resolve to 'resolved'/'method'; got edges: {:#?}",
        edges
    );

    // No unknown edges remain.
    let unknown_edges: Vec<&&PCallEdge> =
        edges.iter().filter(|e| e.resolution == "unknown").collect();
    assert!(
        unknown_edges.is_empty(),
        "no edge should stay unknown; unknowns: {:#?}",
        unknown_edges
    );
}

// ---------------------------------------------------------------------------
// Test 3: Page trigger calling Rec.TableProc() resolves via SourceTable.
// ---------------------------------------------------------------------------

/// A Page with `SourceTable = MyTable` has `Rec` resolved to the source table via
/// `record_types` pass 3. A trigger calling `Rec.TableProc()` must resolve to
/// `resolved`/`method` with a non-None `to`, and no unknown edges.
#[test]
fn page_implicit_rec_resolves_source_table_proc() {
    let tbl = r#"table 50012 MyTable {
    fields { field(1; "No."; Code[20]) { } }
    keys { key(PK; "No.") { } }
    procedure TableProc()
    begin
    end
}"#;
    let page = r#"page 50012 MyPage {
    SourceTable = MyTable;
    trigger OnAfterGetRecord()
    begin
        Rec.TableProc();
    end
}"#;
    let p = project_ws(&[("src/tbl.al", tbl), ("src/page.al", page)]);
    let edges = all_edges(&p);

    let resolved_method: Vec<&&PCallEdge> = edges
        .iter()
        .filter(|e| e.resolution == "resolved" && e.dispatch_kind == "method")
        .collect();
    assert!(
        !resolved_method.is_empty(),
        "Rec.TableProc() in page trigger must resolve; got edges: {:#?}",
        edges
    );
    assert!(
        resolved_method.iter().all(|e| e.to.is_some()),
        "resolved page edge must have a `to`; edges: {:#?}",
        resolved_method
    );

    let unknown_edges: Vec<&&PCallEdge> =
        edges.iter().filter(|e| e.resolution == "unknown").collect();
    assert!(
        unknown_edges.is_empty(),
        "no unknown edges should remain; unknowns: {:#?}",
        unknown_edges
    );
}

// ---------------------------------------------------------------------------
// Test 4: Codeunit with undeclared `Rec` stays unknown.
// ---------------------------------------------------------------------------

/// A codeunit has no effective own table, so `record_types` pass 3 never sets
/// `table_id` on any implicit `Rec` record variable. A call `Rec.Foo()` where `Rec`
/// is not declared in the codeunit's variable section must stay
/// `Unknown { UntrackedReceiver }`.
#[test]
fn codeunit_stray_rec_stays_unknown() {
    let cu = r#"codeunit 50013 MyCodeunit {
    procedure Go()
    begin
        Rec.Foo();
    end
}"#;
    let p = project_ws(&[("src/cu.al", cu)]);
    let edges = all_edges(&p);

    let unknown_edges: Vec<&&PCallEdge> =
        edges.iter().filter(|e| e.resolution == "unknown").collect();
    assert!(
        !unknown_edges.is_empty(),
        "Rec.Foo() in a codeunit with no declared Rec must be 'unknown'; got edges: {:#?}",
        edges
    );

    // No resolved edges — nothing should spuriously resolve.
    let resolved_edges: Vec<&&PCallEdge> = edges
        .iter()
        .filter(|e| e.resolution == "resolved")
        .collect();
    assert!(
        resolved_edges.is_empty(),
        "codeunit stray Rec must not resolve; resolved: {:#?}",
        resolved_edges
    );
}
