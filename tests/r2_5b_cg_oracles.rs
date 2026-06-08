//! R2.5b-b EXIT-GATE — native CROSS-APP L3 call-graph resolution oracle.
//!
//! Ground-truth-free, STRUCTURAL oracles run NATIVELY against the Rust cross-app L3
//! call-graph resolver (`build_cross_app_l3_from_workspace` → the unchanged
//! `resolve_calls` over the merged workspace+`.app`-dep index →
//! `project_call_graph`). Each invariant asserts a SPECIFIC expected id DIRECTLY on
//! the resolved cross-app model — NOT "is-a-dep-routine" (Rev 2 #1: an aggregate
//! `≥1` count passes even if BOTH sides bind to the WRONG dep entity).
//!
//! ## Why a native oracle (not just the byte-parity differential)
//!
//! `r2_5b_cg_differential.rs` is byte-parity with al-sem: if BOTH engines made the
//! same cross-app resolution mistake (e.g. resolved a member call to the WRONG dep
//! routine), a pure equality diff would still pass. These oracles assert the
//! cross-app call-graph CONTRACT in ABSOLUTE terms against the EXACT expected dep
//! StableRoutineId — derived BY NAME from the resolved merged model's dep routines,
//! so the assertion is both SPECIFIC and resilient to a signature-hash refresh. The
//! corpus carries ≥2 dep routines on Dep Mgt (Compute / InternalReset / LocalHelper
//! / Recalc / Apply) so a wrong-but-same binding is DETECTABLE here.
//!
//! ## Covered (cross-app call-graph resolution — R2.5b-b's guard)
//!   - a member call to a present dep object + name/arity match resolves to the
//!     EXACT dep StableRoutineId (Dep Mgt Compute), NOT another dep routine;
//!   - an `internal` dep callee (InternalReset) AND a `local` dep callee
//!     (LocalHelper) BOTH resolve identically to their EXACT dep StableRoutineIds —
//!     the resolver IGNORES accessModifier (Rev 2 #2; a wrongly-added visibility gate
//!     would drop these edges);
//!   - the opaque-vs-external-target split matches the committed al-sem R2.5b cg GOLDEN
//!     (captured with the EMPTY ledger — `primaryDependencies` stamped AFTER resolve):
//!     a member call to an ABSENT object is `external-target`; ONLY the object-run form
//!     into an absent declared dep is `opaque`. NOTE (R3a-0): PRODUCTION al-sem applies
//!     Fix 1 (real ledger DURING resolve) and would classify the member miss `opaque`
//!     for THIS corpus (it has an unfetched declared dep + `gone.M()`); the golden is
//!     STALE on this point (an al-sem capture-harness concern). The production-correct
//!     member-`opaque` resolver behavior is proven by `tests/r3a0_unfetched_dep_opaque.rs`;
//!   - a member miss on a PRESENT dep object is `member-not-found`;
//!   - a cross-app callsite's argumentBindings UPGRADED on resolution: the
//!     `cu.Apply(localCust)` record-arg binding is `resolved` + `calleeParameterIsVar`
//!     (the dep `Apply` param IS `var`) — only observable post-resolve (Rev 2 #3).

use std::collections::HashMap;
use std::path::PathBuf;

use al_call_hierarchy::engine::deps::cross_app_l3::{
    build_cross_app_l3_from_workspace, CrossAppL3,
};
use al_call_hierarchy::engine::l3::call_graph_projection::{L3CallGraphProjection, PCallEdge};

const MODEL_INSTANCE_ID: &str = "r2.5b";
const DEP_CORE: &str = "dddddddd-0000-0000-0000-000000000001";
const DEP_MGT_OBJECT: &str = "dddddddd-0000-0000-0000-000000000001:Codeunit:50100";

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/r2-5b-fixtures/cross-app-resolution")
}

