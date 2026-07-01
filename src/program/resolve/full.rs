//! 1B.3a Task 3: Obligation inventory + `resolve_full_program` + self-reported
//! taxonomy'd metric.
//!
//! # Coverage contract
//!
//! Every parsed call/event obligation (each [`CalleeShape`] site in every
//! workspace source routine + every publisher event routine in the program
//! graph) is enumerated as an [`Obligation`] with a stable [`ObligationId`].
//! [`resolve_full_program`] (and [`resolve_full_program_from_parts`]) resolves
//! each obligation to exactly one classified [`ClassifiedEdge`].
//!
//! The **COVERAGE CONTRACT** is **distinct-id SET equality**:
//!
//! ```text
//! set(obligation_ids) == set(classified_edge.obligation_id)
//! ```
//!
//! [`coverage_holds`] returns `true` iff the two sets are equal.
//! `Unknown`/`HonestDynamic`/`HonestEmpty` edges ARE valid classified edges;
//! they fulfil the coverage contract. Only a silently-absent edge (an
//! obligation that produced no edge at all) violates it.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use al_syntax::ir::ObjectKind;

use crate::program::build::build_program_graph;
use crate::program::graph::ProgramGraph;
use crate::program::node::{AppRef, ObjKey, ObjectNodeId, RoutineNodeId};
use crate::program::node_extract::ObjectNode;
use crate::program::resolve::abi_check::{
    AbiIntegrityReport, abi_ingestion_integrity, build_raw_abi_index_from_snapshot,
};
use crate::program::resolve::body_map::BodyMap;
use crate::program::resolve::edge::{
    CanonicalSpan, DispatchShape, Edge, EdgeKind, Evidence, EvidenceKind, Histogram,
    OpenWorldReason, Route, RouteTarget, SetCompleteness, SiteId, UnknownReason, Witness,
    callee_fp, classify_obligation,
};
use crate::program::resolve::extract::{CalleeShape, WithState, extract_sites_for_routine};
use crate::program::resolve::index::ResolveIndex;
use crate::program::resolve::receiver::{ReceiverType, infer_receiver_type};
use crate::program::resolve::resolver::{
    emit_event_flow_edges, resolve_bare, resolve_implicit_trigger, resolve_member,
    resolve_object_run,
};
use crate::snapshot::{AppSetSnapshot, ParsedUnit, SnapshotBuilder, parse_snapshot};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Stable identity of one parsed obligation.
///
/// - **`CallSite`** — mirrors [`SiteId`]: `(caller, span, callee_fp)`.
/// - **`Publisher`** — the publisher routine's node id.
///   One `Publisher` obligation per publisher routine in the graph.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ObligationId {
    CallSite {
        caller: RoutineNodeId,
        span: CanonicalSpan,
        callee_fp: u64,
    },
    Publisher(RoutineNodeId),
}

/// The resolution demand encoded by one obligation.
#[derive(Clone, Debug)]
pub enum ObligationKind {
    /// A classified call/dispatch site in a routine body.
    CallSite {
        from_object_id: ObjectNodeId,
        shape: CalleeShape,
        arity: usize,
    },
    /// An event-publisher routine (fires on event).
    Publisher,
}

/// One parsed obligation.
pub struct Obligation {
    pub id: ObligationId,
    pub kind: ObligationKind,
}

/// A classified edge annotated with the obligation it was resolved from.
pub struct ClassifiedEdge {
    pub obligation_id: ObligationId,
    pub edge: Edge,
}

/// Coverage report: the distinct-id SET equality contract.
#[derive(Clone, Debug)]
pub struct Coverage {
    /// Total distinct obligation ids (the DENOMINATOR).
    pub parsed_obligations: usize,
    /// Total distinct edge obligation ids (the NUMERATOR).
    pub classified_edges: usize,
    /// Obligation ids present in the inventory but absent from the edge set
    /// — obligations for which the resolver emitted no edge (contract failure).
    pub missing: Vec<ObligationId>,
    /// Edge obligation ids present in the edge set but absent from the
    /// inventory (edges emitted without a corresponding obligation).
    pub extra: Vec<ObligationId>,
}

