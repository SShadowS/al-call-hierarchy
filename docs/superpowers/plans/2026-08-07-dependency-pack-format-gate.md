# Dependency Pack — Format Gate Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the serialization surface for dependency declaration nodes and measure round-trip load cost on real data, so the dependency pack cache's format question is answered by measurement rather than prediction.

**Architecture:** Steps 0, 2 and 3 of `docs/superpowers/specs/2026-08-07-dependency-pack-cache-design.md`. Preparatory doc/dead-code corrections, then a `&'static str` → enum change that currently makes the node types unserializable at all, then serde derives across the node closure, then a self-contained pack codec, then a benchmark on a real workspace that decides whether the record-per-routine format proceeds or is replaced by a shared string table.

**Tech Stack:** Rust, serde, postcard (new dependency), blake3 (already present), rayon (already present), criterion (already present for benches).

## Global Constraints

- Format touched files with `rustfmt <file>`. **Never `cargo fmt`** — whole-crate churn.
- Lint bar is `scripts/ci-steps clippy` (release + `-D warnings`). Clippy must be clean at every commit.
- Package name for test filters is `al-call-hierarchy` (hyphen). `al_call_hierarchy` fails.
- Stage only intended paths. **Never `git add -A`.**
- `CHANGELOG.md` must be updated for feature additions, bug fixes and breaking changes, in [Keep a Changelog](https://keepachangelog.com/) format under Added/Changed/Deprecated/Removed/Fixed/Security.
- A test must pin the USE, not just the helper, and you must **prove it can fail** — break the thing, watch the test fail, revert, watch it pass, and record both outcomes. Assert that a scripted break actually applied (e.g. `assert s.count(old) == 1`); an unasserted break that comes back green proves nothing.
- Work happens on branch `feat/dep-pack-format-gate`. Never commit to `master`.
- **Benches must be clippy-clean.** CI does not *run* benches, but `scripts/ci-steps clippy` uses `--all-targets`, which compiles every `benches/*.rs` under `-D warnings` (see `.github/workflows/ci.yml:41-53` — the comment there records that missing `--all-targets` once left every test and bench target completely unlinted). A bench that only compiles under `cargo bench` will fail CI.
- **Reverting a discrimination-proof break: copy the file to a backup first and restore from that.** Never `git checkout <file>` to undo a break — within a task the file holds uncommitted work from earlier steps, and a newly created file is untracked, so `git checkout` either destroys work or errors.

## Scope

**In this plan:** spec steps 0, 2, 3 — the path to the go/no-go answer.

**Deliberately NOT in this plan:**

- **Spec step 1 (light snapshot)** is independently valuable and independently shippable — it cuts `preflight.snapshot_build` (458.8 ms on DO) for the already-shipped verdict cache regardless of whether packs ever exist. It also carries an unresolved design question found while writing this plan (see below). It gets its own plan and can proceed in parallel; nothing here depends on it.
- **Spec steps 4, 5, 6** (fingerprint, pack seam, equivalence gate) are contingent on Task 6's outcome and on a format choice that does not exist until Task 6 runs. Writing them now would be writing against a format the gate may reject.

**The open question for the light-snapshot plan, recorded here so it is not lost:** `EmbeddedAppProvider::try_provide` returns `Ok(None)` when the extracted file list is empty, and that is how a symbol-only app is distinguished from a source-bearing one (`src/snapshot/provider.rs:87-100`). A light path that skips source materialization therefore cannot tell the two apart without opening the `.app`. Resolving this likely means a cheap archive-directory probe that answers "does this `.app` contain source entries?" without reading their text. Do not assume `app_content_hash` answers it — it does not.

---

## File Structure

| File | Responsibility | Status |
|---|---|---|
| `src/engine/deps/symbol_reference.rs` | `SubtypeTag` enum replaces the `&'static str` tag | Modify |
| `src/program/node_extract.rs` | serde derives on the node types; `AbiParamRetained.subtype_tag` type change | Modify |
| `src/program/resolve/event.rs` | serde derives on `PublisherKind`, `ParsedSubscriberArgs` | Modify |
| `src/program/resolve/edge.rs` | serde derives on `AbiRoutineKind`, `AbiEventKind` | Modify |
| `src/snapshot/identity.rs` | serde derives on `TrustTier` | Modify |
| `crates/al-syntax/src/ir/mod.rs` | delete the dead `Origin.ts_id` field | Modify |
| `crates/al-syntax/src/lower/mod.rs` | drop the single `ts_id` write site | Modify |
| `src/program/pack/mod.rs` | **new** — `DepPack`/`PackedFile`/`PackedOrigin`, encode, decode, integrity checks | Create |
| `src/program/resolve/decl_surface.rs` | serde on `RoutineMeta` / `ParamMeta` | Modify |
| `crates/al-syntax/src/raw/generated/raw_kind.rs` | `try_from_raw` — fallible sibling of the panicking `from_raw` | Modify (generated — see Task 5) |
| `benches/dep_pack_roundtrip.rs` | **new** — the measurement gate | Create |
| `CLAUDE.md` | three doc corrections | Modify |
| `src/program/resolve/preflight_cache.rs` | one doc correction | Modify |
| `docs/2026-08-07-dep-pack-gate-measurement.md` | **new** — the gate's measurement ledger and verdict | Create |

`src/program/pack/mod.rs` is a new module deliberately kept separate from `node_extract.rs`: the extraction logic and the persistence format have different reasons to change, and the spec's `EXTRACTION_FINGERPRINT` closure (§8) must be able to name the codec module separately.

---

## Task 1: Documentation corrections

Spec §17.1, §17.2, §17.4. Pure documentation. No behaviour changes, no tests — the deliverable is that three false claims stop being false.

**Files:**
- Modify: `src/program/resolve/preflight_cache.rs:66-70`
- Modify: `CLAUDE.md:210`
- Modify: `CLAUDE.md:155`, `CLAUDE.md:292`

**Interfaces:**
- Consumes: nothing.
- Produces: nothing. No later task depends on this.

- [ ] **Step 1: Confirm each claim is still false before changing it**

```bash
cd U:/Git/al-call-hierarchy
# §17.1 — pruning genuinely absent from the preflight cache.
# Exclude doc-comment lines: three of them mention `cache prune` /
# `engine/gate/cache_prune.rs`, which is a DIFFERENT, pre-existing module and
# says nothing about whether THIS cache prunes.
grep -nE 'prune|max_age|max_size|remove_file' src/program/resolve/preflight_cache.rs \
  | grep -v '^\s*[0-9]*:\s*///'
# Expected: exactly ONE line, the failed-rename cleanup
# `let _ = std::fs::remove_file(&tmp);`. If you run the unfiltered grep you will
# see FOUR hits — three doc-comment mentions plus that one. Four is the correct
# and expected result of the unfiltered command; it does NOT mean pruning exists.

# §17.2 — no thread-local parsers anywhere
grep -rn 'thread_local' --include=*.rs src/ crates/
# Expected: exactly two hits, src/engine/perf_trace.rs and
# src/engine/gate/policy/predicate_evaluator.rs — neither a parser

# §17.4 — the .scm files are not at the claimed path
ls queries/highlights.scm 2>&1        # Expected: No such file or directory
ls tree-sitter-al/queries/highlights.scm  # Expected: the file
```

If any expectation does not hold, STOP and report — the defect was fixed by someone else and this task needs re-scoping.

- [ ] **Step 2: Correct the preflight cache's over-claim (§17.1)**

In `src/program/resolve/preflight_cache.rs`, the doc on `ENV_CACHE_DIR` currently reads:

```rust
/// Env var pointing the cache at a specific directory. **Required for tests** —
/// without it every test would share one global mutable directory, which is
/// exactly the pre-existing defect in `snapshot::cache` (no override, no
/// pruning) that this module deliberately does not copy.
```

Replace with:

```rust
/// Env var pointing the cache at a specific directory. **Required for tests** —
/// without it every test would share one global mutable directory, which is
/// half of the pre-existing defect in `snapshot::cache` (no override, no
/// pruning).
///
/// This module fixes the OVERRIDE half and, as of today, does NOT fix the
/// pruning half: there is no size or age bound and the directory grows without
/// limit. `docs/superpowers/specs/2026-08-01-preflight-verdict-cache.md` §4
/// asked for size/age-bounded pruning and it was never implemented. Known debt,
/// stated here rather than implied away — an earlier revision of this doc
/// claimed the whole defect was "deliberately not copied", which was false for
/// the pruning half.
```

- [ ] **Step 3: Correct the thread-local parser claim (§17.2)**

In `CLAUDE.md`, under Core Patterns, line 210 currently reads:

```markdown
- **Parallel parsing** (`rayon`): Thread-local parsers process files concurrently
```

Replace with:

```markdown
- **Parallel parsing** (`rayon`): files are parsed concurrently via `par_iter` on a
  dedicated pool (`crate::big_stack::big_stack_pool`, sized for the `al-syntax`
  lowerer's recursion — see `src/snapshot/parse.rs:84-95`). There are **no**
  thread-local parsers: `al_syntax::parse` constructs a fresh
  `tree_sitter::Parser` and calls `set_language` per file
  (`crates/al-syntax/src/parse.rs:11-14`). Reusing a parser per worker thread is
  an open, unmeasured optimisation — see the pack-cache spec §17.5.
```

- [ ] **Step 4: Correct the `.scm` paths (§17.4)**

In `CLAUDE.md`, replace `queries/highlights.scm` with `tree-sitter-al/queries/highlights.scm` and `queries/tags.scm` with `tree-sitter-al/queries/tags.scm`, at both line 155 and line 292. There are exactly four occurrences across the two lines.

Verify none remain unqualified:

```bash
grep -n 'queries/highlights.scm\|queries/tags.scm' CLAUDE.md | grep -v 'tree-sitter-al/queries/'
# Expected: no output
```

- [ ] **Step 5: Verify the paths now resolve**

```bash
cd U:/Git/al-call-hierarchy
grep -oE '`(src|crates|scripts|tests|benches|queries|tree-sitter-al)/[A-Za-z0-9_./-]+`' CLAUDE.md \
  | tr -d '`' | sed 's/[.,)]$//' | sort -u \
  | while read -r p; do [ -e "$p" ] || echo "MISSING: $p"; done
