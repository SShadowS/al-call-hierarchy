//! Builds a `ProgramGraph` from an `AppSetSnapshot`.

use std::collections::{BTreeSet, HashMap};

use crate::program::abi_ingest::AbiCache;
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
///    object + routine nodes; then ingest SymbolOnly dep ABI nodes from
///    `abi_cache` (step 2b).
/// 3. Wire the real dependency topology from each unit's `declared_deps`
///    (GUID-match preferred; name+version fallback; deps absent from the
///    snapshot are silently skipped — open-world assumption); then (step 3b)
///    wire `internalsVisibleTo` friend-app authorizations from each unit's
///    `internals_visible_to` (Task 1.5) the same way — GUID-match preferred,
///    name+publisher fallback (a `<Module>` friend entry carries no
///    version), friends absent from the snapshot silently skipped.
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

    // ── Step 3b: wire internalsVisibleTo friend-app authorizations ──────────
    // AL: a member declared `internal` is visible within its declaring app AND
    // to any app the declaring app's manifest lists in `<InternalsVisibleTo>`
    // (a "friend" app) — one-directional per the app EXPOSING the internals,
    // never the reverse. `friends` is keyed by the exposing app's `AppRef` so
    // `resolver.rs`'s `Access::Internal` visibility rule can do an O(1)
    // `friends.get(&declaring_app).is_some_and(|f| f.contains(&caller_app))`
    // check alongside the existing same-app check (Task 1.5).
    let mut friends: HashMap<AppRef, BTreeSet<AppRef>> = HashMap::new();
    for (i, unit) in snap.apps.iter().enumerate() {
        let exposing_ref = app_refs[i];
        for friend in &unit.internals_visible_to {
            // Same GUID-first, name+publisher-fallback resolution as Step 3's
            // dependency wiring above — a `<Module>` friend entry carries no
            // version, so the fallback compares publisher instead.
            let by_guid = (!friend.app_id.is_empty())
                .then(|| {
                    snap.apps
                        .iter()
                        .zip(app_refs.iter())
                        .find(|(u, _)| !u.id.guid.is_empty() && u.id.guid == friend.app_id)
                        .map(|(_, r)| *r)
                })
                .flatten();
            let friend_ref = by_guid.or_else(|| {
                snap.apps
                    .iter()
                    .zip(app_refs.iter())
                    .find(|(u, _)| {
                        u.id.name.eq_ignore_ascii_case(&friend.name)
                            && u.id.publisher.eq_ignore_ascii_case(&friend.publisher)
                    })
                    .map(|(_, r)| *r)
            });

            if let Some(friend_ref) = friend_ref {
                friends.entry(exposing_ref).or_default().insert(friend_ref);
            }
            // Friends not present in the snapshot are silently skipped (open-world).
        }
    }

    // ── Step 4: sort for determinism, then dedup ─────────────────────────────
    // Same app can appear as both a workspace source and an embedded dep (e.g.
    // sibling apps in a multi-app workspace whose compiled .app lands in
    // .alpackages).  extract_nodes would process the source twice, producing
    // duplicate ObjectNode/RoutineNode entries with identical ids.  Dedup after
    // sort keeps the first occurrence (arbitrary but stable).
    //
    // Objects dedup unconditionally on id — an `ObjectNode` carries no content
    // that can distinguish a re-parse duplicate from anything else, and two
    // objects sharing an id are always the same object.
    //
    // Routines need more care: two DISTINCT source procedures sharing
    // `(object, name_lc, params_count)` also collide onto one `RoutineNodeId`
    // (source `sig_fp` is always `0` — see node.rs) — a genuine same-arity
    // SOURCE overload collision (beyond-1B.3b Task 2), not a duplicate. A
    // blanket `dedup_by` would silently drop one of them with no record, and a
    // later confident `Source` route to the survivor would be a false-positive
    // (the cardinal sin this engine exists to avoid).
    // `dedup_routines_preserving_genuine_overloads` (below) tells the two
    // apart by parameter-type CONTENT rather than by counting how many times
    // the enclosing object was duplicated (beyond-1B.3b Task 2 review fix: the
    // former dup-factor heuristic under-collapsed when both a whole-object
    // re-parse AND a genuine overload collision applied to the same run).
    objects.sort_by(|a, b| a.id.cmp(&b.id));
    objects.dedup_by(|a, b| a.id == b.id);
    routines.sort_by(|a, b| a.id.cmp(&b.id));
    dedup_routines_preserving_genuine_overloads(&mut routines);

    // ── Step 5: build index from sorted objects ───────────────────────────────
    let obj_index = ObjectIndex::build(&objects);

    ProgramGraph {
        apps,
        topology,
        objects,
        routines,
        obj_index,
        friends,
    }
}

/// Collapse a SORTED `routines` vec's runs of equal `RoutineNodeId` down to
/// one canonical entry PER DISTINCT parameter-type signature.
///
/// Two SOURCE routines collide onto the same `RoutineNodeId` whenever they
/// share `(object, name_lc, enclosing_member_lc, params_count)` — source
/// `sig_fp` is always `0` (see node.rs) — so the id alone cannot tell a
/// re-parsed DUPLICATE of one routine (e.g. its owning object embedded both
/// as workspace source and as an embedded dep; see the Step 4 comment above)
/// apart from a genuine same-name/same-arity SOURCE overload PAIR (two
/// textually distinct declarations differing only by parameter type).
///
/// Within a run, entries are grouped by [`RoutineNode::param_sig_key`] — the
/// lowercased, `|`-joined parameter-type-text sequence (mirrors
/// `abi_ingest::param_type_fp`'s normalization, computed for source params).
/// Each distinct key collapses to its first occurrence (arbitrary but
/// stable); a run with N distinct keys yields exactly N canonical entries.
///
/// This is correct independent of how many times the enclosing object itself
/// was duplicated: two re-parses of the SAME declaration always share a
/// param signature and collapse together, while two genuinely distinct
/// overloads always differ in param signature and are both preserved — even
/// in the COMPOUND case where an object is both duplicated AND declares a
/// genuine overload pair (beyond-1B.3b Task 2 review fix: the previous
/// dup-factor heuristic under-collapsed that case, e.g. 2 overloads × 2
/// object copies = 4 raw entries kept instead of the canonical 2). The two
/// canonical entries preserved for a genuine overload pair still share one
/// `RoutineNodeId` (source `sig_fp` stays `0`) — `resolve_in_object`'s `>1`
/// arm still returns an honest `Unresolved` rather than guessing; only the
/// CANONICAL COUNT is fixed here, not distinct node identity (deferred
/// overload-dispatch work). Never drops a genuine collision silently.
fn dedup_routines_preserving_genuine_overloads(routines: &mut Vec<RoutineNode>) {
    let mut out: Vec<RoutineNode> = Vec::with_capacity(routines.len());
    let mut i = 0;
    while i < routines.len() {
        let mut j = i + 1;
        while j < routines.len() && routines[j].id == routines[i].id {
            j += 1;
        }
        // Preserve first-occurrence order for determinism; collapse every
        // later entry in the run that repeats an already-seen param signature.
        let mut seen_sigs: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for r in &routines[i..j] {
            if seen_sigs.insert(r.param_sig_key.as_str()) {
                out.push(r.clone());
            }
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
