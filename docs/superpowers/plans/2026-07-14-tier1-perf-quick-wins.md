# Tier-1 Perf Quick Wins Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land the three review-verified Tier-1 items from the 2026-07-14 investigation synthesis: (1) make `handlers::incoming` O(refs) instead of O(refs²), (2) delete the redundant `LspSnapshot.dep_decl_by_id` and serve dependency decl lookups from the existing `dep_meta` frozen tier (~103 MB RSS + ~150-200 ms cold start), (3) close the rung-1 measurement blind spot by benching/gating the production scoped-context path.

**Architecture:** All three changes are behaviour-preserving. Item 1 restructures one query handler's loop (output byte-identical — ordering is fixed by existing sorts). Item 2 unifies the two per-dep-routine maps onto `dep_meta` via a new borrowed `DeclView<'_>` served by `LspSnapshot::decl_and_text`, deleting `build_dep_indexes`'s O(127k) decl loop and the ~103 MB duplicate map. Item 3 extracts `spawn_updater`'s hot-loop context construction into a public `Rung1Context` type used by BOTH production and the new bench/gate, so the gate measures the path users actually run.

**Tech Stack:** Rust; criterion (`benches/lsp_pipeline.rs`); existing parity gate (`tests/lsp_incremental_parity.rs`); release perf gate (`tests/perf_bounds.rs`).

**Requirements sources:**
- `.superpowers/sdd/investigation-synthesis-2026-07-14.md` (Tier 1 + the Fable review-corrections section — read both)
- `.superpowers/sdd/improvement-hunt-report.md` (F1, F6 findings with evidence)
- Verified sound by an independent errors+performance review (Fable, 2026-07-14): Tier 1 "can be executed as written"; item 2's content parity holds by construction (both maps are built from the same frozen `RoutineMeta` source).

## Global Constraints

- **Zero functionality loss; zero behaviour change.** LSP responses must stay byte-identical; zero golden changes; the resolution `Histogram` taxonomy is untouchable.
- Base branch: `feat/tier1-perf-quick-wins` off `master` (571db0d).
- Format touched files with `rustfmt <file>` per file — NEVER `cargo fmt`.
- `cargo clippy --all-targets --all-features` must be clean at every commit.
- `cargo test` (full suite) green at every commit; `tests/lsp_incremental_parity.rs` green is the parity gate.
- CHANGELOG.md updated per code task (Keep a Changelog; group under Changed/Fixed).
- Stage only intended paths — never `git add -A`. NEVER stage the untracked files `scripts/peak_rss.py`, `finish-cleanup.ps1`, `finish-t1-cleanup.ps1`, `.panel/`.
- Every commit carries the trailer: `Co-authored-by: Copilot <223556219+Copilot@users.noreply.github.com>`
- Never push or merge to `master` without an explicit user request.

---

### Task 1: `incoming` one-pass grouping (F1 — O(refs²) → O(refs))

**Files:**
- Modify: `src/lsp/handlers.rs` (the `incoming` function, ~lines 155-217)
- Modify: `CHANGELOG.md`

**Interfaces:**
- Consumes: `LspSnapshot::edge(&EdgeRef) -> &ClassifiedEdge` (snapshot.rs:463), `LspSnapshot::decl_and_text` (unchanged in this task — Task 2 changes its return type LATER; this task keeps the `(decl, text)` destructuring exactly as-is).
- Produces: no API change — same `incoming(snap, enc, data) -> Vec<CallHierarchyIncomingCall>` signature, byte-identical output.

**Background for the implementer:** today's `incoming` makes three passes over `refs = snap.incoming[&data.node]`: (a) a `has_event_flow` pass resolving every ref, (b) a per-caller loop that RE-FILTERS all refs per caller (`refs.iter().filter(|r| snap.edge(r).edge.from == caller_id)`) — with `callers ≈ refs` this is O(refs²) — and (c) `snap.edge(r)` called again inside the filter body. Each `snap.edge(r)` hashes a file-path String. Measured 27 ms at 999-way fan-in. Output ordering is already pinned by `callers.sort()` and `from_ranges.sort_by_key(range_sort_key)` + `dedup()`, so grouping in one pass is provably byte-identical.

- [ ] **Step 1: Capture the BEFORE bench number**

Run: `cargo bench --bench lsp_pipeline -- query_handlers_1000_files/incoming`
Record the median (expect ~16-27 ms depending on machine). Save the output to `.superpowers/sdd/tier1-quick-wins/task-1-bench-before.txt`.

- [ ] **Step 2: Rewrite the grouping loop**

Replace the body of `incoming` (from the `let mut has_event_flow` line through the end of the `for caller_id in callers` loop) with a single-resolve grouping pass. Each `EdgeRef` is resolved through `snap.edge` EXACTLY once; per-caller edge order is `refs` order (same as today's filter); everything downstream is unchanged:

```rust
    let Some(refs) = snap.incoming.get(&data.node) else {
        return Vec::new();
    };

    // One pass: resolve every EdgeRef exactly once, grouping by caller.
    // (Previously this re-filtered ALL refs per distinct caller — O(refs²)
    // with a string-hashed map lookup per pair; see the 2026-07-14
    // improvement-hunt F1 finding.)
    let mut groups: HashMap<RoutineNodeId, (bool, Vec<&ClassifiedEdge>)> = HashMap::new();
    for r in refs {
        let ce = snap.edge(r);
        let entry = groups
            .entry(ce.edge.from.clone())
            .or_insert_with(|| (false, Vec::new()));
        entry.0 |= ce.edge.kind == EdgeKind::EventFlow;
        entry.1.push(ce);
    }

    let mut callers: Vec<RoutineNodeId> = groups.keys().cloned().collect();
    callers.sort();

    let mut out = Vec::new();
    for caller_id in callers {
        let (has_event_flow, edges) = &groups[&caller_id];
        let Some((decl, text)) = snap.decl_and_text(&caller_id) else {
            // The caller's own decl vanished from the current snapshot —
            // fail closed by dropping this group rather than guessing at a
            // position for an item we can no longer locate.
            continue;
        };
        let table = LineTable::new(text);

        let mut from_ranges: Vec<Range> = Vec::new();
        for ce in edges {
            let range = if ce.edge.kind == EdgeKind::EventFlow {
                // Rule 2: an EventFlow edge's own site span is stale-prone;
                // re-derive from the PUBLISHER's (== this caller's) fresh
                // name_origin instead.
                origin_to_range(&decl.name_origin, &table, enc)
            } else {
                canonical_span_to_range(&ce.edge.site.span, &table, enc)
            };
            from_ranges.push(range);
        }
        from_ranges.sort_by_key(range_sort_key);
        from_ranges.dedup();

        let tag = has_event_flow.then_some("[EventPublisher]");
        let item = build_item(snap, enc, decl, &table, decl_uri(snap, decl), tag);

        out.push(CallHierarchyIncomingCall {
            from: item,
            from_ranges,
        });
    }
    out
