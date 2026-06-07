//! R2a EXIT GATE — native L3-DIRECT record-type resolution oracle.
//!
//! These are ground-truth-free, STRUCTURAL oracles run NATIVELY against the Rust
//! L3 record-type resolver (`src/engine/l3/**`: workspace assembly →
//! `build_symbol_table → resolve_record_types → merge_extension_fields`). Each
//! invariant drives an inline single-app workspace through the real
//! `assemble_and_resolve_default` entry point and asserts a record-type
//! resolution PROPERTY DIRECTLY on the resolved model — NOT a golden diff against
//! al-sem expected strings (that is `l3rt_vectors.rs` + the differential's
//! `*.l3rt.golden.json`).
//!
//! ## Why an L3-DIRECT oracle (not just the byte-parity differential)
//!
//! The corpus differential (`tests/differential.rs`,
//! `differential_l3_record_types_match_goldens`) is byte-parity with al-sem: if
//! BOTH engines made the same resolution mistake, a pure equality diff would
//! still pass. These oracles assert the record-type CONTRACT in absolute terms —
//! a table name resolves IFF it is in the workspace symbol table, an implicit
//! `Rec` binds to its owning object's effective own table, an explicit local
//! `Rec` is never clobbered, a `temporary` record still resolves, the
//! extension-merge collision is FIRST-wins, lookups are case-insensitive. A bug
//! in `resolve_record_types` / `symbol_table` / `extension_fields` breaks one of
//! the invariants below regardless of whether al-sem shares it. Because the
//! differential IS byte-parity, a FAILURE here that the differential misses would
//! mean BOTH engines are wrong — flag it loudly (it is not "fix the golden").
//!
//! ## Covered (source-only intra-workspace record-type resolution — R2a's guard)
//!   - a record var/op's resolved `tableId` is present IFF the declared table name
//!     resolves in the workspace symbol table (`Record "NoSuchTable"` → ABSENT;
//!     `Record Customer` with Customer in-workspace → present);
//!   - an implicit `Rec` in a Table trigger resolves to THAT table;
//!   - an implicit `Rec` in a Page resolves via its `SourceTable`;
//!   - an implicit `Rec` in a TableExtension resolves via the `extends` chain;
//!   - an explicit local `Rec` variable is NEVER overridden by implicit resolution;
//!   - a `temporary` record still resolves its table;
//!   - the FIRST-wins extension-merge collision (the first extension's field wins
//!     on a duplicate field number);
//!   - case-insensitive resolution (`record customer` ≡ `Record CUSTOMER`).
//!
//! ## Deferred (NOT source-only intra-workspace record-types; later gates)
//!   - CROSS-APP record-types — a `Record` whose table lives in a `.app` symbol
//!     package, not the source workspace. The R2a corpus is source-only (no
//!     `.app` ingestion), so a table absent from the source resolves to ABSENT
//!     here; binding it to the dependency's TableId needs `.app` projection → R2.5.
//!   - CALL graph + EVENT graph resolution (callee binding, dispatch,
//!     `callsiteResolutions`, publisher↔subscriber edges) — R2b / R2c. R2a runs
//!     ONLY the first three resolve sub-steps; calls/events are out of scope.
//!
//! As of this gate every invariant passes with NO `src/engine/l3/**` change
//! required — the resolution was correct.

use al_call_hierarchy::engine::l3::l3_workspace::assemble_and_resolve_default;

const APP_GUID: &str = "2a000000-0000-0000-0000-0000000002aa";

/// StableTableId for an in-workspace table number (`${appGuid}:Table:${n}`).
fn stable_table(number: i64) -> String {
    format!("{APP_GUID}:Table:{number}")
}

/// StableObjectId for an in-workspace object (`${appGuid}:${type}:${n}`).
fn stable_object(object_type: &str, number: i64) -> String {
    format!("{APP_GUID}:{object_type}:{number}")
}

