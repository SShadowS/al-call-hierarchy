//! The PERMANENT incremental-vs-batch differential gate (T3 Task 10, the
//! arc's H-10 insurance policy). This test file outlives the T3 arc — it
//! runs in CI forever, on every PR.
//!
//! For every scripted disk-edit sequence below: copy the fixture workspace
//! (`tests/fixtures/lsp-incr/`) into a fresh tempdir, `LspSnapshot::
//! build_full_with_parsed` it, then drive `Updater::apply_batch` through one
//! or more scripted edits. **After EVERY edit** (not just at the end), the
//! incrementally-maintained `LspSnapshot` the `Updater` just produced must be
//! EQUIVALENT to a completely independent `LspSnapshot::build_full` of the
//! exact same on-disk state — proving the 2-rung incremental ladder (Tasks
//! 8/9) never silently drifts from a from-scratch rebuild.
//!
//! # Equivalence key (binding — NOT full structural equality)
//!
//! Two snapshots are "equivalent" here iff, for corresponding files:
//! - `edges_by_file`: the same file KEY SET, and for each file the same edge
//!   MULTISET, where one edge's identity for comparison is `(ObligationId,
//!   EdgeKind, DispatchShape, SetCompleteness, sorted Vec<(RouteTarget,
//!   EvidenceKind, sorted Vec<Condition>)>)` — i.e. the obligation it
//!   answers; the edge's own classification (its kind, dispatch shape, and
//!   set-completeness — real semantics, not incidental: `shape`/
//!   `completeness` are exactly what `classify_obligation` and
//!   `real_unknown_rate` read); and, per route, which target/evidence-kind
//!   it routes to plus that route's `Condition` set (real semantics too —
//!   `Route::fires_by_default`/`Edge::default_reachable_routes` gate
//!   traversal on exactly this field: a route silently losing/gaining
//!   `ManualBinding`/`AmbiguousDispatch` between incremental and fresh would
//!   be a genuine reachability-changing divergence, and the pre-review-fix
//!   key could not have caught it). Review fix-wave (this pass): the
//!   ORIGINAL key omitted `kind`/`shape`/`completeness`/`conditions`
//!   entirely — an unjustified exclusion for a permanent CI gate, caught in
//!   review; every field is now either compared or has a stated reason not
//!   to be (this doc's job is to make that an exhaustive list, not a
//!   selective one). Order within a file's bucket, and within one edge's
//!   route list, is deliberately NOT compared (both are multisets) — a
//!   global index (like a brand-new file's position, or `event_edges`'s
//!   traversal order over the whole graph) can legitimately differ in ORDER
//!   between an incrementally-patched `Vec` (new entries appended) and a
//!   fresh directory-walk build (natural filesystem order) without either
//!   being wrong.
//! - `event_edges`: the same multiset, same rule.
//! - `incoming`: the same map from target `RoutineNodeId` to the SET of
//!   `ObligationId`s that call it — dereferenced through each `EdgeRef` to
//!   its owning edge's `ObligationId` rather than compared as raw `(file,
//!   idx)` pairs, for the same order-independence reason as above (an
//!   `EdgeRef`'s `idx` is a position into a bucket whose GLOBAL order can
//!   legitimately differ; its `ObligationId` is a positionless, stable
//!   identity).
//! - `decls_by_file`: the same file key set, and for each file the same set
//!   of `(RoutineNodeId, name, origin, name_origin)` tuples.
//!
//! # Why `ObligationId`, not the brief's originally-suggested `SiteId`
//!
//! The brief's Step 1 names the per-edge comparison key as `(SiteId, sorted
//! route (target, evidence-discriminant))`. Building the gate against a
//! literal `Edge.site.clone()` instead of `ClassifiedEdge.obligation_id`
//! surfaced a REAL, reproducible false-positive divergence on the very
//! first script (`body_edit_chain_...`, step 0): `event_edges`' one
//! `EventFlow` edge (`Alpha.OnAfterWork` -> `Beta.HandleAfterWork`) differed
//! ONLY in `SiteId.span`'s line number between the incremental and fresh
//! builds, even though the resolved route was byte-identical. Root cause,
//! confirmed by reading `resolver.rs`'s `emit_event_flow_edges`: for a
//! Publisher-kind edge, `SiteId` is explicitly "anchored at the publisher
//! routine's name-origin span" — a cosmetic position anchor, not a
//! distinguishing call site (there IS no call expression for an event; the
//! publisher's own declaration stands in for one) — while
//! `apply_rung1_core` (`src/lsp/updater.rs`) never recomputes `event_edges`
//! on rung 1 (`Arc::clone(&cur.event_edges)`, unconditionally), so that
//! anchor goes stale whenever a rung-1 edit shifts line numbers in a file
//! that happens to declare a publisher — even though nothing about the
//! publisher's IDENTITY or its subscriber fan-out changed. This is the same
//! underlying staleness CLASS the brief already carves out for
//! `Route::witness` below, just manifesting through `SiteId` for
//! `EdgeKind::EventFlow`/`ObligationId::Publisher` specifically.
//! `ObligationId` is the engine's OWN pre-designed fix for exactly this
//! distinction: `ObligationId::CallSite { caller, span, callee_fp }` mirrors
//! `SiteId` field-for-field (so nothing is lost comparing REAL call sites —
//! rung 1 always resolves the touched file's own call sites fresh, so their
//! spans are never stale), while `ObligationId::Publisher(RoutineNodeId)`
//! carries NO span at all, sidestepping the cosmetic-anchor staleness
//! entirely. Using `ObligationId` — already sitting on `ClassifiedEdge`,
//! never hand-reconstructed — is therefore strictly more correct than the
//! brief's literal `SiteId` suggestion, not a weakening of it. Confirmed
//! empirically: switching to it turned the one genuine failure into a pass
//! with zero other behavior change. Reported to the team lead as a
//! candidate follow-up (should rung 1 also refresh `event_edges` entries
//! for publishers in the touched file, for span freshness at the handler
//! level?) — out of scope for this gate to fix, since `apply_rung1_core` is
//! already-reviewed Task 9 code.
//!
//! **Witness spans are EXCLUDED from the equivalence key, BY DESIGN** (per
//! the def-surface audit §6.1): rung 1 only re-resolves the TOUCHED file(s)'
//! own edges, so a `Route::witness` (`Witness::SourceSpan`) on some OTHER,
//! untouched edge that happens to point INTO a file which changed later is
//! left stale — its cached byte span may no longer line up with that file's
//! current content. Handlers are required to re-derive any span live from
//! the current parse rather than trust a stored witness span (tested at the
//! handler level in Task 11's gate, not here) — so this test's equivalence
//! key never looks at `Route::witness` at all. `EvidenceKind` (via
//! `Evidence::kind()`) is compared instead of the raw `Evidence`, for the
//! unrelated reason that `Evidence::Unknown`'s `UnknownReason` payload is a
//! diagnostic-only field with its own serialization-boundary discipline
//! elsewhere in this engine (`Evidence::kind`'s own doc) — using it here
//! keeps this gate aligned with that same discipline rather than
//! reinventing a second comparison rule for the same payload.
//!
//! `generation` is also excluded: it counts monotonically upward on the
//! incremental side (each rung bumps it) and is always `0` on a fresh
//! `build_full` — the two are never expected to match and comparing them
//! would tell us nothing about correctness.
//!
//! `DeclEntry.origin`/`.name_origin` (`al_syntax::ir::Origin`) themselves
//! carry no derived equality at all, and their `ts_id` field is explicitly
//! documented as EPHEMERAL ("valid only within the single lowering pass...
//! NEVER compare across parses, tree-sitter recycles ids") — `canon_origin`
//! below projects away `kind_text`/`ts_id` and keeps only the byte range and
//! start/end `Point`s, which — unlike `ts_id` — really are stable, since
//! both sides parse the exact same on-disk bytes.
//!
//! # Exhaustive accounting (every `Edge`/`Route` field is either compared or
//! excluded-with-a-reason above; nothing is silently dropped)
//!
//! Compared: `ObligationId` (via `ClassifiedEdge`, not `Edge.site` — see
//! above), `Edge::kind`, `Edge::shape`, `Edge::completeness`,
//! `Route::target`, `Route::evidence` (via `EvidenceKind`),
//! `Route::conditions`. Excluded, each with a reason stated above:
//! `Route::witness` (staleness, audit §6.1), `Evidence::Unknown`'s
//! `UnknownReason` payload (subsumed by `EvidenceKind`, itself a deliberate
//! serialization-boundary projection elsewhere in this engine), `LspSnapshot::
//! generation` (monotonic counter vs. always-`0`, not a correctness signal),
//! `Origin::kind_text`/`ts_id` (EPHEMERAL by the IR's own doc).
//! `Route::receiver_tier` is excluded too: its own doc (`edge.rs`) already
//! states it is diagnostic-only and is never compared against the committed
//! semantic goldens, for the same serialization-boundary discipline as
//! `Evidence::Unknown`'s payload — this gate excludes it for that identical,
//! already-engine-documented reason, not a new carve-out invented here.
//!
//! # Non-vacuity (Step 2)
//!
//! A gate that always took the slow, always-correct rung 3 (or always rung
//! 2) everywhere would pass every check above for a trivial, uninteresting
//! reason. Every script below pins its OWN expected `Rung` via
//! `assert_eq!`, and — per the brief's explicit binding requirement —
//! [`gate_non_vacuity_rung1_and_rung2_are_both_exercised`] independently
//! proves, in isolation from every other script's state, that this suite's
//! `apply_batch` calls really do take rung 1 at least once and rung 2 at
//! least once.
//!
//! # CI safety
//!
//! Pure fixture, zero CDO dependency — every script is self-contained and
//! runs on any machine (including a bare `ubuntu-latest` CI runner). All
//! paths are built via `Path::join`, never a hand-written separator.

