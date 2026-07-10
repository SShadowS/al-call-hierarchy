//! 1B.3a Task 3: `resolve_full_program` + self-reported taxonomy'd metric.
//!
//! # Coverage contract
//!
//! Every parsed call/event obligation (each [`CalleeShape`] site in every
//! workspace source routine + every publisher event routine in the program
//! graph) gets a stable [`ObligationId`], tracked INLINE during the single
//! resolution pass in [`resolve_full_program_from_parts`] (an
//! `obligation_id_set` built alongside `classified_edges` — see that
//! function's body). [`resolve_full_program`] resolves each obligation to
//! exactly one classified [`ClassifiedEdge`].
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
//!
//! (Historical note, sigfp-and-ambiguous-reclassification plan Task 2: a
//! separate `pub fn obligation_inventory` used to enumerate obligations as a
//! standalone pre-pass — reviewer-confirmed DEAD CODE with zero callers
//! outside its own definition (coverage was, and is, computed by the inline
//! tracking above, never by comparing against that separate enumeration).
//! Its own [`RoutineNodeId`] reconstruction was one of the 5 audited
//! `sig_fp`-hardcoded-`0` sites; since it had no live caller, it was deleted
//! rather than migrated to [`crate::program::sig_fp::source_routine_node_id`].)

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
use crate::program::resolve::arg_dispatch::{self, ArgDispatchInfo};
use crate::program::resolve::body_map::BodyMap;
use crate::program::resolve::edge::{
    CanonicalSpan, DispatchShape, Edge, EdgeKind, Evidence, EvidenceKind, Histogram,
    OpenWorldReason, Route, RouteTarget, SetCompleteness, SiteId, UnknownReason, Witness,
    callee_fp, classify_obligation,
};
use crate::program::resolve::extract::{
    CalleeShape, WithState, extract_sites_for_routine, static_database_reference_target,
};
use crate::program::resolve::index::ResolveIndex;
use crate::program::resolve::member_catalog::is_entry_dispatch_builtin;
use crate::program::resolve::receiver::{
    FrameworkKind, ReceiverType, infer_receiver_type, is_atomic_receiver_token,
};
use crate::program::resolve::resolver::{
    emit_event_flow_edges, resolve_bare, resolve_bare_with_args, resolve_implicit_trigger,
    resolve_member_with_args, resolve_object_run,
};
use crate::program::sig_fp::source_routine_node_id;
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

/// A classified edge annotated with the obligation it was resolved from.
pub struct ClassifiedEdge {
    pub obligation_id: ObligationId,
    pub edge: Edge,
}

// ---------------------------------------------------------------------------
// T0.3: builtin-dispatch justification audit (diagnostic-only)
// ---------------------------------------------------------------------------

/// One call site whose `Route` resolved to `RouteTarget::Builtin` via a
/// [`crate::program::resolve::member_catalog::ENTRY_DISPATCH_BUILTIN_IDS`]
/// entry AND whose target object is PROVEN statically named — a missed
/// entry-trigger dispatch (T0.3; see that const's doc for the classifier
/// gaps this makes visible). `object` is `"{ObjectKind}::{name_lc}"`
/// (e.g. `"Page::some page"`), always lowercased for deterministic sorting
/// regardless of which extraction path produced it (a declared receiver's
/// own type, or a call argument's `Page::"X"` reference).
///
/// Diagnostic-only: never consulted by `classify_obligation`/
/// `ObligationOutcome`, never compared against a semantic golden — does not
/// change any route/edge/histogram.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct FlaggedBuiltinDispatchSite {
    pub file: String,
    pub object: String,
    pub method: String,
    pub line: u32,
}

