//! Plan 1B.1: building the whole-program node graph over the real CDO snapshot
//! is panic-free and yields a deep, app-qualified node set.
#[test]
fn cdo_program_graph_is_app_qualified_and_panic_free() {
    let Some(ws) = std::env::var_os("CDO_WS")
        .map(std::path::PathBuf::from)
        .filter(|p| p.exists())
    else {
        return;
    };
    let snap = al_call_hierarchy::snapshot::SnapshotBuilder {
        workspace_root: ws,
        local_providers: vec![],
    }
    .build()
    .expect("snapshot");
    let g = al_call_hierarchy::program::build_program_graph(
        &snap,
        &al_call_hierarchy::program::abi_ingest::AbiCache::new(),
    );
    // Print diagnostic counts for the task report.
    let apps: std::collections::BTreeSet<_> = g.objects.iter().map(|o| o.id.app).collect();
    println!(
        "objects={} routines={} apps_spanned={}",
        g.objects.len(),
        g.routines.len(),
        apps.len()
    );
    // Deep node set across workspace + source-bearing deps.
    assert!(g.objects.len() > 500, "objects: {}", g.objects.len());
    assert!(g.routines.len() > 2000, "routines: {}", g.routines.len());
    // App-qualified: nodes span more than one app.
    assert!(
        apps.len() >= 2,
        "nodes should span multiple apps, got {}",
        apps.len()
    );
    // Deterministic: objects sorted by NodeId.
    assert!(
        g.objects.windows(2).all(|w| w[0].id <= w[1].id),
        "objects must be sorted by NodeId"
    );
}
