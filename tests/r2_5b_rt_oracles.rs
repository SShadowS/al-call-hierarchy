//! R2.5b-a EXIT-GATE — native CROSS-APP L3 record-type resolution oracle.
//!
//! Ground-truth-free, STRUCTURAL oracles run NATIVELY against the Rust cross-app L3
//! record-type resolver (`build_cross_app_l3_from_workspace` → the unchanged
//! `l3_workspace::resolve` over the merged workspace+`.app`-dep index →
//! `project_record_types`). Each invariant asserts a SPECIFIC expected id DIRECTLY
//! on the resolved cross-app model — NOT "is-a-dep-table" (Rev 2 #1: an aggregate
//! `≥1` count passes even if BOTH sides bind to the WRONG dep entity).
//!
//! ## Why a native oracle (not just the byte-parity differential)
//!
//! `r2_5b_rt_differential.rs` is byte-parity with al-sem: if BOTH engines made the
//! same cross-app resolution mistake (e.g. bound a dep-table record var to the
//! WRONG dep table, or merged a dep-extension field onto the wrong base), a pure
//! equality diff would still pass. These oracles assert the cross-app record-type
//! CONTRACT in ABSOLUTE terms against the EXACT expected dep StableTableId /
//! StableFieldId. The corpus carries ≥2 dep tables (Dep Customer 50000 + Dep Vendor
//! 50001) so a wrong-but-same binding is DETECTABLE here.
//!
//! ## Covered (cross-app record-type resolution — R2.5b-a's guard)
//!   - a record var typed as a SPECIFIC dep table binds to its EXACT dep
//!     StableTableId (Dep Customer 50000), NOT the other dep table (50001);
//!   - a record OP on a dep-table var carries the EXACT dep StableTableId;
//!   - a base-table field visible ONLY via a DEP TableExtension merge resolves to
//!     the EXACT StableFieldId (Dep Vendor 50001 field 50 "Rating", declared by the
//!     dep TableExtension 50700) — the cross-app dep-extension merge;
//!   - the OTHER merge direction: a WORKSPACE TableExtension field merges onto a DEP
//!     base table (Dep Customer 50000 gains "Loyalty Points" 70010 from ws ext);
//!   - the dep table binding is the POST-resolve backfilled value (Rev 2 #3) — the
//!     dep table only enters the symbol table via the merge.

use std::path::PathBuf;

use al_call_hierarchy::engine::deps::cross_app_l3::build_cross_app_l3_from_workspace;
use al_call_hierarchy::engine::l3::l3_workspace::{
    L3RecordTypeProjection, PRoutineRecordTypes, PTableRecordTypes,
};

const MODEL_INSTANCE_ID: &str = "r2.5b";
const DEP_CORE: &str = "dddddddd-0000-0000-0000-000000000001";
const WS_APP: &str = "11111111-0000-0000-0000-0000000000aa";

// The SPECIFIC expected ids (Rev 2 #1 — exact, not "is-a-dep-*").
fn dep_customer() -> String {
    format!("{DEP_CORE}:Table:50000")
}
fn dep_vendor() -> String {
    format!("{DEP_CORE}:Table:50001")
}
fn dep_vendor_ext() -> String {
    format!("{DEP_CORE}:TableExtension:50700")
}
fn ws_cust_ext() -> String {
    format!("{WS_APP}:TableExtension:70010")
}

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/r2-5b-fixtures/cross-app-resolution")
}

/// Build the cross-app L3 over the committed `.app`-bearing fixture and project the
/// record-type surface.
fn project() -> L3RecordTypeProjection {
    build_cross_app_l3_from_workspace(&fixture(), MODEL_INSTANCE_ID)
        .expect("cross-app L3 builds over the `.app`-bearing workspace")
        .project_record_types()
}

fn routine_by_name<'a>(p: &'a L3RecordTypeProjection, name: &str) -> &'a PRoutineRecordTypes {
    p.routines
        .iter()
        .find(|r| r.name == name)
        .unwrap_or_else(|| panic!("routine {name} present in cross-app projection"))
}

fn table_by_stable_id<'a>(p: &'a L3RecordTypeProjection, id: &str) -> &'a PTableRecordTypes {
    p.tables
        .iter()
        .find(|t| t.stable_table_id == id)
        .unwrap_or_else(|| panic!("table {id} present in cross-app projection"))
}

// ============================================================================
// 1. A record VAR typed as a SPECIFIC dep table binds to the EXACT dep
//    StableTableId — NOT the other dep table (Rev 2 #1; ≥2 dep tables).
// ============================================================================

