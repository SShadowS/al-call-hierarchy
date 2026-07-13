//! Recompute-diff-publish-clear diagnostics engine on the engine-backed
//! `LspSnapshot` (T3 Task 12) — the replacement for `src/server.rs`'s
//! `publish_all_diagnostics`/`get_code_quality_diagnostics` +
//! `src/handlers.rs`'s `get_unused_procedure_diagnostics`, cut over at Task 15.
//!
//! [`compute_all`] recomputes every diagnostic (unused-procedure + code
//! quality) from scratch on every call — no incremental diagnostic state —
//! and [`DiagnosticsState::diff`] is the ONLY place that compares against
//! what was last published, emitting exactly the `(uri, diagnostics)` pairs
//! that changed, INCLUDING a uri whose diagnostics dropped to zero (legacy's
//! `publish_all_diagnostics` never did this: it only ever published
//! non-empty file buckets, so a fixed procedure's stale "unused" hint would
//! linger in the editor until the NEXT unrelated finding in that same file
//! happened to overwrite it — see that function's doc for the gap this
//! closes).
//!
//! # Unused-procedure rule inventory (task brief Step 1 — binding)
//!
//! Ported from `src/indexer.rs:159-218` (attribute-driven exclusions) +
//! `src/graph.rs:888-905`/`865-886` (`get_unused_procedures`/
//! `get_incoming_call_count`), each rule keeping its legacy pinned test name
//! for traceability:
//!
//! | # | Rule | Legacy mechanism | Legacy test | Engine mechanism |
//! |---|------|------------------|-------------|-------------------|
//! | R1 | Only `Procedure`-kind routines are eligible (triggers excluded) | `graph.rs`'s `get_unused_procedures` filters `def.kind == DefinitionKind::Procedure` | (structural; exercised by every other test below, which all use non-trigger fixtures) | `routine.kind == RoutineKind::Procedure` on the `RoutineDecl` correlated via [`crate::lsp::lens::find_routine_by_origin`] |
//! | R2 | An `[EventSubscriber]` routine is never flagged (invoked implicitly by its publisher) | `indexer.rs` reclassifies its `DefinitionKind` to `EventSubscriber`, excluded by R1's kind filter | `test_event_subscriber_not_flagged_unused` | **SUBSUMED** — no attribute check at all. [`crate::lsp::lens::effective_incoming_count`] already counts an `EventFlow` edge targeting this routine (`LspSnapshot::incoming`), so a genuinely-wired subscriber falls out of the zero-incoming check naturally. See "Semantic difference (a)" below. |
//! | R3 | Framework-invoked test methods/handlers (`[Test]`, `[ConfirmHandler]`, `[MessageHandler]`, `[PageHandler]`, `[ModalPageHandler]`, `[ReportHandler]`, `[RequestPageHandler]`, `[SendNotificationHandler]`, `[RecallNotificationHandler]`, `[SessionSettingsHandler]`, `[StrMenuHandler]`, `[FilterPageHandler]`, `[HyperlinkHandler]`) marked `implicitly_invoked` | `parser.rs`'s `is_framework_invocation_attribute` + `indexer.rs:187-197` | `test_test_method_not_flagged_unused`, `test_test_handler_not_flagged_unused` | Reuses the SAME `crate::analysis::is_framework_invocation_attribute` (relocated from `parser.rs` in the review fix-wave — see that module's doc) against `RoutineDecl.attributes` (already lowercased) |
//! | R4 | `[IntegrationEvent]`/`[BusinessEvent]` publishers are ALWAYS excluded — their real subscribers typically live in downstream apps this workspace never loads | `indexer.rs:199-218` marks them `implicitly_invoked` unconditionally | `test_public_event_publishers_not_flagged` | Reuses `program::resolve::event::is_event_publisher` — `Some(PublisherKind::Integration)`/`Some(PublisherKind::Business)` excludes unconditionally, regardless of incoming count |
//! | R5 | `[InternalEvent]` is NOT auto-excluded: flagged unless subscribed OR raised (its subscribers must live in the SAME app, so they're always visible) | `graph.rs`'s `get_incoming_call_count` = direct calls + `event_subscriptions.get(qname).len()` | `test_orphan_internal_event_is_flagged`, `test_subscribed_or_raised_internal_event_not_flagged` | Falls through to the SAME zero-`effective_incoming_count` check every ordinary procedure uses — no special case needed (see that function's doc) |
//! | R6 | An interface method's own SIGNATURE is never flagged — it can never itself be a call target (dispatch always resolves to an IMPLEMENTING object's own routine, a distinct `RoutineNodeId`), so it structurally always shows zero incoming regardless of real usage | **NONE — legacy shared this exact false positive.** `graph.rs`'s `get_unused_procedures` never special-cased an Interface-kind object either | (none — a review-fix-wave finding, not a legacy-pinned case; NEW_BETTER, adjudicated in the T3 Task-12 review fix-wave, not present in either engine before) | `decl.id.object.kind == ObjectKind::Interface` — no rule applies to the IMPLEMENTING codeunit's own routine, which stays subject to every rule above |
//!
//! No PORT-GAP was found for R1-R5: every legacy rule's input data (routine
//! kind, attribute names, incoming-edge evidence) is available on the engine
//! side, either directly on `RoutineDecl` (kind, `attributes`) or via
//! `LspSnapshot`'s edge indexes. R6 is a NEW rule neither engine had —
//! see its table row.
//!
//! ## Known semantic differences (deliberate, not bugs)
//!
//! (a) **A subscriber routine** is "used" because [`effective_incoming_count`]
//!     finds a real `EventFlow` edge targeting it — NOT because of a blanket
//!     `[EventSubscriber]`-attribute exclusion. A subscription whose
//!     publisher/event name typo's or doesn't resolve to any real publisher
//!     produces NO edge, so the engine version correctly flags it as unused
//!     where legacy's attribute-blanket rule never could — a precision
//!     improvement, not a preserved behavior, for the (documented, tested)
//!     well-formed case the two engines agree.
//! (b) **A publisher routine** is NOT "used" merely because it appears as the
//!     `from` of an `EventFlow` edge — `emit_event_flow_edges` unconditionally
//!     emits one edge per publisher declaration even with zero subscribers, so
//!     edge PRESENCE is not usage evidence. [`effective_incoming_count`] sums
//!     `edge.routes.len()` (the REAL resolved-subscriber count) for exactly
//!     this reason — only a publisher with ≥1 real subscriber, or a direct
//!     call/raise landing in `snap.incoming`, counts as used. This mirrors
//!     legacy's OWN semantics (`event_subscriptions.get(qname).len()` counts
//!     subscriptions, not mere event declarations) rather than the mechanism.
//!
//! # Diagnostic codes/severities/messages
//!
//! Ported byte-for-byte from `src/handlers.rs:600-635`
//! (`get_unused_procedure_diagnostics`) and `src/server.rs:353-506`
//! (`get_code_quality_diagnostics`) — see [`push_quality_diagnostics`] and
//! [`unused_procedure_diagnostic`] for the exact strings.

