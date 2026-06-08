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

use al_call_hierarchy::engine::deps::cross_app_l3::build_cross_app_l3_from_workspace;
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

/// modelInstanceId for the R2.5b cross-app L3 emit (StableObjectId/StableRoutineId
/// are modelInstanceId-independent — R0; pinned to match the al-sem capture).
const R2_5B_MODEL_INSTANCE_ID: &str = "r2.5b";

fn usage() -> ExitCode {
    eprintln!(
        "usage: aldump [--l2 | --l3-record-types | --l3-call-graph | --l3-event-graph | \
         --l3-coverage | --r2.5a-merged-index | --l3-cross-app | --r3a1-combined-graph | \
         --r3a2-summary-core | --r3a2-trace | --r3a3-cone-coverage | --r3a4-dep-hooks] \
         <workspace-or-.app>"
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
    let mut l3_cross_app = false;
    let mut r3a1_combined_graph = false;
    let mut r3a2_summary_core = false;
    let mut r3a2_trace = false;
    let mut r3a3_cone_coverage = false;
    let mut r3a4_dep_hooks = false;
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
        if arg == "--l3-cross-app" {
            l3_cross_app = true;
            continue;
        }
        if arg == "--r3a1-combined-graph" {
            r3a1_combined_graph = true;
            continue;
        }
        if arg == "--r3a2-summary-core" {
            r3a2_summary_core = true;
            continue;
        }
        if arg == "--r3a2-trace" {
            r3a2_trace = true;
            continue;
        }
        if arg == "--r3a3-cone-coverage" {
            r3a3_cone_coverage = true;
            continue;
        }
        if arg == "--r3a4-dep-hooks" {
            r3a4_dep_hooks = true;
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
        l3_cross_app,
        r3a1_combined_graph,
        r3a2_summary_core,
        r3a2_trace,
        r3a3_cone_coverage,
        r3a4_dep_hooks,
    ]
    .iter()
    .filter(|f| **f)
    .count()
        > 1
    {
        eprintln!(
            "aldump: error: --l2 / --l3-record-types / --l3-call-graph / --l3-event-graph / \
             --l3-coverage / --r2.5a-merged-index / --l3-cross-app / --r3a1-combined-graph / \
             --r3a2-summary-core / --r3a2-trace / --r3a3-cone-coverage / --r3a4-dep-hooks \
             are mutually exclusive"
        );
        return usage();
    }

    let Some(workspace_arg) = workspace_arg else {
        return usage();
    };
    let workspace = PathBuf::from(workspace_arg);

    if r3a4_dep_hooks {
        // R3a-4 DEP-HOOK PROJECTION: read the workspace's `.alpackages` dep `.app`(s),
        // build each dep's embedded-source PRODUCER artifact, drive the CONSUMER hooks
        // (inject_intra_app_call_edges / collect_cited_dep_evidence /
        // collect_dep_order_index) over a merged model whose routine membership =
        // workspace own routines + every dep's own routines, then STABLE-PROJECT every
        // id-bearing field (appGuid:Type:Num#sigHash — cache/modelInstanceId-independent)
        // and emit the producer payloads + consumed effect in the SAME stable shape /
        // key-order as the al-sem `cross-app-dep-hooks.r3a4.golden.json`. CAPTURE POINT:
        // post-inject/collect hooks; the R3a-5 cross-app cone is NOT projected here.
        // Fail-closed → an empty projection (never throws).
        let projection =
            al_call_hierarchy::engine::deps::r3a4_projection::project_r3a4_from_workspace(
                &workspace,
                "cross-app-dep-hooks",
            );
        return match serde_json::to_string_pretty(&projection) {
            Ok(json) => {
                println!("{json}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("aldump: error: failed to serialize R3a-4 dep-hook projection: {e}");
                ExitCode::FAILURE
            }
        };
    }

    if r3a2_summary_core {
        // R3a-2 SUMMARY CORE: run the SOURCE-ONLY L0→L3 pipeline → buildCombinedGraph
        // → tarjanScc → computeSummaries (the JACOBI fixed point), then project the
        // RoutineSummary CORE (dbEffects / uncertainties / parameterRoles /
        // inRecursiveCycle / hasUnresolvedCalls) in the SAME stable shape/key-order as
        // the al-sem `<fixture>.r3a2.golden.json`. CAPTURE POINT: POST-computeSummaries;
        // NO dep hooks (R3a-4); the cone/coverage (R3a-3) are never declared on the
        // projected types. Fail-closed/empty layouts → an empty projection (never throws).
        let projection = match assemble_and_resolve_workspace_default(&workspace) {
            Some(resolved) => al_call_hierarchy::engine::l4::summary::project_r3a2(&resolved),
            None => {
                eprintln!(
                    "aldump: warning: fail-closed/empty layout at {} — emitting empty R3a-2 projection",
                    workspace.display()
                );
                al_call_hierarchy::engine::l4::summary::R3a2Projection { summaries: vec![] }
            }
        };
        return match serde_json::to_string_pretty(&projection) {
            Ok(json) => {
                println!("{json}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("aldump: error: failed to serialize R3a-2 summary-core projection: {e}");
                ExitCode::FAILURE
            }
        };
    }

    if r3a3_cone_coverage {
        // R3a-3 CAPABILITY CONE + COVERAGE: run the SOURCE-ONLY L0→L3 pipeline, then
        // the cone/coverage pass (direct capability extraction over the resolved
        // features + the publisher-fact injection → composeInheritedCones), and emit
        // the stable projection (capabilityFactsDirect / capabilityFactsInherited /
        // coverage per routine) in the SAME shape/key-order as the al-sem
        // `<fixture>.r3a3.golden.json`. CAPTURE POINT: POST-computeSummaries cone pass;
        // NO dep hooks (R3a-4). Fail-closed/empty layouts → an empty projection.
        let projection = match assemble_and_resolve_workspace_default(&workspace) {
            Some(resolved) => {
                al_call_hierarchy::engine::l4::capability_cone::project_r3a3(&resolved)
            }
            None => {
                eprintln!(
                    "aldump: warning: fail-closed/empty layout at {} — emitting empty R3a-3 projection",
                    workspace.display()
                );
                al_call_hierarchy::engine::l4::capability_cone::R3a3Projection { summaries: vec![] }
            }
        };
        return match serde_json::to_string_pretty(&projection) {
            Ok(json) => {
                println!("{json}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("aldump: error: failed to serialize R3a-3 cone+coverage projection: {e}");
                ExitCode::FAILURE
            }
        };
    }

    if r3a2_trace {
        // R3a-2 JACOBI fingerprint TRACE: run the same SOURCE-ONLY pipeline but ALSO
        // collect the per-recursive-SCC fingerprint trace the fixed-point loop produces
        // (the per-iteration stable fingerprint sequence + iteration count + per-pass
        // `changed`), in the SAME shape/key-order as the al-sem
        // `<fixture>.r3a2-trace.golden.json`. Proves JACOBI parity (frozen prior-pass
        // snapshot, not Gauss-Seidel). Fail-closed → an empty trace (never throws).
        let trace = match assemble_and_resolve_workspace_default(&workspace) {
            Some(resolved) => {
                al_call_hierarchy::engine::l4::summary::project_r3a2_with_trace(&resolved).1
            }
            None => {
                eprintln!(
                    "aldump: warning: fail-closed/empty layout at {} — emitting empty R3a-2 trace",
                    workspace.display()
                );
                al_call_hierarchy::engine::l4::summary::R3a2Trace { traces: vec![] }
            }
        };
        return match serde_json::to_string_pretty(&trace) {
            Ok(json) => {
                println!("{json}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("aldump: error: failed to serialize R3a-2 trace: {e}");
                ExitCode::FAILURE
            }
        };
    }

    if r3a1_combined_graph {
        // R3a-1 L4 GRAPH SUBSTRATE: run the SOURCE-ONLY L0→L3 pipeline, then
        // buildCombinedGraph → tarjanScc → projectR3a1, and emit the stable R3a-1
        // projection (combinedEdges + uncertaintyEdges + typedEdges + the
        // reverse-topo SCC list) in the SAME shape/key-order as the al-sem
        // `<fixture>.r3a1.golden.json`. CAPTURE POINT: POST-buildCombinedGraph /
        // POST-tarjanScc / PRE-computeSummaries — NO dep hooks, NO summaries (R3a-2+).
        // Fail-closed/empty layouts → an empty projection (never throws).
        let projection = match assemble_and_resolve_workspace_default(&workspace) {
            Some(resolved) => resolved.project_r3a1_combined_graph(),
            None => {
                eprintln!(
                    "aldump: warning: fail-closed/empty layout at {} — emitting empty R3a-1 projection",
                    workspace.display()
                );
                al_call_hierarchy::engine::l4::combined_graph::R3a1Projection {
                    combined_edges: vec![],
                    uncertainty_edges: vec![],
                    typed_edges: vec![],
                    sccs: vec![],
                }
            }
        };
        return match serde_json::to_string_pretty(&projection) {
            Ok(json) => {
                println!("{json}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("aldump: error: failed to serialize R3a-1 projection: {e}");
                ExitCode::FAILURE
            }
        };
    }

    if l3_cross_app {
        // R2.5b cross-app L3 SMOKE: read the workspace `.al` source + its dep `.app`(s)
        // under `<workspace>/.alpackages`, build the merged index, run the unchanged L3
        // pipeline over workspace+deps, and emit the four cross-app surfaces (record
        // types / call graph / event graph / coverage) as one JSON envelope. Task 1
        // proves the pipeline RUNS + produces NON-EMPTY cross-app resolution; Tasks 2-5
        // add the per-surface byte-goldens + matrices. Fail-closed → an empty envelope.
        let envelope = match build_cross_app_l3_from_workspace(&workspace, R2_5B_MODEL_INSTANCE_ID)
        {
            Some(cross) => serde_json::json!({
                "recordTypes": cross.project_record_types(),
                "callGraph": cross.project_call_graph(),
                "eventGraph": cross.project_event_graph(),
                "coverage": cross.project_coverage_disk(&workspace),
            }),
            None => {
                eprintln!(
                    "aldump: warning: fail-closed/empty cross-app layout at {} — emitting empty envelope",
                    workspace.display()
                );
                serde_json::json!({
                    "recordTypes": { "tables": [], "routines": [] },
                    "callGraph": { "groups": [], "bindings": [] },
                    "eventGraph": { "events": [], "edges": [] },
                    "coverage": serde_json::Value::Null,
                })
            }
        };
        return match serde_json::to_string_pretty(&envelope) {
            Ok(json) => {
                println!("{json}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("aldump: error: failed to serialize cross-app envelope: {e}");
                ExitCode::FAILURE
            }
        };
    }

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
