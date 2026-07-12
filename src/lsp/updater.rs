//! The incremental updater (T3 Task 9): a debounced, per-path-coalesced
//! event queue feeding a two-rung (plus degenerate rung-3) soundness ladder
//! that produces a fresh [`LspSnapshot`] and atomically swaps it into a
//! [`SharedSnapshot`] — never mutating a published snapshot in place (spec
//! §3, the H-10 lesson).
//!
//! # Mapping from the task brief's signature to this module's shape
//!
//! The brief names a free function `apply_changes(prev: &LspSnapshot, batch:
//! &[ChangeEvent]) -> Option<LspSnapshot>`. This module instead exposes
//! [`Updater::apply_batch`] (`&mut self, cur: &LspSnapshot, batch:
//! &[ChangeEvent]) -> Option<(LspSnapshot, Rung)>`) — the brief's own
//! contingency section explicitly sanctions this restructuring, and the
//! reason is load-bearing, not stylistic (see the next section). The
//! brief's "return/expose the Rung taken (test hook)" requirement is met
//! MORE directly than its suggested `Cell<Rung>` field: `apply_batch` simply
//! returns the [`Rung`] it took as part of its `Option` tuple.
//!
//! # Why a `pending` overlay field, not a straight splice into `parsed`
//!
//! `docs/superpowers/specs/2026-07-12-t3-lsp-migration-design.md` plus
//! `.superpowers/sdd/t3-stage-split.md` measured `ResolveIndex::build` +
//! `BodyMap::build` at ~200-350ms on CDO scale — 2-3.5x rung 1's ENTIRE
//! 100ms budget — so rung 1 must NEVER transiently rebuild them (the
//! brief's "documented contingency," now mandatory). The fix is to cache
//! `ResolveIndex`/`BodyMap` and REUSE them across many consecutive rung-1
//! saves, only rebuilding when a rung-2/3 event actually changes the graph
//! ([`spawn_updater`]'s hot loop does exactly this — see its doc).
//!
//! This collides with a real Rust ownership fact: `BodyMap<'a>` BORROWS
//! `Updater::parsed` for as long as it's alive. If rung 1 spliced a file's
//! fresh parse directly into `self.parsed` (as an earlier draft of this
//! module did), the SECOND rung-1 call reusing the SAME cached `BodyMap`
//! would need `&mut self.parsed` to splice again — which the borrow checker
//! correctly rejects, because `body_map` (built from `&self.parsed`) is
//! still alive and would be invalidated by that mutation.
//!
//! The fix: rung 1 NEVER touches `self.parsed` directly. It records each
//! touched file into `self.pending: HashMap<String, ParsedFile>` instead — a
//! DISJOINT field from `parsed`, so a cached `BodyMap` borrowing `parsed`
//! and a fresh `&mut self.pending` write coexist without conflict (Rust DOES
//! reason about disjoint-field borrows within one function body — this is
//! what lets [`spawn_updater`]'s loop pass `&mut updater.pending` into
//! [`apply_rung1_core`] on every rung-1 call while `index`/`body_map`,
//! built once from `&updater.parsed`, stay alive and cached across many
//! such calls). [`Updater::flush_pending`] folds the overlay into `parsed`
//! whenever a rung-2/3 rebuild is about to consult it (or eagerly, in
//! [`Updater::apply_batch`]'s simple/always-correct path, which doesn't
//! bother caching anything across separate calls and so can flush
//! immediately every time).
//!
//! **Soundness of resolving rung 1's touched file(s) against a STALE
//! (pre-edit) cached `BodyMap`:** per the def-surface audit
//! (`docs/superpowers/specs/2026-07-12-t3-def-surface-audit.md` §3), the
//! ONLY fields any resolution path reads through `BodyMap` are a witness
//! SPAN (never trusted stale by a handler anyway, per that audit's §6.1) and
//! `RoutineDecl::params`/`by_ref`/`parse_incomplete` — pure SIGNATURE data.
//! Rung 1's own gate (`DefSurface` fingerprint unchanged) is EXACTLY the
//! guarantee that this signature data is byte-identical between the OLD and
//! NEW parse of every touched file — so a body_map entry for a touched file
//! that's technically "stale" (pre-this-specific-edit) is field-for-field
//! IDENTICAL to a freshly rebuilt one, for every field any consumer actually
//! reads. This holds transitively across many consecutive rung-1 edits
//! (each individually gated on its own fingerprint-equality check), which is
//! what makes the whole cached-context arrangement sound, not just fast.
//! The touched file's OWN obligations are resolved directly against its
//! fresh [`ParsedFile`] (passed as a plain argument, never looked up through
//! `BodyMap`) — see [`crate::lsp::snapshot::recompute_file`]'s callers.
//!
//! # Rung summary (binding; see the task brief + the def-surface audit for
//! the full justification)
//!
//! - **Rung 1** (every `FileSaved` in the batch is a known workspace file
//!   whose fresh parse is `ParseStatus::Clean` AND whose [`DefSurface`]
//!   fingerprint is unchanged): re-resolve ONLY the touched file(s) — see
//!   [`apply_rung1_core`].
//! - **Rung 2** (a `FileRemoved`, a brand-new file, OR any fingerprint
//!   change, OR a `Recovered` parse — doubt fails toward this rung, never
//!   toward silently taking rung 1): rebuild the workspace layer
//!   (`assemble_program_graph` over the cached, UNCHANGED [`DepLayer`]) and
//!   re-resolve EVERY workspace file — see [`Updater::apply_rung2`].
//! - **Rung 3** (`DepsChanged`/`Overflow`, OR a `FileSaved`/`FileRemoved`
//!   path that isn't workspace-shaped at all — e.g. under `.alpackages/`,
//!   the Task-4-review dep-file-boundary scenario): full rebuild via
//!   [`LspSnapshot::build_full_with_parsed`] — see [`Updater::apply_rung3`].
//! - **Batch semantics** (binding): one coalesced batch may name several
//!   files; the rung actually taken is the MAX rung any single event in the
//!   batch requires (a single rebuild serves the whole batch — never one
//!   rebuild per event).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, RwLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use al_syntax::ir::ParseStatus;
use log::warn;

