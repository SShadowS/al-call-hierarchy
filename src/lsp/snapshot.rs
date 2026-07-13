//! `LspSnapshot` (T3 Task 8): the immutable, batch-built, owned-derived-index
//! snapshot the migrated LSP server serves queries from — the arc's
//! structural centerpiece.
//!
//! [`LspSnapshot::build_full`] composes the engine primitives landed by
//! earlier T3 tasks (`SnapshotBuilder` → `parse_snapshot` → `build_dep_layer`/
//! `assemble_program_graph` [Task 5] → per-file `resolve_file_obligations`
//! [Task 6] → `def_surface_fingerprint` [Task 7] → `emit_event_flow_edges`)
//! into one self-contained, `Arc`-shareable value: every field is OWNED data
//! (never a borrow into another field), so the whole snapshot can be handed
//! to a query thread as `Arc<LspSnapshot>` without any lifetime entanglement.
//!
//! # Ownership law (spec §3 / H-10 lesson)
//!
//! `ResolveIndex`/`BodyMap`/the `ObjectNodeId → &ObjectNode` map all BORROW
//! `graph`/`parsed` and are built TRANSIENTLY inside [`LspSnapshot::build_full`]
//! — they never appear as fields on `LspSnapshot` itself (that would make the
//! struct self-referential). [`build_incoming`] is the one INDEX that IS
//! derived and stored: it is rebuilt WHOLESALE on every `build_full` call,
//! never incrementally edited — a future incremental updater (Task 9) always
//! throws it away and recomputes it from `edges_by_file`/`event_edges`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use al_syntax::ir::AlFile;

use crate::lsp::def_surface::{DefSurface, def_surface_fingerprint};
use crate::program::node::{AppRef, ObjKey, ObjectNodeId, RoutineNodeId};
use crate::program::node_extract::ObjectNode;
use crate::program::resolve::body_map::BodyMap;
use crate::program::resolve::edge::{Edge, RouteTarget};
use crate::program::resolve::emit_event_flow_edges;
use crate::program::resolve::full::{ClassifiedEdge, ObligationId, ProgramContext, build_context};
use crate::program::resolve::index::ResolveIndex;
use crate::program::sig_fp::source_routine_node_id;
use crate::program::{DepLayer, ProgramGraph};
use crate::snapshot::{AppSetSnapshot, ParsedFile, ParsedUnit, parse_snapshot};

/// Reference to one edge: (virtual_path, index into `edges_by_file[path]`).
/// Index-based — never a borrow — so [`LspSnapshot`] stays self-contained and
/// `Arc`-shareable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EdgeRef {
    pub file: String,
    pub idx: u32,
}

/// Reserved `EdgeRef.file` key for [`LspSnapshot::event_edges`] — a
/// NUL-prefixed string no real AL `virtual_path` can ever collide with (a
/// `virtual_path` is built from real filesystem-derived path segments, none
/// of which can embed `\0`), so `EdgeRef` stays uniform (always plainly
/// `(file, idx)`) without needing a separate enum-tagged variant just for
/// event-flow edges.
pub const EVENT_EDGES_KEY: &str = "\u{0}events";

/// [`LspSnapshot::dep_decl_by_id`]'s map type — aliased so
/// [`build_dep_indexes`]'s signature stays readable (clippy
/// `type_complexity`).
pub(crate) type DepDeclById = HashMap<RoutineNodeId, DeclEntry>;
/// [`LspSnapshot::dep_texts`]'s map type — see [`DepDeclById`]'s doc.
pub(crate) type DepTexts = HashMap<(AppRef, String), Arc<str>>;

/// One routine declaration's identity + LSP-facing spans, owned (never
/// borrowing the `AlFile` it was read from — `Origin` is plain data).
#[derive(Clone, Debug)]
pub struct DeclEntry {
    pub id: RoutineNodeId,
    /// Raw casing, for display (`RoutineNodeId::name_lc` is lowercased).
    pub name: String,
    /// Whole declaration span (`CallHierarchyItem.range`).
    pub origin: al_syntax::ir::Origin,
    /// Name-token span (`CallHierarchyItem.selectionRange`).
    pub name_origin: al_syntax::ir::Origin,
    pub virtual_path: String,
}

/// One parsed file's owned data: the `AlFile` IR, its source text, and its
/// definition-surface fingerprint (Task 7) — everything a query needs
/// without re-reading disk or re-parsing.
pub struct ParsedFileEntry {
    pub file: AlFile,
    /// Shares the workspace `SourceFile.text` allocation (perf safe-wins Task 1).
    pub text: Arc<str>,
    pub virtual_path: String,
    pub surface: DefSurface,
}

