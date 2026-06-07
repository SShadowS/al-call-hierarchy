//! R2.5b Task 1 — Rust cross-app L3 SMOKE. Proves the merged-index→L3 wiring RUNS
//! end-to-end over the committed `.app`-bearing workspace fixture and produces
//! NON-EMPTY cross-app resolution: a member call resolves to a dep StableRoutineId
//! (the internal + local dep callees resolve IDENTICALLY — no visibility gate); the
//! named transitions are all present; a record var binds to a dep StableTableId;
//! the ws→dep and dep→ws subscriber edges form; `opaqueApps` is NON-empty.
//!
//! This is the engine half of the Task-1 contract; the al-sem half lives in
//! `test/contracts/r2.5b-cross-app-capture.test.ts`. The resolved cross-app edge
//! `to` StableRoutineIds are byte-identical across both sides (R2.5a identity parity
//! carried into the L3 call-graph edge).

use std::path::PathBuf;

use al_call_hierarchy::engine::deps::cross_app_l3::build_cross_app_l3_from_workspace;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/r2-5b-fixtures/cross-app-resolution")
}

const DEP_CORE: &str = "dddddddd-0000-0000-0000-000000000001";
const DEP_OTHER: &str = "eeeeeeee-0000-0000-0000-000000000002";

#[test]
fn cross_app_l3_builds_and_resolves() {
    let cross = build_cross_app_l3_from_workspace(&fixture(), "r2.5b")
        .expect("cross-app L3 builds over the `.app`-bearing workspace");

    // The merged model carries dep entities: ≥2 dep tables AND ≥2 dep routines
    // (Rev 2 #1 — so a wrong-but-same binding is detectable).
    let dep_tables = cross
        .resolved
        .workspace
        .tables
        .iter()
        .filter(|t| t.app_guid == DEP_CORE || t.app_guid == DEP_OTHER)
        .count();
    let dep_routines = cross
        .resolved
        .workspace
        .routines
        .iter()
        .filter(|r| r.app_guid == DEP_CORE || r.app_guid == DEP_OTHER)
        .count();
    assert!(dep_tables >= 2, "≥2 dep tables (got {dep_tables})");
    assert!(dep_routines >= 2, "≥2 dep routines (got {dep_routines})");
}

#[test]
fn member_call_resolves_to_dep_routine_with_named_transitions() {
    let cross = build_cross_app_l3_from_workspace(&fixture(), "r2.5b").unwrap();
    let cg = cross.project_call_graph();

    let mut resolutions: Vec<String> = Vec::new();
    let mut resolved_to_dep = 0usize;
    for g in &cg.groups {
        for e in &g.edges {
            resolutions.push(e.resolution.clone());
            if e.resolution == "resolved" {
                if let Some(to) = &e.to {
                    if to.starts_with(DEP_CORE) || to.starts_with(DEP_OTHER) {
                        resolved_to_dep += 1;
                    }
                }
            }
        }
    }

    // ≥1 edge resolved to a dep StableRoutineId (Rev 2 #4 — genuinely `resolved`).
    assert!(
        resolved_to_dep >= 1,
        "≥1 member call resolved to a dep routine"
    );
    // The Compute/InternalReset/LocalHelper/Apply edges all resolved — internal +
    // local dep callees resolve IDENTICALLY (no L3 visibility gate, Rev 2 #2). The
    // corpus has exactly 4 resolved dep-routine member edges (Apply added in Task 3
    // as the cross-app argumentBindings-upgrade vector, Rev 2 #3).
    assert_eq!(
        resolved_to_dep, 4,
        "Compute + InternalReset + LocalHelper + Apply all resolve (internal/local NOT gated)"
    );

    // Named transitions all present (Rev 2 #4).
    let has = |r: &str| resolutions.iter().any(|x| x == r);
    assert!(has("resolved"), "resolved present");
    assert!(has("member-not-found"), "member-not-found present");
    assert!(
        has("opaque"),
        "opaque present (object-run on unfetched declared dep)"
    );
    assert!(
        has("external-target"),
        "external-target present (member miss, all deps fetched — al-sem parity)"
    );
}

#[test]
fn record_var_binds_to_dep_table_id() {
    let cross = build_cross_app_l3_from_workspace(&fixture(), "r2.5b").unwrap();
    let rt = cross.project_record_types();

    // ≥1 resolved record-op/var tableId points at a dep StableTableId.
    let dep_bound = rt.routines.iter().any(|r| {
        r.record_operations.iter().any(|op| {
            op.table_id
                .as_deref()
                .map(|t| t.starts_with(DEP_CORE))
                .unwrap_or(false)
        }) || r.record_variables.iter().any(|v| {
            v.table_id
                .as_deref()
                .map(|t| t.starts_with(DEP_CORE))
                .unwrap_or(false)
        })
    });
    assert!(dep_bound, "≥1 record op/var bound to a dep StableTableId");

    // The dep base table carries the WORKSPACE-extension field (cross-boundary merge):
    // Dep Customer (50000) gains "Loyalty Points" from the ws TableExtension 70010.
    let dep_customer = rt
        .tables
        .iter()
        .find(|t| t.stable_table_id == format!("{DEP_CORE}:Table:50000"))
        .expect("dep Customer table present");
    assert!(
        dep_customer
            .fields
            .iter()
            .any(|f| f.name == "Loyalty Points"),
        "dep base table carries the ws-extension field (cross-boundary merge)"
    );
}

#[test]
fn event_graph_forms_cross_app_subscriber_edges() {
    let cross = build_cross_app_l3_from_workspace(&fixture(), "r2.5b").unwrap();
    let eg = cross.project_event_graph();

    assert!(eg.edges.len() >= 2, "≥2 subscriber edges");
    // ws→dep: a subscriber edge whose publisher event lives in the dep app.
    let ws_to_dep = eg.edges.iter().any(|e| e.event_id.starts_with(DEP_CORE));
    assert!(
        ws_to_dep,
        "a ws subscriber → dep publisher event edge forms"
    );
    // dep→ws: a subscriber edge whose subscriber routine lives in a dep app. The
    // subscriberRoutineId is a dep StableRoutineId.
    let dep_to_ws = eg
        .edges
        .iter()
        .any(|e| e.subscriber_routine_id.starts_with(DEP_OTHER));
    assert!(
        dep_to_ws,
        "a dep subscriber → ws publisher event edge forms"
    );
}

#[test]
fn coverage_opaque_apps_non_empty_cross_app() {
    let cross = build_cross_app_l3_from_workspace(&fixture(), "r2.5b").unwrap();
    let cov = cross.project_coverage_disk(&fixture());

    // opaqueApps NON-empty = the symbol-only dep appGuids (vs the source-only
    // baseline's empty). Both deps are symbol-only.
    assert!(
        cov.opaque_apps.contains(&DEP_CORE.to_string()),
        "Lib Core in opaqueApps"
    );
    assert!(
        cov.opaque_apps.contains(&DEP_OTHER.to_string()),
        "Lib Ext in opaqueApps"
    );
}