use std::collections::HashMap;

use al_syntax::ir::{ObjectKind, RoutineKind};
use lsp_types::{Diagnostic, DiagnosticSeverity, DiagnosticTag, NumberOrString};

use crate::config::DiagnosticConfig;
use crate::lsp::encoding::{LineTable, PositionEncoding};
use crate::lsp::handlers::{object_name_for, origin_to_range};
use crate::lsp::lens::{effective_incoming_count, find_routine_by_origin, parameter_count_of};
use crate::lsp::snapshot::{DeclEntry, LspSnapshot};
use crate::program::resolve::event::{PublisherKind, is_event_publisher};
use crate::protocol::path_to_uri;

/// Full recompute over the snapshot: every workspace file gets an entry
/// (possibly an empty `Vec` — "including now-empty URIs", the task brief's
/// binding requirement so [`DiagnosticsState::diff`] can detect a file whose
/// findings all disappeared), keyed by its `file://` URI string.
///
/// `enc` (T3 Task 15 cutover): every position this module emits crosses the
/// LSP boundary through it — the negotiated encoding, never a hardcoded
/// `Utf16` guess (see the two now-removed TODOs this replaced).
#[must_use]
pub fn compute_all(
    snap: &LspSnapshot,
    enc: PositionEncoding,
    cfg: &DiagnosticConfig,
) -> HashMap<String, Vec<Diagnostic>> {
    let mut out: HashMap<String, Vec<Diagnostic>> = HashMap::new();

    for virtual_path in snap.parsed.keys() {
        out.entry(workspace_uri(snap, virtual_path)).or_default();
    }

    // Local, single-pass memoization (t3 whole-branch review, "while you're
    // there" item — linear-but-real, not the quadratic blocker): makes the
    // "at most one `LineTable::new` per file per `compute_all` call"
    // invariant STRUCTURAL rather than incidental, so a future addition to
    // this function's per-decl body can never silently regress into
    // rebuilding a file's table more than once per pass. Deliberately a
    // LOCAL cache, not a snapshot-level one — `compute_all` runs once per
    // swap regardless (recomputing every file's diagnostics from scratch is
    // this module's own documented design, see the module doc), so caching
    // ACROSS calls would require snapshot-level storage this fix explicitly
    // does not add.
    let mut line_tables: HashMap<&str, LineTable<'_>> = HashMap::new();

    for (virtual_path, decls) in &snap.decls_by_file {
        let Some(entry) = snap.parsed.get(virtual_path) else {
            continue;
        };
        let uri = workspace_uri(snap, virtual_path);
        let table = line_tables
            .entry(virtual_path.as_str())
            .or_insert_with(|| LineTable::new(&entry.text));

        for decl in decls.iter() {
            let Some(routine) = find_routine_by_origin(&entry.file, decl.origin.byte.start) else {
                continue;
            };

            // Computed ONCE per declaration and reused below — a t3
            // whole-branch review found this call duplicated (once inside
            // `is_unused_procedure`, once here) on EVERY declaration, and the
            // SECOND call site was not even gated behind
            // `cfg.unused_procedures`, so disabling that rule didn't avoid
            // paying for it. Doubling the cost of an already-hot per-decl
            // call was small next to the O(event_edges) scan
            // `effective_incoming_count` used to do internally (see that
            // function's own doc for the real fix), but halving it here is
            // still a real, free win now that the call itself is O(1).
            let incoming_count = effective_incoming_count(snap, &decl.id);

            if cfg.unused_procedures && is_unused_procedure(decl, routine, incoming_count) {
                out.entry(uri.clone())
                    .or_default()
                    .push(unused_procedure_diagnostic(snap, decl, table, enc));
            }

            let complexity = crate::analysis::routine_complexity_ir(&entry.file.ir, routine);
            let parameter_count = parameter_count_of(routine);
            let line_count = decl.origin.end.row.saturating_sub(decl.origin.start.row) + 1;

            push_quality_diagnostics(
                out.entry(uri.clone()).or_default(),
                snap,
                decl,
                table,
                enc,
                complexity,
                parameter_count,
                line_count,
                incoming_count,
                cfg,
            );
        }
    }

    for diags in out.values_mut() {
        diags.sort_by_key(diagnostic_sort_key);
    }
    out
}

