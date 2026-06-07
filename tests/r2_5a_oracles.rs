//! R2.5a NATIVE structural oracle — runs against the RUST emitter output (NOT a
//! transitive byte-match against the goldens). Proves the merged-index projection
//! holds the structural invariants the goldens encode, independently of the diff:
//!
//!   1. Every dependency routine: `bodyAvailable:false`, `analysisRole:"dependency"`,
//!      and EMPTY features (the projection structurally omits any feature surface —
//!      asserted by a recursive key scan for L1/L2 feature keys).
//!   2. `signatureFingerprint == sha256Hex(canonicalRoutineSignature(name, params,
//!      returnType))` — recomputed INDEPENDENTLY here from the routine's own
//!      parameters/returnType (a fresh canonical build), not read back from the
//!      projection's own fingerprint field.
//!   3. `accessModifier` matches IsInternal/IsLocal: a routine read from a method
//!      with `IsInternal` → "internal"; `IsLocal` → "local"; neither → omitted.
//!      Cross-checked against the ABI parse of the same `.app`.
//!   4. `sourceKind` matches the manifest `includesSource` flag (re-read here).
//!   5. The `#<guid>#` interface-prefix is stripped from `implementsInterfaces`.
//!   6. The dep TableExtension field appears MERGED into the base table (the
//!      capture-point invariant) — base table carries the extension's field number
//!      under the base table's StableFieldId, AND the extension's own table retains
//!      it under its own id (no double-count).

use std::path::PathBuf;

use al_call_hierarchy::engine::deps::app_manifest::parse_app_manifest_xml;
use al_call_hierarchy::engine::deps::app_package_zip::{
    extract_navx_manifest_xml, extract_symbol_reference_json,
};
use al_call_hierarchy::engine::deps::merged_index::build_merged_index_from_path;
use al_call_hierarchy::engine::deps::symbol_reference::{parse_symbol_reference, AbiObject};
use al_call_hierarchy::engine::ids::{canonical_routine_signature, sha256_hex, ParamSpec};
use serde_json::Value;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn fixtures_dir() -> PathBuf {
    repo_root().join("tests").join("r2-5a-fixtures")
}

/// Every committed fixture dir (each holds its dep `.app`(s)).
fn fixture_dirs() -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(fixtures_dir())
        .expect("read fixtures dir")
        .flatten()
    {
        let p = entry.path();
        if p.is_dir() {
            out.push((entry.file_name().to_string_lossy().to_string(), p));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Collect every `.app` byte-blob under a fixture dir.
fn app_blobs(dir: &PathBuf) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let p = entry.path();
        if p.extension().and_then(|e| e.to_str()) == Some("app") {
            out.push(std::fs::read(&p).unwrap());
        }
    }
    out
}

/// Feature-surface keys that must NEVER appear on a dependency routine (the
/// projection is symbol-only → EMPTY features). A leak means a non-empty feature
/// projection bled in.
const FEATURE_KEYS: &[&str] = &[
    "features",
    "operationSites",
    "recordOperations",
    "callSites",
    "loops",
    "fieldAccesses",
    "statementTree",
    "controlContext",
    "capabilityStatus",
    "recordVariables",
    "variables",
];

fn scan_keys(value: &Value, hits: &mut Vec<String>, path: &str) {
    match value {
        Value::Object(map) => {
            for (k, v) in map {
                if FEATURE_KEYS.contains(&k.as_str()) {
                    hits.push(format!("{path}.{k}"));
                }
                scan_keys(v, hits, &format!("{path}.{k}"));
            }
        }
        Value::Array(arr) => {
            for (i, v) in arr.iter().enumerate() {
                scan_keys(v, hits, &format!("{path}[{i}]"));
            }
        }
        _ => {}
    }
}

