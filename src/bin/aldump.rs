//! `aldump <workspace>` — R0 differential-harness producer.
//! `aldump --l2 <workspace>` — R1a L2 features projection producer (Task 3).
//!
//! DEFAULT (no `--l2`): parses an AL workspace and emits the OBJECT/ROUTINE
//! IDENTITY SUBSET as JSON on stdout, in the EXACT shape of al-sem's committed
//! R0 "golden" files. R0 Task 5 diffs this output against those goldens; the
//! extraction logic (`engine::snapshot`) reproduces al-sem's identity derivation
//! precisely so the diff can pass.
//!
//! `--l2`: parses the workspace and emits the ALLOWLISTED L2 FEATURES PROJECTION
//! (`engine::l2::l2_workspace`) — objects + routines with metadata + per-routine
//! `features` (loops/operations/call-sites/record-ops/CFN skeleton/…), matching
//! the R1a goldens (`scripts/r1a-goldens/<fixture>.l2.golden.json`). Forbidden
//! later-gate / L3-resolved fields are structurally absent from the projection
//! types, so they can never appear here.
//!
//! DESIGN DEVIATION (R0, deliberate): the default mode emits the identity-subset
//! JSON directly rather than a v3-shaped CapabilitySnapshot — that subset carries
//! fields (routine sub-kind, `canonicalSignatureText`) a v3 envelope cannot.
//!
//! Output discipline: ONLY JSON goes to stdout; all logs/warnings go to stderr.
//! No absolute paths appear anywhere in the output.

use std::path::PathBuf;
use std::process::ExitCode;

use al_call_hierarchy::engine::deps::merged_index::{
    build_merged_index_from_path, serialize_projection,
};
use al_call_hierarchy::engine::l2::l2_workspace::project_workspace;
use al_call_hierarchy::engine::l3::l3_workspace::assemble_and_resolve_workspace_default;
use al_call_hierarchy::engine::snapshot::snapshot_workspace;

/// modelInstanceId for the R2.5a merged-index emit. The emitted StableObjectId /
/// StableRoutineId are modelInstanceId-INDEPENDENT (R0), so this value never
/// reaches the output — it only feeds the internal routine id. Pinned to match the
/// al-sem dump's `MODEL_INSTANCE_ID` so any future internal-id surfacing stays
/// aligned.
const R2_5A_MODEL_INSTANCE_ID: &str = "r2.5a";

fn usage() -> ExitCode {
    eprintln!(
        "usage: aldump [--l2 | --l3-record-types | --l3-call-graph | --l3-event-graph | \
         --l3-coverage | --r2.5a-merged-index] <workspace-or-.app>"
    );
    ExitCode::FAILURE
}

