//! Task 11 (temp-state-tracking, RV-1): the CalcFields/FlowField gate in d1.
//!
//! A TEMPORARY record's FlowField is still computed by evaluating its CalcFormula
//! against the (physical) flow-target tables — a REAL SQL query, host tempness
//! irrelevant. Blob/Normal field loads on a temp record ARE in-memory. So d1's
//! blanket "temp record ⇒ downgrade to info" is WRONG for `CalcFields` /
//! `SetAutoCalcFields` when a FlowField is involved.
//!
//! RV-1 policy: for a `CalcFields`/`SetAutoCalcFields` op on a record d1 resolved
//! to Temporary, downgrade to info ONLY when EVERY named field argument resolves
//! (via the table model) to `field_class != "FlowField"`. If ANY field arg is a
//! FlowField, OR any field arg is unresolvable (name not in table / table not
//! resolved / no field args) ⇒ KEEP FIRING at normal severity with the honest
//! FlowField note.
//!
//! These drive the REAL d1 detector over inline AL workspaces (mirrors
//! `tests/temp_state_d1_path.rs`).

use al_call_hierarchy::engine::l3::l3_workspace::assemble_and_resolve_default;
use al_call_hierarchy::engine::l5::detectors::registered_detectors;
use al_call_hierarchy::engine::l5::finding::Finding;
use al_call_hierarchy::engine::l5::registry::run_detectors;

const APP_GUID: &str = "11111111-0000-0000-0000-0000000d1abc";

/// Run d1 in isolation over an inline workspace and return its emitted findings.
fn run_d1(files: &[(String, String)]) -> Vec<Finding> {
    let resolved = assemble_and_resolve_default(files, APP_GUID);
    let d1: Vec<_> = registered_detectors()
        .into_iter()
        .filter(|d| d.name == "d1-db-op-in-loop")
        .collect();
    assert_eq!(d1.len(), 1, "d1 detector must be registered exactly once");
    run_detectors(&resolved, &d1).findings
}

fn al(name: &str, body: &str) -> (String, String) {
    (format!("src/{name}.al"), body.to_string())
}

const FLOWFIELD_NOTE: &str = "temporary record, but FlowField calculation queries the flow targets";

// --- (a) CalcFields on a Blob field of a temp record → info (in-memory) -------

/// The CDO LoadFiles case: a TEMPORARY record, `CalcFields("File Blob")` where
/// "File Blob" is a Blob field (field_class "Normal", is_blob_like). The Blob load
/// is in-memory, so d1 downgrades to info exactly as before.
#[test]
fn calcfields_blob_on_temp_downgrades_to_info() {
    let src = r#"
table 50121 "CF Files"
{
    fields
    {
        field(1; "No."; Code[20]) { }
        field(2; "File Blob"; Blob) { }
        field(3; "Amount"; Decimal) { FieldClass = FlowField; CalcFormula = sum("CF Ledger".Amount where("File No." = field("No."))); }
    }
    keys { key(PK; "No.") { } }
}

table 50122 "CF Ledger"
{
    fields { field(1; "File No."; Code[20]) { } field(2; Amount; Decimal) { } }
    keys { key(PK; "File No.") { } }
}

codeunit 50121 "CF D1 Blob"
{
    procedure LoadFiles()
    var TempFiles: Record "CF Files" temporary; i: Integer;
    begin
        for i := 1 to 10 do
            TempFiles.CalcFields("File Blob");
    end;
}
"#;
    let findings = run_d1(&[al("CFD1Blob", src)]);
    assert_eq!(
        findings.len(),
        1,
        "one finding expected. findings: {findings:#?}"
    );
    let f = &findings[0];
    assert_eq!(
        f.severity, "info",
        "CalcFields on a Blob (Normal) field of a temp record is in-memory → info. rootCause: {}",
        f.root_cause
    );
    assert!(
        f.root_cause
            .contains("temporary record — not a SQL round-trip"),
        "Blob-field temp CalcFields keeps the in-memory temporary note. rootCause: {}",
        f.root_cause
    );
    assert!(
        !f.root_cause.contains(FLOWFIELD_NOTE),
        "a pure-Normal-field temp CalcFields must NOT carry the FlowField note. rootCause: {}",
        f.root_cause
    );
}

// --- (b) CalcFields on a FlowField of a temp record → keeps firing ------------

/// A TEMPORARY record, `CalcFields("Amount")` where "Amount" is a FlowField. The
/// FlowField is computed against the physical flow targets (a real SQL query), so
/// d1 must KEEP FIRING at normal severity with the FlowField note — the RV-1 fix
/// (was wrongly downgraded to info before).
#[test]
fn calcfields_flowfield_on_temp_keeps_firing() {
    let src = r#"
table 50123 "FF Files"
{
    fields
    {
        field(1; "No."; Code[20]) { }
        field(2; "File Blob"; Blob) { }
        field(3; "Amount"; Decimal) { FieldClass = FlowField; CalcFormula = sum("FF Ledger".Amount where("File No." = field("No."))); }
    }
    keys { key(PK; "No.") { } }
}

table 50124 "FF Ledger"
{
    fields { field(1; "File No."; Code[20]) { } field(2; Amount; Decimal) { } }
    keys { key(PK; "File No.") { } }
}

codeunit 50123 "FF D1 Flow"
{
    procedure SumFiles()
    var TempFiles: Record "FF Files" temporary; i: Integer;
    begin
        for i := 1 to 10 do
            TempFiles.CalcFields("Amount");
    end;
}
"#;
    let findings = run_d1(&[al("FFD1Flow", src)]);
    assert_eq!(
        findings.len(),
        1,
        "one finding expected. findings: {findings:#?}"
    );
    let f = &findings[0];
    assert_ne!(
        f.severity, "info",
        "CalcFields on a FlowField of a temp record queries the flow targets → must fire. \
         rootCause: {}",
        f.root_cause
    );
    assert!(
        f.root_cause.contains(FLOWFIELD_NOTE),
        "FlowField temp CalcFields must carry the honest FlowField note. rootCause: {}",
        f.root_cause
    );
    assert!(
        !f.root_cause.contains("not a SQL round-trip"),
        "FlowField temp CalcFields must NOT carry the in-memory temporary note. rootCause: {}",
        f.root_cause
    );
}