#[test]
fn oracle_routines_are_body_unavailable_dependency_empty_features() {
    let mut total = 0usize;
    for (fixture, dir) in fixture_dirs() {
        let proj = build_merged_index_from_path(&dir, "r2.5a");
        let json = serde_json::to_value(&proj).unwrap();
        // No feature surface anywhere in the projection.
        let mut hits = Vec::new();
        scan_keys(&json, &mut hits, &fixture);
        assert!(
            hits.is_empty(),
            "[{fixture}] feature surface leaked into the symbol-only projection: {hits:?}"
        );
        for r in &proj.routines {
            assert!(
                !r.body_available,
                "[{fixture}] {} bodyAvailable must be false",
                r.name
            );
            assert_eq!(
                r.analysis_role, "dependency",
                "[{fixture}] {} analysisRole must be dependency",
                r.name
            );
            total += 1;
        }
    }
    assert!(
        total > 0,
        "no dependency routines observed across the corpus"
    );
    eprintln!("R2.5a oracle: {total} dependency routine(s), all bodyAvailable:false / dependency / empty-features.");
}

#[test]
fn oracle_signature_fingerprint_recomputed_independently() {
    let mut checked = 0usize;
    for (fixture, dir) in fixture_dirs() {
        let proj = build_merged_index_from_path(&dir, "r2.5a");
        for r in &proj.routines {
            // Recompute the canonical signature INDEPENDENTLY from the routine's own
            // parameters + returnType (a fresh build), then hash it.
            let params: Vec<ParamSpec> = r
                .parameters
                .iter()
                .map(|p| ParamSpec {
                    type_text: p.type_text.clone(),
                    is_var: p.is_var,
                })
                .collect();
            let canonical = canonical_routine_signature(&r.name, &params, r.return_type.as_deref());
            let expected = sha256_hex(&canonical);
            assert_eq!(
                r.signature_fingerprint, expected,
                "[{fixture}] {} signatureFingerprint != sha256Hex(canonicalRoutineSignature(...))\n  canonical = {canonical:?}",
                r.name
            );
            // And the StableRoutineId embeds that fingerprint.
            assert!(
                r.stable_routine_id.ends_with(&format!("#{expected}")),
                "[{fixture}] {} StableRoutineId must end with #<fingerprint>",
                r.name
            );
            checked += 1;
        }
    }
    assert!(checked > 0, "no routines checked");
    eprintln!(
        "R2.5a oracle: {checked} routine fingerprint(s) recomputed independently, all match."
    );
}

#[test]
fn oracle_access_modifier_matches_is_internal_is_local() {
    // Cross-check the projected accessModifier against the ABI parse of the same
    // `.app` (IsInternal → internal, IsLocal → local, neither → None).
    let mut checked = 0usize;
    for (fixture, dir) in fixture_dirs() {
        let proj = build_merged_index_from_path(&dir, "r2.5a");
        // Build a (routineName → expected accessModifier) map from every `.app`'s
        // ABI parse. Routine names are unique within these fixtures.
        let mut expected: std::collections::HashMap<String, Option<String>> =
            std::collections::HashMap::new();
        for blob in app_blobs(&dir) {
            let sym = extract_symbol_reference_json(&blob).expect("symbolref present");
            let abi = parse_symbol_reference(&sym);
            for o in &abi.objects {
                record_access(o, &mut expected);
            }
        }
        for r in &proj.routines {
            if let Some(exp) = expected.get(&r.name) {
                assert_eq!(
                    &r.access_modifier, exp,
                    "[{fixture}] {} accessModifier mismatch vs IsInternal/IsLocal",
                    r.name
                );
                checked += 1;
            }
        }
    }
    assert!(checked > 0, "no routines cross-checked for accessModifier");
    eprintln!("R2.5a oracle: {checked} routine accessModifier(s) match IsInternal/IsLocal.");
}

fn record_access(o: &AbiObject, into: &mut std::collections::HashMap<String, Option<String>>) {
    for r in &o.routines {
        let am = if r.is_internal {
            Some("internal".to_string())
        } else if r.is_local {
            Some("local".to_string())
        } else {
            None
        };
        into.insert(r.name.clone(), am);
    }
}

