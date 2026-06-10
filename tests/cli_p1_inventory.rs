//! P1 — `fingerprint --inventory-only` lean projection test.
//!
//! For the `ws-d8-commit-in-tx` in-repo corpus fixture, runs BOTH:
//!   1. The full `fingerprint --format json` (no query flag) → capability-snapshot
//!      envelope.
//!   2. The new `fingerprint --inventory-only --format json` → routine-inventory
//!      envelope.
//!
//! Assertions:
//!   (a) Projection-subset self-consistency: `apps`, `coverage`,
//!       `rootClassifications`, `identities` byte-identical between the two docs.
//!   (b) Heavy keys absent from the inventory doc.
//!   (c) Per-routine inventory: every entry has a non-empty `stableRoutineId` plus
//!       parseable `objectType`, `objectNumber`, `routineName` fields.

use std::path::PathBuf;

use al_call_hierarchy::engine::l5::fingerprint_cli::{
    run_fingerprint_pipeline, FingerprintFormat, FingerprintOptions, FingerprintOutput,
};

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("r0-corpus")
}

const FIXTURE: &str = "ws-d8-commit-in-tx";

/// Build the base `FingerprintOptions` for the fixture.
fn base_opts(ws: &std::path::Path) -> FingerprintOptions {
    FingerprintOptions {
        workspace: ws,
        alsem_version: "p1-test-v1",
        format: FingerprintFormat::Json,
        out: None,
        shard: None,
        witness_limit: None,
        roots: None,
        routine_selectors: Vec::new(),
        include_inherited: true,
        is_query_requested: false, // B0 path (full snapshot envelope)
        deterministic: true,
        strict: false,
        verbosity: "compact",
        inventory_only: false,
    }
}

#[test]
fn inventory_only_projection_subset_self_consistency() {
    let ws = corpus_dir().join(FIXTURE);
    assert!(
        ws.is_dir(),
        "fixture {FIXTURE} not found at {}",
        ws.display()
    );

    // --- 1. Full capability-snapshot envelope ---
    let full_opts = base_opts(&ws);
    let full_result = run_fingerprint_pipeline(&full_opts)
        .unwrap_or_else(|e| panic!("full fingerprint pipeline error: {e}"));
    let full_json = match full_result.output {
        FingerprintOutput::Text(t) => t,
        _ => panic!("expected Text from full fingerprint"),
    };

    // --- 2. Inventory-only envelope ---
    let mut inv_opts = base_opts(&ws);
    inv_opts.inventory_only = true;
    let inv_result = run_fingerprint_pipeline(&inv_opts)
        .unwrap_or_else(|e| panic!("inventory-only fingerprint pipeline error: {e}"));
    let inv_json = match inv_result.output {
        FingerprintOutput::Text(t) => t,
        _ => panic!("expected Text from inventory-only fingerprint"),
    };

    // Parse both as serde_json Values.
    let full_doc: serde_json::Value =
        serde_json::from_str(&full_json).expect("full doc must parse as JSON");
    let inv_doc: serde_json::Value =
        serde_json::from_str(&inv_json).expect("inventory doc must parse as JSON");

    // (a) Envelope header checks: kind must be "routine-inventory", schemaVersion
    //     must be "1.0.0" for the inventory doc.
    assert_eq!(
        inv_doc["kind"].as_str().unwrap_or(""),
        "routine-inventory",
        "inventory doc kind must be 'routine-inventory'"
    );
    assert_eq!(
        inv_doc["schemaVersion"].as_str().unwrap_or(""),
        "1.0.0",
        "inventory doc schemaVersion must be '1.0.0'"
    );

    // (a) Projection-subset self-consistency: apps, coverage, rootClassifications,
    //     identities must be BYTE-IDENTICAL (same sub-values from the same derivers).
    for key in &["apps", "coverage", "rootClassifications", "identities"] {
        let full_val = &full_doc["payload"][key];
        let inv_val = &inv_doc["payload"][key];
        assert_eq!(
            full_val, inv_val,
            "payload.{key} must be byte-identical between full and inventory docs"
        );
    }

    // (b) Heavy keys must be ABSENT from the inventory doc payload.
    let heavy_keys = &[
        "capabilityFacts",
        "typedEdges",
        "operationIndex",
        "callsiteIndex",
        "callsiteResolutions",
        "analysisGaps",
        "inputs",
        "inputsMetadata",
    ];
    let inv_payload = inv_doc["payload"]
        .as_object()
        .expect("inventory payload is object");
    for key in heavy_keys {
        assert!(
            !inv_payload.contains_key(*key),
            "inventory doc payload must NOT contain '{key}'"
        );
    }

    // (c) Per-routine inventory: every entry has non-empty stableRoutineId,
    //     non-empty objectType, a numeric objectNumber, and non-empty routineName.
    let routines_val = &inv_doc["payload"]["routineInventory"];
    let routines = routines_val
        .as_array()
        .expect("payload.routineInventory must be an array");
    assert!(
        !routines.is_empty(),
        "routineInventory must be non-empty for {FIXTURE}"
    );
    for (i, entry) in routines.iter().enumerate() {
        let stable_id = entry["stableRoutineId"]
            .as_str()
            .unwrap_or_else(|| panic!("entry[{i}].stableRoutineId must be a string"));
        assert!(
            !stable_id.is_empty(),
            "entry[{i}].stableRoutineId must not be empty"
        );

        let object_type = entry["objectType"]
            .as_str()
            .unwrap_or_else(|| panic!("entry[{i}].objectType must be a string"));
        assert!(
            !object_type.is_empty(),
            "entry[{i}].objectType must not be empty"
        );

        let object_number = entry["objectNumber"]
            .as_i64()
            .unwrap_or_else(|| panic!("entry[{i}].objectNumber must be an integer"));
        let _ = object_number; // parseable is the only assertion needed

        let routine_name = entry["routineName"]
            .as_str()
            .unwrap_or_else(|| panic!("entry[{i}].routineName must be a string"));
        assert!(
            !routine_name.is_empty(),
            "entry[{i}].routineName must not be empty"
        );
    }

    // (c) Determinism: running inventory-only twice yields identical output.
    let mut inv_opts2 = base_opts(&ws);
    inv_opts2.inventory_only = true;
    let inv_result2 = run_fingerprint_pipeline(&inv_opts2)
        .unwrap_or_else(|e| panic!("second inventory-only run error: {e}"));
    let inv_json2 = match inv_result2.output {
        FingerprintOutput::Text(t) => t,
        _ => panic!("expected Text from second inventory-only fingerprint"),
    };
    assert_eq!(
        inv_json, inv_json2,
        "inventory-only output must be deterministic (two runs must be byte-identical)"
    );
}

