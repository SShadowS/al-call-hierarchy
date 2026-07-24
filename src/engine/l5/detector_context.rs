//! `DetectorContext` — port of al-sem `src/detectors/detector-context.ts`.
//!
//! The shared, eager indexes + derived graphs detectors read from, built once at
//! the top of `run_detectors`. This R4-A wave builds the EAGER indexes the task
//! enumerates: routine_by_id / objects_by_id / table_by_id / reverse_call_graph /
//! entry_points / transaction_spans / resolved_call_edge_by_callsite /
//! uncertainty_edges_by_from / call_site_by_id, plus the combined graph + the
//! per-routine `FullRoutineSummary` map (transaction_spans needs it).
//!
//! DEFERRED to later waves (TODO):
//!   - the lazy `get_event_flow_indexes()` hook (D43/D44/D45)
//!   - the lazy `get_ordering_facts()` hook (D47)
//!
//! d4 reads none of these; later detector waves add them as they land.
//!
//! The R4-G wave wired `reachable_roots` + `internal_reachable_externally` (D14):
//! `reachable_roots` is built via `entry_points::find_reachable_roots` over the
//! `access_modifiers` map harvested from `L3Routine.access_modifier`;
//! `internal_reachable_externally` DEFAULTS to `false` (see field doc).

use std::collections::{BTreeSet, HashMap};

use crate::engine::l2::features::PCallSite;
use crate::engine::l3::call_resolver::{
    CallEdge, DeclaredDependency, UpgradedBinding, resolve_calls,
};
use crate::engine::l3::event_graph::build_event_graph;
use crate::engine::l3::event_graph::{EventGraph, EventSymbol};
use crate::engine::l3::l3_workspace::{L3Object, L3Resolved, L3Routine, L3Table};
use crate::engine::l3::symbol_table::SymbolTable;
use crate::engine::l4::capability_cone::{
    CapabilityFact, compose_cone_over_graph, direct_facts_for_routine,
};
use crate::engine::l4::combined_graph::{CombinedGraph, build_combined_graph};
use crate::engine::l4::cone_derived::{ConeDerivedStore, ConeOutput};
use crate::engine::l4::effect_store::SummaryBundle;
use crate::engine::l4::scc::{SccInputGraph, tarjan_scc};
use crate::engine::l4::summary::{RecordRoleSummary, Uncertainty, dedupe_uncertainties};
use crate::engine::l4::summary_runner::{FieldIndex, compute_summaries_v2_bundle};
use crate::engine::l5::entry_points::AccessModifier;
use crate::engine::l5::event_flow::{EventFlowIndexes, build_event_flow_indexes};
use crate::engine::l5::full_summary::FullRoutineSummary;
use crate::engine::l5::reverse_call_graph::{ReverseCallGraph, build_reverse_call_graph};
use crate::engine::l5::transaction_spans::{TransactionSpan, compute_transaction_spans};
use crate::engine::perf_trace as pt;
use serde_json::json;

/// A declared workspace dependency (`model.identity.primaryDependencies[]`): the
/// `appGuid` / `name` / `minVersion` triple d17 iterates. Mirrors al-sem's
/// `ManifestDependency` (the d17-relevant subset). Source-only runs leave this empty.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclaredDep {
    pub app_guid: String,
    pub name: String,
    pub min_version: String,
}

