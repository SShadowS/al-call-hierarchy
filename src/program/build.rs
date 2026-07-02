//! Builds a `ProgramGraph` from an `AppSetSnapshot`.

use std::collections::{BTreeSet, HashMap};

use al_syntax::ir::ObjectKind;

use crate::program::abi_ingest::AbiCache;
use crate::program::graph::{ObjectIndex, ProgramGraph};
use crate::program::node::{AppRef, AppRegistry, RoutineNodeId};
use crate::program::node_extract::{Access, ObjectNode, RoutineNode, extract_nodes};
use crate::program::resolve::event::{
    PublisherKind, is_platform_page_event, is_platform_table_event, platform_event_display_name,
};
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

    let mut graph = ProgramGraph {
        apps,
        topology,
        objects,
        routines,
        obj_index,
        friends,
    };

    // ── Step 6: inject synthetic platform-event publishers ────────────────────
    // Binds subscribers to the table's implicit DB-trigger / validate events,
    // which have no publisher routine in source (see below).
    inject_platform_event_publishers(&mut graph);

    graph
}

/// Synthetic platform-publisher arity: a generous upper bound so the resolve
/// index's `params_count >= sub_params` candidate filter admits every real
/// subscriber. Platform events top out at ~3 params and are never overloaded,
/// so the exact value only needs to dominate any subscriber's arity.
const PLATFORM_EVENT_PUBLISHER_ARITY: usize = 8;

/// Inject synthetic [`PublisherKind::Platform`] publisher routines for the
/// platform-generated TABLE events (`OnAfter*Event` / `OnBefore*Event` + field
/// validate) that a subscriber targets but which have NO publisher routine in
/// source.
///
/// Without these, a `[EventSubscriber(ObjectType::Table, Database::X,
/// 'OnAfterDeleteEvent', …)]` resolves to no publisher (the resolve index needs
/// a `publisher_kind`-bearing routine), so its integration edge — "this
/// subscriber fires when X is deleted", the charter's data-is-control-flow
/// wiring — is silently lost. On real BC apps this orphans ~27% of subscribers.
///
/// One synthetic per distinct `(table, event_name)`; field-level (`element`)
/// granularity is intentionally collapsed so the index's `(object, name)`
/// candidate model resolves each to exactly one publisher (per-field precision
/// is a later refinement). Events already served by a real publisher routine on
/// the table are skipped — a synthetic never shadows source.
pub(crate) fn inject_platform_event_publishers(graph: &mut ProgramGraph) {
    let mut synth: Vec<RoutineNode> = Vec::new();
    let mut seen: std::collections::HashSet<RoutineNodeId> = std::collections::HashSet::new();

    for sub in &graph.routines {
        for args in &sub.event_subscribers {
            // Recognize the platform TABLE and PAGE events (implicit DB triggers /
            // field validate / page lifecycle / record / action) that have no
            // publisher routine in source. Everything else resolves through the
            // normal `[IntegrationEvent]` publisher path.
            let pub_kind = match args.publisher_object_type.as_str() {
                "table" if is_platform_table_event(&args.event_name) => ObjectKind::Table,
                "page" if is_platform_page_event(&args.event_name) => ObjectKind::Page,
                _ => continue,
            };
            // Resolve the publisher object from the subscriber's app (fail-closed).
            let Some(pub_obj) =
                graph.resolve_object(sub.id.object.app, pub_kind, &args.publisher_name)
            else {
                continue;
            };
            let synth_id = RoutineNodeId {
                object: pub_obj.id.clone(),
                name_lc: args.event_name.clone(),
                enclosing_member_lc: None,
                params_count: PLATFORM_EVENT_PUBLISHER_ARITY,
                sig_fp: 0,
            };
            if !seen.insert(synth_id.clone()) {
                continue; // already injected for this (table, event)
            }
            // Never shadow a real source publisher of the same name on the table.
            let has_real_publisher = graph.routines.iter().any(|r| {
                r.id.object == pub_obj.id
                    && r.id.name_lc == args.event_name
                    && r.publisher_kind.is_some()
            });
            if has_real_publisher {
                continue;
            }
            synth.push(RoutineNode {
                id: synth_id,
                name: platform_event_display_name(&args.event_name).to_string(),
                is_trigger: false,
                access: Access::Public,
                tier: pub_obj.tier,
                event_subscribers: vec![],
                subscriber_instance_manual: false,
                publisher_kind: Some(PublisherKind::Platform),
                abi_routine_kind: None,
                abi_event_kind: None,
                param_sig_key: String::new(),
                return_type: None,
                return_type_id: None,
            });
        }
    }

    if synth.is_empty() {
        return;
    }
    graph.routines.extend(synth);
    graph.routines.sort_by(|a, b| a.id.cmp(&b.id));
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
