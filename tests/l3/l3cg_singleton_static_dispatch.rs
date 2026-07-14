//! Phase-2 fixture tests: AL platform STATIC singleton types are reclassified
//! from `unknown { UntrackedReceiver }` to `builtin` via the new per-kind
//! member catalogs.
//!
//! Covered singletons:
//!   * `IsolatedStorage` — 5 methods (Set/Get/Contains/Delete/SetEncrypted)
//!   * `Session`         — 19 methods (LogMessage, etc.)
//!   * `NavApp`          — 16 methods (GetCurrentModuleInfo, etc.)
//!   * `TaskScheduler`   — 5 methods (CreateTask, etc.)
//!   * `Database`        — 29 methods (Commit, etc.)
//!   * `Page` (static)   — reuses PageInstance catalog (RunModal, etc.)
//!   * `Report` (static) — reuses ReportInstance catalog (Run, RunModal, etc.)
//!
//! The variables-first check must be preserved: a user var named e.g. `Session`
//! shadows the singleton and must NOT be intercepted.

use al_call_hierarchy::engine::l3::call_graph_projection::{L3CallGraphProjection, PCallEdge};
use al_call_hierarchy::engine::l3::l3_workspace::assemble_and_resolve_default;

const APP_GUID: &str = "3c000000-0000-0000-0000-0000000003cc";

fn project_ws(files: &[(&str, &str)]) -> L3CallGraphProjection {
    let owned: Vec<(String, String)> = files
        .iter()
        .map(|(n, s)| (n.to_string(), s.to_string()))
        .collect();
    assemble_and_resolve_default(&owned, APP_GUID).project_call_graph()
}

fn all_edges(p: &L3CallGraphProjection) -> Vec<&PCallEdge> {
    p.groups.iter().flat_map(|g| g.edges.iter()).collect()
}

// ── IsolatedStorage.Set ─────────────────────────────────────────────────────

const ISOLATED_STORAGE_SET_SRC: &str = r#"
codeunit 50000 TestIsolatedStorage {
    procedure TestProc()
    begin
        IsolatedStorage.Set('myKey', 'myValue', DataScope::Module);
    end;
}
"#;

#[test]
fn isolated_storage_set_is_builtin() {
    let p = project_ws(&[("src/isolated_storage.al", ISOLATED_STORAGE_SET_SRC)]);
    let edges = all_edges(&p);
    let builtin_edges: Vec<_> = edges
        .iter()
        .filter(|e| e.resolution == "builtin" && e.dispatch_kind == "builtin")
        .collect();
    assert!(
        !builtin_edges.is_empty(),
        "IsolatedStorage.Set() must produce a builtin edge; all edges: {:#?}",
        edges
    );
    // No unknown edges from this IsolatedStorage call.
    let unknown_edges: Vec<_> = edges.iter().filter(|e| e.resolution == "unknown").collect();
    assert!(
        unknown_edges.is_empty(),
        "IsolatedStorage.Set() must not produce unknown edges; unknowns: {:#?}",
        unknown_edges
    );
}

// ── Session.LogMessage ───────────────────────────────────────────────────────

const SESSION_LOGMESSAGE_SRC: &str = r#"
codeunit 50001 TestSession {
    procedure TestProc()
    begin
        Session.LogMessage('0001', 'Test msg', Verbosity::Normal, DataClassification::SystemMetadata, TelemetryScope::ExtensionPublisher, 'custom', 'value');
    end;
}
"#;

#[test]
fn session_logmessage_is_builtin() {
    let p = project_ws(&[("src/session_log.al", SESSION_LOGMESSAGE_SRC)]);
    let edges = all_edges(&p);
    let builtin_edges: Vec<_> = edges
        .iter()
        .filter(|e| e.resolution == "builtin" && e.dispatch_kind == "builtin")
        .collect();
    assert!(
        !builtin_edges.is_empty(),
        "Session.LogMessage() must produce a builtin edge; all edges: {:#?}",
        edges
    );
    let unknown_edges: Vec<_> = edges.iter().filter(|e| e.resolution == "unknown").collect();
    assert!(
        unknown_edges.is_empty(),
        "Session.LogMessage() must not produce unknown edges; unknowns: {:#?}",
        unknown_edges
    );
}

// ── NavApp.GetCurrentModuleInfo ──────────────────────────────────────────────

const NAVAPP_MODULEINFO_SRC: &str = r#"
codeunit 50002 TestNavApp {
    procedure TestProc()
    var
        Info: ModuleInfo;
    begin
        NavApp.GetCurrentModuleInfo(Info);
    end;
}
"#;

