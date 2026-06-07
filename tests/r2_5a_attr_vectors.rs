//! R2.5a ABI-vs-native ATTRIBUTE classification — Rust replay of al-sem Task 1's
//! `abi-native-attr-vectors.json`.
//!
//! Source of truth: `tests/r2-5a-vectors/abi-native-attr-vectors.json`. al-sem
//! already proved native AST `attributeInfoFromNode` == ABI
//! `classifyAbiArg`/`abiAttributeInfo` over the same `.app`; THIS test proves the
//! Rust ABI classifier reproduces the recorded `AttributeInfo` arg shape
//! (`{kind,text,value,qualifier,member}`) and the derived routine `kind` +
//! `eventKind` byte-for-byte.
//!
//! Reconstruction: each row records the EXPECTED `abiAttributesParsed`. We rebuild
//! the raw `SymbolReference.json` `Attributes[].Arguments[].Value` inputs that the
//! ABI classifier consumes — booleans as JSON primitives (the note: "ABI stores
//! booleans as JSON primitives"), everything else as the already-tokenized AL
//! `text` string — then run the FULL `parse_symbol_reference` path and assert.

use al_call_hierarchy::engine::deps::symbol_reference::{parse_symbol_reference, AbiEventKind};
use serde_json::{json, Value};

/// Reconstruct the raw `Arguments[].Value` JSON for one expected arg. A boolean
/// arg becomes a JSON bool primitive; every other kind becomes the recorded
/// `text` string (which is what `SymbolReference.json` carries for tokenized AL
/// argument values).
fn raw_arg_value(arg: &Value) -> Value {
    let kind = arg["kind"].as_str().unwrap();
    let text = arg["text"].as_str().unwrap();
    if kind == "boolean" {
        match text {
            "true" => json!(true),
            "false" => json!(false),
            _ => json!(text),
        }
    } else {
        json!(text)
    }
}

/// Build a `SymbolReference.json` string with one Codeunit method per row,
/// carrying that row's reconstructed `Attributes`.
fn build_symbol_reference_json(rows: &[Value], app_guid: &str) -> String {
    let mut methods = Vec::new();
    for row in rows {
        let name = row["routineName"].as_str().unwrap();
        let attrs_in = row["abiAttributesParsed"].as_array().unwrap();
        let attributes: Vec<Value> = attrs_in
            .iter()
            .map(|a| {
                let attr_name = a["name"].as_str().unwrap();
                let args: Vec<Value> = a["args"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|arg| json!({ "Value": raw_arg_value(arg) }))
                    .collect();
                json!({ "Name": attr_name, "Arguments": args })
            })
            .collect();
        methods.push(json!({
            "Name": name,
            "Parameters": [],
            "Attributes": attributes,
        }));
    }
    let doc = json!({
        "AppId": app_guid,
        "Name": "AttrTest",
        "Publisher": "Test",
        "Version": "1.0.0.0",
        "Codeunits": [
            { "Id": 50100, "Name": "AttrCodeunit", "Methods": methods }
        ],
    });
    serde_json::to_string(&doc).unwrap()
}

