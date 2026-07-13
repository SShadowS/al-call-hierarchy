# Perf Safe Wins (T3 regression mitigations 1, 2, 5) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove the three "sure and safe" costs identified in
`docs/perf-regression-t3-vs-0.9.3.md`: (1) the duplicated full parse in
`LspSnapshot::build_full_with_parsed` (plus the per-file re-parses in rung 1/rung 2),
(2) the three independent copies of all embedded dependency source text, and
(3) parsing `SymbolReference.json` for `.app` files that GUID-dedup then throws away.

**Architecture:** Both LSP-surface fixes are *sharing* fixes, not caching fixes.
Source text becomes one `Arc<str>` per file shared by
`SourceFile`/`ParsedFile`/`ParsedFileEntry`/`dep_texts` (was: 3 independent
allocations of the same ~118 MB). Parsed IR becomes one `Arc<AlFile>` per file
shared by the published snapshot and the updater's working state (was: two fully
independent `parse_snapshot` passes at startup, plus a fresh `al_syntax::parse`
per touched file at rung 1 and per workspace file at rung 2 — all solely because
`AlFile` is not `Clone`). Neither side ever mutates an `AlFile` in place — rung
splices always *replace* whole `ParsedFile` values — so shared immutable
ownership is sound by construction. The `.app` fix reorders `load_all_apps` to
manifest-first: parse only `NavxManifest.xml` (KB-sized) for every discovered
`.app`, GUID-dedup on that identity, then parse `SymbolReference.json`
(MB-sized) for winners only.

**Tech Stack:** Rust; `serde` (needs the `rc` feature for `Arc<str>` derive);
existing test suites (`cargo test`, `tests/lsp_incremental_parity.rs` is the
behavioral parity gate).

## Global Constraints

- Format touched files with `rustfmt <file>` — NEVER `cargo fmt` (whole-crate churn).
- Lint with `cargo clippy --all-targets --all-features` after each task.
- Update `CHANGELOG.md` (Keep-a-Changelog: Added/Changed/Fixed groups) in each task's commit.
- Stage only intended paths; never `git add -A`. Never push or merge to `master`.
- Zero behavior change to any LSP-served data: `tests/lsp_incremental_parity.rs`
  (the incremental-vs-batch differential gate) must pass unmodified except where
  a step below explicitly adds a NEW test.
- `REGEN_TEMP_GOLDENS` must NOT be needed — no golden may change. If a golden
  diff appears, that is a bug in the change, not a rebaseline opportunity.

## File Structure (all files already exist unless marked Create)

| File | Role in this plan |
|---|---|
| `Cargo.toml` | add serde `rc` feature |
| `src/snapshot/embedded.rs` | `SourceFile.text: String → Arc<str>` |
| `src/snapshot/provider.rs` | workspace `SourceFile` construction site |
| `src/snapshot/parse.rs` | `ParsedFile.text → Arc<str>`, `ParsedFile.file → Arc<AlFile>`, `parse_snapshot` shares instead of clones |
| `src/lsp/snapshot.rs` | `ParsedFileEntry` fields, `build_dep_indexes` shares text, `from_context` returns the parse instead of dropping deps, `build_full_with_parsed` loses its second `parse_snapshot` |
| `src/lsp/updater.rs` | rung-1/rung-2 `ParsedFileEntry`s share `Arc`s instead of re-parsing |
| `src/lsp/def_surface.rs` | test helper constructor mechanical update |
| `src/app_package.rs` | Create `extract_app_metadata` / `extract_app_symbols` split |
| `src/dependencies.rs` | manifest-first `load_all_apps`, `dedup_by_guid_keep_highest_version` retyped to manifest-level identity |
| `tests/lsp_incremental_parity.rs` | new sharing-proof tests (fixture `tests/fixtures/lsp-diff-deps/` already exists) |
| `docs/perf-regression-t3-vs-0.9.3.md` | close-out note |

Test-only construction sites that will need the same mechanical field updates
(enumerated so nobody has to re-grep): `src/program/build.rs:653,878`,
`src/program/resolve/semantic_golden.rs:3179`, `src/program/resolve/body_map.rs:130`,
`src/program/resolve/resolver.rs:3152,9768`, `src/program/abi_ingest.rs:1125`,
`src/lsp/handlers.rs:1109`, `src/lsp/custom.rs:922`, `src/lsp/def_surface.rs:462`,
`src/lsp/updater.rs:1623,1662,1955,1982`, `src/snapshot/parse.rs:170`,
`tests/program_resolve_harness.rs:289,6608–7067`.

---

### Task 1: One `Arc<str>` per source file (kills text copies T2 + T3)

