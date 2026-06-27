# Owned AL Syntax IR — Execution Log / Plan

Tracks execution of `docs/superpowers/specs/2026-06-27-owned-al-syntax-ir-design.md`.
Worktree `U:/Git/al-call-hierarchy-ir`, branch `feat/owned-syntax-ir`. Autonomous run.

## Status legend
`[ ]` todo · `[~]` in progress · `[x]` done · `[!]` blocked/needs decision

---

## Phase -2 — Isolate + characterize legacy
- [x] Worktree `feat/owned-syntax-ir` off `master`; grammar submodule init (`eeb2839`,
  node-types.json present, 630 KB); spec committed (`b7e13ee`).
- [~] Characterize legacy feature streams.
  - **FINDING:** characterization is *already* provided by the in-repo Rust-owned golden corpus
    (`tests/r0-*`, `r1a-*`, `r2*`, `r3*`, `r4*`, `l2_vectors`, `l2cap/l2cc/l2order_*`, `l3cg_*`,
    `l3cov_*`, `l3eg_*`, `l3rt_*`, `temp_state_*`, `gap_*`). These pin the L2/L3 feature streams
    byte-for-byte. `aldump --l2 <ws>` emits the L2 projection (`engine::l2::l2_workspace`); the
    `r1a-goldens` are that projection. (aldump's `scripts/r1a-goldens` comment is stale — goldens
    live under `tests/r1a-goldens`.)
  - **The dual-run target (§5 Phase 1) = these goldens + the in-memory feature structs they
    serialize.** No separate characterization corpus needs authoring; the safety net exists.
  - **Legacy/optional:** `AL_SEM_DIR`-gated `cli_a_*` / `cli_b_*` differentials compare against the
    sibling al-sem repo and SKIP when absent (CHANGELOG 0.9.3). Treat as legacy; not part of the
    dual-run gate.
  - [ ] Establish GREEN BASELINE: `cargo test` in the worktree before any change. (in progress)
  - [ ] Enumerate the exact L2/L3 feature-struct fields the dual-run harness must diff (cross-check
    spec §5 1b/1c/1d list against `ExtractBodyResult` + `l3_workspace` projection structs).

- [x] GREEN BASELINE established: **1348 passed, 0 failed, 15 ignored** (worktree, debug).

## Phase -1 — Pin grammar + workspace conversion
**Workspace split = Option B-simplified** (decided 2026-06-27; GPT-5.5 Pro + Gemini both converged
on B over full-split-now). Rationale: a full 3-crate split up front = ~200-file `crate::` path
rewrite BEFORE any IR exists, and the engine still needs `tree-sitter` for the legacy walk until
Phase 5 anyway — so early enforcement is illusory. B-simplified gets the compiler boundary on the
NEW code now at a fraction of the churn:
  - **New crate `al-syntax`** owns the grammar build (`build.rs`) + FFI `language()` + (Phase 0+)
    generated raw layer / lowerer / IR. Its raw internals are crate-private → Cargo physically
    prevents the engine from reaching past the IR.
  - **Existing root crate** keeps its name, gains a path-dep on `al-syntax`, KEEPS `tree-sitter`
    (legacy `body_walk` uses `tree_sitter::Node`) until Phase 5. Calls `al_syntax::language()`.
  - **Defer** `al-engine`/`al-cli` extraction + the engine's `tree-sitter`-drop to **Phase 5** —
    a mechanical final split once no raw-CST dependency remains. Boundary becomes a permanent
    compile error then.
  - CI dep-guard: `tree-sitter` direct dep allowed only in `al-syntax` + the legacy crate during
    migration; final state only `al-syntax`.

- [ ] Pin tree-sitter-al to exact rev; CI stops floating `main`; add `GRAMMAR_NODE_TYPES_HASH`.
- [~] Workspace scaffold (B-simplified): create `al-syntax` (build+FFI), root deps on it, `cargo
  test` green / zero behaviour change.

## Phase 0 — Generated raw layer + IR scaffold

### node-types.json analysis (codegen input, verified)
- 569 type entries: **383 named, 186 anonymous** (keywords/punctuation); **73 distinct field
  names**; **no supertypes/subtypes** in this grammar → no subtype-enum codegen needed.
- Each named entry: `type`, `named`, optional `fields{name→{multiple,required,types[]}}`, optional
  `children{multiple,required,types[]}`.
- **`RawKind`** = one variant per ALL 569 `type` strings + `Error` (kind `"ERROR"`). `from_raw`
  builds a `phf_map` over every type string; unknown → panic. MISSING nodes report their expected
  (in-grammar) kind via `is_missing()`, so they don't break `from_raw`.