/// A call site whose method is in
/// [`crate::program::resolve::member_catalog::ENTRY_DISPATCH_BUILTIN_IDS`]
/// and whose route resolved to `Builtin`, but whose target could NOT be
/// proven statically (fail-closed — e.g. a runtime variable/expression
/// argument, or a receiver shape the audit does not attempt to prove).
/// Reported so the flagged population is honest about what it excludes,
/// never silently dropped. Diagnostic-only (see [`FlaggedBuiltinDispatchSite`]'s doc).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct IndeterminateBuiltinDispatchSite {
    pub file: String,
    pub method: String,
    pub line: u32,
}

/// T0.3 builtin-dispatch justification audit output: the deterministic,
/// sorted `flagged`/`indeterminate` populations produced by
/// [`resolve_full_program`]. See [`FlaggedBuiltinDispatchSite`]'s doc.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BuiltinDispatchAudit {
    pub flagged: Vec<FlaggedBuiltinDispatchSite>,
    pub indeterminate: Vec<IndeterminateBuiltinDispatchSite>,
}

/// Per-call-site signal threaded out of [`resolve_call_site_obligation`] for
/// the T0.3 audit — populated ONLY by the `CalleeShape::Member` arm (every
/// other arm returns `None`); see [`builtin_dispatch_finding`].
enum BuiltinDispatchFinding {
    Flagged { object: String, method: String },
    Indeterminate { method: String },
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
    /// Count of publisher `EventFlow` edges SKIPPED by [`resolver::
    /// emit_event_flow_edges`]'s Task 1 dual-publisher source-overload-alias
    /// collision guard (sigfp-and-ambiguous-reclassification plan) —
    /// [`resolver::dual_publisher_alias_skip_count`]. Expected `0` outside
    /// the CDO-measured known dual-publisher pairs; a nonzero value beyond
    /// those signatures is a threshold alert (investigate, don't mask —
    /// collision-guard-observability addendum).
    pub event_flow_dual_publisher_alias_skips: usize,
    /// File paths of every parsed source file whose parse hit tree-sitter
    /// error recovery (`ParseStatus::Recovered`) — Task 3 (preprocessor
    /// foundations plan). ADDITIVE diagnostic, never gates resolution: see
    /// [`crate::snapshot::parse::recovered_file_paths`]'s doc for the
    /// absence-claim invariant this surfaces. Expected empty on a
    /// well-formed workspace; any entry means that file's IR may be missing
    /// content tree-sitter could not parse.
    pub recovered_files: Vec<String>,
    /// T0.3 builtin-dispatch justification audit — see [`BuiltinDispatchAudit`]'s
    /// doc. ADDITIVE diagnostic: never consulted by `histogram`/
    /// `classify_obligation`, does not change any route/edge.
    pub builtin_dispatch_audit: BuiltinDispatchAudit,
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
        receiver_tier: None,
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
        // Task 3 (sigfp-and-ambiguous-reclassification plan): a same-object
        // overload-ambiguity candidate set is a SNAPSHOT-ENUMERATED, CLOSED
        // set — unlike Polymorphic's open-world reverse-dependent
        // implementers, no future dependent app can add another overload
        // candidate to an already-compiled object. `Complete`, not `Partial`.
        DispatchShape::AmbiguousOverload => SetCompleteness::Complete,
    }
}