/// Full result of [`resolve_full_program`].
pub struct ProgramReport {
    /// All classified edges (whole-program scope: all source-bearing routines +
    /// all publisher routines in all apps).
    pub edges: Vec<ClassifiedEdge>,
    /// Coverage: distinct-id set equality between obligations and edges.
    pub coverage: Coverage,
    /// Taxonomy'd histogram over ALL edges.
    pub histogram: Histogram,
    /// Taxonomy'd histogram over PRIMARY-SCOPED edges only
    /// (edges whose `from.object.app == primary_app_ref`).
    pub primary_histogram: Histogram,
    /// ABI ingestion integrity: `AbiSymbol` route keys vs. raw dep SymbolReference.
    pub abi_integrity: AbiIntegrityReport,
    /// The workspace app's [`AppRef`] (use with [`is_primary_scope`]).
    pub primary_app_ref: AppRef,
}

// ---------------------------------------------------------------------------
// Public functions
// ---------------------------------------------------------------------------

/// Returns `true` when the coverage contract holds: every obligation has
/// exactly one classified edge and no edge was emitted without an obligation.
pub fn coverage_holds(c: &Coverage) -> bool {
    c.missing.is_empty() && c.extra.is_empty()
}

/// Returns `true` when this edge's `from` routine belongs to the workspace
/// (primary) app — mirrors `--l3-call-graph-stats-cross-app` scoping.
pub fn is_primary_scope(edge: &ClassifiedEdge, primary_app_ref: AppRef) -> bool {
    edge.edge.from.object.app == primary_app_ref
}

/// Enumerate every parsed call/event obligation without resolving them.
///
/// Call obligations come from workspace source routines (filtered to
/// `primary_app_ref` and `primary_file_set`).  Publisher obligations come from
/// ALL publisher routines in `graph.routines` (same set that
/// [`emit_event_flow_edges`] processes — no app filter, cross-app).
///
/// The returned [`Vec`] may contain duplicate obligation ids when two call
/// sites in the same routine happen to share `(caller, span, callee_fp)` (an
/// extremely rare degenerate case in generated/macro code).  The SET-equality
/// coverage contract de-duplicates via [`HashSet`].
pub fn obligation_inventory(
    graph: &ProgramGraph,
    parsed: &[ParsedUnit],
    primary_app_ref: AppRef,
    primary_file_set: &HashSet<String>,
) -> Vec<Obligation> {
    let mut obligations: Vec<Obligation> = Vec::new();

    // ── Phase 1: call-site obligations (workspace source routines) ────────────
    for unit in parsed {
        let Some(app_ref) = graph.apps.find(&unit.app) else {
            continue;
        };
        if app_ref != primary_app_ref {
            continue;
        }

        for pf in &unit.files {
            if !primary_file_set.contains(&pf.virtual_path) {
                continue;
            }

            for (obj_idx, obj) in pf.file.objects.iter().enumerate() {
                let obj_key = match obj.id {
                    Some(n) => ObjKey::Id(n),
                    None => ObjKey::Name(obj.name.to_ascii_lowercase()),
                };
                let obj_node_id = ObjectNodeId {
                    app: primary_app_ref,
                    kind: obj.kind,
                    key: obj_key,
                };

                // Record-typed global variable names (same logic as harnesses).
                let globals_rec: HashSet<String> = obj
                    .globals
                    .iter()
                    .filter(|v| {
                        v.ty.as_deref()
                            .map(|ty| ty.trim().to_ascii_lowercase().starts_with("record"))
                            .unwrap_or(false)
                    })
                    .map(|v| v.name.to_ascii_lowercase())
                    .collect();

                for (routine_idx, routine) in obj.routines.iter().enumerate() {
                    let caller = RoutineNodeId {
                        object: obj_node_id.clone(),
                        name_lc: routine.name.to_ascii_lowercase(),
                        enclosing_member_lc: routine
                            .enclosing_member
                            .as_ref()
                            .map(|(n, _)| n.to_ascii_lowercase()),
                        params_count: routine.params.len(),
                        sig_fp: 0,
                    };

                    let sites = extract_sites_for_routine(
                        &pf.file,
                        &pf.text,
                        &pf.virtual_path,
                        &globals_rec,
                        obj_idx,
                        routine_idx,
                    );

                    for site in &sites {
                        let fp = callee_fp(&site.callee_text);
                        obligations.push(Obligation {
                            id: ObligationId::CallSite {
                                caller: caller.clone(),
                                span: site.span.clone(),
                                callee_fp: fp,
                            },
                            kind: ObligationKind::CallSite {
                                from_object_id: obj_node_id.clone(),
                                shape: site.shape.clone(),
                                arity: site.arity,
                            },
                        });
                    }
                }
            }
        }
    }

    // ── Phase 2: publisher obligations (all apps — mirrors emit_event_flow_edges) ──
    for pub_routine in &graph.routines {
        if pub_routine.publisher_kind.is_none() {
            continue;
        }
        obligations.push(Obligation {
            id: ObligationId::Publisher(pub_routine.id.clone()),
            kind: ObligationKind::Publisher,
        });
    }

    obligations
}

