//! Builds a `ProgramGraph` from an `AppSetSnapshot`.

use crate::program::graph::{ObjectIndex, ProgramGraph};
use crate::program::node::{AppRef, AppRegistry};
use crate::program::node_extract::{ObjectNode, RoutineNode, extract_nodes};
use crate::program::topology::DependencyGraph;
use crate::snapshot::{AppSetSnapshot, parse_snapshot};

/// Assemble a `ProgramGraph` from a fully-resolved `AppSetSnapshot`.
///
/// Steps:
/// 1. Intern every app identity from the snapshot into an `AppRegistry`.
/// 2. Deep-parse all source-bearing units (via `parse_snapshot`) and extract
///    object + routine nodes.
/// 3. Wire the real dependency topology from each unit's `declared_deps`
///    (GUID-match preferred; name+version fallback; deps absent from the
///    snapshot are silently skipped — open-world assumption).
/// 4. Sort `objects` and `routines` by node-id for determinism.
/// 5. Build the `ObjectIndex` from the sorted `objects`.
pub fn build_program_graph(snap: &AppSetSnapshot) -> ProgramGraph {
    // ── Step 1: intern all app identities ────────────────────────────────────
    let mut apps = AppRegistry::default();
    let app_refs: Vec<AppRef> = snap.apps.iter().map(|u| apps.intern(&u.id)).collect();

    // ── Step 2: deep-parse + extract nodes ───────────────────────────────────
    let parsed_units = parse_snapshot(snap);
    let mut objects: Vec<ObjectNode> = Vec::new();
    let mut routines: Vec<RoutineNode> = Vec::new();

    for unit in &parsed_units {
        // `intern` is idempotent — returns the same `AppRef` assigned in step 1.
        let app_ref = apps.intern(&unit.app);
        for pf in &unit.files {
            extract_nodes(
                app_ref,
                &pf.file,
                pf.provenance.tier,
                &mut objects,
                &mut routines,
            );
        }
    }

    // ── Step 3: wire real dependency topology ────────────────────────────────
    let mut topology = DependencyGraph::default();

    for (i, unit) in snap.apps.iter().enumerate() {
        let from_ref = app_refs[i];
        for dep in &unit.declared_deps {
            let dep_ref = if !dep.app_id.is_empty() {
                // Primary: match by GUID (globally unique).
                snap.apps
                    .iter()
                    .zip(app_refs.iter())
                    .find(|(u, _)| !u.id.guid.is_empty() && u.id.guid == dep.app_id)
                    .map(|(_, r)| *r)
            } else {
                // Fallback: name (case-insensitive) + version.
                snap.apps
                    .iter()
                    .zip(app_refs.iter())
                    .find(|(u, _)| {
                        u.id.name.eq_ignore_ascii_case(&dep.name) && u.id.version == dep.version
                    })
                    .map(|(_, r)| *r)
            };

            if let Some(dep_ref) = dep_ref {
                topology.add_dependency(from_ref, dep_ref);
            }
            // Deps not present in the snapshot are silently skipped (open-world).
        }
    }

    // ── Step 4: sort for determinism ─────────────────────────────────────────
    objects.sort_by(|a, b| a.id.cmp(&b.id));
    routines.sort_by(|a, b| a.id.cmp(&b.id));

    // ── Step 5: build index from sorted objects ───────────────────────────────
    let obj_index = ObjectIndex::build(&objects);

    ProgramGraph {
        apps,
        topology,
        objects,
        routines,
        obj_index,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_program_graph_over_cdo_workspace() {
        let Some(ws) = std::env::var_os("CDO_WS")
            .map(std::path::PathBuf::from)
            .filter(|p| p.exists())
        else {
            return;
        };

        let snap = crate::snapshot::SnapshotBuilder {
            workspace_root: ws,
            local_providers: vec![],
        }
        .build()
        .expect("snapshot");

        let g = build_program_graph(&snap);

        assert!(!g.objects.is_empty(), "expected objects from CDO workspace");

        // Workspace app should have a non-trivial dependency closure.
        let ws_ref = g
            .apps
            .find_by_name(&snap.workspace_app.name)
            .expect("workspace app must be interned");
        let closure = g.topology.closure(ws_ref);
        assert!(closure.len() > 1, "workspace should have ≥1 dependency");
    }
}
