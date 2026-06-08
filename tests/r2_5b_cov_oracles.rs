//! R2.5b-d EXIT-GATE — native CROSS-APP L3 coverage oracle (REV3).
//!
//! Ground-truth-free, STRUCTURAL oracles run NATIVELY against the Rust cross-app L3
//! coverage (`build_cross_app_l3_from_workspace` → `project_coverage_disk` over the
//! merged workspace+`.app`-dep index). Each invariant asserts a SPECIFIC expected
//! outcome DIRECTLY on the resolved cross-app model — not an aggregate `≥1` count.
//!
//! ## Why a native oracle (not just the byte-parity differential)
//!
//! `r2_5b_cov_differential.rs` is byte-parity with al-sem: if BOTH engines made the
//! same cross-app coverage mistake (e.g. counted a dep routine wrong, or left a resolved
//! callsite IN the unresolved multiset), a pure equality diff would still pass. These
//! oracles assert the cross-app coverage CONTRACT in ABSOLUTE terms.
//!
//! ## Covered (cross-app coverage — R2.5b-d's guard, REV3)
//!   - `opaqueApps` lists the symbol-only dep app guids (R3a-0 Fix 2: buildCoverage reads
//!     identity.apps, which withDependencyArtifacts now stamps with the symbol-only deps).
//!     The two corpus deps (Lib Core / Lib Ext) are symbol-only → both appear;
//!   - `routinesTotal` INCLUDES dep routines (= 13: workspace-side + dep) — verified
//!     against `index.routines.length` semantics (no analysisRole filter);
//!   - the cross-app RESOLVED callsites (Host Mgt 70000 cs0..cs3) are ABSENT from
//!     `unresolvedCallsites` (the cross-app coverage WIN);
//!   - the external-target member miss (Host Opaque 70002 cs1) STAYS IN
//!     `unresolvedCallsites`; the member-not-found (Host Mgt 70000 cs4) too;
//!   - `unresolvedCallsites` / `dynamicDispatchSites` are SORTED multisets (no dedup).

use std::path::PathBuf;

use al_call_hierarchy::engine::deps::cross_app_l3::{
    build_cross_app_l3_from_workspace, CrossAppL3,
};

const MODEL_INSTANCE_ID: &str = "r2.5b";
const DEP_CORE: &str = "dddddddd-0000-0000-0000-000000000001";
const DEP_OTHER: &str = "eeeeeeee-0000-0000-0000-000000000002";

// The workspace caller objects (SPECIFIC). The cross-app callsites that RESOLVE live on
// Host Mgt 70000; the external-target member miss lives on Host Opaque 70002.
const HOST_MGT: &str = "11111111-0000-0000-0000-0000000000aa:Codeunit:70000";
const HOST_OPAQUE: &str = "11111111-0000-0000-0000-0000000000aa:Codeunit:70002";

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/r2-5b-fixtures/cross-app-resolution")
}

/// Build the cross-app L3 over the committed `.app`-bearing fixture.
fn build() -> CrossAppL3 {
    build_cross_app_l3_from_workspace(&fixture(), MODEL_INSTANCE_ID)
        .expect("cross-app L3 builds over the `.app`-bearing workspace")
}

// ============================================================================
// 1. `opaqueApps` lists the symbol-only dep apps (R3a-0 Fix 2 — latent bug FIXED).
// ============================================================================

#[test]
fn opaque_apps_lists_the_symbol_only_deps() {
    let cross = build();
    let cov = cross.project_coverage_disk(&fixture());

    // R3a-0 Fix 2: buildCoverage filters index.identity.apps by sourceKind=="symbol-only",
    // and withDependencyArtifacts now stamps the dep AppIdentitys (with sourceKind) into
    // identity.apps. The two corpus deps (Lib Core / Lib Ext) are symbol-only, so both
    // appear in opaqueApps, in the apps-ledger order (workspace "source" filtered out).
    assert_eq!(
        cov.opaque_apps,
        vec![DEP_CORE.to_string(), DEP_OTHER.to_string()],
        "opaqueApps MUST list the two symbol-only dep apps (Lib Core then Lib Ext) — R3a-0 Fix 2",
    );

    // Sanity: the `apps` ledger carries exactly these symbol-only deps (the source of the
    // opaqueApps rows — the coverage path no longer drops them).
    let symbol_only: Vec<&String> = cross
        .apps
        .iter()
        .filter(|(_, kind)| kind == "symbol-only")
        .map(|(g, _)| g)
        .collect();
    assert!(
        symbol_only.iter().any(|g| g.as_str() == DEP_CORE),
        "the dep ledger carries the symbol-only Lib Core",
    );
    assert!(
        symbol_only.iter().any(|g| g.as_str() == DEP_OTHER),
        "the dep ledger carries the symbol-only Lib Ext",
    );
}

// ============================================================================
// 2. `routinesTotal` INCLUDES dep routines (the Task-1 handoff question).
// ============================================================================