#[test]
fn abi_native_attr_vectors_replay_byte_for_byte() {
    let raw = include_str!("r2-5a-vectors/abi-native-attr-vectors.json");
    let doc: Value = serde_json::from_str(raw).expect("attr vectors must parse");

    let app_guid = doc["appGuid"].as_str().unwrap();
    let rows = doc["rows"].as_array().expect("rows array");
    let declared = doc["count"].as_u64().expect("count") as usize;
    assert_eq!(declared, rows.len(), "declared count != actual rows");

    let json = build_symbol_reference_json(rows, app_guid);
    let abi = parse_symbol_reference(&json);
    assert!(abi.error.is_none(), "parse error: {:?}", abi.error);
    assert_eq!(abi.objects.len(), 1, "one codeunit expected");
    let cu = &abi.objects[0];
    assert_eq!(cu.routines.len(), rows.len(), "one routine per row");

    let mut failures = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        let routine = &cu.routines[i];
        let rn = row["routineName"].as_str().unwrap();
        let mut row_errs = Vec::new();

        // --- derived routine kind + eventKind ---
        let exp_kind = row["abiKind"].as_str().unwrap();
        if routine.kind != exp_kind {
            row_errs.push(format!(
                "kind: expected {exp_kind:?}, got {:?}",
                routine.kind
            ));
        }
        let exp_event_kind = row["expectedEventKind"].as_str().unwrap();
        let got_event_kind = routine.event_kind.as_str();
        if got_event_kind != exp_event_kind {
            row_errs.push(format!(
                "eventKind: expected {exp_event_kind:?}, got {got_event_kind:?}"
            ));
        }

        // --- per-attribute, per-arg AttributeInfo shape ---
        let exp_attrs = row["abiAttributesParsed"].as_array().unwrap();
        if routine.attributes_parsed.len() != exp_attrs.len() {
            row_errs.push(format!(
                "attr count: expected {}, got {}",
                exp_attrs.len(),
                routine.attributes_parsed.len()
            ));
        } else {
            for (ai, exp_attr) in exp_attrs.iter().enumerate() {
                let got_attr = &routine.attributes_parsed[ai];
                let exp_name = exp_attr["name"].as_str().unwrap();
                if got_attr.name != exp_name {
                    row_errs.push(format!(
                        "attr[{ai}].name: expected {exp_name:?}, got {:?}",
                        got_attr.name
                    ));
                }
                let exp_args = exp_attr["args"].as_array().unwrap();
                if got_attr.args.len() != exp_args.len() {
                    row_errs.push(format!(
                        "attr[{ai}] arg count: expected {}, got {}",
                        exp_args.len(),
                        got_attr.args.len()
                    ));
                    continue;
                }
                for (gi, exp_arg) in exp_args.iter().enumerate() {
                    let got = &got_attr.args[gi];
                    assert_arg_field(
                        &mut row_errs,
                        ai,
                        gi,
                        "kind",
                        exp_arg.get("kind"),
                        Some(&got.kind),
                    );
                    assert_arg_field(
                        &mut row_errs,
                        ai,
                        gi,
                        "text",
                        exp_arg.get("text"),
                        Some(&got.text),
                    );
                    assert_arg_opt(
                        &mut row_errs,
                        ai,
                        gi,
                        "value",
                        exp_arg.get("value"),
                        got.value.as_deref(),
                    );
                    assert_arg_opt(
                        &mut row_errs,
                        ai,
                        gi,
                        "qualifier",
                        exp_arg.get("qualifier"),
                        got.qualifier.as_deref(),
                    );
                    assert_arg_opt(
                        &mut row_errs,
                        ai,
                        gi,
                        "member",
                        exp_arg.get("member"),
                        got.member.as_deref(),
                    );
                }
            }
        }

        if !row_errs.is_empty() {
            failures.push(format!(
                "\n  row #{i} ({rn}):\n    {}",
                row_errs.join("\n    ")
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "{} of {} attr vectors FAILED:{}",
        failures.len(),
        rows.len(),
        failures.join("")
    );
    assert_eq!(rows.len(), 3, "expected 3 committed attr vectors");
}

fn assert_arg_field(
    errs: &mut Vec<String>,
    ai: usize,
    gi: usize,
    field: &str,
    exp: Option<&Value>,
    got: Option<&String>,
) {
    let exp_s = exp.and_then(|v| v.as_str());
    let got_s = got.map(|s| s.as_str());
    if exp_s != got_s {
        errs.push(format!(
            "attr[{ai}].arg[{gi}].{field}: expected {exp_s:?}, got {got_s:?}"
        ));
    }
}

/// For optional fields: the vector OMITS the key when absent (None). Assert the
/// presence/value matches exactly.
fn assert_arg_opt(
    errs: &mut Vec<String>,
    ai: usize,
    gi: usize,
    field: &str,
    exp: Option<&Value>,
    got: Option<&str>,
) {
    let exp_s = exp.and_then(|v| v.as_str());
    if exp_s != got {
        errs.push(format!(
            "attr[{ai}].arg[{gi}].{field}: expected {exp_s:?}, got {got:?}"
        ));
    }
}

#[test]
fn event_kind_enum_string_mapping() {
    // Belt-and-suspenders on the enum→string used in the replay assertions.
    assert_eq!(AbiEventKind::Integration.as_str(), "integration");
    assert_eq!(AbiEventKind::Business.as_str(), "business");
    assert_eq!(AbiEventKind::Unknown.as_str(), "unknown");
}
