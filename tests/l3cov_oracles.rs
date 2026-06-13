//! R2d EXIT GATE — native L3-DIRECT coverage-accounting oracle.
//!
//! Ground-truth-free, STRUCTURAL oracles run NATIVELY against the Rust L3 coverage
//! (`src/engine/l3/coverage.rs::build_coverage` + `L3Resolved::project_coverage`).
//! Each invariant drives an inline single-app workspace through the real
//! `assemble_and_resolve_default → resolve_calls → build_coverage` path and asserts
//! a coverage PROPERTY DIRECTLY (NOT a golden diff against al-sem expected strings —
//! that is `l3cov_vectors.rs` + the differential's `*.l3cov.golden.json`).
//!
//! ## Why an L3-DIRECT oracle (not just the byte-parity differential)
//!
//! The corpus differential (`differential_l3_coverage_match_goldens`) is BYTE-PARITY
//! with al-sem: if BOTH engines made the same accounting mistake, a pure equality
//! diff would still pass. These oracles assert the coverage CONTRACT in absolute
//! terms by RE-DERIVING the expected multisets from the resolved call edges + the L2
//! routine flags and comparing to what `build_coverage` produced — a divergence here
//! is a port bug the differential alone could miss.
//!
//! ## Covered (source-only — R2d's guard)
//!   - `unresolvedCallsites` == EXACTLY the stable-callsiteId MULTISET of the call
//!     edges whose `resolution ∈ {unknown, ambiguous, member-not-found,
//!     external-target}` — NOT `opaque` / `builtin` / `maybe` / `resolved`.
//!     Duplicates PRESERVED (an edge per occurrence; the projection sorts, never
//!     dedups).
//!   - `dynamicDispatchSites` == the stable-operationId MULTISET of the
//!     `dispatchKind == "dynamic"` edges. `dynamicDispatchSites` is `OperationId[]`
//!     and `unresolvedCallsites` is `CallsiteId[]` — they are NOT array-subsets of
//!     each other (asserted on the dynamic-Codeunit.Run case: one entry in each, but
//!     a `/csN` id vs a `/opN` id).
//!   - `routinesBodyAvailable` == count of `bodyAvailable` routines;
//!     `routinesParseIncomplete` == the StableRoutineIds of `parseIncomplete`
//!     routines — INDEPENDENT filters, NOT a partition (a syntax-error body is BOTH
//!     bodyAvailable AND parseIncomplete).
//!   - `opaqueApps` is EMPTY source-only.
//!   - MULTISET preserves duplicates (the synthetic-style member multi-edge is not
//!     reachable from AL source — interface multi-edges are `maybe`, excluded — so the
//!     duplicate-preservation contract is proven by `l3cov_vectors.rs`'s synthetic
//!     vector; here we assert the NO-real-dup property: every corpus-shaped multiset
//!     a source workspace produces has max-dup 1).
//!
//! ## Deferred (NOT source-only; later gates — where they land)
//!   - `opaqueApps` becomes NON-EMPTY only with `.app` symbol-only dependency apps
//!     (`sourceKind == "symbol-only"`), which arrive in R2.5 (`.app` ZIP +
//!     SymbolReference projection). The whole `analysisGaps` derivation (body-
//!     unavailable DEPENDENCY routines + dep-app boundaries) is R2.5 too — DROPPED
//!     from R2d.
//!   - The `sourceUnitsParsed` decrement path (index-stage warning → failedUnitRefs)
//!     is corpus-INERT source-only (no fixture emits an index warning); its parity is
//!     proven by the `warning_unparsed` vector, NOT reachable from a clean inline
//!     workspace here.

use std::collections::HashMap;