use crate::lsp::def_surface::def_surface_fingerprint;
use crate::lsp::snapshot::{
    DeclEntry, LspSnapshot, ParsedFileEntry, build_decl_by_id, build_dep_indexes, build_incoming,
    recompute_file,
};
use crate::program::assemble_program_graph;
use crate::program::node::ObjectNodeId;
use crate::program::node_extract::ObjectNode;
use crate::program::resolve::body_map::BodyMap;
use crate::program::resolve::emit_event_flow_edges;
use crate::program::resolve::full::{ClassifiedEdge, ObligationId};
use crate::program::resolve::index::ResolveIndex;
use crate::snapshot::{AppSetSnapshot, ParsedFile, ParsedUnit, Provenance, TrustTier};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Swap-only publication point: readers clone the `Arc` (sub-microsecond,
/// never blocked by a writer for longer than that clone); the ONE writer
/// (the updater thread) replaces the whole `Arc` atomically. Never mutated
/// in place (spec §3 / H-10 lesson).
pub struct SharedSnapshot(RwLock<Arc<LspSnapshot>>);

impl SharedSnapshot {
    #[must_use]
    pub fn new(initial: Arc<LspSnapshot>) -> Self {
        SharedSnapshot(RwLock::new(initial))
    }

    /// Cheap: an `Arc` clone under a read lock.
    #[must_use]
    pub fn get(&self) -> Arc<LspSnapshot> {
        Arc::clone(
            &self
                .0
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
    }

    /// Publish a new snapshot. The old `Arc` is dropped once every existing
    /// reader's clone goes out of scope — no reader ever observes a torn
    /// state.
    pub fn swap(&self, s: Arc<LspSnapshot>) {
        *self
            .0
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = s;
    }
}

/// One coalesced input event. `FileSaved`/`FileRemoved` carry an ABSOLUTE
/// filesystem path (the caller — `didSave`/watcher wiring, a later task —
/// is responsible for handing over paths that share a consistent prefix
/// with the `workspace_root` an [`Updater`] was constructed with; this
/// module does no canonicalization of its own, since `FileRemoved`'s path
/// may no longer exist on disk by the time it's processed, and
/// `Path::canonicalize` requires existence).
#[derive(Clone, Debug)]
pub enum ChangeEvent {
    FileSaved(PathBuf),
    FileRemoved(PathBuf),
    DepsChanged,
    Overflow,
}

/// Which rung an apply actually took — the brief's "test hook," exposed
/// directly via [`Updater::apply_batch`]'s return value rather than a
/// separate `Cell` field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rung {
    One,
    Two,
    Three,
}

/// The incremental updater's owned, long-lived working state. Constructed
/// once (typically from [`LspSnapshot::build_full_with_parsed`]'s second
/// return value) and then driven by a sequence of [`Updater::apply_batch`]
/// calls (simple, self-contained, always-correct — used directly by tests
/// and any caller that doesn't need cross-call caching) or by
/// [`spawn_updater`]'s optimized hot loop (which reuses a cached
/// `ResolveIndex`/`BodyMap` pair across many consecutive rung-1 calls — see
/// the module doc's "why a `pending` overlay" section for why that requires
/// going through [`apply_rung1_core`] directly instead of `apply_batch`).
pub struct Updater {
    workspace_root: PathBuf,
    /// Every source-bearing app's parse AS OF THE LAST rung-2/3 rebuild —
    /// workspace AND embedded-source deps. A transient `ResolveIndex`/
    /// `BodyMap` pair borrows this; rung 1 never mutates it directly (see
    /// `pending` below).
    parsed: Vec<ParsedUnit>,
    /// Rung-1 edits recorded since `parsed` was last fully rebuilt — see the
    /// module doc. [`Updater::flush_pending`] folds this into `parsed`
    /// (always empty immediately after any `apply_batch` call returns;
    /// `spawn_updater`'s hot loop instead leaves it un-flushed across many
    /// consecutive rung-1 applies, flushing only right before a rung-2/3
    /// rebuild needs `parsed` to be current).
    pending: HashMap<String, ParsedFile>,
}

/// The classification outcome for one coalesced batch — shared by
/// [`Updater::apply_batch`] and [`spawn_updater`]'s hot loop so the two
/// paths can never disagree about which rung a batch requires.
enum Decision {
    /// Every event was resolved as a fingerprint-equal `FileSaved` on a
    /// known workspace file — `(virtual_path, fresh ParsedFile)` pairs.
    Rung1(Vec<(String, ParsedFile)>),
    Rung2(Vec<Planned>),
    Rung3,
    /// Every event in the batch was a no-op (e.g. an IO race on every
    /// `FileSaved` — see [`Updater::classify`]).
    Noop,
}

impl Updater {
    #[must_use]
    pub fn new(workspace_root: PathBuf, parsed: Vec<ParsedUnit>) -> Self {
        Updater {
            workspace_root,
            parsed,
            pending: HashMap::new(),
        }
    }

    /// The brief's pure/testable synchronous core. Flushes any accumulated
    /// `pending` overlay into `self.parsed` first (a no-op unless this
    /// `Updater` was ALSO driven by the optimized hot loop in between calls,
    /// which is not the expected usage — this method always leaves
    /// `self.parsed` fully up to date and `self.pending` empty when it
    /// returns, so repeated stand-alone calls stay self-consistent without
    /// requiring the caller to manage `pending` at all).
    ///
    /// Builds a FRESH `ResolveIndex`/`BodyMap` for whichever rung it takes —
    /// simple and always correct, at the cost of not reusing them across
    /// SEPARATE `apply_batch` calls. [`spawn_updater`]'s hot loop is the
    /// OPTIMIZED path that DOES cache them across many consecutive rung-1
    /// calls (calling [`apply_rung1_core`] directly), which is the
    /// arrangement the CDO-scale rung-1 budget actually requires.
    pub fn apply_batch(
        &mut self,
        cur: &LspSnapshot,
        batch: &[ChangeEvent],
    ) -> Option<(LspSnapshot, Rung)> {
        self.flush_pending(&cur.snap);

        match self.classify(cur, batch) {
            Decision::Noop => None,
            Decision::Rung1(saves) => {
                let index = ResolveIndex::build(&cur.graph);
                let body_map = BodyMap::build(&cur.graph, &self.parsed);
                let obj_node_map: HashMap<ObjectNodeId, &ObjectNode> = cur
                    .graph
                    .objects
                    .iter()
                    .map(|o| (o.id.clone(), o))
                    .collect();
                let snapshot = apply_rung1_core(
                    cur,
                    saves,
                    &index,
                    &body_map,
                    &obj_node_map,
                    &mut self.pending,
                );
                drop(body_map);
                self.flush_pending(&cur.snap);
                Some((snapshot, Rung::One))
            }
            Decision::Rung2(planned) => Some((self.apply_rung2(cur, planned), Rung::Two)),
            Decision::Rung3 => self.apply_rung3(cur),
        }
    }

    /// Classify one batch against `cur` (the currently-published snapshot),
    /// per file/event, escalating per the module doc's rung summary.
    /// Read-only (`&self`) — never mutates `self`, so it composes freely
    /// with an outstanding `BodyMap` borrow of `self.parsed`.
    fn classify(&self, cur: &LspSnapshot, batch: &[ChangeEvent]) -> Decision {
        if batch.is_empty() {
            return Decision::Noop;
        }

        let mut planned: Vec<Planned> = Vec::new();
        let mut force_rung3 = false;

        for ev in batch {
            match ev {
                ChangeEvent::DepsChanged | ChangeEvent::Overflow => force_rung3 = true,
                ChangeEvent::FileRemoved(path) => match classify_path(&self.workspace_root, path) {
                    PathClass::Workspace(vp) => planned.push(Planned::Remove { vp }),
                    // Task-4 review Hunt-3 scenario: a path that isn't
                    // workspace-shaped at all (e.g. under `.alpackages/`) —
                    // we have no rung-2 primitive for "one dependency file
                    // changed" (rung 2 only ever touches the WORKSPACE
                    // ParsedUnit, over an unchanged cached `DepLayer`), so
                    // the only sound response is the same one `DepsChanged`
                    // gets.
                    PathClass::NotWorkspaceSource => force_rung3 = true,
                },
                ChangeEvent::FileSaved(path) => match classify_path(&self.workspace_root, path) {
                    PathClass::NotWorkspaceSource => force_rung3 = true,
                    PathClass::Workspace(vp) => {
                        // A rare race (saved-then-deleted between the event
                        // firing and this batch being processed): skip THIS
                        // file only — fail-closed does not mean "never make
                        // progress," it means "never fabricate content";
                        // leaving the file's last-known-good state untouched
                        // satisfies that without discarding the rest of a
                        // legitimate batch.
                        let Ok(text) = std::fs::read_to_string(path) else {
                            continue;
                        };
                        let provenance = self.file_provenance(cur, &vp);
                        let file = al_syntax::parse(&text);
                        // Fail-closed: a `Recovered` parse cannot be trusted
                        // for rung 1's fingerprint-equality shortcut — the
                        // IR may have silently dropped content (see
                        // `crate::snapshot::parse::recovered_file_paths`'s
                        // doc), so force this file's own "changed" verdict
                        // regardless of what its computed fingerprint says.
                        let recovered = file.parse_status != ParseStatus::Clean;
                        let pf = ParsedFile {
                            virtual_path: vp.clone(),
                            file,
                            provenance,
                            text,
                        };
                        let fingerprint_changed = recovered
                            || match cur.parsed.get(&vp) {
                                Some(old) => old.surface != def_surface_fingerprint(&pf),
                                None => true, // brand-new file: no prior surface to compare
                            };
                        planned.push(Planned::Save {
                            vp,
                            pf: Box::new(pf),
                            fingerprint_changed,
                        });
                    }
                },
            }
        }

        if force_rung3 {
            return Decision::Rung3;
        }

        let any_rung2 = planned.iter().any(|p| {
            matches!(
                p,
                Planned::Remove { .. }
                    | Planned::Save {
                        fingerprint_changed: true,
                        ..
                    }
            )
        });

        if any_rung2 {
            return Decision::Rung2(planned);
        }
        if planned.is_empty() {
            return Decision::Noop;
        }
        let saves = planned
            .into_iter()
            .map(|p| match p {
                Planned::Save { vp, pf, .. } => (vp, *pf),
                Planned::Remove { .. } => {
                    unreachable!("any_rung2 is false, so no Remove reached here")
                }
            })
            .collect();
        Decision::Rung1(saves)
    }

    // -----------------------------------------------------------------------
    // Rung 2: definition-surface change / file add / file delete
    // -----------------------------------------------------------------------

    /// Rung 2: flushes any accumulated `pending` overlay first (so a rung-2
    /// event that follows a run of optimized-hot-loop rung-1 saves never
    /// silently drops them), applies every save/remove to the working
    /// primary (workspace) `ParsedUnit`, rebuilds the workspace layer of the
    /// graph over the UNCHANGED cached `dep_layer`, then re-resolves EVERY
    /// workspace file (never just the touched ones — a signature change in
    /// one file can change how ANY other file's call sites resolve, which
    /// is exactly why rung 2 exists) and rebuilds every derived index
    /// wholesale.
    fn apply_rung2(&mut self, cur: &LspSnapshot, planned: Vec<Planned>) -> LspSnapshot {
        self.flush_pending(&cur.snap);
        let primary_idx = self.ensure_primary_unit_idx(&cur.snap);

        for p in planned {
            match p {
                Planned::Save { pf, .. } => splice_file(&mut self.parsed[primary_idx], *pf),
                Planned::Remove { vp } => {
                    self.parsed[primary_idx]
                        .files
                        .retain(|f| f.virtual_path != vp);
                }
            }
        }

        let new_graph =
            assemble_program_graph(&cur.dep_layer, &self.parsed[primary_idx], &cur.snap);
        let index = ResolveIndex::build(&new_graph);
        let body_map = BodyMap::build(&new_graph, &self.parsed);
        let obj_node_map: HashMap<ObjectNodeId, &ObjectNode> = new_graph
            .objects
            .iter()
            .map(|o| (o.id.clone(), o))
            .collect();
        let primary_app_ref = new_graph
            .apps
            .find(&cur.snap.workspace_app)
            .expect("assemble_program_graph must intern the workspace app");

        let mut edges_by_file: HashMap<String, Arc<Vec<ClassifiedEdge>>> = HashMap::new();
        let mut decls_by_file: HashMap<String, Arc<Vec<DeclEntry>>> = HashMap::new();
        let mut parsed_files: HashMap<String, Arc<ParsedFileEntry>> = HashMap::new();

        for pf in &self.parsed[primary_idx].files {
            let (edges, surface, decls) = recompute_file(
                pf,
                primary_app_ref,
                &new_graph,
                &index,
                &body_map,
                &obj_node_map,
            );
            edges_by_file.insert(pf.virtual_path.clone(), Arc::new(edges));
            decls_by_file.insert(pf.virtual_path.clone(), Arc::new(decls));

            let file2 = al_syntax::parse(&pf.text);
            parsed_files.insert(
                pf.virtual_path.clone(),
                Arc::new(ParsedFileEntry {
                    file: file2,
                    text: pf.text.clone(),
                    virtual_path: pf.virtual_path.clone(),
                    surface,
                }),
            );
        }

        let raw_event_edges = emit_event_flow_edges(&new_graph, &index, &body_map);
        let event_edges = Arc::new(
            raw_event_edges
                .into_iter()
                .map(|edge| ClassifiedEdge {
                    obligation_id: ObligationId::Publisher(edge.from.clone()),
                    edge,
                })
                .collect::<Vec<ClassifiedEdge>>(),
        );

        let decl_by_id = build_decl_by_id(&decls_by_file);
        let incoming = build_incoming(&edges_by_file, &event_edges);
        // Dependency source never changes at rung 2 (it reuses the cached,
        // unchanged `dep_layer` — see this method's own doc), but `new_graph`
        // is a freshly assembled `ProgramGraph` value, so `dep_decl_by_id`/
        // `dep_texts` must be recomputed against it (an `Arc::clone` forward
        // would dangle the moment `cur.graph` is dropped) — see
        // `build_dep_indexes`'s doc for why only rung 1 can skip this.
        let (dep_decl_by_id, dep_texts) =
            build_dep_indexes(&new_graph, &body_map, &self.parsed, primary_app_ref);

        LspSnapshot {
            generation: cur.generation + 1,
            graph: Arc::new(new_graph),
            dep_layer: Arc::clone(&cur.dep_layer),
            snap: Arc::clone(&cur.snap),
            parsed: parsed_files,
            edges_by_file,
            event_edges,
            incoming,
            decls_by_file,
            decl_by_id,
            dep_decl_by_id: Arc::new(dep_decl_by_id),
            dep_texts: Arc::new(dep_texts),
            // The workspace root never changes across a rung 2 rebuild — the
            // running server watches ONE root for its whole session.
            workspace_root: Arc::clone(&cur.workspace_root),
        }
    }

    // -----------------------------------------------------------------------
    // Rung 3: deps changed / overflow / non-workspace-shaped path
    // -----------------------------------------------------------------------

    /// Rung 3: full rebuild from disk, including a fresh dep layer. Only
    /// commits `self.parsed`'s replacement AFTER
    /// [`LspSnapshot::build_full_with_parsed`] succeeds — on failure,
    /// `self.parsed`/`self.pending` are left untouched and `None` is
    /// returned so `cur` (already published) survives (fail-closed). On
    /// success, `self.pending` is simply DISCARDED (not flushed): the fresh
    /// rebuild re-reads every workspace file from disk, which already
    /// reflects whatever content generated any pending rung-1 edits, so
    /// there is nothing in `pending` a disk re-read wouldn't already pick up.
    fn apply_rung3(&mut self, cur: &LspSnapshot) -> Option<(LspSnapshot, Rung)> {
        let Some((mut snapshot, parsed)) =
            LspSnapshot::build_full_with_parsed(&self.workspace_root)
        else {
            // Fail-closed (unchanged): `cur` stays published, `self.parsed`
            // stays untouched. But a silently-dropped rung-3 rebuild (e.g. a
            // deleted/malformed `app.json`, or an unreadable workspace root)
            // is otherwise INVISIBLE — nothing else observes this path. Log
            // it so an operator can tell "the server is stuck serving a
            // stale snapshot" from "there was nothing to update."
            warn!(
                "rung-3 rebuild failed for workspace {} — the previous snapshot \
                 (generation {}) remains published; check the workspace root's \
                 app.json and .alpackages",
                self.workspace_root.display(),
                cur.generation
            );
            return None;
        };
        // `build_full_with_parsed` always produces generation 0 (a fresh
        // batch build has no prior generation) — override so the counter
        // stays monotonic across every rung, including rung 3, rather than
        // going backwards.
        snapshot.generation = cur.generation + 1;
        self.parsed = parsed;
        self.pending.clear();
        Some((snapshot, Rung::Three))
    }

    // -----------------------------------------------------------------------
    // Small helpers
    // -----------------------------------------------------------------------

    /// Fold `self.pending` into `self.parsed`'s primary (workspace) unit.
    /// No-op when `pending` is empty (the common case for `apply_batch`'s
    /// simple path, which flushes after every single call).
    fn flush_pending(&mut self, snap: &AppSetSnapshot) {
        if self.pending.is_empty() {
            return;
        }
        let idx = self.ensure_primary_unit_idx(snap);
        let pending = std::mem::take(&mut self.pending);
        for (_, pf) in pending {
            splice_file(&mut self.parsed[idx], pf);
        }
    }

    fn ensure_primary_unit_idx(&mut self, snap: &AppSetSnapshot) -> usize {
        if let Some(idx) = self.parsed.iter().position(|u| u.app == snap.workspace_app) {
            return idx;
        }
        self.parsed.push(ParsedUnit {
            app: snap.workspace_app.clone(),
            files: vec![],
        });
        self.parsed.len() - 1
    }

    /// `Provenance` is uniform across every file of one app (`parse_snapshot`
    /// clones it from the owning `AppUnit`, never varies per file) — reuse
    /// an existing file's copy when one exists (the touched file's own prior
    /// entry, or any sibling), falling back to a freshly constructed
    /// workspace-tier `Provenance` only for the bootstrap case of a
    /// workspace whose primary unit has zero files so far.
    fn file_provenance(&self, cur: &LspSnapshot, vp: &str) -> Provenance {
        if let Some(idx) = self
            .parsed
            .iter()
            .position(|u| u.app == cur.snap.workspace_app)
        {
            if let Some(existing) = self.parsed[idx].files.iter().find(|f| f.virtual_path == vp) {
                return existing.provenance.clone();
            }
            if let Some(any) = self.parsed[idx].files.first() {
                return any.provenance.clone();
            }
        }
        Provenance {
            app: cur.snap.workspace_app.clone(),
            tier: TrustTier::Workspace,
            content_hash: String::new(),
        }
    }
}

/// Rung-1 CORE — see the module doc's "why a `pending` overlay" section.
/// Given an already-built context (POSSIBLY cached and reused across many
/// calls by [`spawn_updater`]'s hot loop), resolves each touched file
/// directly from its FRESH [`ParsedFile`] (passed in `saves`, never
/// re-derived through `body_map`, which may be stale for these exact files
/// — sound per the module doc's soundness argument). Records each touched
/// file into `pending` — NOT into any `ParsedUnit` directly — so a cached
/// `body_map`'s borrow of the updater's `parsed` field is never invalidated
/// by this call.
///
/// A free function (not a method): `body_map: &BodyMap<'_>` borrows
/// `Updater::parsed`, and `pending: &mut HashMap<..>` borrows the DISJOINT
/// `Updater::pending` field — a method taking `&mut self` would erase that
/// field-level distinction and make the two borrows conflict at the call
/// site; passing the two fields in separately keeps them provably disjoint
/// to the borrow checker.
fn apply_rung1_core(
    cur: &LspSnapshot,
    saves: Vec<(String, ParsedFile)>,
    index: &ResolveIndex,
    body_map: &BodyMap<'_>,
    obj_node_map: &HashMap<ObjectNodeId, &ObjectNode>,
    pending: &mut HashMap<String, ParsedFile>,
) -> LspSnapshot {
    let primary_app_ref = cur
        .graph
        .apps
        .find(&cur.snap.workspace_app)
        .expect("the workspace app must already be interned in an existing graph");

    let mut edges_by_file = cur.edges_by_file.clone();
    let mut decls_by_file = cur.decls_by_file.clone();
    let mut parsed_files = cur.parsed.clone();

    for (vp, pf) in saves {
        let (edges, surface, decls) = recompute_file(
            &pf,
            primary_app_ref,
            &cur.graph,
            index,
            body_map,
            obj_node_map,
        );
        edges_by_file.insert(vp.clone(), Arc::new(edges));
        decls_by_file.insert(vp.clone(), Arc::new(decls));

        // A SECOND, independent parse for the snapshot's own owned copy —
        // see `LspSnapshot::build_full_with_parsed`'s doc for why `AlFile`'s
        // lack of a `Clone` impl makes this the honest choice, not a
        // workaround (one file, microseconds, negligible against the 100ms
        // budget).
        let file2 = al_syntax::parse(&pf.text);
        parsed_files.insert(
            vp.clone(),
            Arc::new(ParsedFileEntry {
                file: file2,
                text: pf.text.clone(),
                virtual_path: vp.clone(),
                surface,
            }),
        );

        pending.insert(vp, pf);
    }

    let event_edges = Arc::clone(&cur.event_edges);
    let decl_by_id = build_decl_by_id(&decls_by_file);
    let incoming = build_incoming(&edges_by_file, &event_edges);

    LspSnapshot {
        generation: cur.generation + 1,
        graph: Arc::clone(&cur.graph),
        dep_layer: Arc::clone(&cur.dep_layer),
        snap: Arc::clone(&cur.snap),
        parsed: parsed_files,
        edges_by_file,
        event_edges,
        incoming,
        decls_by_file,
        decl_by_id,
        // Rung 1 touches ONLY workspace files — dependency source is
        // untouched and `cur.graph` is reused unchanged (see this function's
        // doc), so `dep_decl_by_id`/`dep_texts` are byte-identical to the
        // previous snapshot's; `Arc::clone` rather than recompute (see
        // `build_dep_indexes`'s doc).
        dep_decl_by_id: Arc::clone(&cur.dep_decl_by_id),
        dep_texts: Arc::clone(&cur.dep_texts),
        workspace_root: Arc::clone(&cur.workspace_root),
    }
}

/// Replace-or-append `pf` in `unit.files` by `virtual_path` match.
fn splice_file(unit: &mut ParsedUnit, pf: ParsedFile) {
    if let Some(slot) = unit
        .files
        .iter_mut()
        .find(|f| f.virtual_path == pf.virtual_path)
    {
        *slot = pf;
    } else {
        unit.files.push(pf);
    }
}

// ---------------------------------------------------------------------------
// Batch classification
// ---------------------------------------------------------------------------

enum Planned {
    Save {
        vp: String,
        // `Box`ed: `ParsedFile` (via `AlFile`) is much larger than `Remove`'s
        // single `String` — clippy's `large_enum_variant` flags the
        // otherwise-oversized `Planned` (every `Remove` would pay for
        // `Save`'s full size).
        pf: Box<ParsedFile>,
        fingerprint_changed: bool,
    },
    Remove {
        vp: String,
    },
}

enum PathClass {
    /// A `.al` file inside `workspace_root`, not under a dependency/output
    /// directory — `String` is its `virtual_path` (workspace-root-relative,
    /// `/`-separated, mirroring `crate::snapshot::provider`'s own
    /// construction).
    Workspace(String),
    /// Outside `workspace_root` entirely, under a skipped dependency/output
    /// directory (`.alpackages`/`.snapshots`/`node_modules` — the same list
    /// `crate::snapshot::provider::walk_al_source` excludes from the
    /// workspace's own source walk), or not a `.al` file at all.
    NotWorkspaceSource,
}

/// Classify an absolute path against `workspace_root`, purely lexically (no
/// filesystem access — see [`ChangeEvent`]'s doc for why: a `FileRemoved`
/// path may no longer exist).
fn classify_path(workspace_root: &Path, path: &Path) -> PathClass {
    let Ok(rel) = path.strip_prefix(workspace_root) else {
        return PathClass::NotWorkspaceSource;
    };
    let is_al = path.extension().and_then(|e| e.to_str()) == Some("al");
    let under_skip_dir = rel.components().any(|c| {
        matches!(
            c.as_os_str().to_str(),
            Some(".alpackages") | Some(".snapshots") | Some("node_modules")
        )
    });
    if !is_al || under_skip_dir {
        return PathClass::NotWorkspaceSource;
    }
    PathClass::Workspace(rel.to_string_lossy().replace('\\', "/"))
}

// ---------------------------------------------------------------------------
// Thread wrapper: debounce + per-path coalesce + apply + swap + notify
// ---------------------------------------------------------------------------

const DEBOUNCE_WINDOW: Duration = Duration::from_millis(100);

/// Per-path coalesce within one gathered batch: keep only the LAST event for
/// a given path (a save immediately followed by a remove for the SAME path
/// keeps the remove — "last wins," matching real editor semantics), while
/// preserving first-seen ORDER for everything else. `DepsChanged`/`Overflow`
/// have no path — every occurrence is kept (idempotent to see more than
/// once: both force rung 3 regardless of count).
fn coalesce_batch(events: Vec<ChangeEvent>) -> Vec<ChangeEvent> {
    let mut index_of: HashMap<PathBuf, usize> = HashMap::new();
    let mut out: Vec<ChangeEvent> = Vec::new();
    for ev in events {
        match &ev {
            ChangeEvent::FileSaved(p) | ChangeEvent::FileRemoved(p) => {
                if let Some(&idx) = index_of.get(p) {
                    out[idx] = ev;
                } else {
                    index_of.insert(p.clone(), out.len());
                    out.push(ev);
                }
            }
            ChangeEvent::DepsChanged | ChangeEvent::Overflow => out.push(ev),
        }
    }
    out
}

/// Block for one event, then drain everything else arriving within
/// [`DEBOUNCE_WINDOW`] of the first, returning the coalesced batch. Returns
/// `None` when the channel is closed (sender dropped).
fn gather_batch(rx: &Receiver<ChangeEvent>) -> Option<Vec<ChangeEvent>> {
    let first = rx.recv().ok()?;
    let mut batch = vec![first];
    let deadline = Instant::now() + DEBOUNCE_WINDOW;
    loop {
        let now = Instant::now();
        if now >= deadline {
            break;
        }
        match rx.recv_timeout(deadline - now) {
            Ok(ev) => batch.push(ev),
            Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => break,
        }
    }
    Some(coalesce_batch(batch))
}

/// Spawn the updater thread implementing the module doc's "scoped-context
/// loop": right after every swap (or at startup), a `ResolveIndex`/
/// `BodyMap`/`obj_node_map` context is built ONCE from the just-published
/// snapshot's graph + the updater's current `parsed`, then REUSED for every
/// consecutive rung-1 batch (via [`apply_rung1_core`], which never mutates
/// `parsed` — see the module doc) until a rung-2/3 event arrives, at which
/// point the context is dropped (its borrows end at the `{ ... }` block
/// boundary below — no `unsafe`, no self-referential struct: the borrow
/// simply never needs to outlive the block it's confined to) and rebuilt
/// fresh after the rung-2/3 rebuild swaps in a new graph.
///
/// Exits cleanly when the sending side of `rx` is dropped.
pub fn spawn_updater(
    shared: Arc<SharedSnapshot>,
    rx: Receiver<ChangeEvent>,
    workspace_root: PathBuf,
    initial_parsed: Vec<ParsedUnit>,
    on_swap: impl Fn(&LspSnapshot, &LspSnapshot) + Send + 'static,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let mut updater = Updater::new(workspace_root, initial_parsed);
        let mut cur = shared.get();

        loop {
            // `index`/`body_map`/`obj_node_map` all borrow from `cur`/
            // `updater.parsed` as they stand RIGHT NOW; they are built once
            // per iteration of this OUTER loop and reused for every
            // consecutive rung-1 batch the INNER loop processes. `inner_cur`
            // is a SEPARATE cloned `Arc` (not a move of `cur`) so `cur`
            // itself stays unborrowed-from-a-moved-value — `obj_node_map`'s
            // references stay valid through every rung-1 swap because rung
            // 1 always reuses the SAME underlying `Arc<ProgramGraph>`
            // (`apply_rung1_core` never rebuilds `graph`).
            let (new_cur, decision) = {
                let index = ResolveIndex::build(&cur.graph);
                let body_map = BodyMap::build(&cur.graph, &updater.parsed);
                let obj_node_map: HashMap<ObjectNodeId, &ObjectNode> = cur
                    .graph
                    .objects
                    .iter()
                    .map(|o| (o.id.clone(), o))
                    .collect();

                let mut inner_cur = Arc::clone(&cur);
                let escalated = loop {
                    let Some(batch) = gather_batch(&rx) else {
                        return; // sender dropped — shut down cleanly
                    };
                    match updater.classify(&inner_cur, &batch) {
                        Decision::Noop => {}
                        Decision::Rung1(saves) => {
                            let new_snapshot = apply_rung1_core(
                                &inner_cur,
                                saves,
                                &index,
                                &body_map,
                                &obj_node_map,
                                &mut updater.pending,
                            );
                            let new_arc = Arc::new(new_snapshot);
                            shared.swap(Arc::clone(&new_arc));
                            on_swap(&inner_cur, &new_arc);
                            inner_cur = new_arc;
                        }
                        decision @ (Decision::Rung2(_) | Decision::Rung3) => break decision,
                    }
                };
                (inner_cur, escalated)
            }; // `index`/`body_map`/`obj_node_map` dropped here.
            cur = new_cur;

            match decision {
                Decision::Rung2(planned) => {
                    let new_snapshot = updater.apply_rung2(&cur, planned);
                    let new_arc = Arc::new(new_snapshot);
                    shared.swap(Arc::clone(&new_arc));
                    on_swap(&cur, &new_arc);
                    cur = new_arc;
                }
                Decision::Rung3 => {
                    if let Some((new_snapshot, _)) = updater.apply_rung3(&cur) {
                        let new_arc = Arc::new(new_snapshot);
                        shared.swap(Arc::clone(&new_arc));
                        on_swap(&cur, &new_arc);
                        cur = new_arc;
                    }
                }
                Decision::Noop | Decision::Rung1(_) => {
                    unreachable!("the inner loop only ever breaks with Rung2/Rung3")
                }
            }
        }
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::program::resolve::edge::{Evidence, RouteTarget, UnknownReason};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;

    fn write_fixture_workspace(dir: &Path) {
        std::fs::write(
            dir.join("app.json"),
            r#"{
    "id": "44444444-0000-0000-0000-000000000009",
    "name": "Task9 Updater Fixture",
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
}
"#,
        )
        .expect("write Beta.al");

        std::fs::write(
            dir.join("Gamma.al"),
            r#"codeunit 50102 "Gamma"
{
    var
        Beta: Codeunit "Beta";
    procedure Standalone()
    begin
        Beta.Process();
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

    fn build(dir: &Path) -> (LspSnapshot, Vec<ParsedUnit>) {
        LspSnapshot::build_full_with_parsed(dir).expect("build_full_with_parsed")
    }

    // ── (a) body edit, existing target → rung 1, Arc-identical sibling ────

    #[test]
    fn body_edit_calling_existing_target_takes_rung1_and_shares_untouched_files() {
        let dir = fixture_dir();
        let (base, parsed) = build(dir.path());
        let mut updater = Updater::new(dir.path().to_path_buf(), parsed);

        // Body-only edit: Alpha.DoWork now calls Beta.Process() a SECOND
        // time — a new call SITE to an ALREADY-EXISTING target, no object/
        // routine identity change, no signature change: the definition
        // surface is provably unaffected.
        std::fs::write(
            dir.path().join("Alpha.al"),
            r#"codeunit 50100 "Alpha"
{
    procedure DoWork()
    var
        Beta: Codeunit "Beta";
    begin
        Beta.Process();
        Beta.Process();
    end;
}
"#,
        )
        .expect("rewrite Alpha.al");

        let batch = vec![ChangeEvent::FileSaved(dir.path().join("Alpha.al"))];
        let (new_snap, rung) = updater
            .apply_batch(&base, &batch)
            .expect("apply_batch must succeed");

        assert_eq!(
            rung,
            Rung::One,
            "a body-only edit to an existing target must take rung 1"
        );

        // Alpha's bucket changed (2 edges now, was 1).
        let old_alpha = &base.edges_by_file["Alpha.al"];
        let new_alpha = &new_snap.edges_by_file["Alpha.al"];
        assert_eq!(old_alpha.len(), 1);
        assert_eq!(new_alpha.len(), 2, "Alpha must now have 2 call sites");

        // Beta's bucket is ARC-IDENTICAL — proof that rung 1 never re-resolved it.
        assert!(
            Arc::ptr_eq(
                &base.edges_by_file["Beta.al"],
                &new_snap.edges_by_file["Beta.al"]
            ),
            "Beta's edge bucket must be the SAME Arc — rung 1 must not re-resolve untouched files"
        );

        // incoming reflects the new edge: Beta.Process now has 2 incoming
        // callers, both from Alpha.al.
        let beta_process = new_snap.decls_by_file["Beta.al"]
            .iter()
            .find(|d| d.name == "Process")
            .expect("Beta.Process decl")
            .id
            .clone();
        let incoming = new_snap
            .incoming
            .get(&beta_process)
            .expect("Beta.Process must have incoming callers");
        let from_alpha = incoming.iter().filter(|r| r.file == "Alpha.al").count();
        assert_eq!(
            from_alpha, 2,
            "both of Alpha's call sites must be indexed as incoming"
        );

        assert_eq!(new_snap.generation, base.generation + 1);
    }

    // ── (b) signature edit → rung 2, caller re-resolves (arity mismatch) ──

    #[test]
    fn signature_edit_takes_rung2_and_flips_caller_to_unknown() {
        let dir = fixture_dir();
        // Reduce to a minimal 2-file fixture for this scenario: Alpha
        // declares Greet(), Beta calls it with 0 args.
        std::fs::write(
            dir.path().join("Alpha.al"),
            r#"codeunit 50100 "Alpha"
{
    procedure Greet()
    begin
    end;
}
"#,
        )
        .expect("rewrite Alpha.al");
        std::fs::write(
            dir.path().join("Beta.al"),
            r#"codeunit 50101 "Beta"
{
    var
        Alpha: Codeunit "Alpha";
    procedure CallGreet()
    begin
        Alpha.Greet();
    end;
}
"#,
        )
        .expect("rewrite Beta.al");
        std::fs::remove_file(dir.path().join("Gamma.al")).expect("remove Gamma.al");

        let (base, parsed) = build(dir.path());
        let mut updater = Updater::new(dir.path().to_path_buf(), parsed);

        // Baseline sanity: Beta's call site must have resolved to Alpha.Greet.
        let beta_edges_before = &base.edges_by_file["Beta.al"];
        assert_eq!(beta_edges_before.len(), 1);
        assert!(
            beta_edges_before[0]
                .edge
                .routes
                .iter()
                .any(|r| matches!(r.target, RouteTarget::Routine(_))
                    && r.evidence == Evidence::Source),
            "baseline: Beta.CallGreet must resolve to Alpha.Greet before the edit"
        );

        // Signature edit: add a parameter to Alpha.Greet — a DefSurface
        // change (item 3/4/12 of the fingerprint: the routine identity SET
        // and its param_sig_key/sig_fp all move).
        std::fs::write(
            dir.path().join("Alpha.al"),
            r#"codeunit 50100 "Alpha"
{
    procedure Greet(X: Integer)
    begin
    end;
}
"#,
        )
        .expect("rewrite Alpha.al with new signature");

        let batch = vec![ChangeEvent::FileSaved(dir.path().join("Alpha.al"))];
        let (new_snap, rung) = updater
            .apply_batch(&base, &batch)
            .expect("apply_batch must succeed");

        assert_eq!(rung, Rung::Two, "a signature change must take rung 2");

        // Beta (a DIFFERENT file, never itself saved) must have been
        // re-resolved: its call site is now an arity mismatch (0 args vs.
        // 1 declared param) — no 0-arg overload exists, so it must be an
        // honest Unknown, never silently left as the STALE resolved route.
        let beta_edges_after = &new_snap.edges_by_file["Beta.al"];
        assert_eq!(beta_edges_after.len(), 1);
        let route = &beta_edges_after[0].edge.routes[0];
        assert!(
            matches!(route.evidence, Evidence::Unknown(_)),
            "Beta.CallGreet must resolve to Unknown after Alpha.Greet's arity changed \
             out from under it; got {:?}",
            route.evidence
        );
        assert_eq!(
            route.evidence,
            Evidence::Unknown(UnknownReason::ArityMismatch),
            "the specific reason should be ArityMismatch"
        );

        assert_eq!(new_snap.generation, base.generation + 1);
    }

    // ── (c) file delete → rung 2, edges gone from buckets + incoming ─────

    #[test]
    fn file_delete_takes_rung2_and_removes_its_edges_and_incoming_entries() {
        let dir = fixture_dir();
        let (base, parsed) = build(dir.path());
        let mut updater = Updater::new(dir.path().to_path_buf(), parsed);

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
            incoming_before.iter().any(|r| r.file == "Gamma.al"),
            "baseline: Gamma.al must be one of Beta.Process's incoming callers"
        );
        assert!(incoming_before.iter().any(|r| r.file == "Alpha.al"));

        std::fs::remove_file(dir.path().join("Gamma.al")).expect("delete Gamma.al");
        let batch = vec![ChangeEvent::FileRemoved(dir.path().join("Gamma.al"))];
        let (new_snap, rung) = updater
            .apply_batch(&base, &batch)
            .expect("apply_batch must succeed");

        assert_eq!(rung, Rung::Two, "a file delete must take rung 2");

        assert!(
            !new_snap.edges_by_file.contains_key("Gamma.al"),
            "Gamma.al's edge bucket must be gone"
        );
        assert!(!new_snap.decls_by_file.contains_key("Gamma.al"));
        assert!(!new_snap.parsed.contains_key("Gamma.al"));

        let incoming_after = new_snap
            .incoming
            .get(&beta_process)
            .expect("Beta.Process must still have Alpha.al as an incoming caller");
        assert!(
            !incoming_after.iter().any(|r| r.file == "Gamma.al"),
            "Gamma.al's incoming entry must be gone"
        );
        assert!(
            incoming_after.iter().any(|r| r.file == "Alpha.al"),
            "Alpha.al's own incoming entry must survive"
        );

        assert_eq!(new_snap.generation, base.generation + 1);
    }

    // ── (d) parse-error (Recovered) save → escalates past rung 1 ─────────

    #[test]
    fn recovered_parse_save_escalates_to_rung2() {
        let dir = fixture_dir();
        let (base, parsed) = build(dir.path());
        let mut updater = Updater::new(dir.path().to_path_buf(), parsed);

        // An unbalanced `#if` forces tree-sitter error recovery
        // (`ParseStatus::Recovered`) — content-wise this ADDS a call site
        // (Foo() -> Beta.Process(), already-existing target) which, taken
        // at face value, would otherwise look rung-1-eligible; the
        // Recovered status must override that and force rung 2 regardless.
        std::fs::write(
            dir.path().join("Alpha.al"),
            "codeunit 50100 \"Alpha\"\n{\n    procedure DoWork()\n    var\n        \
             Beta: Codeunit \"Beta\";\n    begin\n#if NEVER_CLOSED\n        \
             Beta.Process();\n    end;\n}\n",
        )
        .expect("rewrite Alpha.al with an unbalanced #if");

        let batch = vec![ChangeEvent::FileSaved(dir.path().join("Alpha.al"))];
        let (new_snap, rung) = updater
            .apply_batch(&base, &batch)
            .expect("apply_batch must succeed even for a Recovered parse");

        assert_eq!(
            rung,
            Rung::Two,
            "a Recovered parse must escalate past rung 1, never take it on faith"
        );
        assert_eq!(new_snap.generation, base.generation + 1);
    }

    /// The parenthetical half of scenario (d): "prev snapshot survives if
    /// build fails entirely." Simulated via a rung-3 event against a
    /// workspace whose `app.json` has since been deleted — `apply_batch`
    /// must return `None` and must NOT mutate `self.parsed`.
    #[test]
    fn build_failure_leaves_prev_snapshot_and_working_state_untouched() {
        let dir = fixture_dir();
        let (base, parsed) = build(dir.path());
        let parsed_files_before: Vec<String> = parsed
            .iter()
            .flat_map(|u| u.files.iter().map(|f| f.virtual_path.clone()))
            .collect();
        let mut updater = Updater::new(dir.path().to_path_buf(), parsed);

        std::fs::remove_file(dir.path().join("app.json")).expect("remove app.json");

        let batch = vec![ChangeEvent::DepsChanged];
        let result = updater.apply_batch(&base, &batch);
        assert!(
            result.is_none(),
            "a rung-3 rebuild against a broken workspace must fail closed (None)"
        );

        let parsed_files_after: Vec<String> = updater
            .parsed
            .iter()
            .flat_map(|u| u.files.iter().map(|f| f.virtual_path.clone()))
            .collect();
        assert_eq!(
            parsed_files_before, parsed_files_after,
            "a failed rung-3 build must not mutate the updater's working parse state"
        );
    }

    // ── (e) FileSaved outside the workspace source set → escalates to rung 3 ──

    #[test]
    fn file_saved_under_dependency_dir_escalates_to_rung3() {
        let dir = fixture_dir();
        let (base, parsed) = build(dir.path());
        let mut updater = Updater::new(dir.path().to_path_buf(), parsed);

        // A path under `.alpackages/` — never part of `ws_file_set`/
        // `prev.parsed` — reaching a `didSave`-shaped event (Task-4 review
        // Hunt-3 scenario). It doesn't even need to exist on disk: path
        // classification is purely lexical.
        let dep_path = dir
            .path()
            .join(".alpackages")
            .join("SomeDep")
            .join("Foo.al");

        let batch = vec![ChangeEvent::FileSaved(dep_path)];
        let (new_snap, rung) = updater
            .apply_batch(&base, &batch)
            .expect("apply_batch must succeed (rebuilds from disk unchanged)");

        assert_eq!(
            rung,
            Rung::Three,
            "a path outside the workspace source set must escalate past rung 1 AND \
             past rung 2 (we have no rung-2 primitive for a dependency-scoped change)"
        );
        assert_eq!(new_snap.generation, base.generation + 1);
    }

    // ── batch semantics: any rung-2 event forces the WHOLE batch to rung 2 ──

    #[test]
    fn mixed_batch_with_one_rung2_file_takes_rung2_for_the_whole_batch() {
        let dir = fixture_dir();
        let (base, parsed) = build(dir.path());
        let mut updater = Updater::new(dir.path().to_path_buf(), parsed);

        // Alpha: body-only edit (would be rung-1-eligible alone).
        std::fs::write(
            dir.path().join("Alpha.al"),
            r#"codeunit 50100 "Alpha"
{
    procedure DoWork()
    var
        Beta: Codeunit "Beta";
    begin
        Beta.Process();
        Beta.Process();
    end;
}
"#,
        )
        .expect("rewrite Alpha.al");
        // Gamma: a NEW routine added (a definition-surface change, forces
        // rung 2 on its own).
        std::fs::write(
            dir.path().join("Gamma.al"),
            r#"codeunit 50102 "Gamma"
{
    var
        Beta: Codeunit "Beta";
    procedure Standalone()
    begin
        Beta.Process();
    end;

    procedure Extra()
    begin
    end;
}
"#,
        )
        .expect("rewrite Gamma.al");

        let batch = vec![
            ChangeEvent::FileSaved(dir.path().join("Alpha.al")),
            ChangeEvent::FileSaved(dir.path().join("Gamma.al")),
        ];
        let (new_snap, rung) = updater
            .apply_batch(&base, &batch)
            .expect("apply_batch must succeed");

        assert_eq!(
            rung,
            Rung::Two,
            "one rung-2-eligible file in the batch must force the WHOLE batch to rung 2"
        );
        // Both files' edits must be reflected (a single rebuild served both).
        assert_eq!(new_snap.edges_by_file["Alpha.al"].len(), 2);
        assert!(
            new_snap.decls_by_file["Gamma.al"]
                .iter()
                .any(|d| d.name == "Extra"),
            "Gamma's new routine must be present after the shared rung-2 rebuild"
        );
    }

    // ── the cached-context property: 2 consecutive rung-1 edits reusing ────
    // ── the SAME index/body_map, never rebuilt between them ───────────────

    #[test]
    fn apply_rung1_core_reuses_the_same_context_across_two_consecutive_edits() {
        let dir = fixture_dir();
        let (base, parsed) = build(dir.path());
        let mut updater = Updater::new(dir.path().to_path_buf(), parsed);

        // Built ONCE — never rebuilt for either of the two edits below,
        // proving the exact caching property `spawn_updater`'s hot loop
        // relies on for the rung-1 budget.
        let index = ResolveIndex::build(&base.graph);
        let body_map = BodyMap::build(&base.graph, &updater.parsed);
        let obj_node_map: HashMap<ObjectNodeId, &ObjectNode> = base
            .graph
            .objects
            .iter()
            .map(|o| (o.id.clone(), o))
            .collect();

        // Edit 1: Alpha gets a second call to Beta.Process().
        std::fs::write(
            dir.path().join("Alpha.al"),
            r#"codeunit 50100 "Alpha"
{
    procedure DoWork()
    var
        Beta: Codeunit "Beta";
    begin
        Beta.Process();
        Beta.Process();
    end;
}
"#,
        )
        .expect("rewrite Alpha.al (edit 1)");
        let text1 = std::fs::read_to_string(dir.path().join("Alpha.al")).unwrap();
        let pf1 = ParsedFile {
            virtual_path: "Alpha.al".to_string(),
            file: al_syntax::parse(&text1),
            provenance: updater.file_provenance(&base, "Alpha.al"),
            text: text1,
        };
        let snap1 = apply_rung1_core(
            &base,
            vec![("Alpha.al".to_string(), pf1)],
            &index,
            &body_map,
            &obj_node_map,
            &mut updater.pending,
        );
        assert_eq!(snap1.edges_by_file["Alpha.al"].len(), 2);
        assert!(Arc::ptr_eq(
            &base.edges_by_file["Beta.al"],
            &snap1.edges_by_file["Beta.al"]
        ));

        // Edit 2: Alpha gets a THIRD call — reusing the SAME `index`/
        // `body_map` built above (never rebuilt between edit 1 and edit 2).
        std::fs::write(
            dir.path().join("Alpha.al"),
            r#"codeunit 50100 "Alpha"
{
    procedure DoWork()
    var
        Beta: Codeunit "Beta";
    begin
        Beta.Process();
        Beta.Process();
        Beta.Process();
    end;
}
"#,
        )
        .expect("rewrite Alpha.al (edit 2)");
        let text2 = std::fs::read_to_string(dir.path().join("Alpha.al")).unwrap();
        let pf2 = ParsedFile {
            virtual_path: "Alpha.al".to_string(),
            file: al_syntax::parse(&text2),
            provenance: updater.file_provenance(&base, "Alpha.al"),
            text: text2,
        };
        let snap2 = apply_rung1_core(
            &snap1,
            vec![("Alpha.al".to_string(), pf2)],
            &index,
            &body_map,
            &obj_node_map,
            &mut updater.pending,
        );
        assert_eq!(
            snap2.edges_by_file["Alpha.al"].len(),
            3,
            "the SECOND consecutive rung-1 edit, using the cached context from BEFORE \
             either edit, must still resolve correctly"
        );
        assert!(Arc::ptr_eq(
            &snap1.edges_by_file["Beta.al"],
            &snap2.edges_by_file["Beta.al"]
        ));
        assert_eq!(updater.pending.len(), 1, "both edits target the same file");
    }

    // ── Step 3: debounce/coalesce — 5 rapid saves of one file → 1 apply ───

    #[test]
    fn spawn_updater_coalesces_five_rapid_saves_into_one_apply() {
        let dir = fixture_dir();
        let (snapshot, parsed) = build(dir.path());
        let shared = Arc::new(SharedSnapshot::new(Arc::new(snapshot)));
        let (tx, rx) = mpsc::channel();
        let counter = Arc::new(AtomicUsize::new(0));
        let counter2 = Arc::clone(&counter);

        let handle = spawn_updater(
            Arc::clone(&shared),
            rx,
            dir.path().to_path_buf(),
            parsed,
            move |_old, _new| {
                counter2.fetch_add(1, Ordering::SeqCst);
            },
        );

        let alpha_path = dir.path().join("Alpha.al");
        for _ in 0..5 {
            tx.send(ChangeEvent::FileSaved(alpha_path.clone()))
                .expect("send must succeed");
        }

        // Give the updater thread time to gather the debounce window
        // (100ms) and apply — comfortably over the window without being a
        // flaky hair-trigger.
        std::thread::sleep(Duration::from_millis(400));
        drop(tx);
        handle.join().expect("updater thread must exit cleanly");

        assert_eq!(
            counter.load(Ordering::SeqCst),
            1,
            "5 rapid saves of ONE file must coalesce into exactly 1 apply"
        );
    }

    // ── e2e: rung 1 → rung 2 → rung 1 through the REAL background thread ──
    // (review fix-wave item 3): proves `spawn_updater`'s scoped-context loop
    // actually rebuilds `index`/`body_map`/`obj_node_map` after a rung-2
    // escalation, rather than the next rung-1 batch silently resolving
    // against a stale pre-rung-2 context — the exact guarantee the
    // `{ ... }` block-scoping in `spawn_updater` exists to provide.

    #[test]
    fn spawn_updater_rebuilds_context_after_rung2_escalation() {
        use std::sync::Mutex;

        let dir = fixture_dir();
        let (snapshot, parsed) = build(dir.path());
        let base_generation = snapshot.generation;
        let shared = Arc::new(SharedSnapshot::new(Arc::new(snapshot)));
        let (tx, rx) = mpsc::channel();

        // Classify each swap's rung from Arc identity alone (no test-only
        // hook needed): rung 1 keeps `graph` Arc-identical; rung 2 rebuilds
        // `graph` but keeps `dep_layer` Arc-identical; rung 3 rebuilds both.
        let events: Arc<Mutex<Vec<(u64, Rung)>>> = Arc::new(Mutex::new(Vec::new()));
        let events2 = Arc::clone(&events);

        let handle = spawn_updater(
            Arc::clone(&shared),
            rx,
            dir.path().to_path_buf(),
            parsed,
            move |old, new| {
                let rung = if !Arc::ptr_eq(&old.dep_layer, &new.dep_layer) {
                    Rung::Three
                } else if !Arc::ptr_eq(&old.graph, &new.graph) {
                    Rung::Two
                } else {
                    Rung::One
                };
                events2.lock().unwrap().push((new.generation, rung));
            },
        );

        // Step 1 (rung 1): Alpha gets a 2nd call to the already-existing
        // Beta.Process().
        std::fs::write(
            dir.path().join("Alpha.al"),
            r#"codeunit 50100 "Alpha"
{
    procedure DoWork()
    var
        Beta: Codeunit "Beta";
    begin
        Beta.Process();
        Beta.Process();
    end;
}
"#,
        )
        .expect("edit 1 (rung 1)");
        tx.send(ChangeEvent::FileSaved(dir.path().join("Alpha.al")))
            .expect("send 1");
        std::thread::sleep(Duration::from_millis(300));

        // Step 2 (rung 2): Gamma gains a brand-new routine — a
        // definition-surface change (the routine SET moves).
        std::fs::write(
            dir.path().join("Gamma.al"),
            r#"codeunit 50102 "Gamma"
{
    var
        Beta: Codeunit "Beta";
    procedure Standalone()
    begin
        Beta.Process();
    end;

    procedure Extra()
    begin
    end;
}
"#,
        )
        .expect("edit 2 (rung 2)");
        tx.send(ChangeEvent::FileSaved(dir.path().join("Gamma.al")))
            .expect("send 2");
        std::thread::sleep(Duration::from_millis(300));

        // Step 3 (rung 1 again, AFTER the rung-2 escalation): Alpha gets a
        // 3rd call to Beta.Process() — still fingerprint-equal.
        std::fs::write(
            dir.path().join("Alpha.al"),
            r#"codeunit 50100 "Alpha"
{
    procedure DoWork()
    var
        Beta: Codeunit "Beta";
    begin
        Beta.Process();
        Beta.Process();
        Beta.Process();
    end;
}
"#,
        )
        .expect("edit 3 (rung 1, post-rung-2)");
        tx.send(ChangeEvent::FileSaved(dir.path().join("Alpha.al")))
            .expect("send 3");
        std::thread::sleep(Duration::from_millis(300));

        drop(tx);
        handle.join().expect("updater thread must exit cleanly");

        let events = events.lock().unwrap();
        assert_eq!(
            events.len(),
            3,
            "exactly 3 swaps expected (one per debounced, isolated step); got {events:?}"
        );
        assert_eq!(
            events[0],
            (base_generation + 1, Rung::One),
            "step 1 must be rung 1"
        );
        assert_eq!(
            events[1],
            (base_generation + 2, Rung::Two),
            "step 2 must be rung 2"
        );
        assert_eq!(
            events[2],
            (base_generation + 3, Rung::One),
            "step 3 must be rung 1 again"
        );

        // The FINAL snapshot must reflect BOTH rung 2's change (Gamma.Extra)
        // AND step 3's own rung-1 edit (Alpha now has 3 call sites) — proof
        // that step 3 resolved against the POST-rung-2 graph, not a context
        // cached from before the escalation.
        let final_snap = shared.get();
        assert!(
            final_snap.decls_by_file["Gamma.al"]
                .iter()
                .any(|d| d.name == "Extra"),
            "rung 2's new routine must survive into the final snapshot"
        );
        assert_eq!(
            final_snap.edges_by_file["Alpha.al"].len(),
            3,
            "the post-rung-2 rung-1 edit must resolve correctly against the \
             POST-rung-2 graph — proves the thread rebuilt its cached context \
             after the escalation"
        );
    }

    // -----------------------------------------------------------------------
    // T3 Task 9 Step 3b: RE-MEASURE rung 1/rung 2 against the REAL updater
    // code path (Task 3's original 1.9s rung-2 pin was an UPPER BOUND: it
    // predated `assemble_program_graph`/this task's real rung-2 path
    // entirely — see `.superpowers/sdd/t3-stage-split.md`).
    //
    // This exercises `apply_rung1_core`/`Updater::apply_rung2` DIRECTLY,
    // bypassing `Updater::apply_batch`'s classification (which reads from
    // and would otherwise need to write to real files on the user's ACTUAL
    // CDO workspace on disk — never done here: every `ParsedFile` this test
    // constructs is built from a real workspace file's OWN already-parsed
    // TEXT, re-parsed in memory, with zero `std::fs::write` calls anywhere).
    // This measures the real code path faithfully: `apply_rung2`'s cost is
    // dominated by re-resolving EVERY workspace file regardless of which one
    // "changed," so feeding it the SAME (unchanged) content for one file
    // exercises the identical splice + assemble_program_graph + fresh
    // index/body_map + re-resolve-ALL + event-edges + derived-index cost a
    // real signature edit would pay.
    // -----------------------------------------------------------------------

    /// Run: `CDO_WS=<path> cargo test --release rung1_rung2_wall_clock -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn rung1_rung2_wall_clock_on_cdo() {
        let Some(ws) = std::env::var_os("CDO_WS")
            .map(std::path::PathBuf::from)
            .filter(|p| p.exists())
        else {
            eprintln!("rung1_rung2_wall_clock_on_cdo: CDO_WS unset or missing, skipping");
            return;
        };

        const RUNS: usize = 3;
        fn median(mut xs: Vec<Duration>) -> Duration {
            xs.sort();
            xs[xs.len() / 2]
        }

        let (base, parsed) =
            LspSnapshot::build_full_with_parsed(&ws).expect("build_full_with_parsed on CDO");
        let mut updater = Updater::new(ws.clone(), parsed);

        // Any real workspace file — sorted for a deterministic pick.
        let mut vps: Vec<String> = base.parsed.keys().cloned().collect();
        vps.sort();
        let target_vp = vps
            .into_iter()
            .next()
            .expect("CDO must have at least one workspace file");
        let target_text = base.parsed[&target_vp].text.clone();

        // ── Rung 1: warm context (built ONCE, reused for all RUNS) —
        // resolve-one-file + incoming rebuild. `updater.parsed` is NEVER
        // mutated here (the touched file goes into a throwaway local
        // `pending` map, exactly as `apply_rung1_core`'s real contract
        // promises), so this block cannot perturb the rung-2 measurement
        // that follows it.
        let mut rung1_times = Vec::with_capacity(RUNS);
        {
            let index = ResolveIndex::build(&base.graph);
            let body_map = BodyMap::build(&base.graph, &updater.parsed);
            let obj_node_map: HashMap<ObjectNodeId, &ObjectNode> = base
                .graph
                .objects
                .iter()
                .map(|o| (o.id.clone(), o))
                .collect();
            let mut pending: HashMap<String, ParsedFile> = HashMap::new();

            for _ in 0..RUNS {
                let t0 = Instant::now();
                let provenance = updater.file_provenance(&base, &target_vp);
                let file = al_syntax::parse(&target_text);
                let pf = ParsedFile {
                    virtual_path: target_vp.clone(),
                    file,
                    provenance,
                    text: target_text.clone(),
                };
                let _snapshot = apply_rung1_core(
                    &base,
                    vec![(target_vp.clone(), pf)],
                    &index,
                    &body_map,
                    &obj_node_map,
                    &mut pending,
                );
                rung1_times.push(t0.elapsed());
            }
        }

        // ── Rung 2: splice + assemble_program_graph + fresh index/body_map
        // + re-resolve ALL workspace files + event edges + derived indexes.
        // Reuses the SAME `updater` across all RUNS (each run re-splices the
        // identical, unchanged content — idempotent, so repeating this 3x
        // measures the same real cost 3 times without needing a fresh
        // multi-second `build_full_with_parsed` per run).
        let mut rung2_times = Vec::with_capacity(RUNS);
        for _ in 0..RUNS {
            let provenance = updater.file_provenance(&base, &target_vp);
            let pf = ParsedFile {
                virtual_path: target_vp.clone(),
                file: al_syntax::parse(&target_text),
                provenance,
                text: target_text.clone(),
            };
            let planned = vec![Planned::Save {
                vp: target_vp.clone(),
                pf: Box::new(pf),
                fingerprint_changed: true,
            }];

            let t0 = Instant::now();
            let _snapshot = updater.apply_rung2(&base, planned);
            rung2_times.push(t0.elapsed());
        }

        let rung1_med = median(rung1_times);
        let rung2_med = median(rung2_times);

        eprintln!("=== rung1_rung2_wall_clock_on_cdo (median of {RUNS} runs, CDO_WS={ws:?}) ===");
        eprintln!(
            "rung 1 (warm context: resolve-one-file + incoming rebuild, swap excluded — \
             an Arc write, negligible) : {rung1_med:?}"
        );
        eprintln!(
            "rung 2 (splice + assemble_program_graph + fresh index/bodymap + re-resolve-ALL \
             + event edges + derived indexes) : {rung2_med:?}"
        );
        if rung1_med > Duration::from_millis(100) {
            eprintln!("*** rung-1 EXCEEDED the 100ms budget: {rung1_med:?} ***");
        } else {
            eprintln!("rung-1 <100ms HOLDS: {rung1_med:?}");
        }
    }
}