```

Expected: no output.

- [ ] **Step 6: Format and commit**

```bash
cd U:/Git/al-call-hierarchy
rustfmt src/program/resolve/preflight_cache.rs
git add CLAUDE.md src/program/resolve/preflight_cache.rs
git commit -F - <<'EOF'
docs: correct three false claims found in the pack-cache rot sweep

- preflight_cache.rs claimed it did not copy snapshot::cache's
  "(no override, no pruning)" defect. It fixed the override half only;
  pruning was never implemented despite its own spec asking for it.
- CLAUDE.md claimed thread-local parsers. Repo-wide, thread_local appears
  only in perf_trace.rs and predicate_evaluator.rs, neither a parser;
  al_syntax::parse builds a fresh Parser per file. The rayon half was true
  and is kept, now with the pool named.
- CLAUDE.md's queries/*.scm paths were missing the tree-sitter-al/ prefix.

See docs/superpowers/specs/2026-08-07-dependency-pack-cache-design.md
sections 17.1, 17.2 and 17.4.
EOF
```

---

## Task 2: Delete the dead `Origin.ts_id` field

Spec §17.3. The compiler is the test: if anything still reads the field, this will not build.

**Files:**
- Modify: `crates/al-syntax/src/ir/mod.rs:43`
- Modify: `crates/al-syntax/src/lower/mod.rs:1909`
- Modify: `src/program/resolve/arg_dispatch.rs:2197`, `src/program/resolve/receiver.rs:3172`, `src/program/sig_fp.rs:180` (three `ts_id: 0` fixture constructions)
- Modify: `tests/lsp/lsp_incremental_parity.rs` (doc comments only)

**Interfaces:**
- Consumes: nothing.
- Produces: `Origin` without a `ts_id` field. Task 4 derives `Serialize`/`Deserialize` on types reachable from `Origin`, so this must land first — otherwise Task 4 would serialize a field the spec says must never be serialized.

- [ ] **Step 1: Re-verify the field is dead**

```bash
cd U:/Git/al-call-hierarchy
grep -rn 'ts_id' --include=*.rs src/ crates/ tests/
```

Expected: exactly the sites listed above — one declaration, one write, three `ts_id: 0` fixture literals, and doc-comment mentions in `tests/lsp/lsp_incremental_parity.rs`. **No production read.** If a read exists, STOP and report.

- [ ] **Step 2: Delete the field declaration**

In `crates/al-syntax/src/ir/mod.rs`, remove the `pub ts_id: usize,` field from `Origin` together with its doc comment (the one saying "NEVER serialize… tree-sitter recycles ids" and naming the L2 op/callsite maps).

- [ ] **Step 3: Delete the write site**

In `crates/al-syntax/src/lower/mod.rs` around line 1909, remove the `ts_id: n.id(),` line from the `Origin` construction.

- [ ] **Step 4: Delete the three fixture literals**

Remove `ts_id: 0,` from the `Origin` constructions in `src/program/resolve/arg_dispatch.rs`, `src/program/resolve/receiver.rs`, and `src/program/sig_fp.rs`.

- [ ] **Step 5: Update the parity test's doc comments**

`tests/lsp/lsp_incremental_parity.rs` documents projecting away `kind_text`/`ts_id` at lines 114-118, 146 and 266. The projection code itself does not name `ts_id` (it constructs the kept tuple), so only prose changes. Rewrite each mention to name only `kind_text`, and add one sentence at the first mention:

```rust
//! (`ts_id` was deleted outright — it was written once by the lowerer and read
//! by no production code; see the pack-cache spec §17.3.)
```

- [ ] **Step 6: Build everything, including test targets**

```bash
cd U:/Git/al-call-hierarchy
cargo check --all-targets
```

Expected: clean. `--all-targets` matters — `--bins` alone misses test targets, and three of the five edits are in test-adjacent code.

- [ ] **Step 7: Run the affected suites**

```bash
cargo test -p al-call-hierarchy --lib
cargo test --test lsp
```

Expected: PASS.

- [ ] **Step 8: Lint**

```bash
scripts/ci-steps clippy
```

Expected: clean.

- [ ] **Step 9: Commit**

```bash
cd U:/Git/al-call-hierarchy
rustfmt crates/al-syntax/src/ir/mod.rs crates/al-syntax/src/lower/mod.rs \
        src/program/resolve/arg_dispatch.rs src/program/resolve/receiver.rs \
        src/program/sig_fp.rs
git add crates/al-syntax/src/ir/mod.rs crates/al-syntax/src/lower/mod.rs \
        src/program/resolve/arg_dispatch.rs src/program/resolve/receiver.rs \
        src/program/sig_fp.rs tests/lsp/lsp_incremental_parity.rs
git commit -F - <<'EOF'
refactor(al-syntax): delete the dead Origin.ts_id field

Written once at lower/mod.rs:1909, read by zero production code. Its doc
said "NEVER serialize... tree-sitter recycles ids" and named the L2
op/callsite maps as its consumer; that consumer no longer exists, so the
stated blocker was guarding a dead field.

Removed ahead of the dependency pack cache's serialization work so that
work never has to decide what to do with a field that must not be
serialized. See pack-cache spec section 17.3.
EOF
```

---

## Task 3: Replace `subtype_tag: &'static str` with a `SubtypeTag` enum

**This is a blocker, not a cleanup.** `&'static str` cannot implement `Deserialize` — you cannot produce a `'static` borrow from arbitrary input — so `AbiParamRetained`, and therefore `RoutineNode`, cannot be deserialized at all while this field exists in its current form. The value set is closed and documented at `src/engine/deps/symbol_reference.rs:80`, so it was already an enum in everything but type.

**Files:**
- Modify: `src/engine/deps/symbol_reference.rs:80-85` (the field and its doc), `:759-774` (the construction site), `:1540`, `:1583-1584`, `:1615`, `:1634` (test assertions)
- Modify: `src/engine/deps/projection.rs:424`
- Modify: `src/program/node_extract.rs:183` (`AbiParamRetained.subtype_tag`)
- Test: `src/engine/deps/symbol_reference.rs` (inline `mod tests`)

