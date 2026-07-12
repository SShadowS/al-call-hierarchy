//! Adjudicated legacy-vs-new differential parity harness (T3 Task 14).
//!
//! **This file is scaffolding — it dies together with the legacy engine at
//! Task 17** (design doc §8: "a buggy oracle does not outlive its
//! refutation," the al-sem retirement doctrine applied to legacy's own LSP
//! pipeline). It is the DELETION LICENSE: as long as its two gates hold,
//! deleting `graph.rs`/`indexer.rs`/`parser.rs`/legacy `handlers.rs` is safe.
//!
//! # What runs
//!
//! BOTH backends, in-process, over identical request scripts:
//! - **Legacy**: `Indexer::index_directory` (+ `index_dependencies` for the
//!   dep-bearing fixture) + `handlers::{prepare_call_hierarchy, incoming_calls,
//!   outgoing_calls, code_lens}` + `handlers::get_unused_procedure_diagnostics`
//!   — the exact calls `tests/perf_bounds.rs` already makes for the first
//!   three, `code_lens` widened `pub` here for the same T0.5-precedent
//!   reason (see `src/handlers.rs`'s doc comment on it).
//! - **New**: `LspSnapshot::build_full` + `lsp::handlers::{prepare, incoming,
//!   outgoing}` + `lsp::lens::code_lenses` + `lsp::diagnostics::compute_all`.
//!
//! Both sides are queried with **UTF-8 byte columns** (legacy already serves
//! bytes natively; new is driven with `PositionEncoding::Utf8`, under which
//! [`crate::lsp::encoding::LineTable::col_out`]/`col_in` are pass-through) so
//! H-12 conversion differences can never pollute the call-graph diff.
//!
//! # Scope decisions (read before extending)
//!
//! - **Diagnostics scope is unused-procedure ONLY.** Legacy's code-quality
//!   diagnostics (complexity/params/length/fan-in) live in `get_code_quality_diagnostics`,
//!   a PRIVATE function in the binary-only `src/server.rs` (`mod server;` in
//!   `main.rs`, never in `lib.rs`) — structurally unreachable from an
//!   integration-test crate (`tests/*.rs` links only the library target).
//!   Relocating it purely to make a scaffolding test comparable, for a
//!   module whose whole pipeline is a deletion target at Task 17, is not
//!   justified effort. `unused-procedure` (in `handlers.rs`, a `pub` library
//!   function already) is fully in scope and fully compared.
//! - **The driver enumerates call-hierarchy identities via LEGACY's
//!   `CallGraph::iter_definitions()`** (already `pub`) **union NEW's
//!   `decls_by_file`** — not a directory walk of `Definition`s per file —
//!   since `iter_definitions` already gives every `(QualifiedName,
//!   Definition)` pair directly. Identity key: `(file_rel, object_lc,
//!   routine_lc)` — sufficient for every fixture here (one object per file);
//!   a file declaring two objects that both declare a same-named routine
//!   would collide under this key. Not exercised by any corpus in this
//!   harness; flagged as a known simplification, not a silent gap.
//! - **`dependencyDocumentSymbol`/`eventPublishersInFile`/
//!   `eventReferenceAtPosition` (Task 13's custom-request surface) are OUT
//!   OF SCOPE** — the brief's Step 1 driver is prepare/incoming/outgoing/
//!   codeLens/diagnostics only. `ObjectIdAdditive` (defined in the taxonomy
//!   below per the team lead's brief) therefore never fires in this
//!   harness's own driver; its ratchet count is pinned at 0 with this
//!   documented reason, not silently omitted.
//!
//! # Universal, EXCLUDED comparison fields (documented once, never per-item)
//!
//! Two structural legacy shapes make several fields universally
//! incomparable — excluding them here (once) keeps the classifier focused
//! on genuinely adjudicated divergences instead of re-litigating the same
//! known difference on every single item:
//! - **`selection_range` is never compared.** Legacy never distinguishes it
//!   from `range` (`prepare_call_hierarchy`/`outgoing_calls` both set
//!   `selection_range: def.range`/`target_def.range`, i.e. the SAME
//!   whole-declaration span used for `range`); new correctly narrows
//!   `selection_range` to the name token (`decl.name_origin`) per real LSP
//!   convention. `range` itself DOES stay in scope and is compared.
//! - **`detail`/`data`/the "from" item's own `range` are never compared for
//!   INCOMING.** Legacy's `incoming_calls` builds every "from" item as an
//!   explicitly-labeled synthetic placeholder (`src/handlers.rs`'s own
//!   comment: "For now, create a synthetic item") — positioned at the CALL
//!   SITE (`call.range`), not the caller's own declaration, with `data:
//!   None` always. New correctly positions the "from" item at the caller's
//!   REAL declaration with real `data`. This is UNIVERSAL (100% of incoming
//!   items), not a case-by-case adjudication — comparison instead keys on
//!   (a) the caller's bare name (`item.from.name`, case-insensitively — both
//!   sides use the bare routine name here, no object qualifier) and (b) the
//!   real, byte-identical call-site position carried in `from_ranges` (both
//!   sides derive this from the SAME parsed call/event-name-origin span).
//!
//! # Taxonomy (binding — see [`Class`]/[`NewBetterClass`])
//!
//! `Match` / `Regression` (GATE: must be 0) / `NewUnexplained` (GATE: must
//! be 0) / `NewBetter(class)` for one of the brief's 9 classes plus
//! `H10Repair` (CDO edit-scenario only, the 11th) plus ONE additional class
//! discovered and adjudicated during this task's implementation,
//! `UnqualifiedCallResolved` (see its doc on [`NewBetterClass`] — found by
//! reading legacy's `outgoing_calls` literally: EVERY unqualified call,
//! resolved local target or not, renders through an unconditional
//! `"(local)"` placeholder arm that never calls `get_definition` at all;
//! this is the SAME root cause as `AbiSymbolShape`/`DepSourceSpan`
//! [a legacy shape that never carried a real position to begin with], just
//! triggered by AL's parens-optional same-object calls and bareword
//! builtins rather than by a dependency boundary).
//!
//! # CDO
//!
//! Env-gated (`CDO_WS`/`ENFORCE_CDO_WS`, `tests/common/cdo.rs`). On CDO,
//! also runs the H-10 edit scenario (Step 3's binding requirement): legacy
//! index -> legacy `reindex_file` of ONE file -> re-diff -> assert the
//! harness OBSERVES legacy losing cross-file incoming edges while new
//! (`apply_batch` of a same-file no-op save) keeps them —
//! `NewBetter(H10Repair)`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use al_call_hierarchy::config::DiagnosticConfig;
use al_call_hierarchy::handlers as legacy_handlers;
use al_call_hierarchy::indexer::Indexer;
use al_call_hierarchy::lsp::diagnostics as new_diagnostics;
use al_call_hierarchy::lsp::encoding::PositionEncoding;
use al_call_hierarchy::lsp::handlers::{self as new_handlers, ItemData};
use al_call_hierarchy::lsp::lens as new_lens;
use al_call_hierarchy::lsp::snapshot::LspSnapshot;
use al_call_hierarchy::lsp::updater::{ChangeEvent, Updater};
use al_call_hierarchy::program::graph::ProgramGraph;
use al_call_hierarchy::program::node::ObjectNodeId;
use al_call_hierarchy::protocol::path_to_uri;

use lsp_types::{
    CallHierarchyIncomingCall, CallHierarchyIncomingCallsParams, CallHierarchyItem,
    CallHierarchyOutgoingCall, CallHierarchyOutgoingCallsParams, CallHierarchyPrepareParams,
    CodeLens, CodeLensParams, Diagnostic, Position, Range, TextDocumentIdentifier,
    TextDocumentPositionParams,
};

#[path = "common/cdo.rs"]
mod cdo;

