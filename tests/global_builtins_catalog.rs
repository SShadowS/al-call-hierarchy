//! Integration tests: bare calls to AL compiler-intrinsic global functions
//! are reclassified from `unknown` (BareUnresolved) to `builtin` by the
//! generated catalog in `global_builtins.rs`.
//!
//! Guard tests confirm that genuine unknowns (not in the catalog, not in the
//! caller's own object) still produce `unknown`.

use al_call_hierarchy::engine::l3::call_graph_projection::{L3CallGraphProjection, PCallEdge};
use al_call_hierarchy::engine::l3::l3_workspace::assemble_and_resolve_default;

const APP_GUID: &str = "2b000000-0000-0000-0000-0000000002bb";

fn project(files: &[(&str, &str)]) -> L3CallGraphProjection {
    let owned: Vec<(String, String)> = files
        .iter()
        .map(|(n, s)| (n.to_string(), s.to_string()))
        .collect();
    assemble_and_resolve_default(&owned, APP_GUID).project_call_graph()
}

fn edges(p: &L3CallGraphProjection) -> Vec<&PCallEdge> {
    p.groups.iter().flat_map(|g| g.edges.iter()).collect()
}

/// GuiAllowed() is a bare call — not in the hand allowlist, but IS in the
/// generated catalog.  It must be classified as `builtin`, not `unknown`.
#[test]
fn platform_global_bare_call_is_builtin() {
    let src = "codeunit 50001 A { \
               procedure Go() \
               begin \
                   if GuiAllowed() then; \
               end; \
               }";
    let p = project(&[("src/a.al", src)]);
    let all = edges(&p);
    assert!(
        all.iter().any(|e| e.resolution == "builtin"),
        "GuiAllowed() must produce at least one builtin edge; edges: {:#?}",
        all
    );
    assert_eq!(
        all.iter().filter(|e| e.resolution == "unknown").count(),
        0,
        "GuiAllowed() must NOT produce any unknown edge; edges: {:#?}",
        all
    );
}

/// StrLen, CreateGuid, Format — all bare catalog entries not in the hand list.
#[test]
fn multiple_globals_reclassified() {
    let src = "codeunit 50002 B { \
               procedure Go() \
               var s: Text; g: Guid; n: Integer; \
               begin \
                   n := StrLen('x'); \
                   g := CreateGuid(); \
                   s := Format(1); \
               end; \
               }";
    let p = project(&[("src/b.al", src)]);
    let all = edges(&p);
    let builtin_count = all.iter().filter(|e| e.resolution == "builtin").count();
    let unknown_count = all.iter().filter(|e| e.resolution == "unknown").count();
    assert!(
        builtin_count >= 3,
        "StrLen, CreateGuid, Format must all be builtin (got {}); edges: {:#?}",
        builtin_count,
        all
    );
    assert_eq!(
        unknown_count, 0,
        "no unknown edges expected; edges: {:#?}",
        all
    );
}

/// A bare call to a name that is NOT a builtin and NOT an own-object procedure
/// must remain `unknown`.  The catalog must NOT swallow genuine resolution holes.
#[test]
fn genuine_unknown_stays_unknown() {
    let src = "codeunit 50003 C { \
               procedure Go() \
               begin \
                   ThisIsNotARealGlobalXyz123(); \
               end; \
               }";
    let p = project(&[("src/c.al", src)]);
    let all = edges(&p);
    assert!(
        all.iter().any(|e| e.resolution == "unknown"),
        "a name not in the catalog must remain unknown; edges: {:#?}",
        all
    );
}