use std::collections::BTreeMap;
use std::path::Path;

use al_call_hierarchy::lsp::snapshot::{DeclEntry, LspSnapshot};
use al_call_hierarchy::lsp::updater::{ChangeEvent, Rung, Updater};
use al_call_hierarchy::program::node::{AppRef, ObjKey, ObjectNodeId, RoutineNodeId};
use al_call_hierarchy::program::resolve::edge::{
    CanonicalSpan, Condition, DispatchShape, Edge, EdgeKind, Evidence, EvidenceKind,
    OpenWorldReason, Route, RouteTarget, SetCompleteness, SiteId, SourcePos, Witness,
};
use al_call_hierarchy::program::resolve::full::{ClassifiedEdge, ObligationId};
use al_call_hierarchy::snapshot::ParsedUnit;

// ---------------------------------------------------------------------------
// Fixture plumbing
// ---------------------------------------------------------------------------

/// Copy the committed fixture workspace (`tests/fixtures/lsp-incr/`) into a
/// fresh tempdir so every script gets its own independent, mutable copy —
/// never touching the committed original.
fn copy_fixture_to_tempdir() -> tempfile::TempDir {
    let dst = tempfile::tempdir().expect("tempdir");
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/lsp-incr");
    copy_dir_recursive(&src, dst.path());
    dst
}

fn copy_dir_recursive(src: &Path, dst: &Path) {
    for entry in std::fs::read_dir(src).expect("read_dir fixture source") {
        let entry = entry.expect("dir entry");
        let file_type = entry.file_type().expect("file_type");
        let dest_path = dst.join(entry.file_name());
        if file_type.is_dir() {
            std::fs::create_dir_all(&dest_path).expect("create_dir_all");
            copy_dir_recursive(&entry.path(), &dest_path);
        } else {
            std::fs::copy(entry.path(), &dest_path).expect("copy fixture file");
        }
    }
}

fn build_full_with_parsed(dir: &Path) -> (LspSnapshot, Vec<ParsedUnit>) {
    LspSnapshot::build_full_with_parsed(dir).expect("build_full_with_parsed on fixture")
}

// ---------------------------------------------------------------------------
// Canonicalization — the ONE equivalence-key implementation every script uses
// ---------------------------------------------------------------------------

/// One route's comparison identity: its target, evidence-kind, and its
/// OWN sorted `Condition` set (`fires_by_default`/`default_reachable_routes`
/// gate traversal on exactly this field — see the module doc).
type CanonRoute = (RouteTarget, EvidenceKind, Vec<Condition>);