**Interfaces:**
- Consumes: nothing.
- Produces: `pub enum SubtypeTag { NoName, NoSubtype, Full, NameQuoted, IdOnly }` exported from `src/engine/deps/symbol_reference.rs`, deriving `Copy, Clone, Debug, PartialEq, Eq`. Task 4 adds `Serialize, Deserialize` to it. Both `AbiParameter::subtype_tag` and `AbiParamRetained::subtype_tag` become `SubtypeTag`.

- [ ] **Step 1: Confirm the closed value set**

```bash
cd U:/Git/al-call-hierarchy
sed -n '78,86p' src/engine/deps/symbol_reference.rs
grep -rn 'subtype_tag' --include=*.rs src/ | grep -vE 'assert|^\S+:[0-9]+:\s*///'
```

Expected: the doc lists exactly `"no_name"`, `"no_subtype"`, `"full"`, `"name_quoted"`, `"id_only"`, and the only value-producing sites are `symbol_reference.rs:759-774` and `projection.rs:424`. If a sixth value exists, add it to the enum in Step 3 and note it in the commit.

- [ ] **Step 2: Write the failing test**

Add to the inline `mod tests` in `src/engine/deps/symbol_reference.rs`. This pins the USE — that the real parse path produces each tag — rather than testing a standalone converter:

```rust
#[test]
fn subtype_tag_is_a_closed_enum_over_the_real_parse_path() {
    // Hand-stated preconditions: one raw parameter shape per documented tag.
    // Constructed literally so this survives any change to how the parser
    // reaches these shapes.
    let no_subtype = parse_param_json(r#"{"Name":"p","TypeDefinition":{"Name":"Integer"}}"#);
    assert_eq!(no_subtype.subtype_tag, SubtypeTag::NoSubtype);

    let full = parse_param_json(
        r#"{"Name":"p","TypeDefinition":{"Name":"Record","Subtype":{"Id":18,"Name":"Customer"}}}"#,
    );
    assert_eq!(full.subtype_tag, SubtypeTag::Full);

    let id_only = parse_param_json(
        r#"{"Name":"p","TypeDefinition":{"Name":"Record","Subtype":{"Id":18}}}"#,
    );
    assert_eq!(id_only.subtype_tag, SubtypeTag::IdOnly);

    let name_quoted = parse_param_json(
        r#"{"Name":"p","TypeDefinition":{"Name":"Record","Subtype":{"Name":"Sales Header"}}}"#,
    );
    assert_eq!(name_quoted.subtype_tag, SubtypeTag::NameQuoted);
}
```

If no `parse_param_json` helper exists in that test module, write one that deserializes a single `AbiParameter` from the JSON text using the module's existing parsing entry point, and place it directly above this test. Do not invent a second parsing path — call the same code production calls.

- [ ] **Step 3: Run the test to verify it fails**

```bash
cargo test -p al-call-hierarchy --lib subtype_tag_is_a_closed_enum -- --nocapture
```

Expected: FAIL to compile — `SubtypeTag` does not exist.

- [ ] **Step 4: Add the enum**

In `src/engine/deps/symbol_reference.rs`, above `AbiParameter`:

```rust
/// The provenance of an ABI parameter's `Subtype`, as a closed set.
///
/// Was a `&'static str` carrying exactly these five values (documented, and
/// asserted in this module's tests). Changed to an enum because `&'static str`
/// cannot implement `Deserialize` — there is no way to produce a `'static`
/// borrow from arbitrary input — which made every struct reaching this field
/// unserializable. See the dependency pack cache spec, Task 3.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SubtypeTag {
    /// Outer type name absent.
    NoName,
    /// No `Subtype` object at all — bare pass-through.
    NoSubtype,
    /// Both `Subtype.Id` and `Subtype.Name` present.
    Full,
    /// `Subtype.Name` present without `Subtype.Id`; text degrades to the bare
    /// outer keyword.
    NameQuoted,
    /// `Subtype.Id` present without `Subtype.Name`.
    IdOnly,
}
```

- [ ] **Step 5: Change the field types**

In `src/engine/deps/symbol_reference.rs`, change `pub subtype_tag: &'static str,` to `pub subtype_tag: SubtypeTag,` and update its doc line 80 from the string list to "See [`SubtypeTag`]."

In `src/program/node_extract.rs`, change `pub subtype_tag: &'static str,` to `pub subtype_tag: SubtypeTag,` and add the import:

```rust
use crate::engine::deps::symbol_reference::SubtypeTag;
```

- [ ] **Step 6: Update the two construction sites**

In `src/engine/deps/symbol_reference.rs` around lines 759-774, replace each string literal in the returned tuple: `"no_subtype"` → `SubtypeTag::NoSubtype`, `"full"` → `SubtypeTag::Full`, `"name_quoted"` → `SubtypeTag::NameQuoted`, `"id_only"` → `SubtypeTag::IdOnly`. If a `"no_name"` arm exists, use `SubtypeTag::NoName`.

In `src/engine/deps/projection.rs:424`, replace `subtype_tag: "no_subtype",` with `subtype_tag: SubtypeTag::NoSubtype,` and add the import.

- [ ] **Step 7: Update the existing assertions**

In `src/engine/deps/symbol_reference.rs`, replace `assert_eq!(p.subtype_tag, "full")` with `assert_eq!(p.subtype_tag, SubtypeTag::Full)` and likewise at lines 1583, 1584, 1615, 1634.

- [ ] **Step 8: Run the tests to verify they pass**

```bash
cargo test -p al-call-hierarchy --lib subtype_tag -- --nocapture
cargo check --all-targets
```

Expected: PASS, clean build.

- [ ] **Step 9: Prove the new test discriminates**

Required by the Global Constraints. Break the mapping, confirm the test catches it, revert.

```bash
cd U:/Git/al-call-hierarchy
export BREAK_BACKUP=/tmp/symbol_reference.rs.bak
cp src/engine/deps/symbol_reference.rs "$BREAK_BACKUP"
python - <<'PY'
import pathlib
p = pathlib.Path("src/engine/deps/symbol_reference.rs")
s = p.read_text(encoding="utf-8")
old = "Some(id) => (outer_name.to_string(), Some(id), None, SubtypeTag::IdOnly)"
assert s.count(old) == 1, f"scripted break did not match: {s.count(old)} occurrences"
p.write_text(s.replace(old, old.replace("SubtypeTag::IdOnly", "SubtypeTag::Full"), 1), encoding="utf-8")
print("break applied")
PY
cargo test -p al-call-hierarchy --lib subtype_tag_is_a_closed_enum
```

If the `assert` fires, the construction site's text differs from what this plan
recorded — read the real arm at `src/engine/deps/symbol_reference.rs:774`, adapt the
`old` string to match it exactly, and re-run. Do **not** relax the assertion to a
`>=` or drop it; an unasserted break that comes back green proves nothing.

Expected: **FAIL**, on the `IdOnly` assertion. Record the failure output. Then revert **from the backup, not from git** — Steps 4-7's work in this file is uncommitted, so `git checkout` would destroy it:

```bash
cd U:/Git/al-call-hierarchy
cp "$BREAK_BACKUP" src/engine/deps/symbol_reference.rs
rm "$BREAK_BACKUP"
cargo test -p al-call-hierarchy --lib subtype_tag_is_a_closed_enum
git diff --stat src/engine/deps/symbol_reference.rs   # Steps 4-7's edits must still be present
```

Expected: PASS, and the file still shows Steps 4-7's changes. If the break came back GREEN, the test is defective — do not proceed; the scripted assertion above guarantees the edit applied, so a green run means the assertion is not reaching the code path.

- [ ] **Step 10: Lint and commit**

```bash
cd U:/Git/al-call-hierarchy
rustfmt src/engine/deps/symbol_reference.rs src/engine/deps/projection.rs src/program/node_extract.rs
scripts/ci-steps clippy
git add src/engine/deps/symbol_reference.rs src/engine/deps/projection.rs src/program/node_extract.rs
git commit -F - <<'EOF'
refactor(deps): SubtypeTag enum replaces the &'static str subtype tag

&'static str cannot implement Deserialize -- there is no way to produce a
'static borrow from arbitrary input -- so AbiParamRetained, and therefore
RoutineNode, could not be deserialized at all while this field kept its
string type. The value set was already closed and documented
(no_name | no_subtype | full | name_quoted | id_only), so this was an enum
in everything but type.

Prerequisite for the dependency pack cache's serialization surface.
Discrimination proof recorded: flipping the id_only arm to Full fails the
new test; reverting passes it.
EOF
```

