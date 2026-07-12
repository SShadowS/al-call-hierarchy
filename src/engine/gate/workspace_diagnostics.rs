//! `workspace_diagnostics` — port of al-sem `indexWorkspace`'s `workspace.diagnostics`
//! (the FIRST of the six diagnostic sources `analyzeWorkspace` concatenates;
//! al-sem `src/index.ts:157-173`).
//!
//! al-sem assembles `workspace.diagnostics` as:
//!   1. PROVIDER diagnostics (`WorkspaceProvider.collect`, `providers/workspace.ts`)
//!      REMAPPED to `stage: "discover"` — fail-closed errors (multi-app /
//!      id-less / unreadable root `app.json`) and per-file read warnings.
//!   2. INDEX diagnostics (`buildSemanticIndex`, `index/indexer.ts`) — carry their
//!      OWN stage: `index` ("No object declaration found in <rel>"), `parse`
//!      ([PARSER001] native parser unavailable), or `index` (failed-to-index warn).
//!
//! This is computed DETERMINISTICALLY from disk so BOTH the success path and the
//! fail-closed path (`empty_output_result`, which previously dropped these) can
//! thread it into the JSON envelope's `diagnostics` array. The success path's
//! `assemble_*` silently skips unparseable files, so reproducing the index
//! diagnostics here is the single source of truth for them in the gate.
//!
//! Determinism: the provider message for the multi-app case sorts the discovered
//! `app.json` absolute paths (al-sem `appJsonPaths.slice().sort()`); the index
//! "No object declaration found" diagnostics are emitted in rel-posix-sorted file
//! order (the same total order `assemble_workspace` ingests). No HashMap/HashSet
//! iteration leaks into the output order.

use std::path::Path;

use crate::engine::l2::l2_workspace::{count_app_json_paths, discover_al_files, read_al_source};
use crate::engine::l5::registry::Diagnostic;

/// Compute al-sem's `workspace.diagnostics` for a disk workspace: PROVIDER
/// diagnostics (remapped to `stage: "discover"`) concatenated with INDEX
/// diagnostics (their own `index`/`parse` stage), in al-sem order.
///
/// Engine-never-throws: any read/IO failure degrades to fewer diagnostics, never
/// a panic. Returns an empty vec for a clean sound workspace with objects in every
/// file (the source-only corpus's common case — which is why the 40 goldens have
/// `diagnostics: []`).
pub fn compute_workspace_diagnostics(workspace: &Path) -> Vec<Diagnostic> {
    let mut out: Vec<Diagnostic> = Vec::new();

    // --- 1. PROVIDER diagnostics (remapped to "discover") ----------------------
    // Mirror `WorkspaceProvider.collect`'s fail-closed layout detection EXACTLY:
    //   isMultiApp = (count app.json > 1);  appGuid = readable root id (non-empty).
    let app_json_paths = count_app_json_paths(workspace);
    let is_multi_app = app_json_paths.len() > 1;

    let (root_app_json_readable, app_guid) = read_root_app_json_id(workspace);

    if is_multi_app || app_guid.is_none() {
        if is_multi_app {
            // al-sem: `appJsonPaths.slice().sort().join(", ")` over ABSOLUTE paths.
            let mut sorted: Vec<String> = app_json_paths
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect();
            sorted.sort();
            let found_list = sorted.join(", ");
            out.push(Diagnostic {
                severity: "error".to_string(),
                stage: "discover".to_string(),
                message: format!(
                    "multi-app source workspace unsupported — al-sem analyzes one app project per run; point workspaceRoot at the app root. Found {} app.json files: {found_list}",
                    app_json_paths.len()
                ),
            });
        }
        if app_guid.is_none() {
            let root = workspace.to_string_lossy();
            let message = if root_app_json_readable {
                format!(
                    "root app.json at {root} has no string `id` — cannot assign object identity; emitting no source units (fail-closed)"
                )
            } else {
                format!(
                    "no readable root app.json with an `id` at {root} — cannot assign object identity; emitting no source units (fail-closed)"
                )
            };
            out.push(Diagnostic {
                severity: "error".to_string(),
                stage: "discover".to_string(),
                message,
            });
        }
        // Fail-closed: the provider returns NO units, so `buildSemanticIndex` runs
        // over an empty unit list and produces NO index diagnostics. Done.
        return out;
    }

    // --- 1b. PROVIDER per-file read warnings + collect readable units ----------
    // `discover_al_files` already strips BOM and reads UTF-8-lossy; a read error
    // surfaces as a provider `warning` (remapped to "discover"). Units are in
    // rel-posix-sorted order — the same total order the index walks.
    let Ok(discovered) = discover_al_files(workspace) else {
        return out;
    };
    let mut units: Vec<(String, String)> = Vec::new();
    for f in &discovered {
        match read_al_source(&f.abs_path) {
            Ok(src) => units.push((f.rel_posix.clone(), src)),
            Err(e) => out.push(Diagnostic {
                severity: "warning".to_string(),
                stage: "discover".to_string(),
                message: format!("Could not read {}: {e}", f.abs_path.to_string_lossy()),
            }),
        }
    }

    // --- 2. INDEX diagnostics (buildSemanticIndex; their own stage) ------------
    // Per unit, in ingestion (rel-posix-sorted) order: if the owned-IR parse produces
    // NO object declaration, emit an `info`/`index` "No object declaration found in
    // <rel>" — al-sem `indexer.ts:56-63`. Uses the same `al_syntax::parse` the engine
    // indexes with, so the diagnostic reflects exactly what L3 sees (incl. objects
    // nested under a `namespace`, which the former direct-root-children check missed).
    // Sequential parse loop, run on a big-stack thread (T2.1): this CLI path
    // runs on the process main thread, which has no guaranteed-generous stack
    // — see `big_stack`'s doc. One big-stack thread for the WHOLE loop.
    crate::big_stack::run_with_big_stack(|| {
        for (rel, source) in &units {
            if al_syntax::parse(source).objects.is_empty() {
                out.push(Diagnostic {
                    severity: "info".to_string(),
                    stage: "index".to_string(),
                    message: format!("No object declaration found in {rel}"),
                });
            }
        }
    });

    out
}

/// Read the workspace ROOT `app.json`: `(readable, Some(id) when non-empty string)`.
/// Mirrors `WorkspaceProvider.collect`'s try/catch: `readable` is true iff the file
/// parsed as JSON (al-sem sets `rootAppJsonReadable = true` AFTER `JSON.parse`).
fn read_root_app_json_id(workspace: &Path) -> (bool, Option<String>) {
    let Ok(text) = std::fs::read_to_string(workspace.join("app.json")) else {
        return (false, None);
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
        return (false, None);
    };
    let id = v
        .get("id")
        .and_then(|x| x.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    (true, id)
}