#[test]
fn oracle_source_kind_matches_includes_source() {
    let mut checked = 0usize;
    for (fixture, dir) in fixture_dirs() {
        let proj = build_merged_index_from_path(&dir, "r2.5a");
        // Build appGuid → includesSource by re-reading each manifest.
        let mut inc: std::collections::HashMap<String, bool> = std::collections::HashMap::new();
        for blob in app_blobs(&dir) {
            let xml = extract_navx_manifest_xml(&blob).expect("manifest present");
            let m = parse_app_manifest_xml(&xml);
            inc.insert(m.identity.app_guid.clone(), m.includes_source);
        }
        for a in &proj.apps {
            let want = if *inc.get(&a.app_guid).unwrap_or(&false) {
                "app-source"
            } else {
                "symbol-only"
            };
            assert_eq!(
                a.source_kind, want,
                "[{fixture}] app {} sourceKind must derive from includesSource",
                a.app_guid
            );
            checked += 1;
        }
    }
    assert!(checked > 0, "no apps checked for sourceKind");
    eprintln!("R2.5a oracle: {checked} app sourceKind(s) derive from manifest includesSource.");
}

#[test]
fn oracle_interface_guid_prefix_stripped() {
    // Construct an `.app`-less ABI directly to assert the `#<guid>#` strip on
    // ImplementedInterfaces is applied by the parser the emitter uses, then also
    // confirm the committed corpus carries clean (unprefixed) interface names.
    let json = r##"{
        "AppId": "11111111-2222-3333-4444-555555555555",
        "Codeunits": [
            { "Id": 50100, "Name": "X",
              "ImplementedInterfaces": ["#63ca2fa4#\"Telemetry Logger\"", "\"IPlain\""],
              "Methods": [] }
        ]
    }"##;
    let abi = parse_symbol_reference(json);
    let cu = abi
        .objects
        .iter()
        .find(|o| o.object_type == "Codeunit")
        .unwrap();
    assert_eq!(
        cu.implemented_interfaces.as_deref(),
        Some(["Telemetry Logger".to_string(), "IPlain".to_string()].as_slice()),
        "the #<guid># prefix must be stripped and quotes removed"
    );

    // Corpus check: no projected implementsInterfaces value retains a `#...#` prefix.
    for (fixture, dir) in fixture_dirs() {
        let proj = build_merged_index_from_path(&dir, "r2.5a");
        for o in &proj.objects {
            if let Some(ifaces) = &o.implements_interfaces {
                for i in ifaces {
                    assert!(
                        !i.starts_with('#'),
                        "[{fixture}] {} interface {i:?} still carries a #guid# prefix",
                        o.name
                    );
                }
            }
        }
    }
    eprintln!("R2.5a oracle: #<guid># interface prefix stripped (synthetic + corpus).");
}

#[test]
fn oracle_table_extension_field_merged_into_base() {
    // The capture-point invariant: a dep TableExtension's field is merged into the
    // base table (rekeyed to the base table's StableFieldId), and the extension's
    // own table still retains it under its own id (no double-count).
    let dir = fixtures_dir().join("core-symbol-only");
    assert!(dir.is_dir(), "core-symbol-only fixture present");
    let proj = build_merged_index_from_path(&dir, "r2.5a");

    let base = proj
        .tables
        .iter()
        .find(|t| t.table_number == 50000)
        .expect("base Widget table 50000 present");
    let merged = base
        .fields
        .iter()
        .find(|f| f.field_number == 50)
        .expect("extension field 50 merged into base table");
    assert_eq!(
        merged.stable_field_id, "aaaaaaaa-0000-0000-0000-000000000001:Table:50000#50",
        "merged field's StableFieldId must rekey to the BASE table"
    );
    // 4 own + 1 merged = 5, no double-count of any field number.
    let mut nums: Vec<i64> = base.fields.iter().map(|f| f.field_number).collect();
    nums.sort_unstable();
    assert_eq!(nums, vec![1, 2, 3, 4, 50], "exactly 4 own + 1 merged field");

    let ext = proj
        .tables
        .iter()
        .find(|t| t.table_number == 50700)
        .expect("extension's own table 50700 present");
    let ext_field = ext
        .fields
        .iter()
        .find(|f| f.field_number == 50)
        .expect("extension table retains its own field 50");
    assert_eq!(
        ext_field.stable_field_id, "aaaaaaaa-0000-0000-0000-000000000001:Table:50700#50",
        "extension's own field keeps the extension-table StableFieldId"
    );
    eprintln!("R2.5a oracle: TableExtension field merged into base (capture-point invariant) — no double-count.");
}