**Files:**
- Modify: `Cargo.toml:38`
- Modify: `src/snapshot/embedded.rs:19-22,81`
- Modify: `src/snapshot/provider.rs:50`
- Modify: `src/snapshot/parse.rs:16-24,94,170`
- Modify: `src/lsp/snapshot.rs:82-88,403,630`
- Modify: `src/lsp/updater.rs:327-333,445,648`
- Test: `tests/lsp_incremental_parity.rs` (new test at end of file)

**Interfaces:**
- Consumes: nothing from other tasks (this task goes first).
- Produces: `SourceFile { virtual_path: String, text: Arc<str> }`,
  `ParsedFile { virtual_path, file, provenance, text: Arc<str> }`,
  `ParsedFileEntry { file, text: Arc<str>, virtual_path, surface }`.
  Task 2 relies on `ParsedFile.text`/`ParsedFileEntry.text` being `Arc<str>`
  (cheap to clone when `from_context` stops consuming the parse).

**Why safe:** `Arc<str>` derefs to `str`, so every `&pf.text` passed where
`&str` is expected keeps compiling via deref coercion. The only real semantic
touchpoint is serde: `SourceFile` derives `Serialize`/`Deserialize` (used by
the content-addressed source cache, `src/snapshot/cache.rs`) — `Arc<str>`
serializes as a plain string with serde's `rc` feature, so the on-disk JSON
cache format is byte-identical and existing cache entries stay readable.

- [ ] **Step 1: Enable serde `rc`**

In `Cargo.toml` change:

```toml
serde = { version = "1", features = ["derive"] }
```

to:

```toml
serde = { version = "1", features = ["derive", "rc"] }
```

- [ ] **Step 2: Write the failing test**

Append to `tests/lsp_incremental_parity.rs` (it already has
`copy_fixture_lsp_diff_deps_to_tempdir()` and `build_full_with_parsed(dir)`
helpers, and the `lsp-diff-deps` fixture ships a real embedded-source
dependency):

```rust
/// Perf safe-wins Task 1: embedded dependency source text must be ONE shared
/// allocation — `dep_texts`'s `Arc<str>` and the `AppSetSnapshot`'s own
/// `SourceFile.text` must be pointer-equal, never independent copies (the
/// perf doc's T1/T2 duplication).
#[test]
fn dep_texts_share_the_snapshot_source_text_allocation() {
    let dir = copy_fixture_lsp_diff_deps_to_tempdir();
    let (base, _parsed) = build_full_with_parsed(dir.path());

    assert!(
        !base.dep_texts.is_empty(),
        "fixture sanity: dep_texts must carry Source Mgt.al's embedded source"
    );
    for ((app_ref, vp), dep_text) in base.dep_texts.iter() {
        let app_id = base.graph.apps.resolve(*app_ref);
        let unit = base
            .snap
            .apps
            .iter()
            .find(|u| &u.id == app_id)
            .expect("dep_texts app must exist in snap");
        let sf = unit
            .source
            .as_ref()
            .expect("dep with texts has embedded source")
            .files
            .iter()
            .find(|f| &f.virtual_path == vp)
            .expect("dep_texts path must exist in snap source");
        assert!(
            std::sync::Arc::ptr_eq(dep_text, &sf.text),
            "dep_texts[({app_ref:?}, {vp})] must share the snapshot's text \
             allocation, not copy it"
        );
    }
}
```

(`AppRegistry::resolve(&self, r: AppRef) -> &AppId` exists at
`src/program/node.rs:42` — verified.)

- [ ] **Step 3: Run test to verify it fails (to compile)**

