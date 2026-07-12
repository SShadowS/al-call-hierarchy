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
//! contingency section explicitly sanctions this restructuring. The reason
//! is load-bearing, not stylistic: `docs/superpowers/specs/2026-07-12-t3-lsp-migration-design.md`
//! plus `.superpowers/sdd/t3-stage-split.md` measured `ResolveIndex::build` +
//! `BodyMap::build` at ~200-350ms on CDO scale — 2-3.5x rung 1's ENTIRE
//! 100ms budget — so rung 1 cannot afford to transiently rebuild them (the
//! brief's "documented contingency," now mandatory). The only way to avoid
//! that rebuild is to cache the borrow-context's SOURCE data
//! (`Vec<ParsedUnit>`, both workspace and embedded-source deps) across many
//! consecutive rung-1 saves — and that cache has to live SOMEWHERE with a
//! lifetime longer than one `apply_changes` call. [`Updater`] is that
//! somewhere: it owns `parsed: Vec<ParsedUnit>` as long-lived mutable
//! working state, so a transient `ResolveIndex`/`BodyMap` pair can borrow it
//! fresh on every apply without needing to re-parse anything (rung 1) or
//! re-parse only what changed (rung 2). No self-referential struct is
//! needed: the borrow-context is built and dropped entirely WITHIN each
//! `apply_batch` call — see [`Updater::apply_rung1`]/[`Updater::apply_rung2`].
//!
//! The brief's "return/expose the Rung taken (test hook)" requirement is met
//! MORE directly than its suggested `Cell<Rung>` field: [`Updater::apply_batch`]
//! simply returns the [`Rung`] it took as part of its `Option` tuple.
//!
//! # Rung summary (binding; see the task brief + the def-surface audit,
//! `docs/superpowers/specs/2026-07-12-t3-def-surface-audit.md`, for the full
//! justification)
//!
//! - **Rung 1** (every `FileSaved` in the batch is a known workspace file
//!   whose fresh parse is `ParseStatus::Clean` AND whose [`DefSurface`]
//!   fingerprint is unchanged): re-resolve ONLY the touched file(s) against a
//!   transient `ResolveIndex`/`BodyMap` built over the UNCHANGED cached
//!   graph + the updater's own `parsed` (with the touched file(s) already
//!   spliced in) — see [`Updater::apply_rung1`].
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

use crate::lsp::def_surface::def_surface_fingerprint;
use crate::lsp::snapshot::{
    DeclEntry, LspSnapshot, ParsedFileEntry, build_decl_by_id, build_incoming, recompute_file,
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

/// Which rung an [`Updater::apply_batch`] call actually took — the brief's
/// "test hook," exposed directly via the return value rather than a
/// separate `Cell` field (see the module doc's mapping section).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rung {
    One,
    Two,
    Three,
}

/// The incremental updater's owned, long-lived working state. Constructed
/// once (typically from [`LspSnapshot::build_full_with_parsed`]'s second
/// return value) and then driven by a sequence of [`Updater::apply_batch`]
/// calls, each producing a fresh [`LspSnapshot`] to publish via
/// [`SharedSnapshot::swap`].
pub struct Updater {
    workspace_root: PathBuf,
    /// Every source-bearing app's current parse — workspace AND
    /// embedded-source deps. This is the cache the module doc's mapping
    /// section explains: rebuilding `ResolveIndex`/`BodyMap` needs SOME
    /// `&[ParsedUnit]` to borrow, and re-parsing the whole snapshot on every
    /// save is exactly the ~200-350ms cost rung 1 cannot afford. Rung 1
    /// splices one file's fresh [`ParsedFile`] into the primary (workspace)
    /// unit's file list; rung 2 replaces/removes entries in that same list
    /// (deps untouched); rung 3 replaces this whole `Vec` wholesale.
    parsed: Vec<ParsedUnit>,
}

impl Updater {
    #[must_use]
    pub fn new(workspace_root: PathBuf, parsed: Vec<ParsedUnit>) -> Self {
        Updater {
            workspace_root,
            parsed,
        }
    }

