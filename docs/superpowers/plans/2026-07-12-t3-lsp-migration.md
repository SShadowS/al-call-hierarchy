# T3 — LSP Migration onto the Program Engine: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`) syntax.

> Fifth deep-review-remediation arc (Tier 3, the last tier). Spec:
> `docs/superpowers/specs/2026-07-12-t3-lsp-migration-design.md` (user-approved 2026-07-12) — READ IT FIRST;
> this plan implements it task-by-task and repeats its binding decisions inline where a task needs them.

**Goal:** Migrate the LSP surface onto `src/program/resolve/` + the al-syntax IR with true per-file
incremental updates, fix H-12 (position encoding) and H-13 (URI encoding), prove same-or-better via an
adjudicated differential harness, then DELETE the legacy `graph.rs`/`indexer.rs`/`parser.rs` pipeline.

**Architecture:** Immutable `LspSnapshot` published by atomic Arc swap (queries sub-ms, swap-only, no
in-place mutation — H-10 class structurally impossible). Two-rung incremental ladder: rung 1 (body-only
edit, definition-surface fingerprint unchanged) re-resolves only the edited file's obligations; rung 2
(surface changed) rebuilds the workspace graph layer over an immutable dep layer; rung 3 (.alpackages
change / watcher overflow) full rebuild. Permanent incremental-vs-batch differential gate.

**Tech Stack:** Rust; existing deps only (`percent-encoding`, `blake3`, `rayon`, `notify`, `arc-swap` is
NOT a dep — use `RwLock<Arc<LspSnapshot>>`). Engine changes strictly additive.

## Global Constraints (every task inherits these)

- **Frozen CDO SHA:** `aldump --program-call-graph-stats U:/Git/DO.Support-SlowDOSetup/DocumentOutput/Cloud`
  output SHA-256 must equal `0a3b85bc832ff0a3e77acee118d203edbf62827dc37617c8d9315fe52d5cb7d0` after every
  engine-touching task (Tasks 5, 6 and the capstone; other tasks run it only if they touched `src/program/`
  or `src/snapshot/`). Command:
  `./target/release/aldump.exe --program-call-graph-stats U:/Git/DO.Support-SlowDOSetup/DocumentOutput/Cloud | sha256sum`
- **Fail closed:** an unprovable edge/answer is Unknown/absent, never guessed.
- Clippy bar: `cargo clippy --all-targets --all-features -- -D warnings` clean.
- Format per-file `rustfmt <file>` (NEVER `cargo fmt`); stage only named paths (NEVER `git add -A`).
- CHANGELOG.md `[Unreleased]` entry per task. Goldens regen ONLY via `REGEN_TEMP_GOLDENS=1 cargo test`
  (value-gated); inspect diffs, never auto-bless.
- **Commit-before-gate law:** commit code BEFORE any multi-minute CDO gate; write the task report file
  BEFORE the final status message.