use al_call_hierarchy::engine::l3::call_resolver::{resolve_calls, DeclaredDependency};
use al_call_hierarchy::engine::l3::coverage::{build_coverage, CoverageDiagnostic, CoverageUnit};
use al_call_hierarchy::engine::l3::l3_workspace::{assemble_and_resolve_default, L3Resolved};
use al_call_hierarchy::engine::l3::symbol_table::SymbolTable;
use al_call_hierarchy::engine::l3::taxonomy::DispatchKind;

const APP_GUID: &str = "0d000000-0000-0000-0000-0000000002dd";

/// Resolve an inline single-app workspace and return both the resolved model AND
/// its projected coverage (with implicit `ws:<name>` source units, no diagnostics).
fn resolve_and_cover(files: &[(&str, &str)]) -> (L3Resolved, CoverageView) {
    let owned: Vec<(String, String)> = files
        .iter()
        .map(|(n, s)| (n.to_string(), s.to_string()))
        .collect();
    let units: Vec<CoverageUnit> = files
        .iter()
        .map(|(n, _)| CoverageUnit {
            id: format!("ws:{n}"),
            kind: "source".to_string(),
        })
        .collect();
    let no_diags: Vec<CoverageDiagnostic> = Vec::new();
    let resolved = assemble_and_resolve_default(&owned, APP_GUID);
    let coverage = resolved.project_coverage(&units, &no_diags);
    (
        resolved,
        CoverageView {
            unresolved_callsites: coverage.unresolved_callsites,
            dynamic_dispatch_sites: coverage.dynamic_dispatch_sites,
            routines_body_available: coverage.routines_body_available,
            routines_parse_incomplete: coverage.routines_parse_incomplete,
            routines_total: coverage.routines_total,
            opaque_apps: coverage.opaque_apps,
        },
    )
}

struct CoverageView {
    unresolved_callsites: Vec<String>,
    dynamic_dispatch_sites: Vec<String>,
    routines_body_available: usize,
    routines_parse_incomplete: Vec<String>,
    routines_total: usize,
    opaque_apps: Vec<String>,
}

/// Independently RE-DERIVE the expected unresolved-callsite + dynamic-site multisets
/// from the resolved call edges (the oracle's ground truth), in STABLE id form.
fn rederive_multisets(resolved: &L3Resolved) -> (Vec<String>, Vec<String>) {
    let ws = &resolved.workspace;
    let symbols = SymbolTable::build(&ws.objects, &ws.tables, &ws.routines);
    let no_deps: Vec<DeclaredDependency> = Vec::new();
    let no_fetched: Vec<String> = Vec::new();
    let edges = resolve_calls(ws, &symbols, &no_deps, &no_fetched).edges;

    let by_internal: HashMap<String, String> = ws
        .routines
        .iter()
        .map(|r| (r.id.clone(), r.stable_routine_id.clone()))
        .collect();
    let stable_site = |site: &str| -> String {
        match site.rsplit_once('/') {
            Some((prefix, suffix)) => {
                let sp = by_internal
                    .get(prefix)
                    .cloned()
                    .unwrap_or_else(|| prefix.to_string());
                format!("{sp}/{suffix}")
            }
            None => site.to_string(),
        }
    };

    let mut unresolved: Vec<String> = edges
        .iter()
        .filter(|e| {
            matches!(
                e.resolution.as_str(),
                "unknown" | "ambiguous" | "member-not-found" | "external-target"
            )
        })
        .map(|e| stable_site(&e.callsite_id))
        .collect();
    unresolved.sort();
    let mut dynamic: Vec<String> = edges
        .iter()
        .filter(|e| e.dispatch_kind == DispatchKind::Dynamic)
        .map(|e| stable_site(&e.operation_id))
        .collect();
    dynamic.sort();
    (unresolved, dynamic)
}

// ---------------------------------------------------------------------------
// (1) unresolvedCallsites == the 4-resolutions edge multiset; dynamic == dynamic.
// ---------------------------------------------------------------------------