/// Assemble + resolve a single-file inline workspace (`src/main.al`).
fn resolve_one(source: &str) -> al_call_hierarchy::engine::l3::l3_workspace::L3Resolved {
    assemble_and_resolve_default(&[("src/main.al".to_string(), source.to_string())], APP_GUID)
}

// ============================================================================
// 1. Resolved tableId is present IFF the declared table name resolves in the
//    workspace symbol table.
// ============================================================================

#[test]
fn in_workspace_table_resolves_absent_table_omits() {
    // Customer (50900) is in-workspace; "No Such Table" is not.
    let resolved = resolve_one(
        r#"
table 50900 Customer
{
    fields { field(1; "No."; Code[20]) { } }
}

codeunit 50901 "Probe"
{
    procedure DoWork()
    var
        Cust: Record Customer;
        Missing: Record "No Such Table";
    begin
        Cust.FindSet();
        Missing.Get();
    end;
}
"#,
    );

    let routine = resolved
        .routine_by_name("DoWork")
        .expect("DoWork routine resolves");

    // PRESENT: Customer is in the symbol table.
    assert_eq!(
        routine.record_var_table_id("Cust"),
        Some(stable_table(50900)),
        "an in-workspace Record Customer must resolve to its StableTableId",
    );
    // ABSENT: "No Such Table" is not in the symbol table → tableId omitted (None).
    assert_eq!(
        routine.record_var_table_id("Missing"),
        None,
        "a Record whose table is NOT in the workspace must resolve to ABSENT, never guessed",
    );

    // The same IFF holds for the record OPERATIONS derived from those vars.
    let ops = routine.record_ops();
    let cust_op = ops
        .iter()
        .find(|(_, var, _)| var.eq_ignore_ascii_case("cust"))
        .expect("Cust op present");
    assert_eq!(
        cust_op.2,
        Some(stable_table(50900)),
        "the op on the in-workspace Record carries the resolved tableId",
    );
    let missing_op = ops
        .iter()
        .find(|(_, var, _)| var.eq_ignore_ascii_case("missing"))
        .expect("Missing op present");
    assert_eq!(
        missing_op.2, None,
        "the op on the unresolved Record carries NO tableId (absent)",
    );
}

// ============================================================================
// 2. An implicit `Rec` in a Table trigger resolves to THAT table.
// ============================================================================

#[test]
fn implicit_rec_in_table_trigger_resolves_to_that_table() {
    let resolved = resolve_one(
        r#"
table 50900 Customer
{
    fields { field(1; "No."; Code[20]) { } }

    trigger OnInsert()
    begin
        Rec.Modify();
    end;
}
"#,
    );

    let routine = resolved
        .routine_by_name("OnInsert")
        .expect("OnInsert trigger resolves");
    let ops = routine.record_ops();
    let rec_op = ops
        .iter()
        .find(|(_, var, _)| var.eq_ignore_ascii_case("rec"))
        .expect("implicit Rec op present");
    assert_eq!(
        rec_op.2,
        Some(stable_table(50900)),
        "an implicit Rec in a Table trigger resolves to the OWNING table",
    );
}

// ============================================================================
// 3. An implicit `Rec` in a Page resolves via SourceTable.
// ============================================================================

#[test]
fn implicit_rec_in_page_resolves_via_source_table() {
    let resolved = resolve_one(
        r#"
table 50900 Customer
{
    fields { field(1; "No."; Code[20]) { } }
}

page 50901 "Cust Card"
{
    PageType = Card;
    SourceTable = Customer;

    trigger OnOpenPage()
    begin
        Rec.Modify();
    end;
}
"#,
    );

    let routine = resolved
        .routine_by_name("OnOpenPage")
        .expect("OnOpenPage trigger resolves");
    let ops = routine.record_ops();
    let rec_op = ops
        .iter()
        .find(|(_, var, _)| var.eq_ignore_ascii_case("rec"))
        .expect("implicit Rec op present");
    assert_eq!(
        rec_op.2,
        Some(stable_table(50900)),
        "an implicit Rec in a Page resolves via its SourceTable (Customer / 50900)",
    );
}