fn main() -> ExitCode {
    let mut l2 = false;
    let mut l3_record_types = false;
    let mut l3_call_graph = false;
    let mut l3_event_graph = false;
    let mut l3_coverage = false;
    let mut r2_5a_merged_index = false;
    let mut workspace_arg: Option<std::ffi::OsString> = None;

    for arg in std::env::args_os().skip(1) {
        // `--l2` / `--l3-record-types` / `--l3-call-graph` / `--l3-event-graph` /
        // `--l3-coverage` / `--r2.5a-merged-index` flags (anywhere); else the
        // single positional.
        if arg == "--l2" {
            l2 = true;
            continue;
        }
        if arg == "--l3-record-types" {
            l3_record_types = true;
            continue;
        }
        if arg == "--l3-call-graph" {
            l3_call_graph = true;
            continue;
        }
        if arg == "--l3-event-graph" {
            l3_event_graph = true;
            continue;
        }
        if arg == "--l3-coverage" {
            l3_coverage = true;
            continue;
        }
        if arg == "--r2.5a-merged-index" {
            r2_5a_merged_index = true;
            continue;
        }
        if workspace_arg.is_some() {
            eprintln!("aldump: error: more than one workspace argument");
            return usage();
        }
        workspace_arg = Some(arg);
    }

    if [
        l2,
        l3_record_types,
        l3_call_graph,
        l3_event_graph,
        l3_coverage,
        r2_5a_merged_index,
    ]
    .iter()
    .filter(|f| **f)
    .count()
        > 1
    {
        eprintln!(
            "aldump: error: --l2 / --l3-record-types / --l3-call-graph / --l3-event-graph / \
             --l3-coverage / --r2.5a-merged-index are mutually exclusive"
        );
        return usage();
    }

    let Some(workspace_arg) = workspace_arg else {
        return usage();
    };
    let workspace = PathBuf::from(workspace_arg);

    if r2_5a_merged_index {
        // R2.5a merged-index projection: read the `.app`(s) at `workspace` (a single
        // `.app` OR a dir/`.alpackages` of them), project + merge (incl. the
        // extension-field merge — the post-resolveModel capture-point invariant),
        // and emit the dependency-entity subset in the SAME stable JSON shape as the
        // al-sem `*.r2.5a.golden.json`. NO cross-app L3 resolution (that is R2.5b).
        // Fail-closed: an unreadable / empty path yields an all-empty projection
        // (never throws). Output is byte-stable (serialize_projection appends the
        // trailing newline to match the TS goldens).
        let projection = build_merged_index_from_path(&workspace, R2_5A_MODEL_INSTANCE_ID);
        print!("{}", serialize_projection(&projection));
        return ExitCode::SUCCESS;
    }

    if l3_coverage {
        // L3 coverage projection (R2d): the resolved model's AnalysisCoverage —
        // sourceUnitsTotal/Parsed, routinesTotal/BodyAvailable, parseIncomplete
        // (StableRoutineId[]), opaqueApps (empty source-only), unresolvedCallsites
        // (StableCallsiteId multiset), dynamicDispatchSites (StableOperationId
        // multiset). Runs assemble→resolve→project_coverage_disk (reads the resolved
        // call graph + L2 routine flags; the post-resolve read the dump captures).
        // Fail-closed → an all-empty AnalysisCoverage (never throws).
        let projection = match assemble_and_resolve_workspace_default(&workspace) {
            Some(resolved) => resolved.project_coverage_disk(&workspace),
            None => {
                eprintln!(
                    "aldump: warning: fail-closed/empty layout at {} — emitting empty coverage",
                    workspace.display()
                );
                al_call_hierarchy::engine::l3::coverage::AnalysisCoverage {
                    source_units_total: 0,
                    source_units_parsed: 0,
                    routines_total: 0,
                    routines_body_available: 0,
                    routines_parse_incomplete: vec![],
                    opaque_apps: vec![],
                    unresolved_callsites: vec![],
                    dynamic_dispatch_sites: vec![],
                }
            }
        };
        return match serde_json::to_string_pretty(&projection) {
            Ok(json) => {
                println!("{json}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("aldump: error: failed to serialize L3 coverage projection: {e}");
                ExitCode::FAILURE
            }
        };
    }

    if l3_event_graph {
        // L3 event-graph projection (R2c): the resolved event graph — EventSymbols
        // (publishers + synthesized maybe/unknown) + EventEdges (subscribers,
        // open-world) — in stable id form. Runs assemble→resolve→build_event_graph
        // →project_event_graph (reads model.eventGraph; never re-runs the builder
        // for a later gate). Fail-closed → empty `{events, edges}` (never throws).
        let projection = match assemble_and_resolve_workspace_default(&workspace) {
            Some(resolved) => resolved.project_event_graph(),
            None => {
                eprintln!(
                    "aldump: warning: fail-closed/empty layout at {} — emitting empty projection",
                    workspace.display()
                );
                al_call_hierarchy::engine::l3::event_graph::L3EventGraphProjection {
                    events: vec![],
                    edges: vec![],
                }
            }
        };
        return match serde_json::to_string_pretty(&projection) {
            Ok(json) => {
                println!("{json}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("aldump: error: failed to serialize L3 event-graph projection: {e}");
                ExitCode::FAILURE
            }
        };
    }

    if l3_call_graph {
        // L3 call-graph projection (R2b): the resolved call graph (grouped
        // callsiteId → CallEdge[], multi-edge interface dispatch preserved,
        // group-level dispatchMeta) + the upgraded argumentBindings, all in stable
        // id form. Fail-closed → empty `{groups, bindings}` (never throws).
        let projection = match assemble_and_resolve_workspace_default(&workspace) {
            Some(resolved) => resolved.project_call_graph(),
            None => {
                eprintln!(
                    "aldump: warning: fail-closed/empty layout at {} — emitting empty projection",
                    workspace.display()
                );
                al_call_hierarchy::engine::l3::call_graph_projection::L3CallGraphProjection {
                    groups: vec![],
                    bindings: vec![],
                }
            }
        };
        return match serde_json::to_string_pretty(&projection) {
            Ok(json) => {
                println!("{json}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("aldump: error: failed to serialize L3 call-graph projection: {e}");
                ExitCode::FAILURE
            }
        };
    }

    if l3_record_types {
        // L3 record-type projection (R2a): resolved record-var/op StableTableIds
        // (omitted when unresolved) + per-Table merged fields. Fail-closed →
        // empty `{tables, routines}` (never throws).
        let projection = match assemble_and_resolve_workspace_default(&workspace) {
            Some(resolved) => resolved.project(),
            None => {
                eprintln!(
                    "aldump: warning: fail-closed/empty layout at {} — emitting empty projection",
                    workspace.display()
                );
                al_call_hierarchy::engine::l3::l3_workspace::L3RecordTypeProjection {
                    tables: vec![],
                    routines: vec![],
                }
            }
        };
        return match serde_json::to_string_pretty(&projection) {
            Ok(json) => {
                println!("{json}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("aldump: error: failed to serialize L3 projection: {e}");
                ExitCode::FAILURE
            }
        };
    }

    if l2 {
        let projection = match project_workspace(&workspace) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("aldump: error: {e:#}");
                return ExitCode::FAILURE;
            }
        };
        match serde_json::to_string_pretty(&projection) {
            Ok(json) => {
                println!("{json}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("aldump: error: failed to serialize L2 projection: {e}");
                ExitCode::FAILURE
            }
        }
    } else {
        let snapshot = match snapshot_workspace(&workspace) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("aldump: error: {e:#}");
                return ExitCode::FAILURE;
            }
        };
        // Pretty-print with 2-space indent to mirror the goldens (the differ
        // parses structurally, so pretty-printing is a convenience).
        match serde_json::to_string_pretty(&snapshot) {
            Ok(json) => {
                println!("{json}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("aldump: error: failed to serialize snapshot: {e}");
                ExitCode::FAILURE
            }
        }
    }
}