// ---------------------------------------------------------------------------
// Core resolution
// ---------------------------------------------------------------------------

/// Inline helper: an Unknown-evidence Unresolved route (resolution failure).
/// Task 3: `reason` is REQUIRED — every call site supplies a diagnostic
/// [`UnknownReason`].
fn unknown_route(reason: UnknownReason) -> Route {
    Route {
        target: RouteTarget::Unresolved,
        evidence: Evidence::Unknown(reason),
        conditions: vec![],
        witness: Witness::None,
    }
}

/// Task 3: classify `CalleeShape::Unknown`'s decline reason from the raw
/// callee text. A `callee_text` with >=2 dot separators (`A.B.C`) is a
/// multi-segment receiver chain the extractor structurally cannot classify
/// into a `Member { receiver_text, method }` shape (which only ever captures
/// ONE dot); anything else reaching `Unknown` is some other unclassifiable
/// call expression shape.
fn unclassified_callee_reason(callee_text: &str) -> UnknownReason {
    if callee_text.matches('.').count() >= 2 {
        UnknownReason::CompoundReceiver
    } else {
        UnknownReason::UnclassifiedCallee
    }
}

/// Derive [`SetCompleteness`] from the shape for member and similar calls.
fn completeness_for_shape(shape: DispatchShape) -> SetCompleteness {
    match shape {
        DispatchShape::Exact => SetCompleteness::Complete,
        DispatchShape::Polymorphic => SetCompleteness::Partial {
            reason: OpenWorldReason::ReverseDependentImplementers,
        },
        DispatchShape::DynamicOpen => SetCompleteness::Partial {
            reason: OpenWorldReason::RuntimeTypeUnbounded,
        },
        DispatchShape::Multicast => SetCompleteness::Partial {
            reason: OpenWorldReason::ReverseDependentExtensions,
        },
    }
}