/// Shared context threaded into every detector.
pub struct DetectorContext<'a> {
    /// The combined graph (al-sem passes this as the detector's `graph` arg;
    /// detectors read it from the ctx here).
    pub graph: CombinedGraph,
    /// The raw L3 event graph (al-sem `model.eventGraph`). d12/d38 read its
    /// `events`/`edges`; the combined-graph build already constructs it, so it is
    /// captured here rather than recomputed.
    pub event_graph: EventGraph,
    pub routine_by_id: HashMap<&'a str, &'a L3Routine>,
    pub objects_by_id: HashMap<&'a str, &'a L3Object>,
    pub table_by_id: HashMap<&'a str, &'a L3Table>,
    pub reverse_call_graph: ReverseCallGraph,
    /// Trigger + event-subscriber roots — transaction-span boundaries.
    pub entry_points: BTreeSet<String>,
    pub transaction_spans: Vec<TransactionSpan>,
    /// Resolved CallEdges keyed by callsiteId (first edge per callsite wins).
    pub resolved_call_edge_by_callsite: HashMap<String, CallEdge>,
    /// Uncertainty edges grouped by source routine.
    pub uncertainty_edges_by_from:
        HashMap<String, Vec<crate::engine::l4::combined_graph::Uncertainty>>,
    /// Per-node merged uncertainty set the path-walker accumulates per branch.
    /// `uncertaintiesAt(node) = core_summary.uncertainties ∪
    /// uncertainty_edges_by_from.get(node)`, deduped+sorted by `uncertainty_key`.
    /// Mirrors al-sem `walkEvidence`'s `uncertaintiesAt` (path-walker.ts:103-106).
    /// The UNION ORDER is `[...fromSummary, ...fromEdges]` — the CORE
    /// `RoutineSummary.uncertainties` first, then the combined-graph edge
    /// uncertainties — matching al-sem exactly before the dedupe. Keyed by
    /// internal routine id; `walk_evidence` reads it via this exact field.
    pub uncertainties_by_node: HashMap<String, Vec<Uncertainty>>,
    /// Every call site indexed by id.
    pub call_site_by_id: HashMap<&'a str, &'a PCallSite>,
    /// Per-routine `FullRoutineSummary` (direct + inherited facts + coverage).
    pub summaries: HashMap<String, FullRoutineSummary>,
    /// ⟨C1⟩ The compact DERIVED capability-cone substrate — per-routine presence
    /// flags + interned table/event id-sets folded during the same cone walk that
    /// produces `summaries`, with zero `retag` clones. Every analyze-path consumer
    /// of the old `capability_facts_inherited` Vec reads only a derived predicate
    /// off it, so this row REPLACES that Vec outright: since C1 Task 3 the analyze
    /// path composes under `ConeOutput::DerivedOnly` and the raw Vec is never
    /// built (`summaries[*].inherited_raw()` panics there — see
    /// `FullRoutineSummary`). Parked here — next to `db_effect_bundle` — because
    /// the rows are `Range<u32>` windows into pools this store owns; a row alone
    /// is meaningless. EMPTY when the `SUMMARIES` substrate was not demanded.
    pub cone_derived: ConeDerivedStore,
    /// The shared event-flow indexes (publisher/subscriber lookup tables) the
    /// d43/d44/d45 event-flow detectors consume. al-sem builds this LAZILY
    /// (`ctx.getEventFlowIndexes()`, memoized); the Rust port builds it EAGERLY
    /// here — deterministic, one pass over `event_graph.events`/`.edges`, matching
    /// how `event_graph`/`transaction_spans` are already eager.
    pub event_flow_indexes: EventFlowIndexes,
    /// The CORE `RoutineSummary.parameter_roles` (`RecordRoleSummary[]`) per
    /// routine, keyed by internal RoutineId. al-sem detectors read this as
    /// `routine.summary.parameterRoles`; the Rust `FullRoutineSummary`
    /// (`ctx.summaries`) DROPPED parameter_roles, so d37/d39 read them here.
    /// Harvested from the SAME recomputed core summaries the `uncertainties_by_node`
    /// harvest uses — NOT recomputed. Absent ⇒ no record-parameter roles.
    pub parameter_roles_by_routine: HashMap<String, Vec<RecordRoleSummary>>,
    /// The post-upgrade argument bindings per callsite (the resolver's
    /// `upgradeBindings` side table). The L3 `PCallArgumentBinding` carries the
    /// SOURCE-side fields (sourceKind / sourceVariableName / sourceRecordVariableId
    /// / callerSourceParameterIsVar / argumentAnchor / parameterIndex), but NOT the
    /// upgraded `bindingResolution` / `calleeParameterIsVar` — those live here,
    /// index-aligned with `call_site.argument_bindings`. d37/d39 join the two by
    /// position to read `binding.bindingResolution` / `binding.calleeParameterIsVar`.
    pub upgraded_bindings_by_callsite: HashMap<String, Vec<UpgradedBinding>>,
    /// The D14 forward-reachability root set — entry points (trigger /
    /// event-subscriber) PLUS the procedures al-sem cannot prove app-scoped
    /// (non-`local`; `internal` only when `internal_reachable_externally`). Built
    /// by `entry_points::find_reachable_roots` over the `access_modifiers` map
    /// harvested from `L3Routine.access_modifier`. Sorted; d14 BFS-seeds from it.
    pub reachable_roots: BTreeSet<String>,
    /// al-sem `(model.identity.primaryInternalsVisibleTo?.length ?? 0) > 0` — true
    /// when some other app is granted `internal` access (so `internal` procedures
    /// stay external API surface and are NOT flaggable as dead).
    ///
    /// DEFAULTS to `false`: the Rust model does NOT carry `primaryInternalsVisibleTo`
    /// and the source-only fixtures never set `internalsVisibleTo`. This is the
    /// source-only common case (no granted consumer ⇒ `internal` is app-scoped ⇒
    /// flaggable).
    /// TODO(R4-G+): if any fixture ever sets `internalsVisibleTo`, forward
    /// `primaryInternalsVisibleTo` from the L3 identity and replace this default.
    pub internal_reachable_externally: bool,
    /// R4-F root classifications (`model.rootClassifications`), keyed by INTERNAL
    /// RoutineId — d50/d51 look these up exactly like al-sem's
    /// `model.rootClassifications.find(r => r.routineId === routine.id)`. Carried
    /// verbatim from the resolved workspace (AST classifier + roots.config
    /// overlay). Empty when the resolve path produced no classifications.
    pub root_classifications_by_routine:
        HashMap<String, crate::engine::root_classification::RootClassification>,
    /// The routines whose app_guid ∈ the fetched dependency set (cross-app runs).
    /// `roleOf(caller)` = `dep_routine_ids.contains(caller.id) ? "dependency" :
    /// "primary"` — the d13/d16/d17 cross-app gate. EMPTY for source-only runs (every
    /// routine primary), matching al-sem's source-only `analysisRole` default.
    pub dep_routine_ids: BTreeSet<String>,
    /// The DECLARED workspace dependencies (`model.identity.primaryDependencies`),
    /// `{appGuid, name, minVersion}` per the primary app.json `dependencies[]`. d17
    /// iterates these. EMPTY for source-only runs (no deps declared / read).
    pub declared_dependencies: Vec<DeclaredDep>,
    /// Resolved dependency `.app` versions keyed by appGuid (`model.apps[].version`).
    /// d17 looks up the resolved version to compare against the declared minVersion.
    /// EMPTY for source-only runs (no dep .app parsed).
    pub app_versions: HashMap<String, String>,
    /// R4-F Stage-5b — the L4.5 ordering facts the d47/d49/d51 detectors consume,
    /// keyed by `StableRoutineId`. Computed LAZILY on first `get_ordering_facts()`
    /// access and memoized — exactly al-sem's `ctx.getOrderingFacts()` semantics.
    /// Only d47/d49/d51 (opt-in detectors) read it, so a default `analyze` run
    /// never pays the snapshot→digest→ordering cost (measured 43.6 s+ on CDO —
    /// the "alsem never completes" hang; see
    /// `.superpowers/sdd/alsem-parallel/investigation.md`).
    pub ordering_facts:
        std::sync::OnceLock<HashMap<String, crate::engine::l5::ordering_facts::OrderingFacts>>,
    /// The resolved model `get_ordering_facts()` computes from. `None` for the
    /// cross-app context (whose ordering facts are ALWAYS empty — d13/d16/d17
    /// never read them; matches the previous eager `HashMap::new()`).
    pub ordering_source: Option<&'a L3Resolved>,
    /// G-19 — the closed-world proven-temp `(routineId, paramIndex)` set: a
    /// keyword-less by-var record param of a `local` procedure ALL of whose
    /// resolved callers (and the routine's complete, fully-resolved same-object
    /// call surface) prove a `Known(true)` temporary argument. The d1/d3/d10
    /// temp gates treat such a param exactly like a `Known(true)` temp record.
    /// Built by `closed_world_temp::prove_closed_world_temp_params`; EVERY
    /// uncertainty fails the proof (the firing direction) — see module docs.
    pub closed_world_temp_params: crate::engine::l5::closed_world_temp::ClosedWorldTempParams,
    /// L4 summarize-stage diagnostics — presently just the JACOBI fixed-point
    /// cap-hit (`summary_runner::run_one_scc`). Harvested from the SAME
    /// `compute_summaries*` call this module already makes for
    /// `uncertainties_by_node`/`parameter_roles_by_routine` — not recomputed.
    /// Empty for every workspace whose SCCs converge, which is the overwhelming
    /// common case (additive: `run_detectors` folds this into the "summarize"
    /// slot of the analyze/detect diagnostics envelope).
    pub summarize_diagnostics: Vec<crate::engine::l4::summary_runner::SummarizeDiagnostic>,
    /// ⟨Task B1⟩ The workspace-complete compact db-effect store the analyze path's
    /// v2 solve produced, held HERE instead of eagerly re-materialized into
    /// per-routine `Vec<DbEffect>`. No current detector reads `RoutineSummary.db_effects`
    /// (the analyze path consumes only `.uncertainties` / `.parameter_roles`), so the
    /// old compat-shim materialization was pure waste (~24 GB / ~74 s on 8020). The
    /// rows stay QUERYABLE on demand — `bundle.db_effects(rix)` (lazy projection) or an
    /// `l4::reverse_index::ReverseEffectIndex` built from this bundle — so a future
    /// db-effect-reading detector/hover can query them lazily WITHOUT resurrecting the
    /// eager expansion. `None` when the `CORE_SUMMARIES` substrate was not demanded
    /// (the summary solve is skipped entirely — no bundle exists to hold).
    pub db_effect_bundle: Option<SummaryBundle>,
    /// The shared finding-fingerprint index (routine/object id maps + the
    /// internal→stable routine-id substitution map). Built ONCE per run —
    /// previously every detector rebuilt it (54 × ~2 String clones per routine).
    pub fingerprint_index: crate::engine::l5::fingerprint::FingerprintIndex<'a>,
    /// event id → cross-extension subscriber routine ids (subscribers living in a
    /// DIFFERENT app than the publisher object). Previously rebuilt identically by
    /// d43, d44 AND d45 each run; built once here (same sharing pattern as
    /// `event_flow_indexes`).
    pub cross_extension_subscribers: std::collections::BTreeMap<String, Vec<String>>,
}