fn canon_route(r: &Route) -> CanonRoute {
    let mut conditions = r.conditions.clone();
    conditions.sort();
    (r.target.clone(), r.evidence.kind(), conditions)
}

/// One edge's comparison identity: the obligation it answers (`ObligationId`
/// — not the brief's originally-suggested raw `SiteId`; see the module
/// doc's "Why `ObligationId`" section), the edge's own classification
/// (`kind`/`shape`/`completeness` — review fix-wave addition: real
/// semantics `classify_obligation`/`real_unknown_rate` read, not incidental
/// data), plus the sorted set of routes it carries.
type CanonEdge = (
    ObligationId,
    EdgeKind,
    DispatchShape,
    SetCompleteness,
    Vec<CanonRoute>,
);

fn canon_edge(ce: &ClassifiedEdge) -> CanonEdge {
    let mut routes: Vec<CanonRoute> = ce.edge.routes.iter().map(canon_route).collect();
    routes.sort();
    (
        ce.obligation_id.clone(),
        ce.edge.kind,
        ce.edge.shape,
        ce.edge.completeness,
        routes,
    )
}

/// A file's (or `event_edges`'s) edge bucket as an order-independent
/// multiset: sorted, so two buckets containing the same edges in different
/// orders compare equal.
fn canon_edges(edges: &[ClassifiedEdge]) -> Vec<CanonEdge> {
    let mut v: Vec<CanonEdge> = edges.iter().map(canon_edge).collect();
    v.sort();
    v
}

/// `(byte start, byte end, start (row, col), end (row, col))` — projects
/// away `Origin`'s `kind_text`/EPHEMERAL `ts_id` fields (see the module
/// doc). `Origin` itself carries no derived `PartialEq`/`Ord`, so this is
/// the only way to compare two `Origin`s at all.
type CanonOrigin = (usize, usize, (u32, u32), (u32, u32));

fn canon_origin(o: &al_syntax::ir::Origin) -> CanonOrigin {
    (
        o.byte.start,
        o.byte.end,
        (o.start.row, o.start.column),
        (o.end.row, o.end.column),
    )
}

type CanonDecl = (RoutineNodeId, String, CanonOrigin, CanonOrigin);

fn canon_decl(d: &DeclEntry) -> CanonDecl {
    (
        d.id.clone(),
        d.name.clone(),
        canon_origin(&d.origin),
        canon_origin(&d.name_origin),
    )
}

fn canon_decls(decls: &[DeclEntry]) -> Vec<CanonDecl> {
    let mut v: Vec<CanonDecl> = decls.iter().map(canon_decl).collect();
    v.sort();
    v
}

/// `incoming`, canonicalized: target `RoutineNodeId` -> the SORTED set of
/// `ObligationId`s of the edges that call it. Dereferences every `EdgeRef`
/// through [`LspSnapshot::edge`] to its owning `ClassifiedEdge`'s
/// `obligation_id` — a positionless, stable identity — rather than
/// comparing raw `(file, idx)` pairs, whose global ordering can legitimately
/// differ between an incrementally-patched bucket and a fresh directory-walk
/// build (see the module doc).
fn canon_incoming(snap: &LspSnapshot) -> BTreeMap<RoutineNodeId, Vec<ObligationId>> {
    let mut out: BTreeMap<RoutineNodeId, Vec<ObligationId>> = BTreeMap::new();
    for (target, refs) in &snap.incoming {
        let mut obligations: Vec<ObligationId> = refs
            .iter()
            .map(|r| snap.edge(r).obligation_id.clone())
            .collect();
        obligations.sort();
        out.insert(target.clone(), obligations);
    }
    out
}