- Worktree lanes: `git worktree add .worktrees/<lane> -b <branch> <base>`; set
  `TREE_SITTER_AL_PATH=U:/Git/al-call-hierarchy/tree-sitter-al` in every worktree. ONE release build at a
  time across lanes (U: disk ~40G/target; clean each worktree's target/ as its lane completes).
- Never push or merge to master without the user's explicit request. Push target is the SShadowS fork only.

## Key verified facts (2026-07-12, master `e147264`)

- Full pipeline on CDO: **~5.3–5.6s** (release, warm dep cache; 551 ws files + 20 deps; 43,375 edges).
- `resolve_full_program(workspace_root: &Path) -> Option<ProgramReport>` — `src/program/resolve/full.rs:831`;
  `build_context` returns `ProgramContext { snap, graph, parsed, primary_app_ref, ws_file_set }` (`full.rs:834-839`).
- `resolve_full_program_from_parts(graph, parsed, primary_app_ref, ws_file_set) -> (Vec<ClassifiedEdge>, Coverage, BuiltinDispatchAudit)`
  (`full.rs:619`). Its Phase 1 is ALREADY a per-file loop (`full.rs:639-752`): per `pf` it builds
  `obj_node_map` lookups, `globals_rec`, calls `extract_sites_for_routine` + `resolve_call_site_obligation`
  per site. Phase 2 (`full.rs:757`) is `emit_event_flow_edges(graph, &index, &body_map)` over ALL apps.
- `ResolveIndex::build(graph)` and `BodyMap::build(graph, parsed)` are built inside `from_parts`
  (`full.rs:629-630`) and BORROW graph/parsed — they cannot live inside a persistent snapshot struct
  (self-reference). Owned derived data only in `LspSnapshot`.
- `build_program_graph(snap: &AppSetSnapshot, abi_cache: &AbiCache) -> ProgramGraph` (`src/program/build.rs:33`)
  internally calls `parse_snapshot(snap)` AGAIN (`build.rs:39`) — the pipeline parses twice today (known
  Phase-0 review minor). Task 5 removes the double-parse as a side effect; SHA gate proves neutrality.
- `ProgramGraph { apps, topology, objects: Vec<ObjectNode>, routines: Vec<RoutineNode>, obj_index, friends }`
  (`src/program/graph.rs:33-41`), vecs sorted by id for determinism.
- `Edge { from: NodeId, site: SiteId, kind, shape, completeness, routes: Vec<Route> }` (`edge.rs:446-453`);
  `SiteId { caller, span: CanonicalSpan, callee_fingerprint }` (`edge.rs:85-89`);
  `CanonicalSpan { unit: String, start/end: SourcePos { line: u32, col: u32 } }` (`edge.rs:69-81`) —
  0-based line, 0-based **UTF-8 byte** column;
  `RouteTarget::{Routine(NodeId), Builtin(BuiltinId), AbiSymbol{key}}` (`edge.rs:376-382`);
  `Witness::{SourceSpan{file, span:(u32,u32)}, AbiSymbol{key}, None}` (`edge.rs:389-`); `NodeId = RoutineNodeId`.
- `RoutineNodeId` is content-addressed (`src/program/node.rs:132`): `object(AppRef+ObjectKind+ObjKey) +
  name_lc + enclosing_member_lc + params_count + sig_fp` — stable across rebuilds for unchanged routines
  (apps sorted by AppId ⇒ stable `AppRef`). `sig_fp` stable only within one engine build (fine: item.data
  round-trips within one server session only).
- `RoutineDecl` has `origin` (whole decl) AND `name_origin` (`crates/al-syntax/src/ir/decl.rs:133-136`,
  doc: LSP selection_range), both `Origin { byte, start: Point, end: Point }`.
- Legacy LSP: dispatch `src/handlers.rs:23-120`; server loop single-threaded (`server.rs:245-280`);
  `didSave`-only sync (`text_document_sync.change=NONE`, `server.rs:59`); diagnostics push-once
  (`server.rs:119,283`); watcher loop `server.rs:213-240` (no debounce; didSave+watcher double-reindex);
  H-10 at `graph.rs:762-832`; H-11 at `graph.rs:443-445`; H-13 at `protocol.rs:52-85`.
- `tests/perf_bounds.rs` pins legacy handler signatures + `Arc<RwLock<Indexer>>` + semantics (999 fan-in);
  rewritten in Task 16. `benches/lsp_pipeline.rs` same surface. Corpus `tests/perf_support/`.
- Issue-#20 unused-procedure rules live at `indexer.rs:159-218` + `graph.rs:888-905`, heavily test-pinned.
- All four H-bugs are LATENT in existing tests (ASCII-only corpora/paths) — new coverage is mandatory.

## Execution shape (SDD waves)

- **Wave A (4 parallel lanes):** Task 1 (H-13), Task 2 (H-12 infra), Task 3 (stage-split measurement),
  Task 4 (resolver-read audit).
- **Wave B (one lane, sequential — both edit `full.rs`/`build.rs`):** Task 5 → Task 6.
- **Wave C:** Task 7 (needs 4), Task 8 (needs 5+6) — 2 lanes.
- **Wave D:** Task 9 (needs 7+8) → Task 10 (needs 9) — one lane.
- **Wave E (3 parallel lanes):** Tasks 11, 12, 13 (all consume Task 8's snapshot API read-only).
- **Wave F:** Task 14 (needs 11+12+13). **Wave G:** Task 15 (cutover; needs 2+9+14) → Task 16.
- **Wave H:** Task 17 (capstone deletion + docs).
- Sonnet implementers + task reviewers (refute-by-default), Opus whole-branch review before the merge menu.

---

### Task 0: Branch + frozen baseline artifact

**Files:**
- Create: `.superpowers/sdd/t3-baseline.md`

- [ ] **Step 1:** `git checkout -b feat/t3-lsp-migration e147264` (or current master tip — record it).
- [ ] **Step 2:** Build release (`cargo build --release`), run the CDO stats command from Global
  Constraints, record: engine commit SHA, output SHA-256 (must be `0a3b85bc…`), wall-clock, and
  `cargo test --release -q 2>&1 | tail -5` summary into `.superpowers/sdd/t3-baseline.md`.
- [ ] **Step 3:** Commit: `git add .superpowers/sdd/t3-baseline.md && git commit -m "chore(t3): freeze arc baseline"`.

---

### Task 1: H-13 — URI encoding on `percent-encoding` (both-pipeline fix, lands first)

**Files:**
- Modify: `src/protocol.rs` (path_to_uri, `:52-85`)
- Test: `src/protocol.rs` `#[cfg(test)]` block

**Interfaces:**
- Produces: `pub fn path_to_uri(path: &Path) -> Uri` (lsp_types::Uri — the ACTUAL live signature;
  this plan's original `Option<String>` guess was corrected during Task 1) — SAME signature as
  today; encoding now RFC-3986-correct for arbitrary paths. Case-preserving, colon-literal drive
  convention (not lowercase-drive as this plan originally assumed).

- [ ] **Step 1: Failing tests first.** Add round-trip tests (they must FAIL against the current
  hand-encoder):

```rust
#[test]
fn uri_roundtrip_non_ascii_path() {
    // H-13: Løsninger previously produced file:///unknown via fluent-uri rejection.
    for p in [
        r"C:\Løsninger\App\Fil æøå.al",
        r"C:\repo\100%\a#b\c+d @e\f.al",
        r"C:\repo\emoji 🚀\file.al",
    ] {
        let uri = path_to_uri(Path::new(p)).expect("must encode");
        assert!(uri.starts_with("file:///c%3A/") || uri.starts_with("file:///c:/"), "{uri}");
        let back = uri_to_path(&uri).expect("must decode");
        assert_eq!(back, normalize_path(Path::new(p)), "roundtrip {p}");
    }
}
```

  (Adjust the drive-prefix assertion to the existing lowercase-drive convention at `protocol.rs:48-51` —
  read it first; the ROUNDTRIP equality is the real assertion.)
- [ ] **Step 2:** `cargo test -p al-call-hierarchy protocol` → expected: new tests FAIL (broken URIs).
- [ ] **Step 3: Implement.** Replace the hardcoded 5-char encoder with `percent_encoding` (already in
  `Cargo.toml` — verify with `grep percent Cargo.toml`). Define a path-segment set:

```rust
use percent_encoding::{utf8_percent_encode, AsciiSet, CONTROLS};
/// RFC 3986 pchar-complement for path segments, plus '%' itself.
const PATH_SEGMENT: &AsciiSet = &CONTROLS
    .add(b' ').add(b'"').add(b'#').add(b'%').add(b'<').add(b'>').add(b'?')
    .add(b'`').add(b'{').add(b'}').add(b'[').add(b']').add(b'^').add(b'|').add(b'\\');
```

  Encode each path segment with `utf8_percent_encode(segment, PATH_SEGMENT)`; keep the existing Windows
  drive-letter normalization and forward-slash join EXACTLY as today (read the current function; only the
  per-segment escaping changes). `uri_to_path` stays as-is (it already percent-decodes everything).
- [ ] **Step 4:** `cargo test -p al-call-hierarchy protocol` → PASS. `rustfmt src/protocol.rs`.
- [ ] **Step 5:** Clippy bar; CHANGELOG (`Fixed`: H-13). Commit:
  `git add src/protocol.rs CHANGELOG.md && git commit -m "fix(protocol): percent-encode URIs correctly (H-13)"`.

---

### Task 2: H-12 infrastructure — position-encoding module + negotiation

**Files:**
- Create: `src/lsp/mod.rs` (module root: `pub mod encoding;`), `src/lsp/encoding.rs`
- Modify: `src/lib.rs` (add `pub mod lsp;`), `src/server.rs` (initialize: negotiate + advertise)
- Test: `src/lsp/encoding.rs` `#[cfg(test)]`

**Interfaces:**
- Produces:

```rust
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PositionEncoding { Utf8, Utf16 }

/// Pick from the client's `general.positionEncodings` capability value.
pub fn negotiate(client_encodings: Option<&[String]>) -> PositionEncoding; // utf-8 iff offered, else utf-16

/// Lazy per-line conversion table for ONE file's text.
pub struct LineTable<'t> { /* text + lazily computed per-line data */ }
impl<'t> LineTable<'t> {
    pub fn new(text: &'t str) -> Self;
    /// UTF-8 byte col (engine-native) → column in `enc` for LSP output.
    pub fn col_out(&self, line: u32, byte_col: u32, enc: PositionEncoding) -> u32;
    /// Inbound LSP column in `enc` → UTF-8 byte col for engine lookups.
    pub fn col_in(&self, line: u32, enc_col: u32, enc: PositionEncoding) -> u32;
}
```

- [ ] **Step 1: Failing tests.** `æøå` (2 UTF-8 bytes, 1 UTF-16 unit each), `🚀` (4 bytes, 2 units),
  ASCII passthrough, out-of-range clamps (clamp to line end — fail-closed, never panic):

```rust
#[test]
fn utf16_conversion_danish_and_emoji() {
    let t = LineTable::new("æøå x\n🚀 y\nplain\n");
    assert_eq!(t.col_out(0, 6, PositionEncoding::Utf16), 3);  // after æøå = 6 bytes = 3 UTF-16 units
    assert_eq!(t.col_in(0, 3, PositionEncoding::Utf16), 6);
    assert_eq!(t.col_out(1, 4, PositionEncoding::Utf16), 2);  // after 🚀 = 4 bytes = 2 units (surrogate pair)
    assert_eq!(t.col_out(2, 3, PositionEncoding::Utf8), 3);   // utf-8 mode: identity
    assert_eq!(t.col_out(0, 999, PositionEncoding::Utf16), 5); // clamp to line end (5 chars → 5 units)
}
```

- [ ] **Step 2:** Run → FAIL (module absent). Implement: split `text` into line spans once; per-line
  conversion walks `char_indices()` accumulating `c.len_utf16()`; memoize nothing fancier (lines are
  short; measure only if profiling ever says otherwise). `Utf8` mode is identity + clamp.
- [ ] **Step 3:** `negotiate`: return `Utf8` iff the slice contains `"utf-8"`, else `Utf16`. In
  `server.rs` initialize: read `params.capabilities.general.position_encodings` (serde path — check the
  incoming JSON shape against the LSP 3.17 spec field `general.positionEncodings`), store the negotiated
  value in the server state, and advertise it in `ServerCapabilities.position_encoding`
  (`"utf-8"`/`"utf-16"`). Legacy handlers keep serving byte columns THIS task — for a utf-8 client that
  is now suddenly CORRECT; for utf-16 clients behavior is unchanged-broken until Task 15 wires
  conversion. State that exact sentence in the CHANGELOG entry (honesty).
- [ ] **Step 4:** Tests pass; clippy; rustfmt touched files; CHANGELOG (`Added`). Commit
  `feat(lsp): position-encoding negotiation + LineTable converter (H-12 infra)`.

---

### Task 3: Stage-split measurement (pins rung budgets)

**Files:**
- Create: `benches/engine_stages.rs` (Criterion, perf corpus), `.superpowers/sdd/t3-stage-split.md`
- Modify: `Cargo.toml` (`[[bench]] name = "engine_stages"`)

**Interfaces:**
- Consumes: `SnapshotBuilder` (`src/snapshot/snapshot.rs:81`), `parse_snapshot` (`parse.rs:80`),
  `build_program_graph` (`build.rs:33`), `resolve_full_program_from_parts` (`full.rs:619`) — if
  `from_parts` is not `pub`, bench through `resolve_full_program` total + the public stages and derive
  resolve time by subtraction; note which method was used.
- Produces: `.superpowers/sdd/t3-stage-split.md` with a table: snapshot / parse / build(graph) /
  ResolveIndex::build / BodyMap::build / resolve — each on (a) the 1000-file perf corpus and (b) CDO
  (release binary, wall-clock, 3 runs, median). Plus the pinned rung-2 target number for Task 16.

- [ ] **Step 1:** Write the bench: one Criterion group per stage over `tests/perf_support/` corpus
  (generate the corpus exactly the way `benches/lsp_pipeline.rs` does — read it and reuse its
  `perf_support` module path trick).
- [ ] **Step 2:** CDO timing: a `#[test]` + `#[ignore]` fn (or a small `--timings` addition NOWHERE —
  do NOT touch aldump; use an ignored test) gated on `CDO_WS`, printing per-stage wall-clock.
  Run: `CDO_WS=U:/Git/DO.Support-SlowDOSetup/DocumentOutput/Cloud cargo test --release stage_split -- --ignored --nocapture`.
- [ ] **Step 3:** Record results + derived budgets in `.superpowers/sdd/t3-stage-split.md`:
  rung-1 budget = parse(1 file) + fingerprint + resolve-single-file + ResolveIndex/BodyMap rebuild (if
  needed) + incoming rebuild — state whether <100ms holds on CDO scale; rung-2 = everything minus
  snapshot minus dep-parse. **If ResolveIndex::build + BodyMap::build together exceed ~30ms on CDO
  scale, write that number in red at the top** — Task 9 has a documented contingency for it.
- [ ] **Step 4:** Clippy, rustfmt, CHANGELOG (`Added`: stage bench). Commit
  `chore(bench): engine stage-split measurement (t3 rung budgets)`.

---

### Task 4: Resolver-read audit — the fingerprint field list (soundness spine)

**Files:**
- Create: `docs/superpowers/specs/2026-07-12-t3-def-surface-audit.md`

**Interfaces:**
- Produces: the AUDITED `DefSurface` field list Task 7 implements verbatim. No code change.

- [ ] **Step 1:** Enumerate every data read reachable from `resolve_call_site_obligation`
  (`full.rs:697-712`) and `emit_event_flow_edges`: walk `src/program/resolve/{resolver,receiver,
  arg_dispatch,extract,builtins,member_catalog}.rs` + `ResolveIndex`/`BodyMap` accessors. For EACH read,
  one table row: source structure, field(s), classification **CALLER-side** (only the site's own file
  feeds it) or **SURFACE-side** (another file's data feeds it), and the evidence (file:line).
- [ ] **Step 2:** Derive the surface list: every SURFACE-side read maps to a fingerprint component.
  Expected classes (verify, don't assume): object identity (kind/id/name), routine signatures
  (name_lc, enclosing_member_lc, params_count, sig_fp, visibility incl. Internal/Protected, return type,
  param types incl. var-ness/subtypes), table fields + types, enum values, extends/implements targets,
  event-publisher attributes (incl. IncludeSender), object properties consulted by resolution
  (SourceTable, TableNo, implements, ControlAddIn members), preproc-conflict/ParseStatus degradations.
- [ ] **Step 3:** Explicitly answer the ONE load-bearing question in a section titled "Does resolution
  ever read another file's routine BODY?": trace every `BodyMap` consumer; classify each as
  signature-read or body-read. If ANY body-read exists, rung 1 is unsound as specced — STOP, report to
  the controller (design amendment needed), do not paper over.
- [ ] **Step 4:** Reviewer instruction (put in the task report): refute-by-default — reviewer must
  independently grep for `index.`/`body_map.`/`graph.` reads in the resolver files and diff their list
  against the audit's. Commit the audit doc.

---

### Task 5: Engine (additive): layered graph + shared-parse plumbing

**Files:**
- Modify: `src/program/build.rs`, `src/program/resolve/full.rs` (build_context only)
- Test: `src/program/build.rs` `#[cfg(test)]` + existing suites

**Interfaces:**
- Produces:

```rust
/// Immutable-between-dep-changes layer: everything derived from NON-primary apps.
pub struct DepLayer {
    pub apps: AppRegistry,            // ALL apps interned (primary included — AppRef stability)
    pub topology: DependencyGraph,
    pub friends: /* same type as ProgramGraph.friends */,
    pub dep_objects: Vec<ObjectNode>, // non-primary only, sorted
    pub dep_routines: Vec<RoutineNode>,
}
pub fn build_dep_layer(snap: &AppSetSnapshot, abi_cache: &AbiCache, parsed: &[ParsedUnit]) -> DepLayer;
/// Merge dep layer + freshly extracted workspace nodes → full ProgramGraph (sort + obj_index rebuild).
pub fn assemble_program_graph(dep: &DepLayer, ws_unit: &ParsedUnit, snap: &AppSetSnapshot) -> ProgramGraph;
/// Existing signature — now a thin wrapper: parse once, build_dep_layer + assemble.
pub fn build_program_graph(snap: &AppSetSnapshot, abi_cache: &AbiCache) -> ProgramGraph;
```

  (Adjust names/fields to what `build.rs` actually needs when read in full — the CONTRACT that may not
  change: `build_program_graph`'s output is byte-identical to today's, and `assemble_program_graph`
  called with an updated workspace `ParsedUnit` reuses dep extraction without re-ingesting ABI.)
- Consumes: `parse_snapshot`, `extract_nodes` internals already in `build.rs`.

- [ ] **Step 1: Characterization test first.** In `build.rs` tests: build a fixture snapshot (reuse an
  existing fixture-workspace test in `src/program/` — grep `SnapshotBuilder` in tests), assert
  `build_program_graph(old path)` output equals `assemble_program_graph(build_dep_layer(...), ws_unit, snap)`
  field-by-field (objects, routines, obj_index len, apps order). Write it calling the NEW functions →
  FAILS (don't exist).
- [ ] **Step 2:** Implement the split. `build_context` (`full.rs:929`) changes to: parse ONCE, pass
  `&parsed` into graph building (kills the `build.rs:39` double-parse). Keep `build_program_graph`'s
  public signature working (wrapper).
- [ ] **Step 3:** `cargo test` full suite green; `REGEN_TEMP_GOLDENS` NOT used (nothing may move).
- [ ] **Step 4:** Commit (before gate). Release build; CDO SHA gate → byte-identical `0a3b85bc…`.
  Record wall-clock delta in the task report (double-parse removal should SHAVE time — a free win; if
  it doesn't, investigate before proceeding).
- [ ] **Step 5:** Clippy, rustfmt, CHANGELOG (`Changed`: single-parse pipeline, additive layered build).
  Amend/commit `feat(program): layered dep/workspace graph assembly + single-parse pipeline (t3.5)`.

---

### Task 6: Engine (additive): single-file resolve entry point

**Files:**
- Modify: `src/program/resolve/full.rs`
- Test: `src/program/resolve/full.rs` `#[cfg(test)]` or the existing resolve test file (grep
  `resolve_full_program_from_parts` callers first)

**Interfaces:**
- Produces:

```rust
pub struct FileResolution {
    pub edges: Vec<ClassifiedEdge>,
    pub flagged: Vec<FlaggedBuiltinDispatchSite>,
    pub indeterminate: Vec<IndeterminateBuiltinDispatchSite>,
}
/// Resolve ALL call-site obligations of ONE workspace file — the exact body of the
/// `for pf in &unit.files` loop at full.rs:647-752, extracted verbatim.
pub fn resolve_file_obligations(
    pf: &ParsedFile,
    primary_app_ref: AppRef,
    graph: &ProgramGraph,
    index: &ResolveIndex,
    body_map: &BodyMap<'_>,
    obj_node_map: &HashMap<ObjectNodeId, &ObjectNode>,
) -> FileResolution;
```

  `resolve_full_program_from_parts` becomes: build index/body_map/obj_node_map (unchanged), loop
  Phase 1 files calling `resolve_file_obligations`, then Phase 2 (unchanged). Edge/obligation ORDER must
  stay identical (same iteration order) — the SHA gate depends on it.
- Consumes: Task 5's shapes (rebase on it; same lane).

- [ ] **Step 1: Failing test.** Fixture workspace: assert
  `resolve_file_obligations(pf, …).edges == (edges from full run filtered to site.span.unit == pf.virtual_path)`
  for every workspace file, and that concatenation-in-file-order equals the full run's Phase-1 edge list
  exactly.
- [ ] **Step 2:** Extract the loop body (mechanical; resist "improvements" — verbatim move).
- [ ] **Step 3:** Full suite; commit; release build; CDO SHA gate byte-identical.
- [ ] **Step 4:** Clippy, rustfmt, CHANGELOG. Commit `feat(program): per-file resolve entry point (t3.6)`.

---

### Task 7: Definition-surface fingerprint

**Files:**
- Create: `src/lsp/def_surface.rs` (+ `pub mod def_surface;` in `src/lsp/mod.rs`)
- Test: same file `#[cfg(test)]`

**Interfaces:**
- Consumes: the AUDITED field list from `docs/superpowers/specs/2026-07-12-t3-def-surface-audit.md`
  (Task 4) — implement it VERBATIM; any deviation goes back through the audit doc first. Node extraction
  helpers from `src/program/build.rs`/`node_extract.rs` (whatever extracts ObjectNode/RoutineNode data
  from a ParsedFile — reuse, don't duplicate).
- Produces:

```rust
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DefSurface(pub [u8; 32]); // blake3 of the canonicalized surface encoding
pub fn def_surface_fingerprint(pf: &ParsedFile) -> DefSurface;
```

  Canonical encoding: length-prefixed field writes into a `blake3::Hasher` in a FIXED documented order
  (no format!-based hashing — length-prefix every string; the Plan-1A content_hash framing lesson).

- [ ] **Step 1: Failing tests — one per audit class.** Fixture AL source pairs (before/after) asserting:
  body-only statement edit → EQUAL; each surface class (routine added/removed/renamed, param added,
  param type changed, var-ness flipped, visibility changed, return type changed, table field added/type
  changed, enum value added, implements changed, SourceTable changed, event attribute changed) →
  NOT EQUAL. Write ALL pairs from the audit's class table — if the audit lists a class this test file
  doesn't cover, the task is incomplete.
- [ ] **Step 2:** Run → FAIL. Implement. Run → PASS.
- [ ] **Step 3:** Determinism test: same input parsed twice → same fingerprint (rayon/thread-local
  parser paranoia).
- [ ] **Step 4:** Clippy, rustfmt, CHANGELOG. Commit `feat(lsp): definition-surface fingerprint (t3.7)`.

---

### Task 8: LspSnapshot — batch builder + owned derived indexes

**Files:**
- Create: `src/lsp/snapshot.rs` (+ module wire)
- Test: same file `#[cfg(test)]` over a fixture workspace

**Interfaces:**
- Consumes: Tasks 5/6/7 products; `ProgramContext`/`build_context` (make `pub(crate)` if needed);
  `RoutineDecl.origin/name_origin`.
- Produces (later tasks build on these EXACT names):

```rust
/// Reference to one edge: (virtual_path, index into edges_by_file[path]). Index-based —
/// never a borrow — so the snapshot is self-contained and Arc-shareable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EdgeRef { pub file: String, pub idx: u32 }

#[derive(Clone, Debug)]
pub struct DeclEntry {
    pub id: RoutineNodeId,
    pub name: String,              // raw casing for display
    pub origin: Origin,            // whole decl (CallHierarchyItem.range)
    pub name_origin: Origin,       // name token (selectionRange)
    pub virtual_path: String,
}

pub struct LspSnapshot {
    pub generation: u64,
    pub graph: ProgramGraph,
    pub dep_layer: Arc<DepLayer>,
    pub snap: AppSetSnapshot,                       // identity/roots for rebuilds
    pub parsed: HashMap<String, Arc<ParsedFileEntry>>, // virtual_path → file+text+DefSurface
    pub edges_by_file: HashMap<String, Arc<Vec<ClassifiedEdge>>>,
    pub event_edges: Arc<Vec<ClassifiedEdge>>,      // Phase-2 EventFlow (whole-program)
    pub incoming: HashMap<RoutineNodeId, Vec<EdgeRef>>, // DERIVED — see build_incoming
    pub decls_by_file: HashMap<String, Vec<DeclEntry>>, // sorted by origin.byte.start
    pub decl_by_id: HashMap<RoutineNodeId, DeclEntry>,
}
pub struct ParsedFileEntry { pub file: AlFile, pub text: String, pub virtual_path: String, pub surface: DefSurface }

impl LspSnapshot {
    /// Full batch build — snapshot → dep layer → assemble → resolve per file → derive indexes.
    pub fn build_full(workspace_root: &Path) -> Option<LspSnapshot>;
    /// Position lookup: file + 0-based line + UTF-8 byte col → routine whose name_origin or body
    /// contains it (name hit preferred).
    pub fn decl_at(&self, virtual_path: &str, line: u32, byte_col: u32) -> Option<&DeclEntry>;
    pub fn edge(&self, r: &EdgeRef) -> &ClassifiedEdge;
}
/// O(E) wholesale rebuild — NEVER incrementally edited (spec §3 law / H-10 lesson).
/// Incoming(S) gets: every Call/Run/ImplicitTrigger edge with a route RouteTarget::Routine(S),
/// AND every EventFlow edge from publisher P with a route targeting S (event direction: P calls S).
pub fn build_incoming(
    edges_by_file: &HashMap<String, Arc<Vec<ClassifiedEdge>>>,
    event_edges: &[ClassifiedEdge],
) -> HashMap<RoutineNodeId, Vec<EdgeRef>>;
```

  `event_edges` EdgeRefs use the reserved file key `"\u{0}events"` (documented constant) so `EdgeRef`
  stays uniform. Workspace-scoped: `edges_by_file` holds ONLY Phase-1 (workspace-caller) buckets.
- [ ] **Step 1: Failing tests** on a fixture workspace (cross-file calls, an event pub/sub pair, an
  overload, a non-ASCII identifier): `build_full` produces buckets whose union equals a direct
  `resolve_full_program_from_parts` run (order-insensitive set compare on (SiteId, route targets));
  `decl_at` hits name/body/none correctly; `build_incoming` finds the cross-file caller and the event
  subscriber's publisher; deterministic across two builds (`generation` excluded).
- [ ] **Step 2:** Implement. `build_full` composes: `SnapshotBuilder` → `parse_snapshot` →
  `build_dep_layer`/`assemble_program_graph` → per-file `resolve_file_obligations` →
  `emit_event_flow_edges` → derive. Reuse `build_context` internals where visible; do NOT re-implement
  ws_file_set logic — extract/reuse the existing one (grep `ws_file_set` in full.rs).
- [ ] **Step 3:** Tests pass; clippy; rustfmt; CHANGELOG. Commit `feat(lsp): LspSnapshot batch builder + owned indexes (t3.8)`.

---

### Task 9: Updater — debounced queue, rungs 1/2/3, atomic swap

**Files:**
- Create: `src/lsp/updater.rs`
- Test: same file `#[cfg(test)]` (synchronous rung functions tested directly; the thread wrapper thin)

**Interfaces:**
- Consumes: Tasks 5–8. Produces:

```rust
pub struct SharedSnapshot(RwLock<Arc<LspSnapshot>>);   // read = Arc clone; write = swap only
impl SharedSnapshot { pub fn get(&self) -> Arc<LspSnapshot>; pub fn swap(&self, s: Arc<LspSnapshot>); }

pub enum ChangeEvent { FileSaved(PathBuf), FileRemoved(PathBuf), DepsChanged, Overflow }

/// SYNCHRONOUS core (unit-testable): apply one coalesced batch, return the new snapshot.
pub fn apply_changes(prev: &LspSnapshot, batch: &[ChangeEvent]) -> Option<LspSnapshot>;

/// Thread wrapper: recv events, coalesce per path within a 100ms debounce window, call
/// apply_changes, swap, then notify (diagnostics hook — Task 12 fills it).
pub fn spawn_updater(shared: Arc<SharedSnapshot>, rx: Receiver<ChangeEvent>,
                     on_swap: impl Fn(&LspSnapshot, &LspSnapshot) + Send + 'static) -> JoinHandle<()>;
```

  Rung selection inside `apply_changes`, per file: parse saved file → `def_surface_fingerprint` vs
  `prev.parsed[path].surface` → equal ⇒ **rung 1**: rebuild `ResolveIndex`/`BodyMap` transiently over
  prev.graph + (prev parsed with F swapped), `resolve_file_obligations(F)`, replace F's bucket + parsed
  entry, `build_incoming`, new snapshot (graph/dep_layer/decls of other files shared via Arc/clone).
  Changed ⇒ **rung 2**: re-extract workspace layer (`assemble_program_graph` with updated ws unit),
  re-resolve ALL workspace files + `emit_event_flow_edges`, rebuild all derived. `FileRemoved` ⇒ rung 2.
  `DepsChanged`/`Overflow` ⇒ **rung 3**: `LspSnapshot::build_full`. ANY doubt (parse error on F, missing
  prev entry, fingerprint of a file that didn't exist) ⇒ escalate one rung — fail closed, never guess.
  **Contingency (from Task 3's measurement — now MANDATORY, not conditional):** Task 3 MEASURED
  ResolveIndex+BodyMap rebuild at ~240–340ms on CDO scale (8–11x over the 30ms threshold; red-flag
  rule fired), so the transient-rebuild default is dead: keep BOTH as an updater-owned cache keyed by
  `generation`, rebuilt on rung 2/3
  only, and rung 1 swaps F's parsed unit into the cached BodyMap's source set BEFORE resolving (the
  audit guarantees signature-only reads of other files; F's own reads see the fresh parse). Document
  which branch was taken in the task report.
- [ ] **Step 1: Failing tests** (fixture workspace, synchronous `apply_changes`):
  (a) body edit in A.al (add a statement calling an EXISTING target) → rung 1 taken (assert via a
  returned/logged RungTaken in the result — add a `pub last_rung: Cell<Rung>` test hook or return
  `(LspSnapshot, Rung)` from an inner fn), A's bucket changed, B's bucket ARC-IDENTICAL (Arc::ptr_eq —
  proves no re-resolve), incoming reflects the new edge;
  (b) signature edit in A.al (add a param) → rung 2, caller in B.al now resolves differently
  (arity mismatch → Unknown or overload re-pick — assert the ACTUAL taxonomy outcome);
  (c) file delete → rung 2, its edges gone from buckets + incoming;
  (d) parse-error save → escalates (no partial state; prev snapshot survives if build fails entirely);
  (e) `FileSaved` for a path NOT in `prev.parsed` (e.g. a file under `.alpackages/` reaching didSave) →
  escalates past rung 1, never misapplies it (Task-4 review Hunt-3 scenario — the dep-file boundary).
- [ ] **Step 2:** Implement; tests pass.
- [ ] **Step 3:** Debounce/coalesce test on `spawn_updater`: 5 rapid saves of one file → exactly 1
  `apply_changes` call (inject a counting wrapper).
- [ ] **Step 3b: RE-MEASURE rung 2 with the REAL workspace-layer path** (Task-3 review escalation):
  Task 3's 1.9s rung-2 pin includes 926ms of `build_program_graph` over the WHOLE snapshot (deps
  included) because `assemble_program_graph` didn't exist yet — it is an UPPER BOUND, not the number.
  Re-run the CDO stage-split ignored test's methodology against the actual rung-2 path
  (ws-parse + `assemble_program_graph` + index/bodymap + full re-resolve), record the measured number
  in `.superpowers/sdd/t3-stage-split.md` (append, don't overwrite) — Task 16 pins from THIS number.
- [ ] **Step 4:** Clippy, rustfmt, CHANGELOG. Commit `feat(lsp): incremental updater — 2-rung ladder + swap (t3.9)`.

---

### Task 10: Incremental-vs-batch differential gate (PERMANENT)

**Files:**
- Create: `tests/lsp_incremental_parity.rs`, fixture workspace `tests/fixtures/lsp-incr/` (small AL
  project: 2 codeunits + 1 table + 1 tableextension + 1 page + an event pub/sub pair + one overload set;
  include a `Løbenr` identifier and an `æøå` line — Unicode classes on purpose)

**Interfaces:**
- Consumes: Tasks 8+9 (`LspSnapshot::build_full`, `apply_changes`).

- [ ] **Step 1:** Scripted edit sequences — each script: copy fixture to a tempdir, `build_full`, apply
  N scripted disk edits (write file + `apply_changes(FileSaved)`), then `LspSnapshot::build_full` fresh
  on the SAME tempdir state and assert EQUIVALENCE: identical edge multiset per file (compare
  `(SiteId, sorted route (target, evidence-discriminant))`), identical `incoming` maps, identical
  `decls_by_file`. **Witness spans are EXCLUDED from the equivalence key BY DESIGN** (audit §6.1:
  rung 1 leaves other files' stored witness byte-spans stale; handlers must re-derive spans live —
  the live-derivation guarantee is tested at handler level in Task 11, not here — document this
  exclusion in the test's header comment). Scripts: body-edit chain (3 consecutive rung-1s), signature change, rename routine,
  add file, delete file, edit that flips overload resolution, event-subscriber attribute edit, and one
  MIXED 6-edit script. Every script asserts equivalence AFTER EVERY EDIT, not just at the end.
- [ ] **Step 2:** Negative control (gate is non-vacuous): assert at least one script actually exercised
  rung 1 AND at least one exercised rung 2 (via the Rung hook from Task 9) — a gate that silently ran
  rung 3 everywhere proves nothing.
- [ ] **Step 3:** All pass; clippy; CHANGELOG (`Added`: the permanent gate). Commit
  `test(lsp): incremental-vs-batch differential gate (t3.10)`.

---

### Task 11: Core handlers — prepare / incoming / outgoing on the new backend

**Files:**
- Create: `src/lsp/handlers.rs`
- Test: same file `#[cfg(test)]` over the Task-10 fixture

**Interfaces:**
- Consumes: `LspSnapshot` API (Task 8), `LineTable`/`PositionEncoding` (Task 2).
- Produces (Task 15 wires these into server dispatch; Task 14 drives them for parity):

```rust
/// item.data payload — serde round-trip of the content-addressed id.
#[derive(Serialize, Deserialize)]
pub struct ItemData { pub node: RoutineNodeId }   // derive Serialize/Deserialize on RoutineNodeId
                                                  // + its component types (additive derives)
pub fn prepare(snap: &LspSnapshot, enc: PositionEncoding, uri: &str, line: u32, character: u32)
    -> Option<Vec<CallHierarchyItem>>;
pub fn incoming(snap: &LspSnapshot, enc: PositionEncoding, data: &ItemData) -> Vec<CallHierarchyIncomingCall>;
pub fn outgoing(snap: &LspSnapshot, enc: PositionEncoding, data: &ItemData) -> Vec<CallHierarchyOutgoingCall>;
```

  Reuse the existing lsp-types structs the legacy handlers use (grep `CallHierarchyItem` in
  handlers.rs for the exact crate/import). ALL conversions at this boundary via `LineTable` from
  `snap.parsed[path].text`. A stale `ItemData` (id not in `decl_by_id` — file changed since prepare) →
  EMPTY result, never a panic or a guess (fail closed).
  **Outgoing route taxonomy (spec §5, binding):** `Routine(id)` → item via `decl_by_id` (dep-source
  targets included — real spans); `conditionalResolved`/`ambiguousResolved` closed candidate sets → one
  item PER candidate; `AbiSymbol{key}` → item with zero-range at a synthesized URI matching the legacy
  external-def fallback (READ `handlers.rs:327-389` first and mirror its shape — record what it does in
  the task report); Builtin / honestDynamic / honestEmpty → NO item.
- [ ] **Step 1: Failing tests:** prepare on a name (hit), on a body (enclosing routine), on whitespace
  outside any routine (None); prepare on the `æøå` line asserting utf-16 vs utf-8 column difference;
  incoming for the fixture's cross-file callee (both callers, correct fromRanges, grouped by caller);
  incoming on a publisher-subscribed routine (subscriber's incoming includes the publisher — event
  direction per Task 8); outgoing with one resolved + one ambiguous (2 candidates → 2 items) + one
  builtin (absent); stale-ItemData → empty; AND the audit-§6.1 live-span test (NON-NEGOTIABLE,
  `docs/superpowers/specs/2026-07-12-t3-def-surface-audit.md` §6.1): apply a rung-1 body edit to the
  TARGET file via `apply_changes`, then run incoming/outgoing from the un-edited caller — every
  returned range must match the target's FRESH parse positions (assert against a fresh batch build),
  proving handlers re-derive spans live from decl_index/BodyMap and never serve stored
  `Witness::SourceSpan` bytes. **This rule EXTENDS to EventFlow edges' `SiteId` spans** (Task-10
  finding): an EventFlow edge's SiteId is anchored at the publisher's name-origin and goes stale
  under rung-1 edits (rung 1 Arc-clones event_edges by design) — event-derived `fromRanges` and
  publisher/subscriber item positions must come from `decl_by_id` lookups, NEVER from
  `edge.site.span` on an EventFlow edge. Add a test: rung-1 body edit above a publisher decl →
  incoming-on-subscriber's fromRanges match the FRESH publisher position.
- [ ] **Step 2:** Implement; pass. Clippy, rustfmt, CHANGELOG. Commit
  `feat(lsp): core call-hierarchy handlers on program engine (t3.11)`.

---

### Task 12: codeLens + diagnostics engine (recompute-diff-publish-clear)

**Files:**
- Create: `src/lsp/lens.rs`, `src/lsp/diagnostics.rs`
- Test: same files `#[cfg(test)]`

**Interfaces:**
- Consumes: `LspSnapshot`, `incoming` counts; `analysis.rs` metrics (ALREADY IR-direct — call it on
  `ParsedFileEntry.file`/text, do not re-implement); `config.rs` thresholds (unchanged).
- Produces:

```rust
pub fn code_lenses(snap: &LspSnapshot, enc: PositionEncoding, uri: &str, cfg: &Config) -> Vec<CodeLens>;
/// Full recompute over the snapshot; returns per-URI diagnostic sets INCLUDING now-empty URIs.
pub fn compute_all(snap: &LspSnapshot, cfg: &Config) -> HashMap<String /*uri*/, Vec<Diagnostic>>;
/// Diff vs last published: (to_publish, to_clear). Task 15 wires it to on_swap.
pub struct DiagnosticsState { /* last published per uri */ }
impl DiagnosticsState { pub fn diff(&mut self, new: HashMap<String, Vec<Diagnostic>>) -> Vec<(String, Vec<Diagnostic>)>; }
```

- [ ] **Step 1: Rule inventory FIRST.** Read `indexer.rs:159-218` + `graph.rs:888-905` + their pinned
  tests (grep `issue` / `unused` in indexer.rs tests). Write the inventory into the task report: every
  exclusion rule (event subscribers, publics/API surface, triggers, etc.) with its legacy test name.
  Then port EACH rule onto engine data (`incoming` count zero + rule exclusions from RoutineNode/decl
  metadata) with a test per rule mirroring the legacy pinned case.
- [ ] **Step 2:** Failing tests → implement → pass. codeLens: counts match `incoming` (same fixture
  numbers as Task 11's), metrics delegate to analysis.rs (assert one known complexity value).
  Diagnostics diff: URI that had findings then none → appears in to_clear (THE missing legacy behavior).
- [ ] **Step 3:** Clippy, rustfmt, CHANGELOG. Commit `feat(lsp): lenses + diffing diagnostics engine (t3.12)`.

---

### Task 13: Custom requests + event surfaces

**Files:**
- Create: `src/lsp/custom.rs`
- Test: same file `#[cfg(test)]`

**Interfaces:**
- Consumes: `LspSnapshot` (dep ABI nodes live in `graph` as `TrustTier::SymbolOnly` / dep-source nodes;
  per-file IR events in `ParsedFileEntry.file`).
- Produces: engine-backed equivalents with the SAME wire shapes as the legacy custom handlers (read
  each legacy impl FIRST; the response JSON shape may not change — clients exist):
  `dependency_document_symbol(snap, params)` (legacy `handlers.rs:1545`, from `dependency_objects`),
  `event_publishers_in_file(snap, enc, uri)` (legacy `:1704`), `event_reference_at_position(snap, enc, uri, pos)`
  (legacy `:1792`). `fieldProperties`/`actionProperties`/`telemetryStatus` are ALREADY
  graph-independent (`handlers.rs:554,566`) — NOT touched here; they survive as-is.
- [ ] **Step 1:** Read the three legacy impls; write down their exact response shapes in the task
  report. Failing tests asserting the same shapes from engine data on the fixture + (env-gated, if the
  legacy test `rpc_on_approvals_mgmt` pattern needs `.alpackages`) a real-dep case.
- [ ] **Step 2:** Implement → pass. Clippy, rustfmt, CHANGELOG. Commit
  `feat(lsp): custom requests on program engine (t3.13)`.

---

### Task 14: Adjudicated differential parity harness (scaffolding — dies with legacy)

**Files:**
- Create: `tests/lsp_differential.rs`

**Interfaces:**
- Consumes: BOTH backends in-process: legacy (`Indexer::index_directory` + `handlers::{prepare_call_hierarchy,
  incoming_calls,outgoing_calls}` + codeLens + unused-proc diag — the exact calls `tests/perf_bounds.rs`
  makes today, copy its setup) and new (`LspSnapshot::build_full` + Task 11/12 handlers).

- [ ] **Step 1:** Driver: for a given workspace, enumerate every routine the LEGACY graph knows
  (definitions) ∪ every `DeclEntry` the NEW snapshot knows; for each, run prepare (at its
  name position) + incoming + outgoing on both; plus per-file codeLens and the unused-proc diagnostic
  sets. Normalize both sides to a common JSON shape (byte columns on BOTH sides — normalize legacy and
  new to UTF-8 byte columns so H-12 conversion differences don't pollute the call-graph diff), sort
  deterministically.
- [ ] **Step 2:** Taxonomy per response item, exactly the spec §8 classes:
  `MATCH`; `NEW_BETTER { CaseFoldHit, CrossAppTarget, DepSourceSpan, EventDirectionMoved }` — each
  claimed class must be MECHANICALLY justified (CaseFoldHit: legacy-miss AND names equal
  case-insensitively; CrossAppTarget: target app ≠ workspace app; DepSourceSpan: legacy had zero-range
  external, new has real span; EventDirectionMoved: the same pub/sub pair present on the other axis —
  spec §5: new backend surfaces subscribers under the PUBLISHER's outgoing + publisher under the
  SUBSCRIBER's incoming, where legacy listed subscribers under the publisher's incoming);
  `REGRESSION` = legacy has it, new lacks it, no justification matched. **Gate: REGRESSION == 0.**
  Unjustifiable NEW-side extras: `NEW_UNEXPLAINED` — gate == 0 too (both directions fail closed).
- [ ] **Step 3:** Fixture run in CI (always-on, the Task-10 fixture + `tests/fixtures/` AL projects —
  grep for an existing multi-file fixture workspace to reuse); CDO run env-gated
  (`CDO_WS` + `ENFORCE_CDO_WS` via `tests/common/cdo.rs` — `#[path]`-include it like sibling tests do).
  On CDO also run ONE H-10 scenario: legacy index → legacy reindex of one file (its own API) →
  re-diff → assert the harness OBSERVES legacy losing cross-file incoming edges while new (after
  `apply_changes` of the same no-op save) keeps them, classified `NEW_BETTER(H10Repair)` — the fifth
  class, edit-scenario-only.
- [ ] **Step 4:** Pin the CDO class counts in the test (exact numbers, ratchet-style) once measured;
  record them in the task report + CHANGELOG. Commit `test(lsp): adjudicated legacy-vs-new differential harness (t3.14)`.
- [ ] **Step 5 (Task-11 review carry):** add a DEP-BEARING fixture arm to the Task-10 incremental
  gate (a workspace + one embedded-source dependency, exercising `dep_decl_by_id`/`dep_texts`
  through rung 1/2 transitions) so the three dep-layer snapshot fields — already widened into the
  gate's canonicalize in the Task-11 fix wave, trivially-equal on the dep-less fixture — get
  non-vacuous coverage. Also: expected differential classes for T14's taxonomy from Task 11's
  adjudicated deviations — external/AbiSymbol targets (legacy reused the CALLER's range; new emits
  an object-level `al-preview` item) and outgoing per-site cardinality are LEGACY-SHAPE-CHANGED
  classes, not regressions. From Task 12 (adjudicated): unused-proc R2 requires a REAL resolved
  EventFlow edge (broken subscription: legacy excluded, new flags — NEW_BETTER precision);
  publisher-as-edge-source is not usage; interface-member exclusion R6 (legacy false-positively
  flagged interface method signatures as unused, new excludes them — NEW_BETTER(InterfaceExclusion)).

---

### Task 15: Server cutover

**Files:**
- Modify: `src/server.rs`, `src/main.rs`, `src/watcher.rs` (event forwarding only)
- Test: existing server/watcher tests + new wiring tests

**Interfaces:**
- Consumes: everything above. After this task the server SERVES the new backend; legacy code still
  compiles (deleted in Task 17, after the Task-14 gate has run against the merged state).

- [ ] **Step 1:** Server state: replace `Arc<RwLock<Indexer>>` with
  `Arc<SharedSnapshot>` + updater channel + `DiagnosticsState`. Initialize: build_full (workspace roots
  from the existing init logic), spawn_updater with `on_swap` = diagnostics diff + publish/clear.
  Dispatch: prepare/incoming/outgoing/codeLens/custom → Task 11–13 functions with the negotiated
  encoding. didSave → send `ChangeEvent::FileSaved`; watcher events → mapped `ChangeEvent`s (overflow →
  `Overflow`); `.alpackages` watch → `DepsChanged` (fixes deps-frozen-at-startup).
- [ ] **Step 2:** `main.rs` CLI `--project` (index-report counts) re-pointed at `LspSnapshot::build_full`
  (definitions = workspace `DeclEntry` count, call sites = Σ bucket lens) — CHANGELOG documents the
  count-definition change honestly. `--analyze` path untouched (analysis.rs is IR-direct).
- [ ] **Step 3:** Smoke test: an integration test driving initialize→prepare→incoming over stdio OR
  (if no such harness exists — check `tests/` for an LSP stdio test first) a direct server-state test
  asserting dispatch reaches the new handlers and a didSave round-trips into a swap (generation bump).
- [ ] **Step 4:** Full suite (legacy unit tests still green — legacy code untouched, only unwired);
  Task-14 harness re-run green. Clippy, rustfmt, CHANGELOG (`Changed`: LSP serves program engine).
  Commit `feat(server): cut LSP surface over to the program-engine backend (t3.15)`.

---

### Task 16: perf_bounds + benches rewrite

**Files:**
- Rewrite: `tests/perf_bounds.rs`, `benches/lsp_pipeline.rs`

**Interfaces:**
- Consumes: new backend API; Task 3's measured stage split (the rung-2 pin).

- [ ] **Step 1:** New perf_bounds (release-only, same 3x-bound convention — read the current file's
  gating/env pattern and keep it): on the 1000-file perf corpus: `build_full` < 6s (3× the 2s target);
  prepare/incoming/outgoing < 3ms each (3× 1ms) — keep the SEMANTIC asserts (999-way fan-in count
  survives as an incoming-length assert against the new backend); rung-1 `apply_changes` (body edit)
  < 300ms (3× 100ms); rung-2 (signature edit) < 3× the Task-9-RE-MEASURED rung-2 number from
  `.superpowers/sdd/t3-stage-split.md` (Task 3's 1.9s figure is a whole-snapshot-build UPPER BOUND —
  do NOT bake it; Task-3 review escalation).
- [ ] **Step 2:** `benches/lsp_pipeline.rs` mirrored onto the same operations (Criterion rows renamed
  to match the new table in CLAUDE.md — Task 17 updates the doc).
- [ ] **Step 3:** Run release perf test locally; record numbers in the task report. Clippy, CHANGELOG.
  Commit `test(perf): rewrite perf bounds + benches for the engine-backed LSP (t3.16)`.

---

### Task 17: Capstone — legacy deletion, docs, final gates

**Files:**
- Delete: `src/graph.rs`, `src/indexer.rs`, `src/parser.rs`, `tests/lsp_differential.rs` (scaffolding),
  legacy sections of `src/handlers.rs` (delete the file if Tasks 11–13 + the graph-independent
  field/action/telemetry handlers have fully replaced it — move those three handlers into
  `src/lsp/custom.rs` if they still live in handlers.rs), `tests/parser-ir-goldens/` (the r0 projection
  golden retires WITH parser.rs — CHANGELOG documents the retirement)
- Modify: `src/lib.rs`, `src/main.rs` (drop dead re-exports), `CLAUDE.md` (Pipeline-1 section, Key
  Modules, perf table, "Adding New AL Constructs" step 3 LSP pointer → `src/lsp/`), `README.md` (if it
  names deleted modules), `CHANGELOG.md` (the arc's `Removed` section)

- [ ] **Step 1:** Verify the Task-14 harness ran green on CDO in the merged branch state (it is the
  deletion license). Then delete in one commit: legacy modules + the harness + dead re-exports.
- [ ] **Step 2:** Fix every compile fallout by DELETION (tests of deleted modules go too — list each
  deleted test in the report with the replacement that covers it: graph.rs unit tests → Task 8/10/11
  coverage; indexer issue-#20 tests → Task 12's per-rule ports; parser golden → retired).
  NOTE (Task-12 review): `routine_complexity_ir`/`is_framework_invocation_attribute`/`is_event_publisher`
  were RELOCATED out of parser.rs in the t3.12 fix wave (surviving module; parser.rs re-exports them
  until deletion) — verify no surviving module imports from parser.rs before deleting it.
  `cargo test` full suite green; clippy bar green.
- [ ] **Step 3:** Docs: CLAUDE.md Pipeline-1 rewritten (the two-pipeline framing becomes one engine,
  two consumers: LSP surface + CLI/aldump); perf table replaced with Task 16's rows + measured numbers;
  Key Modules updated (graph/indexer/parser removed, src/lsp/* added); README checked. CHANGELOG.
- [ ] **Step 4:** Final gates, in order: full suite; release build; CDO SHA byte-identical
  (`0a3b85bc…`); Task-10 incremental gate; `scripts/cdo-gate U:/Git/DO.Support-SlowDOSetup/DocumentOutput/Cloud`
  (the full CDO-gated suite). Commit-before-gate law applies.
- [ ] **Step 5:** Whole-branch Opus review (refute-by-default), fix wave if needed, then present the
  finishing-a-development-branch menu — merge to master ONLY on the user's explicit "1".

---

## Self-review notes (done at write time)

- **Spec coverage:** §3 architecture → T8; §4 ladder+audit+contingency → T4/T7/T9; §5 handler table →
  T11/T12/T13; §6 H-12/H-13 → T2/T1; §7 server model → T9/T15; §8 harness+gates → T10/T14; §9 perf →
  T3/T16; §10 deletions+docs → T17; §11 risks → T4 step 3 (STOP rule), T9 contingency, T12 step 1
  inventory, T3 red-flag rule. §12 deferrals: none scheduled (correct).
- **Event direction:** spec §5 lists event-subscribers-as-incoming "publisher's incoming shows
  subscribers … whole-program"; this plan REFINES it to the natural direction (subscriber's incoming ∋
  publisher; publisher's outgoing ∋ subscribers) with the legacy shape adjudicated as
  `NEW_BETTER(EventDirectionMoved)` in T14 — flagged here explicitly as a deliberate refinement for the
  user's plan review, not a silent change.
- **Type consistency:** `EdgeRef`/`DeclEntry`/`LspSnapshot`/`apply_changes`/`SharedSnapshot`/`DefSurface`
  names used identically across T7–T16. `FileResolution`/`resolve_file_obligations` across T6/T8/T9.
- **No placeholders:** every code step carries signatures/tests; where a legacy shape must be mirrored,
  the step says READ the named file:line first and record it — that is an instruction, not a TBD.