Run: `cargo test --test lsp_incremental_parity dep_texts_share_the_snapshot_source_text_allocation`
Expected: FAIL — compile error (`ptr_eq` on `Arc<str>` vs `&String` — the
types don't line up until `SourceFile.text` becomes `Arc<str>`).

- [ ] **Step 4: Change the three text fields**

`src/snapshot/embedded.rs`:

```rust
/// One embedded source file recovered from a `.app`.
///
/// `text` is `Arc<str>` (perf safe-wins Task 1): the SAME allocation is
/// shared by `ParsedFile.text` and `LspSnapshot::dep_texts` — embedded
/// dependency source (~114 MB on a real BC workspace) must exist in memory
/// exactly once. Serde's `rc` feature serializes it as a plain string, so
/// the content-addressed source cache format (`snapshot::cache`) is
/// unchanged.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct SourceFile {
    pub virtual_path: String,
    pub text: std::sync::Arc<str>,
}
```

and its construction at line 81: `out.push(SourceFile { virtual_path, text: text.into() });`
(`String: Into<Arc<str>>`).

`src/snapshot/provider.rs:50`: `files.push(SourceFile { virtual_path, text: text.into() });`

`src/snapshot/parse.rs`:

```rust
pub struct ParsedFile {
    pub virtual_path: String,
    pub file: al_syntax::ir::AlFile,
    pub provenance: Provenance,
    /// The original AL source text — the SAME `Arc<str>` allocation as the
    /// snapshot's `SourceFile.text` (perf safe-wins Task 1), never a copy.
    pub text: std::sync::Arc<str>,
}
```

and in `parse_snapshot` (line 94): `text: Arc::clone(&f.text),`
(add `use std::sync::Arc;`). The test fixture at line 170 becomes
`text: text.into(),` (it maps from `&str`; use `Arc::from(text)` if `.into()`
is ambiguous there).

`src/lsp/snapshot.rs`:

```rust
pub struct ParsedFileEntry {
    pub file: AlFile,
    /// Shares the workspace `SourceFile.text` allocation (perf safe-wins Task 1).
    pub text: Arc<str>,
    pub virtual_path: String,
    pub surface: DefSurface,
}
```

`build_dep_indexes` (line 630): replace the fresh copy with a share:

```rust
dep_texts
    .entry((app_ref, pf.virtual_path.clone()))
    .or_insert_with(|| Arc::clone(&pf.text));
```

Line 403 (`.text.as_str()`): `Arc<str>` has no `as_str`; use `let text: &str = &self.parsed.get(&d.virtual_path)?.text;`.

`src/lsp/updater.rs`: line 327 area (rung-1 disk read) — the fresh text is a
new `String` from `read_to_string`, so: `let text: Arc<str> = text.into();`
right after the parse (parse first — `al_syntax::parse(&text)` works on either
type via deref). Lines 445 and 648: `text: pf.text.clone()` stays literally
the same source text but is now an `Arc` clone (leave `.clone()` — on `Arc<str>`
it IS the cheap share; no code change needed beyond what the compiler demands).

- [ ] **Step 5: Chase the compiler**

Run: `cargo build 2>&1 | head -80` and fix remaining sites mechanically —
they are all in the enumerated test-constructor list in File Structure above,
and every fix is one of: `text: "...".into()`, `text: Arc::from(src)`,
`&*x.text` where a `&str`/`String` was expected, or `x.text.to_string()` where
an owned `String` is genuinely required (test-only).
Expected: clean build.

- [ ] **Step 6: Run the new test + parity gate**

Run: `cargo test --test lsp_incremental_parity`
Expected: PASS, including `dep_texts_share_the_snapshot_source_text_allocation`.

- [ ] **Step 7: Full validation**

Run: `cargo test` then `cargo clippy --all-targets --all-features`
Expected: all green, no new clippy warnings.

- [ ] **Step 8: Format, changelog, commit**

`rustfmt` each touched `.rs` file individually. Add to `CHANGELOG.md` under
`Changed`: one entry describing the single-allocation text sharing
(`SourceFile`/`ParsedFile`/`ParsedFileEntry`/`dep_texts`, serde `rc`), citing
the perf doc §3.1 (~228 MB duplicate text on the reference workspace).

```bash
git add Cargo.toml Cargo.lock src/snapshot/embedded.rs src/snapshot/provider.rs src/snapshot/parse.rs src/lsp/snapshot.rs src/lsp/updater.rs src/lsp/def_surface.rs tests/lsp_incremental_parity.rs CHANGELOG.md
# plus any test files the compiler chase touched (git status first!)
git commit -m "perf: share one Arc<str> per source file across snapshot/parse/dep_texts

Co-authored-by: Copilot <223556219+Copilot@users.noreply.github.com>"
```

---

### Task 2: One `Arc<AlFile>` per parse — delete the duplicate `parse_snapshot` and the rung re-parses

**Files:**
- Modify: `src/snapshot/parse.rs:16-24,90-96` (`ParsedFile.file → Arc<AlFile>`)
- Modify: `src/lsp/snapshot.rs:82-88,226-247,242-380` (`ParsedFileEntry.file → Arc<AlFile>`; `from_context` returns the parse; `build_full_with_parsed` stops double-parsing)
- Modify: `src/lsp/updater.rs:327-333,437-452,640-655` (classify wraps once; rung 1/2 share instead of re-parsing)
- Modify: `src/lsp/def_surface.rs:462-470` and the other test constructors from the File Structure list
- Test: `tests/lsp_incremental_parity.rs` (new test)

**Interfaces:**
- Consumes: Task 1's `Arc<str>` text fields (so `from_context` can populate
  `ParsedFileEntry` without consuming the parse).
- Produces: `ParsedFile { virtual_path: String, file: Arc<al_syntax::ir::AlFile>, provenance: Provenance, text: Arc<str> }`;
  `ParsedFileEntry { file: Arc<AlFile>, text: Arc<str>, virtual_path: String, surface: DefSurface }`;
  `LspSnapshot::from_context(ctx, root) -> (LspSnapshot, Vec<ParsedUnit>)`
  (second element: the ONE parse, returned intact for the updater).
  `build_full`/`build_full_with_parsed` public signatures are UNCHANGED.