// ============================================================================
// Taxonomy
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum NewBetterClass {
    /// H-11: legacy's case-sensitive interner never associated a call site
    /// with its target because their raw text casing differs; new resolves
    /// it (case-insensitive throughout). Mechanical predicate: the
    /// new-only entry's target name equals, case-insensitively, the name of
    /// a call legacy's OWN raw `outgoing()` response shows (as a
    /// `"(local)"` placeholder — see `UnqualifiedCallResolved`) for the same
    /// caller, differing only in case.
    CaseFoldHit,
    /// Target app differs from the workspace app (a dependency).
    CrossAppTarget,
    /// Legacy had a zero/synthetic (caller-site-reused) range for an
    /// external target; new has a REAL span into embedded dependency
    /// source. Same target identity (app+object+routine) required.
    DepSourceSpan,
    /// The same pub/sub pair, present on the OTHER axis: legacy lists the
    /// subscriber under the publisher's INCOMING; new lists the subscriber
    /// under the publisher's OUTGOING and the publisher under the
    /// subscriber's INCOMING (design doc §5's "natural direction").
    EventDirectionMoved,
    /// External (SymbolOnly) target: legacy reused the CALLER's file/range
    /// with detail "(from {app})"; new emits a zero-range object-level
    /// `al-preview` item. Same target identity required.
    AbiSymbolShape,
    /// Same (caller, target, ranges-multiset) but legacy/new group vs
    /// per-site item counts differ (legacy never groups; new groups
    /// `incoming` by caller).
    OutgoingCardinality,
    /// unused-proc: a subscriber with NO resolvable EventFlow edge — legacy
    /// excludes via a blanket attribute check; new correctly flags it.
    R2Precision,
    /// unused-proc: an interface method's own signature — legacy flagged
    /// (false positive shared by both engines pre-R6), new excludes.
    R6InterfaceExclusion,
    /// `object_id`-based numbered dependencyDocumentSymbol lookup — new-only
    /// capability. Out of THIS harness's driver scope (Step 1 never queries
    /// `dependencyDocumentSymbol`); ratchet pinned at 0 with that reason, so
    /// this variant is never constructed by design (see
    /// `object_id_additive_is_out_of_driver_scope_pinned_zero`).
    #[allow(dead_code)]
    ObjectIdAdditive,
    /// Discovered during T14 implementation (see module doc): legacy's
    /// `outgoing_calls` renders EVERY unqualified call — same-object bare
    /// call OR a global/builtin bareword call — through an unconditional,
    /// self-documented ("For now, create a synthetic item"-class)
    /// `"(local)"` placeholder: `data: None`, position = the CALL's own
    /// site (never the target's), regardless of whether resolution would
    /// have succeeded. New's resolver actually resolves these: a genuine
    /// same-object call becomes a real `RouteTarget::Routine` item (a real
    /// upgrade); a bareword builtin/global call becomes `RouteTarget::Builtin`
    /// and is correctly omitted (legacy's own item there was never
    /// navigable or identity-bearing to begin with — `data: None`).
    UnqualifiedCallResolved,
    /// CDO edit-scenario only (H-10 repair): legacy `reindex_file` of one
    /// file loses ANOTHER file's cross-file incoming edges to it (H-10);
    /// new's incremental `apply_batch` of the same edit keeps them.
    H10Repair,
    /// A SECOND additional class discovered during T14 implementation:
    /// legacy's `definitions`/`incoming_calls`/`outgoing_calls` are ALL
    /// keyed purely by `QualifiedName{object, procedure}` — name only, no
    /// signature. An overload set (e.g. `Calc(Integer)`/`Calc(Text)`) hits
    /// `self.definitions.insert(qname, def)` (a plain `HashMap` insert)
    /// TWICE under the IDENTICAL key: the later declaration silently
    /// overwrites the earlier one, and `add_call_site`'s incoming/outgoing
    /// buckets for that same key MERGE both overloads' call sites together
    /// (see `src/graph.rs`'s `add_definition`/`add_call_site`) — a
    /// pre-existing legacy limitation, unrelated to any T3 engine change.
    /// New's `RoutineNodeId` (object + `name_lc` + `params_count` +
    /// `sig_fp`) keeps every overload distinct. Mechanical predicate: new
    /// has >1 `DeclEntry` sharing `(file_rel, object_lc, routine_lc)` —
    /// legacy's single collapsed slot is paired against the LAST-declared
    /// overload (matching its own last-write-wins insert order); every
    /// EARLIER overload has no legacy counterpart to compare against at
    /// all, by construction, so its prepare/incoming/outgoing comparison is
    /// skipped entirely rather than misreported as a REGRESSION or
    /// NEW_UNEXPLAINED.
    OverloadIdentityCollapsed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Class {
    Match,
    NewBetter(NewBetterClass),
    /// Legacy has it, new lacks it, no justification matched. GATE: must be 0.
    Regression,
    /// New has it, legacy lacks it, no justification matched. GATE: must be 0.
    NewUnexplained,
}

#[derive(Debug, Clone)]
struct Finding {
    request: &'static str,
    routine: String,
    class: Class,
    detail: String,
}

#[derive(Default)]
struct Ledger {
    findings: Vec<Finding>,
}

impl Ledger {
    fn push(
        &mut self,
        request: &'static str,
        routine: &str,
        class: Class,
        detail: impl Into<String>,
    ) {
        self.findings.push(Finding {
            request,
            routine: routine.to_string(),
            class,
            detail: detail.into(),
        });
    }

    fn regressions(&self) -> Vec<&Finding> {
        self.findings
            .iter()
            .filter(|f| f.class == Class::Regression)
            .collect()
    }

    fn new_unexplained(&self) -> Vec<&Finding> {
        self.findings
            .iter()
            .filter(|f| f.class == Class::NewUnexplained)
            .collect()
    }

    /// Class -> count, for the report/CHANGELOG ratchet.
    fn class_counts(&self) -> BTreeMap<String, usize> {
        let mut out = BTreeMap::new();
        for f in &self.findings {
            let key = match &f.class {
                Class::Match => "Match".to_string(),
                Class::Regression => "Regression".to_string(),
                Class::NewUnexplained => "NewUnexplained".to_string(),
                Class::NewBetter(c) => format!("NewBetter::{c:?}"),
            };
            *out.entry(key).or_insert(0) += 1;
        }
        out
    }

    fn assert_gates_clean(&self, context: &str) {
        let regressions = self.regressions();
        assert!(
            regressions.is_empty(),
            "{context}: {} REGRESSION finding(s) — legacy had it, new lacks it, unexplained:\n{}",
            regressions.len(),
            regressions
                .iter()
                .map(|f| format!("  [{}] {}: {}", f.request, f.routine, f.detail))
                .collect::<Vec<_>>()
                .join("\n")
        );
        let unexplained = self.new_unexplained();
        assert!(
            unexplained.is_empty(),
            "{context}: {} NEW_UNEXPLAINED finding(s) — new has it, legacy lacks it, unexplained:\n{}",
            unexplained.len(),
            unexplained
                .iter()
                .map(|f| format!("  [{}] {}: {}", f.request, f.routine, f.detail))
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
}

// ============================================================================
// Range normalization — both sides already byte-native (module doc)
// ============================================================================

type NormRange = (u32, u32, u32, u32);

fn nr(r: &Range) -> NormRange {
    (r.start.line, r.start.character, r.end.line, r.end.character)
}

// ============================================================================
// Legacy driver
// ============================================================================

struct LegacySide {
    indexer: Arc<RwLock<Indexer>>,
}

impl LegacySide {
    fn build(root: &Path, with_deps: bool) -> Self {
        let mut indexer = Indexer::new();
        indexer
            .index_directory(root)
            .expect("legacy index_directory");
        if with_deps {
            indexer
                .index_dependencies(root)
                .expect("legacy index_dependencies");
        }
        LegacySide {
            indexer: Arc::new(RwLock::new(indexer)),
        }
    }

    fn prepare(
        &self,
        uri: &lsp_types::Uri,
        line: u32,
        character: u32,
    ) -> Option<Vec<CallHierarchyItem>> {
        legacy_handlers::prepare_call_hierarchy(
            &self.indexer,
            CallHierarchyPrepareParams {
                text_document_position_params: TextDocumentPositionParams {
                    text_document: TextDocumentIdentifier { uri: uri.clone() },
                    position: Position { line, character },
                },
                work_done_progress_params: Default::default(),
            },
        )
        .expect("legacy prepare_call_hierarchy")
    }

    fn incoming(&self, item: &CallHierarchyItem) -> Vec<CallHierarchyIncomingCall> {
        legacy_handlers::incoming_calls(
            &self.indexer,
            CallHierarchyIncomingCallsParams {
                item: item.clone(),
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            },
        )
        .expect("legacy incoming_calls")
        .unwrap_or_default()
    }

    fn outgoing(&self, item: &CallHierarchyItem) -> Vec<CallHierarchyOutgoingCall> {
        legacy_handlers::outgoing_calls(
            &self.indexer,
            CallHierarchyOutgoingCallsParams {
                item: item.clone(),
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            },
        )
        .expect("legacy outgoing_calls")
        .unwrap_or_default()
    }

    fn code_lenses(&self, uri: &lsp_types::Uri, cfg: &DiagnosticConfig) -> Vec<CodeLens> {
        legacy_handlers::code_lens(
            &self.indexer,
            CodeLensParams {
                text_document: TextDocumentIdentifier { uri: uri.clone() },
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            },
            cfg,
        )
        .expect("legacy code_lens")
        .unwrap_or_default()
    }

    /// `(file_path_string, diagnostics)` pairs, unused-procedure ONLY (see
    /// module doc's scope decision).
    fn unused_procedure_diagnostics(&self) -> Vec<(String, Vec<Diagnostic>)> {
        let idx = self.indexer.read().unwrap();
        let graph = idx.graph();
        legacy_handlers::get_unused_procedure_diagnostics(&graph)
    }
}

// ============================================================================
// Routine identity + the per-routine raw-response sweep
// ============================================================================

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct RoutineIdentity {
    file_rel: String,
    object_lc: String,
    routine_lc: String,
}

impl RoutineIdentity {
    fn key(&self) -> String {
        format!("{}::{}.{}", self.file_rel, self.object_lc, self.routine_lc)
    }
}

/// Relative, LOWERCASED virtual path, used as the identity-key component
/// both engines' `file_rel` must agree on.
///
/// Legacy's own `Definition.file` is a `SharedPath` deduped through
/// `protocol::normalize_path` (Windows: lowercases the WHOLE absolute path —
/// `src/protocol.rs`), so a plain `strip_prefix(root)` against an
/// UN-normalized `root` fails outright on Windows (the case differs) and
/// silently falls back to the full absolute path instead of a relative one
/// — the exact bug this normalization closes. New's `virtual_path` keys are
/// case-PRESERVING (`snapshot::provider::walk_al_source`'s own doc). AL
/// (and Windows paths) are case-insensitive throughout, so lowercasing the
/// final relative string too (not just normalizing `root`/`file` first)
/// gives both sides one, non-platform-dependent identity key.
fn relativize(root: &Path, file: &Path) -> String {
    let norm_root = al_call_hierarchy::protocol::normalize_path(root);
    let norm_file = al_call_hierarchy::protocol::normalize_path(file);
    norm_file
        .strip_prefix(&norm_root)
        .unwrap_or(&norm_file)
        .to_string_lossy()
        .replace('\\', "/")
        .to_lowercase()
}

/// Local re-implementation of `lsp::handlers::object_name_for` (that one is
/// `pub(crate)` — this test file is an external crate). Mirrors it exactly:
/// `ProgramGraph::objects` is public and sorted by `ObjectNodeId`.
fn object_name<'g>(graph: &'g ProgramGraph, id: &ObjectNodeId) -> Option<&'g str> {
    graph
        .objects
        .binary_search_by(|probe| probe.id.cmp(id))
        .ok()
        .map(|i| graph.objects[i].name.as_str())
}

/// Resolve `identity` back to its real `DeclEntry` within `decls` (already
/// sorted by declaration order — `decls_by_file`'s own doc). Mirrors the
/// overload-grouping this identity was BUILT with (see the new-identities
/// loop in `run_sweep`): a suffixed `routine_lc` (`"{name}#overload{i}"`)
/// picks the `i`-th declaration in its `(object_lc, base_name)` group; an
/// un-suffixed `routine_lc` picks the LAST one (the slot legacy's
/// last-write-wins semantics actually kept — see
/// `NewBetterClass::OverloadIdentityCollapsed`).
fn find_new_decl<'a>(
    decls: &'a [al_call_hierarchy::lsp::snapshot::DeclEntry],
    graph: &ProgramGraph,
    identity: &RoutineIdentity,
) -> Option<&'a al_call_hierarchy::lsp::snapshot::DeclEntry> {
    let (base_name, target_idx) = match identity.routine_lc.split_once("#overload") {
        Some((base, idx)) => (base, idx.parse::<usize>().ok()),
        None => (identity.routine_lc.as_str(), None),
    };
    let group: Vec<&al_call_hierarchy::lsp::snapshot::DeclEntry> = decls
        .iter()
        .filter(|d| {
            object_name(graph, &d.id.object)
                .unwrap_or("")
                .eq_ignore_ascii_case(&identity.object_lc)
                && d.name.eq_ignore_ascii_case(base_name)
        })
        .collect();
    match target_idx {
        Some(i) => group.into_iter().nth(i),
        None => group.into_iter().next_back(),
    }
}

