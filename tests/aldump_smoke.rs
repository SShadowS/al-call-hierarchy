//! R0 Task 4 smoke test: run the identity-subset extraction on al-sem's ws-d2
//! fixture and assert the output matches the committed golden's identity subset.
//!
//! The fixture path is referenced directly for now; R0 Task 5 wires the
//! committed goldens into a full differential harness. If the bundled-fork
//! grammar yields a different AST shape than al-sem's WASM grammar, some fields
//! may diverge from the golden — that is expected pre-Task-6 (grammar
//! convergence) and would surface as an assertion failure here.

use al_call_hierarchy::engine::snapshot::snapshot_workspace;
use std::path::Path;

/// Absolute path to al-sem's ws-d2 fixture. Task 5 will replace this with the
/// committed golden corpus.
const WS_D2: &str = r"U:\Git\al-sem\test\fixtures\ws-d2";

#[test]
fn ws_d2_identity_subset_matches_golden() {
    let ws = Path::new(WS_D2);
    if !ws.is_dir() {
        // The fixture lives in the sibling al-sem repo; if it is not present on
        // this machine, skip rather than fail (CI wires the corpus in Task 5).
        eprintln!("skipping: ws-d2 fixture not found at {WS_D2}");
        return;
    }

    let snap = snapshot_workspace(ws).expect("snapshot_workspace should succeed on ws-d2");

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
