//! Builds a `ProgramGraph` from an `AppSetSnapshot`.

use std::collections::{BTreeSet, HashMap};

use al_syntax::ir::ObjectKind;

use crate::program::abi_ingest::AbiCache;
use crate::program::graph::{ObjectIndex, ProgramGraph};
use crate::program::node::{AppRef, AppRegistry, RoutineNodeId};
use crate::program::node_extract::{AbiParams, Access, ObjectNode, RoutineNode, extract_nodes};
use crate::program::resolve::event::{
    PublisherKind, is_platform_page_event, is_platform_table_event, platform_event_display_name,
};
use crate::program::topology::DependencyGraph;
use crate::snapshot::{AppSetSnapshot, TrustTier, parse_snapshot};

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
    // whenever their `sig_fp` matches too (see node.rs). Post-Task-2
    // (sigfp-and-ambiguous-reclassification plan) `sig_fp` is a REAL
    // fingerprint of the parameter-type tuple — a genuine same-arity SOURCE
    // overload pair with distinguishable parameter types now gets DISTINCT
    // `sig_fp`s and sorts into separate runs entirely, so an id collision here
    // means either a true re-parse DUPLICATE (identical param types) or,
    // rarely, a residual fnv1a fingerprint COLLISION between two genuinely
    // different overloads — not a duplicate either way. A blanket `dedup_by`
    // would silently drop one of them with no record, and a later confident
    // `Source` route to the survivor would be a false-positive (the cardinal
    // sin this engine exists to avoid).
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
                // No real `[IntegrationEvent]` attribute to read (this
                // publisher is synthesized, not parsed) — and platform
                // DB-trigger/lifecycle events never legally prepend a Sender
                // anyway, so `None` correctly yields no `+1` tolerance via
                // `event::subscriber_arity_bound`.
                include_sender: None,
                abi_routine_kind: None,
                abi_event_kind: None,
                param_sig_key: String::new(),
                return_type: None,
                return_type_id: None,
                abi_overload_collapsed: false,
                source_overload_aliased: false,
                // Synthesized, not ingested from any `SymbolReference.json`
                // — no ABI parameter metadata exists to retain.
                abi_params: AbiParams::Missing,
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
/// share `(object, name_lc, enclosing_member_lc, params_count)` AND `sig_fp`
/// (see node.rs). Post-Task-2 (sigfp-and-ambiguous-reclassification plan)
/// `sig_fp` is a REAL fingerprint of the parameter-type tuple, so a genuine
/// same-name/same-arity SOURCE overload PAIR (two textually distinct
/// declarations differing only by parameter type) now almost always gets
/// DISTINCT ids and never reaches this run-grouping at all. The id alone
/// still cannot tell a re-parsed DUPLICATE of one routine (e.g. its owning
/// object embedded both as workspace source and as an embedded dep; see the
/// Step 4 comment above) apart from the RARE case where two genuinely
/// different overloads' normalized parameter tuples happen to hash to the
/// same `sig_fp` (a residual fnv1a collision) — this function's
/// `param_sig_key` text-based grouping (below) resolves both cases without
/// relying on `sig_fp` alone.
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
/// object copies = 4 raw entries kept instead of the canonical 2). Pre-Task-2
/// (source `sig_fp` was always `0`) the two canonical entries for a genuine
/// overload pair always shared one `RoutineNodeId`; post-Task-2 they only
/// still share one when a residual `sig_fp` collision applies (the common
/// case now gets distinct ids straight from sorting — see the doc comment
/// above). Either way `resolve_in_object`'s `>1` arm still returns an honest
/// `Unresolved` rather than guessing when a shared id survives with >1
/// canonical entry; only the CANONICAL COUNT is fixed here, not distinct node
/// identity (deferred overload-dispatch work). Never drops a genuine
/// collision silently for a SOURCE routine.
///
/// **ABI routines are a narrower case (Task 3 review fix; fp fidelity fixed
/// Task 2).** An ABI routine's `param_sig_key` is hardcoded `String::new()`
/// (see that field's doc on [`RoutineNode`]) — every ABI entry in a run
/// shares the SAME empty key, so a run of ≥2 raw ABI entries always
/// collapses to exactly ONE survivor here. Task 2 made
/// `abi_ingest::param_type_fp` (hence `RoutineNodeId::sig_fp`) fold a
/// length-delimited canonical tuple of EVERY parameter's outer kind +
/// Subtype id + raw Subtype name + a degradation tag (previously: only the
/// OUTER keyword, never a `Subtype` — the gap that let two genuinely
/// DIFFERENT ABI overloads differing only by an object-typed parameter's
/// Subtype silently share one `RoutineNodeId`). Post-fix, two entries reach
/// this function's SAME run only when their ENTIRE canonical tuple matched
/// — i.e. they are either a true re-parse duplicate, or a residual
/// fingerprint collision this engine genuinely cannot distinguish further
/// (round-2 addendum: "any residual same-key multi-entry group is
/// collapse-marked so collisions OVER-DECLINE, never select"). Either way,
/// collapsing to ONE survivor and marking it is the correct fail-closed
/// outcome: this function marks that survivor
/// [`RoutineNode::abi_overload_collapsed`] whenever ≥2 raw
/// `TrustTier::SymbolOnly` entries shared a node id, so a downstream
/// type-query (`resolver::routine_node_for_type_query` /
/// `receiver::receiver_from_routine_node`, Task 3's cross-object call-result
/// chain typing) AND plain dispatch (`resolver::resolve_in_object`'s
/// collapse-marker guard, Task 2 round-2) both decline rather than trust a
/// possibly-wrong candidate. `abi_overload_collapsed` is never set for a
/// SOURCE routine: its `param_sig_key` is real parsed param-type content, so
/// a genuine same-id/same-key collapse there is always a true re-parse
/// duplicate of the identical declaration.
///
/// **Source-overload alias marking (sigfp-and-ambiguous-reclassification
/// plan, Task 1; reframed by Task 2).** A run with ≥2 DISTINCT
/// `param_sig_key`s surviving under one `RoutineNodeId` is the source-tier
/// mirror of the ABI case above. Pre-Task-2 (source `sig_fp` was always `0`)
/// this was the GENERAL case for every same-name/same-arity overload pair;
/// post-Task-2 (`sig_fp` is a real parameter-type fingerprint) a genuine
/// overload pair almost always gets distinct ids and never reaches this
/// function at all — a run surviving here with ≥2 distinct keys now means
/// the two overloads' `sig_fp`s themselves collided (a residual fnv1a
/// fingerprint collision `source_param_sig_fp` cannot rule out; see
/// `sig_fp.rs`'s module doc). Neither survivor collapses (both are real, both
/// are kept — this function's whole point), but EVERY survivor in such a run
/// is marked [`RoutineNode::source_overload_aliased`] — a same-id/different-key
/// COLLISION GUARD consumed by
/// `resolver::emit_event_flow_edges`. A TRUE re-parse duplicate (one
/// distinct key in the run) collapses to a single unmarked survivor, same as
/// always. The two marker fields are mutually exclusive by construction:
/// this branch only ever fires for `r.tier != TrustTier::SymbolOnly`, whose
/// `param_sig_key` is never the ABI-only empty-key sentinel this function
/// collapses on.
fn dedup_routines_preserving_genuine_overloads(routines: &mut Vec<RoutineNode>) {
    let mut out: Vec<RoutineNode> = Vec::with_capacity(routines.len());
    let mut i = 0;
    while i < routines.len() {
        let mut j = i + 1;
        while j < routines.len() && routines[j].id == routines[i].id {
            j += 1;
        }
        // Count raw entries per param-signature key within this run FIRST —
        // a survivor is only markable once the true raw count sharing its
        // key is known (needed for the ABI empty-key case above). The
        // number of DISTINCT keys in the run (`sig_counts.len()`) is what
        // the source-overload-alias marking below needs: >=2 distinct keys
        // surviving under one id is a genuine aliased overload pair.
        let mut sig_counts: std::collections::HashMap<&str, usize> =
            std::collections::HashMap::new();
        for r in &routines[i..j] {
            *sig_counts.entry(r.param_sig_key.as_str()).or_insert(0) += 1;
        }
        let distinct_key_count = sig_counts.len();
        // Preserve first-occurrence order for determinism; collapse every
        // later entry in the run that repeats an already-seen param signature.
        let mut seen_sigs: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for r in &routines[i..j] {
            if seen_sigs.insert(r.param_sig_key.as_str()) {
                let mut survivor = r.clone();
                if r.tier == TrustTier::SymbolOnly && sig_counts[r.param_sig_key.as_str()] >= 2 {
                    survivor.abi_overload_collapsed = true;
                    // Task 2 (roadmap-closure plan): demote the retained ABI
                    // parameter list in LOCKSTEP with the collapse marker —
                    // the SAME survivor, the SAME "≥2 raw entries
                    // fingerprint-collided" condition. `abi_params` on this
                    // survivor belongs to only ONE of the ≥2 real
                    // declarations (arbitrary raw-JSON-order choice, same as
                    // `return_type`/`return_type_id` above) — the structural
                    // guard (`AbiParams::CollapsedUntrusted`) makes reading
                    // it for arg-type dispatch impossible by type, never
                    // merely a convention a future call site could forget.
                    survivor.abi_params = AbiParams::CollapsedUntrusted;
                } else if r.tier != TrustTier::SymbolOnly && distinct_key_count >= 2 {
                    survivor.source_overload_aliased = true;
                }
                out.push(survivor);
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

    /// Pin the post-Task-2 `source_overload_aliased` collision-guard-marked
    /// GROUP count on CDO (sigfp-and-ambiguous-reclassification plan, Task 2,
    /// T1-review fold-in). Pre-Task-2 (source `sig_fp` always `0`), CDO
    /// measured 6 primary / 313 whole-program ALIASED groups — every genuine
    /// same-name/same-arity overload pair, since none had distinct ids.
    /// Post-Task-2, a real overload pair gets a real distinct `sig_fp` and no
    /// longer reaches the guard at all; a NONZERO count here now means a
    /// genuine `sig_fp` NORMALIZATION COLLISION survived — a threshold
    /// alert to investigate, never to silently mask (round-1 addendum,
    /// collision-guard observability). `routines` is sorted by
    /// `RoutineNodeId`, so entries sharing one id are adjacent; a "group" is
    /// one maximal run of adjacent marked entries sharing the SAME id.
    #[test]
    fn source_overload_alias_collision_guard_group_count_pinned_on_cdo() {
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

        fn count_groups(marked: &[&RoutineNode]) -> usize {
            let mut groups = 0usize;
            let mut i = 0;
            while i < marked.len() {
                let mut j = i + 1;
                while j < marked.len() && marked[j].id == marked[i].id {
                    j += 1;
                }
                groups += 1;
                i = j;
            }
            groups
        }

        let primary_app_ref = g
            .apps
            .find_by_name(&snap.workspace_app.name)
            .expect("workspace app must be interned");

        let all_marked: Vec<&RoutineNode> = g
            .routines
            .iter()
            .filter(|r| r.source_overload_aliased)
            .collect();
        let primary_marked: Vec<&RoutineNode> = all_marked
            .iter()
            .copied()
            .filter(|r| r.id.object.app == primary_app_ref)
            .collect();

        let whole_program_groups = count_groups(&all_marked);
        let primary_groups = count_groups(&primary_marked);

        // Measured on CDO_WS 2026-07-03 (Task 2 landing): both `0` — every
        // real overload pair now gets a distinct sig_fp; zero residual
        // normalization collisions. Re-derive (never silently loosen) if
        // this ever moves — see the doc above.
        const EXPECTED_WHOLE_PROGRAM_GROUPS: usize = 0;
        const EXPECTED_PRIMARY_GROUPS: usize = 0;
        assert_eq!(
            whole_program_groups,
            EXPECTED_WHOLE_PROGRAM_GROUPS,
            "whole-program collision-guard-marked group count moved from the \
             pinned CDO baseline — investigate before re-pinning; marked: {:?}",
            all_marked
                .iter()
                .map(|r| (r.name.clone(), r.param_sig_key.clone()))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            primary_groups,
            EXPECTED_PRIMARY_GROUPS,
            "primary-scoped collision-guard-marked group count moved from the \
             pinned CDO baseline — investigate before re-pinning; marked: {:?}",
            primary_marked
                .iter()
                .map(|r| (r.name.clone(), r.param_sig_key.clone()))
                .collect::<Vec<_>>()
        );
    }

    // -----------------------------------------------------------------------
    // Task 3 review fix: `abi_overload_collapsed` marking
    // -----------------------------------------------------------------------

    use crate::program::node::{ObjKey, ObjectNodeId};

    fn dep_obj_id(app: AppRef, number: i64) -> ObjectNodeId {
        ObjectNodeId {
            app,
            kind: ObjectKind::Codeunit,
            key: ObjKey::Id(number),
        }
    }

    /// A minimal ABI-tier (`SymbolOnly`) `RoutineNode` — `param_sig_key`
    /// hardcoded empty exactly as `abi_ingest::ingest_abi` produces it.
    fn abi_routine(
        obj: &ObjectNodeId,
        name_lc: &str,
        params_count: usize,
        sig_fp: u64,
    ) -> RoutineNode {
        RoutineNode {
            id: RoutineNodeId {
                object: obj.clone(),
                name_lc: name_lc.to_string(),
                enclosing_member_lc: None,
                params_count,
                sig_fp,
            },
            name: name_lc.to_string(),
            is_trigger: false,
            access: Access::Public,
            tier: TrustTier::SymbolOnly,
            event_subscribers: vec![],
            subscriber_instance_manual: false,
            publisher_kind: None,
            include_sender: None,
            abi_routine_kind: None,
            abi_event_kind: None,
            param_sig_key: String::new(),
            return_type: None,
            return_type_id: None,
            abi_overload_collapsed: false,
            source_overload_aliased: false,
            abi_params: AbiParams::Missing,
        }
    }

    /// A minimal SOURCE (`Workspace`) `RoutineNode` carrying a real
    /// `param_sig_key` (content, never hardcoded empty for source) AND an
    /// explicit `sig_fp` — since sigfp-and-ambiguous-reclassification plan
    /// Task 2, SOURCE `sig_fp` is a real (non-zero for non-empty params)
    /// fingerprint, so tests that need to construct a SPECIFIC id relationship
    /// (distinct ids for a normal overload pair; a residual same-id
    /// collision) must control it explicitly rather than hardcoding `0`.
    fn source_routine(
        obj: &ObjectNodeId,
        name_lc: &str,
        params_count: usize,
        param_sig_key: &str,
        sig_fp: u64,
    ) -> RoutineNode {
        RoutineNode {
            id: RoutineNodeId {
                object: obj.clone(),
                name_lc: name_lc.to_string(),
                enclosing_member_lc: None,
                params_count,
                sig_fp,
            },
            name: name_lc.to_string(),
            is_trigger: false,
            access: Access::Public,
            tier: TrustTier::Workspace,
            event_subscribers: vec![],
            subscriber_instance_manual: false,
            publisher_kind: None,
            include_sender: None,
            abi_routine_kind: None,
            abi_event_kind: None,
            param_sig_key: param_sig_key.to_string(),
            return_type: None,
            return_type_id: None,
            abi_overload_collapsed: false,
            source_overload_aliased: false,
            abi_params: AbiParams::Missing,
        }
    }

    /// Two raw ABI entries sharing one `RoutineNodeId` (a genuine `sig_fp`
    /// collision — the degraded-Subtype scenario) collapse to ONE survivor,
    /// marked `abi_overload_collapsed`.
    #[test]
    fn abi_sig_fp_collision_marks_survivor_collapsed() {
        let app = AppRef(0);
        let obj = dep_obj_id(app, 60104);
        let mut routines = vec![
            abi_routine(&obj, "get", 1, 777),
            abi_routine(&obj, "get", 1, 777),
        ];
        dedup_routines_preserving_genuine_overloads(&mut routines);

        assert_eq!(
            routines.len(),
            1,
            "two raw entries sharing (object, name, arity, sig_fp) must collapse to one node"
        );
        assert!(
            routines[0].abi_overload_collapsed,
            "the survivor of a ≥2-raw-ABI-entry collapse must be flagged \
             abi_overload_collapsed so a chain type-query declines rather \
             than trust its return type"
        );
        assert_eq!(
            routines[0].abi_params,
            AbiParams::CollapsedUntrusted,
            "Task 2 (roadmap-closure plan): abi_params must demote to \
             CollapsedUntrusted in LOCKSTEP with abi_overload_collapsed — \
             the survivor's retained parameter list is exactly as \
             untrustworthy as its return type"
        );
    }

    /// A SINGLE ABI routine (no collision at all) must NEVER be marked —
    /// this is the `GetContent`-shaped real-world case (CDO's 2 real
    /// resolved chain edges) that must keep resolving after this fix.
    #[test]
    fn abi_single_routine_never_marked_collapsed() {
        let app = AppRef(0);
        let obj = dep_obj_id(app, 60100);
        let mut routines = vec![abi_routine(&obj, "getcontent", 0, 0)];
        dedup_routines_preserving_genuine_overloads(&mut routines);

        assert_eq!(routines.len(), 1);
        assert!(
            !routines[0].abi_overload_collapsed,
            "a lone ABI routine with no id collision must never be marked collapsed"
        );
    }

    /// Two DISTINCT `sig_fp`s (a genuinely non-degenerate ABI overload pair,
    /// e.g. differing OUTER param kind) never collide onto one
    /// `RoutineNodeId` in the first place — both survive as separate nodes,
    /// neither marked (there was no collapse to mark).
    #[test]
    fn abi_distinct_sig_fp_both_survive_unmarked() {
        let app = AppRef(0);
        let obj = dep_obj_id(app, 60103);
        let mut routines = vec![
            abi_routine(&obj, "get", 1, 111),
            abi_routine(&obj, "get", 1, 222),
        ];
        dedup_routines_preserving_genuine_overloads(&mut routines);

        assert_eq!(
            routines.len(),
            2,
            "distinct sig_fp means distinct RoutineNodeId — no collapse at all"
        );
        assert!(routines.iter().all(|r| !r.abi_overload_collapsed));
    }

    /// A genuine SOURCE re-parse duplicate (same declaration, non-empty
    /// matching `param_sig_key`) collapses exactly like before — but is
    /// NEVER marked `abi_overload_collapsed` (only `TrustTier::SymbolOnly`
    /// entries are eligible), since it is content-identical and trustworthy.
    #[test]
    fn source_duplicate_collapses_but_is_never_marked() {
        let app = AppRef(0);
        let obj = dep_obj_id(app, 51200);
        let mut routines = vec![
            source_routine(&obj, "name", 0, "", 0),
            source_routine(&obj, "name", 0, "", 0),
        ];
        dedup_routines_preserving_genuine_overloads(&mut routines);

        assert_eq!(routines.len(), 1);
        assert!(
            !routines[0].abi_overload_collapsed,
            "a SOURCE routine must never be marked, even when it collapses \
             on an empty param_sig_key (a genuine 0-arg re-parse duplicate)"
        );
    }

    // -----------------------------------------------------------------------
    // sigfp-and-ambiguous-reclassification plan, Task 2: the
    // `source_overload_aliased` marker's REFRAMED post-Task-2 role —
    // a same-id/different-`param_sig_key` COLLISION GUARD (T2 Step-1(d)).
    // -----------------------------------------------------------------------

    /// Normal case: two genuine same-name/same-arity SOURCE overloads with
    /// DISTINCT `sig_fp` (the ordinary Task 2 outcome — real overloads never
    /// share an id in the first place) never even reach the same dedup run —
    /// both survive UNMARKED. Mirrors `abi_distinct_sig_fp_both_survive_
    /// unmarked` for the source tier.
    #[test]
    fn source_distinct_sig_fp_overloads_survive_unmarked() {
        let app = AppRef(0);
        let obj = dep_obj_id(app, 51201);
        let mut routines = vec![
            source_routine(&obj, "resolve", 1, "integer", 111),
            source_routine(&obj, "resolve", 1, "text", 222),
        ];
        dedup_routines_preserving_genuine_overloads(&mut routines);

        assert_eq!(
            routines.len(),
            2,
            "distinct sig_fp means distinct RoutineNodeId — no shared run at all"
        );
        assert!(
            routines.iter().all(|r| !r.source_overload_aliased),
            "a normal distinct-id overload pair must NOT be marked \
             source_overload_aliased; got {:?}",
            routines
                .iter()
                .map(|r| r.source_overload_aliased)
                .collect::<Vec<_>>()
        );
    }

    /// Residual collision case: two entries whose `param_sig_key` CONTENT
    /// genuinely differs but whose `sig_fp` nonetheless collides (the
    /// normalization-collision scenario `sig_fp::source_param_sig_fp`'s doc
    /// warns is possible for two SPELLINGS of what a compiler would treat as
    /// the SAME type, e.g. differing internal whitespace — reachable in
    /// practice, see `sig_fp::tests::case_and_whitespace_variants_of_same_
    /// type_collide` for the fingerprint-level proof). Both survive (neither
    /// is a true duplicate), but BOTH are marked `source_overload_aliased`
    /// as a fail-closed COLLISION GUARD — the post-Task-2 role this field
    /// exists for.
    #[test]
    fn source_normalization_collision_marks_both_survivors_collision_guard() {
        let app = AppRef(0);
        let obj = dep_obj_id(app, 51202);
        let mut routines = vec![
            source_routine(&obj, "resolve", 1, "record customer", 999),
            source_routine(&obj, "resolve", 1, "record  customer", 999),
        ];
        dedup_routines_preserving_genuine_overloads(&mut routines);

        assert_eq!(
            routines.len(),
            2,
            "a same-id/different-param_sig_key run is NOT a true duplicate — \
             both entries must survive"
        );
        assert!(
            routines.iter().all(|r| r.source_overload_aliased),
            "every survivor of a same-id/different-param_sig_key collision \
             run must be marked source_overload_aliased (fail-closed); got {:?}",
            routines
                .iter()
                .map(|r| r.source_overload_aliased)
                .collect::<Vec<_>>()
        );
        assert!(
            routines.iter().all(|r| !r.abi_overload_collapsed),
            "the SOURCE collision guard is distinct from the ABI collapse \
             marker — a source routine must never be abi_overload_collapsed"
        );
    }

    // -----------------------------------------------------------------------
    // Task 3 (preprocessor foundations plan): the dedup interplay half of the
    // both-arms union-read pin. `al_syntax::lower` always emits TWO distinct
    // `RoutineDecl`s for a `#if`/`#else`-split procedure (see
    // `al_syntax::lower::tests::preproc_both_arms_distinct_signature_yield_
    // two_routine_decls`); THIS module's `dedup_routines_preserving_genuine_
    // overloads` is what decides whether they survive as two nodes (distinct
    // param types → distinct `sig_fp` → distinct `RoutineNodeId`, never even
    // reach a shared dedup run) or collapse to one (identical param types →
    // identical `sig_fp` → a true re-parse-shaped duplicate).
    // -----------------------------------------------------------------------

    /// Real AL text (not a hand-built fixture) through the FULL
    /// parse → extract_nodes → dedup pipeline: a `#if`/`#else` procedure pair
    /// with DIFFERING parameter types must survive as two distinct, UNMARKED
    /// `RoutineNode`s.
    #[test]
    fn preproc_both_arms_distinct_signature_yield_two_unmarked_source_overloads() {
        let src = r#"
codeunit 50300 "Preproc Overloads"
{
#if SOME_UNDEFINED_SYMBOL
    procedure Bar(X: Integer)
    begin
    end;
#else
    procedure Bar(Y: Text)
    begin
    end;
#endif
}
"#;
        let file = al_syntax::parse(src);
        let mut objects = Vec::new();
        let mut routines = Vec::new();
        extract_nodes(
            AppRef(0),
            &file,
            TrustTier::Workspace,
            &mut objects,
            &mut routines,
        );
        routines.sort_by(|a, b| a.id.cmp(&b.id));
        dedup_routines_preserving_genuine_overloads(&mut routines);

        let bar: Vec<_> = routines.iter().filter(|r| r.id.name_lc == "bar").collect();
        assert_eq!(
            bar.len(),
            2,
            "both #if/#else Bar arms are genuinely different-signature \
             overloads — both must survive; got: {:?}",
            bar.iter().map(|r| &r.param_sig_key).collect::<Vec<_>>()
        );
        assert_ne!(
            bar[0].id.sig_fp, bar[1].id.sig_fp,
            "Integer vs Text params must fingerprint distinctly"
        );
        assert!(
            bar.iter().all(|r| !r.source_overload_aliased),
            "distinct sig_fp means neither shares a dedup run — never marked"
        );
    }

    /// The other half: IDENTICAL parameter types across both `#if`/`#else`
    /// arms is the union-read's honest "same text, duplicated" case — the two
    /// `RoutineDecl`s share one `RoutineNodeId` (same `sig_fp`) and must
    /// collapse to ONE canonical, UNMARKED survivor (a true duplicate, not an
    /// overload — `source_overload_aliased` must never fire for it).
    #[test]
    fn preproc_same_signature_arms_collapse_to_one_unmarked_survivor() {
        let src = r#"
codeunit 50301 "Preproc Dup Sig"
{
#if SOME_UNDEFINED_SYMBOL
    procedure Baz(X: Integer)
    begin
    end;
#else
    procedure Baz(X: Integer)
    begin
    end;
#endif
}
"#;
        let file = al_syntax::parse(src);
        let mut objects = Vec::new();
        let mut routines = Vec::new();
        extract_nodes(
            AppRef(0),
            &file,
            TrustTier::Workspace,
            &mut objects,
            &mut routines,
        );
        routines.sort_by(|a, b| a.id.cmp(&b.id));
        dedup_routines_preserving_genuine_overloads(&mut routines);

        let baz: Vec<_> = routines.iter().filter(|r| r.id.name_lc == "baz").collect();
        assert_eq!(
            baz.len(),
            1,
            "identical-signature #if/#else arms are the SAME procedure \
             textually duplicated by the union-read — must collapse to one \
             canonical entry, not survive as two"
        );
        assert!(
            !baz[0].source_overload_aliased,
            "a true re-parse-shaped duplicate must never be marked \
             source_overload_aliased (that marker is for genuine overload \
             collisions only)"
        );
    }
}