**Soundness argument (record in the code docs you touch):** nothing in the
engine mutates an `AlFile` after `al_syntax::parse` returns — rung 1 splices a
*whole fresh `ParsedFile`* into `pending`, rung 2 `splice_file`s whole values,
rung 3 replaces the whole `Vec<ParsedUnit>`. So snapshot and updater sharing
one immutable `Arc<AlFile>` per file cannot observe each other's updates any
differently than two private copies did. This supersedes
`build_full_with_parsed`'s "a second parse is the honest fix" doc — rewrite
that doc, don't leave it contradicting the code.

- [ ] **Step 1: Grep-verify the no-mutation premise**

Run: `grep -rn "mut.*AlFile\|AlFile.*mut" src/ crates/al-syntax/src/ --include=*.rs | grep -v "^.*://"`
Expected: no hit showing `&mut AlFile` outside `al_syntax`'s own construction
(`lower`). If a real post-parse mutation site exists, STOP and re-plan this task.

- [ ] **Step 2: Write the failing test**

Append to `tests/lsp_incremental_parity.rs`:

```rust
/// Perf safe-wins Task 2: `build_full_with_parsed` must NOT run a second
/// whole-program parse — the published snapshot's workspace `AlFile`s and
/// the updater's working-state `AlFile`s must be the SAME `Arc` allocations.
#[test]
fn build_full_with_parsed_shares_one_parse_between_snapshot_and_updater() {
    let dir = copy_fixture_lsp_diff_deps_to_tempdir();
    let (base, parsed) = build_full_with_parsed(dir.path());

    let ws_unit = parsed
        .iter()
        .find(|u| u.app == base.snap.workspace_app)
        .expect("updater parse must include the workspace unit");
    assert!(
        !base.parsed.is_empty(),
        "fixture sanity: snapshot must hold workspace ParsedFileEntry values"
    );
    for (vp, entry) in base.parsed.iter() {
        let pf = ws_unit
            .files
            .iter()
            .find(|f| &f.virtual_path == vp)
            .expect("every snapshot workspace file must be in the updater unit");
        assert!(
            std::sync::Arc::ptr_eq(&entry.file, &pf.file),
            "{vp}: snapshot and updater must share ONE parsed AlFile, \
             not two independent parses"
        );
        assert!(
            std::sync::Arc::ptr_eq(&entry.text, &pf.text),
            "{vp}: snapshot and updater must share ONE text allocation"
        );
    }
    // The updater also holds the dependency units (rung 2 needs them for
    // BodyMap/build_dep_indexes) — exactly one source-bearing unit per
    // source-bearing app, same as parse_snapshot produced.
    assert!(
        parsed.len() >= 2,
        "lsp-diff-deps has an embedded-source dep: updater must hold its unit too"
    );
}
```

- [ ] **Step 3: Run test to verify it fails (to compile)**

Run: `cargo test --test lsp_incremental_parity build_full_with_parsed_shares_one_parse`
Expected: FAIL — compile error (`Arc::ptr_eq` needs `Arc<AlFile>` fields;
today both are bare `AlFile`).

- [ ] **Step 4: Retype the two `file` fields**

`src/snapshot/parse.rs`:

```rust
pub struct ParsedFile {
    pub virtual_path: String,
    /// `Arc`-shared (perf safe-wins Task 2): the published `LspSnapshot`'s
    /// `ParsedFileEntry.file` and the updater's working state hold the SAME
    /// parse. Sound because no consumer mutates an `AlFile` after
    /// `al_syntax::parse` — updates always REPLACE whole `ParsedFile`s
    /// (rung-1 `pending` splice / rung-2 `splice_file` / rung-3 wholesale).
    pub file: std::sync::Arc<al_syntax::ir::AlFile>,
    pub provenance: Provenance,
    pub text: std::sync::Arc<str>,
}
```

and in `parse_snapshot`: `file: Arc::new(al_syntax::parse(&f.text)),`.

`src/lsp/snapshot.rs`: `ParsedFileEntry.file: Arc<AlFile>` (same doc pointer).

- [ ] **Step 5: Rework `from_context` to return the parse**

In `src/lsp/snapshot.rs`, change the signature to
`pub(crate) fn from_context(ctx: ProgramContext, workspace_root: &Path) -> (LspSnapshot, Vec<ParsedUnit>)`
and replace the whole "Ownership-move phase" block (the
`parsed.into_iter().nth(idx)` loop) with a borrow + Arc-clone loop that leaves
`parsed` intact:

```rust
// ── Sharing phase (perf safe-wins Task 2): `AlFile`/text are Arc-shared,
// so the published snapshot CLONES the Arcs and the ONE parse survives
// intact for the caller (build_full_with_parsed hands it to the updater;
// build_full just drops it — dep IR arenas free at that drop, exactly as
// they did under the old consume-and-drop scheme).
let mut parsed_files: HashMap<String, Arc<ParsedFileEntry>> = HashMap::new();
if let Some(idx) = primary_unit_idx {
    for pf in &parsed[idx].files {
        if !ws_file_set.contains(&pf.virtual_path) {
            continue;
        }
        let surface = surfaces_by_file
            .remove(&pf.virtual_path)
            .expect("a surface was computed for every ws_file_set member above");
        parsed_files.insert(
            pf.virtual_path.clone(),
            Arc::new(ParsedFileEntry {
                file: Arc::clone(&pf.file),
                text: Arc::clone(&pf.text),
                virtual_path: pf.virtual_path.clone(),
                surface,
            }),
        );
    }
}

let snapshot = LspSnapshot {
    /* ...unchanged field list... */
};
(snapshot, parsed)
```

Then:

```rust
pub fn build_full(workspace_root: &Path) -> Option<LspSnapshot> {
    let ctx = build_context(workspace_root)?;
    Some(Self::from_context(ctx, workspace_root).0)
}

pub fn build_full_with_parsed(workspace_root: &Path) -> Option<(LspSnapshot, Vec<ParsedUnit>)> {
    let ctx = build_context(workspace_root)?;
    Some(Self::from_context(ctx, workspace_root))
}
```

Delete the second `parse_snapshot` call and REWRITE `build_full_with_parsed`'s
doc comment: the "AlFile is not Clone → second independent pass is the honest
fix" rationale is now false; the new rationale is "one parse, Arc-shared, per
the sharing soundness note on `ParsedFile.file`". Also update
`from_context`'s callers in `src/lsp/handlers.rs`'s tests (they call it
directly; destructure the tuple, use `.0` or `let (snap, _parsed) = ...`).

- [ ] **Step 6: Delete the rung-1 and rung-2 re-parses**

`src/lsp/updater.rs` `apply_rung1_core` (line ~640): replace

```rust
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
```

with

```rust
// One parse, Arc-shared with the pending working copy (perf safe-wins
// Task 2) — see ParsedFile::file's sharing soundness doc.
parsed_files.insert(
    vp.clone(),
    Arc::new(ParsedFileEntry {
        file: Arc::clone(&pf.file),
        text: Arc::clone(&pf.text),
        virtual_path: vp.clone(),
        surface,
    }),
);
```

(the `pending.insert(vp, pf);` after it is unchanged — the clones happen
before the move). Delete the stale "A SECOND, independent parse…" comment.

`apply_rung2` (line ~443): same replacement for its `file2 =
al_syntax::parse(&pf.text)` block — rung 2 currently re-parses EVERY
workspace file; after this it re-parses none.

Rung-1 classify (line ~327): wrap the one genuinely fresh parse:
`let file = Arc::new(al_syntax::parse(&text));` — the `parse_status` check on
the next line keeps working through the `Arc` (`file.parse_status` auto-derefs).

- [ ] **Step 7: Chase the compiler**

Run: `cargo build 2>&1 | head -80`. Remaining fixes are mechanical, all in the
File Structure list's test constructors: `file: Arc::new(al_syntax::parse(src))`
in fixture builders, `&*pf.file` where a bare `&AlFile` binding is required
(most call sites auto-deref and need nothing).
Expected: clean build.

- [ ] **Step 8: Run the sharing test + the full parity gate**

Run: `cargo test --test lsp_incremental_parity`
Expected: PASS — including both new sharing tests and every pre-existing
incremental-vs-batch equivalence script (this is the proof the sharing changed
no served data).

- [ ] **Step 9: Full validation + perf sanity**

Run: `cargo test` then `cargo clippy --all-targets --all-features`.
Expected: all green.
Then (release perf bounds, the CI gate — rung budgets must not regress and
build_full should improve): `cargo test --release --test perf_bounds`
Expected: PASS.

- [ ] **Step 10: Format, changelog, commit**

`rustfmt` each touched file. `CHANGELOG.md` under `Changed`/`Fixed`: duplicate
whole-program parse at LSP startup eliminated (`build_full_with_parsed` now
returns the one shared parse; rung-1/rung-2 per-file re-parses removed) —
cite perf doc §2.3/§3.2 (~1 s cold start + one full dep text+arena set on the
reference workspace).