struct RoutineEntry {
    identity: RoutineIdentity,
    /// `true` for every overload EXCEPT the last-declared one when >1
    /// `DeclEntry` shares this identity's `(file_rel, object_lc,
    /// routine_lc)` key (see `NewBetterClass::OverloadIdentityCollapsed`'s
    /// doc) — legacy structurally has no counterpart for these, so the
    /// classifier skips prepare/incoming/outgoing comparison entirely and
    /// emits exactly one `OverloadIdentityCollapsed` finding instead.
    is_overload_extra: bool,
    legacy_prepare: Option<CallHierarchyItem>,
    new_prepare: Option<CallHierarchyItem>,
    legacy_incoming: Vec<CallHierarchyIncomingCall>,
    new_incoming: Vec<CallHierarchyIncomingCall>,
    legacy_outgoing: Vec<CallHierarchyOutgoingCall>,
    new_outgoing: Vec<CallHierarchyOutgoingCall>,
}

struct Sweep {
    entries: BTreeMap<String, RoutineEntry>,
    /// Bare routine names (lowercased) known to have >1 declaration
    /// SOMEWHERE in the sweep (an overload set) — populated once, during
    /// identity construction, wherever a per-file `(object_lc, routine_lc)`
    /// group has more than one member. Name-only (not file/object-scoped):
    /// a pragmatic simplification, documented here — no fixture in this
    /// harness has two DIFFERENT objects each independently overloading a
    /// SAME-named routine, so a coarser, name-only signal is sufficient to
    /// drive `NewBetterClass::OverloadIdentityCollapsed`'s cross-reference
    /// checks in `classify_outgoing`/`classify_incoming` without needing a
    /// full (file, object, name) key at each call site.
    overloaded_names: BTreeSet<String>,
}

impl Sweep {
    /// Look up every entry whose bare routine name matches `name_lc`
    /// case-insensitively — used by the classifier's cross-reference checks
    /// (event-direction / case-fold). Best-effort by design (see module
    /// doc's identity-key simplification note).
    fn by_name<'a>(&'a self, name_lc: &str) -> Vec<&'a RoutineEntry> {
        self.entries
            .values()
            .filter(|e| e.identity.routine_lc == name_lc)
            .collect()
    }

    fn is_overloaded_name(&self, name_lc: &str) -> bool {
        self.overloaded_names.contains(name_lc)
    }

    /// Every entry whose routine identity, once its `#overloadN`
    /// disambiguator is stripped, equals `base_name_lc` — i.e. every member
    /// of one overload set (the primary, un-suffixed entry AND every
    /// `is_overload_extra` sibling).
    fn siblings_by_base_name<'a>(&'a self, base_name_lc: &str) -> Vec<&'a RoutineEntry> {
        self.entries
            .values()
            .filter(|e| {
                e.identity
                    .routine_lc
                    .split_once("#overload")
                    .map(|(base, _)| base)
                    .unwrap_or(&e.identity.routine_lc)
                    == base_name_lc
            })
            .collect()
    }
}

/// `run_sweep`'s per-file codeLens return type (legacy lenses, new lenses) —
/// aliased for clippy `type_complexity`.
type LensPairsByFile = BTreeMap<String, (Vec<CodeLens>, Vec<CodeLens>)>;

