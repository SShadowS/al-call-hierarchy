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
- [x] **0c(2)** — typed CST wrappers generated (`nodes.rs`): 383 structs + 14 union enums
  (capped at ≤5 members; larger expression/statement sets → `RawNode` fallback). Single-type →
  `Option<RawT>`/`Vec`; small multi-type → union enum; anon/large → `RawNode`. `RawNode` gained
  `children_by_field`. Committed `936383d`. al-syntax tests + workspace build green, drift clean.
- [x] **0b+0d (merged)** — committed `6b47bb3`. Reviewer-confirmed design (round: GPT+Gemini):
  - `ir/`: own `Point`, `Origin{kind_text,ts_id(ephemeral),byte,start,end}`, push-only Vec arena
    + Copy newtype ids; `AlFile→ObjectDecl→RoutineDecl→Block(Vec<BlockItem>)→Stmt/Expr`. `BlockItem`
    has `Preproc(PreprocGroup)` holding BOTH #if/#else branches (legacy parity). Taxonomy covers
    try/asserterror/foreach/case_else; `Unknown` preserves unmodelled nodes as data.
  - `schema/kind_policy.rs`: exhaustive `class_of` (no wildcard) = loudness gate. `Class
    {Trivia(86), Recovery(1), Structural(297)}`. Descent owned by per-context lowerer (not class_of).
  - `lower/` + `parse.rs`: `parse(source)->AlFile`, status Clean/Recovered. `origin_of` helper.
  - **PHASE 0 COMPLETE.** al-syntax: 6 tests green; workspace green; drift-guarded.

---

## Phase 1 — Lowerer to parity (THE BIG LIFT; 1a–1d, dual-run)
Per-construct lowering of the full AL surface + the parse-once/fork-same-Tree dual-run harness
diffing legacy-vs-IR feature streams across the corpus until zero. Resolves the flat-vs-structured
preproc question empirically. (Spec §5 Phase 1, INV-1/2/3.)
- [x] **1a** outer structure (objects→routines→params/return/locals/globals) — `0e7ed46`.
- [x] **1a** bodies (blocks→statements→expressions; preproc flattened first-cut; Unknown+issue for
  unmodelled) — `487434b`. 8 al-syntax tests green. **Lowerer produces a full IR for common AL.**
- [ ] **1a tail** (driven by dual-run): temporary detection, member-trigger enclosing member,
  structured-vs-flat preproc decision, rare-kind coverage, name/quote parity.
- [~] **1b** dual-run harness LIVE (`tests/ir_dual_run.rs`, legacy query vs IR over r0-corpus,
  335 files). **4 feature streams hard-gated at 100%:** routine inventory, call inventory,
  member access (body-scoped), variable inventory. Caught + fixed 2 real lowerer bugs
  (case-else dropped statements; qualified-enum inner member). Legacy side =
  `src/dual_run_support.rs` (real engine queries / tree walk). Commits `3de62d8`, `eb7e2ba`,
  `5890995`. **The lowerer's structural fidelity is now corpus-validated.**
- [ ] **deeper L2 parity** (merges into Phase 2): the full `PFeatures` projection (loops,
  operationSites, callSites with node.id-keyed maps + visit order, fieldAccesses, recordOps,
  identifierReferences, CFN). Requires re-expressing the L2 walk over the IR + the
  parse-once/fork-same-Tree harness (INV-2). This is the bulk of the remaining migration.
- [ ] **1c** CFN/control/operation-order parity · **1d** L3 projection parity.
- [ ] malformed/live-edit recovery fixtures.

### Grammar feedback (we own tree-sitter-al) — see [[tree-sitter-al-grammar-issues]]
Spec written at `u:\Git\tree-sitter-al\docs\improvements-for-owned-ir-consumer.md`. Decoupled by the
pin; ADOPT after Phase 1 broad parity, then bump+revalidate via this dual-run harness.

> **Honest status:** the grammar-insulation FOUNDATION is complete (substrate + IR contract + a
> working lowerer for common AL). The remaining 1b–5 (dual-run validation + re-expressing L2/L3/LSP
> over the IR + seal) is the bulk of the migration — a large multi-session effort, since it
> reimplements ~3000 lines of body_walk/l3_workspace feature extraction against the IR and proves
> byte-parity. Proceeding incrementally, each step committed + green + dual-run-gated.
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

## Grammar fixes ADOPTED (2026-06-27, commit 6701f21)
tree-sitter-al #1 (named in/is/as expressions) + #2 (case_else body field) made on branch
`owned-ir-grammar-fixes` (a6128d0, local-only) and adopted before Phase 2. Removed the field
bleed; regen vocab (386 kinds), class_of loudness gate fired + classified, lower fallback recurses.
Validated behaviour-preserving: full suite 1353/0/15 + dual-run 5x100%; complexity 8→9 was a fix.
Proves the seam makes grammar bumps cheap+gated. (Push grammar branch to github before CI.)