/// Assert `incremental` (the `Updater`'s output) is EQUIVALENT to `fresh` (an
/// independent `LspSnapshot::build_full` of the exact same on-disk state) per
/// the module doc's binding equivalence key. `context` is prefixed onto
/// every failure message so a failing assertion names exactly which
/// script/step diverged, and on WHICH file/target.
fn assert_snapshots_equivalent(incremental: &LspSnapshot, fresh: &LspSnapshot, context: &str) {
    let mut inc_files: Vec<&String> = incremental.edges_by_file.keys().collect();
    let mut fresh_files: Vec<&String> = fresh.edges_by_file.keys().collect();
    inc_files.sort();
    fresh_files.sort();
    assert_eq!(
        inc_files, fresh_files,
        "{context}: edges_by_file's file SET diverged (incremental vs fresh build_full)"
    );

    for file in &inc_files {
        let inc_edges = canon_edges(&incremental.edges_by_file[*file]);
        let fresh_edges = canon_edges(&fresh.edges_by_file[*file]);
        assert_eq!(
            inc_edges, fresh_edges,
            "{context}: file {file:?}'s edge multiset diverged (incremental vs fresh build_full)"
        );
    }

    let inc_event = canon_edges(&incremental.event_edges);
    let fresh_event = canon_edges(&fresh.event_edges);
    assert_eq!(
        inc_event, fresh_event,
        "{context}: event_edges' multiset diverged (incremental vs fresh build_full)"
    );

    let mut inc_decl_files: Vec<&String> = incremental.decls_by_file.keys().collect();
    let mut fresh_decl_files: Vec<&String> = fresh.decls_by_file.keys().collect();
    inc_decl_files.sort();
    fresh_decl_files.sort();
    assert_eq!(
        inc_decl_files, fresh_decl_files,
        "{context}: decls_by_file's file SET diverged (incremental vs fresh build_full)"
    );

    for file in &inc_decl_files {
        let inc_decls = canon_decls(&incremental.decls_by_file[*file]);
        let fresh_decls = canon_decls(&fresh.decls_by_file[*file]);
        assert_eq!(
            inc_decls, fresh_decls,
            "{context}: file {file:?}'s decl list diverged (incremental vs fresh build_full)"
        );
    }

    let inc_incoming = canon_incoming(incremental);
    let fresh_incoming = canon_incoming(fresh);
    let mut all_targets: Vec<RoutineNodeId> = inc_incoming
        .keys()
        .cloned()
        .chain(fresh_incoming.keys().cloned())
        .collect();
    all_targets.sort();
    all_targets.dedup();
    for target in &all_targets {
        assert_eq!(
            inc_incoming.get(target),
            fresh_incoming.get(target),
            "{context}: incoming map diverged for target {target:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// Script 1: body-edit chain — 3 consecutive rung-1 saves
// ---------------------------------------------------------------------------

#[test]
fn body_edit_chain_three_consecutive_saves_stay_equivalent() {
    let dir = copy_fixture_to_tempdir();
    let (base, parsed) = build_full_with_parsed(dir.path());
    let mut updater = Updater::new(dir.path().to_path_buf(), parsed);
    let mut cur = base;

    // Each step only adds another BODY statement (an extra call to an
    // ALREADY-EXISTING target) — no object/routine identity, arity, or
    // signature change, so every one of these 3 consecutive saves must stay
    // rung-1-eligible.
    let bodies = [
        r#"// Unicode smoke test (Task 10 fixture requirement): æøå
codeunit 50100 "Alpha"
{
    procedure DoWork()
    var
        Beta: Codeunit "Beta";
    begin
        Beta.Process();
        Beta.Process();
        Calc(1);
        Calc('x');
        Løbenr();
    end;

    procedure Calc(X: Integer)
    begin
    end;

    procedure Calc(X: Text)
    begin
    end;

    procedure Løbenr()
    begin
    end;

    [IntegrationEvent(false, false)]
    procedure OnAfterWork()
    begin
    end;
}
"#,
        r#"// Unicode smoke test (Task 10 fixture requirement): æøå
codeunit 50100 "Alpha"
{
    procedure DoWork()
    var
        Beta: Codeunit "Beta";
    begin
        Beta.Process();
        Beta.Process();
        Beta.Process();
        Calc(1);
        Calc('x');
        Løbenr();
    end;

    procedure Calc(X: Integer)
    begin
    end;

    procedure Calc(X: Text)
    begin
    end;

    procedure Løbenr()
    begin
    end;

    [IntegrationEvent(false, false)]
    procedure OnAfterWork()
    begin
    end;
}
"#,
        r#"// Unicode smoke test (Task 10 fixture requirement): æøå
codeunit 50100 "Alpha"
{
    procedure DoWork()
    var
        Beta: Codeunit "Beta";
    begin
        Beta.Process();
        Beta.Process();
        Beta.Process();
        Beta.Process();
        Calc(1);
        Calc('x');
        Løbenr();
        Løbenr();
    end;

    procedure Calc(X: Integer)
    begin
    end;

    procedure Calc(X: Text)
    begin
    end;

    procedure Løbenr()
    begin
    end;

    [IntegrationEvent(false, false)]
    procedure OnAfterWork()
    begin
    end;
}
"#,
    ];

    for (i, body) in bodies.iter().enumerate() {
        std::fs::write(dir.path().join("Alpha.al"), body)
            .unwrap_or_else(|e| panic!("rewrite Alpha.al (step {i}): {e}"));

        let batch = vec![ChangeEvent::FileSaved(dir.path().join("Alpha.al"))];
        let (new_snap, rung) = updater
            .apply_batch(&cur, &batch)
            .unwrap_or_else(|| panic!("apply_batch must succeed (step {i})"));
        assert_eq!(
            rung,
            Rung::One,
            "body-edit chain step {i}: a body-only edit must take rung 1"
        );

        let fresh = LspSnapshot::build_full(dir.path()).expect("fresh build_full");
        assert_snapshots_equivalent(&new_snap, &fresh, &format!("body-edit-chain step {i}"));

        cur = new_snap;
    }
}

// ---------------------------------------------------------------------------
// Script 2: signature change — rung 2, breaks 3 cross-file callers
// ---------------------------------------------------------------------------

#[test]
fn signature_change_stays_equivalent() {
    let dir = copy_fixture_to_tempdir();
    let (base, parsed) = build_full_with_parsed(dir.path());
    let mut updater = Updater::new(dir.path().to_path_buf(), parsed);

    // Beta.Process gains a required parameter — a DefSurface change (arity
    // moves). Alpha.DoWork, MyTable's field(2) OnValidate trigger, and
    // MyPage's OnOpenPage trigger all call the OLD 0-arg shape, so every one
    // of those 3 cross-file/cross-object call sites must flip to Unknown.
    std::fs::write(
        dir.path().join("Beta.al"),
        r#"codeunit 50101 "Beta"
{
    procedure Process(X: Integer)
    begin
    end;

    [EventSubscriber(ObjectType::Codeunit, Codeunit::"Alpha", 'OnAfterWork', '', false, false)]
    local procedure HandleAfterWork()
    begin
    end;
}
"#,
    )
    .expect("rewrite Beta.al with a new required parameter");

    let batch = vec![ChangeEvent::FileSaved(dir.path().join("Beta.al"))];
    let (new_snap, rung) = updater
        .apply_batch(&base, &batch)
        .expect("apply_batch must succeed");
    assert_eq!(
        rung,
        Rung::Two,
        "a signature (arity) change must take rung 2"
    );

    let fresh = LspSnapshot::build_full(dir.path()).expect("fresh build_full");
    assert_snapshots_equivalent(&new_snap, &fresh, "signature-change");
}

// ---------------------------------------------------------------------------
// Script 3: rename routine — rung 2, breaks the same 3 callers
// ---------------------------------------------------------------------------

#[test]
fn rename_routine_stays_equivalent() {
    let dir = copy_fixture_to_tempdir();
    let (base, parsed) = build_full_with_parsed(dir.path());
    let mut updater = Updater::new(dir.path().to_path_buf(), parsed);

    // Renaming Process -> DoProcess moves the RoutineNodeId SET (item 3 of
    // the DefSurface fingerprint) without changing arity — every caller
    // still says `Beta.Process()`, which no longer exists.
    std::fs::write(
        dir.path().join("Beta.al"),
        r#"codeunit 50101 "Beta"
{
    procedure DoProcess()
    begin
    end;

    [EventSubscriber(ObjectType::Codeunit, Codeunit::"Alpha", 'OnAfterWork', '', false, false)]
    local procedure HandleAfterWork()
    begin
    end;
}
"#,
    )
    .expect("rewrite Beta.al renaming Process to DoProcess");

    let batch = vec![ChangeEvent::FileSaved(dir.path().join("Beta.al"))];
    let (new_snap, rung) = updater
        .apply_batch(&base, &batch)
        .expect("apply_batch must succeed");
    assert_eq!(rung, Rung::Two, "a routine rename must take rung 2");

    let fresh = LspSnapshot::build_full(dir.path()).expect("fresh build_full");
    assert_snapshots_equivalent(&new_snap, &fresh, "rename-routine");
}

// ---------------------------------------------------------------------------
// Script 4: add file — rung 2 (brand-new file, no prior surface to compare)
// ---------------------------------------------------------------------------

#[test]
fn add_file_stays_equivalent() {
    let dir = copy_fixture_to_tempdir();
    let (base, parsed) = build_full_with_parsed(dir.path());
    let mut updater = Updater::new(dir.path().to_path_buf(), parsed);

    std::fs::write(
        dir.path().join("Epsilon.al"),
        r#"codeunit 50105 "Epsilon"
{
    procedure DoIt()
    var
        Alpha: Codeunit "Alpha";
    begin
        Alpha.Calc(5);
    end;
}
"#,
    )
    .expect("write brand-new Epsilon.al");

    let batch = vec![ChangeEvent::FileSaved(dir.path().join("Epsilon.al"))];
    let (new_snap, rung) = updater
        .apply_batch(&base, &batch)
        .expect("apply_batch must succeed");
    assert_eq!(
        rung,
        Rung::Two,
        "a brand-new file has no prior surface to compare against — must take rung 2"
    );
    assert!(
        new_snap.edges_by_file.contains_key("Epsilon.al"),
        "the new file's edge bucket must be present"
    );

    let fresh = LspSnapshot::build_full(dir.path()).expect("fresh build_full");
    assert_snapshots_equivalent(&new_snap, &fresh, "add-file");
}

// ---------------------------------------------------------------------------
// Script 5: delete file — rung 2, its edges/incoming entries disappear
// ---------------------------------------------------------------------------

#[test]
fn delete_file_stays_equivalent() {
    let dir = copy_fixture_to_tempdir();
    let (base, parsed) = build_full_with_parsed(dir.path());
    let mut updater = Updater::new(dir.path().to_path_buf(), parsed);

    // Baseline sanity: MyPage.al's OnOpenPage call must be one of
    // Beta.Process's incoming callers before the delete.
    let beta_process = base.decls_by_file["Beta.al"]
        .iter()
        .find(|d| d.name == "Process")
        .expect("Beta.Process decl")
        .id
        .clone();
    let incoming_before = base
        .incoming
        .get(&beta_process)
        .expect("Beta.Process must have incoming callers before delete");
    assert!(
        incoming_before.iter().any(|r| r.file == "MyPage.al"),
        "baseline: MyPage.al must be one of Beta.Process's incoming callers"
    );

    std::fs::remove_file(dir.path().join("MyPage.al")).expect("delete MyPage.al");
    let batch = vec![ChangeEvent::FileRemoved(dir.path().join("MyPage.al"))];
    let (new_snap, rung) = updater
        .apply_batch(&base, &batch)
        .expect("apply_batch must succeed");
    assert_eq!(rung, Rung::Two, "a file delete must take rung 2");

    assert!(
        !new_snap.edges_by_file.contains_key("MyPage.al"),
        "MyPage.al's edge bucket must be gone"
    );
    assert!(!new_snap.decls_by_file.contains_key("MyPage.al"));

    let fresh = LspSnapshot::build_full(dir.path()).expect("fresh build_full");
    assert_snapshots_equivalent(&new_snap, &fresh, "delete-file");
}

// ---------------------------------------------------------------------------
// Script 6: edit that flips overload resolution — body-only, stays rung 1
// ---------------------------------------------------------------------------

/// The Alpha.al call site at line `line` (0-based, matching
/// `Origin`/`SourcePos` conventions) whose route names an overload of
/// `Calc` — i.e. one specific `Calc()` call, identified by its FIXED source
/// position rather than by iteration order (`edges_by_file`'s per-file
/// order is a multiset for equivalence purposes — see the module doc — so
/// this test must not rely on it; a line number is a stable, meaningful
/// identity a real call site actually has).
fn calc_target_at_line(edges: &[ClassifiedEdge], line: u32) -> RoutineNodeId {
    let ce = edges
        .iter()
        .find(|ce| ce.edge.site.span.start.line == line)
        .unwrap_or_else(|| panic!("no call site at line {line}"));
    let route = ce
        .edge
        .routes
        .iter()
        .find(|r| matches!(&r.target, RouteTarget::Routine(t) if t.name_lc == "calc"))
        .unwrap_or_else(|| panic!("line {line}'s edge does not route to a Calc overload"));
    let RouteTarget::Routine(target) = &route.target else {
        unreachable!("just matched on RouteTarget::Routine above")
    };
    target.clone()
}

#[test]
fn overload_flip_body_only_edit_stays_rung1_and_equivalent() {
    let dir = copy_fixture_to_tempdir();
    let (base, parsed) = build_full_with_parsed(dir.path());
    let mut updater = Updater::new(dir.path().to_path_buf(), parsed);

    // The fixture's DoWork body (both before and after this test's edit)
    // calls Calc() twice, on these exact, UNCHANGED source lines (only the
    // literal TOKEN on each line changes below — no line is added or
    // removed) — 0-based, matching `Origin`/`SourcePos`.
    const INTEGER_LITERAL_CALL_LINE: u32 = 8; // `Calc(1)` before -> `Calc('flipped')` after
    const TEXT_LITERAL_CALL_LINE: u32 = 9; // `Calc('x')` before -> `Calc(2)` after

    // Baseline, from the UNTOUCHED fixture: name each overload's specific
    // RoutineNodeId by which literal type resolves to it, so the assertions
    // below can say "Calc(Text)"/"Calc(Integer)" honestly rather than just
    // "site A"/"site B".
    let calc_integer_id =
        calc_target_at_line(&base.edges_by_file["Alpha.al"], INTEGER_LITERAL_CALL_LINE);
    let calc_text_id = calc_target_at_line(&base.edges_by_file["Alpha.al"], TEXT_LITERAL_CALL_LINE);
    assert_ne!(
        calc_integer_id, calc_text_id,
        "baseline sanity: Calc(Integer) and Calc(Text) must be DISTINCT routines \
         (otherwise this test cannot prove anything about a flip)"
    );

    // Swap which literal each Calc() call site passes — Alpha.Calc(Integer)/
    // Calc(Text) is the fixture's overload set, and arg-type dispatch picks
    // between them at CALL-SITE resolution time, which is body content, not
    // definition surface: no object/routine identity, arity, or param type
    // moved, so this must still take rung 1 — and the incremental path must
    // re-run arg-type dispatch against the file's FRESH content rather than
    // a stale cached BodyMap (the module doc's soundness argument for why
    // rung 1 resolves the touched file directly from its fresh parse).
    std::fs::write(
        dir.path().join("Alpha.al"),
        r#"// Unicode smoke test (Task 10 fixture requirement): æøå
codeunit 50100 "Alpha"
{
    procedure DoWork()
    var
        Beta: Codeunit "Beta";
    begin
        Beta.Process();
        Calc('flipped');
        Calc(2);
        Løbenr();
    end;

    procedure Calc(X: Integer)
    begin
    end;

    procedure Calc(X: Text)
    begin
    end;

    procedure Løbenr()
    begin
    end;

    [IntegrationEvent(false, false)]
    procedure OnAfterWork()
    begin
    end;
}
"#,
    )
    .expect("rewrite Alpha.al with swapped Calc() literal types");

    let batch = vec![ChangeEvent::FileSaved(dir.path().join("Alpha.al"))];
    let (new_snap, rung) = updater
        .apply_batch(&base, &batch)
        .expect("apply_batch must succeed");
    assert_eq!(
        rung,
        Rung::One,
        "swapping which literal each Calc() call passes is body-only — the \
         DefSurface fingerprint (routine set/arity/param types) is unaffected"
    );

    let calc_edges: Vec<&ClassifiedEdge> = new_snap.edges_by_file["Alpha.al"]
        .iter()
        .filter(|ce| {
            ce.edge
                .routes
                .iter()
                .any(|r| matches!(&r.target, RouteTarget::Routine(t) if t.name_lc == "calc"))
        })
        .collect();
    assert_eq!(
        calc_edges.len(),
        2,
        "both Calc() call sites must still be present after the swap"
    );
    for ce in &calc_edges {
        assert!(
            ce.edge
                .routes
                .iter()
                .any(|r| r.evidence.kind() == EvidenceKind::Source),
            "each flipped call site must still cleanly resolve (Evidence::Source), \
             proving the incremental path re-ran arg-type dispatch against the \
             fresh file rather than a stale cached BodyMap"
        );
    }

    // The self-contained, honest core of this test (review fix-wave — this
    // must hold WITHOUT leaning on the trailing full-equivalence check
    // below): the line that now passes the TEXT literal (`Calc('flipped')`,
    // the line that passed the INTEGER literal before) must route to
    // `calc_text_id`, and the line that now passes the INTEGER literal
    // (`Calc(2)`, the line that passed the TEXT literal before) must route
    // to `calc_integer_id` — i.e. the overload identity assigned to each
    // FIXED source position genuinely flipped, not merely "stayed resolved
    // to something."
    let new_edges = &new_snap.edges_by_file["Alpha.al"];
    let now_text_literal_site_target = calc_target_at_line(new_edges, INTEGER_LITERAL_CALL_LINE);
    let now_integer_literal_site_target = calc_target_at_line(new_edges, TEXT_LITERAL_CALL_LINE);
    assert_eq!(
        now_text_literal_site_target, calc_text_id,
        "the line that now passes a Text literal (Calc('flipped')) must name \
         Calc(Text)'s specific RoutineNodeId"
    );
    assert_eq!(
        now_integer_literal_site_target, calc_integer_id,
        "the line that now passes an Integer literal (Calc(2)) must name \
         Calc(Integer)'s specific RoutineNodeId"
    );

    let fresh = LspSnapshot::build_full(dir.path()).expect("fresh build_full");
    assert_snapshots_equivalent(&new_snap, &fresh, "overload-flip");
}

// ---------------------------------------------------------------------------
// Script 7: event-subscriber attribute edit — rung 2, event edge disappears
// ---------------------------------------------------------------------------

#[test]
fn event_subscriber_attribute_edit_stays_equivalent() {
    let dir = copy_fixture_to_tempdir();
    let (base, parsed) = build_full_with_parsed(dir.path());
    let mut updater = Updater::new(dir.path().to_path_buf(), parsed);

    assert!(
        !base.event_edges.is_empty(),
        "baseline: Alpha.OnAfterWork -> Beta.HandleAfterWork must be wired"
    );

    // Change the subscribed event name so it no longer matches anything
    // Alpha actually publishes — a DefSurface change (item 14,
    // event_subscribers), so this must take rung 2, and the whole-graph
    // event-flow re-emission must drop the now-orphaned subscriber.
    std::fs::write(
        dir.path().join("Beta.al"),
        r#"codeunit 50101 "Beta"
{
    procedure Process()
    begin
    end;

    [EventSubscriber(ObjectType::Codeunit, Codeunit::"Alpha", 'OnAfterWorkRenamed', '', false, false)]
    local procedure HandleAfterWork()
    begin
    end;
}
"#,
    )
    .expect("rewrite Beta.al with a non-matching subscribed event name");

    let batch = vec![ChangeEvent::FileSaved(dir.path().join("Beta.al"))];
    let (new_snap, rung) = updater
        .apply_batch(&base, &batch)
        .expect("apply_batch must succeed");
    assert_eq!(
        rung,
        Rung::Two,
        "an EventSubscriber attribute edit moves the DefSurface fingerprint's \
         event_subscribers item"
    );

    let still_wired = new_snap.event_edges.iter().any(|ce| {
        ce.edge
            .routes
            .iter()
            .any(|r| matches!(&r.target, RouteTarget::Routine(t) if t.name_lc == "handleafterwork"))
    });
    assert!(
        !still_wired,
        "the subscriber must no longer receive Alpha's OnAfterWork event once \
         its subscribed event name no longer matches"
    );

    let fresh = LspSnapshot::build_full(dir.path()).expect("fresh build_full");
    assert_snapshots_equivalent(&new_snap, &fresh, "event-subscriber-attribute-edit");
}

// ---------------------------------------------------------------------------
// Script 8: one MIXED 6-edit batch — add + delete + rename + signature +
// 2 body-only edits, all coalesced into ONE apply_batch call
// ---------------------------------------------------------------------------

#[test]
fn mixed_six_edit_batch_stays_equivalent() {
    let dir = copy_fixture_to_tempdir();
    let (base, parsed) = build_full_with_parsed(dir.path());
    let mut updater = Updater::new(dir.path().to_path_buf(), parsed);

    // Edit 1 (body-only): Alpha gains one more Løbenr() call.
    std::fs::write(
        dir.path().join("Alpha.al"),
        r#"// Unicode smoke test (Task 10 fixture requirement): æøå
codeunit 50100 "Alpha"
{
    procedure DoWork()
    var
        Beta: Codeunit "Beta";
    begin
        Beta.Process();
        Calc(1);
        Calc('x');
        Løbenr();
        Løbenr();
    end;

    procedure Calc(X: Integer)
    begin
    end;

    procedure Calc(X: Text)
    begin
    end;

    procedure Løbenr()
    begin
    end;

    [IntegrationEvent(false, false)]
    procedure OnAfterWork()
    begin
    end;
}
"#,
    )
    .expect("edit 1: rewrite Alpha.al (body-only)");

    // Edit 2 (body-only): MyPage's OnOpenPage calls Process() twice.
    std::fs::write(
        dir.path().join("MyPage.al"),
        r#"page 50104 "LSP Incr Page"
{
    SourceTable = "LSP Incr Table";

    layout
    {
        area(Content)
        {
            repeater(Group)
            {
                field("No."; Rec."No.")
                {
                }
            }
        }
    }

    trigger OnOpenPage()
    var
        Beta: Codeunit "Beta";
    begin
        Beta.Process();
        Beta.Process();
    end;
}
"#,
    )
    .expect("edit 2: rewrite MyPage.al (body-only)");

    // Edit 3 (brand-new file): Epsilon calls Alpha.Calc(5).
    std::fs::write(
        dir.path().join("Epsilon.al"),
        r#"codeunit 50105 "Epsilon"
{
    procedure DoIt()
    var
        Alpha: Codeunit "Alpha";
    begin
        Alpha.Calc(5);
    end;
}
"#,
    )
    .expect("edit 3: write brand-new Epsilon.al");

    // Edit 4 (delete): the tableextension goes away entirely.
    std::fs::remove_file(dir.path().join("MyTableExt.al")).expect("edit 4: delete MyTableExt.al");

    // Edit 5 (signature change): MyTable gains a new field.
    std::fs::write(
        dir.path().join("MyTable.al"),
        r#"table 50102 "LSP Incr Table"
{
    fields
    {
        field(1; "No."; Code[20]) { }
        field(2; Description; Text[100])
        {
            trigger OnValidate()
            var
                Beta: Codeunit "Beta";
            begin
                Beta.Process();
            end;
        }
        field(3; Amount; Decimal) { }
    }
    keys
    {
        key(PK; "No.") { }
    }
}
"#,
    )
    .expect("edit 5: rewrite MyTable.al (new field)");

    // Edit 6 (rename): Beta.Process -> Beta.DoProcess — breaks every other
    // edit's own calls to the old name in the SAME batch (Alpha, MyPage,
    // MyTable's field trigger all still say `Beta.Process()`).
    std::fs::write(
        dir.path().join("Beta.al"),
        r#"codeunit 50101 "Beta"
{
    procedure DoProcess()
    begin
    end;

    [EventSubscriber(ObjectType::Codeunit, Codeunit::"Alpha", 'OnAfterWork', '', false, false)]
    local procedure HandleAfterWork()
    begin
    end;
}
"#,
    )
    .expect("edit 6: rewrite Beta.al (rename Process -> DoProcess)");

    let batch = vec![
        ChangeEvent::FileSaved(dir.path().join("Alpha.al")),
        ChangeEvent::FileSaved(dir.path().join("MyPage.al")),
        ChangeEvent::FileSaved(dir.path().join("Epsilon.al")),
        ChangeEvent::FileRemoved(dir.path().join("MyTableExt.al")),
        ChangeEvent::FileSaved(dir.path().join("MyTable.al")),
        ChangeEvent::FileSaved(dir.path().join("Beta.al")),
    ];
    let (new_snap, rung) = updater
        .apply_batch(&base, &batch)
        .expect("apply_batch must succeed for the mixed 6-edit batch");
    assert_eq!(
        rung,
        Rung::Two,
        "a batch containing ANY rung-2-eligible event must take rung 2 for the whole batch"
    );

    assert!(new_snap.edges_by_file.contains_key("Epsilon.al"));
    assert!(!new_snap.edges_by_file.contains_key("MyTableExt.al"));

    let fresh = LspSnapshot::build_full(dir.path()).expect("fresh build_full");
    assert_snapshots_equivalent(&new_snap, &fresh, "mixed-6-edit-batch");
}

// ---------------------------------------------------------------------------
// Step 2: non-vacuity negative control (binding)
// ---------------------------------------------------------------------------

/// Independently proves — without relying on any other script's specific
/// assertions — that this suite's `apply_batch` calls really do take BOTH
/// `Rung::One` and `Rung::Two` at least once each. A suite that silently
/// took the slow, always-correct rung 3 (or always rung 2) everywhere would
/// pass every equivalence check above for a trivial, uninteresting reason.
#[test]
fn gate_non_vacuity_rung1_and_rung2_are_both_exercised() {
    let dir = copy_fixture_to_tempdir();
    let (base, parsed) = build_full_with_parsed(dir.path());
    let mut updater = Updater::new(dir.path().to_path_buf(), parsed);

    // A body-only edit -> rung 1.
    std::fs::write(
        dir.path().join("Alpha.al"),
        r#"// Unicode smoke test (Task 10 fixture requirement): æøå
codeunit 50100 "Alpha"
{
    procedure DoWork()
    var
        Beta: Codeunit "Beta";
    begin
        Beta.Process();
        Beta.Process();
        Calc(1);
        Calc('x');
        Løbenr();
    end;

    procedure Calc(X: Integer)
    begin
    end;

    procedure Calc(X: Text)
    begin
    end;

    procedure Løbenr()
    begin
    end;

    [IntegrationEvent(false, false)]
    procedure OnAfterWork()
    begin
    end;
}
"#,
    )
    .expect("rewrite Alpha.al (rung-1 probe)");
    let (r1_snap, r1) = updater
        .apply_batch(
            &base,
            &[ChangeEvent::FileSaved(dir.path().join("Alpha.al"))],
        )
        .expect("apply_batch must succeed (rung-1 probe)");
    assert_eq!(
        r1,
        Rung::One,
        "non-vacuity probe: a body-only edit must take rung 1"
    );

    // A signature edit on a DIFFERENT file -> rung 2.
    std::fs::write(
        dir.path().join("Beta.al"),
        r#"codeunit 50101 "Beta"
{
    procedure Process(X: Integer)
    begin
    end;

    [EventSubscriber(ObjectType::Codeunit, Codeunit::"Alpha", 'OnAfterWork', '', false, false)]
    local procedure HandleAfterWork()
    begin
    end;
}
"#,
    )
    .expect("rewrite Beta.al (rung-2 probe)");
    let (_r2_snap, r2) = updater
        .apply_batch(
            &r1_snap,
            &[ChangeEvent::FileSaved(dir.path().join("Beta.al"))],
        )
        .expect("apply_batch must succeed (rung-2 probe)");
    assert_eq!(
        r2,
        Rung::Two,
        "non-vacuity probe: a signature (arity) edit must take rung 2"
    );
}

// ---------------------------------------------------------------------------
// Meta-test: canon_edge's discriminating power (review fix-wave)
// ---------------------------------------------------------------------------

/// Proves the widened `CanonEdge` key (review fix-wave: added `EdgeKind`,
/// `DispatchShape`, `SetCompleteness`, and each route's `Condition` set) is
/// not vacuous: 4 pairs of hand-constructed `ClassifiedEdge`s, each pair
/// IDENTICAL in everything the PRE-fix-wave key covered (`ObligationId`/
/// `RouteTarget`/`EvidenceKind`) but differing in exactly ONE of these 4
/// newly-added dimensions, must canonicalize UNEQUAL.
///
/// Calibration performed for this review fix-wave (temporary, reverted —
/// described in the task-10 report's fix-wave section): narrowed
/// `canon_edge`/`CanonEdge` back to the pre-fix-wave shape (dropping
/// `kind`/`shape`/`completeness` from the edge tuple and `conditions` from
/// `canon_route`, keeping only `(ObligationId, Vec<(RouteTarget,
/// EvidenceKind)>)`) and re-ran this exact test — all 4 `assert_ne!`s below
/// failed (each pair collapsed to the SAME `CanonEdge`), confirming the
/// widened key is what makes them distinguishable, not an accident of
/// `ObligationId` already differing between the pairs.
#[test]
fn canon_edge_distinguishes_kind_shape_completeness_and_conditions() {
    fn rid(name: &str) -> RoutineNodeId {
        RoutineNodeId {
            object: ObjectNodeId {
                app: AppRef(0),
                kind: al_syntax::ir::ObjectKind::Codeunit,
                key: ObjKey::Id(1),
            },
            name_lc: name.to_string(),
            enclosing_member_lc: None,
            params_count: 0,
            sig_fp: 0,
        }
    }

    fn base_edge(caller: RoutineNodeId, target: RoutineNodeId) -> Edge {
        Edge {
            from: caller.clone(),
            site: SiteId {
                caller,
                span: CanonicalSpan {
                    unit: "F.al".into(),
                    start: SourcePos { line: 1, col: 1 },
                    end: SourcePos { line: 1, col: 2 },
                },
                callee_fingerprint: 1,
            },
            kind: EdgeKind::Call,
            shape: DispatchShape::Exact,
            completeness: SetCompleteness::Complete,
            routes: vec![Route {
                target: RouteTarget::Routine(target),
                evidence: Evidence::Source,
                conditions: vec![],
                witness: Witness::None,
                receiver_tier: None,
            }],
        }
    }

    fn classified(edge: Edge) -> ClassifiedEdge {
        ClassifiedEdge {
            obligation_id: ObligationId::CallSite {
                caller: edge.from.clone(),
                span: edge.site.span.clone(),
                callee_fp: edge.site.callee_fingerprint,
            },
            edge,
        }
    }

    let caller = rid("caller");
    let target = rid("target");
    let base_canon = canon_edge(&classified(base_edge(caller.clone(), target.clone())));

    let mut kind_variant = base_edge(caller.clone(), target.clone());
    kind_variant.kind = EdgeKind::Run;
    assert_ne!(
        canon_edge(&classified(kind_variant)),
        base_canon,
        "two edges differing only in EdgeKind must NOT canonicalize equal"
    );

    let mut shape_variant = base_edge(caller.clone(), target.clone());
    shape_variant.shape = DispatchShape::Multicast;
    assert_ne!(
        canon_edge(&classified(shape_variant)),
        base_canon,
        "two edges differing only in DispatchShape must NOT canonicalize equal"
    );

    let mut completeness_variant = base_edge(caller.clone(), target.clone());
    completeness_variant.completeness = SetCompleteness::Partial {
        reason: OpenWorldReason::RuntimeTypeUnbounded,
    };
    assert_ne!(
        canon_edge(&classified(completeness_variant)),
        base_canon,
        "two edges differing only in SetCompleteness must NOT canonicalize equal"
    );

    let mut condition_variant = base_edge(caller, target);
    condition_variant.routes[0].conditions = vec![Condition::ManualBinding];
    assert_ne!(
        canon_edge(&classified(condition_variant)),
        base_canon,
        "two edges differing only in a route's Condition set must NOT canonicalize equal"
    );
}