/// T0.3 builtin-dispatch audit: classify one `CalleeShape::Member` call's
/// ALREADY-RESOLVED `routes` for the "entry-dispatching builtin absorbed a
/// statically-named target" bug class (see
/// `member_catalog::ENTRY_DISPATCH_BUILTIN_IDS`'s doc for the two classifier
/// gaps this makes visible). Returns `None` when no route in `routes`
/// actually landed on a flagged catalog entry — a no-op for the
/// overwhelming majority of member calls, including every OTHER
/// `PageInstance`/`ReportInstance` method (`SetRecord`, `Caption`, …).
///
/// Fail-closed (T0.3 constraint): a flagged method whose target cannot be
/// PROVEN static returns `Indeterminate`, never a guessed `Flagged`.
///
/// - `recv == ReceiverType::Object { kind, name_lc, .. }` (a declared
///   Page/Report-typed variable/param/global receiver, or the `CurrPage.
///   <part>.Page` subpage shape): the target is the receiver's OWN resolved
///   type — 100% proven, no argument inspection needed. `Flagged`.
/// - `recv == ReceiverType::Framework(PageInstance | ReportInstance)` (the
///   literal `Page`/`CurrPage`/`Report`/`CurrReport` singleton receiver,
///   `receiver.rs:714-715`): the target can ONLY come from a
///   `Page::"X"`/`Report::"X"`-shaped first argument
///   ([`static_database_reference_target`]). `Flagged` when present,
///   `Indeterminate` otherwise (e.g. a runtime variable/expression argument
///   — dynamic dispatch, or zero args — `CurrPage`/`CurrReport` self-dispatch,
///   deliberately not claimed as a foreign target by this audit).
/// - Any other receiver shape reaching a flagged route (not expected given
///   `member_catalog.rs`'s receiver-name-gated `Framework` mapping, but
///   fail-closed rather than assumed impossible): `Indeterminate`.
fn builtin_dispatch_finding(
    recv: &ReceiverType,
    method_lc: &str,
    routes: &[Route],
    file: &al_syntax::ir::AlFile,
    call_args: &[al_syntax::ir::ExprId],
) -> Option<BuiltinDispatchFinding> {
    let flagged = routes.iter().any(|r| match &r.target {
        RouteTarget::Builtin(bid) => is_entry_dispatch_builtin(bid),
        _ => false,
    });
    if !flagged {
        return None;
    }
    match recv {
        ReceiverType::Object { kind, name_lc, .. } => Some(BuiltinDispatchFinding::Flagged {
            object: format!("{kind:?}::{name_lc}"),
            method: method_lc.to_string(),
        }),
        ReceiverType::Framework(
            fk @ (FrameworkKind::PageInstance | FrameworkKind::ReportInstance),
        ) => {
            let kind_str = match fk {
                FrameworkKind::PageInstance => "Page",
                FrameworkKind::ReportInstance => "Report",
                _ => unreachable!("guarded by the outer match arm"),
            };
            match static_database_reference_target(file, call_args) {
                Some((target, _target_is_name)) => Some(BuiltinDispatchFinding::Flagged {
                    object: format!("{kind_str}::{}", target.to_ascii_lowercase()),
                    method: method_lc.to_string(),
                }),
                None => Some(BuiltinDispatchFinding::Indeterminate {
                    method: method_lc.to_string(),
                }),
            }
        }
        _ => Some(BuiltinDispatchFinding::Indeterminate {
            method: method_lc.to_string(),
        }),
    }
}

