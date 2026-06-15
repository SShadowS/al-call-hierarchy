//! Report dataitem implicit `Rec` seeding (Rust-owned, Feature B).
//!
//! A report's dataitem triggers (`OnAfterGetRecord`, …) operate on an implicit
//! `Rec` typed to that dataitem's SOURCE TABLE — exactly like a table/page
//! trigger's implicit `Rec`. A Record-catalog builtin on that implicit `Rec`
//! (e.g. `Rec.TableCaption()`) must classify `builtin`, and a procedure on the
//! source table (`Rec.TableProc()`) must `resolve`. Before the fix both fell to
//! `untracked-receiver` (unknown), because the report-dataitem `Rec` was never
//! seeded.
//!
//! Note: Record DML/filter ops (`SetRange`, `Modify`, `FindFirst`, …) are captured
//! at L2 as `PRecordOperation`, NOT `PCallSite`, so they never reach the call
//! resolver — `TableCaption` IS a `PCallSite` and is in the catalog. (Same as
//! `l3cg_implicit_rec_dispatch.rs`.)
use al_call_hierarchy::engine::l3::call_graph_projection::{L3CallGraphProjection, PCallEdge};
use al_call_hierarchy::engine::l3::l3_workspace::assemble_and_resolve_default;

const APP_GUID: &str = "3c000000-0000-0000-0000-0000000003cc";

fn all_edges(p: &L3CallGraphProjection) -> Vec<&PCallEdge> {
    p.groups.iter().flat_map(|g| g.edges.iter()).collect()
}

fn report_with_dataitem() -> &'static str {
    // A small `Customer` table (with a `TableProc` procedure) + a report whose
    // dataitem sources it. The `OnAfterGetRecord` trigger does
    // `Rec.TableCaption()` (a Record catalog builtin → `builtin` edge) and
    // `Rec.TableProc()` (a procedure on the dataitem's source table → `resolved`).
    r#"table 50101 Customer {
    fields { field(1; "No."; Code[20]) { } }
    procedure TableProc() begin end;
}
report 50100 "My Report" {
    dataset {
        dataitem(Cust; Customer) {
            trigger OnAfterGetRecord() begin Rec.TableCaption(); Rec.TableProc(); end;
        }
    }
}
"#
}

/// `Rec.TableCaption()` (a Record catalog builtin) inside a report dataitem's
/// `OnAfterGetRecord` trigger must classify `builtin`, and `Rec.TableProc()`
/// (a procedure on the dataitem source table `Customer`) must `resolve` — the
/// implicit dataitem `Rec` is typed to its source table. Before the fix both were
/// `untracked-receiver` unknown.
#[test]
fn report_dataitem_rec_builtin_and_resolved() {
    let owned = vec![("u.al".to_string(), report_with_dataitem().to_string())];
    let resolved = assemble_and_resolve_default(&owned, APP_GUID);

    let proj = resolved.project_call_graph();
    let edges = all_edges(&proj);

    // Rec.TableCaption() — a Record builtin → must be `builtin`.
    let builtin_edges: Vec<_> = edges.iter().filter(|e| e.resolution == "builtin").collect();
    assert!(
        !builtin_edges.is_empty(),
        "Rec.TableCaption() in a report dataitem trigger must classify `builtin`; all edges: {:#?}",
        edges
    );

    // Rec.TableProc() — a procedure on the dataitem source table → must `resolve`.
    let resolved_method: Vec<_> = edges
        .iter()
        .filter(|e| e.resolution == "resolved" && e.dispatch_kind == "method")
        .collect();
    assert!(
        !resolved_method.is_empty(),
        "Rec.TableProc() in a report dataitem trigger must `resolve` to the source-table procedure; all edges: {:#?}",
        edges
    );
    assert!(
        resolved_method.iter().all(|e| e.to.is_some()),
        "resolved dataitem edge must carry a `to`; edges: {:#?}",
        resolved_method
    );

    // No edge remains `unknown` — the implicit dataitem Rec is now typed.
    assert!(
        !edges.iter().any(|e| e.resolution == "unknown"),
        "no edge should remain unknown (Rec is now typed); all edges: {:#?}",
        edges
    );
}