/// The immutable, batch-built LSP snapshot: a whole-program resolve pass
/// frozen into owned, `Arc`-shareable data. See the module doc for the
/// composition [`LspSnapshot::build_full`] runs and the ownership law that
/// keeps every field self-contained.
pub struct LspSnapshot {
    /// Monotonic build counter. `build_full` always produces generation `0`
    /// (a full batch build has no prior generation to count from) — a future
    /// incremental updater (Task 9) bumps this on each rung-1/rung-2 apply.
    /// Excluded from cross-build equivalence checks (see this module's tests).
    pub generation: u64,
    /// `Arc`-shared (T3 Task 9): rung 1 (body-only edit) and rung 2
    /// (workspace-layer rebuild reusing the cached dep layer) both need to
    /// hand an UNCHANGED-or-rebuilt graph to a fresh `LspSnapshot` value
    /// without deep-cloning `ProgramGraph`'s node arrays (`ObjectIndex`
    /// carries no `Clone` impl, and cloning tens of thousands of
    /// `ObjectNode`/`RoutineNode` entries on every rung-1 save would itself
    /// blow the <100ms budget) — mirrors `dep_layer`'s existing pattern.
    pub graph: Arc<ProgramGraph>,
    pub dep_layer: Arc<DepLayer>,
    /// Identity/roots for rebuilds. `Arc`-shared for the same reason as
    /// `graph` above: `AppSetSnapshot` carries every app's full source TEXT
    /// (`AppUnit::source`), so a plain `.clone()` on every incremental swap
    /// would copy megabytes of text neither rung 1 nor rung 2 ever touches.
    pub snap: Arc<AppSetSnapshot>,
    /// `virtual_path` → file+text+`DefSurface`, workspace files ONLY (mirrors
    /// `edges_by_file`'s workspace scoping — a dependency's own source is
    /// never queried by the LSP surface).
    pub parsed: HashMap<String, Arc<ParsedFileEntry>>,
    /// Workspace-scoped: holds ONLY Phase-1 (workspace-caller) `Call`/`Run`/
    /// `ImplicitTrigger` edge buckets, keyed by `virtual_path`.
    pub edges_by_file: HashMap<String, Arc<Vec<ClassifiedEdge>>>,
    /// Phase-2 `EventFlow` edges (whole-program: every publisher in every
    /// app, not just the workspace) — kept in ONE flat bucket rather than
    /// per-file, addressed via the reserved [`EVENT_EDGES_KEY`].
    pub event_edges: Arc<Vec<ClassifiedEdge>>,
    /// DERIVED — see [`build_incoming`]'s doc. O(E) wholesale rebuild only;
    /// never incrementally edited.
    pub incoming: HashMap<RoutineNodeId, Vec<EdgeRef>>,
    /// DERIVED, precomputed in the SAME O(E) pass [`build_incoming`] makes
    /// over `event_edges` (t3 whole-branch review, blocker fix): for every
    /// routine `P` that is the `from` (publisher) of at least one
    /// `event_edges` entry, the sum of that entry's `routes.len()` — the
    /// REAL resolved-subscriber count [`crate::lsp::lens::
    /// effective_incoming_count`] needs for its "as-publisher fan-out" term.
    /// Before this field existed, that function computed the identical value
    /// by scanning ALL of `event_edges` on EVERY call — O(E) per query,
    /// called once per declaration by `compute_all` on every diagnostics
    /// recompute (itself run on every snapshot swap, including a rung-1
    /// single-file body edit), making a full diagnostics pass O(decls ×
    /// event_edges) — quadratic in workspace size. Precomputing it here
    /// keeps `effective_incoming_count` O(1) per call, matching `incoming`'s
    /// own precomputed-index pattern; rebuilt wholesale alongside `incoming`
    /// at every rung (H-10 law — never incrementally patched).
    pub publisher_fanout: HashMap<RoutineNodeId, usize>,
    /// Sorted by `origin.byte.start` within each file. `Arc`-wrapped per file
    /// (T3 Task 9) so an incremental rung-1/rung-2 rebuild can share every
    /// UNCHANGED file's decl list via a cheap `Arc::clone` instead of
    /// deep-cloning the whole `HashMap<String, Vec<DeclEntry>>` (every
    /// `DeclEntry`'s `String` fields would otherwise be re-heap-allocated on
    /// every save, across the WHOLE workspace, just to replace one file).
    pub decls_by_file: HashMap<String, Arc<Vec<DeclEntry>>>,
    /// DERIVED — like [`Self::incoming`], always rebuilt WHOLESALE from
    /// `decls_by_file` (see [`build_decl_by_id`]) rather than
    /// cloned-then-patched. Never treat this as an independent source of
    /// truth to surgically edit (H-10 law).
    pub decl_by_id: HashMap<RoutineNodeId, DeclEntry>,
    /// The `RouteTarget::Routine(id)`-target counterpart of [`Self::decl_by_id`]
    /// for every NON-primary (dependency) app — the design doc's §5 promise
    /// that "a dep with embedded source gets REAL navigable spans (legacy
    /// never could)". `make_routine_route` (the resolver) only ever
    /// constructs `RouteTarget::Routine(id)` when the SAME `BodyMap` this
    /// entry is built from just answered `Some` for `id` — so any `id` an
    /// edge carries as a `Routine` target is guaranteed to be found in
    /// EITHER `decl_by_id` (workspace) or here, never neither. A dependency's
    /// own source cannot change except on a rung-3 rebuild (rung 1/2 both
    /// reuse the cached, unchanged `dep_layer` — see `Updater::apply_rung2`'s
    /// doc), so callers `Arc::clone` this forward across rung 1/2 rather than
    /// recomputing it. See [`build_dep_indexes`].
    pub dep_decl_by_id: Arc<DepDeclById>,
    /// Source text for every file contributing an entry to
    /// [`Self::dep_decl_by_id`], keyed `(app, virtual_path)` — a
    /// dependency's `virtual_path` is only unique WITHIN its own app (two
    /// different deps can each have their own "Codeunit1.al"), unlike
    /// `Self::parsed`'s workspace-only, plain-`String`-keyed map. This is
    /// the `LineTable` text source for a dependency-source item's
    /// position-encoding conversion (mirrors [`ParsedFileEntry::text`]'s
    /// role for workspace files). Look both maps up together via
    /// [`Self::decl_and_text`] rather than indexing either directly.
    pub dep_texts: Arc<DepTexts>,
    /// The workspace root every `virtual_path` in this snapshot is relative
    /// to, normalized via [`crate::protocol::normalize_path`] (T3 Task 11) —
    /// so a handler can turn an inbound `textDocument` URI into the SAME
    /// `virtual_path` key `decls_by_file`/`parsed` use, via `uri_to_path`
    /// (which ALSO normalizes) + `strip_prefix`, without either side's
    /// casing silently mismatching on Windows. `Arc`-wrapped like `snap`/
    /// `dep_layer`: identical across every rung (the workspace root a
    /// running server watches never changes mid-session).
    pub workspace_root: Arc<PathBuf>,
}

impl LspSnapshot {
    /// Full batch build — snapshot → dep layer → assemble → resolve per file
    /// → derive indexes. Returns `None` when the underlying snapshot/program
    /// context build fails (fail-closed, mirrors
    /// [`crate::program::resolve::full::resolve_full_program`]).
    #[must_use]
    pub fn build_full(workspace_root: &Path) -> Option<LspSnapshot> {
        let ctx = build_context(workspace_root)?;
        Some(Self::from_context(ctx, workspace_root))
    }