    /// Apply one coalesced batch against `cur` (the currently-published
    /// snapshot), returning the new snapshot + rung taken, or `None` when
    /// there is nothing to publish (every event was a no-op — see
    /// [`classify_path`]/the IO-race note below) or a rung-3 rebuild failed
    /// (fail-closed: `cur` — already published — survives untouched; this
    /// method does not mutate `self.parsed` in that case, since
    /// [`Updater::apply_rung3`] only commits the replacement after
    /// `LspSnapshot::build_full_with_parsed` succeeds).
    pub fn apply_batch(
        &mut self,
        cur: &LspSnapshot,
        batch: &[ChangeEvent],
    ) -> Option<(LspSnapshot, Rung)> {
        if batch.is_empty() {
            return None;
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
                        // file only, rather than failing the whole batch —
                        // fail-closed does not mean "never make progress,"
                        // it means "never fabricate content" — leaving the
                        // file's last-known-good state untouched satisfies
                        // that without discarding the rest of a legitimate
                        // batch.
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
            return self.apply_rung3(cur);
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
            Some(self.apply_rung2(cur, planned))
        } else if planned.is_empty() {
            None // every event in the batch was skipped (IO race) — no-op
        } else {
            Some(self.apply_rung1(cur, planned))
        }
    }

    // -----------------------------------------------------------------------
    // Rung 1: body-only edit(s), fingerprint(s) unchanged
    // -----------------------------------------------------------------------

    /// Rung 1: `planned` is guaranteed (by `apply_batch`'s gating) to
    /// contain only `Planned::Save { fingerprint_changed: false, .. }`
    /// entries. Re-resolves ONLY the touched files against a transient
    /// `ResolveIndex`/`BodyMap` built over the UNCHANGED `cur.graph` +
    /// `self.parsed` (with the touched files already spliced in) — every
    /// other file's edge bucket/decl list/parsed entry is shared via a
    /// cheap `Arc::clone` from `cur`, never recomputed. `event_edges` is
    /// carried forward unchanged: any real event-wiring change (publisher
    /// kind, `IncludeSender`, subscriber attributes) is itself part of the
    /// `DefSurface` fingerprint (audit §4 items 14/16/17), so it would
    /// already have forced `fingerprint_changed = true` and never reached
    /// this method — rung 1 does not need a SEPARATE check for it.
    fn apply_rung1(&mut self, cur: &LspSnapshot, planned: Vec<Planned>) -> (LspSnapshot, Rung) {
        let primary_idx = self.ensure_primary_unit_idx(&cur.snap);

        // Splice every fresh file into the working parse BEFORE building
        // `BodyMap`, so the touched file's own lookups (including a
        // recursive self-call) see the fresh parse — sound per the def-
        // surface audit (§3.4/§6.1): only a file's OWN body is ever read
        // this way; every OTHER file's resolution reads are surface-only
        // (signatures), never body content, so they are unaffected by which
        // version of THIS file's body happens to be spliced in.
        let mut touched: Vec<String> = Vec::with_capacity(planned.len());
        for p in planned {
            if let Planned::Save { vp, pf, .. } = p {
                splice_file(&mut self.parsed[primary_idx], *pf);
                touched.push(vp);
            }
        }

        let index = ResolveIndex::build(&cur.graph);
        let body_map = BodyMap::build(&cur.graph, &self.parsed);
        let obj_node_map: HashMap<ObjectNodeId, &ObjectNode> = cur
            .graph
            .objects
            .iter()
            .map(|o| (o.id.clone(), o))
            .collect();
        let primary_app_ref = cur
            .graph
            .apps
            .find(&cur.snap.workspace_app)
            .expect("the workspace app must already be interned in an existing graph");

        let mut edges_by_file = cur.edges_by_file.clone();
        let mut decls_by_file = cur.decls_by_file.clone();
        let mut parsed_files = cur.parsed.clone();

        for vp in &touched {
            let pf = self.parsed[primary_idx]
                .files
                .iter()
                .find(|f| &f.virtual_path == vp)
                .expect("just spliced above");
            let (edges, surface, decls) = recompute_file(
                pf,
                primary_app_ref,
                &cur.graph,
                &index,
                &body_map,
                &obj_node_map,
            );
            edges_by_file.insert(vp.clone(), Arc::new(edges));
            decls_by_file.insert(vp.clone(), Arc::new(decls));

            // A SECOND, independent parse for the snapshot's own owned copy
            // — see `LspSnapshot::build_full_with_parsed`'s doc for why
            // `AlFile`'s lack of a `Clone` impl makes this the honest
            // choice rather than a workaround (this file's content is
            // small; one extra parse costs microseconds, negligible against
            // the 100ms rung-1 budget).
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
        }

        let event_edges = Arc::clone(&cur.event_edges);
        let decl_by_id = build_decl_by_id(&decls_by_file);
        let incoming = build_incoming(&edges_by_file, &event_edges);

        let snapshot = LspSnapshot {
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
        };
        (snapshot, Rung::One)
    }