// ============================================================================
// 4. A TableExtension implicit Rec resolves via the extends chain.
// ============================================================================

#[test]
fn implicit_rec_in_table_extension_resolves_via_extends() {
    let resolved = resolve_one(
        r#"
table 50900 Customer
{
    fields { field(1; "No."; Code[20]) { } }
}

tableextension 50910 "Customer Ext" extends Customer
{
    fields { field(50000; "Loyalty"; Integer) { } }

    trigger OnBeforeModify()
    begin
        Rec.Modify();
    end;
}
"#,
    );

    let routine = resolved
        .routine_by_name("OnBeforeModify")
        .expect("OnBeforeModify trigger resolves");
    let ops = routine.record_ops();
    let rec_op = ops
        .iter()
        .find(|(_, var, _)| var.eq_ignore_ascii_case("rec"))
        .expect("implicit Rec op present");
    assert_eq!(
        rec_op.2,
        Some(stable_table(50900)),
        "an implicit Rec in a TableExtension resolves via its extends target (Customer / 50900)",
    );
}

// ============================================================================
// 5. An explicit local `Rec` variable is NEVER overridden by implicit
//    resolution.
// ============================================================================

#[test]
fn explicit_local_rec_is_never_overridden_by_implicit() {
    // The Table trigger's implicit Rec would resolve to 50900 (Customer), but an
    // EXPLICIT local `Rec: Record Vendor` shadows it → the op binds to Vendor
    // (50901), NOT the owning table.
    let resolved = resolve_one(
        r#"
table 50900 Customer
{
    fields { field(1; "No."; Code[20]) { } }

    trigger OnInsert()
    var
        Rec: Record Vendor;
    begin
        Rec.Modify();
    end;
}

table 50901 Vendor
{
    fields { field(1; "No."; Code[20]) { } }
}
"#,
    );

    let routine = resolved
        .routine_by_name("OnInsert")
        .expect("OnInsert trigger resolves");

    // The explicit local Rec resolves to Vendor (50901).
    assert_eq!(
        routine.record_var_table_id("Rec"),
        Some(stable_table(50901)),
        "an explicit local `Rec: Record Vendor` resolves to Vendor",
    );

    let ops = routine.record_ops();
    let rec_op = ops
        .iter()
        .find(|(_, var, _)| var.eq_ignore_ascii_case("rec"))
        .expect("Rec op present");
    assert_eq!(
        rec_op.2,
        Some(stable_table(50901)),
        "the op binds to the EXPLICIT local Rec (Vendor / 50901), NOT the owning table — \
         implicit resolution must never override an explicit local Rec",
    );
}

// ============================================================================
// 6. A `temporary` record still resolves its table.
// ============================================================================

#[test]
fn temporary_record_still_resolves_table() {
    let resolved = resolve_one(
        r#"
table 50900 Customer
{
    fields { field(1; "No."; Code[20]) { } }
}

codeunit 50901 "Probe"
{
    procedure DoWork()
    var
        TempCust: Record Customer temporary;
    begin
        TempCust.Insert();
    end;
}
"#,
    );

    let routine = resolved
        .routine_by_name("DoWork")
        .expect("DoWork routine resolves");
    assert_eq!(
        routine.record_var_table_id("TempCust"),
        Some(stable_table(50900)),
        "a `Record Customer temporary` still resolves its base table (Customer / 50900)",
    );
    let ops = routine.record_ops();
    let op = ops
        .iter()
        .find(|(_, var, _)| var.eq_ignore_ascii_case("tempcust"))
        .expect("TempCust op present");
    assert_eq!(
        op.2,
        Some(stable_table(50900)),
        "the op on the temporary record carries the resolved tableId",
    );
}

