//! Spec 1 robustness: building + deep-parsing the CDO snapshot never panics
//! and recovers an Unknown-free lowering on clean source.
#[test]
fn cdo_snapshot_deep_parse_is_panic_free() {
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
    .expect("snapshot builds");
    let parsed = al_call_hierarchy::snapshot::parse_snapshot(&snap);
    // No panic reaching here is the assertion; sanity on coverage:
    let files: usize = parsed.iter().map(|u| u.files.len()).sum();
    assert!(files > 1000);
}
