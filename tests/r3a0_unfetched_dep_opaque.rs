//! Phase R3a-0 (semantic oracle epoch) — Fix 1 RESOLVER PROOF (Rust mirror of
//! al-sem `test/contracts/r3a0-unfetched-dep-opaque.test.ts`).
//!
//! al-sem Fix 1 stamps `index.identity.primaryDependencies` onto the merged index
//! BEFORE `resolveModel`, so the call resolver's `has_unfetched_declared_dependency`
//! sees the declared deps DURING resolution. A cross-app member miss whose receiver
//! object is absent from the index — `helper.RunIt()` where `helper: Codeunit
//! "R3a0 Unfetched Cu"` — then classifies `opaque` (the member MIGHT live in a
//! declared-but-unfetched dep) instead of `external-target`.
//!
//! The Rust `resolve_calls` already implements this branch
//! (`call_resolver.rs::has_unfetched_declared_dependency` + the member-miss split).
//! This test drives `resolve_calls` DIRECTLY with the REAL ledger — a declared dep
//! whose appGuid is ABSENT from the fetched-app set (unfetched) — proving the
//! production-fixed behavior, then re-derives `build_coverage`'s `unresolvedCallsites`
//! to confirm the `opaque` callsite is ABSENT (buildCoverage filters
//! unknown|ambiguous|member-not-found|external-target — NOT opaque).
//!
//! ## Why a DIRECT-resolver test (and NOT the R2.5b cross-app projection)
//!
//! The R2.5b cross-app cg/cov PROJECTIONS feed `resolve_calls` an EMPTY ledger on
//! purpose, to byte-match the committed al-sem R2.5b goldens (which the al-sem capture
//! harness generates with `primaryDependencies` stamped AFTER resolve — the OLD order,
//! so the member-`opaque` branch is dead in the golden). That golden is STALE w.r.t.
//! PRODUCTION `analyzeWorkspace`, which threads the real ledger and DOES produce
//! `opaque` for this shape. So the corpus cannot prove Fix 1; this dedicated test
//! does — it asserts the RESOLVER (the production path) classifies `opaque`, exactly as
//! al-sem's production `analyzeWorkspace` does (verified against the R2.5b fixture).
//!
//! Asserts (matching al-sem's exact behavior):
//!   - the `helper.RunIt()` edge resolution is `opaque` (the now-live Fix-1 branch);
//!   - the callsite is ABSENT from `unresolvedCallsites`;
//!   - NEGATIVE CONTROL: the SAME source with NO declared dependency classifies the
//!     call `external-target` and the callsite IS PRESENT in unresolvedCallsites.

use std::collections::HashMap;

use al_call_hierarchy::engine::l3::call_resolver::{resolve_calls, CallEdge, DeclaredDependency};
use al_call_hierarchy::engine::l3::coverage::{build_coverage, CoverageDiagnostic, CoverageUnit};
use al_call_hierarchy::engine::l3::l3_workspace::{assemble_and_resolve_default, L3Resolved};
use al_call_hierarchy::engine::l3::symbol_table::SymbolTable;

const WS_GUID: &str = "a3a00000-0000-0000-0000-00000000aaaa";
const UNFETCHED_DEP_GUID: &str = "a3a00000-0000-0000-0000-00000000dddd";

/// A caller whose receiver var names a codeunit NOT present in the workspace source
/// (it would live in the declared-but-unfetched dep). `helper.RunIt()` is the member
/// miss under test.
const CALLER_AL: &str = "codeunit 60500 \"R3a0 Opaque Caller\"\n\
     {\n\
         var\n\
             helper: Codeunit \"R3a0 Unfetched Cu\";\n\
         procedure DoWork()\n\
         begin\n\
             helper.RunIt();\n\
         end;\n\
     }";

/// Resolve the inline workspace with the given declared-dep ledger (the fetched set is
/// EMPTY here — a declared dep is therefore UNFETCHED). Returns the resolved model + the
/// resolved edges.
fn resolve_with_deps(declared: &[DeclaredDependency]) -> (L3Resolved, Vec<CallEdge>) {
    let files = [("Caller.Codeunit.al".to_string(), CALLER_AL.to_string())];
    let resolved = assemble_and_resolve_default(&files, WS_GUID);
    let ws = &resolved.workspace;
    let symbols = SymbolTable::build(&ws.objects, &ws.tables, &ws.routines);
    let no_fetched: Vec<String> = Vec::new(); // declared dep is unfetched.
    let edges = resolve_calls(ws, &symbols, declared, &no_fetched).edges;
    (resolved, edges)
}

/// The single member-dispatch edge for `helper.RunIt()` (`method` dispatch into the
/// absent "R3a0 Unfetched Cu" object).
fn member_edge(edges: &[CallEdge]) -> &CallEdge {
    let matches: Vec<&CallEdge> = edges
        .iter()
        .filter(|e| {
            e.dispatch_kind.as_str() == "method"
                && e.external_type_ref
                    .as_ref()
                    .is_some_and(|x| x.name == "R3a0 Unfetched Cu")
        })
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "exactly one member edge into the absent dep object"
    );
    matches[0]
}