```bash
git add src/snapshot/parse.rs src/lsp/snapshot.rs src/lsp/updater.rs src/lsp/handlers.rs src/lsp/def_surface.rs tests/lsp_incremental_parity.rs CHANGELOG.md
# plus test-constructor files the compiler chase touched (git status first!)
git commit -m "perf: share one Arc<AlFile> per parse; delete duplicate startup parse and rung re-parses

Co-authored-by: Copilot <223556219+Copilot@users.noreply.github.com>"
```

---

### Task 3: Manifest-first `.app` dedup — never parse a loser's SymbolReference

**Files:**
- Modify: `src/app_package.rs:303-313` (split extraction)
- Modify: `src/dependencies.rs:30-160,383-460` (`DiscoveredApp`, retyped dedup, reordered `load_all_apps`)
- Test: `src/dependencies.rs` tests module (new fixture writer + new test; retype existing dedup unit tests)

**Interfaces:**
- Consumes: nothing from Tasks 1–2 (independent; ordered last only because it's smallest).
- Produces: `pub fn extract_app_metadata(path: &Path) -> Result<AppMetadata>` and
  `pub fn extract_app_symbols(path: &Path) -> Result<Vec<ExternalObject>>` in
  `app_package.rs`. `load_all_apps`'s public signature is UNCHANGED.

**Known, accepted behavior change (document in CHANGELOG):** previously, a
duplicated-GUID `.app` whose *winner* had a corrupt `SymbolReference.json`
failed extraction and the stale loser silently survived dedup. Now the loser
is dropped on manifest identity first; a winner with corrupt symbols is
warned-and-skipped. A corrupt package is a broken input either way — the new
order is the honest one (identity comes from the manifest, not from whether a
100 MB symbol blob happens to parse).

- [ ] **Step 1: Write the failing test**

In `src/dependencies.rs`'s `mod tests`, add a fixture writer (adapted from
`src/snapshot/snapshot.rs`'s `write_minimal_app`, with the symbol payload
parameterized) and the ordering-proof test:

```rust
/// Like `snapshot::tests::write_minimal_app`, but with a caller-supplied
/// SymbolReference payload so a test can plant a CORRUPT one.
fn write_app_with_symbols(
    dir: &std::path::Path,
    filename: &str,
    guid: &str,
    version: &str,
    symbol_reference: &str,
) -> PathBuf {
    use std::io::Write;
    let manifest = format!(
        r#"<?xml version="1.0" encoding="utf-8"?><Package xmlns="http://schemas.microsoft.com/navx/2015/manifest"><App Id="{guid}" Name="DupApp" Publisher="Pub" Version="{version}" Runtime="13.0" /></Package>"#
    );
    let mut zip_bytes = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut zip_bytes);
        let options: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default();
        zip.start_file("NavxManifest.xml", options).unwrap();
        zip.write_all(manifest.as_bytes()).unwrap();
        zip.start_file("SymbolReference.json", options).unwrap();
        zip.write_all(symbol_reference.as_bytes()).unwrap();
        zip.finish().unwrap();
    }
    let path = dir.join(filename);
    let mut out = std::fs::File::create(&path).unwrap();
    out.write_all(&[0u8; 40]).unwrap(); // NAVX header (content unused)
    out.write_all(zip_bytes.get_ref()).unwrap();
    path
}

/// Perf safe-wins Task 3: GUID dedup must happen on MANIFEST identity,
/// BEFORE any SymbolReference.json is parsed. The stale 24.0 loser here
/// carries deliberately corrupt symbols — under the old order (extract
/// everything, then dedup) it fails extraction and is silently skipped
/// (dropped list EMPTY); under manifest-first it must be reported as a
/// proper dedup drop, and its symbol blob must never need to parse.
#[test]
fn guid_dedup_drops_loser_on_manifest_identity_without_parsing_its_symbols() {
    let dir = tempfile::tempdir().expect("tempdir");
    let alpackages = dir.path().join(".alpackages");
    std::fs::create_dir_all(&alpackages).unwrap();
    let guid = "cccccccc-2222-2222-2222-222222222222";
    write_app_with_symbols(
        &alpackages,
        "Pub_DupApp_24.0.0.0.app",
        guid,
        "24.0.0.0",
        "{ this is not JSON",
    );
    write_app_with_symbols(
        &alpackages,
        "Pub_DupApp_25.0.0.0.app",
        guid,
        "25.0.0.0",
        r#"{"Codeunits":[{"Id":50100,"Name":"DupCU","Methods":[{"Name":"DoIt","Id":1}]}]}"#,
    );

    let (kept, dropped) = load_all_apps(dir.path()).expect("load_all_apps");

    assert_eq!(kept.len(), 1, "exactly the 25.0 winner must survive");
    assert_eq!(kept[0].dependency.version, "25.0.0.0");
    assert_eq!(
        dropped.len(),
        1,
        "the 24.0 loser must be a REPORTED dedup drop — not a silent \
         extraction failure (which is what the old symbols-first order made it)"
    );
    assert_eq!(dropped[0].dropped_version, "24.0.0.0");
    assert_eq!(dropped[0].kept_version, "25.0.0.0");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib guid_dedup_drops_loser_on_manifest_identity`