#[test]
fn unresolved_and_dynamic_are_exactly_the_edge_multisets() {
    // A workspace mixing every resolution kind: a dynamic Codeunit.Run (dynamic +
    // unknown), a member-not-found, a resolved direct call, and a builtin.
    let files = [(
        "a",
        "codeunit 50100 Caller\n\
         {\n\
             procedure Run(which: Integer)\n\
             var\n\
                 h: Codeunit Helper;\n\
                 n: Integer;\n\
             begin\n\
                 Codeunit.Run(which);\n\
                 h.Nonexistent();\n\
                 Ok();\n\
                 n := StrLen('hi');\n\
             end;\n\
             procedure Ok() begin end;\n\
         }\n\
         codeunit 50101 Helper\n\
         {\n\
             procedure Exists() begin end;\n\
         }",
    )];
    let (resolved, cov) = resolve_and_cover(&files);
    let (exp_unresolved, exp_dynamic) = rederive_multisets(&resolved);

    assert_eq!(
        cov.unresolved_callsites, exp_unresolved,
        "unresolvedCallsites MUST equal the stable-callsiteId multiset of the 4-resolution edges"
    );
    assert_eq!(
        cov.dynamic_dispatch_sites, exp_dynamic,
        "dynamicDispatchSites MUST equal the stable-operationId multiset of the dynamic edges"
    );

    // The dynamic Codeunit.Run produced one unresolved callsite AND one dynamic site.
    assert!(
        !cov.unresolved_callsites.is_empty(),
        "the dynamic Codeunit.Run + member-not-found populate unresolvedCallsites"
    );
    assert_eq!(
        cov.dynamic_dispatch_sites.len(),
        1,
        "exactly one dynamic dispatch site (Codeunit.Run(<var>))"
    );

    // OperationId[] vs CallsiteId[] — NOT array-subsets: the dynamic site is a `/opN`
    // id, never present in the `/csN` unresolved list.
    for op in &cov.dynamic_dispatch_sites {
        assert!(
            op.contains("/op"),
            "dynamicDispatchSites carries operationIds (`/opN`): {op}"
        );
        assert!(
            !cov.unresolved_callsites.contains(op),
            "an operationId must NOT appear in the callsiteId list (different id spaces): {op}"
        );
    }
    for cs in &cov.unresolved_callsites {
        assert!(
            cs.contains("/cs"),
            "unresolvedCallsites carries callsiteIds (`/csN`): {cs}"
        );
    }
}

// ---------------------------------------------------------------------------
// (2) the negatives — builtin / opaque / resolved are NEVER in unresolved.
// ---------------------------------------------------------------------------

#[test]
fn builtin_opaque_resolved_excluded_from_unresolved() {
    // Pure builtin + resolved-direct + opaque-codeunit-run (named target not in ws):
    // none should land in unresolvedCallsites.
    let files = [(
        "a",
        "codeunit 50100 X\n\
         {\n\
             procedure P()\n\
             var\n\
                 n: Integer;\n\
             begin\n\
                 n := StrLen('hi');\n\
                 Q();\n\
                 Codeunit.Run(Codeunit::\"Out Of World Cu\");\n\
             end;\n\
             procedure Q() begin end;\n\
         }",
    )];
    let (resolved, cov) = resolve_and_cover(&files);
    let (exp_unresolved, _) = rederive_multisets(&resolved);

    assert_eq!(cov.unresolved_callsites, exp_unresolved);
    assert!(
        cov.unresolved_callsites.is_empty(),
        "builtin (StrLen) / resolved (Q) / opaque (named Codeunit.Run) are all EXCLUDED — \
         unresolvedCallsites must be empty, got {:?}",
        cov.unresolved_callsites
    );
    assert!(
        cov.dynamic_dispatch_sites.is_empty(),
        "a NAMED Codeunit.Run target is not dynamic"
    );
}

// ---------------------------------------------------------------------------
// (3) bodyAvailable (count) and parseIncomplete (list) are INDEPENDENT filters.
// ---------------------------------------------------------------------------