fn workspace_uri(snap: &LspSnapshot, virtual_path: &str) -> String {
    path_to_uri(&snap.workspace_root.join(virtual_path))
        .as_str()
        .to_string()
}

fn diagnostic_sort_key(d: &Diagnostic) -> (u32, u32, String) {
    (
        d.range.start.line,
        d.range.start.character,
        d.code
            .as_ref()
            .map(|c| match c {
                NumberOrString::String(s) => s.clone(),
                NumberOrString::Number(n) => n.to_string(),
            })
            .unwrap_or_default(),
    )
}

// ---------------------------------------------------------------------------
// Unused-procedure rule (see the module doc's rule-inventory table)
// ---------------------------------------------------------------------------

fn is_unused_procedure(
    decl: &DeclEntry,
    routine: &al_syntax::ir::RoutineDecl,
    incoming_count: usize,
) -> bool {
    // R1: only Procedure-kind routines are eligible; a Trigger is always excluded.
    if routine.kind != RoutineKind::Procedure {
        return false;
    }
    // R6: an interface method is a pure signature — never itself callable.
    // Dispatch through an interface-typed variable always routes to an
    // IMPLEMENTING object's routine (a distinct RoutineNodeId keyed to that
    // object, never to the interface's own), so the interface's own
    // declaration can NEVER receive an incoming edge under this model no
    // matter how many real call sites exist — a systematic false positive
    // BOTH engines shared (review fix-wave finding; legacy had the identical
    // false-positive class since `get_unused_procedures` never special-cased
    // Interface objects either). The IMPLEMENTING codeunit's own routine is
    // NOT covered by this rule — it stays subject to every rule below.
    if decl.id.object.kind == ObjectKind::Interface {
        return false;
    }
    // R4: public event publishers (Integration/Business) are ALWAYS excluded,
    // independent of incoming-edge evidence — their real subscribers
    // typically live in a downstream app this workspace never loads.
    if matches!(
        is_event_publisher(routine),
        Some(PublisherKind::Integration) | Some(PublisherKind::Business)
    ) {
        return false;
    }
    // R3: framework-invoked test methods/handlers.
    if routine
        .attributes
        .iter()
        .any(|a| crate::analysis::is_framework_invocation_attribute(a))
    {
        return false;
    }
    // R2 (subscriber "used" via a real EventFlow edge) + R5 (InternalEvent
    // flagged unless subscribed or raised) + every ordinary procedure all
    // fall through to the SAME zero-incoming check — `incoming_count` is the
    // CALLER's already-computed `effective_incoming_count(snap, &decl.id)`,
    // never recomputed here (t3 whole-branch review: this was previously a
    // SECOND `effective_incoming_count` call per declaration, doubling an
    // already-hot per-decl cost — see `compute_all`'s own doc).
    incoming_count == 0
}

/// Byte-for-byte legacy message/code/severity/tags
/// (`src/handlers.rs:600-635`, `get_unused_procedure_diagnostics`).
fn unused_procedure_diagnostic(
    snap: &LspSnapshot,
    decl: &DeclEntry,
    table: &LineTable<'_>,
    enc: PositionEncoding,
) -> Diagnostic {
    let object_name = object_name_for(&snap.graph, &decl.id.object).unwrap_or("Unknown");
    Diagnostic {
        range: origin_to_range(&decl.origin, table, enc),
        severity: Some(DiagnosticSeverity::HINT),
        code: Some(NumberOrString::String("unused-procedure".to_string())),
        source: Some("al-call-hierarchy".to_string()),
        message: format!("Procedure '{object_name}.{}' is never called", decl.name),
        related_information: None,
        tags: Some(vec![DiagnosticTag::UNNECESSARY]),
        code_description: None,
        data: None,
    }
}

