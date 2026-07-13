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
//! ## Why `UnqualifiedCallResolved`'s 36,971 CDO findings are safe to
//! blanket-classify (the license's actual load-bearing argument)
//!
//! `UnqualifiedCallResolved` is, by a wide margin, the single largest class
//! this harness produces (36,971 of 55,216 total CDO findings — see the
//! task report's capstone table). A blanket class this large is only a
//! safe thing to grant WITHOUT per-finding adjudication if a genuinely
//! WRONG new-side resolution for one of these calls is STRUCTURALLY
//! GUARANTEED to surface elsewhere as a `Regression`, not silently
//! disappear into this class too. It is, and the reviewer's own audit
//! found this report never actually stated why — so, verified directly
//! against the source (not asserted):
//!
//! Legacy's OUTGOING and INCOMING axes are **not the same code path**, and
//! they disagree in laziness for exactly the unqualified-call shape this
//! class covers:
//! - **OUTGOING** (`handlers.rs`'s `outgoing_calls`): for an unqualified
//!   call (`call.callee_object.is_none()`), the `else` arm at the bottom of
//!   its match unconditionally renders `detail: Some("(local)"), data:
//!   None` — it NEVER calls `graph.get_definition()` for this shape at all,
//!   regardless of whether a real local definition actually exists. This is
//!   the `is_legacy_placeholder`/`UnqualifiedCallResolved` shape this
//!   harness already classifies.
//! - **INCOMING** (`graph.rs`'s `add_call_site`, called at INDEX time, not
//!   query time): every call site's callee is resolved EAGERLY via
//!   `resolve_call` — and `resolve_call`'s own unqualified branch (lines
//!   ~648-684) ALWAYS returns `Some(QualifiedName{object: caller_qname.object,
//!   procedure: call.callee_method})`, syntactically, whether or not a real
//!   `Definition` exists for that pair. `add_call_site` then unconditionally
//!   files this call site's index into `self.incoming_calls.entry(callee_qname)`
//!   (line ~584-589) — so if a real `Definition` DOES exist matching that
//!   qname, `get_incoming_calls`/the `incoming_calls` handler correctly,
//!   independently, returns this call site as a genuine incoming caller for
//!   that routine, REGARDLESS of what the (lazy, always-placeholder)
//!   OUTGOING handler renders for the SAME call site.
//!
//! **The consequence:** if new's resolver ever got one of these 36,971
//! calls WRONG — dropped it, or misdirected it to the wrong target — that
//! wrongness is NOT explainable by `UnqualifiedCallResolved` at all; it
//! surfaces as a genuine `Regression` on the INCOMING axis of whichever
//! routine legacy's index-time `resolve_call` already, eagerly, correctly
//! attributed it to (a completely independent data path from the lazy
//! OUTGOING placeholder). This harness's `assert_gates_clean` — REGRESSION
//! must be 0 — is therefore not merely trusting the blanket
//! `UnqualifiedCallResolved` grant; it is actively CROSS-CHECKED, on every
//! single one of those 36,971 calls, by legacy's own independently-computed
//! incoming index. A silent resolution bug cannot hide behind this class's
//! size.
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
use al_call_hierarchy::program::resolve::edge::EdgeKind;
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
    /// A SECOND additional class, discovered during T14 implementation and
    /// GENERALIZED after the CDO fix-wave (originally named
    /// `OverloadIdentityCollapsed`, scoped to same-file arg-count overloads
    /// only — the CDO run surfaced the SAME root cause firing across
    /// entirely different objects/files too, e.g. `PAGE 6175343 "CDO
    /// E-Mail"` and `CODEUNIT 6175280 "CDO E-Mail"` sharing routine names,
    /// so the class and its detection are now workspace-GLOBAL, not
    /// per-file): legacy's `object_types`/`definitions`/`incoming_calls`/
    /// `outgoing_calls` are ALL keyed purely by `(object NAME text,
    /// procedure NAME text)` — no object KIND, no enclosing-member, no
    /// signature at all. ANY two declarations sharing that bare
    /// `(object_name, routine_name)` pair — a same-object arg-count
    /// overload, two different fields' same-named triggers on ONE object,
    /// or two ENTIRELY DIFFERENT OBJECTS (even different KINDS, e.g. a page
    /// and a codeunit) that merely happen to share a display name and a
    /// routine name — collide: `self.definitions.insert(qname, def)`
    /// (`src/graph.rs`'s `add_definition`) silently overwrites, and
    /// `add_call_site`'s incoming/outgoing buckets for that key MERGE every
    /// colliding declaration's call sites together. Legacy can even
    /// misreport a query against ONE of the colliding declarations with
    /// data that actually belongs to the OTHER ONE (e.g. `prepare()` on the
    /// codeunit's own routine returning the page's position) — a
    /// pre-existing legacy limitation, unrelated to any T3 engine change.
    /// New's `RoutineNodeId` (app-qualified object identity incl. KIND +
    /// `name_lc` + `enclosing_member_lc` + `params_count` + `sig_fp`) keeps
    /// every one of these distinct. Mechanical predicate: for the legacy
    /// identity key `(object_name_lc, routine_name_lc)`, the new side has
    /// MORE THAN ONE distinct `DeclEntry` (differing in kind, enclosing
    /// member, file, or signature — checked GLOBALLY, not scoped to one
    /// file) AND legacy's reported answer for one of them (position for
    /// `prepare`; attributed caller/count for `incoming`/`outgoing`/
    /// `codeLens`) actually matches a SIBLING declaration's own new-side
    /// truth instead of its own.
    LegacyIdentityCollapse,
    /// CDO layer-2 fix-wave: a database record operation with a
    /// statically-`true` run-trigger argument (e.g. `Rec.Insert(true)`)
    /// implicitly fires the target table's own trigger (`OnInsert`/
    /// `OnModify`/`OnDelete`/`OnRename`/field-`OnValidate`) — the new
    /// resolver models this as a REAL `EdgeKind::ImplicitTrigger` edge
    /// (`src/program/resolve/applicability.rs`'s `RecordOpCtx`/
    /// `RunTrigger::True`), so `incoming(OnInsert)` correctly shows the
    /// call site as a caller. Legacy's parser has no concept of
    /// implicit-trigger semantics at all: `Rec.Insert(true)` is just an
    /// ordinary (unresolvable, since `Insert` is a builtin record method,
    /// never a user `Definition`) call site to legacy — it is NEVER
    /// attributed as an incoming caller of `OnInsert`, which legacy cannot
    /// even connect to the record operation in the first place. Mechanical
    /// predicate: the new-only incoming/codeLens-ref-count site is backed
    /// by an edge whose `EdgeKind == ImplicitTrigger` — checked directly
    /// against `LspSnapshot::incoming`/`edges_by_file`, not inferred from
    /// the LSP response shape (which carries no edge-kind marker of its
    /// own for this case, unlike EventFlow's explicit `[EventPublisher]`
    /// tag).
    ImplicitTriggerEdge,
    /// CDO layer-2b fix-wave: a `Page`/`PageExtension`/`Report`/
    /// `ReportExtension` trigger (an action's `OnAction`, a page-level
    /// `OnAfterGetCurrRecord`, etc.) calls a procedure declared on the
    /// object's BOUND `SourceTable` — CROSS-OBJECT — using EITHER a bare
    /// (unqualified) call OR an explicit `Rec.`/`xRec.`-qualified one,
    /// resolved through the caller's IMPLICIT SourceTable record binding.
    /// Legacy's bare/qualified-call resolution (`src/graph.rs`'s
    /// `resolve_call`) is structurally same-object-only for a bare call
    /// (`QualifiedName{object: caller_qname.object, ..}`, unconditionally —
    /// it never considers an implicit SourceTable redirection) and, for a
    /// `Rec.`-qualified call, `Rec`/`xRec` are never real user-declared
    /// local variables `lookup_variable_type` could resolve for a page/
    /// report scope (they are a LANGUAGE-level implicit binding, not
    /// something `parser.rs`'s var-parsing logic ever sees) — so this call
    /// is structurally invisible to legacy's incoming-call index no matter
    /// which syntax the source uses. New resolves it via the implicit-Rec/
    /// SourceTable machinery. Confirmed by the controller against real CDO
    /// source (`Page 6175306 "CDO E-Mail Template Lines"`, SourceTable
    /// `"CDO E-Mail Templ. Line Report"`, an action's `OnAction` bareword
    /// call into the source table). Mechanical predicate: the new-only
    /// incoming/codeLens site is a Call-kind (not `ImplicitTrigger`) edge
    /// whose caller's OBJECT differs from the callee's, the caller's object
    /// KIND is `Page`/`PageExtension`/`Report`/`ReportExtension`, and the
    /// call-site TEXT (read from the caller's own source — `LspSnapshot`
    /// carries no dedicated marker for this, unlike `ImplicitTriggerEdge`'s
    /// `EdgeKind`) is bare or `Rec.`/`xRec.`-qualified.
    ImplicitRecResolved,
    /// CDO layer-3 fix-wave: an ORDINARY receiver-qualified call
    /// (`Ident.Method(...)` or `"Quoted Ident".Method(...)`) whose receiver
    /// is a `var` PARAMETER, a `Rec`/`xRec` implicit binding, or some other
    /// local/temp shape legacy's variable tracking misses — and NOT the
    /// object's OWN name (an object-name-qualified call legacy CAN resolve
    /// via `object_types`, so it never lands here at all — those rows are
    /// already `Match`). Confirmed by the controller against real CDO
    /// source: `Codeunit 6175274 "CDO Continia Online PDF Mgt"`'s
    /// `procedure MergePdf(var DOFile: Record "CDO File"; ...)` calling
    /// `DOFile.IsPdf()` — `DOFile` is a `var` PARAMETER. Root cause
    /// (confirmed by reading `src/parser.rs`/`src/indexer.rs`):
    /// `variable_bindings` is populated EXCLUSIVELY from a routine's
    /// `var`-section LOCALS (`push_variables_ir(&mut result, &r.locals,
    /// ..)`, `src/parser.rs:293`) — the routine's PARAMETER list
    /// (`r.params`, a structurally separate IR field, used elsewhere only
    /// to compute `parameter_count`) never flows into it at all, so
    /// `lookup_variable_type` can never type a parameter receiver. A LOCAL
    /// `var`-section variable receiver, by contrast, DOES get bound
    /// correctly and resolves as a genuine `Match` — verified empirically,
    /// not assumed (see `VariableReceiverCaller.al`'s `UseLocalVar`
    /// procedure, kept in the SAME fixture as its parameter-receiver
    /// sibling for direct contrast).
    ///
    /// **Generalized in CDO layer 4**: the original predicate additionally
    /// required `caller object != callee object`, which 3 CONCRETE CDO
    /// counterexamples proved wrong — a codeunit's `var` parameter of its
    /// OWN type calling itself, a table's `var` parameter of its OWN record
    /// type calling itself, and a table's own implicit `Rec.`-qualified
    /// self-call (`Table 6175330`'s `GetPlainText`/`Rec.GetHTML()` — a
    /// SAME-object `Rec.`-qualified call that `ImplicitRecResolved` doesn't
    /// claim, since that class is scoped to Page/PageExtension/Report/
    /// ReportExtension callers only). The mechanism is the RECEIVER TOKEN
    /// legacy never modeled, not object identity, so `Rec`/`xRec` are no
    /// longer excluded here either (a cross-object Page/Report bare-or-
    /// `Rec.`-qualified call is still claimed by `ImplicitRecResolved`
    /// FIRST in the classifier chain, so there is no double-classification).
    /// Also independently confirmed to already cover CDO's
    /// `.dependencies/cdo/.../cdoqueuemanagement.codeunit.al::cdo queue
    /// management.onrun: new caller=sendqueue` shape (a `var` Codeunit-typed
    /// variable's `.Run()` dispatching to `OnRun`) WITHOUT any dedicated
    /// `EdgeKind::Run` handling: `resolve_member`'s `Run`-on-Codeunit
    /// special case (`src/program/resolve/resolver.rs`) produces an
    /// ordinary `EdgeKind::Call`/Member-shape edge, so this class's
    /// existing Call-kind receiver check already catches it (see
    /// `RunDispatchTarget.al`/`RunDispatchCaller.al`, which matched
    /// cleanly on the FIRST run, before this generalization was even
    /// written — verified, not assumed). Mechanical predicate: a Call-kind
    /// new-only incoming/codeLens/diagnostics site that is receiver-
    /// qualified (any object kind, any caller/callee relationship) whose
    /// receiver token is NOT (case-insensitive, quote-normalized) the
    /// callee's own object display name.
    VariableReceiverResolved,
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