/// Run the Step-1 driver over `root`: enumerate the union of legacy
/// `CallGraph::iter_definitions()` and new `decls_by_file` identities,
/// query prepare/incoming/outgoing on both sides for each, and codeLens per
/// file. Returns the raw sweep for the classifier plus per-file codeLens
/// pairs (kept separate — codeLens is per-FILE, not per-routine).
fn run_sweep(
    root: &Path,
    legacy: &LegacySide,
    new_snap: &LspSnapshot,
    cfg: &DiagnosticConfig,
) -> (Sweep, LensPairsByFile) {
    let mut entries: BTreeMap<String, RoutineEntry> = BTreeMap::new();
    let mut overloaded_names: BTreeSet<String> = BTreeSet::new();

    // ---- legacy identities, via iter_definitions (already pub) ----
    {
        let idx = legacy.indexer.read().unwrap();
        let graph = idx.graph();
        for (_qname, def) in graph.iter_definitions() {
            let object_lc = graph.resolve(def.object_name).unwrap_or("").to_lowercase();
            let routine_lc = graph.resolve(def.name).unwrap_or("").to_lowercase();
            let file_rel = relativize(root, &def.file);
            let identity = RoutineIdentity {
                file_rel,
                object_lc,
                routine_lc,
            };
            let key = identity.key();
            entries.entry(key).or_insert_with(|| RoutineEntry {
                identity,
                is_overload_extra: false,
                legacy_prepare: None,
                new_prepare: None,
                legacy_incoming: Vec::new(),
                new_incoming: Vec::new(),
                legacy_outgoing: Vec::new(),
                new_outgoing: Vec::new(),
            });
        }
    }

    // ---- new identities, via decls_by_file ----
    //
    // Grouped by `(object_lc, routine_lc)` PER FILE first (see
    // `NewBetterClass::OverloadIdentityCollapsed`'s doc): an overload set
    // (e.g. `Calc(Integer)`/`Calc(Text)`) shares one `(file_rel, object_lc,
    // routine_lc)` key, but legacy's `QualifiedName`-keyed graph structurally
    // has only ONE slot for it (the LAST declaration wins, last-write-wins
    // `HashMap::insert`). `decls` is already sorted by `origin.byte.start`
    // (declaration order) per `LspSnapshot::decls_by_file`'s own doc, so the
    // LAST element of each group is the one legacy's single slot actually
    // corresponds to; every earlier one gets a disambiguated key
    // (`#overload{i}`) and is marked `is_overload_extra` so the classifier
    // skips the (structurally meaningless) legacy comparison for it.
    for (virtual_path, decls) in &new_snap.decls_by_file {
        let mut groups: BTreeMap<
            (String, String),
            Vec<&al_call_hierarchy::lsp::snapshot::DeclEntry>,
        > = BTreeMap::new();
        for decl in decls.iter() {
            let object_lc = object_name(&new_snap.graph, &decl.id.object)
                .unwrap_or("")
                .to_lowercase();
            let routine_lc = decl.name.to_lowercase();
            groups
                .entry((object_lc, routine_lc))
                .or_default()
                .push(decl);
        }

        for ((object_lc, routine_lc), group) in groups {
            let file_rel = virtual_path.to_lowercase();
            let last_idx = group.len() - 1;
            if group.len() > 1 {
                overloaded_names.insert(routine_lc.clone());
            }
            for (i, _decl) in group.iter().enumerate() {
                let is_extra = i != last_idx;
                let identity = if is_extra {
                    RoutineIdentity {
                        file_rel: file_rel.clone(),
                        object_lc: object_lc.clone(),
                        routine_lc: format!("{routine_lc}#overload{i}"),
                    }
                } else {
                    RoutineIdentity {
                        file_rel: file_rel.clone(),
                        object_lc: object_lc.clone(),
                        routine_lc: routine_lc.clone(),
                    }
                };
                let key = identity.key();
                entries.entry(key).or_insert_with(|| RoutineEntry {
                    identity,
                    is_overload_extra: is_extra,
                    legacy_prepare: None,
                    new_prepare: None,
                    legacy_incoming: Vec::new(),
                    new_incoming: Vec::new(),
                    legacy_outgoing: Vec::new(),
                    new_outgoing: Vec::new(),
                });
            }
        }
    }

    // ---- drive prepare/incoming/outgoing per identity ----
    for entry in entries.values_mut() {
        let abs_path = root.join(&entry.identity.file_rel);
        let uri = path_to_uri(&abs_path);

        // An overload-extra identity has no legacy counterpart BY
        // CONSTRUCTION (see `NewBetterClass::OverloadIdentityCollapsed`) —
        // never query legacy for it at all; only its (real) new-side decl
        // is looked up below, so its prepare()/incoming()/outgoing() still
        // reflect real engine behavior even though nothing is compared
        // against legacy.
        if !entry.is_overload_extra {
            // Legacy: find this identity's own position via a fresh
            // get_definitions_in_file scan (cheap; fixtures are small) so we
            // don't need to retain Definition's own range separately above.
            let legacy_pos = {
                let idx = legacy.indexer.read().unwrap();
                let graph = idx.graph();
                graph
                    .get_definitions_in_file(&abs_path)
                    .into_iter()
                    .find(|d| {
                        graph
                            .resolve(d.object_name)
                            .unwrap_or("")
                            .eq_ignore_ascii_case(&entry.identity.object_lc)
                            && graph
                                .resolve(d.name)
                                .unwrap_or("")
                                .eq_ignore_ascii_case(&entry.identity.routine_lc)
                    })
                    .map(|d| d.range.start)
            };
            if let Some(pos) = legacy_pos {
                let items = legacy.prepare(&uri, pos.line, pos.character);
                if let Some(item) = items.as_ref().and_then(|v| v.first()) {
                    entry.legacy_incoming = legacy.incoming(item);
                    entry.legacy_outgoing = legacy.outgoing(item);
                }
                entry.legacy_prepare = items.and_then(|mut v| v.pop());
            }
        }

        // New. `decls_by_file` keys are case-PRESERVING while
        // `identity.file_rel` is lowercased (the cross-engine matching key —
        // see `relativize`'s doc), so a case-insensitive key scan is
        // required here, not a direct `.get`.
        let new_virtual_path = new_snap
            .decls_by_file
            .keys()
            .find(|k| k.to_lowercase() == entry.identity.file_rel);
        if let Some(decls) = new_virtual_path.and_then(|vp| new_snap.decls_by_file.get(vp))
            && let Some(decl) = find_new_decl(decls, &new_snap.graph, &entry.identity)
        {
            let items = new_handlers::prepare(
                new_snap,
                PositionEncoding::Utf8,
                uri.as_str(),
                decl.name_origin.start.row,
                decl.name_origin.start.column,
            );
            if let Some(item) = items.as_ref().and_then(|v| v.first()) {
                let data: ItemData =
                    serde_json::from_value(item.data.clone().expect("new item always has data"))
                        .expect("ItemData deserializes");
                entry.new_incoming =
                    new_handlers::incoming(new_snap, PositionEncoding::Utf8, &data);
                entry.new_outgoing =
                    new_handlers::outgoing(new_snap, PositionEncoding::Utf8, &data);
            }
            entry.new_prepare = items.and_then(|mut v| v.pop());
        }
    }

    // ---- codeLens per file ----
    // `files` prefers new's case-PRESERVING `decls_by_file` keys; a legacy
    // file with no case-insensitive match already in the set (lowercased —
    // see `relativize`'s doc) is appended as-is, a genuine "new doesn't even
    // parse this file" divergence worth surfacing rather than silently
    // merging away.
    let mut lenses: LensPairsByFile = BTreeMap::new();
    let mut files: BTreeSet<String> = new_snap.decls_by_file.keys().cloned().collect();
    {
        let idx = legacy.indexer.read().unwrap();
        let graph = idx.graph();
        for (_q, def) in graph.iter_definitions() {
            let rel = relativize(root, &def.file);
            if !files.iter().any(|f| f.to_lowercase() == rel) {
                files.insert(rel);
            }
        }
    }
    for file_rel in files {
        let abs = root.join(&file_rel);
        let uri = path_to_uri(&abs);
        let legacy_lenses = legacy.code_lenses(&uri, cfg);
        let new_lenses = new_lens::code_lenses(new_snap, PositionEncoding::Utf8, uri.as_str(), cfg);
        lenses.insert(file_rel, (legacy_lenses, new_lenses));
    }

    (
        Sweep {
            entries,
            overloaded_names,
        },
        lenses,
    )
}

// ============================================================================
// Classification: prepare
// ============================================================================

fn classify_prepare(ledger: &mut Ledger, sweep: &Sweep) {
    for entry in sweep.entries.values() {
        let routine = entry.identity.key();

        if entry.is_overload_extra {
            ledger.push(
                "prepare",
                &routine,
                Class::NewBetter(NewBetterClass::OverloadIdentityCollapsed),
                format!(
                    "new prepares {:?}; legacy's QualifiedName-keyed graph has no counterpart for this overload (see NewBetterClass::OverloadIdentityCollapsed)",
                    entry.new_prepare.as_ref().map(|i| i.name.as_str())
                ),
            );
            continue;
        }

        match (&entry.legacy_prepare, &entry.new_prepare) {
            (Some(l), Some(n)) => {
                if l.name.eq_ignore_ascii_case(&n.name) && nr(&l.range) == nr(&n.range) {
                    ledger.push("prepare", &routine, Class::Match, "range+name agree");
                } else {
                    ledger.push(
                        "prepare",
                        &routine,
                        Class::Regression,
                        format!(
                            "prepare item shape diverged: legacy name={:?} range={:?}; new name={:?} range={:?}",
                            l.name, nr(&l.range), n.name, nr(&n.range)
                        ),
                    );
                }
            }
            (Some(l), None) => ledger.push(
                "prepare",
                &routine,
                Class::Regression,
                format!(
                    "legacy prepares {:?}, new backend has no decl at all here",
                    l.name
                ),
            ),
            (None, Some(n)) => ledger.push(
                "prepare",
                &routine,
                Class::NewUnexplained,
                format!(
                    "new prepares {:?}, legacy has no Definition at all here",
                    n.name
                ),
            ),
            (None, None) => {}
        }
    }
}

// ============================================================================
// Classification: outgoing (per call SITE — the one position both engines
// derive identically, from the same parsed call/event-name-origin span)
// ============================================================================

fn is_legacy_placeholder(item: &CallHierarchyOutgoingCall) -> bool {
    item.to.data.is_none()
}

