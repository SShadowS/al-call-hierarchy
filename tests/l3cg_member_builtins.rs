//! Phase-2 fixture tests: a member call on a Record / RecordRef / FieldRef /
//! KeyRef / framework receiver whose method is an intrinsic built-in classifies
//! the edge as `builtin` (NOT `unknown`). Negative control: a Record-receiver
//! method that is NOT a built-in stays `unknown` (table-procedure resolution is
//! Phase 3, deliberately out of scope here).
//!
//! NOTE ON ARCHITECTURE: Record "database operations" (SetRange, FindSet, Get,
//! Modify, etc.) are extracted at L2 as `record_operations`, NOT as `call_sites`,
//! and therefore never reach the L3 call resolver. The methods tested here are
//! the REMAINING Record intrinsics that ARE extracted as call_sites (FieldNo,
//! TableCaption, GetFilter, etc.) plus non-Record framework types whose ALL
//! methods go through the call resolver.

use al_call_hierarchy::engine::l3::call_graph_projection::{L3CallGraphProjection, PCallEdge};
use al_call_hierarchy::engine::l3::l3_workspace::assemble_and_resolve_default;

const APP_GUID: &str = "2b000000-0000-0000-0000-0000000002bb";

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

const CUST_TABLE: &str = "table 50000 Customer { fields { field(1; \"No.\"; Code[20]) { } } keys { key(PK; \"No.\") { } } }";

#[test]
fn record_builtin_member_is_builtin_not_unknown() {
    // Use Record methods that are NOT in record_op_type (those go through the
    // call-site resolver path). FieldNo, TableCaption, GetFilter are intrinsics
    // that reach the resolver as PCallee::Member calls.
    let src = format!(
        "{CUST_TABLE} codeunit 50100 A {{ procedure Go() var C: Record Customer; begin \
         C.FieldNo(\"No.\"); C.TableCaption(); C.GetFilter(\"No.\"); end; }}"
    );
    let p = project_ws(&[("src/a.al", &src)]);
    assert!(
        count_resolution(&p, "builtin") >= 3,
        "Record intrinsics -> builtin; edges: {:#?}",
        all_edges(&p)
    );
    assert_eq!(
        all_edges(&p)
            .iter()
            .filter(|e| e.dispatch_kind == "method" && e.resolution == "unknown")
            .count(),
        0,
        "no Record intrinsic stays unknown"
    );
}

#[test]
fn framework_builtin_members_are_builtin() {
    let src = "codeunit 50101 B { procedure Go() var J: JsonObject; T: TextBuilder; L: List of [Text]; begin \
               J.Add('k', 1); T.Append('x'); L.Add('y'); end; }";
    let p = project_ws(&[("src/b.al", src)]);
    assert!(
        count_resolution(&p, "builtin") >= 3,
        "framework intrinsics -> builtin; edges: {:#?}",
        all_edges(&p)
    );
}

#[test]
fn recordref_builtin_members_are_builtin() {
    let src = "codeunit 50102 C { procedure Go() var R: RecordRef; begin R.Open(18); R.FieldCount(); end; }";
    let p = project_ws(&[("src/c.al", src)]);
    assert!(
        count_resolution(&p, "builtin") >= 2,
        "RecordRef intrinsics -> builtin; edges: {:#?}",
        all_edges(&p)
    );
}

#[test]
fn non_catalog_record_method_stays_unknown() {
    let src = format!(
        "{CUST_TABLE} codeunit 50103 D {{ procedure Go() var C: Record Customer; begin C.CalculateDiscount(); end; }}"
    );
    let p = project_ws(&[("src/d.al", &src)]);
    assert_eq!(
        all_edges(&p)
            .iter()
            .filter(|e| e.resolution == "unknown")
            .count(),
        1,
        "a non-builtin Record method stays unknown; edges: {:#?}",
        all_edges(&p)
    );
    assert_eq!(
        count_resolution(&p, "builtin"),
        0,
        "no false builtin for a user method"
    );
}