Expected: FAIL on the `dropped.len() == 1` assertion (old order: the corrupt
24.0 fails `extract_app_package`, is warn-skipped, `dropped` is empty).

- [ ] **Step 3: Split extraction in `app_package.rs`**

Below `extract_app_package` add:

```rust
/// Manifest-only read (perf safe-wins Task 3): parses NavxManifest.xml
/// (KB-sized) WITHOUT touching SymbolReference.json (MB-sized) — the cheap
/// identity probe `load_all_apps` dedups on before paying for full symbol
/// extraction.
pub fn extract_app_metadata(path: &Path) -> Result<AppMetadata> {
    let mut archive = open_app_zip(path)?;
    parse_manifest(&mut archive)
}

/// Symbols-only read: the expensive half of [`extract_app_package`], for a
/// caller that already holds the manifest via [`extract_app_metadata`].
pub fn extract_app_symbols(path: &Path) -> Result<Vec<ExternalObject>> {
    let mut archive = open_app_zip(path)?;
    parse_symbols(&mut archive)
}
```

(If `parse_symbols`'s return type differs from `Vec<ExternalObject>`, mirror
whatever `ParsedAppPackage.objects`' type actually is.)

- [ ] **Step 4: Retype the dedup to manifest-level identity**

In `src/dependencies.rs` add, near `ResolvedDependency`:

```rust
/// One `.app` discovered on disk with ONLY its manifest read — the
/// pre-symbol-extraction identity `load_all_apps` dedups on
/// (perf safe-wins Task 3).
#[derive(Debug)]
struct DiscoveredApp {
    app_path: PathBuf,
    meta: crate::app_package::AppMetadata,
}
```

and retype `dedup_by_guid_keep_highest_version` to
`fn dedup_by_guid_keep_highest_version(deps: Vec<DiscoveredApp>) -> (Vec<DiscoveredApp>, Vec<DroppedDuplicateDependency>)`
— body identical in structure, with field accesses moved to the manifest:
`rd.dependency.app_id` → `rd.meta.app_id`, `.dependency.version` →
`.meta.version`, `.dependency.name` → `.meta.name`, `.app_path` unchanged.
Keep every doc comment (the H-2 story is unchanged); note in the doc that
identity now comes from the manifest, which is what the old code's
`AppDependency` was built from anyway (`load_all_apps` filled it from
`package.metadata` verbatim — same values, read earlier).

Retype the existing pure dedup unit tests: replace the `resolved_dep` helper with

```rust
fn discovered(guid: &str, name: &str, version: &str, path: &str) -> DiscoveredApp {
    DiscoveredApp {
        app_path: PathBuf::from(path),
        meta: crate::app_package::AppMetadata {
            app_id: guid.to_string(),
            name: name.to_string(),
            publisher: "Pub".to_string(),
            version: version.to_string(),
            runtime: String::new(),
            platform: String::new(),
            application: String::new(),
            dependencies: vec![],
            internals_visible_to: vec![],
        },
    }
}
```

and update the five H-2 dedup tests' assertions accordingly
(`kept[0].dependency.version` → `kept[0].meta.version`, etc. — the scenarios
and expected values are untouched).

- [ ] **Step 5: Reorder `load_all_apps` to manifest-first**

Replace the discovery loop's body (`match extract_app_package(&path)` arm,
line ~423) and the post-loop dedup with the three-phase shape:

```rust
// Phase 1: manifest-only discovery — never touches SymbolReference.json.
let mut discovered: Vec<DiscoveredApp> = Vec::new();
// ... same folders / read_dir / extension / canonical-path-dedup skeleton,
//     with the innermost match becoming:
match crate::app_package::extract_app_metadata(&path) {
    Ok(meta) => {
        debug!(
            "load_all_apps: discovered {} v{} from {}",
            meta.name,
            meta.version,
            alpackages.display()
        );
        discovered.push(DiscoveredApp {
            app_path: path,
            meta,
        });
    }
    Err(e) => {
        warn!("load_all_apps: failed to read manifest of {}: {}", path.display(), e);
    }
}

// Phase 2: GUID dedup on manifest identity (H-2) — losers are dropped
// HERE, before the expensive symbol parse below ever sees them.
let (winners, dropped) = dedup_by_guid_keep_highest_version(discovered);

// Phase 3: full symbol extraction, winners only.
let mut out: Vec<ResolvedDependency> = Vec::new();
for d in winners {
    match crate::app_package::extract_app_symbols(&d.app_path) {
        Ok(objects) => {
            debug!(
                "load_all_apps: loaded {} v{} ({} objects)",
                d.meta.name,
                d.meta.version,
                objects.len()
            );
            out.push(ResolvedDependency {
                dependency: AppDependency {
                    app_id: d.meta.app_id.clone(),
                    name: d.meta.name.clone(),
                    publisher: d.meta.publisher.clone(),
                    version: d.meta.version.clone(),
                },
                app_path: d.app_path,
                package: ParsedAppPackage {
                    metadata: d.meta,
                    objects,
                },
            });
        }
        Err(e) => {
            warn!("load_all_apps: failed to parse {}: {}", d.app_path.display(), e);
        }
    }
}
```