impl DetectorContext<'_> {
    /// The L4.5 ordering facts, keyed by `StableRoutineId`. Lazily computed on
    /// first access (memoized via `OnceLock` — thread-safe for future parallel
    /// detector runs). d47/d49/d51 look up their reportable routine's facts here
    /// exactly as al-sem's `ctx.getOrderingFacts()`.
    pub fn get_ordering_facts(
        &self,
    ) -> &HashMap<String, crate::engine::l5::ordering_facts::OrderingFacts> {
        self.ordering_facts
            .get_or_init(|| match self.ordering_source {
                Some(resolved) => {
                    crate::engine::l5::ordering_facts::compute_ordering_facts(resolved)
                }
                None => HashMap::new(),
            })
    }
}

/// Build the shared context. Runs the SOURCE-ONLY L3→L4 substrate (symbols →
/// resolve_calls → event_graph → combined_graph → cone) to assemble the combined
/// graph + the always-built eager indexes, then builds ONLY the expensive substrates
/// named in `demanded` (see `registry::substrate`).
///
/// `demanded` is the union of every selected detector's `requires` bits (folded by
/// `run_detectors`). Skipped substrates leave their ctx fields EMPTY — the field types
/// are unchanged. Passing `substrate::ALL` reproduces the pre-W1.0 eager build exactly
/// (byte-identical context), which every non-registry caller does. The four gated
/// substrates are:
///   - `SUMMARIES` — capability cones + `summaries` (also built for `TRANSACTION_SPANS`,
///     which folds over the summaries map internally).
///   - `CORE_SUMMARIES` — second Tarjan + the closed-form v2 core summaries
///     (`db_effects`/`uncertainties` via `compute_summaries_v2`, `parameter_roles`
///     via its own JACOBI-disciplined roles-only fixpoint) →
///     `uncertainties_by_node` / `parameter_roles_by_routine` / `summarize_diagnostics`.
///   - `TRANSACTION_SPANS` — `transaction_spans`.
///   - `CLOSED_WORLD_TEMP` — `closed_world_temp_params`.
///
/// ⟨C1 Task 3⟩ `demanded` additionally carries the policy-only
/// `RAW_INHERITED_FACTS` bit (NOT part of `substrate::ALL` — see its doc). Without
/// it the cone composes under [`ConeOutput::DerivedOnly`]: the per-routine raw
/// `Vec<CapabilityFact>` is never allocated (it cost ~10.9 GB on the 8020 corpus)
/// and every summary carries `capability_facts_inherited: None`. With it the cone
/// composes under [`ConeOutput::Both`] — the derived substrate AND the raw Vecs,
/// byte-identical to the pre-Task-3 build.
pub fn build_detector_context(resolved: &L3Resolved, demanded: u32) -> DetectorContext<'_> {
    use crate::engine::l5::registry::substrate;
    let ws = &resolved.workspace;
    // TRANSACTION_SPANS folds over the summaries map, so demand summaries whenever
    // either bit is set (see `compute_transaction_spans`).
    let need_summaries = demanded & (substrate::SUMMARIES | substrate::TRANSACTION_SPANS) != 0;
    // ⟨C1 Task 3 — R1⟩ The gate is a MODE threaded into the cone walk, not a
    // post-hoc check: the raw Vec is allocated INSIDE `compose_inherited_cones`,
    // so a check here could only discard it — zero memory win.
    let want_raw_inherited = demanded & substrate::RAW_INHERITED_FACTS != 0;
    // ⟨C1 Task 3 review fix M-1⟩ `RAW_INHERITED_FACTS` without `SUMMARIES` (nor
    // `TRANSACTION_SPANS`) is a SILENT no-op: `need_summaries` would be false, the
    // block below never runs, `summaries` stays empty, and `select_facts` (which
    // reads through `summaries`) then returns an empty fact list for every
    // routine — the exact silent-empty outcome R6 was designed to foreclose,
    // reached through a different door. No current caller does this (`R2`'s table
    // above), but a future one could; fail loudly instead of degrading quietly.
    debug_assert!(
        !want_raw_inherited || need_summaries,
        "RAW_INHERITED_FACTS demanded without SUMMARIES/TRANSACTION_SPANS — the raw \
         cone would silently build empty; OR in substrate::SUMMARIES alongside it"
    );

    // --- L3→L4 substrate (source-only: no deps) ----------------------------
    // `symbols` feeds BOTH spans below (resolve_calls here, build_event_graph in
    // the next stage), so it is built at this outer scope instead of inside either
    // span's own block — the two spans are closed explicitly (`drop`) at their
    // semantic stage ends rather than by a block boundary (same pattern as
    // `gate/run.rs`'s `gate.project_filter_scope_baseline_suppress`).
    let _symbols_span = pt::span("context", "context.symbols_resolve_calls");
    let symbols = SymbolTable::build(&ws.objects, &ws.tables, &ws.routines);
    let no_deps: Vec<DeclaredDependency> = Vec::new();
    let no_fetched: Vec<String> = Vec::new();
    let mut calls = resolve_calls(ws, &symbols, &no_deps, &no_fetched);
    drop(_symbols_span);

    let _graph_span = pt::span("context", "context.event_combined_graph");
    let event_graph = build_event_graph(&ws.routines, &symbols);
    let graph = build_combined_graph(ws, &calls, &event_graph);
    drop(_graph_span);

    // Per-routine direct facts + direct coverage, then the inherited cone over
    // the combined graph — the same assembly project_r3a3 does inline, here via
    // the reusable `compose_cone_over_graph` seam. SUBSTRATE-GATED: built only when
    // some selected detector demands SUMMARIES (or TRANSACTION_SPANS, which folds
    // over the summaries map). Skipped ⇒ empty `summaries` map.
    let _cones_span = pt::span("context", "context.capability_cones");
    // ⟨C1⟩ Assigned inside the block below (kept a separate binding so the whole
    // cone-assembly block stays at its original indentation).
    let mut cone_derived = ConeDerivedStore::default();
    let summaries: HashMap<String, FullRoutineSummary> = if need_summaries {
        let mut publisher_events_by_routine: HashMap<String, Vec<&EventSymbol>> = HashMap::new();
        for evt in &event_graph.events {
            if let Some(pr) = &evt.publisher_routine_id {
                publisher_events_by_routine
                    .entry(pr.clone())
                    .or_default()
                    .push(evt);
            }
        }
        let empty_pub: Vec<&EventSymbol> = Vec::new();
        // ⟨C1 Task 4⟩ ONE direct-facts map, not two. This used to build a
        // second, byte-identical `direct_in` (a `facts.clone()` per routine)
        // purely to hand to the cone walk, which takes it by shared reference
        // and only reads it — 79.66 MB of pure duplicate on the 8020 corpus,
        // held for the rest of this block because Rust frees a local at its
        // block's end, not at its last use. `direct_full` is not drained until
        // the summary loop below, well after the walk's borrow ends, so the
        // walk can simply read it. (Every OTHER `compose_cone_over_graph` call
        // site — `project_r3a3`, `project_r3a5_cross_app`,
        // `build_detector_context_cross_app` — already passed its `direct_full`
        // directly; this was the one straggler.)
        let mut direct_full: HashMap<String, Vec<CapabilityFact>> = HashMap::new();
        let mut coverage_in: HashMap<String, (String, Vec<String>)> = HashMap::new();
        let nodes: Vec<String> = ws.routines.iter().map(|r| r.id.clone()).collect();
        for r in &ws.routines {
            let pubs = publisher_events_by_routine.get(&r.id).unwrap_or(&empty_pub);
            let (facts, status, reasons) = direct_facts_for_routine(r, pubs);
            coverage_in.insert(r.id.clone(), (status, reasons));
            direct_full.insert(r.id.clone(), facts);
        }
        // ⟨C1 Task 3⟩ `DerivedOnly` — the compact substrate only; the per-routine
        // raw inherited `Vec<CapabilityFact>` is never allocated. `Both` only when
        // the policy-only `RAW_INHERITED_FACTS` bit is demanded.
        let mode = if want_raw_inherited {
            ConeOutput::Both
        } else {
            ConeOutput::DerivedOnly
        };
        let outcome = compose_cone_over_graph(&graph, &nodes, &direct_full, &coverage_in, mode);
        let mut cones = outcome.cones;
        cone_derived = outcome.derived;
        // ⟨C1 Task 4⟩ Both cone inputs are dead here; free them before the
        // summary assembly below rather than at this block's closing brace
        // (`nodes` is one more full copy of every routine id, `coverage_in` one
        // more copy of every routine's direct status + reasons).
        drop(nodes);
        drop(coverage_in);

        // `cones` and `direct_full` are locally owned and dead after this loop, so
        // move their payloads into the summaries instead of cloning them out.
        let mut summaries: HashMap<String, FullRoutineSummary> = HashMap::new();
        for r in &ws.routines {
            let cone_entry = cones.remove(&r.id);
            let direct = direct_full.remove(&r.id);
            // ⟨C1⟩ Two AL routines can COLLIDE on one internal routine id (two
            // same-name triggers in one object — gap G-18; `compute_routine_id`
            // has no member discriminator). Both `remove`s above are then
            // consumed by the FIRST occurrence, so the summary the second
            // occurrence writes — the one that survives the map insert — is
            // fully degenerate: no direct facts, no inherited facts, no
            // coverage. The derived row is keyed by id and holds the FULL fold,
            // so it must be dropped to match the summary this context will
            // actually hold. (PRE-EXISTING behaviour, reproduced deliberately:
            // losing a colliding routine's whole cone is a real precision
            // defect, but it is what today's detector output encodes — see the
            // C1 Task 1 report. `build_detector_context_cross_app` reads its
            // cone with `get()`, so it never has this accident and needs no
            // such adjustment.)
            //
            // ⟨fix M1⟩ `cone_entry.is_none() <=> direct.is_none()` today: both maps
            // are built from the same `ws.routines` iteration and drained by the
            // same per-id `remove()`, so a collision empties them together, never
            // just one. Assert that invariant instead of merely relying on it —
            // if a future change ever filters `nodes` (dep routines, bodyless
            // routines, …) while `direct_full` kept every routine, the OLD `||`
            // would zero a row whose surviving summary still carries direct
            // facts, a silent findings loss once the parity oracle is retired.
            // Keying on `direct.is_none()` alone is the correct long-term
            // condition either way, since `direct` is what the surviving summary
            // actually stores.
            //
            // ⟨final-branch-review M-2⟩ This assert is now the ONLY guard on that
            // divergence. Task 2 had added a second, reverse one — the parity
            // oracle's "the store cannot hold a row `summaries` does not" check —
            // and Task 3 deleted it along with `cone_parity.rs` itself, which is
            // also what returned `ConeDerivedStore`'s `routine_ids()`/`interner()`/
            // `len()` to zero callers (deleted at M-2). The extra-row direction
            // that check covered is structurally unreachable while `nodes` is built
            // from `ws.routines`; if that ever stops being true, this assert is
            // what has to catch it, and only in a debug build.
            debug_assert_eq!(
                cone_entry.is_none(),
                direct.is_none(),
                "cones and direct_full must collide identically — a mismatch means \
                 `nodes` (cone input) and `ws.routines` (direct_full's source) have \
                 silently diverged"
            );
            if direct.is_none() {
                cone_derived.forget(&r.id);
            }
            let (inherited, coverage) = match cone_entry {
                Some(c) => (c.inherited, Some(c.coverage)),
                None => (Vec::new(), None),
            };
            // ⟨C1 Task 3⟩ `Some(inherited)` ONLY under `RAW_INHERITED_FACTS`;
            // `None` records "never materialized" so `inherited_raw()` panics
            // instead of answering "empty cone" (R6). Note the `Some(Vec::new())`
            // case is REAL and must stay distinct from `None`: the G-18 collision
            // arm above yields a drained (empty) cone entry, which the policy path
            // must still read as a materialized-but-empty cone, exactly as it did
            // before this task.
            summaries.insert(
                r.id.clone(),
                FullRoutineSummary::new(
                    r.id.clone(),
                    direct.unwrap_or_default(),
                    want_raw_inherited.then_some(inherited),
                    coverage,
                ),
            );
        }
        summaries
    } else {
        HashMap::new()
    };
    drop(_cones_span);

    // ⟨C1 census⟩ `C1_CONE_CENSUS=1` — one-shot byte census of what the cone
    // build just left resident (`summaries` + `cone_derived`), emitted after
    // the span closes so the census's own (modest) bookkeeping never pollutes
    // the span's own `rss_delta` measurement. No-op when the env var is unset.
    crate::engine::l4::cone_census::emit_full_census(&summaries, &cone_derived);

    // --- Eager indexes -----------------------------------------------------
    let routine_by_id: HashMap<&str, &L3Routine> =
        ws.routines.iter().map(|r| (r.id.as_str(), r)).collect();
    let objects_by_id: HashMap<&str, &L3Object> =
        ws.objects.iter().map(|o| (o.id.as_str(), o)).collect();
    // G-5: REAL table wins an id collision with a tableextension stub (the stub's
    // id reuses the extension's own object number) — otherwise rootCause text
    // renders the EXTENSION's name for ops on the real table.
    let table_by_id: HashMap<&str, &L3Table> =
        crate::engine::l3::l3_workspace::table_by_id_preferring_real(&ws.tables);

    let reverse_call_graph = build_reverse_call_graph(&graph);

    // Source-only: no dep routines.
    let dep_routine_ids: BTreeSet<String> = BTreeSet::new();
    let entry_points: BTreeSet<String> =
        crate::engine::l5::entry_points::find_entry_points(&ws.routines, &dep_routine_ids)
            .into_iter()
            .collect();

    // D14 reachable-roots wiring. Build the RoutineId → AccessModifier map from
    // `L3Routine.access_modifier` ("local"/"internal"/"protected"/None). al-sem maps
    // "local" → Local, "internal" → Internal, "protected"/None/anything-else →
    // Public (default-access). A routine with NO entry is treated as Public by
    // `find_reachable_roots`, so we only need to insert the non-Public cases — but we
    // insert all parsed modifiers explicitly for clarity.
    let mut access_modifiers: HashMap<String, AccessModifier> = HashMap::new();
    for r in &ws.routines {
        let access = match r.access_modifier.as_deref() {
            Some("local") => AccessModifier::Local,
            Some("internal") => AccessModifier::Internal,
            // "protected" / None / any other value → public (al-sem default-access).
            _ => AccessModifier::Public,
        };
        access_modifiers.insert(r.id.clone(), access);
    }
    // See `DetectorContext::internal_reachable_externally` doc: defaults to false
    // (the Rust model carries no `primaryInternalsVisibleTo`; source-only fixtures
    // never set `internalsVisibleTo`).
    let internal_reachable_externally = false;
    let reachable_roots: BTreeSet<String> = crate::engine::l5::entry_points::find_reachable_roots(
        &ws.routines,
        &dep_routine_ids,
        &access_modifiers,
        internal_reachable_externally,
    )
    .into_iter()
    .collect();

    // G-19 — closed-world proven-temp params for `local` procedures (consumed
    // by the d1/d3/d10 temp gates). Pure lookup-table build over the routines +
    // combined graph + reverse graph; entry points are proof-disqualifying.
    // SUBSTRATE-GATED on CLOSED_WORLD_TEMP.
    let closed_world_temp_params = if demanded & substrate::CLOSED_WORLD_TEMP != 0 {
        crate::engine::l5::closed_world_temp::prove_closed_world_temp_params(
            &ws.routines,
            &graph,
            &reverse_call_graph,
            &entry_points,
        )
    } else {
        Default::default()
    };

    // Transaction spans — SUBSTRATE-GATED on TRANSACTION_SPANS (which also forced
    // `need_summaries`, so the `summaries` map above is populated here).
    let transaction_spans = {
        let _s = pt::span("context", "context.transaction_spans");
        if demanded & substrate::TRANSACTION_SPANS != 0 {
            compute_transaction_spans(
                &ws.routines,
                &dep_routine_ids,
                &reverse_call_graph,
                &summaries,
                &cone_derived,
            )
        } else {
            Vec::new()
        }
    };

    // Event-flow indexes — built eagerly from the L3 event graph + routine set +
    // dep set (source-only ⇒ empty dep set ⇒ every routine primary). Consumes
    // `event_graph` by reference before it is moved into the struct.
    let event_flow_indexes = build_event_flow_indexes(&event_graph, &ws.routines, &dep_routine_ids);

    // Cross-extension subscriber lookup, shared by d43/d44/d45 — previously each
    // rebuilt this identically from `ctx.event_graph` + `ws.objects` per run.
    let cross_extension_subscribers =
        crate::engine::l5::event_flow::build_cross_extension_subscribers(&event_graph, &ws.objects);

    // `calls.edges` is not read after this point (`calls.upgraded_bindings` still
    // is), so take the edges by value instead of cloning each retained one.
    let mut resolved_call_edge_by_callsite: HashMap<String, CallEdge> = HashMap::new();
    for ce in std::mem::take(&mut calls.edges) {
        if ce.to.is_none() {
            continue;
        }
        resolved_call_edge_by_callsite
            .entry(ce.callsite_id.clone())
            .or_insert(ce);
    }

    let mut uncertainty_edges_by_from: HashMap<
        String,
        Vec<crate::engine::l4::combined_graph::Uncertainty>,
    > = HashMap::new();
    for ue in &graph.uncertainty_edges {
        uncertainty_edges_by_from
            .entry(ue.from.clone())
            .or_default()
            .push(ue.uncertainty.clone());
    }

    // --- Per-node uncertainty wiring (the path-walker source) --------------
    // al-sem `walkEvidence` computes
    //   uncertaintiesAt(node) = routine.summary.uncertainties ∪ uncertaintyEdgesByFrom.get(node)
    // The CORE `RoutineSummary.uncertainties` is dropped by `FullRoutineSummary`
    // (the cone path keeps only facts + coverage), so we recompute the core
    // summaries here from the SAME combined graph the cone used: Tarjan SCC over
    // `graph.edges_by_from`, then the closed-form v2 solver (`compute_summaries_v2`).
    // This is the only place that needs the core uncertainties; the union is
    // assembled once and exposed on `uncertainties_by_node`.
    //
    // SUBSTRATE-GATED on CORE_SUMMARIES. This is the second Tarjan + v2-solver
    // pass — the most expensive substrate. Skipped ⇒ `uncertainties_by_node` /
    // `parameter_roles_by_routine` / `summarize_diagnostics` are all empty, which by
    // decision (a) means a substrate-skipping run emits no summarize cap-hit
    // diagnostics (they are only ever produced by this `compute_summaries_v2` call).
    #[allow(clippy::type_complexity)]
    let (
        uncertainties_by_node,
        parameter_roles_by_routine,
        summarize_diagnostics,
        db_effect_bundle,
    ): (
        HashMap<String, Vec<Uncertainty>>,
        HashMap<String, Vec<RecordRoleSummary>>,
        Vec<crate::engine::l4::summary_runner::SummarizeDiagnostic>,
        Option<SummaryBundle>,
    ) = if demanded & substrate::CORE_SUMMARIES != 0 {
        let scc = {
            let _s = pt::span("context", "context.core_scc_tarjan");
            let mut scc_adjacency: HashMap<String, Vec<String>> = HashMap::new();
            for (from, list) in &graph.edges_by_from {
                scc_adjacency.insert(from.clone(), list.iter().map(|e| e.to.clone()).collect());
            }
            tarjan_scc(&SccInputGraph {
                nodes: &graph.nodes,
                edges_by_from: &scc_adjacency,
            })
        };

        // STRUCTS one-shot: SCC population stats + the largest-SCC's intra-edge
        // anatomy (which edge KIND fuses the biggest component, member outdegree).
        // Reimplements the historical `SCCSTATS`/`SCCANATOMY` eprintln probe
        // (`git show c8836e7^:src/engine/l5/detector_context.rs`) as one lazy JSON
        // payload — `build` below only runs when tracing is actually enabled.
        pt::instant_lazy("l4", "scc_stats", || {
            let max_scc = scc.sccs.iter().map(|s| s.members.len()).max().unwrap_or(0);
            let recursive_sccs = scc.sccs.iter().filter(|s| s.recursive).count();
            let recursive_members: usize = scc
                .sccs
                .iter()
                .filter(|s| s.recursive)
                .map(|s| s.members.len())
                .sum();
            let largest_scc = scc.sccs.iter().max_by_key(|s| s.members.len()).map(|big| {
                let member_set: std::collections::HashSet<&str> =
                    big.members.iter().map(|s| s.as_str()).collect();
                let mut kind_counts: std::collections::BTreeMap<&str, usize> =
                    std::collections::BTreeMap::new();
                let mut intra_edges = 0usize;
                let mut max_intra_outdegree = 0usize;
                for m in &big.members {
                    let mut out_here = 0usize;
                    if let Some(edges) = graph.edges_by_from.get(m) {
                        for e in edges {
                            if member_set.contains(e.to.as_str()) {
                                intra_edges += 1;
                                out_here += 1;
                                *kind_counts.entry(e.kind.as_str()).or_insert(0) += 1;
                            }
                        }
                    }
                    max_intra_outdegree = max_intra_outdegree.max(out_here);
                }
                json!({
                    "members": big.members.len(),
                    "intra_edges": intra_edges,
                    "max_intra_outdegree": max_intra_outdegree,
                    "kinds": kind_counts,
                })
            });
            json!({
                "nodes": graph.nodes.len(),
                "sccs": scc.sccs.len(),
                "recursive_sccs": recursive_sccs,
                "recursive_members": recursive_members,
                "max_scc": max_scc,
                "largest_scc": largest_scc,
            })
        });

        let _summaries_span = pt::span("context", "context.compute_summaries");
        // Field-resolution index (keyed (tableId, lowercased field name)) — mirrors
        // summary.rs `run_and_project`; parameterRoles need it, uncertainties don't,
        // but `compute_summaries_v2` takes it.
        let mut field_index: FieldIndex = HashMap::new();
        for table in &ws.tables {
            for field in &table.fields {
                field_index
                    .entry((table.id.clone(), field.name.to_lowercase()))
                    .or_insert_with(|| field.id.clone());
            }
        }
        // v2 db-effect solver — ⟨Task B1⟩ the LEAN bundle entry point: it returns the
        // compact `SummaryBundle` PLUS a `core_summaries` map whose `db_effects` are
        // EMPTY (never re-materialized) while `.uncertainties` / `.parameter_roles` are
        // fully populated — the only fields this path reads below. The db-effect rows
        // stay queryable on demand via `db_effect_bundle` (held on the ctx). This drops
        // the compat shim's ~24 GB / ~74 s per-routine `Vec<DbEffect>` expansion that no
        // detector consumed. `summarize_diagnostics` carries the ROLES fixpoint's
        // cap-hit backstop (empty on the corpus — roles converge); the db_effects path
        // is closed-form and never caps.
        let (db_effect_bundle, core_summaries, summarize_diagnostics) = compute_summaries_v2_bundle(
            &ws.routines,
            &graph,
            &scc,
            &calls.upgraded_bindings,
            &field_index,
        );
        drop(_summaries_span);

        // uncertaintiesAt(node) per routine: [...fromSummary, ...fromEdges], deduped.
        // Union ORDER mirrors al-sem `[...fromSummary, ...fromEdges]` — core summary
        // uncertainties FIRST, then the combined-graph edge uncertainties (converted
        // to the summary `Uncertainty` form). `dedupe_uncertainties` keeps first-seen
        // then sorts by key, matching al-sem's `dedupeUncertainties`. This pass only
        // BORROWS `core_summaries` (cloning the uncertainty entries — the union needs
        // both sources), so `core_summaries` can then be drained for parameter_roles.
        let mut uncertainties_by_node: HashMap<String, Vec<Uncertainty>> = HashMap::new();
        for r in &ws.routines {
            let from_summary: &[Uncertainty] = core_summaries
                .get(&r.id)
                .map(|s| s.uncertainties.as_slice())
                .unwrap_or(&[]);
            let from_edges: Vec<Uncertainty> = uncertainty_edges_by_from
                .get(&r.id)
                .map(|edges| edges.iter().map(Uncertainty::from).collect())
                .unwrap_or_default();
            if from_summary.is_empty() && from_edges.is_empty() {
                continue;
            }
            let combined: Vec<Uncertainty> =
                from_summary.iter().cloned().chain(from_edges).collect();
            uncertainties_by_node.insert(r.id.clone(), dedupe_uncertainties(combined));
        }

        // Harvest the CORE parameter_roles per routine from the SAME recomputed core
        // summaries (d37/d39 read these as `routine.summary.parameterRoles`) — draining
        // `core_summaries` (now dead) by value so each non-empty role vec is MOVED, not
        // cloned. Membership matches the prior `ws.routines`+lookup form exactly: only
        // routines present in `core_summaries` with non-empty roles are inserted, and
        // the result is an order-independent HashMap.
        let mut parameter_roles_by_routine: HashMap<String, Vec<RecordRoleSummary>> =
            HashMap::new();
        for (rid, s) in core_summaries {
            if !s.parameter_roles.is_empty() {
                parameter_roles_by_routine.insert(rid, s.parameter_roles);
            }
        }
        (
            uncertainties_by_node,
            parameter_roles_by_routine,
            summarize_diagnostics,
            Some(db_effect_bundle),
        )
    } else {
        (HashMap::new(), HashMap::new(), Vec::new(), None)
    };

    let _final_indexes_span = pt::span("context", "context.final_indexes");
    let mut call_site_by_id: HashMap<&str, &PCallSite> = HashMap::new();
    for r in &ws.routines {
        for cs in &r.call_sites {
            call_site_by_id.insert(cs.id.as_str(), cs);
        }
    }

    // Expose the resolver's post-upgrade bindings (the `upgradeBindings` side
    // table) keyed by callsite id — the join target for d37/d39 which read
    // `binding.bindingResolution` / `binding.calleeParameterIsVar`. `compute_summaries_v2`
    // above was the last reader of `calls.upgraded_bindings`, so move it out here.
    let upgraded_bindings_by_callsite: HashMap<String, Vec<UpgradedBinding>> =
        std::mem::take(&mut calls.upgraded_bindings);

    // R4-F root classifications — keyed by internal RoutineId for d50/d51 lookup.
    let root_classifications_by_routine: HashMap<
        String,
        crate::engine::root_classification::RootClassification,
    > = resolved
        .root_classifications
        .iter()
        .map(|rc| (rc.routine_id.clone(), rc.clone()))
        .collect();

    // R4-F Stage-5b ordering facts — computed lazily on first
    // `get_ordering_facts()` access (see field doc). Keyed by StableRoutineId;
    // d47/d49/d51 read it via `get_ordering_facts()`.

    let fingerprint_index =
        crate::engine::l5::fingerprint::FingerprintIndex::build(&ws.routines, &ws.objects);
    drop(_final_indexes_span);

    DetectorContext {
        graph,
        event_graph,
        routine_by_id,
        objects_by_id,
        table_by_id,
        reverse_call_graph,
        entry_points,
        transaction_spans,
        resolved_call_edge_by_callsite,
        uncertainty_edges_by_from,
        uncertainties_by_node,
        call_site_by_id,
        summaries,
        cone_derived,
        event_flow_indexes,
        parameter_roles_by_routine,
        upgraded_bindings_by_callsite,
        reachable_roots,
        internal_reachable_externally,
        // Source-only: no deps → every routine primary, no declared deps, no versions.
        dep_routine_ids: BTreeSet::new(),
        declared_dependencies: Vec::new(),
        app_versions: HashMap::new(),
        root_classifications_by_routine,
        ordering_facts: std::sync::OnceLock::new(),
        ordering_source: Some(resolved),
        closed_world_temp_params,
        summarize_diagnostics,
        db_effect_bundle,
        fingerprint_index,
        cross_extension_subscribers,
    }
}

