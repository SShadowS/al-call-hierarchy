# Owned DeclSurface Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the borrowed `BodyMap<'a>` with a fully-owned two-tier `DeclSurface` so dependency parse arenas (~99 MB+ of `AlFile` IR) can be dropped after the first full build, cutting LSP steady-state RSS from ~2 GB toward ~150–300 MB.

**Architecture:** A compact owned projection (`RoutineMeta`: name, origins, `parse_incomplete`, param `ty`/`by_ref` — never the body) replaces the borrowed `&RoutineDecl` in the resolution decl-lookup surface. The dep tier is frozen into an `Arc` once at startup/rung-3 and forwarded by `Arc::clone` through rungs 1/2 (sound because `AppRef` indices are stable across rungs 1/2 — the `DepLayer`'s `AppRegistry` is cloned, never re-interned). The updater then retains only the workspace `ParsedUnit`.

**Tech Stack:** Rust, existing crates only (no new dependencies).

**Spec:** `docs/superpowers/specs/2026-07-13-owned-decl-surface-design.md` (approved). The read-surface audit is complete: the ONLY `RoutineDecl` fields read through `BodyMap` accessors in production code are `name`, `origin`, `name_origin`, `parse_incomplete`, `params[].ty`, `params[].by_ref` (plus `enclosing_member` in body_map.rs's own tests). **`decl.body` is never read through BodyMap.**

## Global Constraints

- Branch: `feat/owned-decl-surface`, created off `feat/perf-safe-wins` (this work builds on the Arc-sharing commits; that branch is not yet merged).
- Format touched files with `rustfmt <file>` per-file only — NEVER `cargo fmt`.
- `cargo clippy --all-targets --all-features` must be clean before every commit.
- Update `CHANGELOG.md` (Unreleased) in every code task's commit ([Keep a Changelog](https://keepachangelog.com/) format).
- Stage only intended paths — never `git add -A`. Never push or merge to `master`.
- **Zero golden changes**: `REGEN_TEMP_GOLDENS=1` must never be needed. This refactor must be behaviorally invisible — resolution output byte-identical.
- Parity gate `tests/lsp_incremental_parity.rs` must pass at every commit.
- **Stop-and-reassess rule:** if you find a resolution path reading a dep routine *body* through a BodyMap/DeclSurface lookup, STOP and report BLOCKED — the design assumed none exists.
- Source-tier lookup miss must keep resolving to `Unknown(IndexIntegrationGap)` (resolver.rs ~192) — never add a silent fallback.
- `CDO_WS`-gated tests skip silently when the env var is unset — expected in most environments.

---

### Task 1: `RoutineMeta` + `DeclSurface` module

**Files:**
- Create: `src/program/resolve/decl_surface.rs`
- Modify: `src/program/resolve/mod.rs` (register module)
- Modify: `CHANGELOG.md`

**Interfaces:**
- Consumes: `al_syntax::ir::{RoutineDecl, Origin}`, `crate::program::graph::ProgramGraph`, `crate::program::node::{AppRef, ObjKey, ObjectNodeId, RoutineNodeId}`, `crate::program::sig_fp::source_routine_node_id`, `crate::snapshot::ParsedUnit`.
- Produces (used verbatim by Tasks 2–3):
  ```rust
  pub struct ParamMeta { pub ty: Option<String>, pub by_ref: bool }
  pub struct RoutineMeta {
      pub name: String,
      pub enclosing_member: Option<String>, // name half only
      pub parse_incomplete: bool,
      pub params: Vec<ParamMeta>,
      pub origin: al_syntax::ir::Origin,
      pub name_origin: al_syntax::ir::Origin,
      pub virtual_path: String,
  }
  impl RoutineMeta { pub fn from_decl(decl: &RoutineDecl, virtual_path: &str) -> Self }
  pub type DepMetaMap = HashMap<RoutineNodeId, RoutineMeta>;
  pub struct DeclSurface { /* local: HashMap<RoutineNodeId, RoutineMeta>, frozen: Option<Arc<DepMetaMap>> */ }
  impl DeclSurface {
      pub fn build(graph: &ProgramGraph, parsed: &[ParsedUnit]) -> Self
      pub fn with_frozen(self, frozen: Arc<DepMetaMap>) -> Self
      pub fn freeze_dep_tier(&mut self, primary: AppRef) -> Arc<DepMetaMap>
      pub fn get(&self, id: &RoutineNodeId) -> Option<&RoutineMeta>
      pub fn get_with_path(&self, id: &RoutineNodeId) -> Option<(&RoutineMeta, &str)>
  }
  ```

- [ ] **Step 1: Write the module with failing tests first.** Create `src/program/resolve/decl_surface.rs`. Port ALL scenarios from `src/program/resolve/body_map.rs`'s `#[cfg(test)] mod tests` (read that file first — reuse its fixture helpers `make_app_id`/graph construction verbatim, same expected values): two routines in one object retrievable by distinct ids; same-named member triggers on different fields stored under distinct keys; last-write-wins on true same-key collision; unit with app absent from `graph.apps` silently skipped; empty units produce empty surface. Add NEW tests for the two-tier behavior:

```rust
#[test]
fn freeze_dep_tier_moves_non_primary_entries_and_lookup_still_serves_them() {
    // graph with two apps: primary (AppRef A) and dep (AppRef B), one routine each
    // build surface -> both entries in local tier
    let mut surface = DeclSurface::build(&graph, &units);
    let frozen = surface.freeze_dep_tier(primary_ref);
    // dep routine still found via get()/get_with_path() (served from frozen tier)
    assert!(surface.get(&dep_rid).is_some());
    // workspace routine still found (local tier)
    assert!(surface.get(&ws_rid).is_some());
    // the frozen map contains exactly the dep entry
    assert_eq!(frozen.len(), 1);
    assert!(frozen.contains_key(&dep_rid));
}

#[test]
fn with_frozen_composes_a_workspace_only_build_with_a_prior_dep_tier() {
    // simulate a rung: build from the WORKSPACE unit only, attach prior frozen tier
    let surface = DeclSurface::build(&graph, std::slice::from_ref(&ws_unit))
        .with_frozen(Arc::clone(&frozen));
    assert!(surface.get(&ws_rid).is_some());
    assert!(surface.get(&dep_rid).is_some()); // served by frozen tier
}

#[test]
fn local_tier_shadows_frozen_on_key_collision() {
    // same RoutineNodeId in both tiers: local must win (workspace-first lookup)
}
```

Implementation in the same file (TDD within one module is fine — write tests, watch them fail to compile, then implement):

```rust
//! `DeclSurface`: OWNED per-routine decl metadata, indexed by `RoutineNodeId`.
//!
//! Replaces the retired borrowed `BodyMap<'a>` (see the owned-decl-surface
//! design spec). Two tiers: `local` (workspace, rebuilt per rung) and
//! `frozen` (dependencies, built once at startup/rung-3 and `Arc`-forwarded
//! across rungs 1/2 — sound because `AppRef`s are stable across those rungs:
//! the `DepLayer`'s `AppRegistry` is cloned into every assembled graph).
//! Lookup is local-first, so a workspace entry always shadows a frozen one.
//!
//! `RoutineMeta` holds EXACTLY the fields resolution reads (audited): never
//! the routine body — dropping the dep parse arenas is the whole point.

use std::collections::HashMap;
use std::sync::Arc;

use al_syntax::ir::{Origin, RoutineDecl};

use crate::program::graph::ProgramGraph;
use crate::program::node::{AppRef, ObjKey, ObjectNodeId, RoutineNodeId};
use crate::program::sig_fp::source_routine_node_id;
use crate::snapshot::ParsedUnit;

#[derive(Debug, Clone)]
pub struct ParamMeta {
    pub ty: Option<String>,
    pub by_ref: bool,
}

#[derive(Debug, Clone)]
pub struct RoutineMeta {
    pub name: String,
    /// Name half of `RoutineDecl::enclosing_member` (origin half unused).
    pub enclosing_member: Option<String>,
    pub parse_incomplete: bool,
    pub params: Vec<ParamMeta>,
    pub origin: Origin,
    pub name_origin: Origin,
    pub virtual_path: String,
}

impl RoutineMeta {
    pub fn from_decl(decl: &RoutineDecl, virtual_path: &str) -> Self {
        RoutineMeta {
            name: decl.name.clone(),
            enclosing_member: decl.enclosing_member.as_ref().map(|(n, _)| n.clone()),
            parse_incomplete: decl.parse_incomplete,
            params: decl
                .params
                .iter()
                .map(|p| ParamMeta { ty: p.ty.clone(), by_ref: p.by_ref })
                .collect(),
            origin: decl.origin.clone(),
            name_origin: decl.name_origin.clone(),
            virtual_path: virtual_path.to_string(),
        }
    }
}

pub type DepMetaMap = HashMap<RoutineNodeId, RoutineMeta>;

pub struct DeclSurface {
    local: HashMap<RoutineNodeId, RoutineMeta>,
    frozen: Option<Arc<DepMetaMap>>,
}

impl DeclSurface {
    /// Build from a parsed snapshot. Mirrors the retired `BodyMap::build`
    /// EXACTLY: units whose `AppId` is absent from `graph.apps` are silently
    /// skipped (open-world gap); object key is numeric id when present, else
    /// lowercased name; last-write-wins on true same-key collision.
    pub fn build(graph: &ProgramGraph, parsed: &[ParsedUnit]) -> Self {
        let mut local = HashMap::new();
        for unit in parsed {
            let Some(app_ref) = graph.apps.find(&unit.app) else { continue };
            for pf in &unit.files {
                for obj in &pf.file.objects {
                    let key = match obj.id {
                        Some(n) => ObjKey::Id(n),
                        None => ObjKey::Name(obj.name.to_ascii_lowercase()),
                    };
                    let obj_id = ObjectNodeId { app: app_ref, kind: obj.kind, key };
                    for routine in &obj.routines {
                        let r_id = source_routine_node_id(obj_id.clone(), routine);
                        local.insert(r_id, RoutineMeta::from_decl(routine, &pf.virtual_path));
                    }
                }
            }
        }
        DeclSurface { local, frozen: None }
    }

    #[must_use]
    pub fn with_frozen(mut self, frozen: Arc<DepMetaMap>) -> Self {
        self.frozen = Some(frozen);
        self
    }

    /// Move every non-`primary` entry out of the local tier into the frozen
    /// tier; returns the frozen map (also retained by `self` for lookups).
    pub fn freeze_dep_tier(&mut self, primary: AppRef) -> Arc<DepMetaMap> {
        let mut dep: DepMetaMap = HashMap::new();
        let local = std::mem::take(&mut self.local);
        for (id, meta) in local {
            if id.object.app == primary {
                self.local.insert(id, meta);
            } else {
                dep.insert(id, meta);
            }
        }
        let frozen = Arc::new(dep);
        self.frozen = Some(Arc::clone(&frozen));
        frozen
    }

    pub fn get(&self, id: &RoutineNodeId) -> Option<&RoutineMeta> {
        self.local
            .get(id)
            .or_else(|| self.frozen.as_ref().and_then(|f| f.get(id)))
    }

    pub fn get_with_path(&self, id: &RoutineNodeId) -> Option<(&RoutineMeta, &str)> {
        self.get(id).map(|m| (m, m.virtual_path.as_str()))
    }
}
```

- [ ] **Step 2: Register the module.** In `src/program/resolve/mod.rs`, add `pub mod decl_surface;` alongside the existing `pub mod body_map;` (body_map is deleted in Task 2, not here — the two coexist for exactly one task).

- [ ] **Step 3: Run the new tests.**

Run: `cargo test --lib decl_surface`
Expected: all new tests PASS (ported scenarios + the three two-tier tests).

- [ ] **Step 4: Clippy + rustfmt.**

Run: `cargo clippy --all-targets --all-features` (clean), then `rustfmt src/program/resolve/decl_surface.rs src/program/resolve/mod.rs`.

- [ ] **Step 5: CHANGELOG + commit.** Add under `## [Unreleased]` / `### Added`: `- DeclSurface: owned two-tier routine-decl metadata surface (workspace tier + Arc-frozen dependency tier), groundwork for dropping dependency parse arenas from LSP steady state.`

```bash
git add src/program/resolve/decl_surface.rs src/program/resolve/mod.rs CHANGELOG.md
git commit -m "feat: add owned two-tier DeclSurface (RoutineMeta projection)"
```

---

### Task 2: Migrate every consumer from `BodyMap` to `DeclSurface`; delete `body_map.rs`

**Files:**
- Delete: `src/program/resolve/body_map.rs` (and its `pub mod body_map;` line in `src/program/resolve/mod.rs`)
- Modify: `src/program/resolve/resolver.rs`, `src/program/resolve/arg_dispatch.rs`, `src/program/resolve/receiver.rs`, `src/program/resolve/full.rs`, `src/program/resolve/stub.rs`, `src/program/resolve/differential.rs`, `src/lsp/snapshot.rs`, `src/lsp/updater.rs`, `src/program/sig_fp.rs` + `src/program/node_extract.rs` + `src/lsp/custom.rs` + `src/lsp/def_surface.rs` (doc-comment references only)
- Modify: `CHANGELOG.md`

**Interfaces:**
- Consumes: Task 1's `DeclSurface`/`RoutineMeta`/`ParamMeta` exactly as defined there.
- Produces: every function that took `body_map: &BodyMap<'_>` now takes `surface: &DeclSurface` (same parameter position); `candidate_param_infos` takes `meta: &RoutineMeta` instead of `decl: &RoutineDecl`. Task 3 relies on these signatures.

This task is deliberately one atomic commit: deleting `body_map.rs` makes the compiler enumerate every consumer — that IS the read-surface audit, enforced. The changes are mechanical because `RoutineMeta` keeps the field names `name`/`origin`/`name_origin`/`parse_incomplete`/`params` and `DeclSurface::build` keeps `BodyMap::build`'s exact signature.

- [ ] **Step 1: Delete the old module.** Remove `src/program/resolve/body_map.rs` and its `mod` line. Its unit tests are already ported (Task 1).

- [ ] **Step 2: Mechanical migration, guided by the compiler.** Apply these transformations everywhere the build breaks (production AND test code):
  1. `use crate::program::resolve::body_map::BodyMap;` → `use crate::program::resolve::decl_surface::DeclSurface;`
  2. Type `&BodyMap<'_>` (parameter) → `&DeclSurface`; `BodyMap<'static>` (test fixture returns in resolver.rs ~4342, ~4531) → `DeclSurface`.
  3. `BodyMap::build(` → `DeclSurface::build(` (identical arguments — this covers the ~150 test call sites in resolver.rs / receiver.rs / arg_dispatch.rs / full.rs and the production sites in full.rs:771, stub.rs:93, differential.rs:507, lsp/snapshot.rs:287, lsp/updater.rs:252/414/875 and the test blocks at updater.rs ~1600/1944).
  4. Variable naming: keep `body_map` variable names where churn would be large in test code, but rename the ~10 PRODUCTION bindings and parameters to `surface` (resolver.rs `make_routine_route`/`candidate_param_infos_either`/`emit_event_flow_edges` and every fn that threads it; full.rs; stub.rs; lsp/snapshot.rs `recompute_file`/`build_dep_indexes`; updater.rs `apply_rung1_core`). Update the doc comments that explain the borrow ("BodyMap borrows self.parsed") ONLY where they become false — Task 3 rewrites the updater's ownership story; here just fix compile errors and blatant lies introduced by this task.
  5. `body_map.get(rid)` / `body_map.get_with_path(rid)` → same calls on the new type. Field reads compile unchanged (`meta.origin.byte.start`, `meta.name_origin.start.row`, `meta.name.clone()`, etc.).

- [ ] **Step 3: Retype `candidate_param_infos`.** In `src/program/resolve/arg_dispatch.rs` (~line 1139), change:

```rust
pub(crate) fn candidate_param_infos(
    meta: &crate::program::resolve::decl_surface::RoutineMeta,
    from: &ObjectNodeId,
    graph: &ProgramGraph,
    index: &ResolveIndex,
) -> Option<Vec<ParamDispatchInfo>> {
    if meta.parse_incomplete {
        return None;
    }
    let mut out = Vec::with_capacity(meta.params.len());
    for p in &meta.params {
        let ty = p.ty.as_deref()?;
        let canonical = dispatch_canonical_type_text(ty, from, graph, index)?;
        out.push(ParamDispatchInfo {
            canonical,
            exact_text: normalize_type_text(ty),
            by_ref: p.by_ref,
        });
    }
    Some(out)
}
```

(keep the existing doc comment and interior logic — ONLY the input type and field paths change; body semantics identical.) In resolver.rs's `candidate_param_infos_either` (~line 626): `if let Some(meta) = surface.get(rid) { return candidate_param_infos(meta, &rid.object, graph, index); }`. The two tests at arg_dispatch.rs ~2475/~2491 build a `RoutineDecl` fixture — wrap at the call site: `candidate_param_infos(&RoutineMeta::from_decl(&decl, "test.al"), &from, &graph, &index)`.

- [ ] **Step 4: Full verification — this is the compile-enforced audit.** If ANY consumer needs a `RoutineDecl` field that `RoutineMeta` lacks, apply the Global Constraints stop-and-reassess rule: if the missing field is `body`/arena data → BLOCKED; if it is another scalar/decl-level field (e.g. `kind`, `return_type`) → add it to `RoutineMeta` + `from_decl`, note it in the report, and continue.

Run: `cargo test` (full suite)
Expected: ALL green, zero golden diffs (`git status` shows no modified fixture/golden files).

Run: `cargo test --test lsp_incremental_parity`
Expected: all parity tests PASS.

- [ ] **Step 5: Clippy + rustfmt touched files + CHANGELOG + commit.** CHANGELOG under `### Changed`: `- Resolution decl lookups migrated from the borrowed BodyMap<'a> to the owned DeclSurface; BodyMap deleted. No behavioral change (goldens unchanged).`

```bash
git add -u src/ CHANGELOG.md   # -u stages tracked modifications/deletions only; verify with git status first
git commit -m "refactor: migrate resolution decl lookups from BodyMap<'a> to owned DeclSurface"
```

---

### Task 3: Drop dependency parse arenas from the LSP lifecycle

**Files:**
- Modify: `src/lsp/snapshot.rs` (`LspSnapshot` struct, `from_context`, `build_full`, `build_full_with_parsed`, `build_dep_indexes`)
- Modify: `src/lsp/updater.rs` (`Updater` struct + all rungs + hot loop + module doc)
- Modify: `tests/lsp_incremental_parity.rs` (helper retype + new drop-proof/forwarding tests)
- Modify: `CHANGELOG.md`

**Interfaces:**
- Consumes: Task 2's signatures (`DeclSurface` everywhere; `build_dep_indexes(graph, &DeclSurface, parsed, primary)`).
- Produces:
  - `LspSnapshot` gains `pub dep_meta: Arc<crate::program::resolve::decl_surface::DepMetaMap>`.
  - `from_context` / `build_full_with_parsed` return `(LspSnapshot, ParsedUnit)` — the WORKSPACE unit only (dep `ParsedUnit`s dropped inside).
  - `Updater { workspace_root, workspace: ParsedUnit, pending }` — `parsed: Vec<ParsedUnit>` and `ensure_primary_unit_idx` are gone.

- [ ] **Step 1: Write the failing tests** (in `tests/lsp_incremental_parity.rs`, reusing its existing `copy_fixture_lsp_diff_deps_to_tempdir()` + `build_full_with_parsed(dir)` helpers — retype the helper to the new return type as part of this step):

```rust
/// The whole point of the owned-DeclSurface design: after a full build, the
/// caller receives ONLY the workspace ParsedUnit — dependency parse arenas
/// are dropped, not retained for the updater's lifetime.
#[test]
fn build_full_with_parsed_returns_only_the_workspace_unit() {
    let dir = copy_fixture_lsp_diff_deps_to_tempdir();
    let (snap, workspace) = build_full_with_parsed(dir.path());
    assert_eq!(workspace.app, snap.snap.workspace_app);
    // and the dep tier is populated (deps were parsed, projected, then dropped)
    assert!(!snap.dep_meta.is_empty(), "dep tier must hold the projected dep decls");
}

/// Rungs 1 and 2 must FORWARD the frozen dep tier (and the dep query maps),
/// never rebuild them: Arc identity proves zero recompute.
#[test]
fn rung1_and_rung2_forward_dep_meta_dep_decls_and_dep_texts_by_arc_identity() {
    // build; apply a body-only edit (rung 1); assert Arc::ptr_eq(&old.dep_meta, &new.dep_meta),
    // same for dep_decl_by_id and dep_texts; then apply a signature-changing
    // edit (rung 2) and assert the same three identities again.
    // (Model the edit mechanics on this file's existing rung-1/rung-2 parity tests.)
}
```

- [ ] **Step 2: Run them to verify failure.**

Run: `cargo test --test lsp_incremental_parity build_full_with_parsed_returns_only`
Expected: FAIL to compile (no `dep_meta` field, tuple type mismatch) — that's the RED.

- [ ] **Step 3: Rework `from_context` in `src/lsp/snapshot.rs`.** In the transient borrow phase (~line 270–325): build the surface, freeze the dep tier immediately, then use the two-tier surface for everything (exercising two-tier lookup on the very first build):

```rust
let index = ResolveIndex::build(&graph);
let mut surface = DeclSurface::build(&graph, &parsed);
let dep_meta = surface.freeze_dep_tier(primary_app_ref);
let surface = surface; // immutable from here
```

`recompute_file(...)`, `emit_event_flow_edges(...)` calls pass `&surface` (Task 2 already retyped them). `build_dep_indexes` simplifies: `dep_decl_by_id` no longer needs the surface parameter's map walk against `graph.routines` changed — keep its current shape (walk `graph.routines`, skip primary, `surface.get_with_path`) which now serves from the frozen tier; `dep_texts` still built from `parsed` (dep units are alive here). After the borrow phase: store `dep_meta: Arc::clone(&dep_meta)` in the `LspSnapshot`, and change the function tail to extract and return ONLY the workspace unit:

```rust
let workspace_unit = parsed
    .into_iter()
    .find(|u| u.app == snap_arc.workspace_app)
    .unwrap_or_else(|| ParsedUnit { app: snap_arc.workspace_app.clone(), files: vec![] });
(lsp_snapshot, workspace_unit)
```

(dep `ParsedUnit`s drop here — the memory win's exact release point; add a one-line comment saying so). `build_full` takes `.0` as today; `build_full_with_parsed` returns the tuple. Update both functions' doc comments.

- [ ] **Step 4: Rework the `Updater`.** In `src/lsp/updater.rs`:
  1. Struct: `parsed: Vec<ParsedUnit>` → `workspace: ParsedUnit`; delete `ensure_primary_unit_idx`; `Updater::new(workspace_root, workspace: ParsedUnit)`.
  2. `flush_pending`: `splice_file(&mut self.workspace, pf)` directly (no index search).
  3. `file_provenance`: read `self.workspace.files` directly.
  4. Rung 1 (`apply_batch`'s Rung1 arm AND `spawn_updater`'s hot loop ~line 875): `let surface = DeclSurface::build(&cur.graph, std::slice::from_ref(&self.workspace)).with_frozen(Arc::clone(&cur.dep_meta));` — the per-save all-units rebuild disappears.
  5. `apply_rung1_core`: parameter `surface: &DeclSurface` (Task 2); new `LspSnapshot` literal adds `dep_meta: Arc::clone(&cur.dep_meta),` next to the existing `dep_decl_by_id`/`dep_texts` forwards.
  6. Rung 2 (`apply_rung2`): splice into `self.workspace`; `assemble_program_graph(&cur.dep_layer, &self.workspace, &cur.snap)`; surface as in (4); DELETE the `build_dep_indexes` recompute and its "would dangle" comment — the snapshot literal forwards `dep_decl_by_id: Arc::clone(&cur.dep_decl_by_id)`, `dep_texts: Arc::clone(&cur.dep_texts)`, `dep_meta: Arc::clone(&cur.dep_meta)` with a comment: dependency source cannot change at rung 2 and all three maps are fully owned (keyed by `RoutineNodeId` whose `AppRef`s are stable — the graph reuses the cached `dep_layer`'s cloned `AppRegistry`), so forwarding is sound; rung 3 is the only rung that rebuilds them.
  7. Rung 3 (`apply_rung3`): `let Some((mut snapshot, workspace)) = LspSnapshot::build_full_with_parsed(...)`; `self.workspace = workspace;`.
  8. Rewrite the module doc's ownership story (~lines 23–68): the "BodyMap borrows self.parsed" constraint is gone; the surviving reason the hot loop caches `index`/`surface` across rung-1 calls is COST, and the `pending` overlay still exists so a cached surface stays consistent with the published snapshot between flushes. Also update `Updater.parsed`'s old field doc and `build_dep_indexes`'s doc in snapshot.rs ("called from BOTH from_context and apply_rung2" — now from_context only).
  9. Fix the in-file `#[cfg(test)]` blocks (~lines 1588–1975) that construct `Updater`/`BodyMap` directly — same mechanical patterns.

- [ ] **Step 5: Run the new tests + full verification.**

Run: `cargo test --test lsp_incremental_parity`
Expected: ALL PASS including the two new tests (GREEN).

Run: `cargo test`
Expected: all green, zero golden diffs.

- [ ] **Step 6: Clippy + rustfmt touched files + CHANGELOG + commit.** CHANGELOG under `### Changed`: `- LSP steady state no longer retains dependency parse arenas: the updater keeps only the workspace ParsedUnit; the frozen dep DeclSurface tier, dep_decl_by_id and dep_texts are Arc-forwarded across rungs 1/2 and rebuilt only at rung 3.`

```bash
git add src/lsp/snapshot.rs src/lsp/updater.rs tests/lsp_incremental_parity.rs CHANGELOG.md
git commit -m "perf: drop dependency parse arenas after first build (owned DeclSurface lifecycle)"
```

---

### Task 4: Measurement + close-out

**Files:**
- Modify: `docs/perf-regression-t3-vs-0.9.3.md` (append §7)
- Modify: `CHANGELOG.md` only if numbers warrant a note (optional)

**Interfaces:** none (measurement + docs).

- [ ] **Step 1: Benches + release gate.**

Run: `cargo bench --bench lsp_pipeline` (expect rung-1/rung-2 medians to IMPROVE — the all-units BodyMap rebuild is gone; record all medians), then `cargo test --release --test perf_bounds`.
Expected: perf_bounds 9/9 PASS.

- [ ] **Step 2: CDO gate (skip gracefully if `CDO_WS`/workspace unavailable — note it in the doc).**

Run: `scripts/cdo-gate U:\Git\<cdo-workspace-path-if-known>` or with `CDO_WS` set per its header.
Expected: exit 0 — zero-unknown ratchet, `ambiguousResolved` pin, coverage contract all hold. Any failure = a missing `RoutineMeta` field or lifecycle bug: STOP, report BLOCKED with the failing gate's output.

- [ ] **Step 3: LSP steady-state RSS, before/after.** Workspace: `U:\Git\DO.Support-SlowDOSetup\DocumentOutput\Cloud` (skip gracefully with a doc note if unreachable). Use the perf doc §5's repro: a raw stdio LSP session (spawn release binary with no args from the workspace root as cwd is NOT how it works — the client sends `initialize` with `rootUri` pointing at the workspace; percent-encode spaces in URIs; skip `publishDiagnostics` notifications when matching responses). Drive: `initialize` → `initialized` → `textDocument/didOpen` on one workspace file → wait for first successful `textDocument/prepareCallHierarchy` response → then sample RSS (`scripts/peak_rss.py` polls peak; for steady state, read the process's working set after the first response, e.g. `Get-Process -Id <pid> | Select WorkingSet64`, several samples 10s apart). BEFORE number: check out `feat/perf-safe-wins` (pre-Task-1 of this plan), build release, measure. AFTER: this branch's HEAD. Record both.

- [ ] **Step 4: Append §7 to `docs/perf-regression-t3-vs-0.9.3.md`:** dated heading `## 7. <date> close-out: Mitigation 3 (owned DeclSurface) — IMPLEMENTED`; commit hashes of Tasks 1–3; bench medians table (rung 1/rung 2 before/after); CDO gate result; the LSP steady-state RSS before/after (the headline number — expectation was ~2,000 MB → ~150–300 MB); honest notes for anything skipped or anomalous.

- [ ] **Step 5: Commit (docs only).**

```bash
git add docs/perf-regression-t3-vs-0.9.3.md
git commit -m "docs: record owned-DeclSurface implementation and measured LSP steady-state RSS"
```