## Phase 2 STARTED — real-L2 dual-run gate (commits d15a2c4, 41bdcea)
`dual_run_support::legacy_l2_features(source)` drives the REAL engine L2 walk
(`project_routine_features`) per routine → `(routine_name, PFeatures)`. This is the proper
Phase-2 gate (actual engine features, not query proxies). Per-routine IR traversal built
(block/stmt recursion over the IR). Real-L2 features re-expressed over the IR, both 335/335:
  - **has_branching** (if/case/try present).
  - **nesting_depth** (max loop-nesting; loops by containment, if/case transparent).
8 dual-run streams total (6 query + 2 real-L2). REMAINING real-L2 features (the intricate bulk):
callSites (calleeText/callee + op-numbering keyed on node.id + visit-order, INV-1/2/3),
operationSites, recordOperations, fieldAccesses, identifierReferences, CFN statement_tree.
Then cut L2 over to consume the IR (delete the legacy walk), then L3, LSP, seal.

## Phase 2 finding — IR-level vs cutover-level features (key)
The dual-run revealed a clean split in what's validatable at the IR level vs what needs the cutover:
- **IR-level (SYNTAX) — done, 8 streams 100%:** routine/call-name/member/variable/statement-kind/
  temporary inventories + has_branching + nesting_depth. These are derivable from the IR alone.
- **Cutover-level (SEMANTICS):** callSites vs recordOperations vs operationSites is a 3-way SEMANTIC
  classification legacy's L2 makes (knowing Customer is a Record, FindSet a builtin). The IR is
  syntax and correctly does NOT split. So callSite/recordOp/opSite parity + op-numbering CANNOT be a
  standalone IR stream — it IS the L2 CUTOVER: re-express body_walk's classification + two-phase
  op-numbering (node.id-keyed, visit-order INV-1/2/3) + CFN OVER the IR, producing PFeatures, gated
  against legacy PFeatures (full-struct equality). (Method-name completeness already at 100% via the
  call-inventory stream — the CALLS query captures record-op methods too.)
NEXT: the L2 cutover (re-express body_walk over IR). The intricate multi-session core; the real-L2
gate (legacy_l2_features) is the full-PFeatures comparison target.

## L2 CUTOVER blueprint (reviewer-validated — GPT-5.5 Pro + Gemini, convergent)
Re-express body_walk over the IR. The plan before writing ~1100 lines:

**Architecture:** ONE legacy-shaped ordered IR DFS ("event spine") with a shared Ctx (op/cs counters,
loop stack, implicit-receiver stack, asserterror depth, op_id_by_ts_id / cs_id_by_ts_id maps). Reuse
`Origin.ts_id` for op keys. DO NOT build the numbering from a separate CST pass (violates G1 / leaks
engine semantics into syntax). DO NOT normalize the walk cleaner than legacy.

**Biggest risk:** the walk must mirror legacy VISIT SEMANTICS, not just document order — esp.
`chained_receiver_descent` (body_walk.rs:669: call emits before arg descent, chained receiver visited
after args) and `with_statement` (receiver visited BEFORE pushing implicit frame, body after,
:954-971). If the IR DFS reorders these, every op id diverges.

**Side-channel trace (build FIRST, key debugging tool):** emit an ordered `Vec` of (ts_id, op_id) /
visit events in BOTH the legacy path and the IR DFS; diff THAT before any feature payload. First
divergent element = the node that flipped order. Loop/asserterror/with context stacks must exist from
step 1 (loop_stack is embedded in record-ops AND call-sites).

**Sequencing (layered, dependency order):**
- L1 structural: loops, has_branching, unreachable (no op-counter). [has_branching/nesting DONE]
- L2 op-counter: operation_sites + record_operations + call_sites together (share op0..opN-1 then
  op{N+i} post-visit). Gate per-array to 100% via the trace before proceeding.
- L3 value/refs: field_accesses, identifier_references, var_assignments, condition_references
  (expression descent — lossiness-sensitive: parens unwrap, quote strip).
- L4 CFN statement_tree + control-context + bindings — LAST (consume the id maps + loop_stack).

**Gating:** per-array equality during dev (a desync at node 3 makes full-JSON a wall of red), then
full-PFeatures serde equality as final acceptance. Never set-sort order-sensitive vectors; compare
routines in legacy order. legacy_l2_features is the comparison target.

**Desync detection:** the lossy IR (preproc flattened, empty_statement skipped, parens unwrapped,
qualified_enum lowered) is fine for numbering IF INV-1/2/3 hold (legacy only counts op_index on
call_expression / parenless member ops / commit / error — it skips trivia/parens/empty too). A
desync means the lowerer didn't respect visit/document order on a SEMANTIC node → loud architectural
failure (good), caught by the trace.
