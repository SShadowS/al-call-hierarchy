//! E1 — L3 enclosing-member / originating-object / wrapper-range capture.
//!
//! These tests exercise the additive `L3Routine` fields populated at L3 assembly
//! (`l3_workspace.rs`). They are Rust-only model assertions — `L3Routine` is NOT
//! `Serialize`-derived, so these fields never reach an R0–R3 golden; the parity
//! contract is guarded by the FULL differential suite, not here.
//!
//! Coverage (spec Revision-2 clauses):
//!   (a) RE-9 — routine set/order INVARIANT: the `(id, source_anchor.start_line)`
//!       sequence on a multi-trigger fixture matches a frozen expectation, proving the
//!       `collect_routine_nodes` `(parent, routine)` change did not perturb traversal.
//!   (b) RE-1/RE-2 — a two-field-`OnValidate` table → two routines with the SAME
//!       `stable_routine_id` but DISTINCT `enclosing_member` + DISTINCT wrapper ranges.
//!   (c) RE-3 — a `report_dataitem` `OnAfterGetRecord` → member = the dataitem name.
//!   (d) RE-4 — an escaped-quote / mixed-case field name → the unescaped logical name.
//!   (e) a true object-level trigger (`OnRun`) → `enclosing_member` is `None`.

use al_call_hierarchy::engine::l3::l3_workspace::{assemble_workspace, L3Routine, L3Workspace};

const APP_GUID: &str = "11111111-1111-1111-1111-111111111111";

fn assemble(files: &[(&str, &str)]) -> L3Workspace {
    let owned: Vec<(String, String)> = files
        .iter()
        .map(|(n, s)| ((*n).to_string(), (*s).to_string()))
        .collect();
    assemble_workspace(&owned, APP_GUID, "r0")
}

fn find<'a>(ws: &'a L3Workspace, name: &str) -> Vec<&'a L3Routine> {
    ws.routines.iter().filter(|r| r.name == name).collect()
}

// ---------------------------------------------------------------------------
// (a) Routine set/order invariant (RE-9).
// ---------------------------------------------------------------------------

const MULTI_TRIGGER_TABLE: &str = r#"
table 50100 "Multi Trigger"
{
    fields
    {
        field(1; "First Field"; Integer)
        {
            trigger OnValidate()
            begin
            end;
        }
        field(2; "Second Field"; Integer)
        {
            trigger OnValidate()
            begin
            end;
        }
    }

    trigger OnInsert()
    begin
    end;

    procedure DoStuff()
    begin
    end;
}
"#;

#[test]
fn routine_set_and_order_invariant() {
    let ws = assemble(&[("multi.al", MULTI_TRIGGER_TABLE)]);

    // Four routines: two field OnValidate triggers, the OnInsert object trigger,
    // and the DoStuff procedure — in document (traversal) order.
    let seq: Vec<(String, u32)> = ws
        .routines
        .iter()
        .map(|r| (r.name.clone(), r.source_anchor.start_line))
        .collect();

    // Frozen expectation. start_line is 0-based (tree-sitter rows). The leading newline
    // in the raw string makes `table` line 1; the triggers/procedure follow in source
    // order. If this sequence moves, the collect_routine_nodes change perturbed traversal.
    let expected = vec![
        ("OnValidate".to_string(), 7u32),
        ("OnValidate".to_string(), 13u32),
        ("OnInsert".to_string(), 19u32),
        ("DoStuff".to_string(), 23u32),
    ];
    assert_eq!(
        seq, expected,
        "routine (name, start_line) sequence must be unchanged by the (parent, routine) collect change"
    );
    assert_eq!(ws.routines.len(), 4, "exactly four routines expected");
}

// ---------------------------------------------------------------------------
// (b) Two-field OnValidate: same stable_routine_id, distinct member + range (RE-1/RE-2).
// ---------------------------------------------------------------------------