```

Notes: `has_event_flow` here is `&bool`, so `has_event_flow.then_some(...)` needs a deref — write `(*has_event_flow).then_some("[EventPublisher]")`. Add `ClassifiedEdge` to the existing `use` imports at the top of handlers.rs if it isn't imported (it lives where `snapshot.rs`'s `edges_by_file` value type comes from — follow snapshot.rs's own import). Update `incoming`'s doc comment: it still groups by caller; remove nothing about fail-closed semantics.

- [ ] **Step 3: Run the handler tests**

Run: `cargo test --lib handlers && cargo test --test lsp_incremental_parity`
Expected: PASS — output is byte-identical; existing `incoming_*` tests and the parity suite prove it.

- [ ] **Step 4: Capture the AFTER bench number**

Run: `cargo bench --bench lsp_pipeline -- query_handlers_1000_files/incoming`
Expected: median drops substantially (F1 predicts high-single-digit ms; residual is the per-caller `LineTable::new`, which is out of scope). Save to `.superpowers/sdd/tier1-quick-wins/task-1-bench-after.txt`. If it does NOT improve, STOP and investigate before committing — do not commit a no-op restructure.

- [ ] **Step 5: Full validation, CHANGELOG, commit**

Run: `cargo test` (full) and `cargo clippy --all-targets --all-features`. Both clean.
`rustfmt src/lsp/handlers.rs`.
CHANGELOG.md under `[Unreleased] > Changed`:
```markdown
- `callHierarchy/incomingCalls` now groups call sites by caller in a single
  pass (previously O(refs²) re-filtering with a string-hashed edge lookup per
  caller×ref pair) — measured ~NN ms → ~NN ms on the 999-way fan-in bench;
  output unchanged.
```
(Fill NN from steps 1/4.)

```bash
git add src/lsp/handlers.rs CHANGELOG.md
git commit -m "perf: single-pass caller grouping in incomingCalls (O(refs^2) -> O(refs))