#[test]
fn routines_total_includes_dep_routines() {
    let cross = build();
    let cov = cross.project_coverage_disk(&fixture());

    // VERIFIED: al-sem's buildCoverage sets routinesTotal = index.routines.length over
    // the MERGED index (no analysisRole filter), so dep routines are counted. The Rust
    // build_coverage uses ws.routines.len() over the merged routine list (dep routines
    // appended in append_dep_entities) — identical semantics. Expected = 13.
    assert_eq!(
        cov.routines_total, 13,
        "routinesTotal counts dep routines (5 workspace-side + 8 dep = 13)",
    );

    // The merged total is strictly greater than the workspace-side routine count alone:
    // count dep-owned routines in the merged model and assert the total exceeds it.
    let dep_routines = cross
        .resolved
        .workspace
        .routines
        .iter()
        .filter(|r| r.app_guid == DEP_CORE || r.app_guid == DEP_OTHER)
        .count();
    assert!(dep_routines > 0, "the merged model carries dep routines");
    assert!(
        cov.routines_total > dep_routines,
        "routinesTotal ({}) exceeds the dep-routine count ({dep_routines}) — workspace routines also counted",
        cov.routines_total,
    );
    assert_eq!(
        cov.routines_total,
        cross.resolved.workspace.routines.len(),
        "routinesTotal == the merged routine count",
    );

    // dep routines carry no body under noDepSummaries:true → bodyAvailable is the
    // workspace-side body count only.
    assert_eq!(
        cov.routines_body_available, 5,
        "bodyAvailable counts workspace-side routines"
    );
}

// ============================================================================
// 3. cross-app RESOLVED callsites are ABSENT from unresolvedCallsites (the win);
//    the external-target member miss + the member-not-found STAY IN.
// ============================================================================

#[test]
fn cross_app_resolved_callsites_absent_external_target_present() {
    let cross = build();
    let cov = cross.project_coverage_disk(&fixture());

    // The cross-app RESOLVED callsites (Host Mgt 70000 cs0..cs3 — cu.Compute /
    // InternalReset / LocalHelper / Apply) RESOLVE to dep routines, so they are ABSENT
    // from unresolvedCallsites. In a source-only world (deps opaque) they would be
    // unresolved — the cross-app coverage delta.
    for i in 0..=3 {
        let present = cov
            .unresolved_callsites
            .iter()
            .any(|id| id.starts_with(&format!("{HOST_MGT}#")) && id.ends_with(&format!("/cs{i}")));
        assert!(
            !present,
            "Host Mgt 70000 /cs{i} (a cross-app RESOLVED callsite) must be ABSENT from unresolvedCallsites",
        );
    }

    // The external-target member miss (gone.M() — Host Opaque 70002 / cs1) STAYS IN
    // unresolvedCallsites (external-target is one of the 4 unresolved resolutions).
    let ext_target = cov
        .unresolved_callsites
        .iter()
        .filter(|id| id.starts_with(&format!("{HOST_OPAQUE}#")) && id.ends_with("/cs1"))
        .count();
    assert_eq!(
        ext_target, 1,
        "the external-target member miss (Host Opaque 70002 /cs1) STAYS IN unresolvedCallsites",
    );

    // The member-not-found (cu.Missing() — Host Mgt 70000 / cs4) also STAYS IN.
    let member_not_found = cov
        .unresolved_callsites
        .iter()
        .filter(|id| id.starts_with(&format!("{HOST_MGT}#")) && id.ends_with("/cs4"))
        .count();
    assert_eq!(
        member_not_found, 1,
        "the member-not-found (Host Mgt 70000 /cs4) STAYS IN unresolvedCallsites",
    );

    // Exactly these two unresolved callsites; no dynamic dispatch.
    assert_eq!(
        cov.unresolved_callsites.len(),
        2,
        "the unresolved multiset has exactly two entries (member-not-found + external-target)",
    );
    assert_eq!(
        cov.dynamic_dispatch_sites.len(),
        0,
        "no dynamic dispatch in this corpus"
    );
}

// ============================================================================
// 4. unresolvedCallsites / dynamicDispatchSites are SORTED multisets (no dedup);
//    the cross-app RESOLVED count is non-degenerate (≥1, REV3 matrix).
// ============================================================================

#[test]
fn unresolved_multiset_sorted_and_resolution_delta_non_degenerate() {
    let cross = build();
    let cov = cross.project_coverage_disk(&fixture());

    // Sorted ascending (cmpStable byte order).
    let mut sorted = cov.unresolved_callsites.clone();
    sorted.sort();
    assert_eq!(
        cov.unresolved_callsites, sorted,
        "unresolvedCallsites is sorted (cmpStable byte order)",
    );

    // The cross-app RESOLVED count (the fail-on-zero matrix axis, REV3): classify via the
    // call graph — `resolved` edges whose `to` is a dep routine.
    let cg = cross.project_call_graph();
    let cg_json = serde_json::to_value(&cg).expect("serialize cross-app call graph");
    let mut resolved_to_dep = 0usize;
    let mut external_target = 0usize;
    if let Some(groups) = cg_json.get("groups").and_then(|g| g.as_array()) {
        for group in groups {
            if let Some(edges) = group.get("edges").and_then(|e| e.as_array()) {
                for e in edges {
                    let res = e.get("resolution").and_then(|r| r.as_str()).unwrap_or("");
                    let to = e.get("to").and_then(|t| t.as_str()).unwrap_or("");
                    let dep_owned = to.starts_with(&format!("{DEP_CORE}:"))
                        || to.starts_with(&format!("{DEP_OTHER}:"));
                    if res == "resolved" && dep_owned {
                        resolved_to_dep += 1;
                    } else if res == "external-target" {
                        external_target += 1;
                    }
                }
            }
        }
    }
    assert!(
        resolved_to_dep >= 1,
        "≥1 cross-app callsite RESOLVED to a dep routine (the cross-app coverage delta)",
    );
    assert_eq!(
        resolved_to_dep, 4,
        "the four cu.* calls (Compute / InternalReset / LocalHelper / Apply) resolved cross-app",
    );
    assert!(
        external_target >= 1,
        "≥1 external-target member miss stays IN unresolvedCallsites",
    );
}