/// A `CanonicalSpan` (engine-native byte-column line/col) into the SAME
/// `NormRange` space `nr` produces from an LSP `Range` under
/// `PositionEncoding::Utf8` (a pass-through/clamp, per `LineTable::col_out`'s
/// own doc) — so a raw engine edge site and an LSP response range compare
/// directly, with no `LineTable` needed (the span is always in-bounds for
/// its own file).
fn canonical_span_to_norm_range(
    span: &al_call_hierarchy::program::resolve::edge::CanonicalSpan,
) -> NormRange {
    (span.start.line, span.start.col, span.end.line, span.end.col)
}

/// A call site's own receiver, extracted from its `site.span`'s text.
#[derive(Debug, Clone, PartialEq, Eq)]
enum CallSiteReceiver {
    /// No receiver at all: `Method(...)`.
    Bare,
    /// An explicit receiver (quote-stripped, case-preserved): `Receiver.Method(...)`
    /// or `"Quoted Receiver".Method(...)`.
    Qualified(String),
}

/// Parse the call site at `span` (within `text`, the CALLER's own source)
/// into its own receiver shape.
///
/// **`site.span`'s shape differs between a bare and a member call — this is
/// NOT symmetric, confirmed empirically from TWO independent measurements,
/// not assumed from one:** for a BARE call the span starts exactly at the
/// callee identifier's own first character (`tests/fixtures/lsp-diff-nested/
/// ImplicitRecPage.al`'s bare calls); for a MEMBER call the span covers the
/// WHOLE reference expression INCLUDING the receiver (the CDO layer-3
/// brief's own measurement: `DOFile.IsPdf()`'s span was `(130,19,130,33)` —
/// 14 columns, matching `"DOFile.IsPdf()"` in full, not just `"IsPdf()"`).
/// So this function parses the site's OWN text (`[span.start, span.end)`)
/// directly rather than inferring shape from what precedes `span.start` —
/// the one design that's correct for BOTH shapes without needing to know
/// which one a given site is in advance. Quote-aware: a `.` inside a
/// `"Quoted Identifier"` is never treated as the receiver/method separator.
fn call_site_receiver(
    text: &str,
    span: &al_call_hierarchy::program::resolve::edge::CanonicalSpan,
) -> Option<CallSiteReceiver> {
    let line = text.split('\n').nth(span.start.line as usize)?;
    let line = line.strip_suffix('\r').unwrap_or(line);
    let start = span.start.col as usize;
    let end = (span.end.col as usize).min(line.len());
    if start > end || start > line.len() {
        return None;
    }
    let site_text = &line[start..end];

    let mut in_quotes = false;
    let mut dot_idx = None;
    for (i, c) in site_text.char_indices() {
        match c {
            '"' => in_quotes = !in_quotes,
            '.' if !in_quotes => {
                dot_idx = Some(i);
                break;
            }
            '(' if !in_quotes => break, // reached the arg list — no receiver
            _ => {}
        }
    }
    match dot_idx {
        None => Some(CallSiteReceiver::Bare),
        Some(i) => {
            let receiver = site_text[..i].trim();
            let receiver = receiver
                .strip_prefix('"')
                .and_then(|s| s.strip_suffix('"'))
                .unwrap_or(receiver);
            Some(CallSiteReceiver::Qualified(receiver.to_string()))
        }
    }
}

/// `true` iff the call site is a BARE call, or an explicit `Rec.`/`xRec.`-
/// qualified one (case-insensitive) — `ImplicitRecResolved`'s call-site-shape
/// half of its predicate.
fn is_bare_or_rec_qualified_call(
    text: &str,
    span: &al_call_hierarchy::program::resolve::edge::CanonicalSpan,
) -> bool {
    match call_site_receiver(text, span) {
        Some(CallSiteReceiver::Bare) => true,
        Some(CallSiteReceiver::Qualified(r)) => {
            r.eq_ignore_ascii_case("rec") || r.eq_ignore_ascii_case("xrec")
        }
        None => false,
    }
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
    /// LOWERCASED — the cross-engine MATCHING key only (`.key()` uses this).
    /// Never join this onto `root` to build a path for querying legacy (see
    /// `relativize_case_preserving`'s doc, review fix-wave HIGH-1) — use
    /// `file_rel_case` for that.
    file_rel: String,
    /// CASE-PRESERVING form of the same path, carried alongside `file_rel`
    /// purely so `run_sweep`'s per-identity query loop can build a path that
    /// round-trips through legacy's OWN case-sensitive `path_cache`
    /// correctly on every platform, not just Windows (see
    /// `relativize_case_preserving`'s doc). Never part of the identity
    /// KEY — two identities differing only in this field's case are the
    /// same identity (`.key()` deliberately excludes it).
    file_rel_case: String,
    object_lc: String,
    routine_lc: String,
}

impl RoutineIdentity {
    fn key(&self) -> String {
        format!("{}::{}.{}", self.file_rel, self.object_lc, self.routine_lc)
    }
}