#[test]
fn dep_table_typed_record_var_binds_to_exact_dep_stable_table_id() {
    let p = project();
    // `var Cust: Record "Dep Customer"` in the ws subscriber HandleOnBeforeCompute.
    let sub = routine_by_name(&p, "HandleOnBeforeCompute");
    let cust = sub
        .record_variables
        .iter()
        .find(|v| v.name == "cust")
        .expect("cust record var present");

    // EXACT dep StableTableId (Dep Customer 50000) — the POST-resolve backfilled
    // value (the dep table enters the symbol table via withDependencyArtifacts).
    assert_eq!(
        cust.table_id.as_deref(),
        Some(dep_customer().as_str()),
        "a `Record \"Dep Customer\"` var binds to the EXACT dep StableTableId (50000)",
    );
    // NOT the OTHER dep table (50001) — a wrong-but-same binding would fail here.
    assert_ne!(
        cust.table_id.as_deref(),
        Some(dep_vendor().as_str()),
        "must NOT bind to Dep Vendor (50001) — ≥2 dep tables make a wrong binding detectable",
    );
}

// ============================================================================
// 2. A record OP on a dep-table var carries the EXACT dep StableTableId.
// ============================================================================

#[test]
fn dep_table_record_ops_carry_exact_dep_stable_table_id() {
    let p = project();
    // `cust.SetRange(...)` / `cust.FindFirst()` in DriveDeps.
    let drive = routine_by_name(&p, "DriveDeps");
    let dep_ops: Vec<_> = drive
        .record_operations
        .iter()
        .filter(|o| o.record_variable_name == "cust")
        .collect();
    assert!(
        dep_ops.len() >= 2,
        "≥2 ops on the dep-table var `cust` (SetRange + FindFirst)",
    );
    for op in &dep_ops {
        assert_eq!(
            op.table_id.as_deref(),
            Some(dep_customer().as_str()),
            "op {} on `cust` carries the EXACT dep StableTableId (Dep Customer 50000)",
            op.operation_id,
        );
    }
}

// ============================================================================
// 3. A field visible ONLY via a DEP TableExtension merge resolves to the EXACT
//    StableFieldId (Dep Vendor 50001 field 50 "Rating" via dep ext 50700).
// ============================================================================

#[test]
fn dep_extension_merged_field_resolves_to_exact_stable_field_id() {
    let p = project();
    let vendor = table_by_stable_id(&p, &dep_vendor());
    let rating = vendor
        .fields
        .iter()
        .find(|f| f.field_number == 50)
        .expect("Dep Vendor field 50 present (merged via dep TableExtension)");

    assert_eq!(
        rating.name, "Rating",
        "the merged dep-ext field is `Rating`"
    );
    // The field lives on Dep Vendor (50001) but its DECLARING provenance is the dep
    // TableExtension (50700) — only visible via the dep-extension merge. This is the
    // EXACT StableFieldId surface (physical table 50001 + declaring ext 50700).
    assert_eq!(
        rating.declaring_object_id,
        dep_vendor_ext(),
        "the merged field's declaring provenance is the EXACT dep TableExtension (50700)",
    );
    // It is genuinely a cross-app dep-extension field (not a base-table field).
    assert!(
        rating
            .declaring_object_id
            .starts_with(&format!("{DEP_CORE}:")),
        "the declaring TableExtension is dep-owned (cross-app merge)",
    );
}

// ============================================================================
// 4. The OTHER merge direction: a WORKSPACE TableExtension field merges onto a
//    DEP base table (Dep Customer 50000 gains "Loyalty Points" 70010 from ws ext).
// ============================================================================

#[test]
fn workspace_extension_field_merges_onto_dep_base_table() {
    let p = project();
    let customer = table_by_stable_id(&p, &dep_customer());
    let loyalty = customer
        .fields
        .iter()
        .find(|f| f.field_number == 70010)
        .expect("Dep Customer field 70010 present (merged via ws TableExtension)");

    assert_eq!(loyalty.name, "Loyalty Points");
    // The ws extension's field on the DEP base table — declaring provenance is the
    // WORKSPACE TableExtension (70010), crossing the app boundary the other way.
    assert_eq!(
        loyalty.declaring_object_id,
        ws_cust_ext(),
        "the ws-extension field on the dep base table keeps the WS TableExtension provenance",
    );
    assert!(
        loyalty
            .declaring_object_id
            .starts_with(&format!("{WS_APP}:")),
        "the declaring TableExtension is workspace-owned (cross-app merge, other direction)",
    );
}

// ============================================================================
// 5. ≥2 dep tables are present so a wrong-but-same binding is detectable (the
//    structural precondition for the SPECIFIC-id oracles above, Rev 2 #1).
// ============================================================================

#[test]
fn corpus_has_at_least_two_dep_tables() {
    let p = project();
    let dep_table_count = p
        .tables
        .iter()
        .filter(|t| t.stable_table_id == dep_customer() || t.stable_table_id == dep_vendor())
        .count();
    assert_eq!(
        dep_table_count, 2,
        "the corpus carries ≥2 dep tables (Dep Customer 50000 + Dep Vendor 50001)",
    );
}