    /// As [`Self::build_full`], but ALSO returns a fresh, fully INDEPENDENT
    /// full parse (`parse_snapshot` over the same snapshot — every
    /// source-bearing app, workspace + embedded-source deps) for T3 Task 9's
    /// incremental updater (`src/lsp/updater.rs`) to own as its mutable
    /// working state.
    ///
    /// This can't just hand back `ctx.parsed` (the parse [`Self::from_context`]
    /// itself consumes below) because `AlFile` carries no `Clone` impl — one
    /// parsed `AlFile`/text can only ever have ONE owner. `LspSnapshot::
    /// parsed`'s per-file `ParsedFileEntry` needs independent ownership for
    /// the immutable PUBLISHED snapshot; the updater's `Vec<ParsedUnit>`
    /// needs independent ownership for its own MUTABLE working copy (rung 1
    /// splices one file's fresh parse into it; rung 2 replaces the primary
    /// unit's file list; rung 3 replaces the whole `Vec` wholesale — see
    /// updater.rs). A second, fully independent `parse_snapshot` pass is the
    /// honest fix, not a workaround: it runs once here (server startup) and
    /// again only on a rung-3 rebuild (rare — `.alpackages` change / watcher
    /// overflow), never on the rung-1/rung-2 hot path.
    ///
    /// `pub` (T3 Task 10, widened from `pub(crate)`): the permanent
    /// incremental-vs-batch differential gate (`tests/lsp_incremental_parity.rs`)
    /// is an external integration-test crate — it needs this to construct an
    /// [`Updater`](crate::lsp::updater::Updater) exactly as `main.rs`/
    /// `server.rs` eventually will, so this is the arc's real future public
    /// server-construction surface, not test-only scaffolding.
    #[must_use]
    pub fn build_full_with_parsed(workspace_root: &Path) -> Option<(LspSnapshot, Vec<ParsedUnit>)> {
        let ctx = build_context(workspace_root)?;
        let parsed_for_updater = parse_snapshot(&ctx.snap);
        Some((Self::from_context(ctx, workspace_root), parsed_for_updater))
    }

    /// The composition shared by [`Self::build_full`]/
    /// [`Self::build_full_with_parsed`]: dep layer → assemble → resolve per
    /// file → derive indexes, given an already-built [`ProgramContext`].
    ///
    /// `pub(crate)` (T3 Task 11): `handlers.rs`'s own tests construct a
    /// two-app (workspace + embedded-source dependency) [`ProgramContext`]
    /// by hand — mirroring `program::build`'s in-memory layer-split fixture
    /// pattern — and call this directly, the same way `build_full`/
    /// `build_full_with_parsed` do, rather than re-implementing this
    /// composition a second time just to exercise it without disk I/O.
    pub(crate) fn from_context(ctx: ProgramContext, workspace_root: &Path) -> LspSnapshot {
        let ProgramContext {
            snap,
            graph,
            parsed,
            primary_app_ref,
            ws_file_set,
            dep_layer,
        } = ctx;

        // Locate the ONE primary (workspace) `ParsedUnit` — `snap.apps` is
        // GUID-deduped upstream, so at most one can match (mirrors
        // `build_context`'s own find-or-synthesize, but a workspace with zero
        // source files never reaches here anyway: `ws_file_set` would be
        // empty and every loop below is a no-op).
        let primary_unit_idx = parsed.iter().position(|u| u.app == snap.workspace_app);

        // ── Transient borrow phase: index/body_map borrow `graph`/`parsed`,
        // and per the module's ownership law must never survive into
        // `LspSnapshot` — everything they produce is copied into owned data
        // (or, for `pf.file`/`pf.text`, deferred to the ownership-move phase
        // below, since `AlFile` is not `Clone` and `body_map` borrows `parsed`
        // for as long as it's alive).
        let mut edges_by_file: HashMap<String, Arc<Vec<ClassifiedEdge>>> = HashMap::new();
        let mut surfaces_by_file: HashMap<String, DefSurface> = HashMap::new();
        let mut decls_by_file: HashMap<String, Arc<Vec<DeclEntry>>> = HashMap::new();
        let event_edges: Arc<Vec<ClassifiedEdge>>;
        let dep_decl_by_id: HashMap<RoutineNodeId, DeclEntry>;
        let dep_texts: HashMap<(AppRef, String), Arc<str>>;

        {
            let obj_node_map: HashMap<ObjectNodeId, &ObjectNode> =
                graph.objects.iter().map(|o| (o.id.clone(), o)).collect();
            let index = ResolveIndex::build(&graph);
            let body_map = BodyMap::build(&graph, &parsed);

            if let Some(idx) = primary_unit_idx {
                for pf in &parsed[idx].files {
                    if !ws_file_set.contains(&pf.virtual_path) {
                        continue;
                    }

                    let (edges, surface, decls) = recompute_file(
                        pf,
                        primary_app_ref,
                        &graph,
                        &index,
                        &body_map,
                        &obj_node_map,
                    );
                    edges_by_file.insert(pf.virtual_path.clone(), Arc::new(edges));
                    surfaces_by_file.insert(pf.virtual_path.clone(), surface);
                    decls_by_file.insert(pf.virtual_path.clone(), Arc::new(decls));
                }
            }

            let raw_event_edges = emit_event_flow_edges(&graph, &index, &body_map);
            event_edges = Arc::new(
                raw_event_edges
                    .into_iter()
                    .map(|edge| ClassifiedEdge {
                        obligation_id: ObligationId::Publisher(edge.from.clone()),
                        edge,
                    })
                    .collect(),
            );

            (dep_decl_by_id, dep_texts) =
                build_dep_indexes(&graph, &body_map, &parsed, primary_app_ref);
            // `index`/`body_map`/`obj_node_map` drop here, at the end of this
            // block — their borrows of `graph`/`parsed` end before the
            // ownership-move phase below needs to consume `parsed` by value.
        }

        let (incoming, publisher_fanout) = build_incoming(&edges_by_file, &event_edges);
        let decl_by_id = build_decl_by_id(&decls_by_file);

        // ── Ownership-move phase: consume `parsed` by value, pairing each
        // workspace file's already-computed `DefSurface` with its owned
        // `AlFile`/text (never cloned — `AlFile` carries no `Clone` impl, by
        // design: the IR arena is exactly as large as the source file and
        // cloning it on every snapshot build would be wasteful).
        let mut parsed_files: HashMap<String, Arc<ParsedFileEntry>> = HashMap::new();
        if let Some(idx) = primary_unit_idx {
            for pf in parsed.into_iter().nth(idx).expect("idx is in bounds").files {
                if !ws_file_set.contains(&pf.virtual_path) {
                    continue;
                }
                let surface = surfaces_by_file
                    .remove(&pf.virtual_path)
                    .expect("a surface was computed for every ws_file_set member above");
                parsed_files.insert(
                    pf.virtual_path.clone(),
                    Arc::new(ParsedFileEntry {
                        file: pf.file,
                        text: pf.text,
                        virtual_path: pf.virtual_path,
                        surface,
                    }),
                );
            }
        }

        LspSnapshot {
            generation: 0,
            graph: Arc::new(graph),
            dep_layer: Arc::new(dep_layer),
            snap: Arc::new(snap),
            parsed: parsed_files,
            edges_by_file,
            event_edges,
            incoming,
            publisher_fanout,
            decls_by_file,
            decl_by_id,
            dep_decl_by_id: Arc::new(dep_decl_by_id),
            dep_texts: Arc::new(dep_texts),
            workspace_root: Arc::new(crate::protocol::normalize_path(workspace_root)),
        }
    }