// ---------------------------------------------------------------------------
// Code-quality diagnostics — byte-for-byte legacy port of
// `src/server.rs:353-506`'s `get_code_quality_diagnostics`.
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn push_quality_diagnostics(
    out: &mut Vec<Diagnostic>,
    snap: &LspSnapshot,
    decl: &DeclEntry,
    table: &LineTable<'_>,
    enc: PositionEncoding,
    complexity: u32,
    parameter_count: u32,
    line_count: u32,
    incoming_count: usize,
    cfg: &DiagnosticConfig,
) {
    let object_name = object_name_for(&snap.graph, &decl.id.object).unwrap_or("Unknown");
    let range = origin_to_range(&decl.origin, table, enc);
    let proc = decl.name.as_str();

    let plain = |code: &str, message: String, severity: DiagnosticSeverity| Diagnostic {
        range,
        severity: Some(severity),
        code: Some(NumberOrString::String(code.to_string())),
        source: Some("al-call-hierarchy".to_string()),
        message,
        related_information: None,
        tags: None,
        code_description: None,
        data: None,
    };

    if cfg.complexity_enabled && complexity >= cfg.complexity_critical {
        out.push(plain(
            "high-complexity",
            format!(
                "Procedure '{object_name}.{proc}' has cyclomatic complexity {complexity} (critical threshold: {}) - consider simplifying",
                cfg.complexity_critical
            ),
            DiagnosticSeverity::WARNING,
        ));
    } else if cfg.complexity_enabled && complexity >= cfg.complexity_warning {
        out.push(plain(
            "high-complexity",
            format!(
                "Procedure '{object_name}.{proc}' has cyclomatic complexity {complexity} (warning threshold: {})",
                cfg.complexity_warning
            ),
            DiagnosticSeverity::INFORMATION,
        ));
    }

    if cfg.params_enabled && parameter_count >= cfg.params_critical {
        out.push(plain(
            "too-many-parameters",
            format!(
                "Procedure '{object_name}.{proc}' has {parameter_count} parameters (critical threshold: {}) - consider using a record or reducing parameters",
                cfg.params_critical
            ),
            DiagnosticSeverity::WARNING,
        ));
    } else if cfg.params_enabled && parameter_count >= cfg.params_warning {
        out.push(plain(
            "too-many-parameters",
            format!(
                "Procedure '{object_name}.{proc}' has {parameter_count} parameters (warning threshold: {})",
                cfg.params_warning
            ),
            DiagnosticSeverity::INFORMATION,
        ));
    }

    if cfg.fan_in_enabled && incoming_count > cfg.fan_in_warning {
        out.push(plain(
            "high-fan-in",
            format!(
                "Procedure '{object_name}.{proc}' has {incoming_count} callers - consider if it's doing too much"
            ),
            DiagnosticSeverity::INFORMATION,
        ));
    }

    if cfg.length_enabled && line_count > cfg.length_critical {
        out.push(plain(
            "long-method",
            format!(
                "Procedure '{object_name}.{proc}' spans {line_count} lines - consider breaking it down"
            ),
            DiagnosticSeverity::INFORMATION,
        ));
    }
}

// ---------------------------------------------------------------------------
// DiagnosticsState — the diff half of recompute-diff-publish-clear
// ---------------------------------------------------------------------------

/// Tracks the last-published diagnostic set per uri so [`Self::diff`] can
/// emit only what changed — including clearing a uri whose diagnostics
/// dropped to zero (legacy never did this; see the module doc).
#[derive(Debug, Default)]
pub struct DiagnosticsState {
    last_published: HashMap<String, Vec<Diagnostic>>,
}

