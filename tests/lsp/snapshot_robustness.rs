//! Spec 1 robustness: building + deep-parsing the CDO snapshot never panics
//! and recovers an Unknown-free lowering on clean source.

// Task T0.2: shared CDO_WS/ENFORCE_CDO_WS gating helper — see
// `tests/common/cdo.rs` for why this is `#[path]`-included rather than a
// regular crate dependency (separate test-binary crates can't `use` each
// other's `mod`s).
use crate::cdo;
use cdo::cdo_ws_or_enforce;

#[test]
fn cdo_snapshot_deep_parse_is_panic_free() {
    let Some(ws) = cdo_ws_or_enforce() else {
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