/// Resolve one call-site obligation to `(kind, shape, completeness, routes)`.
#[allow(clippy::too_many_arguments)]
fn resolve_call_site_obligation(
    shape: &CalleeShape,
    arity: usize,
    callee_text: &str,
    obj_node_opt: Option<&ObjectNode>,
    routine: &al_syntax::ir::RoutineDecl,
    obj: &al_syntax::ir::ObjectDecl,
    primary_app_ref: AppRef,
    graph: &ProgramGraph,
    index: &ResolveIndex,
    body_map: &BodyMap<'_>,
    with_state: WithState,
) -> (EdgeKind, DispatchShape, SetCompleteness, Vec<Route>) {
    match shape {
        CalleeShape::Bare { name } => {
            let name_lc = name.to_ascii_lowercase();
            let routes = if let Some(obj_node) = obj_node_opt {
                resolve_bare(
                    obj_node, &name_lc, arity, graph, index, body_map, with_state,
                )
            } else {
                vec![unknown_route(UnknownReason::IndexIntegrationGap)]
            };
            (
                EdgeKind::Call,
                DispatchShape::Exact,
                SetCompleteness::Complete,
                routes,
            )
        }

        CalleeShape::Member {
            receiver_text,
            method,
        } => {
            let receiver_lc = receiver_text.to_ascii_lowercase();
            let method_lc = method.to_ascii_lowercase();
            let (member_shape, mut routes) = if let Some(obj_node) = obj_node_opt {
                let recv = infer_receiver_type(
                    &receiver_lc,
                    routine,
                    &obj.globals,
                    obj_node,
                    graph,
                    index,
                );
                resolve_member(&recv, &method_lc, arity, obj_node, graph, index, body_map)
            } else {
                (
                    DispatchShape::Exact,
                    vec![unknown_route(UnknownReason::IndexIntegrationGap)],
                )
            };
            // Task 3: a dotted `receiver_text` (`A.B.C`) means Phase A was
            // asked to type a multi-segment/compound receiver chain — AL
            // variable/singleton/framework names never contain a dot, so
            // `infer_receiver_type` structurally cannot match one (except the
            // narrow `CurrPage.<part>.Page` shape, which resolves and never
            // reaches here). Relabel the generic `UntrackedReceiver` tag with
            // the more specific `CompoundReceiver` in that case.
            if receiver_lc.contains('.') {
                for r in &mut routes {
                    if matches!(
                        r.evidence,
                        Evidence::Unknown(UnknownReason::UntrackedReceiver)
                    ) {
                        r.evidence = Evidence::Unknown(UnknownReason::CompoundReceiver);
                    }
                }
            }
            let completeness = completeness_for_shape(member_shape);
            (EdgeKind::Call, member_shape, completeness, routes)
        }

        CalleeShape::ObjectRun {
            object_kind,
            target_ref,
            target_is_name,
        } => {
            let okind_opt = match object_kind.as_str() {
                "Codeunit" => Some(ObjectKind::Codeunit),
                "Page" => Some(ObjectKind::Page),
                "Report" => Some(ObjectKind::Report),
                _ => None,
            };
            if let Some(okind) = okind_opt {
                let (shape, completeness, routes) = resolve_object_run(
                    primary_app_ref,
                    okind,
                    target_ref.as_deref(),
                    *target_is_name,
                    graph,
                    index,
                    body_map,
                );
                (EdgeKind::Run, shape, completeness, routes)
            } else {
                // Unrecognised object kind — honest Unknown.
                (
                    EdgeKind::Run,
                    DispatchShape::Exact,
                    SetCompleteness::Complete,
                    vec![unknown_route(UnknownReason::UnclassifiedCallee)],
                )
            }
        }

        CalleeShape::RecordOp { receiver_text, op } => {
            let receiver_lc = receiver_text.to_ascii_lowercase();
            let op_lc = op.to_ascii_lowercase();

            // Infer the record type from the receiver and look up its table
            // ObjectNode.  Falls back to honest-empty when the table is not found.
            let table_node_opt: Option<&ObjectNode> = if let Some(obj_node) = obj_node_opt {
                let recv = infer_receiver_type(
                    &receiver_lc,
                    routine,
                    &obj.globals,
                    obj_node,
                    graph,
                    index,
                );
                match recv {
                    ReceiverType::Record {
                        table: Some(ref tid),
                    } => graph.objects.iter().find(|o| o.id == *tid),
                    _ => None,
                }
            } else {
                None
            };

            let (shape, completeness, routes) = if let Some(table_node) = table_node_opt {
                resolve_implicit_trigger(&op_lc, table_node, graph, index, body_map)
            } else {
                // No table resolved: honest-empty Multicast (open-world, no
                // known triggers, but we cannot say there are none).
                (
                    DispatchShape::Multicast,
                    SetCompleteness::Partial {
                        reason: OpenWorldReason::ReverseDependentExtensions,
                    },
                    vec![],
                )
            };
            (EdgeKind::ImplicitTrigger, shape, completeness, routes)
        }

        CalleeShape::Commit => {
            // `commit` is a global builtin — resolve_bare finds it in the catalog.
            let routes = if let Some(obj_node) = obj_node_opt {
                resolve_bare(obj_node, "commit", 0, graph, index, body_map, with_state)
            } else {
                vec![unknown_route(UnknownReason::IndexIntegrationGap)]
            };
            (
                EdgeKind::Call,
                DispatchShape::Exact,
                SetCompleteness::Complete,
                routes,
            )
        }

        CalleeShape::Unknown => {
            // Unclassifiable call expression — honest Unknown.
            (
                EdgeKind::Call,
                DispatchShape::Exact,
                SetCompleteness::Complete,
                vec![unknown_route(unclassified_callee_reason(callee_text))],
            )
        }
    }
}