The trailing deterministic sort block is UNCHANGED (it sorts `out`). The
`AppDependency` values above are built from `meta` exactly as the old code
built them from `package.metadata` — same source, read one phase earlier.
`ParsedAppPackage` construction requires its fields to be visible here — they
already are (`ParsedAppPackage` is `pub` with `pub` fields; the tests'
`resolved_dep` helper constructed it the same way).

- [ ] **Step 6: Run the new test + the H-2 suites**

Run: `cargo test --lib dependencies` and `cargo test --lib snapshot::snapshot`
Expected: PASS — the new ordering test, the retyped pure dedup tests, and the
end-to-end `build_with_diagnostics_*` H-2 fixtures (which prove the
`SnapshotBuilder` pipeline still sees identical dedup outcomes).

- [ ] **Step 7: Full validation**

Run: `cargo test` then `cargo clippy --all-targets --all-features`
Expected: all green.

- [ ] **Step 8: Format, changelog, commit**

`rustfmt` touched files. `CHANGELOG.md`: `Changed` — `load_all_apps` is
manifest-first (GUID dedup before symbol extraction; duplicate Base App /
System App copies across ancestor `.alpackages` no longer pay a
SymbolReference parse each), including the corrupt-winner behavior note from
this task's header.

```bash
git add src/app_package.rs src/dependencies.rs CHANGELOG.md
git commit -m "perf: dedup .app packages on manifest identity before parsing SymbolReference

Co-authored-by: Copilot <223556219+Copilot@users.noreply.github.com>"
```

---

### Task 4: Close-out — measurement + perf doc update

**Files:**
- Modify: `docs/perf-regression-t3-vs-0.9.3.md`
- Test: none (documentation + measurement only; per repo rules doc changes need no build)

**Interfaces:**
- Consumes: Tasks 1–3 landed.
- Produces: an updated regression doc so the next reader doesn't re-diagnose fixed problems.

- [ ] **Step 1: Benchmark sanity on the synthetic corpus**

Run: `cargo bench --bench lsp_pipeline -- build_full`
Expected: `build_full` medians at or below the CLAUDE.md table
(~8 ms / ~74 ms); rung-1/rung-2 benches should IMPROVE (rung 2 lost its
every-file re-parse). Record the numbers.

- [ ] **Step 2: Real-workspace measurement (requires access to the baseline workspace)**

If `U:\Git\DO.Support-SlowDOSetup\DocumentOutput\Cloud` is reachable:

```powershell
cargo build --release --bin al-call-hierarchy
# CLI wall time (was 3.69-4.04 s)
Measure-Command { .\target\release\al-call-hierarchy.exe --project U:\Git\DO.Support-SlowDOSetup\DocumentOutput\Cloud }
# Peak RSS (was 1,869 MB CLI / ~2,000 MB LSP steady state)
python scripts/peak_rss.py .\target\release\al-call-hierarchy.exe --project U:\Git\DO.Support-SlowDOSetup\DocumentOutput\Cloud
```

Expected direction: LSP cold start toward CLI parity (§3.2 predicted
5.1 s → ~4 s, minus Task 3's savings), steady-state RSS down by roughly two
dep-text copies (~228 MB) + one full dep arena+text set. Dep IR arenas are
STILL retained once by the updater — that is §3.3 (stable dep identity),
explicitly out of scope here.

- [ ] **Step 3: Update the regression doc**

Append a dated "Mitigations 1, 2, 5 — IMPLEMENTED" section to
`docs/perf-regression-t3-vs-0.9.3.md` recording: what landed (one-line each,
with commit hashes), the re-measured numbers from Steps 1–2, and that the
remaining gap is owned by §3.3(a)/§4-item-3 (graph-independent dep decl
identity) and §4-item-4 (persistent dep-layer artifact cache).

- [ ] **Step 4: Commit**

```bash
git add docs/perf-regression-t3-vs-0.9.3.md
git commit -m "docs: record implemented perf safe-wins and re-measured numbers

Co-authored-by: Copilot <223556219+Copilot@users.noreply.github.com>"
```
