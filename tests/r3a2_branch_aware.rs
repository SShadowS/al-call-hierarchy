//! R3a-2 Task 2 (FIX) — branch-aware parameterRoles + opaque-guard parity.
//!
//! These tests exercise the COMPOSITION fed into the JACOBI loop on branching /
//! opaque routines — the gaps the prior straight-line port missed:
//!
//!   * FIX 1: a `Validate`/`Modify`/`Insert` INSIDE an `if` yields branch-joined
//!     `dirtyAtExit = unknown` (not `yes`/`no` from a straight-line walk), while
//!     the entry-requirement accumulators (`requiresLoadedAtEntry` /
//!     `mutatesBeforeLoad`) stay `yes` (they only grow).
//!   * FIX 2: a var/var forward to a BODYLESS (`bodyAvailable == false`) callee
//!     joins `unknown` into the exit-effect facts (not `no`).
//!   * FIX 3: a body-available caller with a resolved DIRECT edge to a bodyless
//!     callee gets an `opaque-callee` uncertainty entry.
//!
//! The al-sem goldens (scripts/r3a2-goldens/) already encode the branch-aware
//! behavior; these tests pin the specific facts so the Rust port converges TOWARD
//! them ahead of the Task-3 158-fixture differential.

use al_call_hierarchy::engine::l3::l3_workspace::assemble_and_resolve;
use al_call_hierarchy::engine::l4::summary::project_r3a2;
use serde_json::Value;

const APP_GUID: &str = "dddddddd-dddd-dddd-dddd-dddddddddd01";
const MODEL_INSTANCE_ID: &str = "r0";

fn files(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(n, s)| (n.to_string(), s.to_string()))
        .collect()
}

/// Find the projected summary whose single parameterRole has the given table and
/// return that role object. Panics if not found (test-only).
fn role_of<'a>(summaries: &'a [Value], routine_name_marker: &str) -> &'a Value {
    // We can't address by name (the projection carries stable ids only), so the
    // caller passes a discriminating fact present in dbEffects or readsFields.
    summaries
        .iter()
        .find(|s| {
            serde_json::to_string(s)
                .unwrap()
                .contains(routine_name_marker)
        })
        .unwrap_or_else(|| panic!("no summary containing marker {routine_name_marker}"))
}

// ---------------------------------------------------------------------------
// FIX 1: branch-aware dirtyAtExit.
// ---------------------------------------------------------------------------

/// A routine that `Insert`s a record INSIDE an `if` must have a branch-joined
/// `dirtyAtExit = unknown` (the not-taken else path is pristine), while
/// `requiresLoadedAtEntry`/`mutatesBeforeLoad` stay `yes`. Mirrors al-sem
/// `ws-event-ishandled-conditional-set` golden (DoPost).
#[test]
fn conditional_insert_yields_unknown_dirty_at_exit() {
    let src_publisher = r#"codeunit 50000 PostingMgr
{
    procedure DoPost(var Rec: Record PostingEntry)
    var
        IsHandled: Boolean;
    begin
        IsHandled := false;
        OnBeforePost(Rec, IsHandled);
        if not IsHandled then
            Rec.Insert(true);
    end;

    [IntegrationEvent(false, false)]
    procedure OnBeforePost(var Rec: Record PostingEntry; var IsHandled: Boolean)
    begin
    end;
}
"#;
    let src_table = r#"table 50000 PostingEntry
{
    fields
    {
        field(1; "Entry No."; Integer) { }
        field(2; "No."; Code[20]) { }
        field(3; "Description"; Text[50]) { }
    }

    keys
    {
        key(PK; "Entry No.") { Clustered = true; }
    }
}
"#;
    let resolved = assemble_and_resolve(
        &files(&[("Publisher.al", src_publisher), ("Table.al", src_table)]),
        APP_GUID,
        MODEL_INSTANCE_ID,
    );
    let proj = project_r3a2(&resolved);
    let summaries = serde_json::to_value(&proj.summaries).unwrap();
    let summaries = summaries.as_array().unwrap();

    // DoPost is the only routine with an Insert dbEffect.
    let do_post = role_of(summaries, "\"op\":\"Insert\"");
    let role = &do_post["parameterRoles"][0];

    assert_eq!(
        role["dirtyAtExit"], "unknown",
        "Insert inside `if` must branch-join to dirtyAtExit=unknown (not no/yes); got summary {do_post}"
    );
    assert_eq!(
        role["requiresLoadedAtEntry"], "yes",
        "Insert-before-load on the taken branch sets requiresLoadedAtEntry=yes (accumulator)"
    );
    assert_eq!(
        role["mutatesBeforeLoad"], "yes",
        "Insert-before-load on the taken branch sets mutatesBeforeLoad=yes (accumulator)"
    );
    assert_eq!(role["persistsCurrentRecord"], "yes");
}

// ---------------------------------------------------------------------------
// FIX 3: opaque-callee uncertainty on a resolved edge to a bodyless callee.
// ---------------------------------------------------------------------------

/// A body-available caller with a resolved member/direct call to a BODYLESS
/// (`bodyAvailable == false`) callee gets an `opaque-callee` uncertainty.
///
/// In source-only AL a bodyless procedure never forms a resolved edge (al-sem
/// reports it `unresolved-call` — the disjunct is unreachable source-only). It IS
/// reachable across apps: a `.app`-dependency routine is `bodyAvailable == false`
/// yet resolves as a member-call target. This test drives the committed cross-app
/// fixture through the full L4 (`project_r3a2`) and asserts the restored
/// `|| calleeOpaque(edge.to)` disjunct (al-sem summary-runner.ts:213) fires:
/// ≥1 resolved-to-dep edge produces an `opaque-callee` uncertainty.
#[test]
fn resolved_edge_to_bodyless_dep_callee_yields_opaque_uncertainty() {
    use al_call_hierarchy::engine::deps::cross_app_l3::build_cross_app_l3_from_workspace;
    let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/r2-5b-fixtures/cross-app-resolution");
    let cross = build_cross_app_l3_from_workspace(&fixture, "r2.5b")
        .expect("cross-app L3 builds over the `.app`-bearing workspace");

    let proj = project_r3a2(&cross.resolved);
    let summaries = serde_json::to_value(&proj.summaries).unwrap();
    let summaries = summaries.as_array().unwrap();

    let opaque_callee_count: usize = summaries
        .iter()
        .filter_map(|s| s["uncertainties"].as_array())
        .flatten()
        .filter(|u| u["kind"] == "opaque-callee")
        .count();

    assert!(
        opaque_callee_count >= 1,
        "the restored `|| calleeOpaque(edge.to)` disjunct must emit ≥1 opaque-callee \
         uncertainty for a resolved edge to a bodyless dep callee; got {opaque_callee_count}.\n{}",
        serde_json::to_string_pretty(summaries).unwrap()
    );
}