    /// Position lookup: file + 0-based line + UTF-8 byte col → routine whose
    /// `name_origin` or whole-decl `origin` contains it (name hit preferred).
    ///
    /// `line`/`byte_col` share [`al_syntax::ir::Point`]'s own semantics
    /// (`column` is a UTF-8 byte column within the line) — no encoding
    /// conversion needed; compare directly against `Origin.start`/`.end`.
    #[must_use]
    pub fn decl_at(&self, virtual_path: &str, line: u32, byte_col: u32) -> Option<&DeclEntry> {
        let decls = self.decls_by_file.get(virtual_path)?;
        let pos = (line, byte_col);

        // Name hit, preferred: an exact click on the symbol's own name token.
        if let Some(d) = decls.iter().find(|d| point_in_origin(pos, &d.name_origin)) {
            return Some(d);
        }
        // Whole-decl (body) hit fallback.
        decls.iter().find(|d| point_in_origin(pos, &d.origin))
    }

    /// Look up one classified edge by its [`EdgeRef`].
    #[must_use]
    pub fn edge(&self, r: &EdgeRef) -> &ClassifiedEdge {
        if r.file == EVENT_EDGES_KEY {
            &self.event_edges[r.idx as usize]
        } else {
            &self.edges_by_file[&r.file][r.idx as usize]
        }
    }

    /// Resolve ANY `RoutineNodeId` — workspace OR dependency — to its live
    /// decl entry plus the source text needed for position-encoding
    /// conversion (`LineTable::new(text)`). The one lookup handlers.rs uses
    /// for every position-bearing `RouteTarget::Routine(id)` surface, so a
    /// caller never needs to know which of [`Self::decl_by_id`]/
    /// [`Self::dep_decl_by_id`] actually holds `id`. Returns `None` for a
    /// stale id (not in either map) — the fail-closed "never guess" contract
    /// every handler built on this must honor.
    #[must_use]
    pub fn decl_and_text(&self, id: &RoutineNodeId) -> Option<(&DeclEntry, &str)> {
        if let Some(d) = self.decl_by_id.get(id) {
            let text: &str = &self.parsed.get(&d.virtual_path)?.text;
            return Some((d, text));
        }
        let d = self.dep_decl_by_id.get(id)?;
        let text = self
            .dep_texts
            .get(&(id.object.app, d.virtual_path.clone()))?;
        Some((d, text.as_ref()))
    }
}

/// `true` when the half-open span `[origin.start, origin.end)` — compared as
/// `(row, column)` tuples, matching source-span containment (a later line
/// always sorts after an earlier one; same-line spans compare by column) —
/// contains `pos`.
fn point_in_origin(pos: (u32, u32), origin: &al_syntax::ir::Origin) -> bool {
    let start = (origin.start.row, origin.start.column);
    let end = (origin.end.row, origin.end.column);
    pos >= start && pos < end
}

/// O(E) wholesale rebuild — NEVER incrementally edited (spec §3 law / H-10
/// lesson: a stale incrementally-patched index is exactly the bug class that
/// law exists to rule out).
///
/// `Incoming(S)` gets: every `Call`/`Run`/`ImplicitTrigger` edge with a route
/// `RouteTarget::Routine(S)` (from `edges_by_file`), AND every `EventFlow`
/// edge from publisher `P` with a route targeting `S` (from `event_edges` —
/// event direction: `P` calls `S`). Both populations are scanned uniformly:
/// every route on every edge (matching `Edge::all_routes`'s RESOLUTION-context
/// semantics, not a reachability filter — an LSP "incoming calls" view is
/// meant to show every statically-possible caller, including one gated behind
/// `ManualBinding`/`AmbiguousDispatch`, not just the unconditionally-firing
/// subset `Edge::default_reachable_routes` would give).
///
/// Returns `(incoming, publisher_fanout)` — see [`LspSnapshot::publisher_fanout`]'s
/// doc for why the second map is precomputed HERE, in the SAME loop over
/// `event_edges` this function already runs, rather than via a separate pass
/// (t3 whole-branch review, blocker fix): `publisher_fanout[P]` is the sum of
/// `routes.len()` over every `event_edges` entry whose `edge.from == P` —
/// the REAL resolved-subscriber count, never mere edge presence (an
/// `emit_event_flow_edges` publisher entry always exists even with zero
/// subscribers, so counting entries rather than summing routes would
/// overcount an unsubscribed publisher as "used").
#[must_use]
pub fn build_incoming(
    edges_by_file: &HashMap<String, Arc<Vec<ClassifiedEdge>>>,
    event_edges: &[ClassifiedEdge],
) -> (
    HashMap<RoutineNodeId, Vec<EdgeRef>>,
    HashMap<RoutineNodeId, usize>,
) {
    let mut incoming: HashMap<RoutineNodeId, Vec<EdgeRef>> = HashMap::new();

    for (file, edges) in edges_by_file {
        for (idx, ce) in edges.iter().enumerate() {
            push_edge_targets(&mut incoming, &ce.edge, file, idx as u32);
        }
    }

    let mut publisher_fanout: HashMap<RoutineNodeId, usize> = HashMap::new();
    for (idx, ce) in event_edges.iter().enumerate() {
        push_edge_targets(&mut incoming, &ce.edge, EVENT_EDGES_KEY, idx as u32);
        if !ce.edge.routes.is_empty() {
            *publisher_fanout.entry(ce.edge.from.clone()).or_insert(0) += ce.edge.routes.len();
        }
    }

    (incoming, publisher_fanout)
}