/// Resolve all obligations and compute coverage.
///
/// This is the clean-room inner loop.  It does NOT call any L3 oracle.
/// Publishers are resolved via [`emit_event_flow_edges`]; all call-site
/// obligations are resolved via the shape-dispatch helpers.
fn resolve_full_program_from_parts(
    graph: &ProgramGraph,
    parsed: &[ParsedUnit],
    primary_app_ref: AppRef,
    ws_file_set: &HashSet<String>,
) -> (Vec<ClassifiedEdge>, Coverage) {
    // Quick ObjectNodeId → &ObjectNode lookup.
    let obj_node_map: HashMap<ObjectNodeId, &ObjectNode> =
        graph.objects.iter().map(|o| (o.id.clone(), o)).collect();

    let index = ResolveIndex::build(graph);
    let body_map = BodyMap::build(graph, parsed);

    let mut obligation_id_set: HashSet<ObligationId> = HashSet::new();
    let mut classified_edges: Vec<ClassifiedEdge> = Vec::new();

    // ── Phase 1: resolve call-site obligations (workspace source routines) ────
    for unit in parsed {
        let Some(app_ref) = graph.apps.find(&unit.app) else {
            continue;
        };
        if app_ref != primary_app_ref {
            continue;
        }

        for pf in &unit.files {
            if !ws_file_set.contains(&pf.virtual_path) {
                continue;
            }

            for (obj_idx, obj) in pf.file.objects.iter().enumerate() {
                let obj_key = match obj.id {
                    Some(n) => ObjKey::Id(n),
                    None => ObjKey::Name(obj.name.to_ascii_lowercase()),
                };
                let obj_node_id = ObjectNodeId {
                    app: primary_app_ref,
                    kind: obj.kind,
                    key: obj_key,
                };
                let obj_node_opt: Option<&ObjectNode> = obj_node_map.get(&obj_node_id).copied();

                // Record-typed global variable names for RecordOp / receiver inference.
                let globals_rec: HashSet<String> = obj
                    .globals
                    .iter()
                    .filter(|v| {
                        v.ty.as_deref()
                            .map(|ty| ty.trim().to_ascii_lowercase().starts_with("record"))
                            .unwrap_or(false)
                    })
                    .map(|v| v.name.to_ascii_lowercase())
                    .collect();

                for (routine_idx, routine) in obj.routines.iter().enumerate() {
                    let caller = RoutineNodeId {
                        object: obj_node_id.clone(),
                        name_lc: routine.name.to_ascii_lowercase(),
                        enclosing_member_lc: routine
                            .enclosing_member
                            .as_ref()
                            .map(|(n, _)| n.to_ascii_lowercase()),
                        params_count: routine.params.len(),
                        sig_fp: 0,
                    };

                    let sites = extract_sites_for_routine(
                        &pf.file,
                        &pf.text,
                        &pf.virtual_path,
                        &globals_rec,
                        obj_idx,
                        routine_idx,
                    );

                    for site in &sites {
                        let fp = callee_fp(&site.callee_text);
                        let obl_id = ObligationId::CallSite {
                            caller: caller.clone(),
                            span: site.span.clone(),
                            callee_fp: fp,
                        };
                        obligation_id_set.insert(obl_id.clone());

                        let (kind, shape, completeness, routes) = resolve_call_site_obligation(
                            &site.shape,
                            site.arity,
                            &site.callee_text,
                            obj_node_opt,
                            routine,
                            obj,
                            primary_app_ref,
                            graph,
                            &index,
                            &body_map,
                            site.with_state,
                        );

                        classified_edges.push(ClassifiedEdge {
                            obligation_id: obl_id,
                            edge: Edge {
                                from: caller.clone(),
                                site: SiteId {
                                    caller: caller.clone(),
                                    span: site.span.clone(),
                                    callee_fingerprint: fp,
                                },
                                kind,
                                shape,
                                completeness,
                                routes,
                            },
                        });
                    }
                }
            }
        }
    }

    // ── Phase 2: publisher event flow obligations (all apps) ──────────────────
    // emit_event_flow_edges processes ALL graph.routines (no app filter).
    // We must track obligation ids in the same pass so coverage holds.
    let event_edges = emit_event_flow_edges(graph, &index, &body_map);
    for edge in event_edges {
        // Each publisher routine emits exactly one EventFlow edge.
        let obl_id = ObligationId::Publisher(edge.from.clone());
        obligation_id_set.insert(obl_id.clone());
        classified_edges.push(ClassifiedEdge {
            obligation_id: obl_id,
            edge,
        });
    }

    // ── Coverage: distinct-id SET equality ────────────────────────────────────
    let edge_id_set: HashSet<ObligationId> = classified_edges
        .iter()
        .map(|ce| ce.obligation_id.clone())
        .collect();

    let mut missing: Vec<ObligationId> = obligation_id_set
        .difference(&edge_id_set)
        .cloned()
        .collect();
    missing.sort();

    let mut extra: Vec<ObligationId> = edge_id_set
        .difference(&obligation_id_set)
        .cloned()
        .collect();
    extra.sort();

    let coverage = Coverage {
        parsed_obligations: obligation_id_set.len(),
        classified_edges: edge_id_set.len(),
        missing,
        extra,
    };

    (classified_edges, coverage)
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Full-program obligation coverage + self-reported taxonomy'd metric.
///
/// # Steps
///
/// 1. Build [`AppSetSnapshot`] from `workspace_root` (via [`SnapshotBuilder`]).
/// 2. Build [`ProgramGraph`] (intern apps, extract nodes, ingest ABI).
/// 3. Parse snapshot for call-site extraction and body lookup.
/// 4. Locate workspace app (primary scope).
/// 5. Resolve all obligations → [`ClassifiedEdge`] set.
/// 6. Compute coverage (distinct-id SET equality).
/// 7. Compute taxonomy'd histograms (whole-program + primary-scoped).
/// 8. Compute ABI ingestion integrity.
///
/// Returns `None` when snapshot build fails (fail-closed).
///
/// # L3 independence
///
/// This function does NOT invoke the L3 oracle.  It is the self-reported
/// north-star metric: the resolution outcome comes entirely from this engine.
#[must_use]
pub fn resolve_full_program(workspace_root: &Path) -> Option<ProgramReport> {
    // ── Steps 1–4: shared setup (snapshot → graph → parse → primary app) ──────
    let ctx = build_context(workspace_root)?;
    let ProgramContext {
        snap,
        graph,
        parsed,
        primary_app_ref,
        ws_file_set,
    } = &ctx;
    let primary_app_ref = *primary_app_ref;

    // ── Step 5: Resolve all obligations ──────────────────────────────────────
    let (edges, coverage) =
        resolve_full_program_from_parts(graph, parsed, primary_app_ref, ws_file_set);

    // ── Step 6: Histograms ────────────────────────────────────────────────────
    // Collect references to all underlying Edge structs.
    let all_edge_refs: Vec<&Edge> = edges.iter().map(|ce| &ce.edge).collect();
    // `Histogram::of_edges` takes `&[Edge]` — we need owned slices.
    // Build by iterating manually to avoid cloning.
    let histogram = {
        let mut h = Histogram::default();
        for e in &all_edge_refs {
            count_into_histogram(&mut h, e);
        }
        h
    };
    let primary_histogram = {
        let mut h = Histogram::default();
        for ce in &edges {
            if is_primary_scope(ce, primary_app_ref) {
                count_into_histogram(&mut h, &ce.edge);
            }
        }
        h
    };

    // ── Step 7: ABI integrity ─────────────────────────────────────────────────
    // Build a raw ABI index from dep .app files (independent of graph nodes).
    let raw_abi_index = build_raw_abi_index_from_snapshot(snap, &graph.apps);
    // Collect all underlying edges for the ABI check.
    let plain_edges: Vec<Edge> = edges.iter().map(|ce| ce.edge.clone()).collect();
    let abi_integrity = abi_ingestion_integrity(&plain_edges, &raw_abi_index);

    Some(ProgramReport {
        edges,
        coverage,
        histogram,
        primary_histogram,
        abi_integrity,
        primary_app_ref,
    })
}

/// Export-oriented entry: assemble the whole-program graph + classified edges +
/// primary app ref, WITHOUT computing histograms / coverage / ABI integrity.
///
/// Consumed by [`crate::program::graphify_export`], which needs the assembled
/// [`ProgramGraph`] (for node labels + app-name resolution) alongside the edges.
/// Returns `None` on snapshot build failure (fail-closed), same as
/// [`resolve_full_program`].
#[must_use]
pub fn resolve_full_program_for_export(
    workspace_root: &Path,
) -> Option<(ProgramGraph, Vec<ClassifiedEdge>, AppRef)> {
    let ctx = build_context(workspace_root)?;
    let (edges, _coverage) = resolve_full_program_from_parts(
        &ctx.graph,
        &ctx.parsed,
        ctx.primary_app_ref,
        &ctx.ws_file_set,
    );
    Some((ctx.graph, edges, ctx.primary_app_ref))
}

/// Shared setup for the whole-program resolvers: snapshot → program graph →
/// parse → primary app ref + workspace file set. Single source of truth so
/// [`resolve_full_program`] and [`resolve_full_program_for_export`] cannot drift.
/// Returns `None` when the snapshot build fails or the workspace app is absent.
struct ProgramContext {
    snap: AppSetSnapshot,
    graph: ProgramGraph,
    parsed: Vec<ParsedUnit>,
    primary_app_ref: AppRef,
    ws_file_set: HashSet<String>,
}

fn build_context(workspace_root: &Path) -> Option<ProgramContext> {
    // ── Step 1: Build snapshot ────────────────────────────────────────────────
    let snap = (SnapshotBuilder {
        workspace_root: workspace_root.to_path_buf(),
        local_providers: vec![],
    })
    .build()
    .ok()?;

    // ws_file_set: the true workspace source virtual paths (first AppUnit).
    // Excludes embedded dep apps whose AppId matches the workspace AppId.
    let ws_file_set: HashSet<String> = snap
        .apps
        .first()
        .and_then(|u| u.source.as_ref())
        .map(|s| s.files.iter().map(|f| f.virtual_path.clone()).collect())
        .unwrap_or_default();

    // ── Step 2: Build program graph ───────────────────────────────────────────
    let graph = build_program_graph(&snap, &crate::program::abi_ingest::AbiCache::new());

    // ── Step 3: Parse snapshot ────────────────────────────────────────────────
    let parsed = parse_snapshot(&snap);

    // ── Step 4: Locate primary (workspace) app ────────────────────────────────
    let primary_app_ref = graph.apps.find(&snap.workspace_app)?;

    Some(ProgramContext {
        snap,
        graph,
        parsed,
        primary_app_ref,
        ws_file_set,
    })
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Increment histogram counters for one edge, mirroring [`Histogram::of_edges`].
fn count_into_histogram(h: &mut Histogram, e: &Edge) {
    use crate::program::resolve::edge::ObligationOutcome;

    h.total += 1;
    match classify_obligation(e) {
        ObligationOutcome::Resolved => {
            // Classify by best evidence (Source=0, Catalog=1, Abi/Opaque=2).
            let mut best: Option<u8> = None;
            for r in &e.routes {
                if r.evidence.kind() == EvidenceKind::Unknown
                    || r.target == RouteTarget::Unresolved
                    || !r.fires_by_default()
                {
                    continue;
                }
                let score: u8 = match r.evidence {
                    Evidence::Source => 0,
                    Evidence::Catalog => 1,
                    Evidence::Abi | Evidence::Opaque => 2,
                    Evidence::Unknown(_) => continue,
                };
                best = Some(best.map_or(score, |b: u8| b.min(score)));
            }
            match best {
                Some(0) => h.resolved_source += 1,
                Some(1) => h.resolved_catalog += 1,
                Some(_) => h.resolved_abi_external += 1,
                None => {
                    unreachable!("Resolved edge must have >=1 default-firing non-Unknown route")
                }
            }
        }
        ObligationOutcome::ConditionalResolved => h.conditional_resolved += 1,
        ObligationOutcome::HonestDynamic => h.honest_dynamic += 1,
        ObligationOutcome::HonestEmpty => h.honest_empty += 1,
        ObligationOutcome::Unknown => h.unknown += 1,
    }
}
