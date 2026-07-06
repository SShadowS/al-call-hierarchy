//! R0 Task 4 smoke test: run the identity-subset extraction on the vendored
//! ws-d2 fixture and assert the output matches the committed golden's identity
//! subset.
//!
//! Task 3.3 (al-sem parity retirement) vendored the ws-d2 fixture tree and its
//! L3 event-graph golden in-repo (`tests/fixtures/ws-d2/`,
//! `tests/aldump-smoke-goldens/`); this test no longer reads from any al-sem
//! checkout and hard-requires its inputs (no skip-gate).

use al_call_hierarchy::engine::l3::l3_workspace::assemble_and_resolve_workspace_default;
use al_call_hierarchy::engine::snapshot::snapshot_workspace;
use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Vendored ws-d2 fixture (Task 3.3; see `tests/fixtures/ws-d2/PROVENANCE.md`).
fn ws_d2_dir() -> PathBuf {
    repo_root().join("tests").join("fixtures").join("ws-d2")
}

/// In-repo home for the L3 event-graph golden, regenerated from THIS engine
/// (Task 3.3) — Rust-owned baseline, not a copy of al-sem's TS output.
fn l3eg_golden_path() -> PathBuf {
    repo_root()
        .join("tests")
        .join("aldump-smoke-goldens")
        .join("ws-d2.l3eg.golden.json")
}

#[test]
fn ws_d2_identity_subset_matches_golden() {
    let ws = ws_d2_dir();
    assert!(
        ws.is_dir(),
        "vendored ws-d2 fixture missing at {} (Task 3.3 vendoring)",
        ws.display()
    );

    let snap = snapshot_workspace(&ws).expect("snapshot_workspace should succeed on ws-d2");

    // Serializes cleanly as JSON.
    let json = serde_json::to_string_pretty(&snap).expect("snapshot serializes to JSON");
    let _parsed: serde_json::Value =
        serde_json::from_str(&json).expect("emitted output parses as JSON");

    // --- Objects: the three from ws-d2.golden.json with exact ids + fingerprints. ---
    let find_obj = |id: &str| {
        snap.objects
            .iter()
            .find(|o| o.stable_object_id == id)
            .unwrap_or_else(|| panic!("missing object {id}"))
    };

    let pub_obj = find_obj("22222222-d200-0000-0000-000000000002:Codeunit:64101");
    assert_eq!(pub_obj.name, "D2 Publisher");
    assert_eq!(pub_obj.kind, "Codeunit");
    assert_eq!(
        pub_obj.signature_fingerprint,
        "377fb0f90a7fd7704067c8f976cd5436ee1dcb4a57b9cd0acf61cdcaaf7b0c4a"
    );

    let sub_obj = find_obj("22222222-d200-0000-0000-000000000002:Codeunit:64102");
    assert_eq!(sub_obj.name, "D2 Subscriber");
    assert_eq!(sub_obj.kind, "Codeunit");
    assert_eq!(
        sub_obj.signature_fingerprint,
        "bfc4e34885feeb6a82dd67e03cca121cab27224b653eede5ab2160a91b209cd3"
    );

    let cust_obj = find_obj("22222222-d200-0000-0000-000000000002:Table:64100");
    assert_eq!(cust_obj.name, "Customer");
    assert_eq!(cust_obj.kind, "Table");
    assert_eq!(
        cust_obj.signature_fingerprint,
        "c89886eb4c10302d7de10838ebfff1b3c7651f9b409b89ecfd1301f9697a8999"
    );

    // --- Routines: RaiseInLoop (procedure) + OnQuietEvent (event-publisher). ---
    let find_routine = |id: &str| {
        snap.routines
            .iter()
            .find(|r| r.stable_routine_id == id)
            .unwrap_or_else(|| panic!("missing routine {id}"))
    };

    let raise = find_routine(
        "22222222-d200-0000-0000-000000000002:Codeunit:64101#299663ee14d29f43470da2f218237c42dc9923d39062c86dbc2982a454f2e0ac",
    );
    assert_eq!(raise.name, "RaiseInLoop");
    assert_eq!(raise.kind, "procedure");
    assert_eq!(raise.canonical_signature_text, "raiseinloop():");

    let on_quiet = snap
        .routines
        .iter()
        .find(|r| r.name == "OnQuietEvent")
        .expect("missing OnQuietEvent");
    assert_eq!(on_quiet.kind, "event-publisher");
}

/// R2c smoke: the L3 event-graph emitter (`aldump --l3-event-graph`) on ws-d2
/// must match the committed golden EXACTLY. ws-d2 has two integration-event
/// publishers (OnProcessLine / OnQuietEvent) each with one in-workspace
/// subscriber → two `resolved` edges. Guards both the projection shape and that
/// the in-workspace pub+sub pair resolves rather than synthesizing a maybe.
#[test]
fn ws_d2_l3_event_graph_matches_golden() {
    let ws = ws_d2_dir();
    assert!(
        ws.is_dir(),
        "vendored ws-d2 fixture missing at {} (Task 3.3 vendoring)",
        ws.display()
    );
    let resolved = assemble_and_resolve_workspace_default(&ws)
        .expect("ws-d2 assembles + resolves (sound single-app layout)");
    let projection = resolved.project_event_graph();

    // Two integration publishers, two resolved edges (open-world, both in-workspace).
    assert_eq!(projection.events.len(), 2, "ws-d2 has 2 event publishers");
    assert_eq!(projection.edges.len(), 2, "ws-d2 has 2 subscriber edges");
    assert!(
        projection
            .events
            .iter()
            .all(|e| e.event_kind == "integration"),
        "both ws-d2 publishers are integration events"
    );
    assert!(
        projection.edges.iter().all(|e| e.resolution == "resolved"),
        "both ws-d2 subscribers resolve to an indexed publisher"
    );

    let rust = serde_json::to_value(&projection).expect("projection serializes");
    let golden_path = l3eg_golden_path();

    // REGEN path (Task 3.3 vendoring): `REGEN_TEMP_GOLDENS=1` writes the ENGINE
    // projection to the in-repo golden instead of comparing — this is a
    // Rust-owned baseline, not a copy of al-sem's TS output.
    if std::env::var("REGEN_TEMP_GOLDENS").is_ok() {
        let mut pretty = serde_json::to_string_pretty(&projection).expect("regen serialize l3eg");
        pretty.push('\n');
        std::fs::create_dir_all(golden_path.parent().expect("golden has a parent"))
            .expect("create aldump-smoke-goldens dir");
        std::fs::write(&golden_path, pretty)
            .unwrap_or_else(|e| panic!("regen write {}: {e}", golden_path.display()));
        eprintln!("REGEN aldump-smoke l3eg golden: {}", golden_path.display());
        return;
    }

    assert!(
        golden_path.is_file(),
        "missing golden {} (run `REGEN_TEMP_GOLDENS=1 cargo test --test aldump_smoke`)",
        golden_path.display()
    );
    let golden_text = std::fs::read_to_string(&golden_path).expect("read ws-d2 l3eg golden");
    let golden: serde_json::Value =
        serde_json::from_str(&golden_text).expect("golden parses as JSON");
    assert_eq!(
        rust, golden,
        "ws-d2 L3 event-graph projection must match ws-d2.l3eg.golden.json exactly"
    );
}