Co-authored-by: Copilot <223556219+Copilot@users.noreply.github.com>"
```

---

### Task 2: Delete `dep_decl_by_id` — serve dependency decls from `dep_meta` via `DeclView`

**Files:**
- Modify: `src/lsp/snapshot.rs` (new `DeclView`; `decl_and_text`; delete `dep_decl_by_id` field + `DepDeclById` alias; `build_dep_indexes` → `build_dep_texts`)
- Modify: `src/lsp/handlers.rs` (`build_item`/`decl_uri` retyped to `DeclView`; call sites)
- Modify: `src/lsp/updater.rs` (delete the two `dep_decl_by_id:` forwards, lines ~523 and ~703; doc comments)
- Modify: `src/main.rs:140` (`dep_decl_by_id.len()` → `dep_meta.len()`)
- Test: `tests/lsp_incremental_parity.rs` (retarget 31 `dep_decl_by_id` mentions; new `canon_dep_meta` helper; one new key-set test)
- Modify: `CHANGELOG.md`

**Interfaces:**
- Consumes: `RoutineMeta` (src/program/resolve/decl_surface.rs:30 — fields `name: String`, `origin: Origin`, `name_origin: Origin`, `virtual_path: String`, plus resolver-only fields), `LspSnapshot.dep_meta: Arc<DepMetaMap>` (`DepMetaMap = HashMap<RoutineNodeId, RoutineMeta>`), `LspSnapshot.dep_texts: Arc<DepTexts>`.
- Produces: `pub struct DeclView<'a> { pub id: &'a RoutineNodeId, pub name: &'a str, pub origin: &'a al_syntax::ir::Origin, pub name_origin: &'a al_syntax::ir::Origin, pub virtual_path: &'a str }` with `DeclView::from_entry(&'a DeclEntry) -> DeclView<'a>`; `LspSnapshot::decl_and_text(&self, id: &RoutineNodeId) -> Option<(DeclView<'_>, &str)>`. `DeclEntry` itself REMAINS (workspace `decls_by_file`/`decl_by_id` still use it).

**Why this is safe (verified by two independent reviews):** `build_dep_indexes` (snapshot.rs:675-716) built every dep `DeclEntry` FROM `surface.get_with_path(&node.id)` — i.e. from the very `RoutineMeta` map that is published as `dep_meta`. Name (raw casing), both `Origin`s, and `virtual_path` are byte-identical by construction. `decl_and_text` (snapshot.rs:480) is the ONLY production content-read; `main.rs:140` reads only `.len()` (count identical — key-set parity verified); updater lines 523/703 are pure `Arc::clone` forwards that simply disappear with the field.

- [ ] **Step 1: Write the new parity-test helper and key-set test (they will fail to compile / fail — that's the TDD gate)**

In `tests/lsp_incremental_parity.rs`, next to `canon_dep_decl_by_id` (line ~297):

```rust
/// `LspSnapshot::dep_meta`, canonicalized to the same `CanonDecl` shape
/// `canon_decl` projects a workspace `DeclEntry` to — replaces
/// `canon_dep_decl_by_id` (the `dep_decl_by_id` map was deleted; `dep_meta`
/// is the same data, built from the same frozen `RoutineMeta` source).
fn canon_dep_meta(snap: &LspSnapshot) -> BTreeMap<RoutineNodeId, CanonDecl> {
    snap.dep_meta
        .iter()
        .map(|(k, v)| {
            (
                k.clone(),
                (
                    k.clone(),
                    v.name.clone(),
                    canon_origin(&v.origin),
                    canon_origin(&v.name_origin),
                ),
            )
        })
        .collect()
}
```

And a NEW test (in the same file, near the other dep-fixture tests ~line 1339) pinning that every id an edge can name resolves — the fail-closed contract survives the map deletion:

```rust
/// Every `RouteTarget::Routine(id)` naming a DEPENDENCY routine must resolve
/// through `decl_and_text` (served by `dep_meta` since the `dep_decl_by_id`
/// deletion) — the fail-closed "never guess" contract must not lose a single
/// id in the migration.
#[test]
fn every_dep_routine_route_target_resolves_via_dep_meta() {
    let dir = copy_fixture_lsp_diff_deps_to_tempdir();
    let snap = LspSnapshot::build_full(dir.path()).expect("build_full");
    let workspace_app = snap.graph.apps.find(&snap.snap.workspace_app);
    let mut dep_targets = 0usize;
    for edges in snap
        .edges_by_file
        .values()
        .map(|a| a.as_slice())
        .chain(std::iter::once(snap.event_edges.as_slice()))
    {
        for ce in edges {
            for route in &ce.edge.routes {
                if let RouteTarget::Routine(rid) = &route.target {
                    if Some(rid.object.app) != workspace_app {
                        dep_targets += 1;
                        assert!(
                            snap.decl_and_text(rid).is_some(),
                            "dep routine target {rid:?} must resolve via dep_meta"
                        );
                    }
                }
            }
        }
    }
    assert!(
        dep_targets > 0,
        "fixture sanity: at least one dependency-routine route target must exist"
    );
}
```

(Adjust the fixture-helper name to whatever this file's existing dep-fixture tests at ~line 1339-1400 use — grep `fn.*fixture.*dep` in the file; reuse THEIR helper. Add `RouteTarget` to the test file's imports if missing.)

Run: `cargo test --test lsp_incremental_parity every_dep_routine_route_target_resolves`
Expected: PASS already (it must pass BEFORE and AFTER — it is the migration's safety net, not a red test; the red gate for this task is the compile break in step 2).

- [ ] **Step 2: Add `DeclView`, retype `decl_and_text`, delete the field**

In `src/lsp/snapshot.rs`:

(a) Add below `DeclEntry`:

```rust
/// A borrowed, source-agnostic view of one routine declaration's LSP-facing
/// data — the common shape of a workspace [`DeclEntry`] and a dependency
/// [`RoutineMeta`] (`dep_meta` tier), so [`LspSnapshot::decl_and_text`] can
/// serve BOTH without materializing a second owned map for dependencies
/// (the old `dep_decl_by_id` duplicated ~103 MB of `dep_meta`'s data on a
/// CDO-scale workspace, plus an O(all-dep-decls) build pass at every rung-3).
#[derive(Clone, Copy, Debug)]
pub struct DeclView<'a> {
    pub id: &'a RoutineNodeId,
    /// Raw casing, for display (`RoutineNodeId::name_lc` is lowercased).
    pub name: &'a str,
    /// Whole declaration span (`CallHierarchyItem.range`).
    pub origin: &'a al_syntax::ir::Origin,
    /// Name-token span (`CallHierarchyItem.selectionRange`).
    pub name_origin: &'a al_syntax::ir::Origin,
    pub virtual_path: &'a str,
}

impl<'a> DeclView<'a> {
    #[must_use]
    pub fn from_entry(e: &'a DeclEntry) -> Self {
        DeclView {
            id: &e.id,
            name: &e.name,
            origin: &e.origin,
            name_origin: &e.name_origin,
            virtual_path: &e.virtual_path,
        }
    }
}
```

Import `RoutineMeta` where snapshot.rs already imports `DepMetaMap`/`DeclSurface` from `crate::program::resolve::decl_surface`.

(b) Replace `decl_and_text` (keep its doc comment, updating the `dep_decl_by_id` mention to `dep_meta`):

```rust
    #[must_use]
    pub fn decl_and_text(&self, id: &RoutineNodeId) -> Option<(DeclView<'_>, &str)> {
        if let Some(d) = self.decl_by_id.get(id) {
            let text: &str = &self.parsed.get(&d.virtual_path)?.text;
            return Some((DeclView::from_entry(d), text));
        }
        let (key, m) = self.dep_meta.get_key_value(id)?;
        let text = self.dep_texts.get(&(id.object.app, m.virtual_path.clone()))?;
        Some((
            DeclView {
                id: key,
                name: &m.name,
                origin: &m.origin,
                name_origin: &m.name_origin,
                virtual_path: &m.virtual_path,
            },
            text.as_ref(),
        ))
    }
```

(`get_key_value` is used so the returned `id` borrows the MAP's key, not the caller's transient argument.)

(c) Delete the `pub dep_decl_by_id: Arc<DepDeclById>` field and the `pub(crate) type DepDeclById = ...` alias. Update `dep_texts`'s doc comment ("every file contributing an entry to `dep_decl_by_id`" → "`dep_meta`") and `decl_by_id`'s dep-counterpart doc (the long paragraph at the `dep_decl_by_id` field — move its still-true content, e.g. the `make_routine_route` guarantee, onto `dep_meta`'s doc).

(d) Rename `build_dep_indexes` → `build_dep_texts`, deleting the decl loop (only the `dep_texts` loop over `parsed` remains) and the now-unused `surface` parameter:

```rust
#[must_use]
pub(crate) fn build_dep_texts(
    graph: &ProgramGraph,
    parsed: &[ParsedUnit],
    primary_app: AppRef,
) -> DepTexts {
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
    dep_texts
}
```

Keep the function's doc, rewritten for the narrower job (text source for dependency-decl position conversion; called once per rung-3). Fix its call site in `from_context` (drop the tuple destructure and the `surface` argument; delete the `dep_decl_by_id: Arc::new(...)` field init).

- [ ] **Step 3: Chase the compiler through the remaining call sites**

Run: `cargo build --all-targets` and fix every error — this is the compile-enforced audit:
- `src/lsp/handlers.rs`: retype `build_item(..., decl: &DeclEntry, ...)` → `decl: DeclView<'_>` and `fn decl_uri(snap, decl: &DeclEntry)` → `decl: DeclView<'_>` (fields read identically: `decl.id.object.app`, `decl.name`, `decl.origin`, `decl.name_origin`, `decl.virtual_path`; `decl.id.clone()` in `build_item`'s `ItemData` still works — `id` is `&RoutineNodeId`). Call sites that hold a `&DeclEntry` (e.g. `prepare`'s workspace path via `decl_at_position`) wrap with `DeclView::from_entry(decl)`. `incoming`/`push_route_items` already receive the view from `decl_and_text` — pass it through. Update the module-doc/`push_route_items` comment mentions of `dep_decl_by_id` to `dep_meta`.
- `src/lsp/updater.rs`: delete the `dep_decl_by_id: Arc::clone(&cur.dep_decl_by_id),` lines (~523 and ~703) and update the adjacent comment blocks (`dep_decl_by_id`/`dep_texts`/`dep_meta` → `dep_texts`/`dep_meta`) plus the module-doc mention at line ~63.
- `src/main.rs:140`: `let dep_definitions = snap.dep_meta.len();`
- Anything else the compiler finds (e.g. `benches/`, in-file tests constructing `LspSnapshot` literally).

Expected end state: `grep -rn dep_decl_by_id src benches` returns ZERO hits; `cargo build --all-targets` clean.

- [ ] **Step 4: Retarget the parity tests**

In `tests/lsp_incremental_parity.rs`, using the enumeration from the investigation (all 31 mentions):
- Delete `canon_dep_decl_by_id` (line ~297); `canon_dep_meta` from Step 1 replaces it.
- Content asserts → `canon_dep_meta`: lines ~408-410, ~1389, ~1429-1431, ~1474-1476 (same assertion shape, messages updated to say `dep_meta`).
- Non-vacuous checks: ~1382 (`!base.dep_meta.is_empty()`), ~1470 (`!rung2_snap.dep_meta.is_empty()`).
- Membership checks: ~1758, ~1796 → `.dep_meta.contains_key(...)`.
- Arc-identity asserts: ~1616-1617, ~1654-1655, ~1674+1705-1706 → `Arc::ptr_eq(&....dep_meta, &....dep_meta)` (dep_meta identity is ALREADY asserted alongside in this file — if a given assert block already has a dep_meta twin, just delete the dep_decl_by_id line rather than duplicating).
- Doc comments: ~121, 126, 404, 1339, 1349, 1354, 1437 — update prose.

Run: `cargo test --test lsp_incremental_parity`
Expected: ALL PASS (16+ tests, including Step 1's new test and the retargeted content asserts).

- [ ] **Step 5: Full validation**

Run: `cargo test` (full suite), `cargo clippy --all-targets --all-features`, and `cargo test --release --test perf_bounds` (long build; allow 10+ min).
Expected: all green, clippy clean, perf_bounds 9/9. Zero golden changes (`git status` shows no `tests/fixtures`/golden diffs).

- [ ] **Step 6: rustfmt, CHANGELOG, commit**

`rustfmt` each touched file individually: `src/lsp/snapshot.rs`, `src/lsp/handlers.rs`, `src/lsp/updater.rs`, `src/main.rs`, `tests/lsp_incremental_parity.rs`.
CHANGELOG.md under `[Unreleased] > Changed`:
```markdown
- Deleted `LspSnapshot.dep_decl_by_id` — dependency decl lookups
  (`decl_and_text`) are now served directly from the `dep_meta` frozen tier
  via a borrowed `DeclView`, removing a fully redundant ~126k-entry map
  (~103 MB steady-state RSS on a CDO-scale workspace) and the O(all-dep-decls)
  `build_dep_indexes` decl pass (~150-200 ms of every cold start / rung-3
  rebuild). LSP responses are byte-identical (both maps were built from the
  same frozen `RoutineMeta` source).
```

```bash
git add src/lsp/snapshot.rs src/lsp/handlers.rs src/lsp/updater.rs src/main.rs tests/lsp_incremental_parity.rs CHANGELOG.md
git commit -m "perf: delete dep_decl_by_id, serve dependency decls from the dep_meta frozen tier

Co-authored-by: Copilot <223556219+Copilot@users.noreply.github.com>"
```

---

### Task 3: Rung-1 scoped-context extraction + honest bench/gate (F6)

**Files:**
- Modify: `src/lsp/updater.rs` (new `pub struct Rung1Context`; `Updater::apply_batch_scoped`; refactor `spawn_updater`'s hot loop onto them)
- Modify: `benches/lsp_pipeline.rs` (new `rung1_body_edit_scoped_1000_files` bench)
- Modify: `tests/perf_bounds.rs` (new scoped-path bound)
- Modify: `CHANGELOG.md`

**Interfaces:**
- Consumes: `apply_rung1_core` (updater.rs:628, private — stays private), `ResolveIndex::build(&ProgramGraph) -> ResolveIndex` (owned, no lifetime), `DeclSurface::build(..).with_frozen(..)` (owned), `Updater::classify` (private — stays private; `apply_batch_scoped` wraps it).
- Produces: `pub struct Rung1Context<'g>` with `pub fn build(cur: &'g LspSnapshot, workspace: &ParsedUnit) -> Rung1Context<'g>`; `pub fn Updater::apply_batch_scoped(&mut self, cur: &LspSnapshot, batch: &[ChangeEvent], ctx: &Rung1Context<'_>) -> Option<LspSnapshot>` (Some = the batch classified rung-1 and was applied; None = Noop or would escalate — the caller falls back to `apply_batch`).

**Background:** the bench + release gate drive `Updater::apply_batch`, which rebuilds `ResolveIndex` + `DeclSurface` + `obj_node_map` on EVERY call (updater.rs:264-303). Production (`spawn_updater`, updater.rs:876-940) builds that context ONCE per published graph and reuses it across consecutive rung-1 batches — so the gate currently guards a scenario the server never runs, and a real regression in the production path could hide behind context-rebuild noise. Fix: ONE code path — extract the context into a type used by both production and the new bench.

- [ ] **Step 1: Extract `Rung1Context` and `apply_batch_scoped`**

In `src/lsp/updater.rs` (above `spawn_updater`):

```rust
/// The rung-1 scoped context [`spawn_updater`]'s outer loop builds ONCE per
/// published graph and reuses across every consecutive rung-1 batch (see the
/// module doc's "scoped-context loop"). Public so the bench/perf gate can
/// measure the EXACT production path (previously they could only reach
/// [`Updater::apply_batch`], which rebuilds this context per call — a
/// worst-case the live server never pays per keystroke).
pub struct Rung1Context<'g> {
    index: ResolveIndex,
    surface: DeclSurface,
    obj_node_map: HashMap<ObjectNodeId, &'g ObjectNode>,
}

impl<'g> Rung1Context<'g> {
    /// Build from the currently published snapshot + the updater's current
    /// workspace unit — exactly the construction the hot loop performs at
    /// the top of each outer iteration. `index`/`surface` are fully owned;
    /// only `obj_node_map` borrows `cur.graph` (hence the lifetime).
    #[must_use]
    pub fn build(cur: &'g LspSnapshot, workspace: &ParsedUnit) -> Self {
        Rung1Context {
            index: ResolveIndex::build(&cur.graph),
            surface: DeclSurface::build(&cur.graph, std::slice::from_ref(workspace))
                .with_frozen(Arc::clone(&cur.dep_meta)),
            obj_node_map: cur
                .graph
                .objects
                .iter()
                .map(|o| (o.id.clone(), o))
                .collect(),
        }
    }
}
```

And on `impl Updater`:

```rust
    /// Classify `batch` and, if (and only if) it lands on rung 1, apply it
    /// against the prebuilt `ctx` — the EXACT call [`spawn_updater`]'s inner
    /// loop makes. Returns `None` for a `Noop` batch or one that would
    /// escalate to rung 2/3 (the caller must then take the
    /// [`Self::apply_batch`] path, whose context `ctx` — built against the
    /// OLD graph — would be stale for).
    pub fn apply_batch_scoped(
        &mut self,
        cur: &LspSnapshot,
        batch: &[ChangeEvent],
        ctx: &Rung1Context<'_>,
    ) -> Option<LspSnapshot> {
        match self.classify(cur, batch) {
            Decision::Rung1(saves) => Some(apply_rung1_core(
                cur,
                saves,
                &ctx.index,
                &ctx.surface,
                &ctx.obj_node_map,
                &mut self.pending,
            )),
            _ => None,
        }
    }
```

- [ ] **Step 2: Refactor `spawn_updater`'s hot loop onto the new type**

Replace the inline `let index = ...; let surface = ...; let obj_node_map = ...;` block with `let ctx = Rung1Context::build(&cur, &updater.workspace);` and the `Decision::Rung1(saves) => apply_rung1_core(&inner_cur, saves, &index, &surface, &obj_node_map, &mut updater.pending)` arm with the equivalent through `ctx` (`apply_rung1_core(&inner_cur, saves, &ctx.index, &ctx.surface, &ctx.obj_node_map, &mut updater.pending)` — the classify/apply split in the inner loop stays as-is, do NOT route the inner loop through `apply_batch_scoped` (it must keep its explicit `Decision` match for the escalation break)). Keep the block-scoped drop pattern and its comment (updating variable names). The in-file test `apply_rung1_core_reuses_the_same_context_across_two_consecutive_edits` (~line 1613) should also construct via `Rung1Context::build` where it hand-rolled the same three values — same for the test at ~1968 if applicable.

Run: `cargo test --lib updater && cargo test --test lsp_incremental_parity`
Expected: PASS — including the hot-loop Arc-identity parity test (proves the refactored production path is behaviorally unchanged).

- [ ] **Step 3: Add the scoped bench**

In `benches/lsp_pipeline.rs`, next to `bench_rung1_body_edit`:

```rust
/// The PRODUCTION rung-1 path: context built once (like `spawn_updater`'s
/// scoped-context loop), then reused across every iteration — measures what
/// a user's keystroke-save actually costs, unlike `rung1_body_edit_1000_files`
/// (kept as the worst-case: context rebuilt per call). See the 2026-07-14
/// improvement-hunt F6 finding.
fn bench_rung1_body_edit_scoped(c: &mut Criterion) {
    let dir = corpus_dir(1000);
    let (base, parsed) =
        LspSnapshot::build_full_with_parsed(dir.path()).expect("build_full_with_parsed");
    let mut updater = Updater::new(dir.path().to_path_buf(), parsed);
    let target = dir.path().join(perf_support::file_name(1));
    perf_support::body_only_comment_edit(dir.path(), 1000, 1);
    let batch = vec![ChangeEvent::FileSaved(target)];

    let ctx = Rung1Context::build(&base, updater.workspace());
    let warm = updater
        .apply_batch_scoped(&base, &batch, &ctx)
        .expect("a comment-only body edit must stay rung 1");
    let mut cur = warm;

    c.bench_function("rung1_body_edit_scoped_1000_files", |b| {
        b.iter(|| {
            let next = updater
                .apply_batch_scoped(&cur, black_box(&batch), &ctx)
                .expect("must stay rung 1");
            cur = next;
            black_box(&cur);
        });
    });
}
```

IMPORTANT soundness note for the implementer: `ctx` borrows `base.graph`'s `Arc` contents, and rung-1 snapshots `Arc::clone` the SAME graph forward — `base` must stay alive for the bench's whole body (it does, as a local), exactly mirroring the hot loop's `cur` binding. `updater.workspace()` requires a tiny accessor if `workspace` is private: add `pub fn workspace(&self) -> &ParsedUnit { &self.workspace }` on `Updater` (or make `Rung1Context::build` take `&Updater` — pick whichever is cleaner; keep the field private). Register the new bench in the existing `criterion_group!`.

- [ ] **Step 4: Add the release-gate bound**

In `tests/perf_bounds.rs`, mirror `rung1_body_edit_apply_batch_within_bound` (~line 637) as `rung1_body_edit_scoped_within_bound`, driving `apply_batch_scoped` with a once-built `Rung1Context` exactly as the bench does. Bounds: reuse `RUNG1_BOUND` for the absolute ceiling and add
```rust
    const RUNG1_SCOPED_SYNTHETIC_BOUND: Duration = Duration::from_millis(75); // 5x measured scoped baseline — tighten after first measurement
```
with a doc comment following `RUNG1_SYNTHETIC_BOUND`'s pattern; after first release-mode run, set it to ~5x the measured median (record the measured number in the constant's doc).

Run: `cargo test --release --test perf_bounds`
Expected: 10/10 PASS (the 9 existing + the new scoped bound).

- [ ] **Step 5: Full validation, CHANGELOG, commit**

Run: `cargo test` (full), `cargo clippy --all-targets --all-features`, `cargo bench --bench lsp_pipeline -- rung1` (capture both rung-1 medians — scoped should be measurably below the apply_batch worst-case; record both in the commit body).
`rustfmt src/lsp/updater.rs benches/lsp_pipeline.rs tests/perf_bounds.rs` (each individually).
CHANGELOG.md under `[Unreleased] > Added`/`Changed`:
```markdown
- The rung-1 bench + release perf gate now also measure the PRODUCTION
  scoped-context path (`Rung1Context` + `Updater::apply_batch_scoped`,
  extracted from `spawn_updater`'s hot loop so bench and server share one
  code path); the old `apply_batch` bench remains as the worst-case.
```

```bash
git add src/lsp/updater.rs benches/lsp_pipeline.rs tests/perf_bounds.rs CHANGELOG.md
git commit -m "perf(bench): gate the production rung-1 scoped-context path (extract Rung1Context)

Co-authored-by: Copilot <223556219+Copilot@users.noreply.github.com>"
```

---

### Task 4: Measurement + docs close-out

**Files:**
- Modify: `docs/perf-regression-t3-vs-0.9.3.md` (append §10)
- Modify: `CHANGELOG.md` (only if numbers materially differ from Task 2's entry — otherwise no change)

**Interfaces:**
- Consumes: Task 1's bench before/after files (`.superpowers/sdd/tier1-quick-wins/task-1-bench-*.txt`), Task 3's two rung-1 medians, the §8/§9 methodology in the perf doc.

- [ ] **Step 1: Measure LSP steady-state RSS + cold start (AFTER only)**

Workspace: `U:\Git\DO.Support-SlowDOSetup\DocumentOutput\Cloud`. Methodology: EXACTLY §8's (scratch raw-stdio LSP client, documented in `.superpowers/sdd/owned-decl-surface/task-4-report.md` §4 — spawn release binary, `initialize` (percent-encoded rootUri), `initialized`, `didOpen` one workspace file, `prepareCallHierarchy`, RSS via psutil at first response and +30s; ≥4 fresh-process trials, exclude the disk-cold first, report the median of the rest). BEFORE baselines (do NOT re-measure): steady-state RSS ~726-750 MB, cold start ~2.82 s (perf doc §8, measured at the cold-start fix). Expected AFTER: RSS down ~100 MB (the dep_decl_by_id deletion), cold start down ~150-200 ms. Delete the scratch driver afterwards — never commit it.

- [ ] **Step 2: Append §10 to `docs/perf-regression-t3-vs-0.9.3.md`**

Following §6/§8's close-out format: a before/after table (RSS, cold start, `incoming` bench median, both rung-1 medians), what changed (the three items, one line each), and honest notes (if RSS lands off the ~100 MB prediction, say so and why — the attribution report predicts 102.8 MB of net heap, but allocator slack means RSS moves less than heap).

- [ ] **Step 3: Validate + commit**

Docs-only commit (no tests needed beyond confirming `git status` shows only the two intended files):

```bash
git add docs/perf-regression-t3-vs-0.9.3.md CHANGELOG.md
git commit -m "docs: Tier-1 quick-wins close-out (measured RSS/cold-start/incoming/rung-1)

Co-authored-by: Copilot <223556219+Copilot@users.noreply.github.com>"
```

(If CHANGELOG.md was not touched, stage only the perf doc.)

---

## Post-plan (not tasks): final whole-branch review (errors+performance focus), then CDO gate (`scripts/cdo-gate` semantics: `cargo test --release --test program_resolve_harness -- --test-threads=1` then `--test program_graph --test snapshot_robustness`, with `CDO_WS=u:\Git\DO.Support-SlowDOSetup\DocumentOutput\Cloud` and `ENFORCE_CDO_WS=1`) before any merge to master — merge only on explicit user request.
