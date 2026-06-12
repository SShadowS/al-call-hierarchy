//! Gap G-18 (docs/engine-gaps.md): d1 reports an op as "in a loop" when, on the
//! REAL call path to that op, it is NOT inside any loop — the loop is attributed
//! from a SIBLING call path. Root cause: two same-name same-signature triggers in
//! one object (e.g. two page actions, each with `trigger OnAction()`) collide on
//! the internal routine id (`compute_routine_id` keys app/object/kind/name/
//! signature — no member discriminator), so their call-site ids (`{rid}/cs{n}`)
//! collide too and `edges_by_from[{rid}]` mixes BOTH bodies' edges. d1's root
//! edge lookup (`find(callsite_id == cs.id)`) could then pick the SIBLING
//! action's edge for the LOOPING action's in-loop call site — walking a chain
//! the loop is not on (the CDO batch-7 `eDocumentsConfigExists` shape).
//!
//! The fix: the picked edge's TARGET must match the call site's own callee name
//! (the resolver resolves by name, so a genuinely-own edge ALWAYS matches —
//! the guard only ever filters cross-body edges under a colliding id).
//!
//! Firing-preservation guards:
//!   - a REAL in-loop chain THROUGH a colliding trigger still fires (control b);
//!   - a vanilla transitive in-loop chain still fires at `high` (control c —
//!     the G-1/G-4 behavior).

use al_call_hierarchy::engine::l3::l3_workspace::assemble_and_resolve_default;
use al_call_hierarchy::engine::l5::detectors::registered_detectors;
use al_call_hierarchy::engine::l5::finding::Finding;
use al_call_hierarchy::engine::l5::registry::run_detectors;

const APP_GUID: &str = "11111111-0000-0000-0000-0000000g18ab";

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

fn root_causes(findings: &[Finding]) -> Vec<&str> {
    findings.iter().map(|f| f.root_cause.as_str()).collect()
}

const TABLES: &str = r#"
table 50801 "G18 Setup"
{
    fields { field(1; "No."; Code[20]) { } }
    keys { key(PK; "No.") { } }
}

table 50802 "G18 Cust"
{
    fields { field(1; "No."; Code[20]) { } }
    keys { key(PK; "No.") { } }
}

table 50803 "G18 Log"
{
    fields { field(1; "No."; Code[20]) { } }
    keys { key(PK; "No.") { } }
}
"#;

// --- (a) the G-18 false positive: sibling-path loop mis-attribution ---------

/// Two actions on one page, both with `trigger OnAction()` (colliding routine
/// ids). `RunBatch.OnAction` loops calling an UNRESOLVED external routine (no
/// edge of its own); `Finish.OnAction` is STRAIGHT-LINE and calls the local
/// `HandleSetup → CreateSetup` chain that does `IsEmpty`/`Insert` on G18 Setup.
///
/// The loop is NOT on any path to `CreateSetup`'s ops, so d1 must emit NOTHING.
/// (Pre-fix, the looping action's in-loop call site picked the SIBLING action's
/// `HandleSetup` edge — same colliding callsite id — and flagged both ops as
/// "reached from a loop in OnAction".)
#[test]
fn loop_in_sibling_onaction_is_not_attributed_to_straightline_path() {
    let page = r#"
page 50801 "G18 Wizard"
{
    PageType = Card;
    SourceTable = "G18 Cust";

    actions
    {
        area(Processing)
        {
            action(RunBatch)
            {
                trigger OnAction()
                var
                    Cust: Record "G18 Cust";
                begin
                    repeat
                        ProcessExternalLine();
                    until Cust.Next() = 0;
                end;
            }
            action(Finish)
            {
                trigger OnAction()
                begin
                    HandleSetup();
                end;
            }
        }
    }

    local procedure HandleSetup()
    begin
        CreateSetup();
    end;

    local procedure CreateSetup()
    var
        Setup: Record "G18 Setup";
    begin
        if Setup.IsEmpty() then
            Setup.Insert();
    end;
}
"#;
    let src = format!("{TABLES}{page}");
    let findings = run_d1(&[al("G18Wizard", &src)]);
    assert!(
        findings.is_empty(),
        "no loop is on the actual path to CreateSetup's ops — the loop in the \
         SIBLING action's OnAction must not be attributed to the straight-line \
         Finish → HandleSetup → CreateSetup chain (G-18). findings: {:#?}",
        root_causes(&findings)
    );
}

