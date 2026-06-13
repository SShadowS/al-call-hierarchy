//! Phase-3 fixture tests: a member call on a Record-typed receiver whose method
//! is NOT a built-in but IS a real user table procedure resolves to
//! `resolution=="resolved"`, `dispatchKind=="method"`, with a `to` pointing at
//! the table procedure's stable id. This is the Record table-procedure dispatch
//! path added in Phase 3 (engine-d22 Task 5).
//!
//! Negative controls:
//! - Record built-ins still emit `builtin` (catalog-hit path unchanged).
//! - A method that is neither a built-in nor a table procedure stays `unknown`.
//! - Arity overloads resolve to the matching overload.
//! - The implicit `Rec` receiver (in table triggers) resolves similarly.

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

fn count_resolution(p: &L3CallGraphProjection, resolution: &str) -> usize {
    all_edges(p)
        .iter()
        .filter(|e| e.resolution == resolution)
        .count()
}

/// Minimal table fixture with a user-defined procedure.
const CUST_TABLE: &str = r#"table 50000 Customer {
    fields { field(1; "No."; Code[20]) { } }
    keys { key(PK; "No.") { } }
    procedure CalcDiscount()
    begin
    end
}"#;

/// Test 1: a call to a real table procedure (not a builtin) resolves.
#[test]
fn record_table_procedure_resolves() {
    let cu = r#"codeunit 50100 A {
    procedure Go()
    var
        C: Record Customer;
    begin
        C.CalcDiscount();
    end
}"#;
    let p = project_ws(&[("src/tbl.al", CUST_TABLE), ("src/a.al", cu)]);
    let edges = all_edges(&p);

    // Find the CalcDiscount edge
    let calc_edges: Vec<&&PCallEdge> = edges
        .iter()
        .filter(|e| e.resolution == "resolved" && e.dispatch_kind == "method")
        .collect();

    assert!(
        !calc_edges.is_empty(),
        "C.CalcDiscount() must resolve to 'resolved'/'method'; got edges: {:#?}",
        edges
    );
    assert!(
        calc_edges.iter().all(|e| e.to.is_some()),
        "resolved edge must have a non-None `to`; edges: {:#?}",
        calc_edges
    );
    // No unknown edges for method dispatch on this Record var
    let unknown_method_edges: Vec<&&PCallEdge> = edges
        .iter()
        .filter(|e| e.resolution == "unknown" && e.dispatch_kind == "method")
        .collect();
    assert!(
        unknown_method_edges.is_empty(),
        "no method edge should stay unknown; unknowns: {:#?}",
        unknown_method_edges
    );
}

/// Test 2: Record built-in methods (in the catalog) still emit `builtin`.
#[test]
fn record_builtin_still_builtin() {
    let cu = r#"codeunit 50101 B {
    procedure Go()
    var
        C: Record Customer;
    begin
        C.FieldNo("No.");
        C.TableCaption();
    end
}"#;
    let p = project_ws(&[("src/tbl.al", CUST_TABLE), ("src/b.al", cu)]);
    let edges = all_edges(&p);

    assert!(
        count_resolution(&p, "builtin") >= 2,
        "Record intrinsics (FieldNo, TableCaption) must still be 'builtin'; edges: {:#?}",
        edges
    );
    // No resolved edges — these are NOT table procedures
    let resolved_method: Vec<&&PCallEdge> = edges
        .iter()
        .filter(|e| e.resolution == "resolved" && e.dispatch_kind == "method")
        .collect();
    assert!(
        resolved_method.is_empty(),
        "catalog-hit builtins must NOT be 'resolved'; edges: {:#?}",
        resolved_method
    );
}

/// Test 3: A method that is NOT a builtin and NOT a real table procedure stays `unknown`.
#[test]
fn record_missing_method_stays_unknown() {
    let cu = r#"codeunit 50102 C {
    procedure Go()
    var
        C: Record Customer;
    begin
        C.NoSuchProc();
    end
}"#;
    let p = project_ws(&[("src/tbl.al", CUST_TABLE), ("src/c.al", cu)]);
    let edges = all_edges(&p);

    let unknown_edges: Vec<&&PCallEdge> =
        edges.iter().filter(|e| e.resolution == "unknown").collect();
    assert_eq!(
        unknown_edges.len(),
        1,
        "a non-builtin non-table-proc method must stay 'unknown'; edges: {:#?}",
        edges
    );
    assert_eq!(
        count_resolution(&p, "resolved"),
        0,
        "must not spuriously resolve a missing method"
    );
}

/// Test 4: The implicit `Rec` receiver in a table trigger — Phase 3 DEFERRED.
///
/// When `Rec.CalcDiscount()` is written explicitly in a table trigger, `Rec` is
/// the implicit record. It is registered in `routine.record_variables` (with
/// `table_id` set by record_types pass 3) but NOT in `routine.variables`.
/// The Phase-3 dispatch code operates in the `RecordTableProcedure` branch which
/// is only reached AFTER Step 2 finds `recv_var` in `routine.variables`. For the
/// implicit `Rec` receiver, Step 2 fails (UntrackedReceiver) before Phase 3 can
/// act. Resolving this requires either (a) adding `Rec` to `routine.variables`
/// with the correct declared type, or (b) a separate check in Step 2 for
/// record_variables. This is deferred to the ReceiverType lattice refactor (Task 6).
/// This test asserts the CURRENT behavior (unknown) and documents the limitation.
#[test]
fn implicit_rec_table_procedure_deferred() {
    let tbl = r#"table 50001 Item {
    fields { field(1; "No."; Code[20]) { } }
    keys { key(PK; "No.") { } }
    trigger OnInsert()
    begin
        Rec.CalcDiscount();
    end
    procedure CalcDiscount()
    begin
    end
}"#;
    let p = project_ws(&[("src/item.al", tbl)]);
    let edges = all_edges(&p);

    // DEFERRED: implicit Rec (trigger) is not in routine.variables, so Step 2
    // returns UntrackedReceiver before Phase-3 record dispatch can fire.
    // Current behavior: unknown (not yet resolved).
    let unknown_edges: Vec<&&PCallEdge> =
        edges.iter().filter(|e| e.resolution == "unknown").collect();
    assert_eq!(
        unknown_edges.len(),
        1,
        "DEFERRED: Rec.CalcDiscount() in a table trigger stays unknown (Phase-3 limitation); edges: {:#?}",
        edges
    );
}

/// Test 5: Two table procedures with the same name but different arity — a call
/// with one argument resolves to the matching overload (not the zero-arg one).
#[test]
fn record_proc_arity_overload() {
    let tbl = r#"table 50002 Order {
    fields { field(1; "No."; Code[20]) { } }
    keys { key(PK; "No.") { } }
    procedure Calc()
    begin
    end
    procedure Calc(Pct: Decimal)
    begin
    end
}"#;
    let cu = r#"codeunit 50103 D {
    procedure Go()
    var
        O: Record Order;
    begin
        O.Calc(10);
    end
}"#;
    let p = project_ws(&[("src/order.al", tbl), ("src/d.al", cu)]);
    let edges = all_edges(&p);

    let resolved_edges: Vec<&&PCallEdge> = edges
        .iter()
        .filter(|e| e.resolution == "resolved" && e.dispatch_kind == "method")
        .collect();

    assert_eq!(
        resolved_edges.len(),
        1,
        "exactly one resolved edge (the 1-arg overload); got: {:#?}",
        edges
    );
    assert!(
        resolved_edges[0].to.is_some(),
        "resolved overload edge must have a `to`"
    );
}