/// Build the coverage `unresolvedCallsites` for the resolved model + edges (the stable
/// callsite multiset of the 4 unresolved resolutions — opaque EXCLUDED).
fn unresolved_callsites(resolved: &L3Resolved, edges: &[CallEdge]) -> Vec<String> {
    let ws = &resolved.workspace;
    let by_internal: HashMap<String, String> = ws
        .routines
        .iter()
        .map(|r| (r.id.clone(), r.stable_routine_id.clone()))
        .collect();
    let units: Vec<CoverageUnit> = vec![CoverageUnit {
        id: "ws:Caller.Codeunit.al".to_string(),
        kind: "source".to_string(),
    }];
    let diags: Vec<CoverageDiagnostic> = Vec::new();
    build_coverage(&ws.routines, &[], edges, &units, &diags, &by_internal).unresolved_callsites
}

/// Stable form of an edge's callsite id (matches `build_coverage`'s `stable_site`).
fn stable_callsite(resolved: &L3Resolved, edge: &CallEdge) -> String {
    let by_internal: HashMap<String, String> = resolved
        .workspace
        .routines
        .iter()
        .map(|r| (r.id.clone(), r.stable_routine_id.clone()))
        .collect();
    match edge.callsite_id.rsplit_once('/') {
        Some((prefix, suffix)) => {
            let sp = by_internal
                .get(prefix)
                .cloned()
                .unwrap_or_else(|| prefix.to_string());
            format!("{sp}/{suffix}")
        }
        None => edge.callsite_id.clone(),
    }
}

// ---------------------------------------------------------------------------
// 1. declared-but-unfetched dep: member miss classifies `opaque` and is ABSENT
//    from unresolvedCallsites.
// ---------------------------------------------------------------------------

#[test]
fn unfetched_declared_dep_member_miss_is_opaque_and_absent_from_unresolved() {
    let declared = vec![DeclaredDependency {
        app_guid: UNFETCHED_DEP_GUID.to_string(),
    }];
    let (resolved, edges) = resolve_with_deps(&declared);
    let edge = member_edge(&edges);

    // The now-live Fix-1 branch: the member MIGHT live in the unfetched declared dep.
    assert_eq!(
        edge.resolution.as_str(),
        "opaque",
        "an unfetched-declared-dep member miss classifies `opaque` (Fix 1, production path)"
    );

    // `opaque` is NOT one of buildCoverage's 4 unresolved resolutions → the callsite is
    // ABSENT from unresolvedCallsites (accounted for as a dep boundary, not a gap).
    let unresolved = unresolved_callsites(&resolved, &edges);
    let site = stable_callsite(&resolved, edge);
    assert!(
        !unresolved.contains(&site),
        "the opaque member-miss callsite must be ABSENT from unresolvedCallsites: {site:?} in {unresolved:?}"
    );
}

// ---------------------------------------------------------------------------
// 2. NEGATIVE CONTROL — no declared dep: same call is `external-target` and PRESENT
//    in unresolvedCallsites.
// ---------------------------------------------------------------------------

#[test]
fn no_declared_dep_member_miss_is_external_target_and_present() {
    let declared: Vec<DeclaredDependency> = Vec::new();
    let (resolved, edges) = resolve_with_deps(&declared);
    let edge = member_edge(&edges);

    // All declared deps fetched (here: none) and the object still absent → genuinely
    // external. This is the outcome the buggy/old resolver produced for ALL such misses;
    // with Fix 1 it is reserved for the all-deps-fetched (or no-deps) case.
    assert_eq!(
        edge.resolution.as_str(),
        "external-target",
        "with NO declared dep, the member miss is `external-target`"
    );

    // external-target IS one of the 4 unresolved resolutions → present.
    let unresolved = unresolved_callsites(&resolved, &edges);
    let site = stable_callsite(&resolved, edge);
    assert!(
        unresolved.contains(&site),
        "the external-target member-miss callsite must be PRESENT in unresolvedCallsites: {site:?} in {unresolved:?}"
    );
}

// ---------------------------------------------------------------------------
// 3. determinism — the opaque classification is byte-stable across two resolves.
// ---------------------------------------------------------------------------

#[test]
fn opaque_classification_is_byte_stable() {
    let declared = vec![DeclaredDependency {
        app_guid: UNFETCHED_DEP_GUID.to_string(),
    }];
    let (ra, ea) = resolve_with_deps(&declared);
    let (rb, eb) = resolve_with_deps(&declared);
    assert_eq!(member_edge(&ea).resolution.as_str(), "opaque");
    assert_eq!(member_edge(&eb).resolution, member_edge(&ea).resolution);
    assert_eq!(
        unresolved_callsites(&ra, &ea),
        unresolved_callsites(&rb, &eb),
        "the unresolved multiset is workspace-independent and stable"
    );
}
