//! Task 12 (temp-state-tracking, G6): RecordRef GetTable / OpenTemporary local-only tempState.
//!
//! A `RecordRef` variable's tempState is derivable in two cases:
//!   - `RecRef.Open(no, true)` → Known(true); `Open(no)` or `Open(no, false)` → Known(false).
//!   - `RecRef.GetTable(SomeRec)` → inherits SomeRec's tempState (resolved from the routine's
//!     record_variables by name).
//!
//! These are ONLY applied when the call is in the same routine, unconditional flow (no
//! branching, not inside a loop).  Anything uncertain → Unknown (conservative; fires d1).
//!
//! Tests drive L3 resolution directly and inspect the RecordRef op's temp_state via
//! `first_record_op_temp_known`.  RecordRef ops (FindSet, DeleteAll, …) ARE already captured
//! as record operations because `classify_receiver` returns `ReceiverClass::Record` for a
//! variable whose `declaredType == "RecordRef"`.

use al_call_hierarchy::engine::l3::l3_workspace::assemble_and_resolve_default;

const APP_GUID: &str = "1c000000-0000-0000-0000-0000000c6afe";

fn al(name: &str, body: &str) -> (String, String) {
    (format!("src/{name}.al"), body.to_string())
}

// ---------------------------------------------------------------------------
// (a) GetTable from a temp local → RecRef op resolves Known(true)
// ---------------------------------------------------------------------------

/// `RecRef.GetTable(TempRec)` where `TempRec` is declared `temporary`, then
/// `RecRef.DeleteAll()` in the same routine, unconditional flow.
/// The DeleteAll op must resolve Known(true) via the GetTable propagation.
#[test]
fn gettable_from_temp_local_propagates_known_true() {
    let src = r#"
table 50200 "G6 Item"
{
    fields { field(1; Id; Integer) { } }
    keys { key(PK; Id) { } }
}

codeunit 50200 "G6 GetTable Temp"
{
    procedure ClearTemp()
    var
        TempRec: Record "G6 Item" temporary;
        RecRef: RecordRef;
    begin
        RecRef.GetTable(TempRec);
        RecRef.DeleteAll();
    end;
}
"#;
    let resolved = assemble_and_resolve_default(&[al("G6GetTableTemp", src)], APP_GUID);
    let routine = resolved
        .routine_by_name("ClearTemp")
        .expect("ClearTemp must be resolved");

    assert_eq!(
        routine.first_record_op_temp_known("RecRef"),
        Some(true),
        "RecRef.DeleteAll() after GetTable(TempRec) must resolve Known(true) — \
         GetTable propagates the source record's tempState"
    );
}

/// `RecRef.GetTable(PhysRec)` where `PhysRec` is a plain (non-temp) local.
/// The op must resolve Known(false) — not temp.
#[test]
fn gettable_from_physical_local_propagates_known_false() {
    let src = r#"
table 50201 "G6 Item2"
{
    fields { field(1; Id; Integer) { } }
    keys { key(PK; Id) { } }
}

codeunit 50201 "G6 GetTable Phys"
{
    procedure ClearPhys()
    var
        PhysRec: Record "G6 Item2";
        RecRef: RecordRef;
    begin
        RecRef.GetTable(PhysRec);
        RecRef.DeleteAll();
    end;
}
"#;
    let resolved = assemble_and_resolve_default(&[al("G6GetTablePhys", src)], APP_GUID);
    let routine = resolved
        .routine_by_name("ClearPhys")
        .expect("ClearPhys must be resolved");

    assert_eq!(
        routine.first_record_op_temp_known("RecRef"),
        Some(false),
        "RecRef.DeleteAll() after GetTable(PhysRec) must resolve Known(false) — \
         GetTable propagates the non-temp source record's tempState"
    );
}

// ---------------------------------------------------------------------------
// (b) Open(no, true) → Known(true); plain Open(no) → Known(false)
// ---------------------------------------------------------------------------

/// `RecRef.Open(Database::"G6 Item3", true)` → OpenTemporary form → Known(true).
#[test]
fn open_temporary_true_resolves_known_true() {
    let src = r#"
table 50202 "G6 Item3"
{
    fields { field(1; Id; Integer) { } }
    keys { key(PK; Id) { } }
}

codeunit 50202 "G6 OpenTemp"
{
    procedure DoWork()
    var
        RecRef: RecordRef;
        i: Integer;
    begin
        RecRef.Open(Database::"G6 Item3", true);
        for i := 1 to 5 do
            RecRef.DeleteAll();
    end;
}
"#;
    let resolved = assemble_and_resolve_default(&[al("G6OpenTemp", src)], APP_GUID);
    let routine = resolved
        .routine_by_name("DoWork")
        .expect("DoWork must be resolved");

    assert_eq!(
        routine.first_record_op_temp_known("RecRef"),
        Some(true),
        "RecRef.DeleteAll() after Open(no, true) must resolve Known(true)"
    );
}

