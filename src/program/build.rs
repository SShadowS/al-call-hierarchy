//! Builds a `ProgramGraph` from an `AppSetSnapshot`.

use std::collections::HashMap;

use crate::program::abi_ingest::AbiCache;
use crate::program::graph::{ObjectIndex, ProgramGraph};
use crate::program::node::{AppRef, AppRegistry, ObjectNodeId};
use crate::program::node_extract::{ObjectNode, RoutineNode, extract_nodes};
use crate::program::topology::DependencyGraph;
use crate::snapshot::{AppSetSnapshot, parse_snapshot};

/// Assemble a `ProgramGraph` from a fully-resolved `AppSetSnapshot`.
///
/// Steps:
/// 1. Intern every app identity from the snapshot into an `AppRegistry`.
/// 2. Deep-parse all source-bearing units (via `parse_snapshot`) and extract
///    object + routine nodes; then ingest SymbolOnly dep ABI nodes from
///    `abi_cache` (step 2b).
/// 3. Wire the real dependency topology from each unit's `declared_deps`
///    (GUID-match preferred; name+version fallback; deps absent from the
///    snapshot are silently skipped — open-world assumption).
/// 4. Sort `objects` and `routines` by node-id for determinism.
/// 5. Build the `ObjectIndex` from the sorted `objects`.
pub fn build_program_graph(snap: &AppSetSnapshot, abi_cache: &AbiCache) -> ProgramGraph {
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

    // ── Step 2b: ingest SymbolOnly dep ABI nodes ─────────────────────────────
    for unit in &snap.apps {
        if unit.source.is_some() {
            continue;
        }
        let app_ref = apps.intern(&unit.id);
        let (new_objs, new_routs) =
            crate::program::abi_ingest::ingest_abi(unit, app_ref, abi_cache);
        objects.extend(new_objs);
        routines.extend(new_routs);
    }

    // ── Step 3: wire real dependency topology ────────────────────────────────
    let mut topology = DependencyGraph::default();

    for (i, unit) in snap.apps.iter().enumerate() {
        let from_ref = app_refs[i];
        for dep in &unit.declared_deps {
            // Match the dep to a snapshot app: GUID is globally unique and tried
            // first; fall through to name (case-insensitive) + version when no
            // GUID match (e.g. a snapshot unit whose manifest GUID was unavailable).
            let by_guid = (!dep.app_id.is_empty())
                .then(|| {
                    snap.apps
                        .iter()
                        .zip(app_refs.iter())
                        .find(|(u, _)| !u.id.guid.is_empty() && u.id.guid == dep.app_id)
                        .map(|(_, r)| *r)
                })
                .flatten();
            let dep_ref = by_guid.or_else(|| {
                snap.apps
                    .iter()
                    .zip(app_refs.iter())
                    .find(|(u, _)| {
                        u.id.name.eq_ignore_ascii_case(&dep.name) && u.id.version == dep.version
                    })
                    .map(|(_, r)| *r)
            });

            if let Some(dep_ref) = dep_ref {
                topology.add_dependency(from_ref, dep_ref);
            }
            // Deps not present in the snapshot are silently skipped (open-world).
        }
    }

    // ── Step 4: sort for determinism, then dedup ─────────────────────────────
    // Same app can appear as both a workspace source and an embedded dep (e.g.
    // sibling apps in a multi-app workspace whose compiled .app lands in
    // .alpackages).  extract_nodes would process the source twice, producing
    // duplicate ObjectNode/RoutineNode entries with identical ids.  Dedup after
    // sort keeps the first occurrence (arbitrary but stable).
    //
    // Count each object's raw duplication factor BEFORE any dedup runs — it is
    // the yardstick `dedup_routines_preserving_genuine_overloads` (below) uses
    // to tell that legitimate whole-file re-parse apart from a genuine
    // same-arity SOURCE overload collision (beyond-1B.3b Task 2). Two DISTINCT
    // source procedures sharing `(object, name_lc, params_count)` collide onto
    // one `RoutineNodeId` (source `sig_fp` is always `0` — see node.rs), so a
    // blanket `dedup_by` would silently drop one of them with no record. A
    // later confident `Source` route to the survivor would then be a
    // false-positive (the cardinal sin this engine exists to avoid).
    let mut obj_dup_counts: HashMap<ObjectNodeId, usize> = HashMap::new();
    for o in &objects {
        *obj_dup_counts.entry(o.id.clone()).or_insert(0) += 1;
    }

    objects.sort_by(|a, b| a.id.cmp(&b.id));
    objects.dedup_by(|a, b| a.id == b.id);
    routines.sort_by(|a, b| a.id.cmp(&b.id));
    dedup_routines_preserving_genuine_overloads(&mut routines, &obj_dup_counts);

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

/// Collapse a SORTED `routines` vec's runs of equal `RoutineNodeId` down to
/// the enclosing object's raw duplication factor — never below it.
///
/// A run whose length is fully explained by `obj_dup_counts` (the object
/// itself was extracted that many times — e.g. a whole file re-parsed
/// because its app appears as both workspace source and embedded dep; see
/// the Step 4 comment above) collapses to one canonical entry, exactly like
/// the previous blanket `dedup_by`. A run LONGER than that factor holds
/// genuinely DISTINCT source routines that collided onto one `RoutineNodeId`
/// (beyond-1B.3b Task 2: source `sig_fp` is always `0`, so two same-arity
/// overloads are indistinguishable at the id level) — every entry in that
/// excess is preserved so `ResolveIndex`/`resolve_in_object` observe the true
/// candidate count downstream and can fail closed instead of guessing.
/// Never drops a genuine collision silently.
fn dedup_routines_preserving_genuine_overloads(
    routines: &mut Vec<RoutineNode>,
    obj_dup_counts: &HashMap<ObjectNodeId, usize>,
) {
    let mut out: Vec<RoutineNode> = Vec::with_capacity(routines.len());
    let mut i = 0;
    while i < routines.len() {
        let mut j = i + 1;
        while j < routines.len() && routines[j].id == routines[i].id {
            j += 1;
        }
        let run_len = j - i;
        let obj_dup = obj_dup_counts
            .get(&routines[i].id.object)
            .copied()
            .unwrap_or(1)
            .max(1);
        if run_len > obj_dup {
            // Genuine overload collision (or a compound case with BOTH a
            // whole-file re-parse AND a genuine collision) — keep every raw
            // entry so the true ambiguity is visible downstream.
            out.extend(routines[i..j].iter().cloned());
        } else {
            // Fully explained by whole-file re-parse (or no duplication at
            // all) — collapse to a single canonical entry, as before.
            out.push(routines[i].clone());
        }
        i = j;
    }
    *routines = out;
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

        let cache = AbiCache::new();
        let g = build_program_graph(&snap, &cache);

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