---

## Task 4: Serialization surface for the node closure

**Files:**
- Modify: `src/program/node_extract.rs` (`ObjectRef`, `PageControlKind`, `PageControlNode`, `DataitemNode`, `FieldNode`, `ObjectNode`, `AbiParamRetained`, `AbiParams`, `Access`, `RoutineNode`)
- Modify: `src/program/resolve/event.rs` (`ParsedSubscriberArgs`, `PublisherKind`)
- Modify: `src/program/resolve/edge.rs` (`AbiRoutineKind`, `AbiEventKind`)
- Modify: `src/snapshot/identity.rs` (`TrustTier`)
- Modify: `src/engine/deps/symbol_reference.rs` (`SubtypeTag`, and `AbiEventKind` if the definition there is the one reachable from `RoutineNode`)
- Test: `src/program/node_extract.rs` (inline `mod tests`)

**Interfaces:**
- Consumes: `SubtypeTag` from Task 3; `Origin` without `ts_id` from Task 2.
- Produces: `ObjectNode` and `RoutineNode` both implement `serde::Serialize` and `serde::Deserialize<'de>`. Task 5 depends on this and on nothing else from this task.

Already carrying serde and needing no change: `AppRef` and `ObjKey` (`src/program/node.rs:9-10`, `:75-76`), `RoutineNodeId` and `ObjectNodeId` (`node.rs:117-122`, `:173-174`, the latter via the `sig_fp_as_string` helper and `ObjectKindDef` remote derive).

- [ ] **Step 1: Let the compiler enumerate the closure — do not work from a list**

**Read this before starting.** Earlier tasks in this plan shipped three separate list-based enumerations that were all wrong: a file list that named 3 files where 9 were required, a value set that named 5 variants where 8 existed, and a name-based grep that resolved two type names to the wrong definitions. Name-matching does not trace reachability. The compiler does.

So the list below is a **starting hint, explicitly not authoritative**:

> `ObjectRef`, `PageControlKind`, `PageControlNode`, `DataitemNode`, `FieldNode`, `ObjectNode`, `AbiParamRetained`, `AbiParams`, `Access`, `RoutineNode`, `ParsedSubscriberArgs`, `PublisherKind`, `AbiRoutineKind`, `AbiEventKind`, `TrustTier`, `SubtypeTag`

**Two known traps, both of which a grep by name gets wrong:**

- `PageControlKind` has two definitions — `src/program/node_extract.rs:54` and `src/engine/l3/l3_workspace.rs:116`. Only the first is reachable from `ObjectNode`.
- `AbiEventKind` likewise has two — `src/program/resolve/edge.rs` and `src/engine/deps/symbol_reference.rs`. Follow `RoutineNode::abi_event_kind`'s actual type to see which.
- `ObjectKind` needs **nothing**: `ObjectNodeId` already routes it through a `#[serde(with = "ObjectKindDef")]` remote derive (`src/program/node.rs:117-122`).

**The method:** add `Serialize, Deserialize` to `ObjectNode` and `RoutineNode` first, then run

```bash
cd U:/Git/al-call-hierarchy
cargo check --all-targets 2>&1 | grep -E "^error|the trait bound" | head -40
```

Each error names the next type missing a derive. Add it, re-run, repeat until clean. **`cargo check --all-targets` is the completeness oracle** — `--all-targets` is load-bearing, because `tests/` files reference these types too and a `src/`-scoped search will never find them. Task 3 discovered three such files exactly this way.

Record in your report the full list the compiler actually demanded, and call out explicitly any type the hint above missed or named wrongly.

- [ ] **Step 2: Write the failing round-trip test**

Add to the inline `mod tests` in `src/program/node_extract.rs`. This states its precondition literally — a hand-built node carrying every optional field populated — so it survives any change to how extraction produces one:

```rust
#[test]
fn routine_node_survives_a_json_round_trip_with_every_field_populated() {
    let node = RoutineNode {
        id: RoutineNodeId {
            object: ObjectNodeId {
                app: crate::program::node::AppRef(7),
                kind: al_syntax::ir::ObjectKind::Codeunit,
                key: crate::program::node::ObjKey::Id(50100),
            },
            name_lc: "doThing".to_ascii_lowercase(),
            enclosing_member_lc: Some("no.".to_string()),
            params_count: 2,
            sig_fp: 0xDEAD_BEEF_CAFE_F00D,
        },
        name: "DoThing".to_string(),
        is_trigger: false,
        access: Access::Internal,
        tier: TrustTier::EmbeddedSource,
        event_subscribers: vec![],
        subscriber_instance_manual: true,
        publisher_kind: None,
        include_sender: Some(false),
        abi_routine_kind: None,
        abi_event_kind: None,
        param_sig_key: "integer|code[20]".to_string(),
        return_type: Some("Codeunit \"Sales-Post\"".to_string()),
        return_type_id: Some(("Sales-Post".to_string(), 80)),
        abi_overload_collapsed: false,
        source_overload_aliased: true,
        abi_params: AbiParams::Complete(vec![AbiParamRetained {
            type_text: "Record".to_string(),
            is_var: true,
            subtype_id: Some(18),
            subtype_raw_name: Some("Customer".to_string()),
            subtype_tag: SubtypeTag::Full,
        }]),
    };

    let json = serde_json::to_string(&node).expect("serialize");
    let back: RoutineNode = serde_json::from_str(&json).expect("deserialize");

    // sig_fp specifically: it spans the full u64 range and JSON numbers are
    // IEEE-754 doubles, exact only to 2^53. RoutineNodeId already routes it
    // through a decimal string for this reason; assert the value survives.
    assert_eq!(back.id.sig_fp, 0xDEAD_BEEF_CAFE_F00D);
    assert_eq!(back.id, node.id);
    assert_eq!(back.name, node.name);
    assert_eq!(back.param_sig_key, node.param_sig_key);
    assert_eq!(back.return_type_id, node.return_type_id);
    assert_eq!(back.abi_params, node.abi_params);
    assert_eq!(back.tier, node.tier);
    assert_eq!(back.access, node.access);
}
```

`RoutineNode` does not derive `PartialEq` today; the assertions above compare fields that do. If you add `PartialEq` to `RoutineNode` to simplify this, that is acceptable — but it is a separate observable change, so mention it in the commit message.

- [ ] **Step 3: Run the test to verify it fails**

```bash
cargo test -p al-call-hierarchy --lib routine_node_survives_a_json_round_trip
```

Expected: FAIL to compile — `RoutineNode` does not implement `Serialize`.

- [ ] **Step 4: Add the derives**