/// Build the cross-app L3 over the committed `.app`-bearing fixture.
fn build() -> CrossAppL3 {
    build_cross_app_l3_from_workspace(&fixture(), MODEL_INSTANCE_ID)
        .expect("cross-app L3 builds over the `.app`-bearing workspace")
}

/// Map a dep Dep-Mgt routine NAME → its EXACT StableRoutineId from the resolved
/// merged model (Rev 2 #1: SPECIFIC, not "is-a-dep-*"; derived by name so it stays
/// correct across a signature-hash refresh).
fn dep_mgt_stable_id_by_name(cross: &CrossAppL3, name: &str) -> String {
    cross
        .resolved
        .workspace
        .routines
        .iter()
        .find(|r| r.app_guid == DEP_CORE && r.object_number == 50100 && r.name == name)
        .unwrap_or_else(|| panic!("dep routine Dep Mgt.{name} present in the merged model"))
        .stable_routine_id
        .clone()
}

/// All edges across all groups (flattened), preserving the multiset.
fn all_edges(cg: &L3CallGraphProjection) -> Vec<&PCallEdge> {
    cg.groups.iter().flat_map(|g| g.edges.iter()).collect()
}

/// The resolved method edges whose `to` is a dep StableRoutineId on Dep Mgt.
fn resolved_to_dep_mgt(cg: &L3CallGraphProjection) -> Vec<&PCallEdge> {
    all_edges(cg)
        .into_iter()
        .filter(|e| {
            e.resolution == "resolved"
                && e.dispatch_kind == "method"
                && e.to
                    .as_deref()
                    .map(|t| t.starts_with(&format!("{DEP_MGT_OBJECT}#")))
                    .unwrap_or(false)
        })
        .collect()
}

// ============================================================================
// 1. A member call resolves to the EXACT dep StableRoutineId (Dep Mgt.Compute),
//    NOT another dep routine (Rev 2 #1; ≥2 dep routines on the object).
// ============================================================================

#[test]
fn member_call_resolves_to_exact_dep_compute_stable_routine_id() {
    let cross = build();
    let cg = cross.project_call_graph();
    let expected = dep_mgt_stable_id_by_name(&cross, "Compute");

    let resolved = resolved_to_dep_mgt(&cg);
    // The EXACT Compute id is among the resolved targets.
    assert!(
        resolved
            .iter()
            .any(|e| e.to.as_deref() == Some(expected.as_str())),
        "a member call resolves to the EXACT dep StableRoutineId for Dep Mgt.Compute ({expected})",
    );
    // It is NOT confused with Recalc (the other public arity-1 routine) — a
    // wrong-but-same binding would fail here (≥2 dep routines make it detectable).
    let recalc = dep_mgt_stable_id_by_name(&cross, "Recalc");
    assert_ne!(
        expected, recalc,
        "Compute and Recalc are DISTINCT dep routines"
    );
}

// ============================================================================
// 2.+3. internal AND local dep callees resolve IDENTICALLY, each to its EXACT
//    dep StableRoutineId (Rev 2 #2 — the resolver ignores accessModifier).
// ============================================================================

#[test]
fn internal_and_local_dep_callees_resolve_to_their_exact_stable_routine_ids() {
    let cross = build();
    let cg = cross.project_call_graph();

    let internal = dep_mgt_stable_id_by_name(&cross, "InternalReset");
    let local = dep_mgt_stable_id_by_name(&cross, "LocalHelper");

    let resolved = resolved_to_dep_mgt(&cg);
    let targets: Vec<&str> = resolved.iter().filter_map(|e| e.to.as_deref()).collect();

    // The internal callee's edge resolves to its EXACT dep StableRoutineId — the edge
    // FORMS regardless of the `internal` modifier (no L3 visibility gate).
    assert!(
        targets.contains(&internal.as_str()),
        "the `internal` dep callee InternalReset resolves to its EXACT id ({internal}) — no accessModifier gate",
    );
    // Same for the `local` callee.
    assert!(
        targets.contains(&local.as_str()),
        "the `local` dep callee LocalHelper resolves to its EXACT id ({local}) — no accessModifier gate",
    );

    // The internal + local edges are SHAPE-IDENTICAL to the public Compute edge:
    // same resolution / dispatchKind / receiverType (only `to`/`operationId` differ).
    for e in &resolved {
        assert_eq!(e.resolution, "resolved");
        assert_eq!(e.dispatch_kind, "method");
        assert_eq!(e.receiver_type.as_deref(), Some("Codeunit \"Dep Mgt\""));
    }
}