#[test]
fn two_field_on_validate_distinct_member_same_stable_id() {
    let ws = assemble(&[("multi.al", MULTI_TRIGGER_TABLE)]);

    let validates = find(&ws, "OnValidate");
    assert_eq!(validates.len(), 2, "two OnValidate triggers expected");

    // Same StableRoutineId (the collapse the discriminator must work around).
    assert_eq!(
        validates[0].stable_routine_id, validates[1].stable_routine_id,
        "both field OnValidate triggers collapse to the SAME stable_routine_id"
    );

    // Distinct enclosing members (the unescaped logical field names).
    let members: Vec<&str> = validates
        .iter()
        .map(|r| r.enclosing_member.as_deref().expect("member present"))
        .collect();
    assert!(
        members.contains(&"First Field") && members.contains(&"Second Field"),
        "members must be the two field names, got {members:?}"
    );
    assert_ne!(members[0], members[1], "members must be distinct");

    // Distinct wrapper ranges (the position discriminator boundary).
    let r0 = validates[0]
        .enclosing_member_range
        .as_ref()
        .expect("wrapper range present");
    let r1 = validates[1]
        .enclosing_member_range
        .as_ref()
        .expect("wrapper range present");
    assert_ne!(
        (r0.start_line, r0.end_line),
        (r1.start_line, r1.end_line),
        "the two field wrappers occupy distinct source ranges"
    );

    // originating_object = the StableObjectId of the declaring table, identical for both.
    assert!(validates[0].originating_object.is_some());
    assert_eq!(
        validates[0].originating_object, validates[1].originating_object,
        "originating_object is the declaring object (same table)"
    );
}

// ---------------------------------------------------------------------------
// (c) report_dataitem OnAfterGetRecord → member = dataitem name (RE-3).
// ---------------------------------------------------------------------------

const REPORT_DATAITEM: &str = r#"
report 50101 "Cust Report"
{
    dataset
    {
        dataitem(Customer; Customer)
        {
            trigger OnAfterGetRecord()
            begin
            end;
        }
    }
}
"#;

#[test]
fn report_dataitem_member_is_dataitem_name() {
    let ws = assemble(&[("rep.al", REPORT_DATAITEM)]);
    let r = find(&ws, "OnAfterGetRecord");
    assert_eq!(r.len(), 1, "one OnAfterGetRecord trigger expected");
    assert_eq!(
        r[0].enclosing_member.as_deref(),
        Some("Customer"),
        "report dataitem member = the dataitem name"
    );
    assert!(r[0].enclosing_member_range.is_some());
    assert!(r[0].originating_object.is_some());
}

// ---------------------------------------------------------------------------
// (d) Escaped-quote / mixed-case field name → unescaped logical name (RE-4).
// ---------------------------------------------------------------------------

const ESCAPED_QUOTE_FIELD: &str = r#"
table 50102 "Quote Table"
{
    fields
    {
        field(1; "Sell-to ""Custom"" No."; Code[20])
        {
            trigger OnValidate()
            begin
            end;
        }
    }
}
"#;

#[test]
fn escaped_quote_member_is_unescaped_logical_name() {
    let ws = assemble(&[("q.al", ESCAPED_QUOTE_FIELD)]);
    let r = find(&ws, "OnValidate");
    assert_eq!(r.len(), 1);
    // strip_quotes trims the boundary quotes; unescape_al_identifier collapses the
    // inner "" → ". The logical name matches the profiler display form.
    assert_eq!(
        r[0].enclosing_member.as_deref(),
        Some(r#"Sell-to "Custom" No."#),
        "member must be the unescaped logical identifier"
    );
}

// ---------------------------------------------------------------------------
// (e) Object-level trigger (OnRun) → member is None.
// ---------------------------------------------------------------------------

const OBJECT_LEVEL_TRIGGER: &str = r#"
codeunit 50103 "Runner"
{
    trigger OnRun()
    begin
    end;

    procedure Helper()
    begin
    end;
}
"#;

#[test]
fn object_level_trigger_has_no_member() {
    let ws = assemble(&[("cu.al", OBJECT_LEVEL_TRIGGER)]);

    let onrun = find(&ws, "OnRun");
    assert_eq!(onrun.len(), 1);
    assert_eq!(
        onrun[0].enclosing_member, None,
        "object-level OnRun has no enclosing member"
    );
    assert_eq!(onrun[0].enclosing_member_range, None);
    assert_eq!(onrun[0].originating_object, None);

    // A plain procedure likewise has no member.
    let helper = find(&ws, "Helper");
    assert_eq!(helper.len(), 1);
    assert_eq!(helper[0].enclosing_member, None);
}