// ============================================================================
// 7. The FIRST-wins extension-merge collision: the first extension's field wins
//    on a duplicate field number.
// ============================================================================

#[test]
fn extension_merge_collision_is_first_wins() {
    // Two TableExtensions both add field 50000 to Customer. In file/object
    // ingestion order Ext A (50910) precedes Ext B (50911), so the merge keeps
    // Ext A's "Loyalty Points" and DROPS Ext B's "Collides 50000". Field 50002
    // (new, from Ext B) DOES merge.
    let resolved = resolve_one(
        r#"
table 50900 Customer
{
    fields
    {
        field(1; "No."; Code[20]) { }
    }
}

tableextension 50910 "Customer Ext A" extends Customer
{
    fields
    {
        field(50000; "Loyalty Points"; Integer) { }
    }
}

tableextension 50911 "Customer Ext B" extends Customer
{
    fields
    {
        field(50000; "Collides 50000"; Integer) { }
        field(50002; "Segment"; Code[10]) { }
    }
}
"#,
    );

    let table = resolved
        .table_by_name("Customer")
        .expect("Customer table resolves");
    let fields = table.merged_fields();

    // Field 50000: FIRST-wins → "Loyalty Points", declared by Ext A (50910).
    let f50000 = fields
        .iter()
        .find(|f| f.field_number == 50000)
        .expect("merged field 50000 present");
    assert_eq!(
        f50000.name, "Loyalty Points",
        "on a duplicate field number the FIRST extension (Ext A) wins — `Collides 50000` is dropped",
    );
    assert_eq!(
        f50000.declaring_object_id,
        stable_object("TableExtension", 50910),
        "the surviving field's provenance is the FIRST extension (Ext A / 50910)",
    );

    // Exactly ONE field numbered 50000 survives (the collision dropped the dupe).
    assert_eq!(
        fields.iter().filter(|f| f.field_number == 50000).count(),
        1,
        "the duplicate field number must collapse to a single merged field",
    );

    // Field 50002 (new, from Ext B) DID merge, with Ext B (50911) provenance.
    let f50002 = fields
        .iter()
        .find(|f| f.field_number == 50002)
        .expect("merged field 50002 present");
    assert_eq!(f50002.name, "Segment");
    assert_eq!(
        f50002.declaring_object_id,
        stable_object("TableExtension", 50911),
        "a NON-colliding extension field merges with its own provenance (Ext B / 50911)",
    );
}

// ============================================================================
// 8. Case-insensitive resolution: `record customer` ≡ `Record CUSTOMER`.
// ============================================================================

#[test]
fn record_type_resolution_is_case_insensitive() {
    // The declared types use mixed/odd casing for both the `Record` keyword and
    // the table name; AL identifiers are case-insensitive, so BOTH resolve to the
    // same Customer (50900).
    let resolved = resolve_one(
        r#"
table 50900 Customer
{
    fields { field(1; "No."; Code[20]) { } }
}

codeunit 50901 "Probe"
{
    procedure DoWork()
    var
        Lower: record customer;
        Upper: Record CUSTOMER;
    begin
        Lower.FindSet();
        Upper.FindSet();
    end;
}
"#,
    );

    let routine = resolved
        .routine_by_name("DoWork")
        .expect("DoWork routine resolves");
    assert_eq!(
        routine.record_var_table_id("Lower"),
        Some(stable_table(50900)),
        "`record customer` (lowercased keyword + name) resolves case-insensitively",
    );
    assert_eq!(
        routine.record_var_table_id("Upper"),
        Some(stable_table(50900)),
        "`Record CUSTOMER` (uppercased name) resolves to the SAME table — case-insensitive",
    );
    assert_eq!(
        routine.record_var_table_id("Lower"),
        routine.record_var_table_id("Upper"),
        "case variants of the same table name must resolve identically",
    );
}