/// Resolve one call-site obligation to `(kind, shape, completeness, routes,
/// builtin_dispatch_finding)`. The 5th element is the T0.3 audit signal
/// (`Some` only from the `CalleeShape::Member` arm — see
/// [`builtin_dispatch_finding`]); every other arm returns `None`.
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
    // Task 2 enabling primitive: the parsed `AlFile` this obligation's call
    // site was extracted from, so a `CalleeShape::Member.receiver` `ExprId`
    // can be dereferenced into `infer_receiver_type`'s `receiver_expr` param.
    // Task 3 is the first consumer (Step 5, `Func().Method()` compound
    // receivers) — Steps 0-4 remain unaffected.
    file: &al_syntax::ir::AlFile,
    // argtype-dispatch-and-page-catalog plan, Task 2: the call site's raw
    // argument expression ids (`RawSiteV2::args`), typed ONCE below into
    // `ArgDispatchInfo` and threaded to `resolve_bare_with_args`/
    // `resolve_member_with_args` so `resolve_in_object`'s fail-closed pick
    // has real argument evidence to work with.
    call_args: &[al_syntax::ir::ExprId],
) -> (
    EdgeKind,
    DispatchShape,
    SetCompleteness,
    Vec<Route>,
    Option<BuiltinDispatchFinding>,
) {
    // Built ONCE per obligation (not per-arm): SOURCE-tier only (`arg_
    // dispatch`'s own SymbolOnly gate lives in `resolve_in_object`, but
    // there is nothing to type at all without a resolved calling object).
    // Task 2 review fix: `with_state` threads into arg typing too — a bare-
    // identifier arg can be REBOUND by an enclosing `with` block, exactly
    // the hazard `resolve_bare`'s Step 3 with-guard already exists to close
    // for bare CALLS (see `arg_dispatch`'s module doc, "`with`-scope gate
    // for bare-identifier args").
    let args_info: Vec<ArgDispatchInfo> = match obj_node_opt {
        Some(obj_node) => arg_dispatch::type_call_args(
            call_args,
            file,
            routine,
            &obj.globals,
            &obj_node.id,
            graph,
            index,
            body_map,
            with_state,
        ),
        None => Vec::new(),
    };

    match shape {
        CalleeShape::Bare { name } => {
            let name_lc = name.to_ascii_lowercase();
            // Task 4 (sigfp-and-ambiguous-reclassification plan): thread the
            // REAL shape `resolve_bare` determined through — a bare call is
            // `DispatchShape::Exact` in every case except a genuine
            // same-object overload ambiguity, which is now
            // `DispatchShape::AmbiguousOverload` (previously hardcoded
            // `Exact` unconditionally, which would have mislabeled the
            // multi-route ambiguous case). `completeness_for_shape` maps
            // BOTH `Exact` and `AmbiguousOverload` to `SetCompleteness::
            // Complete`, so this is behavior-preserving for every other
            // shape.
            let (shape, routes) = if let Some(obj_node) = obj_node_opt {
                resolve_bare_with_args(
                    obj_node, &name_lc, arity, graph, index, body_map, with_state, &args_info,
                )
            } else {
                (
                    DispatchShape::Exact,
                    vec![unknown_route(UnknownReason::IndexIntegrationGap)],
                )
            };
            (
                EdgeKind::Call,
                shape,
                completeness_for_shape(shape),
                routes,
                None,
            )
        }

        CalleeShape::Member {
            receiver_text,
            method,
            receiver,
        } => {
            let receiver_lc = receiver_text.to_ascii_lowercase();
            let method_lc = method.to_ascii_lowercase();
            let mut finding: Option<BuiltinDispatchFinding> = None;
            let (member_shape, mut routes) = if let Some(obj_node) = obj_node_opt {
                let recv = infer_receiver_type(
                    &receiver_lc,
                    routine,
                    &obj.globals,
                    obj_node,
                    graph,
                    index,
                    receiver.map(|id| (file, id)),
                    Some((body_map, with_state)),
                );
                let (s, r) = resolve_member_with_args(
                    &recv, &method_lc, arity, obj_node, graph, index, body_map, &args_info,
                );
                finding = builtin_dispatch_finding(&recv, &method_lc, &r, file, call_args);
                (s, r)
            } else {
                (
                    DispatchShape::Exact,
                    vec![unknown_route(UnknownReason::IndexIntegrationGap)],
                )
            };
            // Task 3: a COMPOUND `receiver_text` (`A.B.C`, an UNQUOTED `.`
            // segment separator) means Phase A was asked to type a
            // multi-segment/compound receiver chain — AL variable/singleton/
            // framework/dataitem names never contain an unquoted dot, so
            // `infer_receiver_type` structurally cannot match one (except the
            // narrow `CurrPage.<part>.Page` shape, which resolves and never
            // reaches here). Relabel the generic `UntrackedReceiver` tag with
            // the more specific `CompoundReceiver` in that case.
            //
            // `is_atomic_receiver_token` (dataitem-receivers plan, Task 1)
            // replaces the naive `receiver_lc.contains('.')` check here: a
            // QUOTED receiver with an EMBEDDED period
            // (`"Sales Cr.Memo Header Filter"`) is a single ATOMIC identifier,
            // not a compound chain, so it must NOT be relabeled
            // `CompoundReceiver` — the naive check mislabeled it before this
            // fix, hiding a real dataitem-name receiver behind the wrong
            // Unknown reason.
            if !is_atomic_receiver_token(&receiver_lc) {
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
            (EdgeKind::Call, member_shape, completeness, routes, finding)
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
                (EdgeKind::Run, shape, completeness, routes, None)
            } else {
                // Unrecognised object kind — honest Unknown.
                (
                    EdgeKind::Run,
                    DispatchShape::Exact,
                    SetCompleteness::Complete,
                    vec![unknown_route(UnknownReason::UnclassifiedCallee)],
                    None,
                )
            }
        }

        CalleeShape::RecordOp { receiver_text, op } => {
            let receiver_lc = receiver_text.to_ascii_lowercase();
            let op_lc = op.to_ascii_lowercase();

            // Infer the record type from the receiver and look up its table
            // ObjectNode.  Falls back to honest-empty when the table is not found.
            let table_node_opt: Option<&ObjectNode> = if let Some(obj_node) = obj_node_opt {
                // `RecordOp` carries no `ExprId` (Task 2 scoped the primitive
                // to `CalleeShape::Member` only) — `None`/`None` here is
                // unchanged behavior, not a gap (Task 3's Step 5 is also
                // scoped to `CalleeShape::Member`).
                let recv = infer_receiver_type(
                    &receiver_lc,
                    routine,
                    &obj.globals,
                    obj_node,
                    graph,
                    index,
                    None,
                    None,
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
            (EdgeKind::ImplicitTrigger, shape, completeness, routes, None)
        }

        CalleeShape::Commit => {
            // `commit` is a global builtin — resolve_bare finds it in the
            // catalog (Step 4). Threading the real shape through (Task 4)
            // rather than hardcoding `Exact` costs nothing here (Step 4
            // always yields `Exact`) and stays consistent with the `Bare`
            // arm above for the case an object declares its OWN overloaded
            // 0-arity `commit` procedure (Step 1 would then reach it before
            // Step 4 ever runs) — structurally impossible in valid AL
            // (`Commit` is a reserved statement keyword; no compiling AL
            // source can declare a procedure that collides with it), so
            // this arm stays defensive-only rather than a live path any
            // real CDO/workspace source can reach.
            let (shape, routes) = if let Some(obj_node) = obj_node_opt {
                resolve_bare(obj_node, "commit", 0, graph, index, body_map, with_state)
            } else {
                (
                    DispatchShape::Exact,
                    vec![unknown_route(UnknownReason::IndexIntegrationGap)],
                )
            };
            (
                EdgeKind::Call,
                shape,
                completeness_for_shape(shape),
                routes,
                None,
            )
        }

        CalleeShape::Unknown => {
            // Unclassifiable call expression — honest Unknown.
            (
                EdgeKind::Call,
                DispatchShape::Exact,
                SetCompleteness::Complete,
                vec![unknown_route(unclassified_callee_reason(callee_text))],
                None,
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
) -> (Vec<ClassifiedEdge>, Coverage, BuiltinDispatchAudit) {
    // Quick ObjectNodeId → &ObjectNode lookup.
    let obj_node_map: HashMap<ObjectNodeId, &ObjectNode> =
        graph.objects.iter().map(|o| (o.id.clone(), o)).collect();

    let index = ResolveIndex::build(graph);
    let body_map = BodyMap::build(graph, parsed);

    let mut obligation_id_set: HashSet<ObligationId> = HashSet::new();
    let mut classified_edges: Vec<ClassifiedEdge> = Vec::new();
    // T0.3: builtin-dispatch audit accumulators — sorted once, after the loop.
    let mut flagged: Vec<FlaggedBuiltinDispatchSite> = Vec::new();
    let mut indeterminate: Vec<IndeterminateBuiltinDispatchSite> = Vec::new();

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
                    let caller = source_routine_node_id(obj_node_id.clone(), routine);

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

                        let (kind, shape, completeness, routes, finding) =
                            resolve_call_site_obligation(
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
                                &pf.file,
                                &site.args,
                            );

                        match finding {
                            Some(BuiltinDispatchFinding::Flagged { object, method }) => {
                                flagged.push(FlaggedBuiltinDispatchSite {
                                    file: pf.virtual_path.clone(),
                                    object,
                                    method,
                                    line: site.span.start.line,
                                });
                            }
                            Some(BuiltinDispatchFinding::Indeterminate { method }) => {
                                indeterminate.push(IndeterminateBuiltinDispatchSite {
                                    file: pf.virtual_path.clone(),
                                    method,
                                    line: site.span.start.line,
                                });
                            }
                            None => {}
                        }

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

    // T0.3: deterministic sort — the accumulation order above already follows
    // parsed-file/object/routine/site document order (no HashMap iteration),
    // but sorting here makes the output ORDER independent of that traversal
    // order too, per the audit's determinism constraint.
    flagged.sort();
    indeterminate.sort();
    let builtin_dispatch_audit = BuiltinDispatchAudit {
        flagged,
        indeterminate,
    };

    (classified_edges, coverage, builtin_dispatch_audit)
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
    let (edges, coverage, builtin_dispatch_audit) =
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

    let event_flow_dual_publisher_alias_skips =
        crate::program::resolve::resolver::dual_publisher_alias_skip_count(&graph.routines);

    // Task 3 (preprocessor foundations plan): additive Recovered-parse
    // diagnostic — surfaced, never gating (see `recovered_files`'s doc).
    let recovered_files = crate::snapshot::parse::recovered_file_paths(parsed);

    Some(ProgramReport {
        edges,
        coverage,
        histogram,
        primary_histogram,
        abi_integrity,
        primary_app_ref,
        event_flow_dual_publisher_alias_skips,
        recovered_files,
        builtin_dispatch_audit,
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
    let (edges, _coverage, _builtin_dispatch_audit) = resolve_full_program_from_parts(
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
        ObligationOutcome::AmbiguousResolved => h.ambiguous_resolved += 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::program::node::ObjKey;
    use crate::program::resolve::edge::{Condition, SourcePos};

    fn rid(name: &str) -> RoutineNodeId {
        RoutineNodeId {
            object: ObjectNodeId {
                app: AppRef(0),
                kind: ObjectKind::Codeunit,
                key: ObjKey::Id(1),
            },
            name_lc: name.to_string(),
            enclosing_member_lc: None,
            params_count: 0,
            sig_fp: 0,
        }
    }

    fn ambiguous_route(name: &str) -> Route {
        Route {
            target: RouteTarget::Routine(rid(name)),
            evidence: Evidence::Source,
            conditions: vec![Condition::AmbiguousDispatch],
            witness: Witness::SourceSpan {
                file: "f.al".into(),
                span: (0, 1),
            },
            receiver_tier: None,
        }
    }

    fn edge_with(shape: DispatchShape, completeness: SetCompleteness, routes: Vec<Route>) -> Edge {
        let caller = rid("c");
        Edge {
            from: caller.clone(),
            site: SiteId {
                caller,
                span: CanonicalSpan {
                    unit: "u".into(),
                    start: SourcePos { line: 1, col: 1 },
                    end: SourcePos { line: 1, col: 2 },
                },
                callee_fingerprint: 1,
            },
            kind: EdgeKind::Call,
            shape,
            completeness,
            routes,
        }
    }

    /// `completeness_for_shape(AmbiguousOverload) == Complete` (Task 3): the
    /// candidate set is a snapshot-enumerated CLOSED set, unlike
    /// Polymorphic's open-world `Partial { ReverseDependentImplementers }`.
    #[test]
    fn completeness_for_ambiguous_overload_shape_is_complete() {
        assert_eq!(
            completeness_for_shape(DispatchShape::AmbiguousOverload),
            SetCompleteness::Complete
        );
    }

    /// `count_into_histogram` is a DOCUMENTED duplicate of
    /// `Histogram::of_edges` (full.rs's own module doc calls this out) — Task
    /// 3 requires BOTH copies stay in lockstep. Pins the `ambiguous_resolved`
    /// arm here independently of `edge.rs`'s own `Histogram::of_edges` test.
    #[test]
    fn count_into_histogram_counts_ambiguous_resolved_like_of_edges() {
        let edges = vec![
            edge_with(
                DispatchShape::AmbiguousOverload,
                SetCompleteness::Complete,
                vec![ambiguous_route("overload_a"), ambiguous_route("overload_b")],
            ),
            edge_with(
                DispatchShape::Exact,
                SetCompleteness::Complete,
                vec![Route {
                    target: RouteTarget::Routine(rid("helper")),
                    evidence: Evidence::Source,
                    conditions: vec![],
                    witness: Witness::SourceSpan {
                        file: "f.al".into(),
                        span: (0, 1),
                    },
                    receiver_tier: None,
                }],
            ),
        ];

        // The `count_into_histogram`-driven path (what `resolve_full_program`
        // actually calls).
        let mut h = Histogram::default();
        for e in &edges {
            count_into_histogram(&mut h, e);
        }
        assert_eq!(h.ambiguous_resolved, 1);
        assert_eq!(h.resolved_source, 1);
        assert_eq!(h.unknown, 0);
        assert_eq!(h.total, 2);

        // The two copies must agree exactly (the "documented duplicate" contract).
        let h2 = Histogram::of_edges(&edges);
        assert_eq!(
            h, h2,
            "count_into_histogram must mirror Histogram::of_edges"
        );
    }

    // -----------------------------------------------------------------------
    // Task 3 (preprocessor foundations plan): the `recovered_files`
    // diagnostic, wired end to end through `resolve_full_program` — no
    // CDO_WS needed, a bare on-disk temp workspace suffices (mirrors
    // `snapshot::tests::write_minimal_app_json`'s pattern).
    // -----------------------------------------------------------------------

    fn write_minimal_workspace(dir: &std::path::Path) {
        let app_json = r#"{
    "id": "22222222-0000-0000-0000-000000000002",
    "name": "Task3 Recovered Probe",
    "publisher": "probe",
    "version": "1.0.0.0"
}"#;
        std::fs::write(dir.join("app.json"), app_json).expect("write app.json");
    }

    #[test]
    fn resolve_full_program_reports_recovered_file_with_its_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_minimal_workspace(dir.path());
        std::fs::write(
            dir.path().join("Clean.al"),
            "codeunit 50000 T { procedure Foo() begin end; }",
        )
        .expect("write Clean.al");
        // An unbalanced #if forces tree-sitter error recovery.
        std::fs::write(
            dir.path().join("Broken.al"),
            "codeunit 50001 T { procedure Foo() begin\n#if NEVER_CLOSED\nBar();\nend; }",
        )
        .expect("write Broken.al");

        let report = resolve_full_program(dir.path()).expect("resolve_full_program");
        assert_eq!(
            report.recovered_files,
            vec!["Task3 Recovered Probe::Broken.al".to_string()],
            "only Broken.al must be reported, with its path — got {:?}",
            report.recovered_files
        );
    }

    #[test]
    fn resolve_full_program_recovered_files_empty_when_workspace_is_clean() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_minimal_workspace(dir.path());
        std::fs::write(
            dir.path().join("Clean.al"),
            "codeunit 50000 T { procedure Foo() begin end; }",
        )
        .expect("write Clean.al");

        let report = resolve_full_program(dir.path()).expect("resolve_full_program");
        assert!(
            report.recovered_files.is_empty(),
            "a whole-clean workspace must report zero recovered files; got {:?}",
            report.recovered_files
        );
    }
}
