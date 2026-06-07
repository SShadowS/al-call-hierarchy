//! R1a Task 3 smoke test: run the workspace-level L2 features emitter
//! (`aldump --l2`) on al-sem's ws-d2 fixture and assert:
//!   1. the output parses as JSON and has the golden top-level shape;
//!   2. a known routine's `features` (operationSite/recordOp + a loop callsite)
//!      match `ws-d2.l2.golden.json` (the golden is ground truth);
//!   3. NO forbidden later-gate / L3-resolved field key appears ANYWHERE in the
//!      output (recursive key scan).
//!
//! Full-corpus comparison is Task 4 — this is just a smoke + forbidden-field
//! guard. If ws-d2's L2 features diverge from the golden, that is a real
//! walker/emitter bug; the fix belongs in `src/engine/l2/**`, not here.

use al_call_hierarchy::engine::l2::l2_workspace::project_workspace;
use std::path::Path;

/// Absolute path to al-sem's ws-d2 fixture (sibling repo).
const WS_D2: &str = r"U:\Git\al-sem\test\fixtures\ws-d2";

/// The committed R1a golden for ws-d2.
const WS_D2_GOLDEN: &str = r"U:\Git\al-sem\scripts\r1a-goldens\ws-d2.l2.golden.json";

/// Keys that must NEVER appear anywhere in the L2 projection (later-gate / L3 —
/// mirrors `scripts/r1a-l2-projection.ts` FORBIDDEN_KEYS + the binding subset).
const FORBIDDEN_KEYS: &[&str] = &[
    "controlContext",
    "order",
    "scopeFrames",
    "capability",
    "resourceId",
    "tableId",
    "calleeParameterIsVar",
    "bindingResolution",
    "sourceTableId",
];

/// Recursively collect every object key in a JSON value.
fn scan_forbidden(value: &serde_json::Value, hits: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(map) => {
            for (k, v) in map {
                if FORBIDDEN_KEYS.contains(&k.as_str()) {
                    hits.push(k.clone());
                }
                scan_forbidden(v, hits);
            }
        }
        serde_json::Value::Array(arr) => {
            for v in arr {
                scan_forbidden(v, hits);
            }
        }
        _ => {}
    }
}

#[test]
fn ws_d2_l2_features_match_golden_and_have_no_forbidden_fields() {
    let ws = Path::new(WS_D2);
    if !ws.is_dir() {
        // The fixture lives in the sibling al-sem repo; skip rather than fail
        // when it is not present on this machine (Task 4 wires the corpus).
        eprintln!("skipping: ws-d2 fixture not found at {WS_D2}");
        return;
    }

    let projection = project_workspace(ws).expect("project_workspace should succeed on ws-d2");

    // (1) Serializes + parses as JSON, with the golden top-level shape.
    let json = serde_json::to_string_pretty(&projection).expect("projection serializes to JSON");
    let parsed: serde_json::Value =
        serde_json::from_str(&json).expect("emitted output parses as JSON");
    assert!(parsed.get("objects").is_some(), "top-level has `objects`");
    assert!(parsed.get("routines").is_some(), "top-level has `routines`");
    assert_eq!(projection.objects.len(), 3, "ws-d2 has 3 objects");
    assert_eq!(projection.routines.len(), 7, "ws-d2 has 7 routines");

    // (3) Forbidden-field guard: recursive key scan over the WHOLE output.
    let mut hits = Vec::new();
    scan_forbidden(&parsed, &mut hits);
    assert!(
        hits.is_empty(),
        "forbidden later-gate/L3 field(s) leaked into the L2 output: {hits:?}"
    );

    // (2) Known-routine feature parity against the golden. When the golden file
    // is present, assert the FULL projection equals it (the strongest smoke).
    let golden_path = Path::new(WS_D2_GOLDEN);
    if golden_path.is_file() {
        let golden_text = std::fs::read_to_string(golden_path).expect("read ws-d2 golden");
        let golden: serde_json::Value =
            serde_json::from_str(&golden_text).expect("golden parses as JSON");
        assert_eq!(
            parsed, golden,
            "ws-d2 L2 projection must match ws-d2.l2.golden.json exactly (golden is ground truth)"
        );
    }

    // Targeted assertions (independent of the golden file's presence on disk) on
    // the known subscriber routine `HandleProcessLine` — a record-op operation
    // site + recordOperation, and the publisher loop callsite shape.
    let handle = projection
        .routines
        .iter()
        .find(|r| r.name == "HandleProcessLine")
        .expect("ws-d2 has HandleProcessLine");
    assert_eq!(handle.kind, "event-subscriber");
    assert_eq!(handle.access_modifier.as_deref(), Some("local"));
    assert_eq!(
        handle.features.operation_sites.len(),
        1,
        "HandleProcessLine has one record-op operation site"
    );
    assert_eq!(handle.features.operation_sites[0].kind, "record-op");
    assert_eq!(
        handle.features.record_operations.len(),
        1,
        "HandleProcessLine has one record operation"
    );
    assert_eq!(handle.features.record_operations[0].op, "FindSet");
    assert_eq!(
        handle.features.record_operations[0].record_variable_name,
        "Customer"
    );

    let raise = projection
        .routines
        .iter()
        .find(|r| r.name == "RaiseInLoop")
        .expect("ws-d2 has RaiseInLoop");
    assert_eq!(raise.kind, "procedure");
    assert_eq!(raise.features.loops.len(), 1, "RaiseInLoop has one loop");
    assert_eq!(raise.features.loops[0].loop_type, "for");
    assert_eq!(
        raise.features.call_sites.len(),
        1,
        "RaiseInLoop has one call site"
    );
    // The callsite is nested in the loop — its loopStack carries the loop id.
    assert_eq!(
        raise.features.call_sites[0].loop_stack,
        vec![raise.features.loops[0].id.clone()],
        "the call site is inside the loop"
    );
}
