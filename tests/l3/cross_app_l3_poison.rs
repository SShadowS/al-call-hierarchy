//! R2.5b Task 1 — L4/cone/summary LEAKAGE POISON negative test (Rev 2 #5).
//!
//! The L3 input boundary is an L3-ONLY merged index. The dep side is produced by
//! `parse_symbol_reference → project_abi_to_index`, whose output structs
//! (`ProjectedObject`/`ProjectedTable`/`ProjectedRoutine`) STRUCTURALLY carry NO L4
//! field — no `summary`, `intraAppCallEdges`, `citedOperationEvidence`,
//! `depOrderIndex`, capability-cone, or typed-edge field. So an L3 input that
//! "carries" such fields cannot influence L3: there is nowhere for them to live.
//!
//! This test PROVES it executably: a SymbolReference.json POISONED with bogus
//! L4/cone/summary keys at EVERY level (top-level, per-object, per-routine,
//! per-table, per-field) must yield a BYTE-IDENTICAL projected merged-index (the L3
//! input) vs the clean one. If the projection changed, L3 would be reading
//! out-of-scope state. The parser uses lenient `serde_json::Value` navigation, so
//! unknown keys are dropped — exactly the boundary guarantee, enforced end-to-end.

use al_call_hierarchy::engine::deps::projection::project_abi_to_index;
use al_call_hierarchy::engine::deps::symbol_reference::parse_symbol_reference;

const APP_GUID: &str = "dddddddd-0000-0000-0000-000000000001";
const MODEL_INSTANCE_ID: &str = "r2.5b";

/// A clean SymbolReference.json with a table (+ field), a codeunit with a routine.
fn clean_symbol_reference() -> serde_json::Value {
    serde_json::json!({
        "RuntimeVersion": "16.0",
        "Tables": [{
            "Id": 50000,
            "Name": "Dep Customer",
            "Fields": [{ "Id": 1, "Name": "No.", "TypeDefinition": { "Name": "Code[20]" } }],
            "Keys": [{ "Name": "PK", "FieldNames": ["No."] }],
            "Methods": []
        }],
        "Codeunits": [{
            "Id": 50100,
            "Name": "Dep Mgt",
            "Methods": [{
                "Name": "Compute",
                "Parameters": [{ "Name": "qty", "TypeDefinition": { "Name": "Integer" } }],
                "ReturnTypeDefinition": { "Name": "Decimal" }
            }]
        }]
    })
}

/// The same SymbolReference, POISONED with bogus L4/cone/summary keys at every
/// level. None of these is an L3 field; all must be silently dropped.
fn poisoned_symbol_reference() -> serde_json::Value {
    let bogus = serde_json::json!({
        "summary": { "returnability": "returns", "bogus": true },
        "intraAppCallEdges": [{ "from": "x", "to": "y" }],
        "citedOperationEvidence": [{ "operationId": "fake", "witness": "z" }],
        "depOrderIndex": 42,
        "cone": { "capabilityFactsDirect": ["WRITES"], "inherited": ["COMMIT"] },
        "typedEdge": { "kind": "call", "resourceId": "poison" },
        "capabilityFactsDirect": ["IO_BEFORE_ESCAPING_ERROR"]
    });
    serde_json::json!({
        "RuntimeVersion": "16.0",
        // top-level poison.
        "summary": bogus,
        "intraAppCallEdges": [{ "from": "top", "to": "level" }],
        "depOrderIndex": 99,
        "Tables": [{
            "Id": 50000,
            "Name": "Dep Customer",
            // per-table poison.
            "summary": bogus,
            "cone": bogus,
            "Fields": [{
                "Id": 1,
                "Name": "No.",
                "TypeDefinition": { "Name": "Code[20]" },
                // per-field poison.
                "citedOperationEvidence": bogus,
                "capabilityFactsDirect": ["READS"]
            }],
            "Keys": [{ "Name": "PK", "FieldNames": ["No."] }],
            "Methods": []
        }],
        "Codeunits": [{
            "Id": 50100,
            "Name": "Dep Mgt",
            // per-object poison.
            "summary": bogus,
            "typedEdge": bogus,
            "Methods": [{
                "Name": "Compute",
                "Parameters": [{ "Name": "qty", "TypeDefinition": { "Name": "Integer" } }],
                "ReturnTypeDefinition": { "Name": "Decimal" },
                // per-routine poison.
                "summary": bogus,
                "intraAppCallEdges": [{ "from": "r", "to": "s" }],
                "cone": bogus,
                "capabilityFactsDirect": ["WRITES"]
            }]
        }]
    })
}

/// Project a SymbolReference JSON to the dep merged-index entities, then serialize
/// the L3-relevant input fields deterministically for a byte comparison. We compare
/// the Debug form of the projected entities (ProjectedObject/Table/Routine) — these
/// ARE the L3 input; if a poison key leaked into any field it would show here.
fn project_and_serialize(symref: &serde_json::Value) -> String {
    let abi = parse_symbol_reference(&symref.to_string());
    let projected = project_abi_to_index(&abi, APP_GUID, MODEL_INSTANCE_ID);
    // Deterministic, total serialization of the projected entities (the L3 input).
    format!("{:#?}", projected)
}

#[test]
fn poisoned_symbol_reference_yields_byte_identical_l3_input() {
    let clean = project_and_serialize(&clean_symbol_reference());
    let poisoned = project_and_serialize(&poisoned_symbol_reference());
    assert_eq!(
        clean, poisoned,
        "L4/cone/summary keys in the SymbolReference MUST NOT change the projected L3 \
         input — the boundary is L3-only (Rev 2 #5)"
    );
    // Sanity: the projection is non-trivial (a real object + routine + table), so the
    // equality is meaningful, not two empty projections.
    assert!(clean.contains("Compute"), "projection carries the routine");
    assert!(
        clean.contains("Dep Customer"),
        "projection carries the table"
    );
    // And NONE of the poison tokens survived into the L3 input.
    for token in [
        "intraAppCallEdges",
        "citedOperationEvidence",
        "depOrderIndex",
        "capabilityFactsDirect",
        "typedEdge",
        "IO_BEFORE_ESCAPING_ERROR",
        "poison",
    ] {
        assert!(
            !clean.contains(token),
            "no L4 token `{token}` in the L3 input"
        );
    }
}