// ============================================================================
// 4. The four resolved member edges are FOUR DISTINCT dep StableRoutineIds
//    (Compute / InternalReset / LocalHelper / Apply) — proving NOT all-collapse.
// ============================================================================

#[test]
fn resolved_member_edges_are_four_distinct_dep_routines() {
    let cross = build();
    let cg = cross.project_call_graph();

    let resolved = resolved_to_dep_mgt(&cg);
    assert_eq!(
        resolved.len(),
        4,
        "Compute + InternalReset + LocalHelper + Apply = 4 resolved member edges to Dep Mgt",
    );
    let distinct: std::collections::HashSet<&str> =
        resolved.iter().filter_map(|e| e.to.as_deref()).collect();
    assert_eq!(
        distinct.len(),
        4,
        "the 4 resolved targets are 4 DISTINCT dep StableRoutineIds (a wrong-but-same binding would collapse this)",
    );

    // Each distinct target is exactly one of the 4 expected dep routine ids.
    let expected: HashMap<&str, String> = ["Compute", "InternalReset", "LocalHelper", "Apply"]
        .iter()
        .map(|&n| (n, dep_mgt_stable_id_by_name(&cross, n)))
        .collect();
    let expected_set: std::collections::HashSet<&str> =
        expected.values().map(|s| s.as_str()).collect();
    assert_eq!(
        distinct, expected_set,
        "the resolved targets are EXACTLY {{Compute, InternalReset, LocalHelper, Apply}}",
    );
}

// ============================================================================
// 5. The opaque-vs-external-target split matches the committed al-sem R2.5b cg GOLDEN
//    (captured with the EMPTY ledger): a MEMBER call to an absent object is
//    `external-target`; ONLY the object-run form into an absent declared dep is
//    `opaque`. (PRODUCTION al-sem would classify the member miss `opaque` here — Fix 1;
//    the golden is stale, see the header. tests/r3a0_unfetched_dep_opaque.rs proves the
//    production member-opaque resolver behavior.)
// ============================================================================

#[test]
fn opaque_is_object_run_only_and_member_miss_is_external_target() {
    let cross = build();
    let cg = cross.project_call_graph();
    let edges = all_edges(&cg);

    // EXACTLY one `opaque` edge, and it is the OBJECT-RUN form (Codeunit.Run).
    let opaque: Vec<&&PCallEdge> = edges.iter().filter(|e| e.resolution == "opaque").collect();
    assert_eq!(opaque.len(), 1, "exactly one opaque edge (the object-run)");
    assert_eq!(
        opaque[0].dispatch_kind, "codeunit-run",
        "the opaque edge is the OBJECT-RUN form (Codeunit.Run into an absent declared dep)",
    );

    // NO member-call edge is `opaque` in the GOLDEN-PARITY (empty-ledger) projection.
    let member_opaque = edges
        .iter()
        .filter(|e| e.resolution == "opaque" && e.dispatch_kind == "method")
        .count();
    assert_eq!(
        member_opaque, 0,
        "NO member call is opaque in the empty-ledger projection (matches the committed golden; \
         production al-sem applies Fix 1 → member-opaque, proven in r3a0_unfetched_dep_opaque.rs)",
    );

    // EXACTLY one `external-target` edge, and it is a MEMBER call to the absent obj.
    let ext: Vec<&&PCallEdge> = edges
        .iter()
        .filter(|e| e.resolution == "external-target")
        .collect();
    assert_eq!(ext.len(), 1, "exactly one external-target edge");
    assert_eq!(
        ext[0].dispatch_kind, "method",
        "the external-target edge is a MEMBER call (gone.M()) whose object is absent",
    );
    let ext_ref = ext[0]
        .external_type_ref
        .as_ref()
        .expect("external-target carries an externalTypeRef");
    assert_eq!(ext_ref.kind, "Codeunit");
    assert_eq!(ext_ref.name, "Nowhere Cu");
}