For each type identified in Step 1, extend its existing derive list with `Serialize, Deserialize`. Do not reorder or remove existing derives. Example, `src/program/node_extract.rs`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutineNode {
```

Add the import once per file that needs it:

```rust
use serde::{Deserialize, Serialize};
```

`Access` and `PageControlKind` are `Copy` enums with unit variants — plain derives suffice. `AbiParams` is an enum with a `Vec` payload — plain derives suffice.

- [ ] **Step 5: Run the test to verify it passes**

```bash
cargo test -p al-call-hierarchy --lib routine_node_survives_a_json_round_trip
cargo check --all-targets
```

Expected: PASS, clean build.

- [ ] **Step 6: Prove the round-trip test discriminates on the field most likely to break silently**

`sig_fp` is the documented hazard — a JSON number would round it. Break the helper, confirm the test catches it, revert.

```bash
cd U:/Git/al-call-hierarchy
export BREAK_BACKUP=/tmp/node.rs.bak
cp src/program/node.rs "$BREAK_BACKUP"
python - <<'PY'
import pathlib
p = pathlib.Path("src/program/node.rs")
s = p.read_text(encoding="utf-8")
old = '#[serde(with = "sig_fp_as_string")]'
assert s.count(old) == 1, f"scripted break did not match: {s.count(old)} occurrences"
p.write_text(s.replace(old, ""), encoding="utf-8")
print("break applied")
PY
cargo test -p al-call-hierarchy --lib routine_node_survives_a_json_round_trip
```

Expected: **FAIL** on the `sig_fp` assertion, because `0xDEAD_BEEF_CAFE_F00D` exceeds 2^53 and round-trips through an IEEE-754 double. Record the output. Then:

```bash
cd U:/Git/al-call-hierarchy
cp "$BREAK_BACKUP" src/program/node.rs
rm "$BREAK_BACKUP"
cargo test -p al-call-hierarchy --lib routine_node_survives_a_json_round_trip
```

Expected: PASS. A green break here means the test value is too small to lose precision — raise it and repeat.

- [ ] **Step 7: Run the full golden gate**

Adding derives should change no behaviour, but `ObjectNode`/`RoutineNode` are reachable from several golden families and the repo's rule is to run them together, never one at a time.

```bash
cd U:/Git/al-call-hierarchy
scripts/check-goldens > /tmp/goldens.log 2>&1; echo "exit=$?"
grep -iE 'FAILED|test result: FAILED|error' /tmp/goldens.log | head -20
```

Expected: `exit=0`. **Do not pipe this to `tail`** — the pipeline's exit code would be `tail`'s, so a failure would read as success.

- [ ] **Step 8: Lint and commit**

```bash
cd U:/Git/al-call-hierarchy
rustfmt src/program/node_extract.rs src/program/resolve/event.rs \
        src/program/resolve/edge.rs src/snapshot/identity.rs \
        src/engine/deps/symbol_reference.rs
scripts/ci-steps clippy
git add src/program/node_extract.rs src/program/resolve/event.rs \
        src/program/resolve/edge.rs src/snapshot/identity.rs \
        src/engine/deps/symbol_reference.rs
git commit -F - <<'EOF'
feat(program): serde surface for ObjectNode / RoutineNode and their closure

Adds Serialize/Deserialize across the types reachable from the dependency
declaration nodes: ObjectRef, PageControlKind, PageControlNode,
DataitemNode, FieldNode, ObjectNode, AbiParamRetained, AbiParams, Access,
RoutineNode, ParsedSubscriberArgs, PublisherKind, AbiRoutineKind,
AbiEventKind, TrustTier, SubtypeTag. AppRef, ObjKey, ObjectNodeId and
RoutineNodeId already carried serde and are unchanged.

No behaviour change; goldens byte-stable via scripts/check-goldens.

Discrimination proof recorded: removing RoutineNodeId's
#[serde(with = "sig_fp_as_string")] fails the round-trip test on a sig_fp
above 2^53, which is exactly the precision loss that helper exists to
prevent; restoring it passes.

Prerequisite for the dependency pack codec (spec step 2).
EOF
```

---

## Task 5: The pack codec

A self-contained module that turns one dependency app's extraction output into bytes and back. No engine wiring — that is spec step 5, out of scope here. This exists so Task 6 has something real to measure.

**Files:**
- Create: `src/program/pack/mod.rs`
- Modify: `src/program/mod.rs` (add `pub mod pack;`)
- Modify: `Cargo.toml` (add `postcard`)
- Modify: `src/program/resolve/decl_surface.rs` — `Serialize, Deserialize` on `RoutineMeta` and `ParamMeta`
- Modify: `crates/al-syntax/src/raw/generated/raw_kind.rs` — add `try_from_raw`. **This file is GENERATED** (`cargo run -p xtask -- gen-syntax`); check whether the generator must emit `try_from_raw` instead of hand-editing, and if so change the generator. Report which you did and why.
- Test: `src/program/pack/mod.rs` (inline `mod tests`)

**The `Origin.kind_text` problem, and the design that solves it.** `RoutineMeta` carries two `Origin`s, and `Origin.kind_text` is `&'static str` (`crates/al-syntax/src/ir/mod.rs:38`) — which cannot implement `Deserialize`, the same blocker class Task 3 hit with `subtype_tag`.

Three rejected options and why, so they are not re-proposed:

- **`Origin.kind_text: RawKind` directly** — unsafe. `kind_str()` returns tree-sitter's raw string for ANY node, named or anonymous (`raw/node.rs:30-32`), while `RawKind::from_raw` covers only the 388 named kinds plus `ERROR` and **panics** on anything else (`raw_kind.rs:801-805`; `NAMED_KIND_COUNT` pinned at 388 in `raw/mod.rs:32`). Nothing type-level guarantees `origin_of` is only called on named nodes.
- **`kind_text: String`** — ~253k extra allocations per load (126,640 routines × 2 `Origin`s), paid on every warm run forever, to encode a value from a closed 389-member set. That is the precise cost the gate exists to price.
- **Omit `kind_text` from the pack** — `ir/mod.rs:38` documents a live consumer: it is "fed verbatim to anchor `syntax_kind` (parity)", and `syntax_kind: String` is real (`src/engine/deps/dep_artifact_l4.rs:944`). Making `RoutineMeta` lossy on an unaudited field trades a small known cost for an unbounded correctness audit.

**Do this instead:** leave `Origin` unchanged in the IR (its `&'static str` is load-bearing for the parity contract). Add a wire codec that encodes `kind_text` through a new **fallible** `RawKind::try_from_raw`, and decodes via `RawKind::as_str()` (`raw_kind.rs:809`) — which returns `&'static str`, so `Deserialize` is satisfied with **zero allocations** and the wire carries a varint. An encode-side `None` means the app is simply not packed (`PackError::Codec`), which is cold-path and fail-closed, mirroring spec §9. Never panic, and never fail on the read path.

**Interfaces:**
- Consumes: `ObjectNode`, `RoutineNode` with serde from Task 4.
- Produces:
  - `pub struct DepPack { pub schema: u32, pub app_guid: String, pub app_name: String, pub app_publisher: String, pub app_version: String, pub files: Vec<PackedFile>, pub self_hash: String }`
  - `pub struct PackedFile { pub virtual_path: String, pub parse_status_recovered: bool, pub objects: Vec<ObjectNode>, pub routines: Vec<RoutineNode>, pub routine_meta: Vec<(RoutineNodeId, RoutineMeta)> }`
  - `pub fn try_from_raw(s: &str) -> Option<RawKind>` on `RawKind` in `crates/al-syntax/src/raw/generated/raw_kind.rs` — a NON-panicking sibling of `from_raw`
  - a serde codec for `Origin` (see Step 4a) encoding `kind_text` as a `RawKind` discriminant
  - `pub fn encode(pack: &DepPack) -> Result<Vec<u8>, PackError>`
  - `pub fn decode(bytes: &[u8]) -> Result<DepPack, PackError>`
  - `pub enum PackError { Codec(String), SchemaMismatch { found: u32, expected: u32 }, SelfHashMismatch }`
  - `pub const PACK_SCHEMA: u32 = 1;`

**`RoutineMeta` IS in `PackedFile`.** An earlier revision of this plan deferred it to spec step 5 and told Task 6 to *estimate* its cost. That was wrong twice over:

- **It is not derivable on a hit.** `RoutineMeta::from_decl` consumes a `RoutineDecl` (`decl_surface.rs:43`), and `DeclSurface::build`/`build_split` obtain those by iterating `ParsedUnit`s (`decl_surface.rs:78-98`, `:115-146`) — which do not exist on a pack hit. Spec §4 says verbatim: "Packs persist nodes **and `RoutineMeta`**." Nothing in `RoutineNode` can reconstruct `virtual_path`, the two `Origin`s, or per-param `ty`/`by_ref`.
- **It is the exact cost class the gate exists to price.** Per routine it adds 1–3 heap `String`s, a `Vec<ParamMeta>` each with an `Option<String>`, and two `Origin`s. At 126,640 routines that is on the order of 400–600k extra allocations plus UTF-8 validation on every string — precisely what spec §13 identifies as separating decode cost from the clone analogy it rejected.

A gate that omits it is not a gate. It would be a lower bound requiring a re-run after the seam lands, and this plan's Goal states the question is "answered by measurement rather than prediction" — an estimate to complete the number contradicts that directly.

- [ ] **Step 1: Add the dependency**

In `Cargo.toml`, under `[dependencies]`:

```toml
postcard = { version = "1", features = ["use-std"] }
```

```bash
cargo fetch
```

- [ ] **Step 2: Write the failing test**

Create `src/program/pack/mod.rs` containing only this test module for now:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn sample_pack() -> DepPack {
        DepPack {
            schema: PACK_SCHEMA,
            app_guid: "437dbf0e-84ff-417a-965d-ed2bb9650972".to_string(),
            app_name: "Base Application".to_string(),
            app_publisher: "Microsoft".to_string(),
            app_version: "28.2.0.0".to_string(),
            files: vec![PackedFile {
                virtual_path: "src/Sales/SalesPost.Codeunit.al".to_string(),
                parse_status_recovered: true,
                objects: vec![],
                routines: vec![],
            }],
            self_hash: String::new(),
        }
    }

    #[test]
    fn encode_decode_round_trips() {
        let mut pack = sample_pack();
        pack.self_hash = compute_self_hash(&pack);
        let bytes = encode(&pack).expect("encode");
        let back = decode(&bytes).expect("decode");
        assert_eq!(back.app_guid, pack.app_guid);
        assert_eq!(back.files.len(), 1);
        assert_eq!(back.files[0].virtual_path, pack.files[0].virtual_path);
        assert!(back.files[0].parse_status_recovered);
    }

    #[test]
    fn a_corrupted_payload_is_rejected_not_misread() {
        let mut pack = sample_pack();
        pack.self_hash = compute_self_hash(&pack);
        let mut bytes = encode(&pack).expect("encode");
        // Flip one byte deep in the payload, past the schema prefix.
        let idx = bytes.len() / 2;
        bytes[idx] ^= 0xFF;
        match decode(&bytes) {
            Err(PackError::SelfHashMismatch) | Err(PackError::Codec(_)) => {}
            Err(other) => panic!("unexpected error: {other:?}"),
            Ok(_) => panic!("a corrupted pack decoded successfully — integrity check is absent"),
        }
    }

    #[test]
    fn a_pack_from_a_different_schema_is_rejected() {
        let mut pack = sample_pack();
        pack.schema = PACK_SCHEMA + 1;
        pack.self_hash = compute_self_hash(&pack);
        let bytes = encode(&pack).expect("encode");
        match decode(&bytes) {
            Err(PackError::SchemaMismatch { found, expected }) => {
                assert_eq!(found, PACK_SCHEMA + 1);
                assert_eq!(expected, PACK_SCHEMA);
            }
            other => panic!("expected SchemaMismatch, got {other:?}"),
        }
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail**

```bash
cargo test -p al-call-hierarchy --lib pack::
```

Expected: FAIL to compile — nothing in `super` exists.

- [ ] **Step 4: Implement the module**

Above the test module in `src/program/pack/mod.rs`:

```rust
//! Dependency pack codec — one app's extraction output, to bytes and back.
//!
//! Persistence only. No engine wiring lives here: obtaining a pack, keying it
//! and loading it into `build_dep_layer` is spec step 5. This module exists so
//! the format's round-trip cost can be measured before that work starts, per
//! `docs/superpowers/specs/2026-08-07-dependency-pack-cache-design.md` §13.
//!
//! Kept separate from `node_extract.rs` deliberately: extraction logic and the
//! persistence format have different reasons to change, and the spec's
//! EXTRACTION_FINGERPRINT closure (§8) names this module in its own right.

use serde::{Deserialize, Serialize};

use crate::program::node_extract::{ObjectNode, RoutineNode};

/// Bump when `DepPack`'s or `PackedFile`'s shape changes. Old packs then fail
/// the check in [`decode`] and are recomputed. Never migrate in place.
pub const PACK_SCHEMA: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackedFile {
    pub virtual_path: String,
    /// `AlFile::parse_status == ParseStatus::Recovered` for this file. Stored
    /// rather than recomputed: on a pack hit the file is never parsed, and
    /// `snapshot::parse::recovered_file_paths` is the load-bearing
    /// absence-proof diagnostic that must still see it (spec §11.2).
    pub parse_status_recovered: bool,
    pub objects: Vec<ObjectNode>,
    pub routines: Vec<RoutineNode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DepPack {
    pub schema: u32,
    /// App identity, SYMBOLIC. Never an `AppRef` — that is a per-run interning
    /// index (`src/program/build.rs:76-77`) and persisting one yields
    /// silently-wrong graphs rather than errors.
    pub app_guid: String,
    pub app_name: String,
    pub app_publisher: String,
    pub app_version: String,
    /// Per-file contributions in extraction order, pre-dedup. Order is
    /// load-bearing: `dedup_routines_preserving_genuine_overloads` keeps the
    /// first occurrence per key, so a reordered pack changes which survivor
    /// wins (spec §12).
    pub files: Vec<PackedFile>,
    /// blake3 over the postcard encoding of everything above. Cheap
    /// belt-and-suspenders against bit-rot and hand edits, and what makes the
    /// corrupted-payload test expressible.
    pub self_hash: String,
}

#[derive(Debug)]
pub enum PackError {
    Codec(String),
    SchemaMismatch { found: u32, expected: u32 },
    SelfHashMismatch,
}

impl std::fmt::Display for PackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PackError::Codec(e) => write!(f, "pack codec error: {e}"),
            PackError::SchemaMismatch { found, expected } => {
                write!(f, "pack schema {found}, expected {expected}")
            }
            PackError::SelfHashMismatch => write!(f, "pack self-hash mismatch"),
        }
    }
}

impl std::error::Error for PackError {}

/// blake3 over every field except `self_hash` itself.
#[must_use]
pub fn compute_self_hash(pack: &DepPack) -> String {
    let mut probe = pack.clone();
    probe.self_hash = String::new();
    let bytes = postcard::to_stdvec(&probe).unwrap_or_default();
    blake3::hash(&bytes).to_hex().to_string()
}

/// Serialize a pack. The caller must have set `self_hash` via
/// [`compute_self_hash`] first.
pub fn encode(pack: &DepPack) -> Result<Vec<u8>, PackError> {
    postcard::to_stdvec(pack).map_err(|e| PackError::Codec(e.to_string()))
}

/// Deserialize a pack, checking schema then integrity.
///
/// Every abnormal state is an `Err`, never a partial `Ok` — the consumer in
/// spec step 5 treats any `Err` as a cache miss and recomputes.
pub fn decode(bytes: &[u8]) -> Result<DepPack, PackError> {
    let pack: DepPack =
        postcard::from_bytes(bytes).map_err(|e| PackError::Codec(e.to_string()))?;
    if pack.schema != PACK_SCHEMA {
        return Err(PackError::SchemaMismatch {
            found: pack.schema,
            expected: PACK_SCHEMA,
        });
    }
    if compute_self_hash(&pack) != pack.self_hash {
        return Err(PackError::SelfHashMismatch);
    }
    Ok(pack)
}
```

Register the module in `src/program/mod.rs`:

```rust
pub mod pack;
```

- [ ] **Step 4a: Add `try_from_raw` and the `Origin` wire codec**

In `raw_kind.rs`, beside `from_raw`, add the non-panicking sibling. If the file is generated, change the generator and regenerate rather than hand-editing:

```rust
/// Fallible sibling of [`Self::from_raw`]. Returns `None` for an anonymous
/// token kind or an unknown string, where `from_raw` panics. Exists so a
/// persistence layer can FAIL TO STORE rather than crash on a kind it cannot
/// encode — see `src/program/pack`'s Origin codec.
pub fn try_from_raw(s: &str) -> Option<RawKind> { /* same match, `other => None` */ }
```

In `src/program/pack/mod.rs`, add the codec. `PackedOrigin` is the wire shape; conversion is fallible on encode, infallible on decode:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackedOrigin {
    /// `kind_text` as a closed-set discriminant. A varint on the wire, and
    /// `RawKind::as_str()` gives back a `&'static str` on load — so decoding an
    /// Origin allocates NOTHING, which is the whole point (see this module's doc).
    pub kind: RawKind,
    pub byte_start: usize,
    pub byte_end: usize,
    pub start: (u32, u32),
    pub end: (u32, u32),
}

impl PackedOrigin {
    /// `None` when `kind_text` is not a named grammar kind — the app is then not
    /// packed. Cold path, fail-closed, never a panic.
    pub fn try_from_origin(o: &al_syntax::ir::Origin) -> Option<Self> { /* ... */ }
    pub fn into_origin(self) -> al_syntax::ir::Origin { /* kind.as_str() */ }
}
```

- [ ] **Step 4b: Write the corpus test that turns "plausibly always named" into a guarantee**

The design assumes every `RoutineDecl.origin`/`name_origin` carries a NAMED kind. Prove it rather than assuming it:

```rust
#[test]
fn every_decl_origin_kind_is_a_named_grammar_kind() {
    // Lower a real fixture corpus and assert both Origins on every routine decl
    // survive try_from_raw. If this ever fails, the encoder's None arm is live
    // and that app silently stops being packable -- which is safe, but we want
    // to KNOW, not discover it as a mystery cache-miss rate.
    let mut checked = 0usize;
    for path in fixture_al_files() {
        let file = al_syntax::parse(&std::fs::read_to_string(&path).unwrap());
        for obj in &file.objects {
            for r in &obj.routines {
                assert!(RawKind::try_from_raw(r.origin.kind_text).is_some(),
                    "{}: routine {} origin kind {:?} is not a named grammar kind",
                    path.display(), r.name, r.origin.kind_text);
                assert!(RawKind::try_from_raw(r.name_origin.kind_text).is_some(),
                    "{}: routine {} name_origin kind {:?} is not a named grammar kind",
                    path.display(), r.name, r.name_origin.kind_text);
                checked += 1;
            }
        }
    }
    assert!(checked > 0, "the corpus produced no routines -- the test proved nothing");
}
```

The final assertion is not decoration: a corpus that yields zero routines would make every other assertion vacuously true. Point `fixture_al_files()` at an existing fixture directory (`tests/r0-corpus/` or `tests/fixtures/`); do not invent new fixtures.

- [ ] **Step 5: Run the tests to verify they pass**

```bash
cargo test -p al-call-hierarchy --lib pack::
```

Expected: all pack tests pass, including the corpus test with a non-zero `checked` count.

- [ ] **Step 6: Prove the corruption test discriminates**

The integrity check is the guard; confirm the test dies without it.

```bash
cd U:/Git/al-call-hierarchy
export BREAK_BACKUP=/tmp/pack_mod.rs.bak
cp src/program/pack/mod.rs "$BREAK_BACKUP"
python - <<'PY'
import pathlib
p = pathlib.Path("src/program/pack/mod.rs")
s = p.read_text(encoding="utf-8")
old = "    if compute_self_hash(&pack) != pack.self_hash {\n        return Err(PackError::SelfHashMismatch);\n    }\n"
assert s.count(old) == 1, f"scripted break did not match: {s.count(old)} occurrences"
p.write_text(s.replace(old, ""), encoding="utf-8")
print("break applied")
PY
cargo test -p al-call-hierarchy --lib pack::a_corrupted_payload
```

Expected: **FAIL**. Note the honest limit: postcard may itself reject a corrupted byte stream, in which case the test passes via the `Codec` arm and the break comes back GREEN. **If that happens, it is a real property of the code, not a broken test** — record it as a stated limit of this guard (the self-hash adds value only for corruption postcard accepts as well-formed), and move on. Do not weaken the test to force a failure.

Revert either way — **from the backup, not from git**: this file is new and untracked at
this point, so `git checkout` would error rather than restore it.

```bash
cd U:/Git/al-call-hierarchy
cp "$BREAK_BACKUP" src/program/pack/mod.rs
rm "$BREAK_BACKUP"
cargo test -p al-call-hierarchy --lib pack::
```

Expected: 3 passed.

- [ ] **Step 7: Lint and commit**

```bash
cd U:/Git/al-call-hierarchy
rustfmt src/program/pack/mod.rs
scripts/ci-steps clippy
git add Cargo.toml Cargo.lock src/program/pack/mod.rs src/program/mod.rs
git commit -F - <<'EOF'
feat(program): dependency pack codec

DepPack / PackedFile plus postcard encode and decode, with a schema check
and a blake3 self-hash. Persistence only -- no engine wiring; obtaining,
keying and loading a pack is spec step 5.

Exists so the format's round-trip cost can be measured on real data before
the ingestion seam is built, which is where the spec places the go/no-go.

App identity is stored symbolically because AppRef is a per-run interning
index. Per-file order is preserved because dedup keeps the first
occurrence per key, so a reordered pack changes which survivor wins.

RoutineMeta IS carried. It is not derivable on a pack hit (from_decl needs a
RoutineDecl, and the parsed units do not exist), and it is the cost class the
gate exists to price, so a pack without it would make the gate a lower bound
rather than a decision.

Origin.kind_text is &'static str and cannot deserialize. Rather than changing
the IR type, widening it to String, or dropping it, the wire encodes it as a
RawKind discriminant via a new fallible try_from_raw; decode goes through
as_str(), which returns &'static str, so loading an Origin allocates nothing.
An unencodable kind means the app is not packed -- cold path, fail-closed,
never a panic.

EOF
```

---

## Task 6: The measurement gate

The decision point. Everything downstream in the spec is contingent on this.

**Files:**
- Create: `benches/dep_pack_roundtrip.rs`
- Modify: `Cargo.toml` (register the bench)
- Create: `docs/2026-08-07-dep-pack-gate-measurement.md`

**Interfaces:**
- Consumes: `DepPack`, `encode`, `decode`, `compute_self_hash`, `PACK_SCHEMA` from Task 5.
- Produces: a committed verdict — PROCEED with postcard, or SWITCH to the shared string table — plus the ledger backing it.

**Gate shape, from spec §13.** Measure the *hit-path shape*, not a micro-benchmark: per-app packs, decoded in parallel on the same pool `parse_snapshot` uses, guid re-intern included, from a cold OS file cache at least once. A serial in-memory `to_vec`/`from_bytes` round trip will sail under 200 ms and tell you nothing about what step 5 builds.

**Thresholds:** under ~200 ms → proceed with postcard. Approaching ~600 ms → switch format and re-measure. The 600 ms line is a product-quality choice, not a soundness line: even a 300 ms load against ~1,280 ms saved is a net win.

- [ ] **Step 1: Register the bench**

In `Cargo.toml`:

```toml
[[bench]]
name = "dep_pack_roundtrip"
harness = false
```

- [ ] **Step 2: Write the bench**

Create `benches/dep_pack_roundtrip.rs`:

```rust
//! The dependency pack format's go/no-go gate.
//!
//! Spec: `docs/superpowers/specs/2026-08-07-dependency-pack-cache-design.md` §13.
//! Measures the HIT-PATH shape — per-app packs, parallel decode, guid
//! re-intern included — because that is what spec step 5 will actually build.
//! A serial in-memory round trip sails under the threshold and proves nothing.
//!
//! Requires a real workspace: set `PACK_BENCH_WS` to a BC workspace root. The
//! bench is skipped with a loud message when unset, never silently passed.

use std::path::PathBuf;
use std::time::Instant;

use al_call_hierarchy::program::build::build_dep_layer;
use al_call_hierarchy::program::node::AppRef;
use al_call_hierarchy::program::pack::{compute_self_hash, decode, encode, DepPack, PackedFile, PACK_SCHEMA};
use al_call_hierarchy::snapshot::parse::parse_snapshot;
use al_call_hierarchy::snapshot::snapshot::SnapshotBuilder;
use rayon::prelude::*;

fn main() {
    let Some(ws) = std::env::var_os("PACK_BENCH_WS").map(PathBuf::from) else {
        eprintln!(
            "PACK_BENCH_WS is unset — this gate needs a real BC workspace and will not \
             guess. Set it to a workspace root and re-run. NOT a pass."
        );
        std::process::exit(2);
    };

    // ---- Build the real population -------------------------------------
    let snap = SnapshotBuilder {
        workspace_root: ws.clone(),
        local_providers: vec![],
    }
    .build()
    .expect("build snapshot");

    let parsed = parse_snapshot(&snap);
    let dep_layer = build_dep_layer(
        &snap,
        &al_call_hierarchy::program::abi_ingest::AbiCache::new(),
        &parsed,
    );

    println!(
        "population: {} dep objects, {} dep routines",
        dep_layer.dep_objects.len(),
        dep_layer.dep_routines.len()
    );

    // ---- Group into per-app packs, mirroring the real hit path ----------
    // One pack per non-primary app, which is the unit spec step 5 loads.
    let mut per_app: std::collections::BTreeMap<u32, (Vec<_>, Vec<_>)> =
        std::collections::BTreeMap::new();
    for o in &dep_layer.dep_objects {
        per_app.entry(o.id.app.0).or_default().0.push(o.clone());
    }
    for r in &dep_layer.dep_routines {
        per_app.entry(r.id.object.app.0).or_default().1.push(r.clone());
    }

    let packs: Vec<Vec<u8>> = per_app
        .iter()
        .map(|(app_ix, (objects, routines))| {
            let mut pack = DepPack {
                schema: PACK_SCHEMA,
                app_guid: format!("app-{app_ix}"),
                app_name: String::new(),
                app_publisher: String::new(),
                app_version: String::new(),
                // One synthetic file per app: this gate measures RECORD cost,
                // not per-file framing. Step 5 splits by real file.
                files: vec![PackedFile {
                    virtual_path: format!("app-{app_ix}"),
                    parse_status_recovered: false,
                    objects: objects.clone(),
                    routines: routines.clone(),
                }],
                self_hash: String::new(),
            };
            pack.self_hash = compute_self_hash(&pack);
            encode(&pack).expect("encode")
        })
        .collect();

    let total_bytes: usize = packs.iter().map(Vec::len).sum();
    println!(
        "artifact: {} packs, {:.1} MB total",
        packs.len(),
        total_bytes as f64 / 1_048_576.0
    );

    // ---- Write to disk and drop the OS cache read ----------------------
    let dir = std::env::temp_dir().join("alsem-pack-gate");
    std::fs::create_dir_all(&dir).expect("mkdir");
    let paths: Vec<PathBuf> = packs
        .iter()
        .enumerate()
        .map(|(i, bytes)| {
            let p = dir.join(format!("pack-{i}.bin"));
            std::fs::write(&p, bytes).expect("write");
            p
        })
        .collect();

    // ---- The measurement: parallel read + decode + re-intern -----------
    for round in 0..3 {
        let start = Instant::now();
        let decoded: Vec<DepPack> = paths
            .par_iter()
            .map(|p| {
                let bytes = std::fs::read(p).expect("read");
                let mut pack = decode(&bytes).expect("decode");
                // Guid -> AppRef re-intern, which the real hit path pays.
                let app_ref = AppRef(pack.app_guid.trim_start_matches("app-").parse().unwrap_or(0));
                for f in &mut pack.files {
                    for o in &mut f.objects {
                        o.id.app = app_ref;
                    }
                    for r in &mut f.routines {
                        r.id.object.app = app_ref;
                    }
                }
                pack
            })
            .collect();
        let elapsed = start.elapsed();
        let routines: usize = decoded
            .iter()
            .flat_map(|p| p.files.iter())
            .map(|f| f.routines.len())
            .sum();
        println!(
            "round {round}: {:.1} ms wall, {routines} routines materialized",
            elapsed.as_secs_f64() * 1000.0
        );
    }

    println!(
        "\nGATE: under ~200 ms -> proceed with postcard. \
         Approaching ~600 ms -> switch to the shared string table and re-measure."
    );
}
```

If any `use` path above does not resolve, fix the path — do not stub the call. The module paths are `src/program/build.rs`, `src/program/pack/mod.rs`, `src/snapshot/parse.rs`, `src/snapshot/snapshot.rs`. Check `src/lib.rs` for which are publicly re-exported and add `pub` visibility if a needed item is crate-private.

- [ ] **Step 3: Run the gate cold**

The first round reads from a cold OS file cache; rounds 2 and 3 are warm. Report all three.

```bash
cd U:/Git/al-call-hierarchy
cargo build --profile release-fast --bench dep_pack_roundtrip
PACK_BENCH_WS=<path-to-a-real-BC-workspace> \
  cargo bench --profile release-fast --bench dep_pack_roundtrip 2>&1 | tee /tmp/gate.log
```

Use `release-fast` per the build guidance — full `--release` is for the SHA-pinned north-star measure only.

- [ ] **Step 4: Record the ledger**

Create `docs/2026-08-07-dep-pack-gate-measurement.md` with, at minimum: the workspace used and its object/routine/`RoutineMeta` counts, the artifact size in MB, all three round timings, the machine and profile, and the verdict.

State explicitly what the pack DOES contain — nodes plus `RoutineMeta`, i.e. the full spec §6 payload apart from the per-file `ParseStatus`/recovered paths, which are a bool and a path list and cannot move the number. The point of saying so is that a reader must be able to tell whether the measurement covers the real hit path. It does.

Do not round in the favourable direction and do not report a single round.

- [ ] **Step 5: Take the decision**

- **Under ~200 ms:** verdict PROCEED. The next plan implements spec steps 4, 5, 6 against this format.
- **Between 200 and 600 ms:** verdict PROCEED WITH NOTE. Record the reasoning; a 300 ms load against ~1,280 ms saved is still a net win.
- **Approaching ~600 ms:** verdict SWITCH. The next plan implements the shared string table format described in spec §13 and re-runs this gate before anything else.

**Take the verdict from the measured number only.** An earlier revision of this step said "unless `RoutineMeta`'s estimated addition pushes it past 600" — an estimate, in a plan whose Goal is to answer this by measurement rather than prediction. `RoutineMeta` is now IN the pack (Task 5), so the measured number is the whole payload and there is nothing left to estimate. If you find yourself wanting to adjust the measured figure by a guess, the gate has been built wrong — stop and say so.

- [ ] **Step 6: Update the CHANGELOG and commit**

Add under `Added`:

```markdown
- Dependency pack codec (`src/program/pack/`) and its format gate bench
  (`benches/dep_pack_roundtrip.rs`) — persistence only, no engine wiring. Measures
  the parallel per-app decode cost that decides the dependency pack cache's
  artifact format. See `docs/2026-08-07-dep-pack-gate-measurement.md` for the
  verdict and `docs/superpowers/specs/2026-08-07-dependency-pack-cache-design.md`
  §13 for why the gate measures the hit-path shape rather than a micro-benchmark.
```

```bash
cd U:/Git/al-call-hierarchy
rustfmt benches/dep_pack_roundtrip.rs
scripts/ci-steps clippy
git add Cargo.toml benches/dep_pack_roundtrip.rs \
        docs/2026-08-07-dep-pack-gate-measurement.md CHANGELOG.md
git commit -F - <<'EOF'
bench(pack): the dependency pack format go/no-go gate

Measures the hit-path shape -- per-app packs, parallel decode on the same
pool parse_snapshot uses, guid re-intern included, cold OS file cache on the
first round -- because a serial in-memory round trip sails under the
threshold and proves nothing about what the ingestion seam will build.

Verdict and ledger in docs/2026-08-07-dep-pack-gate-measurement.md.

The gate exits non-zero when PACK_BENCH_WS is unset rather than silently
passing, so a missing workspace cannot read as a green run.
EOF
```

---

## Self-Review

**Spec coverage for steps 0, 2, 3:**

| Spec section | Task |
|---|---|
| §17.1 preflight over-claim | Task 1 Step 2 |
| §17.2 thread-local claim | Task 1 Step 3 |
| §17.3 `Origin.ts_id` | Task 2 |
| §17.4 `.scm` paths | Task 1 Step 4 |
| §17.5 per-file parser | Out of scope by design; spec keeps it a separate deliverable with its own measurement |
| Step 2 serialization surface | Tasks 3 and 4 |
| Step 3 measurement gate | Tasks 5 and 6 |
| §6 pack contents | Task 5, partially — `RoutineMeta` deferred to step 5 and flagged in the ledger |
| §13 gate shape and thresholds | Task 6 Steps 2, 3, 5 |

**Gaps deliberately left, each with its reason stated in Scope:** spec step 1 (light snapshot, independently shippable, carries its own unresolved design question), steps 4–6 (contingent on Task 6's verdict).

**Type consistency:** `SubtypeTag` is defined in Task 3 and used in Task 4's test fixture and Task 5's payload. `DepPack`/`PackedFile`/`PackError`/`PACK_SCHEMA`/`compute_self_hash`/`encode`/`decode` are defined in Task 5 and consumed by Task 6 under the same names. `ObjectNode`/`RoutineNode` field names in Task 4's fixture match the declarations read from `src/program/node_extract.rs`.

**Known risk in Task 6 Step 2:** the bench's `use` paths assume public visibility that may not exist. The step says to fix visibility rather than stub the call, because a stubbed gate would produce a number for something other than the real population.