// --- (c) unresolvable field arg → keeps firing (conservative) -----------------

/// A TEMPORARY record CalcFields naming a field NOT in the table. Unresolvable ⇒
/// conservative ⇒ keep firing with the FlowField note.
#[test]
fn calcfields_unresolvable_field_keeps_firing() {
    let src = r#"
table 50125 "UR Files"
{
    fields
    {
        field(1; "No."; Code[20]) { }
        field(2; "File Blob"; Blob) { }
    }
    keys { key(PK; "No.") { } }
}

codeunit 50125 "UR D1 Unres"
{
    procedure LoadFiles()
    var TempFiles: Record "UR Files" temporary; i: Integer;
    begin
        for i := 1 to 10 do
            TempFiles.CalcFields("Nonexistent Field");
    end;
}
"#;
    let findings = run_d1(&[al("URD1Unres", src)]);
    assert_eq!(
        findings.len(),
        1,
        "one finding expected. findings: {findings:#?}"
    );
    let f = &findings[0];
    assert_ne!(
        f.severity, "info",
        "an unresolvable field arg is conservative → keep firing. rootCause: {}",
        f.root_cause
    );
    assert!(
        f.root_cause.contains(FLOWFIELD_NOTE),
        "unresolvable temp CalcFields keeps the FlowField note. rootCause: {}",
        f.root_cause
    );
}

// --- (d) mixed field args: any FlowField → keeps firing -----------------------

/// `CalcFields("File Blob", "Amount")` on a temp record: "File Blob" is Normal but
/// "Amount" is a FlowField. ANY FlowField ⇒ keep firing.
#[test]
fn calcfields_mixed_args_any_flowfield_keeps_firing() {
    let src = r#"
table 50127 "MX Files"
{
    fields
    {
        field(1; "No."; Code[20]) { }
        field(2; "File Blob"; Blob) { }
        field(3; "Amount"; Decimal) { FieldClass = FlowField; CalcFormula = sum("MX Ledger".Amount where("File No." = field("No."))); }
    }
    keys { key(PK; "No.") { } }
}

table 50128 "MX Ledger"
{
    fields { field(1; "File No."; Code[20]) { } field(2; Amount; Decimal) { } }
    keys { key(PK; "File No.") { } }
}

codeunit 50127 "MX D1 Mixed"
{
    procedure LoadFiles()
    var TempFiles: Record "MX Files" temporary; i: Integer;
    begin
        for i := 1 to 10 do
            TempFiles.CalcFields("File Blob", "Amount");
    end;
}
"#;
    let findings = run_d1(&[al("MXD1Mixed", src)]);
    assert_eq!(
        findings.len(),
        1,
        "one finding expected. findings: {findings:#?}"
    );
    let f = &findings[0];
    assert_ne!(
        f.severity, "info",
        "mixed args with ANY FlowField → keep firing. rootCause: {}",
        f.root_cause
    );
    assert!(
        f.root_cause.contains(FLOWFIELD_NOTE),
        "mixed-args temp CalcFields with a FlowField keeps the FlowField note. rootCause: {}",
        f.root_cause
    );
}

// --- (e) non-CalcFields temp op unchanged (regression guard) ------------------

/// A TEMPORARY record DeleteAll inside a loop. The RV-1 gate only touches
/// CalcFields/SetAutoCalcFields; a DeleteAll on a temp record still downgrades to
/// info exactly as Task 10 (regression guard).
#[test]
fn non_calcfields_temp_op_still_downgrades_to_info() {
    let src = r#"
table 50129 "NC Files"
{
    fields { field(1; "No."; Code[20]) { } }
    keys { key(PK; "No.") { } }
}

codeunit 50129 "NC D1 NonCalc"
{
    procedure Purge()
    var TempFiles: Record "NC Files" temporary; i: Integer;
    begin
        for i := 1 to 10 do
            TempFiles.DeleteAll();
    end;
}
"#;
    let findings = run_d1(&[al("NCD1NonCalc", src)]);
    assert_eq!(
        findings.len(),
        1,
        "one finding expected. findings: {findings:#?}"
    );
    let f = &findings[0];
    assert_eq!(
        f.severity, "info",
        "a non-CalcFields temp op still downgrades to info (gate only affects CalcFields). \
         rootCause: {}",
        f.root_cause
    );
    assert!(
        f.root_cause
            .contains("temporary record — not a SQL round-trip"),
        "non-CalcFields temp op keeps the in-memory temporary note. rootCause: {}",
        f.root_cause
    );
    assert!(
        !f.root_cause.contains(FLOWFIELD_NOTE),
        "non-CalcFields temp op must NOT carry the FlowField note. rootCause: {}",
        f.root_cause
    );
}