// ============================================================================
// 6. A member miss on a PRESENT dep object is `member-not-found` (NOT
//    external-target / opaque — the object IS present).
// ============================================================================

#[test]
fn member_miss_on_present_dep_object_is_member_not_found() {
    let cross = build();
    let cg = cross.project_call_graph();
    let edges = all_edges(&cg);

    let mnf: Vec<&&PCallEdge> = edges
        .iter()
        .filter(|e| e.resolution == "member-not-found")
        .collect();
    assert_eq!(
        mnf.len(),
        1,
        "exactly one member-not-found edge (cu.Missing())"
    );
    assert_eq!(mnf[0].dispatch_kind, "method");
    // member-not-found carries NO resolved `to`.
    assert!(
        mnf[0].to.is_none(),
        "member-not-found has no resolved target",
    );
}

// ============================================================================
// 7. A cross-app callsite's argumentBindings UPGRADED on resolution (Rev 2 #3
//    anti-stale): cu.Apply(localCust) → resolved + calleeParameterIsVar.
// ============================================================================

#[test]
fn cross_app_argument_binding_upgrades_to_resolved_var() {
    let cross = build();
    let cg = cross.project_call_graph();

    // Find the resolved Apply edge's callsite, then assert ITS binding upgraded.
    let apply_id = dep_mgt_stable_id_by_name(&cross, "Apply");
    let apply_group = cg
        .groups
        .iter()
        .find(|g| {
            g.edges
                .iter()
                .any(|e| e.resolution == "resolved" && e.to.as_deref() == Some(apply_id.as_str()))
        })
        .expect("a group resolving to dep Dep Mgt.Apply");

    let binding_site = cg
        .bindings
        .iter()
        .find(|b| b.callsite_id == apply_group.callsite_id)
        .expect("the Apply callsite carries argumentBindings");

    assert_eq!(binding_site.bindings.len(), 1, "Apply takes one record arg");
    let ab = &binding_site.bindings[0];
    // POST-resolve upgrade (Rev 2 #3): the dep callee was ABSENT pre-resolve, so the
    // binding was `unresolved-callee`; upgradeBindings set it to `resolved` +
    // calleeParameterIsVar=true (the dep `Apply` param IS `var`).
    assert_eq!(
        ab.binding_resolution, "resolved",
        "the cross-app record-arg binding UPGRADED to resolved (only observable post-resolve)",
    );
    assert!(
        ab.callee_parameter_is_var,
        "calleeParameterIsVar upgraded to true (the dep Apply param IS `var`)",
    );
    assert_eq!(ab.parameter_index, 0);
}

// ============================================================================
// 8. ≥2 dep routines on Dep Mgt so a wrong-but-same binding is detectable (the
//    structural precondition for the SPECIFIC-id oracles above, Rev 2 #1).
// ============================================================================

#[test]
fn corpus_has_at_least_two_dep_routines_on_dep_mgt() {
    let cross = build();
    let dep_routine_count = cross
        .resolved
        .workspace
        .routines
        .iter()
        .filter(|r| r.app_guid == DEP_CORE && r.object_number == 50100)
        .count();
    assert!(
        dep_routine_count >= 2,
        "Dep Mgt carries ≥2 dep routines (got {dep_routine_count}) so a wrong-but-same binding is detectable",
    );
}