impl DiagnosticsState {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Diff `new` (a fresh [`compute_all`] result) against what this state
    /// last published. Returns `(uri, diagnostics)` pairs to actually send,
    /// sorted by uri for deterministic output:
    ///
    /// - A uri whose diagnostics CHANGED (added, removed, or edited) from the
    ///   last-published set is included with its NEW (possibly empty) vec.
    /// - A uri that was published before but is ABSENT from `new` entirely
    ///   (e.g. the file left the snapshot) is included with an empty vec —
    ///   the same clear behavior as a uri present-but-emptied.
    /// - An UNCHANGED uri (byte-identical diagnostics, including the
    ///   unchanged-empty case) is omitted — no redundant re-publish.
    pub fn diff(
        &mut self,
        new: HashMap<String, Vec<Diagnostic>>,
    ) -> Vec<(String, Vec<Diagnostic>)> {
        let mut out = Vec::new();
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

        let mut entries: Vec<(String, Vec<Diagnostic>)> = new.into_iter().collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));

        for (uri, diags) in entries {
            seen.insert(uri.clone());
            let changed = self.last_published.get(&uri) != Some(&diags);
            if changed {
                out.push((uri.clone(), diags.clone()));
            }
            if diags.is_empty() {
                self.last_published.remove(&uri);
            } else {
                self.last_published.insert(uri, diags);
            }
        }

        let mut stale: Vec<String> = self
            .last_published
            .keys()
            .filter(|u| !seen.contains(*u))
            .cloned()
            .collect();
        stale.sort();
        for uri in stale {
            self.last_published.remove(&uri);
            out.push((uri, Vec::new()));
        }

        out
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use lsp_types::Range;

    fn write_app(dir: &std::path::Path, id: &str, name: &str) {
        std::fs::write(
            dir.join("app.json"),
            format!(r#"{{"id":"{id}","name":"{name}","publisher":"probe","version":"1.0.0.0"}}"#),
        )
        .expect("write app.json");
    }

    fn build(dir: &std::path::Path) -> LspSnapshot {
        LspSnapshot::build_full(dir).expect("build_full")
    }

    fn diagnostics_for(snap: &LspSnapshot, cfg: &DiagnosticConfig, file: &str) -> Vec<Diagnostic> {
        let all = compute_all(snap, PositionEncoding::Utf16, cfg);
        let uri = workspace_uri(snap, file);
        all.get(&uri).cloned().unwrap_or_default()
    }

    fn codes_of(diags: &[Diagnostic]) -> Vec<String> {
        diags
            .iter()
            .filter_map(|d| d.code.as_ref())
            .map(|c| match c {
                NumberOrString::String(s) => s.clone(),
                NumberOrString::Number(n) => n.to_string(),
            })
            .collect()
    }

    // ── R1: a trigger is never eligible for unused-procedure, even orphaned ─

    #[test]
    fn unused_rule_r1_trigger_never_flagged() {
        let dir = tempfile::tempdir().unwrap();
        write_app(dir.path(), "10000000-0000-0000-0000-000000000001", "R1");
        std::fs::write(
            dir.path().join("Cu.al"),
            r#"codeunit 50100 "Cu"
{
    trigger OnRun()
    begin
    end;
}
"#,
        )
        .unwrap();
        let snap = build(dir.path());
        let cfg = DiagnosticConfig::default();
        let diags = diagnostics_for(&snap, &cfg, "Cu.al");
        assert!(
            !codes_of(&diags).contains(&"unused-procedure".to_string()),
            "an orphaned trigger must never be flagged unused-procedure; got {diags:#?}"
        );
    }

    // ── R2: a well-formed [EventSubscriber] is used via a real EventFlow edge ─

    #[test]
    fn unused_rule_r2_event_subscriber_wired_to_real_publisher_not_flagged() {
        let dir = tempfile::tempdir().unwrap();
        write_app(dir.path(), "10000000-0000-0000-0000-000000000002", "R2");
        std::fs::write(
            dir.path().join("Publisher.al"),
            r#"codeunit 50100 "Publisher"
{
    // Real BC requires the subscribed-to procedure to itself carry a
    // publisher attribute — [InternalEvent] here (not Integration/Business)
    // so this test proves R2's incoming-based mechanism specifically,
    // undiluted by R4's separate blanket exclusion.
    [InternalEvent(false)]
    procedure OnBeforePost()
    begin
    end;
}
"#,
        )
        .unwrap();
        std::fs::write(
            dir.path().join("Subscriber.al"),
            r#"codeunit 50101 "Subscriber"
{
    [EventSubscriber(ObjectType::Codeunit, Codeunit::Publisher, 'OnBeforePost', '', false, false)]
    local procedure HandleOnBeforePost()
    begin
    end;

    procedure PlainUnused()
    begin
    end;
}
"#,
        )
        .unwrap();
        let snap = build(dir.path());
        let cfg = DiagnosticConfig::default();
        let diags = diagnostics_for(&snap, &cfg, "Subscriber.al");
        let unused_names: Vec<&str> = diags
            .iter()
            .filter(|d| d.code == Some(NumberOrString::String("unused-procedure".to_string())))
            .map(|d| d.message.as_str())
            .collect();
        assert!(
            !unused_names
                .iter()
                .any(|m| m.contains("HandleOnBeforePost")),
            "a subscriber wired to a real publisher must not be flagged; got {unused_names:?}"
        );
        // Guard against over-exclusion: a genuinely unused plain procedure in
        // the SAME file is still flagged.
        assert!(
            unused_names.iter().any(|m| m.contains("PlainUnused")),
            "a genuinely unused plain procedure must still be flagged; got {unused_names:?}"
        );
    }

    // ── R2 negative: a BROKEN/misdirected [EventSubscriber] IS flagged ─────
    // (review fix-wave hunt-4 gap: the highest-risk semantic claim in this
    // module — "engine mechanism, not blanket exclusion" — is untested
    // without this half. A subscriber whose attribute names an event that
    // does not exist gets NO EventFlow edge at all, so it must fall through
    // to the ordinary zero-incoming check exactly like any orphaned plain
    // procedure would.)

    #[test]
    fn unused_rule_r2_negative_misdirected_event_subscriber_is_flagged() {
        let dir = tempfile::tempdir().unwrap();
        write_app(dir.path(), "10000000-0000-0000-0000-00000000000c", "R2neg");
        std::fs::write(
            dir.path().join("Publisher.al"),
            r#"codeunit 50100 "Publisher"
{
    [InternalEvent(false)]
    procedure OnRealEvent()
    begin
    end;
}
"#,
        )
        .unwrap();
        std::fs::write(
            dir.path().join("Subscriber.al"),
            r#"codeunit 50101 "Subscriber"
{
    [EventSubscriber(ObjectType::Codeunit, Codeunit::Publisher, 'ThisEventDoesNotExist', '', false, false)]
    local procedure HandleNonExistentEvent()
    begin
    end;
}
"#,
        )
        .unwrap();
        let snap = build(dir.path());
        let cfg = DiagnosticConfig::default();
        let diags = diagnostics_for(&snap, &cfg, "Subscriber.al");
        let unused_names: Vec<&str> = diags
            .iter()
            .filter(|d| d.code == Some(NumberOrString::String("unused-procedure".to_string())))
            .map(|d| d.message.as_str())
            .collect();
        assert!(
            unused_names
                .iter()
                .any(|m| m.contains("HandleNonExistentEvent")),
            "a subscriber pointing at a NONEXISTENT event must be flagged \
             unused — no EventFlow edge can ever resolve for it, unlike \
             legacy's blanket [EventSubscriber] attribute exclusion which \
             could never catch this; got {unused_names:?}"
        );
    }

    // ── R3: [Test]/handler-attributed procedures are never flagged ─────────

    #[test]
    fn unused_rule_r3_test_and_handler_attributes_not_flagged() {
        let dir = tempfile::tempdir().unwrap();
        write_app(dir.path(), "10000000-0000-0000-0000-000000000003", "R3");
        std::fs::write(
            dir.path().join("Tests.al"),
            r#"codeunit 50200 "Tests"
{
    Subtype = Test;

    [Test]
    procedure MyTest()
    begin
    end;

    [ConfirmHandler]
    procedure MyConfirm(Question: Text; var Reply: Boolean)
    begin
    end;

    [MessageHandler]
    procedure MyMessage(Msg: Text)
    begin
    end;

    procedure PlainUnused()
    begin
    end;
}
"#,
        )
        .unwrap();
        let snap = build(dir.path());
        let cfg = DiagnosticConfig::default();
        let diags = diagnostics_for(&snap, &cfg, "Tests.al");
        let unused_names: Vec<&str> = diags
            .iter()
            .filter(|d| d.code == Some(NumberOrString::String("unused-procedure".to_string())))
            .map(|d| d.message.as_str())
            .collect();
        for h in ["MyTest", "MyConfirm", "MyMessage"] {
            assert!(
                !unused_names.iter().any(|m| m.contains(h)),
                "{h} must not be flagged unused; got {unused_names:?}"
            );
        }
        assert!(unused_names.iter().any(|m| m.contains("PlainUnused")));
    }

    // ── R4: public event publishers (Integration/Business) never flagged ──

    #[test]
    fn unused_rule_r4_public_event_publishers_not_flagged() {
        let dir = tempfile::tempdir().unwrap();
        write_app(dir.path(), "10000000-0000-0000-0000-000000000004", "R4");
        std::fs::write(
            dir.path().join("Publisher.al"),
            r#"codeunit 50100 "Publisher"
{
    [IntegrationEvent(false, false)]
    procedure OnAfterIntegration()
    begin
    end;

    [BusinessEvent(false)]
    procedure OnAfterBusiness()
    begin
    end;
}
"#,
        )
        .unwrap();
        let snap = build(dir.path());
        let cfg = DiagnosticConfig::default();
        let diags = diagnostics_for(&snap, &cfg, "Publisher.al");
        let unused_names: Vec<&str> = diags
            .iter()
            .filter(|d| d.code == Some(NumberOrString::String("unused-procedure".to_string())))
            .map(|d| d.message.as_str())
            .collect();
        for p in ["OnAfterIntegration", "OnAfterBusiness"] {
            assert!(
                !unused_names.iter().any(|m| m.contains(p)),
                "{p} must never be flagged; got {unused_names:?}"
            );
        }
    }

    // ── R5: an orphan InternalEvent IS flagged (no auto-exclusion) ─────────

    #[test]
    fn unused_rule_r5_orphan_internal_event_is_flagged() {
        let dir = tempfile::tempdir().unwrap();
        write_app(dir.path(), "10000000-0000-0000-0000-000000000005", "R5a");
        std::fs::write(
            dir.path().join("Publisher.al"),
            r#"codeunit 50100 "Publisher"
{
    [InternalEvent(false)]
    procedure OnNobodyListens()
    begin
    end;
}
"#,
        )
        .unwrap();
        let snap = build(dir.path());
        let cfg = DiagnosticConfig::default();
        let diags = diagnostics_for(&snap, &cfg, "Publisher.al");
        let unused_names: Vec<&str> = diags
            .iter()
            .filter(|d| d.code == Some(NumberOrString::String("unused-procedure".to_string())))
            .map(|d| d.message.as_str())
            .collect();
        assert!(
            unused_names.iter().any(|m| m.contains("OnNobodyListens")),
            "an orphan InternalEvent must be flagged; got {unused_names:?}"
        );
    }

    // ── R5: subscribed-or-raised InternalEvent is NOT flagged ──────────────

    #[test]
    fn unused_rule_r5_subscribed_or_raised_internal_event_not_flagged() {
        let dir = tempfile::tempdir().unwrap();
        write_app(dir.path(), "10000000-0000-0000-0000-000000000006", "R5b");
        std::fs::write(
            dir.path().join("Publisher.al"),
            r#"codeunit 50100 "Publisher"
{
    [InternalEvent(false)]
    procedure OnSubscribed()
    begin
    end;

    [InternalEvent(false)]
    procedure OnRaised()
    begin
    end;

    procedure Raise()
    begin
        OnRaised();
    end;
}
"#,
        )
        .unwrap();
        std::fs::write(
            dir.path().join("Subscriber.al"),
            r#"codeunit 50101 "Subscriber"
{
    [EventSubscriber(ObjectType::Codeunit, Codeunit::Publisher, 'OnSubscribed', '', false, false)]
    local procedure HandleOnSubscribed()
    begin
    end;
}
"#,
        )
        .unwrap();
        let snap = build(dir.path());
        let cfg = DiagnosticConfig::default();
        let diags = diagnostics_for(&snap, &cfg, "Publisher.al");
        let unused_names: Vec<&str> = diags
            .iter()
            .filter(|d| d.code == Some(NumberOrString::String("unused-procedure".to_string())))
            .map(|d| d.message.as_str())
            .collect();
        assert!(
            !unused_names.iter().any(|m| m.contains("OnSubscribed")),
            "an InternalEvent with a real subscriber must not be flagged; got {unused_names:?}"
        );
        assert!(
            !unused_names.iter().any(|m| m.contains("OnRaised")),
            "a raised InternalEvent must not be flagged; got {unused_names:?}"
        );
    }

    // ── R6: an interface method's signature is never flagged; the ─────────
    // ── implementing codeunit's routine stays subject to normal rules ─────

    #[test]
    fn unused_rule_r6_interface_signature_never_flagged_but_implementation_still_checked() {
        let dir = tempfile::tempdir().unwrap();
        write_app(dir.path(), "10000000-0000-0000-0000-00000000000d", "R6");
        std::fs::write(
            dir.path().join("IFoo.al"),
            r#"interface "IFoo"
{
    procedure DoSomething();
}
"#,
        )
        .unwrap();
        std::fs::write(
            dir.path().join("Impl.al"),
            r#"codeunit 50100 "Impl" implements "IFoo"
{
    procedure DoSomething()
    begin
    end;

    procedure PlainUnused()
    begin
    end;
}
"#,
        )
        .unwrap();
        let snap = build(dir.path());
        let cfg = DiagnosticConfig::default();

        let iface_diags = diagnostics_for(&snap, &cfg, "IFoo.al");
        assert!(
            codes_of(&iface_diags)
                .iter()
                .all(|c| c != "unused-procedure"),
            "an interface method signature must never be flagged unused; got {iface_diags:#?}"
        );

        let impl_diags = diagnostics_for(&snap, &cfg, "Impl.al");
        let unused_names: Vec<&str> = impl_diags
            .iter()
            .filter(|d| d.code == Some(NumberOrString::String("unused-procedure".to_string())))
            .map(|d| d.message.as_str())
            .collect();
        assert!(
            unused_names.iter().any(|m| m.contains("DoSomething")),
            "the IMPLEMENTING codeunit's routine is still subject to normal \
             rules (nothing calls it in this fixture); got {unused_names:?}"
        );
        assert!(
            unused_names.iter().any(|m| m.contains("PlainUnused")),
            "guard against over-exclusion of the whole implementing object; \
             got {unused_names:?}"
        );
    }

    // ── quality diagnostics: codes/severities/thresholds ───────────────────

    #[test]
    fn quality_diagnostics_fire_at_the_expected_thresholds() {
        let dir = tempfile::tempdir().unwrap();
        write_app(dir.path(), "10000000-0000-0000-0000-000000000007", "Q");
        std::fs::write(
            dir.path().join("Cu.al"),
            r#"codeunit 50100 "Cu"
{
    procedure Complex(A: Integer; B: Integer; C: Integer; D: Integer)
    begin
        if A > 0 then begin
            if B > 0 then begin
                if C > 0 then begin
                    if D > 0 then begin
                    end;
                end;
            end;
        end;
    end;
}
"#,
        )
        .unwrap();
        let snap = build(dir.path());
        let cfg = DiagnosticConfig {
            complexity_critical: 3, // Complex's complexity is 5 (base 1 + 4 nested ifs).
            params_warning: 2,
            params_critical: 10,
            length_critical: 1, // any multi-line body trips this.
            fan_in_warning: 0,  // 0 callers > 0 is false, so no fan-in fires here — sanity only.
            ..DiagnosticConfig::default()
        };

        let diags = diagnostics_for(&snap, &cfg, "Cu.al");
        let codes = codes_of(&diags);
        assert!(codes.contains(&"high-complexity".to_string()), "{codes:?}");
        assert!(
            codes.contains(&"too-many-parameters".to_string()),
            "{codes:?}"
        );
        assert!(codes.contains(&"long-method".to_string()), "{codes:?}");

        let complexity_diag = diags
            .iter()
            .find(|d| d.code == Some(NumberOrString::String("high-complexity".to_string())))
            .unwrap();
        assert_eq!(complexity_diag.severity, Some(DiagnosticSeverity::WARNING));
        assert!(complexity_diag.message.contains("critical threshold: 3"));
    }

    #[test]
    fn quality_diagnostics_high_fan_in_fires_past_warning_threshold() {
        let dir = tempfile::tempdir().unwrap();
        write_app(dir.path(), "10000000-0000-0000-0000-000000000008", "F");
        std::fs::write(
            dir.path().join("Cu.al"),
            r#"codeunit 50100 "Cu"
{
    procedure Callee()
    begin
    end;

    procedure Caller1()
    begin
        Callee();
    end;

    procedure Caller2()
    begin
        Callee();
    end;
}
"#,
        )
        .unwrap();
        let snap = build(dir.path());
        let cfg = DiagnosticConfig {
            fan_in_warning: 1, // Callee has 2 callers > 1.
            ..DiagnosticConfig::default()
        };

        let diags = diagnostics_for(&snap, &cfg, "Cu.al");
        let fan_in = diags
            .iter()
            .find(|d| d.code == Some(NumberOrString::String("high-fan-in".to_string())));
        assert!(fan_in.is_some(), "{diags:#?}");
        assert!(fan_in.unwrap().message.contains("2 callers"));
    }

    // ── compute_all includes now-empty URIs (every parsed file) ────────────

    #[test]
    fn compute_all_includes_a_clean_files_uri_with_an_empty_vec() {
        let dir = tempfile::tempdir().unwrap();
        write_app(dir.path(), "10000000-0000-0000-0000-000000000009", "Clean");
        std::fs::write(
            dir.path().join("Clean.al"),
            r#"codeunit 50100 "Clean"
{
    procedure OnRun()
    begin
        Message('used elsewhere conceptually');
    end;
}
"#,
        )
        .unwrap();
        let snap = build(dir.path());
        let cfg = DiagnosticConfig {
            unused_procedures: false,
            complexity_enabled: false,
            params_enabled: false,
            fan_in_enabled: false,
            length_enabled: false,
            ..DiagnosticConfig::default()
        };

        let all = compute_all(&snap, PositionEncoding::Utf16, &cfg);
        let uri = workspace_uri(&snap, "Clean.al");
        assert!(
            all.contains_key(&uri),
            "a parsed file with zero findings must still get an entry; got keys {:?}",
            all.keys().collect::<Vec<_>>()
        );
        assert!(all[&uri].is_empty());
    }

    // ── DiagnosticsState::diff ──────────────────────────────────────────────

    #[test]
    fn diff_publishes_new_findings_then_clears_when_they_disappear() {
        let mut state = DiagnosticsState::new();
        let diag = Diagnostic {
            range: Range::default(),
            severity: Some(DiagnosticSeverity::HINT),
            code: Some(NumberOrString::String("unused-procedure".to_string())),
            source: Some("al-call-hierarchy".to_string()),
            message: "Procedure 'Cu.Foo' is never called".to_string(),
            related_information: None,
            tags: Some(vec![DiagnosticTag::UNNECESSARY]),
            code_description: None,
            data: None,
        };

        let mut first = HashMap::new();
        first.insert("file:///Cu.al".to_string(), vec![diag.clone()]);
        let publish1 = state.diff(first);
        assert_eq!(publish1, vec![("file:///Cu.al".to_string(), vec![diag])]);

        // The SAME set again must produce no re-publish (unchanged).
        let mut same = HashMap::new();
        same.insert("file:///Cu.al".to_string(), publish1[0].1.clone());
        assert!(
            state.diff(same).is_empty(),
            "an unchanged diagnostic set must not be re-published"
        );

        // The finding disappears — THE missing legacy behavior: must appear
        // in the diff with an empty vec (a clear).
        let mut second = HashMap::new();
        second.insert("file:///Cu.al".to_string(), Vec::new());
        let publish2 = state.diff(second);
        assert_eq!(
            publish2,
            vec![("file:///Cu.al".to_string(), Vec::new())],
            "a uri whose findings dropped to zero must be included as a clear"
        );
    }

    #[test]
    fn diff_clears_a_uri_missing_entirely_from_the_new_set() {
        let mut state = DiagnosticsState::new();
        let diag = Diagnostic {
            range: Range::default(),
            severity: Some(DiagnosticSeverity::HINT),
            code: Some(NumberOrString::String("unused-procedure".to_string())),
            source: Some("al-call-hierarchy".to_string()),
            message: "msg".to_string(),
            related_information: None,
            tags: None,
            code_description: None,
            data: None,
        };
        let mut first = HashMap::new();
        first.insert("file:///Gone.al".to_string(), vec![diag]);
        state.diff(first);

        // The file vanished from the snapshot entirely (not even an empty
        // entry) — must still be cleared.
        let publish = state.diff(HashMap::new());
        assert_eq!(publish, vec![("file:///Gone.al".to_string(), Vec::new())]);
    }

    #[test]
    fn diff_output_is_sorted_by_uri() {
        let mut state = DiagnosticsState::new();
        let mk = |msg: &str| Diagnostic {
            range: Range::default(),
            severity: Some(DiagnosticSeverity::HINT),
            code: None,
            source: None,
            message: msg.to_string(),
            related_information: None,
            tags: None,
            code_description: None,
            data: None,
        };
        let mut new = HashMap::new();
        new.insert("file:///Z.al".to_string(), vec![mk("z")]);
        new.insert("file:///A.al".to_string(), vec![mk("a")]);
        new.insert("file:///M.al".to_string(), vec![mk("m")]);

        let out = state.diff(new);
        let uris: Vec<&str> = out.iter().map(|(u, _)| u.as_str()).collect();
        assert_eq!(uris, vec!["file:///A.al", "file:///M.al", "file:///Z.al"]);
    }
}
