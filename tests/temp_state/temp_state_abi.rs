//! Task 6 (temp-state-tracking, G7 / RV-4): ABI (dependency) side reads the temp
//! markers — `TypeDefinition.Temporary` on record params, `TableType = Temporary`
//! on tables — and the ABI→L3 projection synthesizes per-param `record_variables`
//! with the SAME `Known(true)/ParameterDependent(i)/Known(false)` temp shapes the
//! native source path produces (the native+ABI shape-parity rule).
//!
//! (a)/(b) assert at the `parse_symbol_reference` (AbiTable/AbiParameter) level.
//! (c) asserts at the ABI→L3 projection level: after `project_abi_to_index` +
//! `dep_routine_to_l3`, a record param exposes a record var whose `temp_state`
//! matches the native rule, and a param typed on a `TableType=Temporary` ABI table
//! resolves to `Known(true)` via the table-level override that `resolve()` runs.

use al_call_hierarchy::engine::deps::projection::project_abi_to_index;
use al_call_hierarchy::engine::deps::symbol_reference::parse_symbol_reference;

/// A SymbolReference.json with:
///   - Table 50000 "Temp Buffer" carrying property {"Name":"TableType","Value":"Temporary"}
///   - Table 50001 "Normal Table" with no TableType property
///   - Codeunit 50100 with procedures whose record params carry various temp markers
const SYMBOL_REFERENCE: &str = r#"{
  "AppId": "11111111-2222-3333-4444-555555555555",
  "Name": "TempDep",
  "Publisher": "P",
  "Version": "1.0.0.0",
  "Tables": [
    {
      "Id": 50000,
      "Name": "Temp Buffer",
      "Properties": [ { "Name": "TableType", "Value": "Temporary" } ],
      "Fields": [ { "Id": 1, "Name": "Entry No.", "TypeDefinition": { "Name": "Integer" } } ]
    },
    {
      "Id": 50001,
      "Name": "Normal Table",
      "Fields": [ { "Id": 1, "Name": "Entry No.", "TypeDefinition": { "Name": "Integer" } } ]
    }
  ],
  "Codeunits": [
    {
      "Id": 50100,
      "Name": "Dep Proc",
      "Methods": [
        {
          "Name": "TempMarkedParam",
          "Parameters": [
            { "Name": "Rec", "IsVar": true, "TypeDefinition": { "Name": "Record \"Normal Table\"", "Subtype": { "Name": "Normal Table" }, "Temporary": true } }
          ]
        },
        {
          "Name": "ByVarUnmarked",
          "Parameters": [
            { "Name": "Rec", "IsVar": true, "TypeDefinition": { "Name": "Record \"Normal Table\"" } }
          ]
        },
        {
          "Name": "ByValueUnmarked",
          "Parameters": [
            { "Name": "Rec", "IsVar": false, "TypeDefinition": { "Name": "Record \"Normal Table\"" } }
          ]
        },
        {
          "Name": "TempTableParam",
          "Parameters": [
            { "Name": "Rec", "IsVar": true, "TypeDefinition": { "Name": "Record \"Temp Buffer\"" } }
          ]
        }
      ]
    }
  ]
}"#;

// --- (a) table-level TableType=Temporary -----------------------------------

#[test]
fn abi_table_reads_tabletype_temporary() {
    let abi = parse_symbol_reference(SYMBOL_REFERENCE);
    let temp = abi
        .tables
        .iter()
        .find(|t| t.name == "Temp Buffer")
        .expect("Temp Buffer table");
    assert!(
        temp.is_temporary,
        "Table with TableType=Temporary property → AbiTable.is_temporary == true"
    );
    let normal = abi
        .tables
        .iter()
        .find(|t| t.name == "Normal Table")
        .expect("Normal Table");
    assert!(
        !normal.is_temporary,
        "Table without TableType property → AbiTable.is_temporary == false"
    );
}

// --- (b) param TypeDefinition.Temporary ------------------------------------