/// Build the shared context for a CROSS-APP run from a pre-assembled
/// `R3a5CrossAppBase` (the merged workspace+dep model + cross-app combined graph +
/// `dep_routine_ids`). Mirrors `build_detector_context` but reads every substrate
/// from `base` instead of recomputing source-only, and threads `dep_routine_ids`
/// into the entry-point / reachable-root / transaction-span / event-flow builders so
/// dep routines are NOT treated as primary roots. d13/d16/d17 read
/// `dep_routine_ids` (the roleOf gate), `declared_dependencies` + `app_versions`
/// (d17), and the eager indexes; the path-walker substrate (uncertainties /
/// summaries) is built identically for any future cross-app detector.
///
/// `root_classifications` are EMPTY here; `ordering_source` is `None` here (ordering
/// facts lazily resolve to EMPTY — d13/d16/d17 never read them; the base does not
/// carry the resolved-model classifier inputs). A future cross-app ordering detector
/// would thread them additively.
pub(crate) fn build_detector_context_cross_app(
    base: &crate::engine::l4::capability_cone::R3a5CrossAppBase,
) -> DetectorContext<'_> {
    use crate::engine::l4::summary_runner::compute_summaries_v2_bundle_with_leaves;

    let ws_routines = &base.ws_routines;
    let dep_routine_ids = &base.dep_routine_ids;
    let graph = base.graph.clone();

    // Cone over the merged graph (direct facts/coverage already assembled in `base`).
    // ⟨C1 Task 3⟩ `DerivedOnly`, unconditionally. Unlike the source-only builder
    // there is no mode choice to make here: this context is reachable ONLY from
    // `registry::run_detectors_cross_app` (its single caller), i.e. from the
    // detector path, and no detector reads raw inherited facts. The one consumer
    // that does — `gate::policy` — builds its context through the SOURCE-ONLY
    // `build_detector_context` (`gate/policy/pipeline.rs`), never this one. Adding
    // a `demanded` parameter here would therefore only add a branch that no caller
    // can ever take.
    let outcome = compose_cone_over_graph(
        &base.graph,
        &base.nodes,
        &base.direct_full,
        &base.direct_coverage,
        ConeOutput::DerivedOnly,
    );
    let cones = outcome.cones;
    let cone_derived = outcome.derived;
    let empty_facts: Vec<CapabilityFact> = Vec::new();
    let mut summaries: HashMap<String, FullRoutineSummary> = HashMap::new();
    for r in ws_routines {
        // ⟨C1 Task 3, carry #2⟩ The `inherited` FIELD POPULATION is gone; the
        // `cones.get()` is NOT. Switching this to `cones.remove()` would import
        // the G-18 routine-id-collision degeneracy that the source-only builder
        // accepts and this builder does not — a real output change, not a
        // refactor. `coverage` still comes off the same borrowed entry.
        let cone = cones.get(&r.id);
        let coverage = cone.map(|c| c.coverage.clone());
        summaries.insert(
            r.id.clone(),
            FullRoutineSummary::new(
                r.id.clone(),
                base.direct_full.get(&r.id).unwrap_or(&empty_facts).clone(),
                None,
                coverage,
            ),
        );
    }

    // --- Eager indexes (over the merged routine/object/table sets) ---------
    let routine_by_id: HashMap<&str, &L3Routine> =
        ws_routines.iter().map(|r| (r.id.as_str(), r)).collect();
    let objects_by_id: HashMap<&str, &L3Object> =
        base.objects.iter().map(|o| (o.id.as_str(), o)).collect();
    // G-5: REAL table wins an id collision with a tableextension stub.
    let table_by_id: HashMap<&str, &L3Table> =
        crate::engine::l3::l3_workspace::table_by_id_preferring_real(&base.tables);

    let reverse_call_graph = build_reverse_call_graph(&graph);

    let entry_points: BTreeSet<String> =
        crate::engine::l5::entry_points::find_entry_points(ws_routines, dep_routine_ids)
            .into_iter()
            .collect();

    let mut access_modifiers: HashMap<String, AccessModifier> = HashMap::new();
    for r in ws_routines {
        let access = match r.access_modifier.as_deref() {
            Some("local") => AccessModifier::Local,
            Some("internal") => AccessModifier::Internal,
            _ => AccessModifier::Public,
        };
        access_modifiers.insert(r.id.clone(), access);
    }
    let internal_reachable_externally = false;
    let reachable_roots: BTreeSet<String> = crate::engine::l5::entry_points::find_reachable_roots(
        ws_routines,
        dep_routine_ids,
        &access_modifiers,
        internal_reachable_externally,
    )
    .into_iter()
    .collect();

    // G-19 — closed-world proven-temp params (see the source-only builder).
    // Dep routines carry `access_modifier: None` (the ABI does not expose it),
    // so they can never be proven; primary `local` procedures still can.
    let closed_world_temp_params =
        crate::engine::l5::closed_world_temp::prove_closed_world_temp_params(
            ws_routines,
            &graph,
            &reverse_call_graph,
            &entry_points,
        );

    let transaction_spans = compute_transaction_spans(
        ws_routines,
        dep_routine_ids,
        &reverse_call_graph,
        &summaries,
        &cone_derived,
    );

    let event_flow_indexes =
        build_event_flow_indexes(&base.event_graph, ws_routines, dep_routine_ids);

    // Cross-extension subscriber lookup, from the SAME inputs as `event_flow_indexes`
    // above — `base.event_graph` + `base.objects` (the merged cross-app event graph
    // + object set), consistent with how `fingerprint_index` below anchors to `base`.
    let cross_extension_subscribers =
        crate::engine::l5::event_flow::build_cross_extension_subscribers(
            &base.event_graph,
            &base.objects,
        );

    // Resolved-call-edge-by-callsite index: EMPTY for the cross-app context. The
    // cross-app build does not retain the raw resolver `calls.edges`, and d13/d16/d17
    // read edges directly off `ctx.graph` (the combined graph). Future cross-app
    // detectors that need this index would thread `calls` through `R3a5CrossAppBase`.
    let resolved_call_edge_by_callsite: HashMap<
        String,
        crate::engine::l3::call_resolver::CallEdge,
    > = HashMap::new();

    let mut uncertainty_edges_by_from: HashMap<
        String,
        Vec<crate::engine::l4::combined_graph::Uncertainty>,
    > = HashMap::new();
    for ue in &graph.uncertainty_edges {
        uncertainty_edges_by_from
            .entry(ue.from.clone())
            .or_default()
            .push(ue.uncertainty.clone());
    }

    // Core summaries (v2 db-effect solver WITH dep leaves) for the path-walker
    // uncertainty union + parameter roles — same as project_r3a5_cross_app's core.
    // ⟨Task B1⟩ the LEAN bundle entry point: `core_summaries` carries EMPTY
    // `db_effects` (never re-materialized) with `.uncertainties` / `.parameter_roles`
    // populated — the only fields read below — while the compact rows stay queryable
    // via `db_effect_bundle` (held on the ctx). `summarize_diagnostics` carries the
    // roles fixpoint's cap-hit backstop (empty on the corpus).
    let (db_effect_bundle, core_summaries, summarize_diagnostics) =
        compute_summaries_v2_bundle_with_leaves(
            ws_routines,
            &graph,
            &base.combined_scc,
            &base.upgraded_bindings,
            &base.field_index,
            &base.leaf_summaries,
        );

    let mut parameter_roles_by_routine: HashMap<String, Vec<RecordRoleSummary>> = HashMap::new();
    for r in ws_routines {
        if let Some(s) = core_summaries.get(&r.id)
            && !s.parameter_roles.is_empty()
        {
            parameter_roles_by_routine.insert(r.id.clone(), s.parameter_roles.clone());
        }
    }

    let mut uncertainties_by_node: HashMap<String, Vec<Uncertainty>> = HashMap::new();
    for r in ws_routines {
        let from_summary: &[Uncertainty] = core_summaries
            .get(&r.id)
            .map(|s| s.uncertainties.as_slice())
            .unwrap_or(&[]);
        let from_edges: Vec<Uncertainty> = uncertainty_edges_by_from
            .get(&r.id)
            .map(|edges| edges.iter().map(Uncertainty::from).collect())
            .unwrap_or_default();
        if from_summary.is_empty() && from_edges.is_empty() {
            continue;
        }
        let combined: Vec<Uncertainty> = from_summary.iter().cloned().chain(from_edges).collect();
        uncertainties_by_node.insert(r.id.clone(), dedupe_uncertainties(combined));
    }

    let mut call_site_by_id: HashMap<&str, &PCallSite> = HashMap::new();
    for r in ws_routines {
        for cs in &r.call_sites {
            call_site_by_id.insert(cs.id.as_str(), cs);
        }
    }

    let upgraded_bindings_by_callsite: HashMap<String, Vec<UpgradedBinding>> =
        base.upgraded_bindings.clone();

    // Build the fingerprint index from `base.ws_routines`/`base.objects` — NOT from
    // the throwaway `merged_workspace_view` (registry.rs's `run_detectors_cross_app`
    // clones the merged sets into a local `L3Resolved` it builds AFTER calling this
    // function, so that clone doesn't exist yet at this point and can't be borrowed
    // from here anyway). `base: &'a R3a5CrossAppBase` is already the ctx's own
    // borrow source for every other eager index above, so anchoring the fingerprint
    // index to it too keeps the lifetime honest — the same 'a the whole ctx uses.
    let fingerprint_index =
        crate::engine::l5::fingerprint::FingerprintIndex::build(&base.ws_routines, &base.objects);

    let app_versions: HashMap<String, String> = base.resolved_app_versions.clone();
    let declared_dependencies: Vec<DeclaredDep> = base
        .declared_dependencies
        .iter()
        .map(|d| DeclaredDep {
            app_guid: d.app_guid.clone(),
            name: d.name.clone(),
            min_version: d.min_version.clone(),
        })
        .collect();

    DetectorContext {
        graph,
        event_graph: base.event_graph.clone(),
        routine_by_id,
        objects_by_id,
        table_by_id,
        reverse_call_graph,
        entry_points,
        transaction_spans,
        resolved_call_edge_by_callsite,
        uncertainty_edges_by_from,
        uncertainties_by_node,
        call_site_by_id,
        summaries,
        cone_derived,
        event_flow_indexes,
        parameter_roles_by_routine,
        upgraded_bindings_by_callsite,
        reachable_roots,
        internal_reachable_externally,
        dep_routine_ids: dep_routine_ids.clone(),
        declared_dependencies,
        app_versions,
        root_classifications_by_routine: HashMap::new(),
        ordering_facts: std::sync::OnceLock::new(),
        ordering_source: None,
        closed_world_temp_params,
        summarize_diagnostics,
        db_effect_bundle: Some(db_effect_bundle),
        fingerprint_index,
        cross_extension_subscribers,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Laziness contract: `build_detector_context` must NOT compute ordering facts
    /// (the OnceLock starts empty); first `get_ordering_facts()` call computes and
    /// memoizes a map EQUAL to a direct `compute_ordering_facts(resolved)` run.
    #[test]
    fn ordering_facts_are_lazy_and_parity_with_direct_compute() {
        // Empty workspace: cheap, and exercises the full lazy path end-to-end.
        let resolved = crate::engine::l3::l3_workspace::L3Resolved {
            workspace: crate::engine::l3::l3_workspace::L3Workspace {
                objects: Vec::new(),
                tables: Vec::new(),
                routines: Vec::new(),
            },
            root_classifications: Vec::new(),
            primary_app: None,
            infra_diagnostics: Vec::new(),
        };
        let ctx = build_detector_context(&resolved, crate::engine::l5::registry::substrate::ALL);
        assert!(
            ctx.ordering_facts.get().is_none(),
            "ordering facts must not be computed eagerly"
        );
        let via_ctx = ctx.get_ordering_facts();
        let direct = crate::engine::l5::ordering_facts::compute_ordering_facts(&resolved);
        assert_eq!(via_ctx.len(), direct.len());
        assert!(
            ctx.ordering_facts.get().is_some(),
            "first access must memoize"
        );
    }

    /// ⟨C1 Task 3, carry #1⟩ RELOCATED from the retired `l5::cone_parity`'s test
    /// module (that file's raw-vs-derived oracle died with the raw Vec; this test
    /// never depended on the Vec and is the ONLY unconditional pin on
    /// [`ConeDerivedStore::forget`], so it moves rather than dies). Its parity
    /// tail — `assert_cone_parity(&ctx.summaries, &ctx.cone_derived)` — is gone
    /// with the oracle; the degeneracy assertions below are the part that pinned
    /// real behaviour.
    ///
    /// Two page actions each declaring `trigger OnAction()` COLLIDE on one
    /// internal routine id (`compute_routine_id` carries no member discriminator
    /// — gap G-18). `build_detector_context` assembles summaries by `remove()`-ing
    /// each routine's cone entry, so the second occurrence gets nothing and the
    /// summary that SURVIVES is fully degenerate. The derived row must be dropped
    /// to match, or every colliding trigger in a real BC workspace would silently
    /// change output now that detectors read the row instead of the Vec.
    #[test]
    fn colliding_routine_ids_leave_summary_and_derived_row_equally_degenerate() {
        use crate::engine::l3::l3_workspace::assemble_and_resolve_default;
        use crate::engine::l5::registry::substrate;

        let src = r#"
table 50811 "CP Setup"
{
    fields { field(1; "No."; Code[20]) { } }
    keys { key(PK; "No.") { } }
}

page 50811 "CP Wizard"
{
    PageType = Card;

    actions
    {
        area(Processing)
        {
            action(First)
            {
                trigger OnAction()
                begin
                    Touch();
                end;
            }
            action(Second)
            {
                trigger OnAction()
                begin
                    Touch();
                end;
            }
        }
    }

    local procedure Touch()
    var
        Setup: Record "CP Setup";
    begin
        Setup.Insert();
    end;
}
"#;
        let files = vec![("src/CPWizard.al".to_string(), src.to_string())];
        let resolved = assemble_and_resolve_default(&files, "11111111-0000-0000-0000-0000000cp001");
        let ctx = build_detector_context(&resolved, substrate::SUMMARIES);

        // ⟨carry #1, re-review finding N-B⟩ Without this the whole test would pass
        // identically on an EMPTY store — every `flags_of` would read the empty
        // row and every `writes_tables_of` would be empty for the trivial reason,
        // silently voiding what it pins.
        assert!(
            !ctx.cone_derived.is_empty(),
            "precondition: the derived store must actually hold rows, or the \
             degeneracy assertions below are vacuous"
        );

        // The collision is real: two routines share one id, so the routine list
        // is longer than the summaries map.
        let ids: BTreeSet<&str> = resolved
            .workspace
            .routines
            .iter()
            .map(|r| r.id.as_str())
            .collect();
        assert!(
            ids.len() < resolved.workspace.routines.len(),
            "fixture precondition: the two OnAction triggers must collide on one routine id"
        );

        // The colliding id's summary is degenerate — and its derived row matches.
        // ⟨Task 3⟩ The degeneracy predicate no longer mentions
        // `capability_facts_inherited`: under `DerivedOnly` it is `None` for EVERY
        // routine, so it discriminates nothing. `direct.is_empty() &&
        // coverage.is_none()` is exactly the condition `build_detector_context`'s
        // `forget` arm keys on.
        let degenerate: Vec<&String> = ctx
            .summaries
            .iter()
            .filter(|(_, s)| s.capability_facts_direct.is_empty() && s.coverage.is_none())
            .map(|(id, _)| id)
            .collect();
        assert!(
            !degenerate.is_empty(),
            "fixture precondition: the collision must produce a degenerate summary"
        );
        for id in &degenerate {
            assert_eq!(
                ctx.cone_derived.flags_of(id),
                0,
                "{id}: a degenerate summary must carry an empty derived row"
            );
            assert!(ctx.cone_derived.writes_tables_of(id).is_empty());
        }

        // And a NON-degenerate routine still carries its folded row — the other
        // half of the pin (`forget` must drop exactly the degenerate rows, not
        // wipe the store).
        assert!(
            ctx.summaries
                .values()
                .any(|s| ctx.cone_derived.touches_table(&s.routine_id)),
            "at least one surviving routine must still reach the table write"
        );
    }
}