/// The `strip_prefix`+slash-normalize core of `relativize`/
/// `relativize_case_preserving`, WITHOUT the final lowercase step — factored
/// out so the two callers can never drift apart on anything except that one
/// step (see the invariant test `relativize_is_lowercased_relativize_case_preserving`).
fn relativize_raw(root: &Path, file: &Path) -> String {
    let norm_root = al_call_hierarchy::protocol::normalize_path(root);
    let norm_file = al_call_hierarchy::protocol::normalize_path(file);
    norm_file
        .strip_prefix(&norm_root)
        .unwrap_or(&norm_file)
        .to_string_lossy()
        .replace('\\', "/")
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
///
/// **This is the cross-engine MATCHING key only — never use it to build a
/// path for QUERYING legacy back.** See `relativize_case_preserving`'s doc
/// (review fix-wave HIGH-1) for why: on a case-sensitive filesystem (Linux,
/// CI's `ubuntu-latest`), lowercasing throws away information legacy's own
/// `CallGraph::path_cache` never lowercased at index time, so a lowercased
/// query path silently misses every entry.
fn relativize(root: &Path, file: &Path) -> String {
    relativize_raw(root, file).to_lowercase()
}

/// Relative, CASE-PRESERVING virtual path — the form to use when re-querying
/// legacy's OWN handlers for an identity discovered via `relativize`'s
/// lowercased key (review fix-wave HIGH-1, arc-critical: the always-on CI
/// arm did not actually run on CI before this fix).
///
/// **The bug this closes:** `graph.rs`'s `get_shared_path`/`path_cache`
/// (backing `get_definitions_in_file` and everything else legacy queries by
/// path) store `Definition.file` as `protocol::normalize_path(file)` — the
/// NORMALIZED path itself, not the raw one — and every lookup does an EXACT
/// `HashMap::get(&normalize_path(query_path))`, never a case-insensitive
/// fallback (unlike new's `resolve_virtual_path`, `src/lsp/handlers.rs`,
/// which explicitly falls back to a case-insensitive scan for exactly this
/// reason). `normalize_path` itself is a no-op on Linux/macOS (only Windows
/// lowercases the whole path), so on a case-sensitive filesystem,
/// `Definition.file` is indexed under the file's REAL case (e.g.
/// `Alpha.al`, since every fixture file in this harness is capitalized).
/// Querying legacy with `relativize`'s LOWERCASED path (`alpha.al`) then
/// normalizes to the SAME lowercased string on Linux (normalize_path is
/// identity there) — an exact-match miss against a path_cache keyed by
/// `Alpha.al` — so `get_definitions_in_file` silently returns `&[]` for
/// EVERY identity, every legacy `prepare`/`incoming`/`outgoing` call in
/// `run_sweep`'s per-identity loop comes back empty, and every routine in
/// the whole differential run becomes a `NewUnexplained` finding, panicking
/// the gate on CI (`cargo test --workspace` on `ubuntu-latest`). On Windows
/// this bug is invisible: `normalize_path` ALREADY lowercases the whole
/// path at INDEX time (`get_shared_path`), so `Definition.file` itself has
/// no case information left to lose — `relativize`/`relativize_case_preserving`
/// coincide on Windows, by construction, which is exactly why this slipped
/// past every fixture run in this dev environment.
///
/// New's own `prepare`/`incoming`/`outgoing` handlers are UNAFFECTED by
/// which form is used to build the query URI either way — `resolve_virtual_path`
/// tries an exact match first, then a case-insensitive fallback scan over
/// `snap.parsed`'s keys — so this fix is purely additive there (the exact
/// match just becomes reachable instead of always falling through to the
/// scan).
fn relativize_case_preserving(root: &Path, file: &Path) -> String {
    relativize_raw(root, file)
}

/// Platform-independent structural invariant (review fix-wave HIGH-1): for
/// ANY `(root, file)` pair, `relativize`'s output must be EXACTLY
/// `relativize_case_preserving`'s output lowercased — never anything else.
/// This is what guarantees `file_rel`/`file_rel_case` can only ever differ
/// in CASE, never in content, so every place that matches on `file_rel`
/// (the identity key) stays correct regardless of which of the two fields
/// `run_sweep`'s query loop happens to build a path from.
///
/// This test CANNOT observe the actual Linux-only bug this fix closes
/// (`protocol::normalize_path` already destroys case at the `normalize_path`
/// step on Windows, before either function gets a chance to differ — see
/// `relativize_case_preserving`'s own doc) — that half of the fix rests on
/// the code-reading argument in this doc plus `relativize_case_preserving`'s,
/// not on a locally-observable red/green cycle. What IS fully verifiable on
/// any platform, including this dev box, is the MECHANICAL relationship
/// between the two functions, which this test pins down.
#[test]
fn relativize_is_lowercased_relativize_case_preserving() {
    let cases: &[(&str, &str)] = &[
        ("/workspace", "/workspace/Alpha.al"),
        ("/workspace", "/workspace/sub/Gamma.al"),
        ("/workspace", "/Outside/Other.al"),
        ("C:/Workspace", "C:/Workspace/Beta.al"),
    ];
    for (root, file) in cases {
        let root = Path::new(root);
        let file = Path::new(file);
        assert_eq!(
            relativize_case_preserving(root, file).to_lowercase(),
            relativize(root, file),
            "relativize/relativize_case_preserving diverged on more than case for root={root:?} file={file:?}"
        );
    }
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

/// Strips a same-file disambiguator suffix (`"{name}#dup{i}"`, added in
/// `run_sweep`'s new-identities loop whenever >1 `DeclEntry` shares a
/// `(file_rel, object_lc, routine_lc)` triple — same-object arg-count
/// overloads, or two different fields' same-named triggers on ONE object)
/// back to the bare routine name, for GLOBAL (cross-file)
/// `LegacyIdentityCollapse` grouping — see `Sweep::legacy_collision_group`.
fn strip_dup_suffix(routine_lc: &str) -> &str {
    routine_lc.split_once("#dup").map_or(routine_lc, |(b, _)| b)
}

/// Resolve `identity` back to its real `DeclEntry` within `decls` (already
/// sorted by declaration order — `decls_by_file`'s own doc). Mirrors the
/// same-file grouping this identity was BUILT with (see the new-identities
/// loop in `run_sweep`): a suffixed `routine_lc` (`"{name}#dup{i}"`) picks
/// the `i`-th declaration in its `(object_lc, base_name)` group; an
/// un-suffixed `routine_lc` picks the LAST one.
fn find_new_decl<'a>(
    decls: &'a [al_call_hierarchy::lsp::snapshot::DeclEntry],
    graph: &ProgramGraph,
    identity: &RoutineIdentity,
) -> Option<&'a al_call_hierarchy::lsp::snapshot::DeclEntry> {
    let (base_name, target_idx) = match identity.routine_lc.split_once("#dup") {
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
    legacy_prepare: Option<CallHierarchyItem>,
    new_prepare: Option<CallHierarchyItem>,
    legacy_incoming: Vec<CallHierarchyIncomingCall>,
    new_incoming: Vec<CallHierarchyIncomingCall>,
    legacy_outgoing: Vec<CallHierarchyOutgoingCall>,
    new_outgoing: Vec<CallHierarchyOutgoingCall>,
    /// Incoming call-site ranges (byte-native, same `NormRange` space as
    /// `from_ranges`) backed by an `EdgeKind::ImplicitTrigger` edge — looked
    /// up directly against `LspSnapshot::incoming` (see `run_sweep`), since
    /// the LSP wire shape carries no marker distinguishing this from an
    /// ordinary call (unlike EventFlow's explicit `[EventPublisher]` tag).
    /// `ImplicitTriggerEdge`'s mechanical predicate.
    new_incoming_implicit_trigger_sites: BTreeSet<NormRange>,
    /// Incoming call-site ranges resolved CROSS-OBJECT from a `Page`/
    /// `PageExtension`/`Report`/`ReportExtension` caller via its implicit
    /// `Rec`/`xRec` SourceTable binding (a bare call, or an explicit
    /// `Rec.`/`xRec.`-qualified one) — looked up directly against
    /// `LspSnapshot::incoming`/the caller's own source text (see
    /// `run_sweep`), since legacy's bare-call resolution is same-object-only
    /// and structurally cannot model this. `ImplicitRecResolved`'s
    /// mechanical predicate.
    new_incoming_implicit_rec_sites: BTreeSet<NormRange>,
    /// Incoming call-site ranges resolved via a receiver-qualified call
    /// (same-object or cross-object — layer 4 dropped the cross-object
    /// requirement) whose receiver is a `var` parameter, `Rec`/`xRec`, or
    /// another local/temp shape legacy's `variable_bindings` misses — not
    /// the callee's own object name (an object-name-qualified call legacy
    /// CAN resolve). A cross-object Page/Report bare-or-`Rec.`-qualified
    /// site is claimed by `new_incoming_implicit_rec_sites` FIRST (see
    /// `run_sweep`'s `else`-chain ordering), so there is no overlap with
    /// that set. Looked up directly against `LspSnapshot::incoming`/the
    /// caller's own source text (see `run_sweep`). `VariableReceiverResolved`'s
    /// mechanical predicate.
    new_incoming_variable_receiver_sites: BTreeSet<NormRange>,
}

struct Sweep {
    entries: BTreeMap<String, RoutineEntry>,
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

    /// Every entry sharing the legacy identity key `(object_lc,
    /// routine_lc_base)` — GLOBALLY, across every file, not scoped to one
    /// object's own file (see `NewBetterClass::LegacyIdentityCollapse`'s
    /// doc: legacy's `object_types`/`definitions` maps are keyed by bare
    /// NAME TEXT only, no file/kind/member component at all). A same-file
    /// `#dup{i}`-suffixed identity is un-suffixed via `strip_dup_suffix`
    /// before comparing, so it's grouped with its siblings correctly.
    fn legacy_collision_group<'a>(
        &'a self,
        object_lc: &str,
        routine_lc_base: &str,
    ) -> Vec<&'a RoutineEntry> {
        self.entries
            .values()
            .filter(|e| {
                e.identity.object_lc == object_lc
                    && strip_dup_suffix(&e.identity.routine_lc) == routine_lc_base
            })
            .collect()
    }

    /// `true` iff `(object_lc, routine_lc_base)` names more than one REAL,
    /// distinct declaration anywhere in the workspace — i.e. legacy's own
    /// `(object_name, routine_name)`-keyed identity is genuinely collided
    /// for this pair, regardless of which file(s) the colliding
    /// declarations live in.
    fn is_legacy_identity_collision(&self, object_lc: &str, routine_lc_base: &str) -> bool {
        self.legacy_collision_group(object_lc, routine_lc_base)
            .len()
            > 1
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

    // ---- legacy identities, via iter_definitions (already pub) ----
    {
        let idx = legacy.indexer.read().unwrap();
        let graph = idx.graph();
        for (_qname, def) in graph.iter_definitions() {
            let object_lc = graph.resolve(def.object_name).unwrap_or("").to_lowercase();
            let routine_lc = graph.resolve(def.name).unwrap_or("").to_lowercase();
            let file_rel = relativize(root, &def.file);
            let file_rel_case = relativize_case_preserving(root, &def.file);
            let identity = RoutineIdentity {
                file_rel,
                file_rel_case,
                object_lc,
                routine_lc,
            };
            let key = identity.key();
            entries.entry(key).or_insert_with(|| RoutineEntry {
                identity,
                legacy_prepare: None,
                new_prepare: None,
                legacy_incoming: Vec::new(),
                new_incoming: Vec::new(),
                legacy_outgoing: Vec::new(),
                new_outgoing: Vec::new(),
                new_incoming_implicit_trigger_sites: BTreeSet::new(),
                new_incoming_implicit_rec_sites: BTreeSet::new(),
                new_incoming_variable_receiver_sites: BTreeSet::new(),
            });
        }
    }

    // ---- new identities, via decls_by_file ----
    //
    // Grouped by `(object_lc, routine_lc)` PER FILE first, purely to give
    // same-file duplicates (a same-object arg-count overload, or two
    // different fields' same-named triggers) distinct MAP KEYS — `decls` is
    // already sorted by `origin.byte.start` (declaration order) per
    // `LspSnapshot::decls_by_file`'s own doc, so every group member EXCEPT
    // the last gets a disambiguating `#dup{i}` suffix. Unlike the ORIGINAL
    // (pre-CDO-fix-wave) design, this suffix is ONLY a map-key
    // disambiguator now — every identity, suffixed or not, is queried
    // against legacy normally below (see `NewBetterClass::
    // LegacyIdentityCollapse`'s doc for why that's safe: legacy's
    // `get_definitions_in_file` returns the SAME globally-collided
    // `Definition` regardless of which file/position queries it, so no
    // identity needs to be skipped — the classifier's GLOBAL sibling
    // cross-check handles the collision instead of a construction-time
    // skip).
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
            // `virtual_path` itself IS the case-preserving form
            // (`snapshot::provider::walk_al_source`'s own doc, cited above) —
            // no separate derivation needed, unlike the legacy-identities
            // loop above.
            let file_rel_case = virtual_path.clone();
            let last_idx = group.len() - 1;
            for (i, _decl) in group.iter().enumerate() {
                let identity = if i == last_idx {
                    RoutineIdentity {
                        file_rel: file_rel.clone(),
                        file_rel_case: file_rel_case.clone(),
                        object_lc: object_lc.clone(),
                        routine_lc: routine_lc.clone(),
                    }
                } else {
                    RoutineIdentity {
                        file_rel: file_rel.clone(),
                        file_rel_case: file_rel_case.clone(),
                        object_lc: object_lc.clone(),
                        routine_lc: format!("{routine_lc}#dup{i}"),
                    }
                };
                let key = identity.key();
                entries.entry(key).or_insert_with(|| RoutineEntry {
                    identity,
                    legacy_prepare: None,
                    new_prepare: None,
                    legacy_incoming: Vec::new(),
                    new_incoming: Vec::new(),
                    legacy_outgoing: Vec::new(),
                    new_outgoing: Vec::new(),
                    new_incoming_implicit_trigger_sites: BTreeSet::new(),
                    new_incoming_implicit_rec_sites: BTreeSet::new(),
                    new_incoming_variable_receiver_sites: BTreeSet::new(),
                });
            }
        }
    }

    // ---- drive prepare/incoming/outgoing per identity ----
    for entry in entries.values_mut() {
        // CASE-PRESERVING path (review fix-wave HIGH-1) — legacy's own
        // `path_cache` is keyed on the real, case-sensitive path on any
        // platform where `normalize_path` isn't itself lowercasing (Linux/
        // CI); `file_rel` (the lowercased cross-engine matching key) would
        // silently miss every lookup there. See `relativize_case_preserving`'s
        // doc for the full mechanism.
        let abs_path = root.join(&entry.identity.file_rel_case);
        let uri = path_to_uri(&abs_path);

        // Legacy: find this identity's own position via a fresh
        // get_definitions_in_file scan (cheap; fixtures are small) so we
        // don't need to retain Definition's own range separately above.
        // Queried for EVERY identity, including same-file `#dup{i}`
        // siblings — legacy's `get_definitions_in_file` returns the same
        // (possibly globally-collided) `Definition` regardless (see the
        // comment on the loop above), which the classifier itself
        // recognizes as `LegacyIdentityCollapse` rather than a REGRESSION.
        let legacy_pos = {
            let idx = legacy.indexer.read().unwrap();
            let graph = idx.graph();
            let base_routine_lc = strip_dup_suffix(&entry.identity.routine_lc);
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
                            .eq_ignore_ascii_case(base_routine_lc)
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

                // ImplicitTriggerEdge's mechanical predicate: look up this
                // routine's REAL incoming edges directly (bypassing the LSP
                // wire shape entirely, which carries no edge-kind marker for
                // this case) and record every site backed by an
                // `EdgeKind::ImplicitTrigger` edge.
                //
                // ImplicitRecResolved's mechanical predicate (CDO layer-2b):
                // an ORDINARY (Call-kind) incoming edge whose caller's OBJECT
                // differs from this routine's own object, where the caller
                // is a Page/PageExtension/Report/ReportExtension AND the
                // call-site text is bare or `Rec.`/`xRec.`-qualified — i.e.
                // resolved through the caller's IMPLICIT SourceTable binding,
                // never a genuine same-object call legacy's `resolve_call`
                // could ever produce.
                if let Some(refs) = new_snap.incoming.get(&data.node) {
                    for r in refs {
                        let ce = new_snap.edge(r);
                        if ce.edge.kind == EdgeKind::ImplicitTrigger {
                            entry
                                .new_incoming_implicit_trigger_sites
                                .insert(canonical_span_to_norm_range(&ce.edge.site.span));
                        } else if ce.edge.from.object != data.node.object
                            && matches!(
                                ce.edge.from.object.kind,
                                al_syntax::ir::ObjectKind::Page
                                    | al_syntax::ir::ObjectKind::PageExtension
                                    | al_syntax::ir::ObjectKind::Report
                                    | al_syntax::ir::ObjectKind::ReportExtension
                            )
                            && new_snap.parsed.get(&r.file).is_some_and(|caller_entry| {
                                is_bare_or_rec_qualified_call(
                                    &caller_entry.text,
                                    &ce.edge.site.span,
                                )
                            })
                        {
                            entry
                                .new_incoming_implicit_rec_sites
                                .insert(canonical_span_to_norm_range(&ce.edge.site.span));
                        } else {
                            // VariableReceiverResolved's mechanical predicate
                            // (CDO layer 3, GENERALIZED in layer 4): a
                            // receiver-qualified call (ANY object kind, ANY
                            // caller/callee object relationship — layer 3's
                            // `caller object != callee object` restriction
                            // was proven wrong by 3 CONCRETE same-object CDO
                            // counterexamples: a codeunit's `var` parameter
                            // of its OWN type calling itself
                            // (`Codeunit 6175324 "CDO XML Node"`'s `AddNode`/
                            // `NewXmlNode.SetXmlNode(...)`), a table's `var`
                            // parameter of its OWN record type
                            // (`Table 6175301 "CDO File"`'s `MergeWithPdf`/
                            // `PDFDocument.IsPdf()`), and a table's own
                            // implicit `Rec.`-qualified self-call
                            // (`Table 6175330`'s `GetPlainText`/
                            // `Rec.GetHTML()`) — the MECHANISM is the
                            // receiver TOKEN legacy's `variable_bindings`
                            // never modeled, not object identity, so `Rec`/
                            // `xRec` are no longer excluded here either:
                            // for a Page/PageExtension/Report/
                            // ReportExtension caller, a cross-object bare-or-
                            // `Rec.`-qualified call is ALREADY claimed by
                            // `ImplicitRecResolved` above (this `else` only
                            // runs when that arm didn't match), so a
                            // same-object `Rec.`-qualified call on a
                            // Table/Codeunit/other kind reaching HERE is
                            // never double-classified) whose receiver is NOT
                            // the callee's own object name (case-
                            // insensitive, quote-normalized — an
                            // object-name-qualified call legacy CAN resolve
                            // via `object_types`, so it's excluded here).
                            let callee_object_name =
                                object_name(&new_snap.graph, &data.node.object).unwrap_or("");
                            if let Some(caller_entry) = new_snap.parsed.get(&r.file)
                                && let Some(CallSiteReceiver::Qualified(receiver)) =
                                    call_site_receiver(&caller_entry.text, &ce.edge.site.span)
                                && !receiver.eq_ignore_ascii_case(callee_object_name)
                            {
                                entry
                                    .new_incoming_variable_receiver_sites
                                    .insert(canonical_span_to_norm_range(&ce.edge.site.span));
                            }
                        }
                    }
                }
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

    (Sweep { entries }, lenses)
}

// ============================================================================
// Classification: prepare
// ============================================================================

fn classify_prepare(ledger: &mut Ledger, sweep: &Sweep) {
    for entry in sweep.entries.values() {
        let routine = entry.identity.key();
        let base_name = strip_dup_suffix(&entry.identity.routine_lc);

        match (&entry.legacy_prepare, &entry.new_prepare) {
            (Some(l), Some(n)) => {
                if l.name.eq_ignore_ascii_case(&n.name) && nr(&l.range) == nr(&n.range) {
                    ledger.push("prepare", &routine, Class::Match, "range+name agree");
                } else if legacy_answer_matches_a_sibling(
                    sweep,
                    &entry.identity.object_lc,
                    base_name,
                    &entry.identity.key(),
                    |sib| {
                        sib.new_prepare.as_ref().is_some_and(|sp| {
                            l.name.eq_ignore_ascii_case(&sp.name) && nr(&l.range) == nr(&sp.range)
                        })
                    },
                ) {
                    ledger.push(
                        "prepare",
                        &routine,
                        Class::NewBetter(NewBetterClass::LegacyIdentityCollapse),
                        format!(
                            "legacy's answer (name={:?} range={:?}) actually matches a SIBLING declaration sharing legacy's (object,routine) identity key, not this one's own (new name={:?} range={:?})",
                            l.name, nr(&l.range), n.name, nr(&n.range)
                        ),
                    );
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

/// `true` iff `(object_lc, routine_lc_base)` is a genuine, GLOBAL legacy
/// identity collision (`Sweep::is_legacy_identity_collision`) AND at least
/// one OTHER member of that collision group (excluding `self_key`) matches
/// `predicate` — the shared shape every `LegacyIdentityCollapse` check
/// across prepare/incoming/outgoing/codeLens uses.
fn legacy_answer_matches_a_sibling(
    sweep: &Sweep,
    object_lc: &str,
    routine_lc_base: &str,
    self_key: &str,
    predicate: impl Fn(&RoutineEntry) -> bool,
) -> bool {
    sweep.is_legacy_identity_collision(object_lc, routine_lc_base)
        && sweep
            .legacy_collision_group(object_lc, routine_lc_base)
            .iter()
            .any(|sib| sib.identity.key() != self_key && predicate(sib))
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

/// `item.to.data.object` (lowercased) — present ONLY on legacy's arm 1
/// ("local definition found", `outgoing_calls`'s `data: {"object":...,
/// "procedure":...}`) — the object identity `LegacyIdentityCollapse`'s
/// GLOBAL collision check keys on.
fn legacy_local_object(item: &CallHierarchyOutgoingCall) -> Option<String> {
    item.to
        .data
        .as_ref()
        .and_then(|d| d.get("object"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_lowercase())
}

/// NEW's own resolved target object (lowercased), via `item.to.data`
/// deserialized as `ItemData` and looked up in `new_snap.graph` — the
/// `LegacyIdentityCollapse` predicate's MED-2 review fix: without this, a
/// same-named target legacy claims is collided would launder ANY new
/// answer, even one that resolves to a completely UNRELATED object never
/// part of that collision group at all (`Foo.Bar()` "explained" by a
/// collision when new actually, correctly, resolved to some unrelated
/// `Baz.Bar`).
fn new_target_object_lc(
    new_snap: &LspSnapshot,
    item: &CallHierarchyOutgoingCall,
) -> Option<String> {
    let data: ItemData = serde_json::from_value(item.to.data.clone()?).ok()?;
    object_name(&new_snap.graph, &data.node.object).map(|s| s.to_lowercase())
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
    // REVIEW FIX (LOW): `Iterator::all` on an EMPTY `from_ranges` is
    // vacuously `true`, which used to make an item with no from_ranges at
    // all silently count as "event-derived" regardless of `self_range` —
    // dropping it from `new_ordinary` (and thus from the whole outgoing
    // diff) with no finding at all. An item genuinely event-derived always
    // HAS a from_range (re-derived from the publisher's own name_origin,
    // per this function's own doc) — require at least one before trusting
    // the `all` check.
    self_prepare_range.is_some_and(|self_range| {
        !item.from_ranges.is_empty() && item.from_ranges.iter().all(|r| nr(r) == self_range)
    })
}

fn classify_outgoing(ledger: &mut Ledger, sweep: &Sweep, new_snap: &LspSnapshot) {
    for entry in sweep.entries.values() {
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
                    // REVIEW FIX (MED-3): the ORIGINAL check fired
                    // `OutgoingCardinality` on a raw COUNT mismatch across
                    // the WHOLE site with no content check, and (when
                    // counts happened to match) paired items up by
                    // POSITIONAL zip regardless of target identity —
                    // silently laundering a real divergence (legacy targets
                    // {Foo, Bar}, new targets {Foo, Baz} — same COUNT,
                    // totally different Bar-vs-Baz reality) as a harmless
                    // grouping artifact. Group each side's items by TARGET
                    // NAME (case-insensitive — the same identity
                    // `classify_outgoing_pair` itself keys on) and compare
                    // per-target SETS, not raw lengths: a name present on
                    // BOTH sides with EQUAL counts pairs up normally (still
                    // deep-adjudicated via `classify_outgoing_pair`); equal
                    // name but UNEQUAL counts is a genuine
                    // `OutgoingCardinality` — a grouping-count difference
                    // for that SPECIFIC target, not the whole site; a name
                    // present on only ONE side is never grouping noise —
                    // it's routed through the same legacy-only/new-only
                    // handling the empty-side match arms above use.
                    let mut l_by_name: BTreeMap<String, Vec<&CallHierarchyOutgoingCall>> =
                        BTreeMap::new();
                    for item in &l_items {
                        l_by_name
                            .entry(item.to.name.to_lowercase())
                            .or_default()
                            .push(*item);
                    }
                    let mut n_by_name: BTreeMap<String, Vec<&CallHierarchyOutgoingCall>> =
                        BTreeMap::new();
                    for item in &n_items {
                        n_by_name
                            .entry(item.to.name.to_lowercase())
                            .or_default()
                            .push(*item);
                    }
                    let mut all_names: BTreeSet<String> = l_by_name.keys().cloned().collect();
                    all_names.extend(n_by_name.keys().cloned());
                    for name in all_names {
                        let l_group = l_by_name.get(&name).cloned().unwrap_or_default();
                        let n_group = n_by_name.get(&name).cloned().unwrap_or_default();
                        match (l_group.len(), n_group.len()) {
                            (0, _) => {
                                for n in &n_group {
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
                            (_, 0) => {
                                for l in &l_group {
                                    classify_outgoing_legacy_only(ledger, &routine, l);
                                }
                            }
                            (lc, nc) if lc == nc => {
                                for (l, n) in l_group.iter().zip(n_group.iter()) {
                                    classify_outgoing_pair(ledger, sweep, new_snap, &routine, l, n);
                                }
                            }
                            (lc, nc) => {
                                ledger.push(
                                    "outgoing",
                                    &routine,
                                    Class::NewBetter(NewBetterClass::OutgoingCardinality),
                                    format!(
                                        "site {site:?}, target={name:?}: legacy {lc} item(s) vs new {nc} item(s)"
                                    ),
                                );
                            }
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
    // LegacyIdentityCollapse: legacy's single collapsed (object, name) slot
    // can point at the WRONG declaration's position entirely (it has no
    // arg-type dispatch, no object-kind discriminator, and no
    // enclosing-member discriminator at all — `resolve_call` just matches
    // qualified/unqualified object+method NAME TEXT) — so a SAME-named
    // target with a DIFFERENT range, where legacy's own claimed object
    // identity (`data.object`, arm 1's "local definition found") names a
    // GLOBALLY collided `(object, routine)` pair, is this class, not an
    // unexplained divergence.
    //
    // REVIEW FIX (MED-2): the ORIGINAL check stopped at "is (object, name)
    // EVER collided somewhere" — it never verified new's OWN resolved
    // target actually belongs to that collision at all, so a genuinely
    // UNRELATED new answer (some other object entirely) would be laundered
    // as "explained" merely because the NAME happened to collide elsewhere
    // in the workspace. Require new's resolved target's own object to
    // equal `l_object_lc` (legacy's claimed object) too — the two same-name
    // colliding declarations are, by definition, both named `l_object_lc`
    // (that's what makes them collide), so a genuine explanation always
    // satisfies this; an unrelated new answer never does.
    if l.to.name.eq_ignore_ascii_case(&n.to.name)
        && let Some(l_object_lc) = legacy_local_object(l)
        && sweep.is_legacy_identity_collision(&l_object_lc, &n.to.name.to_lowercase())
        && new_target_object_lc(new_snap, n).as_deref() == Some(l_object_lc.as_str())
    {
        ledger.push(
            "outgoing",
            routine,
            Class::NewBetter(NewBetterClass::LegacyIdentityCollapse),
            format!(
                "target {:?} on object {l_object_lc:?} is a GLOBAL legacy identity collision: legacy's single collapsed slot (data={:?}) may not even be the SAME declaration new's resolver correctly targets (new range={:?})",
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
        //
        // WIDENED predicate (CDO fix-wave, team-lead finding): the ORIGINAL
        // predicate additionally required `l_app.eq_ignore_ascii_case(&n_app)`
        // (legacy's and new's reported APP NAMES agree) — real CDO data
        // (`LogMessage`/`ToBase64`/`FromBase64`, 7 findings) showed legacy's
        // arm 2 can report a DIFFERENT app name than new's resolver for the
        // exact same target (legacy's `DependencyKey`/`ExternalSource`
        // resolution and new's `AppId` resolution can disagree on which
        // declaring app "owns" a transitively-visible symbol) — an
        // app-name mismatch there is NOT evidence of a different target,
        // just of the two engines' independent app-attribution logic
        // disagreeing. The robust identity check is the ROUTINE NAME
        // instead (case-insensitive) — already known to match at this
        // point in the classifier for every one of these findings.
        if let Some(n_app) = new_dep_source_app(new_snap, n)
            && l.to.name.eq_ignore_ascii_case(&n.to.name)
        {
            ledger.push(
                "outgoing",
                routine,
                Class::NewBetter(NewBetterClass::DepSourceSpan),
                format!(
                    "target {:?}: legacy app={l_app:?} (caller-site stand-in) vs new app={n_app:?} (REAL dep-source span {:?}) — app names need not agree, routine name does",
                    n.to.name, nr(&n.to.range)
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

        // REVIEW FIX (LOW): a plain `BTreeMap<NormRange, String>` with
        // `.insert()` silently drops every item after the first when TWO
        // items share a site (a multimap situation, however rare) — use a
        // `Vec<String>` per site instead, and pair up positionally below
        // (padding the shorter side with `None`, matching the SAME
        // one-item-per-side match arms this used to run directly).
        let mut legacy_by_site: BTreeMap<NormRange, Vec<String>> = BTreeMap::new();
        for item in &legacy_ordinary {
            for r in &item.from_ranges {
                legacy_by_site
                    .entry(nr(r))
                    .or_default()
                    .push(item.from.name.to_lowercase());
            }
        }
        let mut new_by_site: BTreeMap<NormRange, Vec<String>> = BTreeMap::new();
        for item in &new_ordinary {
            for r in &item.from_ranges {
                new_by_site
                    .entry(nr(r))
                    .or_default()
                    .push(item.from.name.to_lowercase());
            }
        }

        let mut all_sites: BTreeSet<NormRange> = legacy_by_site.keys().copied().collect();
        all_sites.extend(new_by_site.keys().copied());

        for site in all_sites {
            let l_names = legacy_by_site.get(&site).cloned().unwrap_or_default();
            let n_names = new_by_site.get(&site).cloned().unwrap_or_default();
            let max_len = l_names.len().max(n_names.len());
            for i in 0..max_len {
                match (l_names.get(i), n_names.get(i)) {
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
                        // LegacyIdentityCollapse: legacy merges every colliding
                        // declaration's callers into ONE incoming bucket (no
                        // object-kind/enclosing-member/arg-type discriminator
                        // at all — see the class's doc). If THIS exact site
                        // resolves, on the new side, into a GLOBAL collision
                        // SIBLING's own incoming set instead (same `(object_lc,
                        // routine_lc)` pair, any file), that's the explanation
                        // — the caller genuinely targets a different, distinct
                        // declaration new correctly attributes it to.
                        let base_name = strip_dup_suffix(&entry.identity.routine_lc);
                        let found_in_sibling = legacy_answer_matches_a_sibling(
                            sweep,
                            &entry.identity.object_lc,
                            base_name,
                            &entry.identity.key(),
                            |sib| {
                                sib.new_incoming
                                    .iter()
                                    .any(|i| i.from_ranges.iter().any(|r| nr(r) == site))
                            },
                        );
                        if found_in_sibling {
                            ledger.push(
                            "incoming",
                            &routine,
                            Class::NewBetter(NewBetterClass::LegacyIdentityCollapse),
                            format!(
                                "legacy caller={l_name} at site {site:?} merged into this identity's bucket; new correctly attributes it to a SIBLING declaration of ({}, {base_name:?})",
                                entry.identity.object_lc
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
                        // ImplicitTriggerEdge: a record operation with a
                        // statically-true run-trigger argument (checked here
                        // via the pre-recorded, `LspSnapshot`-native set — see
                        // `run_sweep` — since the LSP response itself carries no
                        // edge-kind marker for this). Legacy structurally never
                        // models implicit-trigger dispatch at all, so this is
                        // unconditional — no legacy cross-reference needed or
                        // possible.
                        if entry.new_incoming_implicit_trigger_sites.contains(&site) {
                            ledger.push(
                            "incoming",
                            &routine,
                            Class::NewBetter(NewBetterClass::ImplicitTriggerEdge),
                            format!(
                                "new caller={n_name} at site {site:?}: a record operation's run-trigger fires this routine — legacy never models implicit-trigger dispatch"
                            ),
                        );
                            continue;
                        }
                        // ImplicitRecResolved: a Page/PageExtension/Report/
                        // ReportExtension trigger's bare-or-Rec-qualified call
                        // resolved cross-object via the implicit SourceTable
                        // binding (checked via the pre-recorded set — see
                        // `run_sweep`). Legacy's bare/qualified-call resolution
                        // is structurally same-object-only, so this is
                        // unconditional too — no legacy cross-reference possible.
                        if entry.new_incoming_implicit_rec_sites.contains(&site) {
                            ledger.push(
                            "incoming",
                            &routine,
                            Class::NewBetter(NewBetterClass::ImplicitRecResolved),
                            format!(
                                "new caller={n_name} at site {site:?}: resolved cross-object via the caller's implicit SourceTable (Rec) binding — legacy's bare/qualified-call resolution is same-object-only"
                            ),
                        );
                            continue;
                        }
                        // VariableReceiverResolved: a receiver-qualified call
                        // (same-object or cross-object — layer 4 generalized
                        // this off the caller/callee object-identity axis)
                        // whose receiver is a `var` parameter, `Rec`/`xRec`, or
                        // another local/temp shape legacy's `variable_bindings`
                        // misses — checked via the pre-recorded set (see
                        // `run_sweep`). Legacy's `lookup_variable_type` never
                        // binds parameter receivers at all, so this is
                        // unconditional too.
                        if entry.new_incoming_variable_receiver_sites.contains(&site) {
                            ledger.push(
                            "incoming",
                            &routine,
                            Class::NewBetter(NewBetterClass::VariableReceiverResolved),
                            format!(
                                "new caller={n_name} at site {site:?}: resolved via a receiver legacy's variable tracking never bound (e.g. a `var` parameter or an implicit Rec/xRec self-reference)"
                            ),
                        );
                            continue;
                        }
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
        }

        // OutgoingCardinality's incoming-axis counterpart: same caller
        // (case-insensitive), non-empty on both sides, but a DIFFERENT
        // number of DISCRETE response items (legacy never groups by caller;
        // new does) even though the flattened site set already matched
        // above 1:1. Detected by comparing raw item counts per caller name.
        //
        // SKIPPED entirely when this identity is a known `LegacyIdentityCollapse`
        // (`(object_lc, base_name)` genuinely collided): a collided
        // identity's legacy count is INFLATED by callers of SIBLING
        // declarations merged into the same bucket — every one of those
        // sites is already individually explained, per-site, by the
        // `LegacyIdentityCollapse` check above. Running this raw-count
        // heuristic on top would double-classify the SAME divergence under
        // a semantically wrong class (a grouping artifact, when the real
        // cause is the collision) rather than skip a redundant check.
        let base_name = strip_dup_suffix(&entry.identity.routine_lc);
        if sweep.is_legacy_identity_collision(&entry.identity.object_lc, base_name) {
            continue;
        }
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

            // REVIEW FIX (MED-1): `sweep.by_name` cross-references by BARE
            // NAME workspace-wide — endemic in BC, a genuinely LOST
            // subscription can be "explained" by some entirely unrelated
            // same-named routine on a different object. Legacy's own
            // detail string carries the subscriber's OBJECT too
            // (`"{obj}.{proc} [EventSubscriber]"`, `src/handlers.rs`'s
            // `incoming_calls`, lines ~230-237) — parse it out (anchored on
            // the KNOWN `.{proc} [EventSubscriber]` suffix so a quoted
            // object name with an embedded dot is never mis-split) and
            // require the sibling's OWN `object_lc` to match before
            // trusting its `new_incoming`/`new_outgoing` as the real
            // explanation.
            let expected_suffix = format!(".{} [EventSubscriber]", item.from.name);
            let subscriber_object_lc = detail
                .strip_suffix(expected_suffix.as_str())
                .unwrap_or("")
                .to_lowercase();

            // New: subscriber should appear under the publisher's OUTGOING.
            let found_in_new_outgoing = entry
                .new_outgoing
                .iter()
                .any(|o| o.to.name.eq_ignore_ascii_case(&subscriber_lc));
            // New: publisher should appear under the subscriber's INCOMING —
            // scoped to the sibling entry whose OWN object matches the
            // subscriber's object legacy itself named, not just any
            // same-named routine anywhere in the workspace.
            let found_in_new_incoming_of_subscriber =
                sweep.by_name(&subscriber_lc).iter().any(|sub_entry| {
                    sub_entry.identity.object_lc == subscriber_object_lc
                        && sub_entry
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
    // REVIEW FIX (LOW): `args[0]` panics on `Some(vec![])` (an empty but
    // present arguments array) — `.first()` degrades to `None` instead.
    let arg0 = args.first()?;
    let obj = arg0.get("object")?.as_str()?.to_lowercase();
    let proc = arg0.get("procedure")?.as_str()?.to_lowercase();
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
                    // lens key needs disambiguating between THREE DIFFERENT
                    // root causes that all inflate `effective_incoming_count`
                    // relative to legacy's `get_incoming_call_count`:
                    // ImplicitTriggerEdge (a record-operation-fired trigger
                    // counts a caller legacy never models at all) and
                    // EventDirectionMoved (this routine is a SUBSCRIBER —
                    // its publisher now counts as an incoming caller, a
                    // linkage legacy's `event_subscriptions` map, keyed by
                    // PUBLISHER not subscriber, can never show on the
                    // subscriber's OWN lens) both take priority; otherwise
                    // it's CaseFoldHit's codeLens footprint (an extra caller
                    // legacy's interner never associated at all).
                    // `file_rel_case` is irrelevant here — this identity is
                    // only ever used for `.key()` (a `file_rel`/object/
                    // routine-only computation, see `RoutineIdentity::key`),
                    // never to query legacy, so any value satisfies the type.
                    let entry_key = RoutineIdentity {
                        file_rel: file_rel.to_lowercase(),
                        file_rel_case: file_rel.to_string(),
                        object_lc: key.0.clone(),
                        routine_lc: key.1.clone(),
                    }
                    .key();
                    let sweep_entry = sweep.entries.get(&entry_key);
                    let is_implicit_trigger_linked = sweep_entry
                        .is_some_and(|e| !e.new_incoming_implicit_trigger_sites.is_empty());
                    let is_implicit_rec_linked =
                        sweep_entry.is_some_and(|e| !e.new_incoming_implicit_rec_sites.is_empty());
                    let is_variable_receiver_linked = sweep_entry
                        .is_some_and(|e| !e.new_incoming_variable_receiver_sites.is_empty());
                    let is_event_linked = sweep_entry.is_some_and(|e| {
                        e.new_incoming.iter().any(|i| {
                            i.from
                                .detail
                                .as_deref()
                                .is_some_and(|d| d.contains("[EventPublisher]"))
                        })
                    });
                    if is_implicit_trigger_linked {
                        ledger.push(
                            "codeLens",
                            &routine,
                            Class::NewBetter(NewBetterClass::ImplicitTriggerEdge),
                            format!("ref count legacy={l_refs:?} vs new={n_refs:?} (new counts a record-operation-fired implicit-trigger caller legacy never models)"),
                        );
                    } else if is_implicit_rec_linked {
                        ledger.push(
                            "codeLens",
                            &routine,
                            Class::NewBetter(NewBetterClass::ImplicitRecResolved),
                            format!("ref count legacy={l_refs:?} vs new={n_refs:?} (new counts a cross-object caller resolved via the caller's implicit SourceTable binding)"),
                        );
                    } else if is_variable_receiver_linked {
                        ledger.push(
                            "codeLens",
                            &routine,
                            Class::NewBetter(NewBetterClass::VariableReceiverResolved),
                            format!("ref count legacy={l_refs:?} vs new={n_refs:?} (new counts a caller resolved via a receiver legacy's variable tracking never bound)"),
                        );
                    } else if is_event_linked {
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
                } else if sweep.is_legacy_identity_collision(&key.0, &key.1) {
                    // LegacyIdentityCollapse: legacy's merged (object,
                    // name) incoming bucket counts callers of EVERY
                    // colliding declaration (same-object overload, or an
                    // entirely different object/kind sharing the name);
                    // new correctly counts only THIS declaration's own
                    // callers.
                    ledger.push(
                        "codeLens",
                        &routine,
                        Class::NewBetter(NewBetterClass::LegacyIdentityCollapse),
                        format!(
                            "ref count legacy={l_refs:?} (merges every colliding declaration's callers) vs new={n_refs:?} (this declaration only)"
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
            // ImplicitTriggerEdge/ImplicitRecResolved: legacy sees ZERO
            // incoming calls because the ONLY real caller is a record-
            // operation-fired trigger dispatch or a cross-object implicit-
            // SourceTable call — checked BEFORE CaseFoldHit, since either
            // would ALSO make `routine_name_has_new_incoming` return `true`
            // for the wrong reason.
            let implicit_class = routine_implicit_dispatch_class(sweep, &file_rel, msg);
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
            } else if let Some(class) = implicit_class {
                ledger.push(
                    "diagnostics",
                    &file_rel,
                    Class::NewBetter(class),
                    format!("legacy flags {msg:?} (zero incoming); new's only real caller is an implicit-dispatch edge legacy structurally cannot model"),
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
            // REVIEW FIX (HIGH-2): the ORIGINAL check was an unconditional
            // blanket over this WHOLE direction — any new-only unused-
            // procedure message, regardless of cause, was auto-granted this
            // class, which would silently launder a narrowed-exclusion-rule
            // regression (a hypothetical R3/R4 attribute-set drift) as if it
            // were R2Precision. Require POSITIVE evidence of the actual R2
            // mechanism instead: the flagged routine must itself carry a
            // real `[EventSubscriber(...)]` attribute (read from the owned
            // IR's `RoutineDecl.attributes`, not text-sniffed) — i.e. legacy
            // really would have excluded it via R2's blanket check, and
            // new's edge-based check correctly flags it anyway because no
            // real EventFlow edge resolves. Anything else falls through to
            // `NewUnexplained` below — never silently granted.
            if routine_has_event_subscriber_attribute(new_snap, &file_rel, msg) {
                ledger.push(
                    "diagnostics",
                    &file_rel,
                    Class::NewBetter(NewBetterClass::R2Precision),
                    format!(
                        "new flags {msg:?}, legacy's blanket [EventSubscriber] exclusion hides it"
                    ),
                );
            } else {
                ledger.push(
                    "diagnostics",
                    &file_rel,
                    Class::NewUnexplained,
                    format!(
                        "new flags {msg:?}, legacy does not, and the routine carries no [EventSubscriber] attribute to explain it via R2Precision"
                    ),
                );
            }
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

/// R2Precision's positive-evidence gate (review fix-wave HIGH-2): does the
/// routine named in `msg` (legacy's `unused-procedure` message format:
/// "Procedure '{object}.{name}' is never called") actually carry a
/// source-level `[EventSubscriber(...)]` attribute? Reads the OWNED IR's
/// `RoutineDecl.attributes` (already-lowercased attribute names, `crates/
/// al-syntax/src/ir/decl.rs`'s own doc) directly — never text-sniffed —
/// scoped to the SAME object the flagged routine belongs to (via
/// `decl.id.object`'s display name), matching by routine name.
fn routine_has_event_subscriber_attribute(
    new_snap: &LspSnapshot,
    file_rel: &str,
    msg: &str,
) -> bool {
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
    let Some(decl) = new_snap.decls_by_file[virtual_path]
        .iter()
        .find(|d| d.name.eq_ignore_ascii_case(name))
    else {
        return false;
    };
    let Some(object_display_name) = object_name(&new_snap.graph, &decl.id.object) else {
        return false;
    };
    let Some(parsed) = new_snap.parsed.get(virtual_path) else {
        return false;
    };
    parsed.file.objects.iter().any(|obj| {
        obj.name.eq_ignore_ascii_case(object_display_name)
            && obj.routines.iter().any(|r| {
                r.name.eq_ignore_ascii_case(&decl.name)
                    && r.attributes.iter().any(|a| a == "eventsubscriber")
            })
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

/// Same name-extraction as `routine_name_has_new_incoming`, but returns
/// WHICH implicit-dispatch class (if any) explains legacy's zero-incoming
/// count for this routine — checked before `CaseFoldHit`, since either
/// would also make that helper return `true` for the wrong reason.
fn routine_implicit_dispatch_class(
    sweep: &Sweep,
    file_rel: &str,
    msg: &str,
) -> Option<NewBetterClass> {
    let name = msg
        .split('\'')
        .nth(1)
        .and_then(|qualified| qualified.split('.').next_back())?;
    let name_lc = name.to_lowercase();
    let entry = sweep
        .entries
        .values()
        .find(|e| e.identity.file_rel == file_rel && e.identity.routine_lc == name_lc)?;
    if !entry.new_incoming_implicit_trigger_sites.is_empty() {
        Some(NewBetterClass::ImplicitTriggerEdge)
    } else if !entry.new_incoming_implicit_rec_sites.is_empty() {
        Some(NewBetterClass::ImplicitRecResolved)
    } else if !entry.new_incoming_variable_receiver_sites.is_empty() {
        Some(NewBetterClass::VariableReceiverResolved)
    } else {
        None
    }
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
    // OutgoingCardinality is NOT exercised by this fixture: Beta.Process's
    // 3 callers (Alpha.DoWork, MyTable.OnValidate, MyPage.OnOpenPage) each
    // have exactly ONE call site — no per-caller grouping-vs-ungrouped item
    // count ever differs. (A prior version of this pin asserted `1` here,
    // but that count was an ARTIFACT of a since-fixed double-classification
    // bug — see the CDO fix-wave's report — where the Calc-overload
    // collision's raw item-count mismatch was ALSO being caught by this
    // unrelated cardinality heuristic; the collision is now exhaustively
    // explained per-site by `LegacyIdentityCollapse` alone, and this
    // heuristic correctly stays silent for a collided identity.)
    // OutgoingCardinality is exercised for real by `lsp-diff-core`'s
    // `Zeta.CallTwice` script below.
    assert_eq!(
        counts
            .get("NewBetter::OutgoingCardinality")
            .copied()
            .unwrap_or(0),
        0,
        "OutgoingCardinality must be 0 on lsp-incr; counts={counts:?}"
    );
    // LegacyIdentityCollapse (6, re-measured after the CDO fix-wave's
    // generalization from same-file-only to GLOBAL collision detection):
    // 1 prepare finding (Calc-Integer's position mismatches, matches its
    // Calc-Text sibling) + 1 outgoing finding (MyTableExt.OnValidate's
    // qualified `Alpha.Calc(1)` call) + 3 incoming findings (2 for the
    // primary `alpha.calc` entry's merged DoWork/OnValidate callers, 1 for
    // the `alpha.calc#dup0` sibling's own merged DoWork caller) + 1
    // codeLens finding (Alpha.Calc's inflated ref count). Went from 7 to 6
    // net: the fix-wave's generalization ALSO surfaced 2 real
    // `UnqualifiedCallResolved` findings that a pre-existing bug in this
    // harness had been misclassifying as the overload class (see
    // `UnqualifiedCallResolved`'s count comment below) — a net -2 there,
    // +1 here from independently querying `alpha.calc#dup0` for the first
    // time (previously skipped entirely by construction).
    assert_eq!(
        counts
            .get("NewBetter::LegacyIdentityCollapse")
            .copied()
            .unwrap_or(0),
        6,
        "LegacyIdentityCollapse: Alpha's Calc(Integer)/Calc(Text) overload set, across prepare/incoming/outgoing/codeLens; counts={counts:?}"
    );
    // UnqualifiedCallResolved (3, up from 1): the fix-wave's
    // `legacy_local_object`-gated `LegacyIdentityCollapse` check no longer
    // matches a bareword-placeholder item just because its NAME happens to
    // be an overloaded routine (`Calc`) — a placeholder (`data: None`) was
    // never really "resolved to the wrong overload" to begin with, so
    // `Alpha.DoWork`'s TWO unqualified `Calc(1)`/`Calc('x')` calls now
    // correctly join the pre-existing `Løbenr()` finding here instead of
    // being misclassified as `LegacyIdentityCollapse`.
    assert_eq!(
        counts
            .get("NewBetter::UnqualifiedCallResolved")
            .copied()
            .unwrap_or(0),
        3,
        "UnqualifiedCallResolved: Alpha.DoWork's bareword Calc(1)/Calc('x')/Løbenr() calls; counts={counts:?}"
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

/// `LegacyIdentityCollapse` fixture reproduction (CDO fix-wave, binding —
/// "fixtures can express both shapes... so the class is pinned always-on
/// in CI, not CDO-only"): a codeunit and a page sharing the display name
/// "Shared Name" (`SharedCU.al`/`SharedPage.al`, each with its own
/// `GetRecipients`, called from `Caller.al`), plus a table with two
/// different fields' same-named `OnValidate` triggers (`TwoTriggers.al`).
/// `ImplicitTriggerEdge`/`ImplicitRecResolved` fixture reproduction (CDO
/// layer-2/2b fix-waves). ALSO serves as the empirical FALSIFICATION record
/// for the layer-2 brief's `NestedTriggerCaller` hypothesis ("legacy's
/// `ParsedFile` projection never captured nested [field/action/dataitem-
/// scoped] triggers as caller definitions"): `NestedTable.al`/`NestedPage.al`/
/// `NestedPageField.al`/`NestedReport.al` reproduce the table-field,
/// page-action, page-field, and report-dataitem trigger-as-caller shapes
/// respectively, each with a SAME-OBJECT bareword call inside — every one of
/// them MATCHES on `incoming` (legacy DOES correctly capture the nested
/// trigger as a caller-scope `Definition` and correctly attributes both
/// qualified and unqualified calls inside it — verified against
/// `src/parser.rs`'s `collect_routines`/`parse_file_ir`, which walk the
/// object subtree unconditionally regardless of nesting depth).
/// `ImplicitTrigger.al`/`ImplicitTriggerCaller.al` reproduce the GENUINE
/// implicit-TRIGGER gap: `Rec.Insert(true)` implicitly fires `OnInsert`,
/// which legacy structurally cannot model at all. `ImplicitRecTable.al`/
/// `ImplicitRecPage.al` reproduce the layer-2b GENUINE implicit-REC gap
/// (confirmed by the controller against real CDO source, `Page 6175306
/// "CDO E-Mail Template Lines"`): a page action's `OnAction` and the page's
/// own `OnAfterGetCurrRecord` each make a bareword call CROSS-OBJECT into
/// the page's bound `SourceTable`'s own procedure — legacy's bare-call
/// resolution is same-object-only, so this is structurally invisible to it,
/// unlike the SAME-OBJECT bareword calls the falsification fixtures above
/// prove legacy handles correctly. `VariableReceiverTable.al`/
/// `VariableReceiverCaller.al` reproduce the layer-3 GENUINE parameter-
/// receiver gap (confirmed by the controller against real CDO source,
/// `Codeunit 6175274 "CDO Continia Online PDF Mgt"`'s `MergePdf` calling
/// `DOFile.IsPdf()` where `DOFile` is a `var` PARAMETER): legacy's
/// `variable_bindings` is populated ONLY from a routine's `var`-section
/// LOCALS, never its parameter list, so a parameter receiver is invisible
/// to `lookup_variable_type`. The SAME fixture's `UseLocalVar` procedure
/// (a LOCAL `var`-section variable receiver) is a deliberate CONTRAST case:
/// it MATCHES cleanly (verified empirically, not assumed) — legacy's
/// `push_variables_ir` DOES capture `var`-section locals correctly, so only
/// the parameter shape is a genuine gap, not "any variable receiver."
#[test]
fn lsp_diff_nested_fixture_has_zero_regressions_and_zero_unexplained() {
    let ledger = run_differential(&fixture_path("lsp-diff-nested"), false);
    ledger.assert_gates_clean("lsp-diff-nested");
    let counts = ledger.class_counts();

    // 2 total: Rec.Insert(true)'s ImplicitTrigger edge shows up once on the
    // incoming axis (OnInsert's caller) and once on codeLens (OnInsert's
    // inflated ref count) — outgoing's own divergence for the SAME call
    // site is already explained by UnqualifiedCallResolved (legacy's arm-3
    // "totally unresolved" placeholder — `Insert` is a builtin record
    // method, never a user Definition — is the IDENTICAL `data: None` shape
    // an unqualified call produces, so the existing predicate already
    // covers it without double-counting).
    assert_eq!(
        counts
            .get("NewBetter::ImplicitTriggerEdge")
            .copied()
            .unwrap_or(0),
        2,
        "ImplicitTriggerEdge: Rec.Insert(true) firing OnInsert, incoming + codeLens; counts={counts:?}"
    );
    // 6 total: the page action's OnAction -> SetBackgroundPDF and the
    // page's OnAfterGetCurrRecord -> RefreshCache cross-object implicit-Rec
    // calls each contribute 1 incoming + 1 codeLens + 1 diagnostics finding
    // (both routines otherwise show zero incoming to legacy, so legacy
    // ALSO flags them as unused-procedure false positives) = 2 * 3 = 6.
    // Outgoing's own divergence for both call sites is already covered by
    // UnqualifiedCallResolved (both are bare/unqualified call syntax).
    assert_eq!(
        counts
            .get("NewBetter::ImplicitRecResolved")
            .copied()
            .unwrap_or(0),
        6,
        "ImplicitRecResolved: page action + page trigger cross-object SourceTable calls, incoming + codeLens + diagnostics; counts={counts:?}"
    );
    // 11 total (layer 3's 3 + layer 4's 8 generalization findings) — every
    // individual finding inspected via a temporary probe, not just the
    // summed count:
    // - MergePdf's `var DOFile` parameter receiver calling `DOFile.IsPdf()`:
    //   incoming + codeLens + diagnostics = 3 (layer 3's original pin;
    //   `UseLocalVar`'s LOCAL variable receiver still contributes NOTHING
    //   here — it matches cleanly — proving the gap is parameter-specific,
    //   not "any variable receiver").
    // - `RunDispatchCaller`'s `QueueMgt.Run()` (a `var` Codeunit-typed LOCAL
    //   dispatching to `RunDispatchTarget`'s `OnRun`): incoming + codeLens
    //   = 2 (no diagnostics — `OnRun` is a trigger, and this fixture's
    //   diagnostics scan only flags unused PLAIN PROCEDURES, never
    //   triggers). Matched WITHOUT any dedicated `EdgeKind::Run` handling
    //   (`resolve_member`'s `Run`-on-Codeunit special case in
    //   `src/program/resolve/resolver.rs` produces an ordinary Call-kind
    //   edge) — and, verified via a temporary probe BEFORE any layer-4 code
    //   change, this shape ALREADY matched even under layer 3's
    //   cross-object-restricted predicate (caller and callee are different
    //   objects), so it needed no fix, just this regression-guard fixture.
    // - `AddNode`'s same-object `var` parameter of its OWN codeunit type
    //   calling `NewNode.SetNode()`: incoming + codeLens + diagnostics = 3
    //   (`SetNode` has no other caller in this fixture, so legacy ALSO
    //   flags it unused).
    // - `MergeWithSelf`'s same-object `var` parameter of its OWN record
    //   type calling `Other.IsPasswordProtected()`: incoming + codeLens = 2
    //   — NOT diagnostics, since `IsPasswordProtected` already has a real
    //   caller via `UseLocalVar` that legacy resolves correctly, so legacy
    //   never flags it unused in the first place.
    // - `GetPlainText`'s same-object implicit `Rec.`-qualified call to
    //   `Rec.IsPdf()`: incoming ONLY = 1 — codeLens and diagnostics for
    //   `IsPdf` are ALREADY counted once each via MergePdf's original
    //   divergence (codeLens is a per-ROUTINE ref-count comparison, not
    //   per-call-site; diagnostics is a per-routine unused-flag), so a
    //   second unresolved caller adds only a second incoming site.
    // 3 + 2 + 3 + 2 + 1 = 11, matching the probe exactly.
    assert_eq!(
        counts
            .get("NewBetter::VariableReceiverResolved")
            .copied()
            .unwrap_or(0),
        11,
        "VariableReceiverResolved: layer 3's MergePdf pin (3) + layer 4's RunDispatch (2: incoming+codeLens, OnRun is a trigger so never diagnostics-eligible) + AddNode/SetNode (3) + MergeWithSelf/IsPasswordProtected (2: no new diagnostics, already has a real caller via UseLocalVar) + GetPlainText/IsPdf (1: incoming only, codeLens+diagnostics already counted via MergePdf); counts={counts:?}"
    );
    // The 4 nested-trigger-as-caller shapes (table field, page action, page
    // field, report dataitem) must ALL match cleanly — this IS the
    // falsification record for NestedTriggerCaller (see this test's doc).
    // `UseLocalVar`'s local-variable receiver ALSO matches cleanly — proof
    // that legacy's variable-binding gap is parameter-specific. Bumped from
    // 43 to 56 (+13) by the layer-4 fixture additions' own remaining facts
    // (prepare/outgoing/codeLens/diagnostics Matches for `AddNode`,
    // `SetNode`, `MergeWithSelf`, `GetPlainText`, `RunDispatchCaller`'s
    // `SendQueue`, and `RunDispatchTarget`'s `OnRun` — every axis of every
    // new routine that ISN'T one of the 11 VariableReceiverResolved
    // divergences above) — inspected via the same probe, not blindly
    // copied forward.
    assert_eq!(
        counts.get("Match").copied().unwrap_or(0),
        56,
        "Match: the 4 nested-trigger-caller falsification shapes + UseLocalVar's local-variable-receiver match + every other routine's remaining facts (layer 3 + layer 4 fixtures); counts={counts:?}"
    );
}

#[test]
fn lsp_diff_identity_fixture_has_zero_regressions_and_zero_unexplained() {
    let ledger = run_differential(&fixture_path("lsp-diff-identity"), false);
    ledger.assert_gates_clean("lsp-diff-identity");
    let counts = ledger.class_counts();

    // 8 total: the page+codeunit "Shared Name" collision contributes 1
    // prepare + 2 outgoing (one per caller) + 2 incoming (one per
    // declaration's merged-in sibling caller) + 2 codeLens (both
    // declarations' inflated ref count) = 7; the two-field same-object
    // OnValidate collision contributes 1 prepare finding (triggers aren't
    // callable, so incoming/outgoing/codeLens never diverge for them).
    assert_eq!(
        counts
            .get("NewBetter::LegacyIdentityCollapse")
            .copied()
            .unwrap_or(0),
        8,
        "LegacyIdentityCollapse: page+codeunit name collision (7) + two-field same-trigger collision (1); counts={counts:?}"
    );
}

/// `ObjectIdAdditive` is out-of-scope for this harness's own driver (module
/// doc's scope decision — Step 1 never queries `dependencyDocumentSymbol`).
/// Pinned at 0 across every always-on fixture, with the reason documented,
/// rather than silently omitted from the ratchet.
#[test]
fn object_id_additive_is_out_of_driver_scope_pinned_zero() {
    for fixture in [
        "lsp-incr",
        "lsp-diff-core",
        "lsp-diff-deps",
        "lsp-diff-identity",
        "lsp-diff-nested",
    ] {
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
    let counts = ledger.class_counts();

    // RATCHET PINS — measured on the real 551-file Continia (CDO)
    // workspace at commit `ebad1b9` (the layer-4 `VariableReceiverResolved`
    // generalization), via a temporary count-dump probe the controller ran
    // and then reverted (this file was clean at HEAD when these numbers
    // were captured — not self-reported by this test). The gate held:
    // 8/8 differential tests green, REGRESSION=0, NEW_UNEXPLAINED=0,
    // H-10 green. Every one of these numbers is a PIN, not a floor or
    // ceiling: a future change that moves ANY of them must be explained
    // (either a new fixture-verified mechanical class, or a deliberate,
    // documented rebaseline) — never blind-updated to make a test pass.
    // What the numbers MEAN (per the controller, recorded here so the
    // headline evidence isn't just bare integers):
    // - `Match` (12,089): identical answers from both engines on this
    //   real workspace — the bulk of the surface area.
    // - `UnqualifiedCallResolved` (36,971, by far the largest NewBetter
    //   class): legacy's blanket `"(local)"` placeholder for EVERY
    //   unqualified call (same-object bare call, or a global/builtin
    //   bareword) — legacy never even attempts resolution for these; new
    //   correctly resolves or correctly omits.
    // - `LegacyIdentityCollapse` (1,625): legacy's bare
    //   `(object NAME text, routine NAME text)` keying collides same-named
    //   routines across different objects/files/kinds into ONE slot —
    //   these are legacy's WRONG answers, not just missing ones.
    // - `VariableReceiverResolved` (2,169): calls through a variable or
    //   parameter receiver legacy's `variable_bindings` never bound
    //   (parameters, `Rec`/`xRec`, same-object or cross-object — see the
    //   layer-3/layer-4 fix-waves).
    // - `ImplicitTriggerEdge` (989): record operations with a statically-
    //   true run-trigger argument (`Rec.Insert(true)`, etc.) — legacy
    //   structurally never models trigger dispatch from a builtin record
    //   method.
    // - `ImplicitRecResolved` (778): Page/PageExtension/Report/
    //   ReportExtension bare-or-`Rec.`-qualified calls resolved
    //   cross-object via the caller's implicit SourceTable binding.
    // - `OutgoingCardinality`, `R2Precision`, `EventDirectionMoved`,
    //   `R6InterfaceExclusion`, `CaseFoldHit`, `CrossAppTarget`,
    //   `DepSourceSpan`: the brief's original 9 mechanical classes,
    //   each non-zero and real on this workspace at the scale shown.
    const CDO_PINS: &[(&str, Option<usize>)] = &[
        ("Match", Some(12089)),
        ("NewBetter::UnqualifiedCallResolved", Some(36971)),
        ("NewBetter::VariableReceiverResolved", Some(2169)),
        ("NewBetter::LegacyIdentityCollapse", Some(1625)),
        ("NewBetter::ImplicitTriggerEdge", Some(989)),
        ("NewBetter::ImplicitRecResolved", Some(778)),
        ("NewBetter::OutgoingCardinality", Some(430)),
        ("NewBetter::R2Precision", Some(68)),
        ("NewBetter::EventDirectionMoved", Some(47)),
        ("NewBetter::R6InterfaceExclusion", Some(14)),
        ("NewBetter::CaseFoldHit", Some(12)),
        ("NewBetter::CrossAppTarget", Some(12)),
        ("NewBetter::DepSourceSpan", Some(12)),
        // Absent from the controller's dump. `class_counts()` builds its
        // `BTreeMap` by incrementing an entry per finding (see its impl
        // above) — a class with ZERO findings never gains a key at all,
        // so absence from an exhaustive dump is structurally equivalent
        // to a measured 0, not an oversight. Flag for re-verification if
        // a future CDO re-run ever shows this nonzero.
        ("NewBetter::AbiSymbolShape", Some(0)),
        // Always 0 — out of this driver's scope, see the module doc and
        // `object_id_additive_is_out_of_driver_scope_pinned_zero`.
        ("NewBetter::ObjectIdAdditive", Some(0)),
    ];
    for (class, expected) in CDO_PINS {
        if let Some(expected) = expected {
            assert_eq!(
                counts.get(*class).copied().unwrap_or(0),
                *expected,
                "CDO class-count pin for {class}: counts={counts:?}"
            );
        }
    }
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
    // REVIEW FIX (MED-4): the ORIGINAL selection only required an ordinary
    // (non-event) legacy incoming entry — never checking the caller was
    // actually in a DIFFERENT file, so a same-file caller could satisfy it
    // despite the test's own name promising "cross-file". Require at least
    // one ordinary caller whose OWN file (`i.from.uri`, which legacy's
    // `incoming_calls` sets to the real call-site file — see
    // `src/handlers.rs`'s `path_to_uri(&call.file)`) differs from the
    // target's own file.
    let cfg = DiagnosticConfig::default();
    let (sweep, _lenses) = run_sweep(&ws, &legacy, &base_new, &cfg);
    let Some(target) = sweep.entries.values().find(|e| {
        e.legacy_prepare.is_some()
            && e.legacy_incoming.iter().any(|i| {
                i.from.detail.is_none()
                    && uri_to_rel(&ws, i.from.uri.as_str()) != e.identity.file_rel
            })
    }) else {
        panic!("CDO workspace sanity: expected at least one routine with a real CROSS-FILE caller");
    };
    // CASE-PRESERVING path (review fix-wave HIGH-1, same fix as `run_sweep`'s
    // per-identity query loop) — querying legacy with the lowercased
    // `file_rel` would miss `path_cache` on a case-sensitive filesystem.
    let target_file = ws.join(&target.identity.file_rel_case);
    let target_uri = path_to_uri(&target_file);
    let pre_edit_legacy_incoming_count = target.legacy_incoming.len();
    // REVIEW FIX (MED-4): capture the PRE-edit RAW `EdgeRef` count (the SAME
    // unit `post_edit_new_incoming` below measures — `new_snap.incoming`'s
    // raw per-edge Vec, NOT the grouped-by-caller LSP `incoming()` item
    // count `target.new_incoming.len()` used to be compared against) so the
    // post-edit assertion is a genuine same-unit equality check, not a
    // `.max()` construction that silently accepted any count `>=` a
    // DIFFERENT-unit baseline.
    let pre_target_vp = base_new
        .decls_by_file
        .keys()
        .find(|k| k.to_lowercase() == target.identity.file_rel)
        .expect("target file present in the pre-edit snapshot (it's where target was found)");
    let pre_target_decl = base_new.decls_by_file[pre_target_vp]
        .iter()
        .find(|d| d.name.eq_ignore_ascii_case(&target.identity.routine_lc))
        .expect("target decl present in the pre-edit snapshot");
    let pre_edit_new_incoming_raw = base_new
        .incoming
        .get(&pre_target_decl.id)
        .map(Vec::len)
        .unwrap_or(0);

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
        post_edit_new_incoming, pre_edit_new_incoming_raw,
        "new engine must KEEP its incoming edges (same raw EdgeRef count) across the same no-op save"
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