/// Push one [`EdgeRef`] per DISTINCT `RouteTarget::Routine` target `edge`
/// resolves to (T3 Task 9 review carry-over from Task 8: a single edge can
/// carry >1 route to the exact SAME target — e.g. a pathological
/// ambiguous-overload candidate set where two routes happen to name the
/// same routine — and without this per-edge dedup guard, `incoming[target]`
/// would carry the IDENTICAL `EdgeRef` more than once: pure noise for a
/// consumer, e.g. `incomingCalls`' `fromRanges` showing the same call site
/// twice for no reason). Routes from a DIFFERENT edge naming the same
/// target are NOT deduplicated — those are genuinely distinct callers (a
/// different `idx`), never touched by this guard.
fn push_edge_targets(
    incoming: &mut HashMap<RoutineNodeId, Vec<EdgeRef>>,
    edge: &Edge,
    file: &str,
    idx: u32,
) {
    let mut seen_this_edge: Vec<&RoutineNodeId> = Vec::new();
    for route in &edge.routes {
        if let RouteTarget::Routine(target) = &route.target {
            if seen_this_edge.contains(&target) {
                continue;
            }
            seen_this_edge.push(target);
            incoming.entry(target.clone()).or_default().push(EdgeRef {
                file: file.to_string(),
                idx,
            });
        }
    }
}

/// One workspace file's contribution to a snapshot: its resolved edge list,
/// definition-surface fingerprint, and (sorted) decl list. Shared by
/// [`LspSnapshot::from_context`]'s whole-batch build loop and the
/// incremental updater's rung-1 (one file) / rung-2 (every file) per-file
/// recompute (`src/lsp/updater.rs`) — the ONE place "what a file
/// contributes to a snapshot" is defined, so the batch and incremental paths
/// can never drift apart.
#[must_use]
pub(crate) fn recompute_file(
    pf: &ParsedFile,
    primary_app_ref: AppRef,
    graph: &ProgramGraph,
    index: &ResolveIndex,
    body_map: &BodyMap<'_>,
    obj_node_map: &HashMap<ObjectNodeId, &ObjectNode>,
) -> (Vec<ClassifiedEdge>, DefSurface, Vec<DeclEntry>) {
    let file_res = crate::program::resolve::full::resolve_file_obligations(
        pf,
        primary_app_ref,
        graph,
        index,
        body_map,
        obj_node_map,
    );
    let surface = def_surface_fingerprint(pf);

    let mut decls: Vec<DeclEntry> = Vec::new();
    for obj in &pf.file.objects {
        let obj_key = match obj.id {
            Some(n) => ObjKey::Id(n),
            None => ObjKey::Name(obj.name.to_ascii_lowercase()),
        };
        let obj_node_id = ObjectNodeId {
            app: primary_app_ref,
            kind: obj.kind,
            key: obj_key,
        };
        for routine in &obj.routines {
            let id = source_routine_node_id(obj_node_id.clone(), routine);
            decls.push(DeclEntry {
                id,
                name: routine.name.clone(),
                origin: routine.origin.clone(),
                name_origin: routine.name_origin.clone(),
                virtual_path: pf.virtual_path.clone(),
            });
        }
    }
    decls.sort_by_key(|d| d.origin.byte.start);

    (file_res.edges, surface, decls)
}

/// DERIVED index (see [`LspSnapshot::decl_by_id`]'s doc): every `DeclEntry`
/// across every file, keyed by its `RoutineNodeId`. Always rebuilt WHOLESALE
/// from `decls_by_file` — never cloned-then-patched — mirroring
/// [`build_incoming`]'s own H-10-law rebuild pattern.
#[must_use]
pub(crate) fn build_decl_by_id(
    decls_by_file: &HashMap<String, Arc<Vec<DeclEntry>>>,
) -> HashMap<RoutineNodeId, DeclEntry> {
    let mut decl_by_id = HashMap::new();
    for decls in decls_by_file.values() {
        for d in decls.iter() {
            decl_by_id.insert(d.id.clone(), d.clone());
        }
    }
    decl_by_id
}