/// `RecRef.Open(Database::"G6 Item4")` (single arg — non-temporary form) → Known(false).
#[test]
fn open_no_second_arg_resolves_known_false() {
    let src = r#"
table 50203 "G6 Item4"
{
    fields { field(1; Id; Integer) { } }
    keys { key(PK; Id) { } }
}

codeunit 50203 "G6 OpenPhys"
{
    procedure DoWork()
    var
        RecRef: RecordRef;
        i: Integer;
    begin
        RecRef.Open(Database::"G6 Item4");
        for i := 1 to 5 do
            RecRef.DeleteAll();
    end;
}
"#;
    let resolved = assemble_and_resolve_default(&[al("G6OpenPhys", src)], APP_GUID);
    let routine = resolved
        .routine_by_name("DoWork")
        .expect("DoWork must be resolved");

    assert_eq!(
        routine.first_record_op_temp_known("RecRef"),
        Some(false),
        "RecRef.DeleteAll() after Open(no) with no second arg must resolve Known(false)"
    );
}

/// `RecRef.Open(Database::"G6 Item5", false)` → explicit false → Known(false).
#[test]
fn open_explicit_false_resolves_known_false() {
    let src = r#"
table 50204 "G6 Item5"
{
    fields { field(1; Id; Integer) { } }
    keys { key(PK; Id) { } }
}

codeunit 50204 "G6 OpenExplicitFalse"
{
    procedure DoWork()
    var
        RecRef: RecordRef;
        i: Integer;
    begin
        RecRef.Open(Database::"G6 Item5", false);
        for i := 1 to 5 do
            RecRef.DeleteAll();
    end;
}
"#;
    let resolved = assemble_and_resolve_default(&[al("G6OpenExplicitFalse", src)], APP_GUID);
    let routine = resolved
        .routine_by_name("DoWork")
        .expect("DoWork must be resolved");

    assert_eq!(
        routine.first_record_op_temp_known("RecRef"),
        Some(false),
        "RecRef.DeleteAll() after Open(no, false) must resolve Known(false)"
    );
}

// ---------------------------------------------------------------------------
// (c) Conditional / uncertain cases → Unknown (conservative)
// ---------------------------------------------------------------------------

/// `GetTable` inside an if-conditional → branching detected → Unknown (conservative).
/// The RecRef op must NOT be Known(true) — the engine must fire, not suppress.
#[test]
fn gettable_inside_conditional_stays_unknown() {
    let src = r#"
table 50205 "G6 Item6"
{
    fields { field(1; Id; Integer) { } }
    keys { key(PK; Id) { } }
}

codeunit 50205 "G6 GetTable Cond"
{
    procedure MaybeTemp(SomeFlag: Boolean)
    var
        TempRec: Record "G6 Item6" temporary;
        RecRef: RecordRef;
    begin
        if SomeFlag then
            RecRef.GetTable(TempRec);
        RecRef.DeleteAll();
    end;
}
"#;
    let resolved = assemble_and_resolve_default(&[al("G6GetTableCond", src)], APP_GUID);
    let routine = resolved
        .routine_by_name("MaybeTemp")
        .expect("MaybeTemp must be resolved");

    // The engine is conservative: if the routine has branching, the GetTable
    // derivation is NOT applied → temp_state stays Unknown (None from the accessor).
    assert_eq!(
        routine.first_record_op_temp_known("RecRef"),
        None,
        "RecRef op after a conditional GetTable must stay Unknown (conservative) — \
         branching prevents safe propagation"
    );
}

/// `Open` inside an if-conditional → Unknown (conservative).
#[test]
fn open_inside_conditional_stays_unknown() {
    let src = r#"
table 50206 "G6 Item7"
{
    fields { field(1; Id; Integer) { } }
    keys { key(PK; Id) { } }
}

codeunit 50206 "G6 Open Cond"
{
    procedure MaybeOpen(SomeFlag: Boolean)
    var
        RecRef: RecordRef;
        i: Integer;
    begin
        if SomeFlag then
            RecRef.Open(Database::"G6 Item7", true);
        for i := 1 to 5 do
            RecRef.DeleteAll();
    end;
}
"#;
    let resolved = assemble_and_resolve_default(&[al("G6OpenCond", src)], APP_GUID);
    let routine = resolved
        .routine_by_name("MaybeOpen")
        .expect("MaybeOpen must be resolved");

    assert_eq!(
        routine.first_record_op_temp_known("RecRef"),
        None,
        "RecRef op after a conditional Open must stay Unknown (conservative)"
    );
}