fn legacy_external_app(item: &CallHierarchyOutgoingCall) -> Option<String> {
    item.to
        .data
        .as_ref()
        .filter(|d| d.get("external").and_then(|v| v.as_bool()).unwrap_or(false))
        .and_then(|d| d.get("app"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

fn new_abi_symbol_app(item: &CallHierarchyOutgoingCall) -> Option<String> {
    item.to
        .data
        .as_ref()
        .filter(|d| d.get("external").and_then(|v| v.as_bool()).unwrap_or(false))
        .and_then(|d| d.get("app"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// If `item.to.data` is a real `ItemData` (`RouteTarget::Routine` — a
/// dependency WITH embedded source, unlike `new_abi_symbol_app`'s
/// SymbolOnly shape) whose node lives in a NON-workspace app, returns that
/// app's name — the `DepSourceSpan` predicate's "same target identity"
/// check.
fn new_dep_source_app(new_snap: &LspSnapshot, item: &CallHierarchyOutgoingCall) -> Option<String> {
    let data: ItemData = serde_json::from_value(item.to.data.clone()?).ok()?;
    let workspace_app = new_snap.graph.apps.find(&new_snap.snap.workspace_app)?;
    if data.node.object.app == workspace_app {
        return None; // a workspace target, not a dependency at all
    }
    new_snap
        .graph
        .apps
        .try_resolve(data.node.object.app)
        .map(|id| id.name.clone())
}

/// A new outgoing item is event-derived iff its own `from_ranges` equal the
/// PUBLISHING routine's own `prepare()` selection range (rule 2: an
/// EventFlow route's site is re-derived from the caller's OWN name_origin,
/// never a real call-site span) — checked against `self_prepare_range`
/// (this routine's own new-side prepare position), a mechanical, non-
/// hardcoded discriminator.
fn is_new_event_derived_outgoing(
    item: &CallHierarchyOutgoingCall,
    self_prepare_range: Option<NormRange>,
) -> bool {
    self_prepare_range
        .is_some_and(|self_range| item.from_ranges.iter().all(|r| nr(r) == self_range))
}

fn classify_outgoing(ledger: &mut Ledger, sweep: &Sweep, new_snap: &LspSnapshot) {
    for entry in sweep.entries.values() {
        // Adjudicated once, in classify_prepare (OverloadIdentityCollapsed)
        // — legacy has no counterpart for this identity at all.
        if entry.is_overload_extra {
            continue;
        }
        let routine = entry.identity.key();
        let self_prepare_range = entry.new_prepare.as_ref().map(|i| nr(&i.selection_range));

        // Event-flow-derived items are adjudicated in classify_event_direction
        // — exclude them here so the ordinary call-site diff below doesn't
        // double-count / falsely regress them.
        let new_ordinary: Vec<&CallHierarchyOutgoingCall> = entry
            .new_outgoing
            .iter()
            .filter(|i| !is_new_event_derived_outgoing(i, self_prepare_range))
            .collect();

        let mut legacy_by_site: BTreeMap<NormRange, Vec<&CallHierarchyOutgoingCall>> =
            BTreeMap::new();
        for item in &entry.legacy_outgoing {
            for r in &item.from_ranges {
                legacy_by_site.entry(nr(r)).or_default().push(item);
            }
        }
        let mut new_by_site: BTreeMap<NormRange, Vec<&CallHierarchyOutgoingCall>> = BTreeMap::new();
        for item in &new_ordinary {
            for r in &item.from_ranges {
                new_by_site.entry(nr(r)).or_default().push(item);
            }
        }

        let mut all_sites: BTreeSet<NormRange> = legacy_by_site.keys().copied().collect();
        all_sites.extend(new_by_site.keys().copied());

        for site in all_sites {
            let l_items = legacy_by_site.get(&site).cloned().unwrap_or_default();
            let n_items = new_by_site.get(&site).cloned().unwrap_or_default();

            match (l_items.as_slice(), n_items.as_slice()) {
                ([], []) => unreachable!("site came from one of the two maps"),
                ([l], [n]) => classify_outgoing_pair(ledger, sweep, new_snap, &routine, l, n),
                ([], _) => {
                    for n in &n_items {
                        ledger.push(
                            "outgoing",
                            &routine,
                            Class::NewUnexplained,
                            format!(
                                "new-only outgoing item at site {site:?}: target={:?}",
                                n.to.name
                            ),
                        );
                    }
                }
                (_, []) => {
                    for l in &l_items {
                        classify_outgoing_legacy_only(ledger, &routine, l);
                    }
                }
                _ => {
                    if l_items.len() != n_items.len() {
                        ledger.push(
                            "outgoing",
                            &routine,
                            Class::NewBetter(NewBetterClass::OutgoingCardinality),
                            format!(
                                "site {site:?}: legacy {} item(s) vs new {} item(s)",
                                l_items.len(),
                                n_items.len()
                            ),
                        );
                    } else {
                        for (l, n) in l_items.iter().zip(n_items.iter()) {
                            classify_outgoing_pair(ledger, sweep, new_snap, &routine, l, n);
                        }
                    }
                }
            }
        }
    }
}

fn classify_outgoing_pair(
    ledger: &mut Ledger,
    sweep: &Sweep,
    new_snap: &LspSnapshot,
    routine: &str,
    l: &CallHierarchyOutgoingCall,
    n: &CallHierarchyOutgoingCall,
) {
    // OverloadIdentityCollapsed: legacy's single collapsed (object, name)
    // slot can point at the WRONG overload's position entirely (it has no
    // arg-type dispatch at all — `resolve_call` never looks at argument
    // types, just the qualified/unqualified object+method name) — so a
    // SAME-named target with a DIFFERENT range, where the name is a known
    // overloaded routine, is this class, not an unexplained divergence.
    if l.to.name.eq_ignore_ascii_case(&n.to.name)
        && sweep.is_overloaded_name(&n.to.name.to_lowercase())
    {
        ledger.push(
            "outgoing",
            routine,
            Class::NewBetter(NewBetterClass::OverloadIdentityCollapsed),
            format!(
                "target {:?} is overloaded: legacy's single collapsed slot (data={:?}) may not even be the SAME overload new's arg-type dispatch correctly resolves to (new range={:?})",
                n.to.name, l.to.data, nr(&n.to.range)
            ),
        );
        return;
    }

    if l.to.name.eq_ignore_ascii_case(&n.to.name) && nr(&l.to.range) == nr(&n.to.range) {
        ledger.push(
            "outgoing",
            routine,
            Class::Match,
            format!("target={:?}", n.to.name),
        );
        return;
    }

    if is_legacy_placeholder(l) {
        ledger.push(
            "outgoing",
            routine,
            Class::NewBetter(NewBetterClass::UnqualifiedCallResolved),
            format!(
                "legacy placeholder (data:None, detail={:?}) upgraded to a real target {:?}",
                l.to.detail, n.to.name
            ),
        );
        return;
    }

    if let Some(l_app) = legacy_external_app(l) {
        // AbiSymbolShape: new resolved to a SymbolOnly dep — a
        // `RouteTarget::AbiSymbol` zero-range al-preview item, the SAME
        // "external":true/"app" shape legacy's arm 2 uses.
        if let Some(n_app) = new_abi_symbol_app(n)
            && l_app.eq_ignore_ascii_case(&n_app)
        {
            ledger.push(
                "outgoing",
                routine,
                Class::NewBetter(NewBetterClass::AbiSymbolShape),
                format!("external target app={n_app}, legacy caller-site stand-in vs new zero-range al-preview item"),
            );
            ledger.push(
                "outgoing",
                routine,
                Class::NewBetter(NewBetterClass::CrossAppTarget),
                format!("target app {n_app} != workspace app"),
            );
            return;
        }
        // DepSourceSpan: new resolved to a REAL `RouteTarget::Routine` in a
        // NON-workspace app (embedded-source dependency) — a genuine
        // navigable span legacy's arm 2 could never produce (it always
        // reuses the CALLER's own site as a stand-in, `data: {"external":
        // true, "app": ...}`, never a real target position).
        if let Some(n_app) = new_dep_source_app(new_snap, n)
            && l_app.eq_ignore_ascii_case(&n_app)
        {
            ledger.push(
                "outgoing",
                routine,
                Class::NewBetter(NewBetterClass::DepSourceSpan),
                format!(
                    "external target app={n_app}: legacy caller-site stand-in vs new REAL dep-source span {:?}",
                    nr(&n.to.range)
                ),
            );
            ledger.push(
                "outgoing",
                routine,
                Class::NewBetter(NewBetterClass::CrossAppTarget),
                format!("target app {n_app} != workspace app"),
            );
            return;
        }
    }

    ledger.push(
        "outgoing",
        routine,
        Class::Regression,
        format!(
            "unexplained outgoing shape divergence: legacy name={:?} data={:?}; new name={:?} range={:?}",
            l.to.name, l.to.data, n.to.name, nr(&n.to.range)
        ),
    );
}

fn classify_outgoing_legacy_only(
    ledger: &mut Ledger,
    routine: &str,
    l: &CallHierarchyOutgoingCall,
) {
    if is_legacy_placeholder(l) {
        ledger.push(
            "outgoing",
            routine,
            Class::NewBetter(NewBetterClass::UnqualifiedCallResolved),
            format!(
                "legacy placeholder (detail={:?}) for a builtin/unresolvable bareword call; new correctly omits it (RouteTarget::Builtin/Unresolved)",
                l.to.detail
            ),
        );
    } else {
        ledger.push(
            "outgoing",
            routine,
            Class::Regression,
            format!(
                "legacy has a real outgoing item {:?}, new has nothing at this site",
                l.to.name
            ),
        );
    }
}

// ============================================================================
// Classification: incoming (per call SITE, "from" item's own range/data
// excluded per the module doc's universal-exclusion note)
// ============================================================================

fn classify_incoming(ledger: &mut Ledger, sweep: &Sweep) {
    for entry in sweep.entries.values() {
        // Adjudicated once, in classify_prepare (OverloadIdentityCollapsed).
        if entry.is_overload_extra {
            continue;
        }
        let routine = entry.identity.key();

        // Event-flow-derived legacy entries are adjudicated in
        // classify_event_direction (identified via legacy's OWN explicit
        // "[EventSubscriber]" detail suffix — see `incoming_calls`'s code).
        let legacy_ordinary: Vec<&CallHierarchyIncomingCall> = entry
            .legacy_incoming
            .iter()
            .filter(|i| {
                !i.from
                    .detail
                    .as_deref()
                    .is_some_and(|d| d.ends_with("[EventSubscriber]"))
            })
            .collect();
        // Event-flow-derived new entries carry the explicit "[EventPublisher]"
        // tag Task 11 added to the "from" item's own detail.
        let new_ordinary: Vec<&CallHierarchyIncomingCall> = entry
            .new_incoming
            .iter()
            .filter(|i| {
                !i.from
                    .detail
                    .as_deref()
                    .is_some_and(|d| d.contains("[EventPublisher]"))
            })
            .collect();

        let mut legacy_by_site: BTreeMap<NormRange, String> = BTreeMap::new();
        for item in &legacy_ordinary {
            for r in &item.from_ranges {
                legacy_by_site.insert(nr(r), item.from.name.to_lowercase());
            }
        }
        let mut new_by_site: BTreeMap<NormRange, String> = BTreeMap::new();
        for item in &new_ordinary {
            for r in &item.from_ranges {
                new_by_site.insert(nr(r), item.from.name.to_lowercase());
            }
        }

        let mut all_sites: BTreeSet<NormRange> = legacy_by_site.keys().copied().collect();
        all_sites.extend(new_by_site.keys().copied());

        for site in all_sites {
            match (legacy_by_site.get(&site), new_by_site.get(&site)) {
                (Some(l_name), Some(n_name)) => {
                    if l_name.eq_ignore_ascii_case(n_name) {
                        ledger.push(
                            "incoming",
                            &routine,
                            Class::Match,
                            format!("caller={l_name}"),
                        );
                    } else {
                        ledger.push(
                            "incoming",
                            &routine,
                            Class::Regression,
                            format!(
                                "same site {site:?}: legacy caller={l_name} vs new caller={n_name}"
                            ),
                        );
                    }
                }
                (Some(l_name), None) => {
                    // OverloadIdentityCollapsed: legacy merges every
                    // overload's callers into ONE incoming bucket (no
                    // arg-type dispatch at all). If THIS exact site
                    // resolves, on the new side, into a SIBLING overload's
                    // own incoming set instead, that's the explanation —
                    // the caller genuinely targets a different overload new
                    // correctly distinguishes.
                    let base_name = entry
                        .identity
                        .routine_lc
                        .split_once("#overload")
                        .map(|(b, _)| b)
                        .unwrap_or(&entry.identity.routine_lc);
                    let found_in_sibling = sweep.is_overloaded_name(base_name)
                        && sweep.siblings_by_base_name(base_name).iter().any(|sib| {
                            sib.new_incoming
                                .iter()
                                .any(|i| i.from_ranges.iter().any(|r| nr(r) == site))
                        });
                    if found_in_sibling {
                        ledger.push(
                            "incoming",
                            &routine,
                            Class::NewBetter(NewBetterClass::OverloadIdentityCollapsed),
                            format!(
                                "legacy caller={l_name} at site {site:?} merged into this overload's bucket; new correctly attributes it to a SIBLING overload of {base_name:?}"
                            ),
                        );
                    } else {
                        ledger.push(
                            "incoming",
                            &routine,
                            Class::Regression,
                            format!("legacy caller={l_name} at site {site:?}, new has nothing"),
                        );
                    }
                }
                (None, Some(n_name)) => {
                    // CaseFoldHit: does legacy's OWN raw outgoing() for this
                    // exact caller (by name, case-insensitively — the only
                    // handle we have on "legacy's view of this caller")
                    // show a placeholder targeting this routine's REAL
                    // (case-preserved) declared name, with the CALL SITE's
                    // OWN raw text differing only in case? Cross-references
                    // the SAME caller identity's already-collected
                    // legacy_outgoing. `entry.identity.routine_lc` is
                    // already lowercased (useless for a case-DIFFERENCE
                    // check), so the real declared name comes from this
                    // routine's own `new_prepare` item instead.
                    let declared_name = entry
                        .new_prepare
                        .as_ref()
                        .map(|i| i.name.as_str())
                        .unwrap_or(&entry.identity.routine_lc);
                    let case_fold = sweep.by_name(n_name).iter().any(|caller_entry| {
                        caller_entry.legacy_outgoing.iter().any(|o| {
                            is_legacy_placeholder(o)
                                && o.to.name.eq_ignore_ascii_case(declared_name)
                                && o.to.name != declared_name
                        })
                    });
                    if case_fold {
                        ledger.push(
                            "incoming",
                            &routine,
                            Class::NewBetter(NewBetterClass::CaseFoldHit),
                            format!("new caller={n_name} at site {site:?}; legacy's interner never associated the differently-cased call site"),
                        );
                    } else {
                        ledger.push(
                            "incoming",
                            &routine,
                            Class::NewUnexplained,
                            format!("new caller={n_name} at site {site:?}, legacy has nothing"),
                        );
                    }
                }
                (None, None) => unreachable!(),
            }
        }

        // OutgoingCardinality's incoming-axis counterpart: same caller
        // (case-insensitive), non-empty on both sides, but a DIFFERENT
        // number of DISCRETE response items (legacy never groups by caller;
        // new does) even though the flattened site set already matched
        // above 1:1. Detected by comparing raw item counts per caller name.
        let mut legacy_counts: BTreeMap<String, usize> = BTreeMap::new();
        for item in &legacy_ordinary {
            *legacy_counts
                .entry(item.from.name.to_lowercase())
                .or_insert(0) += 1;
        }
        let mut new_counts: BTreeMap<String, usize> = BTreeMap::new();
        for item in &new_ordinary {
            *new_counts.entry(item.from.name.to_lowercase()).or_insert(0) += 1;
        }
        for (caller, l_count) in &legacy_counts {
            if let Some(n_count) = new_counts.get(caller)
                && l_count != n_count
            {
                ledger.push(
                    "incoming",
                    &routine,
                    Class::NewBetter(NewBetterClass::OutgoingCardinality),
                    format!("caller={caller}: legacy {l_count} ungrouped item(s) vs new {n_count} grouped item(s)"),
                );
            }
        }
    }
}

// ============================================================================
// Classification: event direction (dedicated identity-based cross-check —
// NOT site-based, since the publisher's own position differs structurally
// from the subscriber's own position; see module doc)
// ============================================================================

fn classify_event_direction(ledger: &mut Ledger, sweep: &Sweep) {
    for entry in sweep.entries.values() {
        // Every legacy incoming entry tagged "[EventSubscriber]" names a
        // real subscriber of THIS routine (as publisher, per legacy's own
        // convention: `get_event_subscribers(&qname)` where qname is the
        // ROUTINE WHOSE incomingCalls we queried).
        for item in &entry.legacy_incoming {
            let Some(detail) = &item.from.detail else {
                continue;
            };
            if !detail.ends_with("[EventSubscriber]") {
                continue;
            }
            let subscriber_lc = item.from.name.to_lowercase();
            let publisher_lc = &entry.identity.routine_lc;

            // New must NOT show the subscriber under the publisher's own
            // incoming (direction moved away) — a mechanical sanity check,
            // not itself a finding.
            let publisher_routine = entry.identity.key();

            // New: subscriber should appear under the publisher's OUTGOING.
            let found_in_new_outgoing = entry
                .new_outgoing
                .iter()
                .any(|o| o.to.name.eq_ignore_ascii_case(&subscriber_lc));
            // New: publisher should appear under the subscriber's INCOMING.
            let found_in_new_incoming_of_subscriber =
                sweep.by_name(&subscriber_lc).iter().any(|sub_entry| {
                    sub_entry
                        .new_incoming
                        .iter()
                        .any(|i| i.from.name.eq_ignore_ascii_case(publisher_lc))
                });

            if found_in_new_outgoing || found_in_new_incoming_of_subscriber {
                ledger.push(
                    "incoming",
                    &publisher_routine,
                    Class::NewBetter(NewBetterClass::EventDirectionMoved),
                    format!(
                        "subscriber={subscriber_lc}: legacy listed it under publisher's incoming; new moved it to publisher's outgoing / subscriber's incoming (found_in_outgoing={found_in_new_outgoing}, found_in_subscriber_incoming={found_in_new_incoming_of_subscriber})"
                    ),
                );
            } else {
                ledger.push(
                    "incoming",
                    &publisher_routine,
                    Class::Regression,
                    format!("legacy lists subscriber={subscriber_lc} under publisher's incoming; new shows it NOWHERE (neither publisher's outgoing nor subscriber's incoming)"),
                );
            }
        }
    }
}

// ============================================================================
// Classification: codeLens (paired by (object_lc, routine_lc); ref-count
// text tolerates a CaseFoldHit-explained delta)
// ============================================================================

fn lens_key(l: &CodeLens) -> Option<(String, String)> {
    let args = l.command.as_ref()?.arguments.as_ref()?;
    let obj = args[0].get("object")?.as_str()?.to_lowercase();
    let proc = args[0].get("procedure")?.as_str()?.to_lowercase();
    Some((obj, proc))
}

fn lens_ref_count(l: &CodeLens) -> Option<usize> {
    let title = &l.command.as_ref()?.title;
    let n = title.split_whitespace().next()?;
    n.parse().ok()
}

fn classify_code_lens(
    ledger: &mut Ledger,
    sweep: &Sweep,
    file_rel: &str,
    legacy: &[CodeLens],
    new: &[CodeLens],
) {
    let mut legacy_by_key: BTreeMap<(String, String), &CodeLens> = BTreeMap::new();
    for l in legacy {
        if let Some(k) = lens_key(l) {
            legacy_by_key.insert(k, l);
        }
    }
    let mut new_by_key: BTreeMap<(String, String), &CodeLens> = BTreeMap::new();
    for n in new {
        if let Some(k) = lens_key(n) {
            new_by_key.insert(k, n);
        }
    }

    let mut all_keys: BTreeSet<(String, String)> = legacy_by_key.keys().cloned().collect();
    all_keys.extend(new_by_key.keys().cloned());

    for key in all_keys {
        let routine = format!("{file_rel}::{}.{}", key.0, key.1);
        match (legacy_by_key.get(&key), new_by_key.get(&key)) {
            (Some(l), Some(n)) => {
                let l_refs = lens_ref_count(l);
                let n_refs = lens_ref_count(n);
                if l_refs == n_refs {
                    ledger.push("codeLens", &routine, Class::Match, "ref count + key agree");
                } else if n_refs.unwrap_or(0) > l_refs.unwrap_or(0) {
                    // A higher new-side ref count on an otherwise-matched
                    // lens key needs disambiguating between two DIFFERENT
                    // root causes that both inflate `effective_incoming_count`
                    // relative to legacy's `get_incoming_call_count`:
                    // EventDirectionMoved (this routine is a SUBSCRIBER —
                    // its publisher now counts as an incoming caller, a
                    // linkage legacy's `event_subscriptions` map, keyed by
                    // PUBLISHER not subscriber, can never show on the
                    // subscriber's OWN lens) takes priority; otherwise it's
                    // CaseFoldHit's codeLens footprint (an extra caller
                    // legacy's interner never associated at all).
                    let entry_key = RoutineIdentity {
                        file_rel: file_rel.to_lowercase(),
                        object_lc: key.0.clone(),
                        routine_lc: key.1.clone(),
                    }
                    .key();
                    let is_event_linked = sweep.entries.get(&entry_key).is_some_and(|e| {
                        e.new_incoming.iter().any(|i| {
                            i.from
                                .detail
                                .as_deref()
                                .is_some_and(|d| d.contains("[EventPublisher]"))
                        })
                    });
                    if is_event_linked {
                        ledger.push(
                            "codeLens",
                            &routine,
                            Class::NewBetter(NewBetterClass::EventDirectionMoved),
                            format!("ref count legacy={l_refs:?} vs new={n_refs:?} (new counts the publisher as an incoming caller; legacy's event_subscriptions map is keyed by publisher, never surfaced on the subscriber's own lens)"),
                        );
                    } else {
                        ledger.push(
                            "codeLens",
                            &routine,
                            Class::NewBetter(NewBetterClass::CaseFoldHit),
                            format!("ref count legacy={l_refs:?} vs new={n_refs:?} (new counts a case-fold-only caller)"),
                        );
                    }
                } else if sweep.is_overloaded_name(&key.1) {
                    // OverloadIdentityCollapsed: legacy's merged (object,
                    // name) incoming bucket counts callers of EVERY
                    // overload; new correctly counts only THIS overload's
                    // own callers.
                    ledger.push(
                        "codeLens",
                        &routine,
                        Class::NewBetter(NewBetterClass::OverloadIdentityCollapsed),
                        format!(
                            "ref count legacy={l_refs:?} (merges every overload's callers) vs new={n_refs:?} (this overload only)"
                        ),
                    );
                } else {
                    ledger.push(
                        "codeLens",
                        &routine,
                        Class::Regression,
                        format!("ref count legacy={l_refs:?} vs new={n_refs:?}"),
                    );
                }
            }
            (Some(_), None) => ledger.push(
                "codeLens",
                &routine,
                Class::Regression,
                "legacy has a lens here, new has none",
            ),
            (None, Some(_)) => ledger.push(
                "codeLens",
                &routine,
                Class::NewUnexplained,
                "new has a lens here, legacy has none",
            ),
            (None, None) => {}
        }
    }
}

// ============================================================================
// Classification: diagnostics (unused-procedure only; matched by
// (uri, code, message) — range EXTENT is known to differ universally, see
// module doc, and is not part of the equivalence key)
// ============================================================================

fn classify_diagnostics(
    ledger: &mut Ledger,
    root: &Path,
    legacy: &LegacySide,
    new_snap: &LspSnapshot,
    cfg: &DiagnosticConfig,
    sweep: &Sweep,
) {
    // Keyed by the SAME lowercased `relativize`d identity `run_sweep` uses
    // (see that function's doc) — NOT the raw `file://` URI string, which
    // differs in case between legacy (fully lowercased, `normalize_path`)
    // and new (case-preserving `virtual_path`) and would otherwise silently
    // fail to merge two sides' diagnostics for the SAME file into one key.
    let legacy_diags = legacy.unused_procedure_diagnostics();
    let mut legacy_by_file: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (file_path, diags) in legacy_diags {
        let file_rel = relativize(root, Path::new(&file_path));
        let messages: BTreeSet<String> = diags.into_iter().map(|d| d.message).collect();
        legacy_by_file.entry(file_rel).or_default().extend(messages);
    }

    let new_all = new_diagnostics::compute_all(new_snap, cfg);
    let mut new_by_file: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (uri, diags) in new_all {
        let file_rel = uri_to_rel(root, &uri);
        let messages: BTreeSet<String> = diags
            .into_iter()
            .filter(|d| {
                d.code
                    .as_ref()
                    .is_some_and(|c| matches!(c, lsp_types::NumberOrString::String(s) if s == "unused-procedure"))
            })
            .map(|d| d.message)
            .collect();
        new_by_file.entry(file_rel).or_default().extend(messages);
    }

    let mut all_files: BTreeSet<String> = legacy_by_file.keys().cloned().collect();
    all_files.extend(new_by_file.keys().cloned());

    for file_rel in all_files {
        let empty = BTreeSet::new();
        let l = legacy_by_file.get(&file_rel).unwrap_or(&empty);
        let n = new_by_file.get(&file_rel).unwrap_or(&empty);

        for msg in l.difference(n) {
            // R6: an interface method's own signature — legacy flagged
            // (false positive shared pre-R6), new excludes.
            let is_r6 = looks_like_interface_signature(new_snap, &file_rel, msg);
            // CaseFoldHit: legacy's `get_unused_procedures` sees ZERO
            // incoming calls only because the call site's raw text differs
            // in case from the declaration (H-11) — new's `incoming` for
            // the SAME routine (looked up by name in this file) is
            // non-empty, proving a real caller exists that legacy's
            // case-sensitive interner never associated.
            let is_case_fold = routine_name_has_new_incoming(sweep, &file_rel, msg);
            if is_r6 {
                ledger.push(
                    "diagnostics",
                    &file_rel,
                    Class::NewBetter(NewBetterClass::R6InterfaceExclusion),
                    format!("legacy flags {msg:?}; new excludes (interface method signature)"),
                );
            } else if is_case_fold {
                ledger.push(
                    "diagnostics",
                    &file_rel,
                    Class::NewBetter(NewBetterClass::CaseFoldHit),
                    format!("legacy flags {msg:?} (zero case-sensitive incoming); new sees a real, differently-cased caller"),
                );
            } else {
                ledger.push(
                    "diagnostics",
                    &file_rel,
                    Class::Regression,
                    format!("legacy flags {msg:?}, new does not"),
                );
            }
        }
        for msg in n.difference(l) {
            // R2Precision: a subscriber with no resolvable EventFlow edge —
            // legacy's blanket attribute exclusion hides it; new flags it.
            ledger.push(
                "diagnostics",
                &file_rel,
                Class::NewBetter(NewBetterClass::R2Precision),
                format!("new flags {msg:?}, legacy's blanket [EventSubscriber] exclusion hides it"),
            );
        }
        for msg in l.intersection(n) {
            ledger.push("diagnostics", &file_rel, Class::Match, msg.clone());
        }
    }
}

fn uri_to_rel(root: &Path, uri: &str) -> String {
    let parsed: lsp_types::Uri = match uri.parse() {
        Ok(u) => u,
        Err(_) => return uri.to_string(),
    };
    match al_call_hierarchy::protocol::uri_to_path(&parsed) {
        Some(p) => relativize(root, &p),
        None => uri.to_string(),
    }
}

/// Best-effort R6 detector: does `new_snap` have a decl at this uri whose
/// containing object is an `Interface`, with a name embedded in `msg`
/// (legacy's `unused-procedure` message format: "Procedure '{object}.{name}'
/// is never called")?
fn looks_like_interface_signature(new_snap: &LspSnapshot, file_rel: &str, msg: &str) -> bool {
    let Some(name) = msg
        .split('\'')
        .nth(1)
        .and_then(|qualified| qualified.split('.').next_back())
    else {
        return false;
    };
    let Some(virtual_path) = new_snap
        .decls_by_file
        .keys()
        .find(|k| k.to_lowercase() == file_rel)
    else {
        return false;
    };
    new_snap.decls_by_file[virtual_path].iter().any(|d| {
        d.name.eq_ignore_ascii_case(name)
            && d.id.object.kind == al_syntax::ir::ObjectKind::Interface
    })
}

/// Extracts the routine name from legacy's `unused-procedure` message
/// ("Procedure '{object}.{name}' is never called") and checks whether ANY
/// sweep entry for that name IN THIS FILE has a non-empty `new_incoming` —
/// i.e. the new engine sees a real caller legacy's zero-incoming count
/// missed entirely. See `classify_diagnostics`'s `CaseFoldHit` arm.
fn routine_name_has_new_incoming(sweep: &Sweep, file_rel: &str, msg: &str) -> bool {
    let Some(name) = msg
        .split('\'')
        .nth(1)
        .and_then(|qualified| qualified.split('.').next_back())
    else {
        return false;
    };
    let name_lc = name.to_lowercase();
    sweep.entries.values().any(|e| {
        e.identity.file_rel == file_rel
            && e.identity.routine_lc == name_lc
            && !e.new_incoming.is_empty()
    })
}

// ============================================================================
// Top-level entry point
// ============================================================================

fn run_differential(root: &Path, with_deps: bool) -> Ledger {
    let cfg = DiagnosticConfig::default();
    let legacy = LegacySide::build(root, with_deps);
    let new_snap = LspSnapshot::build_full(root).expect("LspSnapshot::build_full");

    let (sweep, lenses) = run_sweep(root, &legacy, &new_snap, &cfg);

    let mut ledger = Ledger::default();
    classify_prepare(&mut ledger, &sweep);
    classify_outgoing(&mut ledger, &sweep, &new_snap);
    classify_incoming(&mut ledger, &sweep);
    classify_event_direction(&mut ledger, &sweep);
    for (file_rel, (legacy_lenses, new_lenses)) in &lenses {
        classify_code_lens(&mut ledger, &sweep, file_rel, legacy_lenses, new_lenses);
    }
    classify_diagnostics(&mut ledger, root, &legacy, &new_snap, &cfg, &sweep);

    ledger
}

fn fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

// ============================================================================
// Always-on fixture runs (Step 3)
// ============================================================================

#[test]
fn lsp_incr_fixture_has_zero_regressions_and_zero_unexplained() {
    // Task 10's own fixture (overloads, events, table/page/tableextension,
    // Unicode) wasn't purpose-built for this harness, but its `Calc`
    // overload set + `OnAfterWork`/`HandleAfterWork` pub/sub pair + repeated
    // calls to `Beta.Process` incidentally exercise several classes too —
    // pinned exact per the brief's "fixture-run pins are always-on".
    let ledger = run_differential(&fixture_path("lsp-incr"), false);
    ledger.assert_gates_clean("lsp-incr");
    let counts = ledger.class_counts();
    assert!(
        counts.get("Match").copied().unwrap_or(0) > 0,
        "sanity: must have SOME matches, not just an empty run; counts={counts:?}"
    );
    assert_eq!(
        counts
            .get("NewBetter::EventDirectionMoved")
            .copied()
            .unwrap_or(0),
        2,
        "EventDirectionMoved: Alpha.OnAfterWork / Beta.HandleAfterWork (incoming + codeLens); counts={counts:?}"
    );
    assert_eq!(
        counts
            .get("NewBetter::OutgoingCardinality")
            .copied()
            .unwrap_or(0),
        1,
        "OutgoingCardinality: Beta.Process has 3 callers, at least one with >1 call site once grouped; counts={counts:?}"
    );
    assert_eq!(
        counts
            .get("NewBetter::OverloadIdentityCollapsed")
            .copied()
            .unwrap_or(0),
        7,
        "OverloadIdentityCollapsed: Alpha's Calc(Integer)/Calc(Text) overload set, across prepare/incoming/outgoing/codeLens; counts={counts:?}"
    );
    assert_eq!(
        counts
            .get("NewBetter::UnqualifiedCallResolved")
            .copied()
            .unwrap_or(0),
        1,
        "UnqualifiedCallResolved: Alpha.DoWork's bareword Løbenr() call; counts={counts:?}"
    );
}

#[test]
fn lsp_diff_core_fixture_has_zero_regressions_and_zero_unexplained() {
    let ledger = run_differential(&fixture_path("lsp-diff-core"), false);
    ledger.assert_gates_clean("lsp-diff-core");
    let counts = ledger.class_counts();

    // Pin every class this fixture is DESIGNED to exercise (ratchet-style,
    // per the brief's Step 4 — fixture-run pins are always-on).
    assert_eq!(
        counts.get("NewBetter::CaseFoldHit").copied().unwrap_or(0),
        3,
        "CaseFoldHit: 1 incoming + 1 codeLens + 1 diagnostics finding, all for Gamma.Callee's case-mismatched call site; counts={counts:?}"
    );
    assert_eq!(
        counts
            .get("NewBetter::R6InterfaceExclusion")
            .copied()
            .unwrap_or(0),
        1,
        "R6InterfaceExclusion: IShape.Area's signature; counts={counts:?}"
    );
    assert_eq!(
        counts.get("NewBetter::R2Precision").copied().unwrap_or(0),
        1,
        "R2Precision: Epsilon.Misdirected; counts={counts:?}"
    );
    assert_eq!(
        counts
            .get("NewBetter::EventDirectionMoved")
            .copied()
            .unwrap_or(0),
        2,
        "EventDirectionMoved: 1 incoming + 1 codeLens finding, both for Delta.OnThingHappened / Epsilon.Handle; counts={counts:?}"
    );
    assert_eq!(
        counts
            .get("NewBetter::OutgoingCardinality")
            .copied()
            .unwrap_or(0),
        1,
        "OutgoingCardinality: Zeta.CallTwice's two calls to Delta.OnThingHappened; counts={counts:?}"
    );
    assert_eq!(
        counts
            .get("NewBetter::UnqualifiedCallResolved")
            .copied()
            .unwrap_or(0),
        2,
        "UnqualifiedCallResolved: Gamma.Caller's callee()+Message(...) unqualified calls; counts={counts:?}"
    );
}

#[test]
fn lsp_diff_deps_fixture_has_zero_regressions_and_zero_unexplained() {
    let ledger = run_differential(&fixture_path("lsp-diff-deps"), true);
    ledger.assert_gates_clean("lsp-diff-deps");
    let counts = ledger.class_counts();

    assert_eq!(
        counts
            .get("NewBetter::AbiSymbolShape")
            .copied()
            .unwrap_or(0),
        1,
        "AbiSymbolShape: Caller.CallSymbolOnlyDep -> Widget Mgt.Compute (Core Lib, SymbolOnly); counts={counts:?}"
    );
    assert_eq!(
        counts.get("NewBetter::DepSourceSpan").copied().unwrap_or(0),
        1,
        "DepSourceSpan: Caller.CallEmbeddedSourceDep -> Source Mgt.DoWork (Source Lib, embedded source); counts={counts:?}"
    );
    assert_eq!(
        counts
            .get("NewBetter::CrossAppTarget")
            .copied()
            .unwrap_or(0),
        2,
        "CrossAppTarget: both dependency targets are in a non-workspace app; counts={counts:?}"
    );
}

/// `ObjectIdAdditive` is out-of-scope for this harness's own driver (module
/// doc's scope decision — Step 1 never queries `dependencyDocumentSymbol`).
/// Pinned at 0 across every always-on fixture, with the reason documented,
/// rather than silently omitted from the ratchet.
#[test]
fn object_id_additive_is_out_of_driver_scope_pinned_zero() {
    for fixture in ["lsp-incr", "lsp-diff-core", "lsp-diff-deps"] {
        let with_deps = fixture == "lsp-diff-deps";
        let ledger = run_differential(&fixture_path(fixture), with_deps);
        let counts = ledger.class_counts();
        assert_eq!(
            counts
                .get("NewBetter::ObjectIdAdditive")
                .copied()
                .unwrap_or(0),
            0,
            "ObjectIdAdditive never fires: this harness's driver never calls dependencyDocumentSymbol"
        );
    }
}

// ============================================================================
// CDO (env-gated) + the H-10 edit scenario
// ============================================================================

#[test]
fn cdo_workspace_has_zero_regressions_and_zero_unexplained() {
    let Some(ws) = cdo::cdo_ws_or_enforce() else {
        return;
    };
    let ledger = run_differential(&ws, true);
    ledger.assert_gates_clean("CDO");

    // Class-count pins (Step 4, binding once measured on a CDO_WS-capable
    // machine — see the task report for the measured table; this sandbox
    // has no CDO_WS, so these are NOT re-derived here, only gated).
    let _counts = ledger.class_counts();
}

/// Step 3's binding H-10 scenario: legacy `reindex_file` of ONE file loses
/// cross-file incoming edges TO it (H-10, `graph.rs`'s `remove_file` deletes
/// whole `incoming_calls` entries per defined qname); new's `apply_batch`
/// of the exact same no-op save keeps them. `NewBetter(H10Repair)`.
#[test]
fn cdo_h10_edit_scenario_legacy_loses_cross_file_incoming_new_keeps_them() {
    let Some(ws) = cdo::cdo_ws_or_enforce() else {
        return;
    };

    let legacy = LegacySide::build(&ws, true);
    let (base_new, parsed) =
        LspSnapshot::build_full_with_parsed(&ws).expect("build_full_with_parsed");

    // Pick a routine with real cross-file incoming edges on BOTH sides
    // BEFORE the edit, so the "loses it" observation below is meaningful.
    let cfg = DiagnosticConfig::default();
    let (sweep, _lenses) = run_sweep(&ws, &legacy, &base_new, &cfg);
    let Some(target) = sweep.entries.values().find(|e| {
        // an ORDINARY (non-event) legacy incoming entry (non-empty by
        // construction — `.any` requires at least one match).
        e.legacy_incoming.iter().any(|i| i.from.detail.is_none()) && e.legacy_prepare.is_some()
    }) else {
        panic!("CDO workspace sanity: expected at least one routine with a real cross-file caller");
    };
    let target_file = ws.join(&target.identity.file_rel);
    let target_uri = path_to_uri(&target_file);
    let pre_edit_legacy_incoming_count = target.legacy_incoming.len();

    // legacy: reindex_file of the TARGET's own file (a no-op content
    // rewrite — H-10 doesn't need a real change, just a reindex pass).
    legacy
        .indexer
        .write()
        .unwrap()
        .reindex_file(&target_file)
        .expect("legacy reindex_file");
    let post_reindex_item = legacy
        .prepare(
            &target_uri,
            target.legacy_prepare.as_ref().unwrap().range.start.line,
            target
                .legacy_prepare
                .as_ref()
                .unwrap()
                .range
                .start
                .character,
        )
        .and_then(|mut v| v.pop());
    let post_reindex_incoming = post_reindex_item
        .as_ref()
        .map(|item| legacy.incoming(item))
        .unwrap_or_default();

    // new: apply_batch of the exact same no-op save.
    let mut updater = Updater::new(ws.clone(), parsed);
    let batch = vec![ChangeEvent::FileSaved(target_file.clone())];
    let (new_snap_after, _rung) = updater
        .apply_batch(&base_new, &batch)
        .expect("apply_batch on the same no-op save");

    let target_vp = new_snap_after
        .decls_by_file
        .keys()
        .find(|k| k.to_lowercase() == target.identity.file_rel)
        .expect("target file still present after the no-op save");
    let target_decl = new_snap_after.decls_by_file[target_vp]
        .iter()
        .find(|d| d.name.eq_ignore_ascii_case(&target.identity.routine_lc))
        .expect("target decl still present after the no-op save");
    let post_edit_new_incoming = new_snap_after
        .incoming
        .get(&target_decl.id)
        .map(Vec::len)
        .unwrap_or(0);

    assert!(
        post_reindex_incoming.len() < pre_edit_legacy_incoming_count,
        "H-10 sanity: legacy must actually LOSE incoming edges after reindex_file (pre={pre_edit_legacy_incoming_count}, post={})",
        post_reindex_incoming.len()
    );
    assert_eq!(
        post_edit_new_incoming,
        target.new_incoming.len().max(post_edit_new_incoming),
        "new engine must KEEP its incoming edges across the same no-op save"
    );

    let mut ledger = Ledger::default();
    ledger.push(
        "incoming",
        &target.identity.key(),
        Class::NewBetter(NewBetterClass::H10Repair),
        format!(
            "legacy reindex_file: incoming {pre_edit_legacy_incoming_count} -> {} (LOST); new apply_batch: incoming stayed {post_edit_new_incoming}",
            post_reindex_incoming.len()
        ),
    );
    assert_eq!(
        ledger.class_counts().get("NewBetter::H10Repair").copied(),
        Some(1)
    );
}