/// Exit code must be 0 for a valid workspace.
#[test]
fn inventory_only_exit_code_zero() {
    let ws = corpus_dir().join(FIXTURE);
    assert!(
        ws.is_dir(),
        "fixture {FIXTURE} not found at {}",
        ws.display()
    );
    let mut opts = base_opts(&ws);
    opts.inventory_only = true;
    let result = run_fingerprint_pipeline(&opts).expect("pipeline");
    assert_eq!(
        result.exit_code, 0,
        "inventory-only must exit 0 for valid workspace"
    );
}

/// `--inventory-only` with `--format cbor` must be rejected (json only).
#[test]
fn inventory_only_cbor_rejected() {
    use al_call_hierarchy::engine::l5::fingerprint_cli::{
        default_format, reject_illegal_combos, SpecifiedFlags,
    };
    // Simulate: --inventory-only --format cbor (no query flags).
    let fmt = default_format(Some("cbor"), false).expect("cbor is a valid format");
    let specified = SpecifiedFlags::default();
    // rejectIllegalCombos must not reject for cbor alone (existing behavior),
    // but the CLI layer must reject --inventory-only + cbor.
    // We test this via the CLI combo-validator that will be added.
    let _ = reject_illegal_combos(specified, &fmt, false); // existing path: ok
                                                           // The new rejection is in run_fingerprint_pipeline when inventory_only + non-json.
                                                           // Test via the pipeline directly.
    let ws = corpus_dir().join(FIXTURE);
    assert!(ws.is_dir());
    let opts = FingerprintOptions {
        workspace: &ws,
        alsem_version: "p1-test-v1",
        format: FingerprintFormat::Cbor,
        out: None,
        shard: None,
        witness_limit: None,
        roots: None,
        routine_selectors: Vec::new(),
        include_inherited: true,
        is_query_requested: false,
        deterministic: true,
        strict: false,
        verbosity: "compact",
        inventory_only: true,
    };
    // Must return Err (rejected combo).
    let result = run_fingerprint_pipeline(&opts);
    assert!(
        result.is_err(),
        "--inventory-only + cbor must be rejected by the pipeline"
    );
}