#[test]
fn abi_param_reads_typedefinition_temporary() {
    let abi = parse_symbol_reference(SYMBOL_REFERENCE);
    let cu = abi
        .objects
        .iter()
        .find(|o| o.name == "Dep Proc")
        .expect("Dep Proc codeunit");

    let marked = cu
        .routines
        .iter()
        .find(|r| r.name == "TempMarkedParam")
        .expect("TempMarkedParam");
    assert!(
        marked.parameters[0].is_temporary,
        "param with TypeDefinition.Temporary=true → AbiParameter.is_temporary == true"
    );

    let unmarked = cu
        .routines
        .iter()
        .find(|r| r.name == "ByVarUnmarked")
        .expect("ByVarUnmarked");
    assert!(
        !unmarked.parameters[0].is_temporary,
        "param without TypeDefinition.Temporary → AbiParameter.is_temporary == false"
    );
}

// --- (c) ABI→L3 projection per-param record-var temp shapes -----------------

/// Resolve the cross-app L3 for the synthetic dep, then return one routine's
/// record-variables (the projected dep routine after `resolve()` over the merged
/// whole). Uses the public projection + the test-only helper on cross_app_l3.
fn project_and_resolve() -> al_call_hierarchy::engine::l3::l3_workspace::L3Workspace {
    use al_call_hierarchy::engine::deps::cross_app_l3::project_dep_abi_to_l3_for_test;
    let abi = parse_symbol_reference(SYMBOL_REFERENCE);
    let projected = project_abi_to_index(
        &abi,
        "11111111-2222-3333-4444-555555555555",
        "test-instance",
    );
    project_dep_abi_to_l3_for_test(&projected)
}

fn temp_kind(
    ws: &al_call_hierarchy::engine::l3::l3_workspace::L3Workspace,
    routine_name: &str,
) -> (String, Option<bool>, Option<u32>) {
    let r = ws
        .routines
        .iter()
        .find(|r| r.name == routine_name)
        .unwrap_or_else(|| panic!("routine {routine_name}"));
    let rv = r
        .record_variables
        .iter()
        .find(|v| v.is_parameter)
        .unwrap_or_else(|| panic!("record var param on {routine_name}"));
    (
        rv.temp_state.kind.clone(),
        rv.temp_state.value,
        rv.temp_state.parameter_index,
    )
}

#[test]
fn abi_temp_marked_param_projects_known_true() {
    let ws = project_and_resolve();
    let (kind, value, _) = temp_kind(&ws, "TempMarkedParam");
    assert_eq!(kind, "known");
    assert_eq!(
        value,
        Some(true),
        "Temporary:true record param → Known(true)"
    );

    // "Both markers active" path: Temporary:true AND a resolvable table_name (the
    // type text `Record "Normal Table"` a real ABI param carries). The synthesized
    // var must keep its table_name (so the table-level override could also apply),
    // not drop it — covers the realistic shape, not the degenerate `Record`-only one.
    let rv = ws
        .routines
        .iter()
        .find(|r| r.name == "TempMarkedParam")
        .unwrap()
        .record_variables
        .iter()
        .find(|v| v.is_parameter)
        .unwrap();
    assert_eq!(
        rv.table_name.as_deref(),
        Some("Normal Table"),
        "record_table_name_of resolves the param's table from the full type text"
    );
}

#[test]
fn abi_by_var_unmarked_param_projects_parameter_dependent() {
    let ws = project_and_resolve();
    let (kind, _, idx) = temp_kind(&ws, "ByVarUnmarked");
    assert_eq!(
        kind, "parameter-dependent",
        "by-var unmarked record param → ParameterDependent(index)"
    );
    assert_eq!(idx, Some(0), "the param's positional index");
}

#[test]
fn abi_by_value_unmarked_param_projects_known_false() {
    let ws = project_and_resolve();
    let (kind, value, _) = temp_kind(&ws, "ByValueUnmarked");
    assert_eq!(kind, "known");
    assert_eq!(value, Some(false), "by-value record param → Known(false)");
}

#[test]
fn abi_param_typed_on_temp_table_projects_known_true() {
    let ws = project_and_resolve();
    let (kind, value, _) = temp_kind(&ws, "TempTableParam");
    assert_eq!(kind, "known");
    assert_eq!(
        value,
        Some(true),
        "param typed on TableType=Temporary ABI table → Known(true) (table-level override)"
    );
}