// --- (b) CONTROL: a REAL in-loop chain through a colliding trigger fires ----

/// Same colliding two-OnAction shape, but the LOOPING action's in-loop call is
/// RESOLVED to `LoopHelper` (which writes G18 Log). That chain genuinely runs
/// per iteration → d1 must still fire on `LoopHelper`'s Insert at `high`, and
/// must still NOT flag the sibling straight-line `StraightHelper` op.
#[test]
fn real_inloop_chain_through_colliding_trigger_still_fires() {
    let page = r#"
page 50802 "G18 Worklist"
{
    PageType = Card;
    SourceTable = "G18 Cust";

    actions
    {
        area(Processing)
        {
            action(RunBatch)
            {
                trigger OnAction()
                var
                    Cust: Record "G18 Cust";
                begin
                    repeat
                        LoopHelper();
                    until Cust.Next() = 0;
                end;
            }
            action(Finish)
            {
                trigger OnAction()
                begin
                    StraightHelper();
                end;
            }
        }
    }

    local procedure LoopHelper()
    var
        Log: Record "G18 Log";
    begin
        Log.Insert();
    end;

    local procedure StraightHelper()
    var
        Setup: Record "G18 Setup";
    begin
        if Setup.IsEmpty() then
            Setup.Insert();
    end;
}
"#;
    let src = format!("{TABLES}{page}");
    let findings = run_d1(&[al("G18Worklist", &src)]);
    assert_eq!(
        findings.len(),
        1,
        "exactly the genuine in-loop chain (OnAction loop → LoopHelper.Insert) \
         must fire — nothing on the sibling straight-line path. findings: {:#?}",
        root_causes(&findings)
    );
    let f = &findings[0];
    assert!(
        f.root_cause
            .contains("A loop in OnAction reaches Insert on G18 Log in LoopHelper"),
        "the genuine transitive finding must keep firing with the loop \
         attributed to the looping OnAction. rootCause: {}",
        f.root_cause
    );
    assert_eq!(
        f.severity, "high",
        "write at loop depth 1 stays high. rootCause: {}",
        f.root_cause
    );
    assert!(
        !findings
            .iter()
            .any(|f| f.root_cause.contains("StraightHelper") || f.root_cause.contains("G18 Setup")),
        "the sibling straight-line path must stay clean. findings: {:#?}",
        root_causes(&findings)
    );
}

// --- (c) CONTROL: vanilla transitive in-loop finding unaffected -------------

/// The plain G-1/G-4 shape — a codeunit loop calling a leaf that inserts. Must
/// keep firing at `high` (the guard must never suppress a genuine transitive
/// finding outside the colliding-trigger shape).
#[test]
fn vanilla_transitive_inloop_finding_still_fires() {
    let src = format!(
        r#"{TABLES}
codeunit 50801 "G18 Vanilla"
{{
    procedure LoopCaller(var Cust: Record "G18 Cust")
    begin
        repeat
            CreateLogEntry();
        until Cust.Next() = 0;
    end;

    procedure CreateLogEntry()
    var
        Log: Record "G18 Log";
    begin
        Log.Insert();
    end;
}}
"#
    );
    let findings = run_d1(&[al("G18Vanilla", &src)]);
    assert_eq!(
        findings.len(),
        1,
        "the vanilla transitive Insert must fire. findings: {:#?}",
        root_causes(&findings)
    );
    assert!(
        findings[0]
            .root_cause
            .contains("A loop in LoopCaller reaches Insert on G18 Log"),
        "rootCause: {}",
        findings[0].root_cause
    );
    assert_eq!(findings[0].severity, "high");
}