- **`FieldName`** = the 73 field names; `as_raw` generated.
- **Typed nodes (`nodes.rs`)**: per named type, a `Raw<Type><'t>(RawNode)` wrapper. Field accessor
  rule: single-type field (e.g. `code_block.body→statement_block`) → typed `Option<RawStatementBlock>`;
  multi-type field (e.g. `procedure.name→[identifier|quoted_identifier]`) → `Option<RawNode>` (lowerer
  matches on `.kind()`). `multiple:true` field → `Vec`/iterator.
- **`shape.rs`**: required/optional facts → `required_field()` debug-asserts (§3.3).
- **Generator** = bespoke `xtask` using `serde_json` + `quote!`; output checked in + hash-guarded.

- [x] **0a** — `xtask gen-syntax` generates `al-syntax/src/raw/generated/{raw_kind,field,mod}.rs`
  (383 named `RawKind` + `Error`, 73 `FieldName`, `from_raw` panics on unknown, `GRAMMAR_NODE_TYPES_HASH`).
  Hash sidecar + al-syntax `build.rs` guard (grammar swap → build fails). `--check` drift guard.
  al-syntax vocabulary unit tests pass. Workspace green/unchanged (pending confirm).
- [ ] **0b** — `schema/kind_policy.rs`: exhaustive `class_of(RawKind)->Class`
  (Semantic/Transparent/Trivia/Recovery/Ignored) + generated coverage test asserting exhaustiveness.
- [x] **0c(1)** — `RawNode` zero-copy lens (`raw/node.rs`): public payload accessors,
  crate-internal `field`/`named_children` (document-order), `id()` documented ephemeral. Committed
  `4e73eb3`. Reviewer-confirmed (both): order-preserving children, private inner node, no
  `RawKind::Missing`.
- [ ] **0c(2)** — typed CST wrappers (`nodes.rs`, generated) + `shape.rs`. **Reviewer-confirmed
  decisions:** generate a struct per named kind (`RawProcedure<'t>(RawNode)`), `cast()`/`node()` +
  field accessors. Accessor rule: single named-type field → `Option<RawT>`; multiple → `Vec<RawT>`
  (needs `RawNode::children_by_field_name`); **multi named-type field → deduplicated union enum**
  (Gemini's correction — `RawNode` fallback loses compile-time wrapper-safety); any **anonymous**
  member type (operators/punct, not a `RawKind`) → `RawNode` fallback (can't type). Typed layer is
  SHAPE safety only, no AL semantics; stays private to al-syntax.
- [ ] **0b** — `schema/kind_policy.rs`: **exhaustive `match class_of(RawKind)->Class` with NO
  wildcard** (Gemini wins the split: compile-time gate beats test-time union; a new variant →
  non-exhaustive match → `cargo check` fails → forced human decision). Classes:
  Semantic/Transparent/Trivia/Recovery/Ignored. Generate a one-time starter to seed the ~340
  Semantic arm; thereafter hand-owned.
- [ ] **0d** — `ir/` types (Origin + Stmt/Expr taxonomy incl. try/asserterror/foreach/case_else),
  `lower/` skeleton, `parse.rs`. ← **reviewer checkpoint here** (IR taxonomy + class_of design).
- [ ] Boundary scanner in CI, monotonic-decrease, seeded from real grep.

## Phase 1 — Lowerer to parity (1a–1d, dual-run; parse-once/fork-same-Tree)
- [ ] 1a raw layer + lowerer + IR snapshots · 1b L2 stream parity · 1c CFN/control/order ·
  1d L3 parity. Malformed-AL fixtures for recovery. Until zero diff across corpus.

## Phases 2–5
- [ ] 2 cut L2 → 3 cut L3 → 4 cut LSP front-end + retire queries → 5 seal (drop engine tree-sitter
  dep, clippy disallowed_methods, CHANGELOG + CLAUDE.md).

---

## Reviewer checkpoints (GPT-5.5 Pro + Gemini 3.1 Pro Preview via pal/OpenRouter)
- [x] Spec v1→v2→v3 (3 rounds, both approved).
- [ ] After Phase 0 (codegen + IR types shape) — review the generated-vocab + IR taxonomy.
- [ ] After Phase 1a/1b (lowerer + dual-run harness) — review parity methodology + first diffs.
- [ ] Before Phase 5 seal — final review of the boundary + deletion of legacy path.