#[test]
fn body_available_and_parse_incomplete_are_independent_not_a_partition() {
    // One clean routine + one with a syntax-error body. The broken routine STILL has
    // a code block → bodyAvailable counts BOTH; parseIncomplete lists ONLY the broken
    // one. A routine can be bodyAvailable AND parseIncomplete simultaneously.
    let files = [(
        "a",
        "codeunit 50100 X\n\
         {\n\
             procedure Good() begin end;\n\
             procedure Bad()\n\
             begin\n\
                 if x then ;;; @@@ broken\n\
             end;\n\
         }",
    )];
    let (resolved, cov) = resolve_and_cover(&files);

    assert_eq!(cov.routines_total, 2, "two routines");
    assert_eq!(
        cov.routines_body_available, 2,
        "BOTH routines have a code block (the broken one too) — bodyAvailable is independent"
    );
    assert_eq!(
        cov.routines_parse_incomplete.len(),
        1,
        "exactly one parse-incomplete routine"
    );

    // The parse-incomplete id is a StableRoutineId (appGuid:Type:num#hash) and equals
    // the broken routine's stable id (re-derived from the resolved model).
    let broken = resolved
        .workspace
        .routines
        .iter()
        .find(|r| r.parse_incomplete)
        .expect("the Bad routine is parse-incomplete");
    assert!(
        broken.body_available,
        "the parse-incomplete routine ALSO has bodyAvailable (the independence proof)"
    );
    assert_eq!(
        cov.routines_parse_incomplete[0], broken.stable_routine_id,
        "routinesParseIncomplete carries the broken routine's StableRoutineId"
    );
    assert!(
        cov.routines_parse_incomplete[0].contains(':')
            && cov.routines_parse_incomplete[0].contains('#'),
        "StableRoutineId form (appGuid:Type:num#hash): {}",
        cov.routines_parse_incomplete[0]
    );
}

// ---------------------------------------------------------------------------
// (4) opaqueApps is empty source-only; multisets have no real duplicate.
// ---------------------------------------------------------------------------

#[test]
fn opaque_apps_empty_and_no_real_duplicate_source_only() {
    // A multi-object workspace exercising several unresolved edges — opaqueApps stays
    // empty (no symbol-only dependency apps), and no callsiteId repeats (interface
    // multi-edges are `maybe`, excluded; AL source produces at most one unresolved
    // edge per callsite).
    let files = [(
        "a",
        "codeunit 50100 Caller\n\
         {\n\
             procedure R1(w: Integer) begin Codeunit.Run(w); end;\n\
             procedure R2(w: Integer) begin Codeunit.Run(w); end;\n\
         }",
    )];
    let (_resolved, cov) = resolve_and_cover(&files);

    assert!(
        cov.opaque_apps.is_empty(),
        "opaqueApps MUST be empty source-only (no symbol-only deps); becomes non-empty in R2.5"
    );

    // Two DISTINCT dynamic Codeunit.Run callsites (different routines) → two distinct
    // ids, max-dup 1 (no real duplicate from AL source).
    let max_dup = |ids: &[String]| -> usize {
        let mut counts: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
        for id in ids {
            *counts.entry(id.as_str()).or_insert(0) += 1;
        }
        counts.values().copied().max().unwrap_or(0)
    };
    assert_eq!(
        max_dup(&cov.unresolved_callsites),
        1,
        "no real duplicate in unresolvedCallsites from AL source (max-dup 1)"
    );
    assert_eq!(
        max_dup(&cov.dynamic_dispatch_sites),
        1,
        "no real duplicate in dynamicDispatchSites from AL source (max-dup 1)"
    );
    assert_eq!(
        cov.dynamic_dispatch_sites.len(),
        2,
        "two distinct dynamic sites (R1 + R2)"
    );
}