#[test]
fn navapp_moduleinfo_is_builtin() {
    let p = project_ws(&[("src/navapp_info.al", NAVAPP_MODULEINFO_SRC)]);
    let edges = all_edges(&p);
    let builtin_edges: Vec<_> = edges
        .iter()
        .filter(|e| e.resolution == "builtin" && e.dispatch_kind == "builtin")
        .collect();
    assert!(
        !builtin_edges.is_empty(),
        "NavApp.GetCurrentModuleInfo() must produce a builtin edge; all edges: {:#?}",
        edges
    );
    let unknown_edges: Vec<_> = edges.iter().filter(|e| e.resolution == "unknown").collect();
    assert!(
        unknown_edges.is_empty(),
        "NavApp.GetCurrentModuleInfo() must not produce unknown edges; unknowns: {:#?}",
        unknown_edges
    );
}

// ── Page.RunModal (static call from non-page object) ────────────────────────

const PAGE_RUNMODAL_STATIC_SRC: &str = r#"
codeunit 50003 TestPageStatic {
    procedure TestProc()
    begin
        Page.RunModal(50000);
    end;
}
"#;

#[test]
fn page_runmodal_static_is_builtin() {
    let p = project_ws(&[("src/page_static.al", PAGE_RUNMODAL_STATIC_SRC)]);
    let edges = all_edges(&p);
    let builtin_edges: Vec<_> = edges
        .iter()
        .filter(|e| e.resolution == "builtin" && e.dispatch_kind == "builtin")
        .collect();
    assert!(
        !builtin_edges.is_empty(),
        "Page.RunModal() (static) must produce a builtin edge; all edges: {:#?}",
        edges
    );
    let unknown_edges: Vec<_> = edges.iter().filter(|e| e.resolution == "unknown").collect();
    assert!(
        unknown_edges.is_empty(),
        "Page.RunModal() (static) must not produce unknown edges; unknowns: {:#?}",
        unknown_edges
    );
}

// ── User var named Session shadows the singleton ─────────────────────────────

const USER_VAR_NAMED_SESSION_SRC: &str = r#"
codeunit 50010 SomeCodeunit { }
codeunit 50004 TestUserVarSession {
    procedure TestProc()
    var
        Session: Codeunit SomeCodeunit;
    begin
        Session.Foo();
    end;
}
"#;

#[test]
fn user_var_named_session_not_intercepted() {
    // When a user declares `var Session: Codeunit X`, Session.Foo() must NOT
    // be intercepted as the platform singleton. It should resolve through the
    // Object (Codeunit) path — MemberNotFound (since Foo doesn't exist on
    // SomeCodeunit), NOT builtin.
    let p = project_ws(&[("src/user_var_session.al", USER_VAR_NAMED_SESSION_SRC)]);
    let edges = all_edges(&p);
    // Must NOT produce a builtin edge for the Foo() call.
    let builtin_edges: Vec<_> = edges.iter().filter(|e| e.resolution == "builtin").collect();
    assert!(
        builtin_edges.is_empty(),
        "Session.Foo() via user var must NOT be classified as builtin; edges: {:#?}",
        builtin_edges
    );
    // The Foo() call should NOT be an UntrackedReceiver unknown — Session WAS found
    // as a variable (Codeunit), so it routes through the Object dispatch path.
    // It should be MemberNotFound (not a platform unknown).
    let unknown_edges: Vec<_> = edges.iter().filter(|e| e.resolution == "unknown").collect();
    assert!(
        unknown_edges.is_empty(),
        "Session.Foo() via a Codeunit var must be member-not-found, not unknown; edges: {:#?}",
        unknown_edges
    );
}

// ── IsolatedStorage.NoSuchZzz stays unknown (catalog miss) ──────────────────

const ISOLATED_STORAGE_MISS_SRC: &str = r#"
codeunit 50005 TestIsolatedStorageMiss {
    procedure TestProc()
    begin
        IsolatedStorage.NoSuchZzz('x');
    end;
}
"#;

#[test]
fn unknown_singleton_method_stays_unknown() {
    // IsolatedStorage is intercepted but NoSuchZzz is not in the catalog —
    // must emit Unknown { FrameworkMethodNotInCatalog }, NOT builtin.
    let p = project_ws(&[("src/isolated_storage_miss.al", ISOLATED_STORAGE_MISS_SRC)]);
    let edges = all_edges(&p);
    let unknown_edges: Vec<_> = edges.iter().filter(|e| e.resolution == "unknown").collect();
    assert!(
        !unknown_edges.is_empty(),
        "IsolatedStorage.NoSuchZzz() (catalog miss) must stay unknown; all edges: {:#?}",
        edges
    );
    let builtin_edges: Vec<_> = edges.iter().filter(|e| e.resolution == "builtin").collect();
    assert!(
        builtin_edges.is_empty(),
        "IsolatedStorage.NoSuchZzz() must not be a false builtin; edges: {:#?}",
        builtin_edges
    );
}