    // -----------------------------------------------------------------------
    // Rung 2: definition-surface change / file add / file delete
    // -----------------------------------------------------------------------

    /// Rung 2: apply every save/remove to the working primary (workspace)
    /// `ParsedUnit`, rebuild the workspace layer of the graph over the
    /// UNCHANGED cached `dep_layer`, then re-resolve EVERY workspace file
    /// (never just the touched ones — a signature change in one file can
    /// change how ANY other file's call sites resolve, which is exactly why
    /// rung 2 exists) and rebuild every derived index wholesale.
    fn apply_rung2(&mut self, cur: &LspSnapshot, planned: Vec<Planned>) -> (LspSnapshot, Rung) {
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

        let snapshot = LspSnapshot {
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
        };
        (snapshot, Rung::Two)
    }

    // -----------------------------------------------------------------------
    // Rung 3: deps changed / overflow / non-workspace-shaped path
    // -----------------------------------------------------------------------

    /// Rung 3: full rebuild from disk, including a fresh dep layer. Only
    /// commits `self.parsed`'s replacement AFTER
    /// [`LspSnapshot::build_full_with_parsed`] succeeds — on failure,
    /// `self.parsed` is left untouched and `None` is returned so `cur`
    /// (already published) survives (fail-closed).
    fn apply_rung3(&mut self, cur: &LspSnapshot) -> Option<(LspSnapshot, Rung)> {
        let (mut snapshot, parsed) = LspSnapshot::build_full_with_parsed(&self.workspace_root)?;
        // `build_full_with_parsed` always produces generation 0 (a fresh
        // batch build has no prior generation) — override so the counter
        // stays monotonic across every rung, including rung 3, rather than
        // going backwards.
        snapshot.generation = cur.generation + 1;
        self.parsed = parsed;
        Some((snapshot, Rung::Three))
    }

    // -----------------------------------------------------------------------
    // Small helpers
    // -----------------------------------------------------------------------

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

/// Spawn the updater thread: blocks for the first event, then drains
/// everything else that arrives within [`DEBOUNCE_WINDOW`] of the first,
/// coalesces per path, applies the batch, and — on success — swaps the new
/// snapshot into `shared` and invokes `on_swap(old, new)`. Exits cleanly
/// when the sending side of `rx` is dropped.
pub fn spawn_updater(
    shared: Arc<SharedSnapshot>,
    rx: Receiver<ChangeEvent>,
    workspace_root: PathBuf,
    initial_parsed: Vec<ParsedUnit>,
    on_swap: impl Fn(&LspSnapshot, &LspSnapshot) + Send + 'static,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let mut updater = Updater::new(workspace_root, initial_parsed);
        loop {
            let Ok(first) = rx.recv() else {
                return; // sender dropped — shut down cleanly
            };
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
            let batch = coalesce_batch(batch);

            let cur = shared.get();
            if let Some((new_snapshot, _rung)) = updater.apply_batch(&cur, &batch) {
                let new_arc = Arc::new(new_snapshot);
                shared.swap(Arc::clone(&new_arc));
                on_swap(&cur, &new_arc);
            }
            // `None`: nothing to publish (no-op batch) or a rung-3 build
            // failed — `cur` (already published) survives untouched.
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
}