// ---------------------------------------------------------------------------
// (5) the multiset is SORTED + the synthetic duplicate-preservation contract is
//     re-asserted natively (the differential never exercises a real dup).
// ---------------------------------------------------------------------------

#[test]
fn multiset_is_sorted_and_preserves_duplicates_synthetically() {
    use al_call_hierarchy::engine::l3::call_resolver::{CallEdge, UnknownReason};
    use al_call_hierarchy::engine::l3::taxonomy::{DispatchKind, Resolution};

    // Two unresolved edges sharing one callsiteId + two dynamic edges sharing one
    // operationId — exactly the synthetic shape al-sem's `buildCoverage` preserves.
    let dk_of = |dk: &str| match dk {
        "unresolved" => DispatchKind::Unresolved,
        "interface" => DispatchKind::Interface,
        "dynamic" => DispatchKind::Dynamic,
        "method" => DispatchKind::Method,
        "builtin" => DispatchKind::Builtin,
        other => panic!("unexpected dispatch_kind in test: {other}"),
    };
    let res_of = |res: &str| match res {
        "unknown" => Resolution::Unknown(UnknownReason::CalleeUnknown),
        "member-not-found" => Resolution::MemberNotFound,
        "maybe" => Resolution::Maybe,
        "opaque" => Resolution::Opaque,
        "builtin" => Resolution::Builtin,
        other => panic!("unexpected resolution in test: {other}"),
    };
    let edge = |cs: &str, op: &str, dk: &str, res: &str| CallEdge {
        from: "r0/deadbeef".to_string(),
        to: None,
        callsite_id: cs.to_string(),
        operation_id: op.to_string(),
        dispatch_kind: dk_of(dk),
        resolution: res_of(res),
        candidates: None,
        external_type_ref: None,
        receiver_type: None,
        dispatch_meta: None,
    };
    let edges = vec![
        edge(
            "r0/deadbeef/cs0",
            "r0/deadbeef/op0",
            "unresolved",
            "unknown",
        ),
        edge(
            "r0/deadbeef/cs0",
            "r0/deadbeef/op1",
            "interface",
            "member-not-found",
        ),
        edge("r0/deadbeef/cs1", "r0/deadbeef/op9", "dynamic", "unknown"),
        edge("r0/deadbeef/cs2", "r0/deadbeef/op9", "dynamic", "unknown"),
        // excluded resolutions — must NOT appear in unresolvedCallsites.
        edge("r0/deadbeef/cs3", "r0/deadbeef/op3", "interface", "maybe"),
        edge("r0/deadbeef/cs4", "r0/deadbeef/op4", "method", "opaque"),
        edge("r0/deadbeef/cs5", "r0/deadbeef/op5", "builtin", "builtin"),
    ];
    let by_internal: HashMap<String, String> = HashMap::new();
    let cov = build_coverage(&[], &[], &edges, &[], &[], &by_internal);

    // cs0 twice; cs1, cs2 once; maybe/opaque/builtin excluded → 4 entries.
    assert_eq!(
        cov.unresolved_callsites,
        vec![
            "r0/deadbeef/cs0".to_string(),
            "r0/deadbeef/cs0".to_string(),
            "r0/deadbeef/cs1".to_string(),
            "r0/deadbeef/cs2".to_string(),
        ],
        "unresolvedCallsites preserves the cs0 duplicate, is sorted, excludes maybe/opaque/builtin"
    );
    // op9 twice.
    assert_eq!(
        cov.dynamic_dispatch_sites,
        vec!["r0/deadbeef/op9".to_string(), "r0/deadbeef/op9".to_string()],
        "dynamicDispatchSites preserves the op9 duplicate, is sorted"
    );

    // Sorted invariant (explicit).
    let mut s = cov.unresolved_callsites.clone();
    s.sort();
    assert_eq!(cov.unresolved_callsites, s, "unresolvedCallsites is sorted");
}