/// Build [`LspSnapshot::dep_decl_by_id`]/[`LspSnapshot::dep_texts`] — the
/// dependency-app counterpart of `decl_by_id`/`parsed`, which stay
/// workspace-only (see their own docs). Walks `graph.routines` (every app,
/// pre-sorted at graph-build time) rather than `parsed`'s per-file objects,
/// since `graph.routines` already carries the exact node identity any edge's
/// `RouteTarget::Routine(id)` names — skipping the primary (workspace) app
/// (already covered by `decl_by_id`) and any routine `body_map` can't find a
/// decl for (a `SymbolOnly` boundary routine with no embedded source — no
/// position exists to serve, so no entry is produced; see
/// `resolver::make_routine_route`'s doc for why `Routine(id)` is only ever
/// constructed when this SAME `body_map` lookup just succeeded).
///
/// Called from BOTH [`LspSnapshot::from_context`] and
/// [`crate::lsp::updater::Updater::apply_rung2`] (the two places that ever
/// rebuild `graph`/`body_map` from scratch) — rung 1 never calls this,
/// `Arc::clone`-ing the previous snapshot's values forward instead, since a
/// body-only workspace edit can never touch dependency source (see
/// `dep_decl_by_id`'s doc).
#[must_use]
pub(crate) fn build_dep_indexes(
    graph: &ProgramGraph,
    body_map: &BodyMap<'_>,
    parsed: &[ParsedUnit],
    primary_app: AppRef,
) -> (DepDeclById, DepTexts) {
    let mut dep_decl_by_id: HashMap<RoutineNodeId, DeclEntry> = HashMap::new();
    for node in &graph.routines {
        if node.id.object.app == primary_app {
            continue;
        }
        if let Some((decl, path)) = body_map.get_with_path(&node.id) {
            dep_decl_by_id.insert(
                node.id.clone(),
                DeclEntry {
                    id: node.id.clone(),
                    name: decl.name.clone(),
                    origin: decl.origin.clone(),
                    name_origin: decl.name_origin.clone(),
                    virtual_path: path.to_string(),
                },
            );
        }
    }

    let mut dep_texts: HashMap<(AppRef, String), Arc<str>> = HashMap::new();
    for unit in parsed {
        let Some(app_ref) = graph.apps.find(&unit.app) else {
            continue;
        };
        if app_ref == primary_app {
            continue;
        }
        for pf in &unit.files {
            dep_texts
                .entry((app_ref, pf.virtual_path.clone()))
                .or_insert_with(|| Arc::clone(&pf.text));
        }
    }

    (dep_decl_by_id, dep_texts)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::program::resolve::edge::{Edge, EdgeKind};
    use crate::program::resolve::full::resolve_full_program;

    /// A fixture workspace exercising: a cross-file call (Alpha.DoWork calls
    /// Beta.Process via a declared `Codeunit "Beta"` local var), a same-name
    /// overload pair (`Alpha.Calc(Integer)` / `Alpha.Calc(Text)`), an event
    /// publisher/subscriber pair (`Beta.OnAfterProcess` / `Gamma.
    /// HandleAfterProcess`), and a non-ASCII (Danish) identifier (`Løbenr`) —
    /// per the task brief's Step-1 fixture requirements.
    fn write_fixture_workspace(dir: &std::path::Path) {
        std::fs::write(
            dir.join("app.json"),
            r#"{
    "id": "33333333-0000-0000-0000-000000000008",
    "name": "Task8 LspSnapshot Fixture",
    "publisher": "probe",
    "version": "1.0.0.0"
}"#,
        )
        .expect("write app.json");

        std::fs::write(
            dir.join("Alpha.al"),
            r#"codeunit 50100 "Alpha"
{
    procedure DoWork()
    var
        Beta: Codeunit "Beta";
    begin
        Beta.Process();
        Calc(1);
        Calc('x');
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
}
"#,
        )
        .expect("write Alpha.al");

        std::fs::write(
            dir.join("Beta.al"),
            r#"codeunit 50101 "Beta"
{
    procedure Process()
    begin
    end;

    [IntegrationEvent(false, false)]
    procedure OnAfterProcess()
    begin
    end;
}
"#,
        )
        .expect("write Beta.al");

        std::fs::write(
            dir.join("Gamma.al"),
            r#"codeunit 50102 "Gamma"
{
    [EventSubscriber(ObjectType::Codeunit, Codeunit::"Beta", 'OnAfterProcess', '', false, false)]
    local procedure HandleAfterProcess()
    begin
    end;
}
"#,
        )
        .expect("write Gamma.al");
    }

    fn fixture_dir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        write_fixture_workspace(dir.path());
        dir
    }

    // ── build_full: union equals a direct resolve_full_program run ────────

    #[test]
    fn build_full_edges_match_resolve_full_program() {
        let dir = fixture_dir();
        let snap = LspSnapshot::build_full(dir.path()).expect("build_full");
        let report = resolve_full_program(dir.path()).expect("resolve_full_program");

        let mut got: Vec<Edge> = snap
            .edges_by_file
            .values()
            .flat_map(|v| v.iter().map(|ce| ce.edge.clone()))
            .collect();
        got.extend(snap.event_edges.iter().map(|ce| ce.edge.clone()));
        got.sort();

        let mut want: Vec<Edge> = report.edges.into_iter().map(|ce| ce.edge).collect();
        want.sort();

        assert_eq!(
            got, want,
            "build_full's edges_by_file + event_edges union must equal a \
             direct resolve_full_program run (order-insensitive)"
        );
        assert!(!got.is_empty(), "fixture must produce real edges");
    }

    // ── determinism across two builds (generation excluded) ───────────────

    #[test]
    fn build_full_is_deterministic_across_two_builds() {
        let dir = fixture_dir();
        let s1 = LspSnapshot::build_full(dir.path()).expect("build 1");
        let s2 = LspSnapshot::build_full(dir.path()).expect("build 2");

        let mut files1: Vec<_> = s1.decls_by_file.keys().cloned().collect();
        let mut files2: Vec<_> = s2.decls_by_file.keys().cloned().collect();
        files1.sort();
        files2.sort();
        assert_eq!(files1, files2, "same workspace file set");
        for f in &files1 {
            let ids1: Vec<_> = s1.decls_by_file[f].iter().map(|d| d.id.clone()).collect();
            let ids2: Vec<_> = s2.decls_by_file[f].iter().map(|d| d.id.clone()).collect();
            assert_eq!(ids1, ids2, "file {f}: same decl ids in the same order");
        }

        let mut ef1: Vec<_> = s1.edges_by_file.keys().cloned().collect();
        let mut ef2: Vec<_> = s2.edges_by_file.keys().cloned().collect();
        ef1.sort();
        ef2.sort();
        assert_eq!(ef1, ef2);
        for f in &ef1 {
            let mut a: Vec<Edge> = s1.edges_by_file[f]
                .iter()
                .map(|ce| ce.edge.clone())
                .collect();
            let mut b: Vec<Edge> = s2.edges_by_file[f]
                .iter()
                .map(|ce| ce.edge.clone())
                .collect();
            a.sort();
            b.sort();
            assert_eq!(a, b, "file {f}: same edge set");
        }

        let mut e1: Vec<Edge> = s1.event_edges.iter().map(|ce| ce.edge.clone()).collect();
        let mut e2: Vec<Edge> = s2.event_edges.iter().map(|ce| ce.edge.clone()).collect();
        e1.sort();
        e2.sort();
        assert_eq!(e1, e2, "same event-edge set");

        let mut inc1: Vec<_> = s1
            .incoming
            .iter()
            .map(|(k, v)| {
                let mut v = v.clone();
                v.sort_by(|a, b| (a.file.as_str(), a.idx).cmp(&(b.file.as_str(), b.idx)));
                (k.clone(), v)
            })
            .collect();
        let mut inc2: Vec<_> = s2
            .incoming
            .iter()
            .map(|(k, v)| {
                let mut v = v.clone();
                v.sort_by(|a, b| (a.file.as_str(), a.idx).cmp(&(b.file.as_str(), b.idx)));
                (k.clone(), v)
            })
            .collect();
        inc1.sort_by(|a, b| a.0.cmp(&b.0));
        inc2.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(inc1, inc2, "same incoming index (generation excluded)");
    }

    // ── decl_at: name hit, body-fallback hit, and none ─────────────────────

    #[test]
    fn decl_at_hits_name_then_falls_back_to_whole_decl_then_none() {
        let dir = fixture_dir();
        let snap = LspSnapshot::build_full(dir.path()).expect("build_full");

        let alpha_decls = snap
            .decls_by_file
            .get("Alpha.al")
            .expect("Alpha.al must be indexed");
        let lobenr = alpha_decls
            .iter()
            .find(|d| d.name == "Løbenr")
            .expect("Løbenr decl must be present (non-ASCII identifier fixture)");

        // Name hit: querying exactly at the name token's start must resolve
        // to Løbenr's own DeclEntry.
        let hit = snap
            .decl_at(
                "Alpha.al",
                lobenr.name_origin.start.row,
                lobenr.name_origin.start.column,
            )
            .expect("name-position hit");
        assert_eq!(hit.id, lobenr.id);

        // Whole-decl (body) hit: `origin.start` precedes `name_origin.start`
        // (the `procedure` keyword comes before the name token), so this
        // point is inside `origin` but outside `name_origin` — exercising the
        // fallback arm specifically.
        assert!(
            (lobenr.origin.start.row, lobenr.origin.start.column)
                < (
                    lobenr.name_origin.start.row,
                    lobenr.name_origin.start.column
                ),
            "fixture assumption: origin must start before name_origin"
        );
        let body_hit = snap
            .decl_at(
                "Alpha.al",
                lobenr.origin.start.row,
                lobenr.origin.start.column,
            )
            .expect("whole-decl-position hit");
        assert_eq!(body_hit.id, lobenr.id);

        // None: a position far past the end of the file, and an unknown file.
        assert!(snap.decl_at("Alpha.al", 9_999, 0).is_none());
        assert!(snap.decl_at("NoSuchFile.al", 0, 0).is_none());
    }

    // ── build_incoming: cross-file caller + event subscriber's publisher ──

    #[test]
    fn build_incoming_finds_cross_file_caller_and_event_subscriber_publisher() {
        let dir = fixture_dir();
        let snap = LspSnapshot::build_full(dir.path()).expect("build_full");

        let beta_process = snap.decls_by_file["Beta.al"]
            .iter()
            .find(|d| d.name == "Process")
            .expect("Beta.Process decl")
            .id
            .clone();
        let incoming_process = snap
            .incoming
            .get(&beta_process)
            .expect("Beta.Process must have an incoming caller");
        assert!(
            incoming_process.iter().any(|r| r.file == "Alpha.al"),
            "Alpha.DoWork's cross-file call must be indexed as incoming to \
             Beta.Process; got {incoming_process:?}"
        );
        for r in incoming_process.iter().filter(|r| r.file == "Alpha.al") {
            let ce = snap.edge(r);
            assert!(
                ce.edge.routes.iter().any(
                    |route| matches!(&route.target, RouteTarget::Routine(t) if *t == beta_process)
                ),
                "the referenced edge must actually route to Beta.Process"
            );
        }

        let gamma_sub = snap.decls_by_file["Gamma.al"]
            .iter()
            .find(|d| d.name == "HandleAfterProcess")
            .expect("Gamma.HandleAfterProcess decl")
            .id
            .clone();
        let incoming_sub = snap
            .incoming
            .get(&gamma_sub)
            .expect("the subscriber must have an incoming publisher edge");
        assert!(
            incoming_sub.iter().any(|r| r.file == EVENT_EDGES_KEY),
            "the event edge must be indexed under the reserved event-edges \
             key; got {incoming_sub:?}"
        );
        for r in incoming_sub.iter().filter(|r| r.file == EVENT_EDGES_KEY) {
            let ce = snap.edge(r);
            assert_eq!(ce.edge.kind, EdgeKind::EventFlow);
        }
    }

    // ── publisher_fanout: precomputed, O(1)-lookupable (t3 whole-branch ───
    // ── review blocker fix — see LspSnapshot::publisher_fanout's doc) ──────

    #[test]
    fn publisher_fanout_counts_real_routes_and_omits_unpublished_routines() {
        let dir = fixture_dir();
        let snap = LspSnapshot::build_full(dir.path()).expect("build_full");

        // Beta.OnAfterProcess is a real publisher with exactly one real
        // subscriber (Gamma.HandleAfterProcess, per the fixture) — its
        // publisher_fanout entry must equal 1, matching the OLD
        // effective_incoming_count's `event_edges.iter().filter(from ==
        // id).map(routes.len()).sum()` computation exactly (same value, now
        // precomputed instead of scanned per call).
        let beta_on_after_process = snap.decls_by_file["Beta.al"]
            .iter()
            .find(|d| d.name == "OnAfterProcess")
            .expect("Beta.OnAfterProcess decl")
            .id
            .clone();
        assert_eq!(
            snap.publisher_fanout.get(&beta_on_after_process).copied(),
            Some(1),
            "a publisher with exactly one real subscriber must have \
             publisher_fanout == 1"
        );

        // A routine that is NEVER a publisher (Beta.Process, an ordinary
        // procedure) must have NO publisher_fanout entry at all — never a
        // spurious Some(0) that would silently inflate a future consumer's
        // sum by an extra hashmap probe for no reason.
        let beta_process = snap.decls_by_file["Beta.al"]
            .iter()
            .find(|d| d.name == "Process")
            .expect("Beta.Process decl")
            .id
            .clone();
        assert_eq!(
            snap.publisher_fanout.get(&beta_process),
            None,
            "an ordinary (non-publisher) routine must have no publisher_fanout entry"
        );
    }

    #[test]
    fn publisher_fanout_omits_a_publisher_with_zero_real_subscribers() {
        // emit_event_flow_edges emits ONE ClassifiedEdge per publisher
        // declaration UNCONDITIONALLY, even with zero subscribers — mere
        // edge PRESENCE must never count as fan-out (mirrors
        // effective_incoming_count's own "as-publisher fan-out" doc).
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("app.json"),
            r#"{
    "id": "33333333-0000-0000-0000-00000000000f",
    "name": "PublisherFanoutZeroFixture",
    "publisher": "probe",
    "version": "1.0.0.0"
}"#,
        )
        .expect("write app.json");
        std::fs::write(
            dir.path().join("Lonely.al"),
            r#"codeunit 50100 "Lonely"
{
    [IntegrationEvent(false, false)]
    procedure OnNobodyListens()
    begin
    end;
}
"#,
        )
        .expect("write Lonely.al");

        let snap = LspSnapshot::build_full(dir.path()).expect("build_full");
        let publisher = snap.decls_by_file["Lonely.al"]
            .iter()
            .find(|d| d.name == "OnNobodyListens")
            .expect("OnNobodyListens decl")
            .id
            .clone();
        assert_eq!(
            snap.publisher_fanout.get(&publisher),
            None,
            "a publisher with ZERO real subscribers must have no \
             publisher_fanout entry — edge presence alone is never fan-out"
        );
    }

    // ── build_incoming: one edge, 2 routes to the SAME target → 1 EdgeRef ──
    // (T3 Task 9 review carry-over from Task 8: a pathological
    // ambiguous-overload-style edge whose routes list happens to name the
    // same target twice must not produce a duplicate incoming entry.)

    #[test]
    fn build_incoming_dedups_one_edges_repeated_route_to_the_same_target() {
        use crate::program::node::{AppRef, ObjKey, ObjectNodeId};
        use crate::program::resolve::edge::{
            CanonicalSpan, DispatchShape, Evidence, Route, RouteTarget, SetCompleteness, SiteId,
            SourcePos, Witness,
        };
        use al_syntax::ir::ObjectKind;

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

        fn dup_route(target: &RoutineNodeId) -> Route {
            Route {
                target: RouteTarget::Routine(target.clone()),
                evidence: Evidence::Source,
                conditions: vec![],
                witness: Witness::None,
                receiver_tier: None,
            }
        }

        let target = rid("target");
        let caller = rid("caller");
        let edge = Edge {
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
            shape: DispatchShape::AmbiguousOverload,
            completeness: SetCompleteness::Complete,
            // Pathological: the SAME target named twice in one edge's routes.
            routes: vec![dup_route(&target), dup_route(&target)],
        };

        let mut edges_by_file: HashMap<String, Arc<Vec<ClassifiedEdge>>> = HashMap::new();
        edges_by_file.insert(
            "F.al".to_string(),
            Arc::new(vec![ClassifiedEdge {
                obligation_id: ObligationId::CallSite {
                    caller: edge.from.clone(),
                    span: edge.site.span.clone(),
                    callee_fp: edge.site.callee_fingerprint,
                },
                edge,
            }]),
        );

        let (incoming, _fanout) = build_incoming(&edges_by_file, &[]);
        let refs = incoming
            .get(&target)
            .expect("target must have an incoming entry");
        assert_eq!(
            refs.len(),
            1,
            "one edge with 2 routes to the SAME target must produce exactly 1 \
             EdgeRef, not one per route; got {refs:?}"
        );
    }

    // ── build_incoming: TWO DIFFERENT edges to the same target → 2 EdgeRefs ──
    // (review fix-wave item 4: the per-edge dedup guard above must never
    // collapse genuinely distinct callers — only a repeated route WITHIN one
    // edge is deduplicated.)

    #[test]
    fn build_incoming_keeps_two_different_edges_to_the_same_target_as_2_edgerefs() {
        use crate::program::node::{AppRef, ObjKey, ObjectNodeId};
        use crate::program::resolve::edge::{
            CanonicalSpan, DispatchShape, Evidence, Route, RouteTarget, SetCompleteness, SiteId,
            SourcePos, Witness,
        };
        use al_syntax::ir::ObjectKind;

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

        fn single_route_edge(
            caller: RoutineNodeId,
            target: &RoutineNodeId,
            callee_fp: u64,
        ) -> Edge {
            Edge {
                from: caller.clone(),
                site: SiteId {
                    caller,
                    span: CanonicalSpan {
                        unit: "F.al".into(),
                        start: SourcePos { line: 1, col: 1 },
                        end: SourcePos { line: 1, col: 2 },
                    },
                    callee_fingerprint: callee_fp,
                },
                kind: EdgeKind::Call,
                shape: DispatchShape::Exact,
                completeness: SetCompleteness::Complete,
                routes: vec![Route {
                    target: RouteTarget::Routine(target.clone()),
                    evidence: Evidence::Source,
                    conditions: vec![],
                    witness: Witness::None,
                    receiver_tier: None,
                }],
            }
        }

        let target = rid("target");
        let caller_a = rid("caller_a");
        let caller_b = rid("caller_b");
        let edge_a = single_route_edge(caller_a, &target, 1);
        let edge_b = single_route_edge(caller_b, &target, 2);

        let mut edges_by_file: HashMap<String, Arc<Vec<ClassifiedEdge>>> = HashMap::new();
        edges_by_file.insert(
            "F.al".to_string(),
            Arc::new(vec![
                ClassifiedEdge {
                    obligation_id: ObligationId::CallSite {
                        caller: edge_a.from.clone(),
                        span: edge_a.site.span.clone(),
                        callee_fp: edge_a.site.callee_fingerprint,
                    },
                    edge: edge_a,
                },
                ClassifiedEdge {
                    obligation_id: ObligationId::CallSite {
                        caller: edge_b.from.clone(),
                        span: edge_b.site.span.clone(),
                        callee_fp: edge_b.site.callee_fingerprint,
                    },
                    edge: edge_b,
                },
            ]),
        );

        let (incoming, _fanout) = build_incoming(&edges_by_file, &[]);
        let refs = incoming
            .get(&target)
            .expect("target must have incoming entries");
        assert_eq!(
            refs.len(),
            2,
            "two DIFFERENT edges naming the same target must NOT be deduped \
             against each other — got {refs:?}"
        );
        assert_ne!(
            refs[0].idx, refs[1].idx,
            "the two EdgeRefs must point at two distinct edge indices"
        );
    }
}
