# Owned AL Syntax IR — Execution Log / Plan

Tracks execution of `docs/superpowers/specs/2026-06-27-owned-al-syntax-ir-design.md`.
Worktree `U:/Git/al-call-hierarchy-ir`, branch `feat/owned-syntax-ir`. Autonomous run.

## PHASE 2 — THE CUT IS UNDERWAY (engine module `engine::l2::ir_walk`, commits 9dfb8b4, 2af36db, e4dbb0d)
`engine::l2::ir_walk` walks the OWNED IR (not tree-sitter) and produces REAL engine
`PFeatures` types — `project_routine_features_ir`'s growing validated slice. Gated vs the
real legacy L2 walk over 591 routines in `engine_ir_walk_statement_tree_parity`:
- **statement_tree** (real `PCFNNode`) 591/591 ✓ · **has_branching** 591/591 ✓
- **loops** (`Vec<PLoop>`, byte-fidelity anchors) 591/591 ✓ · **field_accesses** 591/591 ✓
- **var_assignments** 591/591 ✓ · **condition_references** 591/591 ✓
- **identifier_references** 585/591 (measured — known parenless/chained residual)
Anchors built exactly as legacy `Ctx::anchor` (utf16 cols via `Utf16Cols` over the IR
`Origin` byte column; `syntax_kind` = raw grammar kind). `Origin.byte` → lossless raw text.
### 9 REAL PFeatures fields now gated at 591/591 (engine ir_walk, commits …0226a1f, 0efd8b0):
statement_tree, has_branching, nesting_depth, loops, field_accesses, var_assignments,
condition_references, **unreachable_statements** (block-scoped exit→next-sibling) — all 591/591;
identifier_references 585/591 (measured). These are exactly the fields that need only the IR +
LIGHT scope (record-receiver sets). nesting_depth reuses the engine `compute_nesting_depth`
UNCHANGED on the IR loops — first proof a post-pass grafts onto IR output cleanly.

### UPDATE — IR object properties + record_variables landed (commits d209a70, bce3325).
- **IR extension:** `ObjectDecl.properties` (lowered from `property` nodes) — SourceTable/TableNo/
  PageType now IR-visible. First IR enhancement driven by the cut.
- **record_variables** 591/591 (HARD GATE): record params (temp_state known-temp / param-dependent
  when by-ref / known-false; table_name parsed from the `Record …` type string) + local record vars
  + implicit `Rec` (table/tableext/pageext always; codeunit with TableNo → Rec.table_name = TableNo;
  page gated on `source_table_name` param, None in the harness = legacy parity). **10 real PFeatures
  fields now gated.** Not yet IR-modelled (absent from corpus): named return-value records, report
  dataitem record vars — IR extensions for production.

### UPDATE 2 — record_operations + operation_sites landed (commits 414bad9, 87d68a8). **12 real
PFeatures fields now gated 591/591:** statement_tree, has_branching, nesting_depth, loops,
field_accesses, var_assignments, condition_references, unreachable_statements, record_variables,
**record_operations** (full payload: receiver text via an implicit-frame stack; field_arguments +
field_argument_infos via a ported `expression_info_from_node`; temp_state/record_variable_id
backfilled from `ir_record_variables`), **operation_sites** (unified op0..opN: record-op/lock/commit/
error-call, only error-call carries under_asserterror). identifier_references 585/591 measured.
REMAINING engine fields: **call_sites** (callee classify + argument_texts/infos + argument_bindings
[needs enclosing params/record-vars + variable_types] + result_consumed/object_run_return_used) and
**variables** (all var decls + initializers). Then assemble full `PFeatures`, run operation_order/
control_context UNCHANGED on the IR PCFNNode, gate serde-equality, wire into the driver, delete
`body_walk`. Then Phases 3/4/5.

### UPDATE 3 — record_variables, record_operations, operation_sites, variables ALL landed.
**13 of 14 PFeatures fields now gated 591/591** by the engine `ir_walk`:
statement_tree, has_branching, nesting_depth, loops, field_accesses, var_assignments,
condition_references, unreachable_statements, record_variables, record_operations, operation_sites,
**variables** (params + locals w/ first-assignment initializer + globals). identifier_references
585/591 measured. Also fixed a real LEGACY bug: `classify_rhs` checked stale v2 node kinds
(`boolean_literal`/`true`/`false`); v3 emits `boolean`, so boolean initializers were misclassified
`{kind:expression}` — recognized `boolean`, rebaselined the 11 affected Rust-owned R1a goldens
(diff = exclusively boolean expression→literal). **ONLY `call_sites` REMAINS** before full PFeatures.

### UPDATE 4 — L2 FEATURE EXTRACTION COMPLETE + FULL PFEATURES ASSEMBLED (commits c2b6bf0, a3803d5).
**ALL 14 PFeatures fields produced by the engine `ir_walk`, gated 591/591** (statement_tree,
has_branching, nesting_depth, loops, field_accesses, var_assignments, condition_references,
unreachable_statements, record_variables, record_operations, operation_sites, variables, **call_sites**).
`project_routine_features_ir` assembles the COMPLETE PFeatures; capstone gate
`engine_ir_walk_full_pfeatures_equality` serde-compares byte-for-byte (hash-normalized): **584/591
(98.8%)**. statement_tree is now BYTE-IDENTICAL — the residual 7 are ONLY the known
identifier_references parenless/chained-receiver edge cases (≈6) + 1 broken-AL ERROR-recovery case.
TWO root-cause comment fixes (comments aren't statements): IR `lower_block_child` skip
comment/multiline_comment/pragma (was lowering them as Unknown→phantom "other"); legacy `cfn.rs`
build_block + repeat-loop skip the same (rebaselined R1a goldens, diff = 135 comment-"other" removals).
Also fixed legacy `classify_rhs` v3-`boolean` staleness (variables initializer). order/control_context/
scope_frames stay empty (post-pass fields legacy_l2_features also leaves empty).
### UPDATE 5 — FULL PFEATURES 99.5% BYTE-IDENTICAL (commit 22fb392). identifier_references
parenless-receiver + enum-type fixes drove the capstone `engine_ir_walk_full_pfeatures_equality`
to **588/591 (99.5%)**. Remaining 3 are distinct intricate edge cases: a broken-AL ERROR-recovery
statement_tree node, a chained-receiver-in-condition value ref, a with-receiver bare-`Modify` ref.
Four real engine bugs fixed along the way (boolean classify_rhs v3 staleness; IR + legacy CFN comment
skip; identifier enum/parenless). The owned-IR L2 feature extraction is FEATURE-COMPLETE and
byte-validated at 99.5%.
### UPDATE 6 — L2 CUTOVER VALIDATED 4 WAYS @ 99.5%; IR byte-ready for the driver (commits 6d949e9,
04d65e3, d722554). The validation chain is COMPLETE:
  1. all 14 PFeatures fields gated 591/591,
  2. full PFeatures 99.5% (hash-normalized),
  3. post-passes (control_context + operation_order) graft UNCHANGED → 99.5%, no new divergence,
  4. byte-exact with REAL routine ids 99.5% (compute_routine_id from IR → ids match exactly).
Second IR extension landed: `RoutineDecl.attributes` (EventSubscriber/TryFunction/… — for
classify_kind + control-context guards). Residual 3 = broken-AL ERROR node, chained-receiver-in-cond,
with-bare-Modify (intricate edge cases, diminishing returns).
### UPDATE 7 — BYTE-EXACT @ 99.8% (100% on WELL-FORMED code) (commits d722554, +parenless/chained).
Three more identifier_references fixes (parenless bare-identifier call counted; chained-call receiver
function counted via a one-shot flag) drove byte-exact PFeatures equality (real ids, no normalization)
to **590/591 (99.8%)**. The SINGLE remaining divergence is `ws-callsite-resolutions/Incomplete` —
intentionally-malformed AL (`CallSomething(); @@@` stray tokens), an ERROR-recovery edge case where
IR/legacy differ on the recovery fragment. So the L2 cutover is byte-identical on ALL well-formed
code, validated FIVE ways.
NEXT (mechanical, fully de-risked): (a) swap the `project_routine_features` call in `project_workspace`
for `project_routine_features_ir` (parse the file's IR alongside, match routines by byte position,
compute routine_id as in the byte-exact test) + gate the workspace `L2Projection`; (b) delete
`body_walk` + the engine's per-routine tree-sitter walk. Then Phase 3 (L3) / 4 (LSP) / 5 (seal).

### (historical) NEXT: `call_sites` — the last + most complex field. Needs: callee classification (bare/member/
object-run/unknown via classify.rs adapted to IR exprs + `Origin.byte`; the with-frame member
upgrade); callee_text; argument_texts + argument_infos (ir_expression_info, already built);
argument_bindings (each arg → source variable/param/literal — needs enclosing params + record-vars +
a variable_types_by_name map computed from IR var decls); result_consumed / object_run_return_used
(object-run parent context). Then assemble full `PFeatures`, run operation_order + control_context
UNCHANGED on the IR PCFNNode, gate full serde-`PFeatures` equality, wire `project_routine_features_ir`
into the L2 driver, delete `body_walk`. Then Phase 3 (L3) / 4 (LSP) / 5 (seal).

### (historical) THE SECOND BOUNDARY: the remaining engine fields — **record_operations,
call_sites, operation_sites, variables** — need the object-level scope subsystem
the L2 driver assembles: implicit-`Rec` seeding (object-type-dependent: table self / page+pageext
SourceTable / tableext extends-target / report dataitem source tables / codeunit `TableNo`
property), `variable_types_by_name`, `object_procedure_names`, parameter symbols, and the
record-var temp_state/table_name resolution. record_variables ALONE requires replicating the
implicit-Rec + dataitem seeding (mod.rs:274-356). This is the next major build: port/reuse the
scope assembly for the IR (it is object-structural, largely IR-expressible), produce the rich
payloads (callee via classify.rs adapted to IR exprs + `Origin.byte`; field_arguments; bindings;
temp_state backfill), then run `operation_order.rs` + `control_context.rs` UNCHANGED on the IR
`PCFNNode`, assemble full `PFeatures`, gate serde-equality, wire into the driver, delete `body_walk`.

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

## L2 CUTOVER PROGRESS — op/cs numbering foundation DONE (100%)
The cutover's parity-critical heart (the two-phase op/cs numbering, INV-1/2/3) is validated at
100% across the r0-corpus (591 routines) via the trace-first methodology:
- **record_operations** (commits 9a470e2, ac89161): classification (record_op_type catalog +
  explicit/implicit Rec receivers + with-receiver stack + parenless calls + Rec/xRec convention)
  AND visit order — 591/591. HARD GATE.
- **operation_sites** (5a35295): bare Commit()/Error() interleaved in the unified op0..opN-1
  counter — 591/591. (Key: operation_sites mirrors every record_op + adds commit/error.) HARD GATE.
- **call_sites** (6a21058): cs0..csM order; Error()=op+callsite, Commit()=op-only — 591/591. HARD GATE.
- Lowerer improvement: parenless call statements (`Modify;`) normalized to Call (matches engine).

THE BIGGEST RISK (op-counter desync from chained-receiver/with/visit-order) IS RETIRED. A natural
pre-order IR DFS reproduces legacy's exact op/cs order.

REMAINING cutover (layered, lower-risk, sequenced):
- L3: field_accesses, identifier_references, var_assignments, condition_references (expression descent).
- L4: CFN statement_tree + control-context.
- Assemble full PFeatures from the IR walk; gate full serde-PFeatures equality vs legacy_l2_features.
- Then CUT L2 over: build engine-side project_routine_features_ir consuming the IR, delete legacy
  body_walk. Then Phase 3 (L3), Phase 4 (LSP + retire queries), Phase 5 (seal: drop engine tree-sitter).

## L2 CUTOVER — Layer-3 progress (commits baf8057, 3f2ba88, a2538ff)
Feature re-expression over the IR vs real engine PFeatures (591 routines), trace-validated:
- field_accesses 100% (HARD GATE) — value-position member on a record var; frvars (field rvar set,
  record_var_names semantics) distinct from op rvars (Rec/xRec convention).
- var_assignments 100% (HARD GATE) — lhs base name + literal rhs. Drove a lowerer fix: Member.member
  now RAW (quotes preserved) — source-faithful, consumers norm().
- identifier_references 98.1% (measurement) — value-ref idents; excludes callee/method names +
  keyword_identifier (via Origin.kind_text). Residual = Codeunit::Name object-ref enum values.

### IR refinement needed (next): Member.member carries no position
condition_references' reference_anchor = the MEMBER identifier node's position, but IR Member.member
is a bare String (no Origin). Same lossy-String pattern as var-assignment quotes. FIX: make
ExprKind::Member carry the member's Origin (or member as an ExprId → Identifier expr). Clean but
ripples through the walkers/streams — do fresh. Then condition_references + the rich PFeatures fields
(callee structures, argument bindings) become expressible.

### State: op/cs NUMBERING + 5 FEATURES at 100%; the deepest risk (op-order) is retired.
REMAINING: condition_references (after Member-position refinement) → CFN statement_tree → assemble
full PFeatures (with loop_stack/argument_bindings/callee/control_context/temp_state — the rich
fields, only ANCHORS validated so far) → gate full serde-equality → cut L2 over (engine consumes IR,
delete body_walk) → Phase 3 (L3) → Phase 4 (LSP, retire queries) → Phase 5 (seal). Substantial,
multi-session; the parity-critical heart is DONE.

## L2 CUTOVER — Layer-4: condition_references + statement_tree (commits 1e8d84c, cd77243, 0d269a1)
- **condition_references** 100% (HARD GATE) — idents in if/while/until/case conditions. Drove the
  Member-position refinement: `ExprKind::Member` now carries `member_origin` (the member identifier
  node's provenance) so reference_anchor is expressible. Member.member stored RAW (quotes preserved).
- **identifier_references** 99.0% (measurement) — added DatabaseReference name handling
  (`Database::"Customer"` → unquoted table_name is a value ref). Residual 3 routines = subtle
  parenless/chained-receiver value-ref edge cases.
- **statement_tree (CFN)** 591/591 STRUCTURAL parity (HARD GATE) — the most complex L2 feature. Full
  port of cfn.rs (build_block/build_statement/harvest/guard) over the IR, op/cs leaves referenced by
  DFS SEQUENCE NUMBER. Error renders as cs "error" leaf (never op), mirroring op_id_by_node_id. Three
  lowerer root-cause fixes surfaced: empty `begin end` (skip statement-position keyword tokens),
  case `else` keyword leak (same skip set), case `else begin..end` double-wrap (unwrap sole code_block
  like a branch body). The only exact-match gap is legacy's inert comment-`other` artifact (legacy
  build_block has no comment skip; the IR correctly omits comments — no op/cs, no order entry).

### State: op/cs NUMBERING + 7 L2 FEATURES at 100% (record_ops, operation_sites, call_sites,
field_accesses, var_assignments, condition_references, statement_tree) + identifier_references 99%.
REMAINING: identifier_references residual → assemble full PFeatures rich fields (loop_stack,
argument_bindings, callee shapes, control_context, temp_state) → gate full serde-PFeatures equality →
cut L2 over (engine `project_routine_features_ir`, delete body_walk) → Phase 3 (L3) → Phase 4 (LSP) →
Phase 5 (seal). The structural heart of L2 (op-order + CFN) is DONE; the rich fields are additive.

### Follow-up (cutover-time, not blocking): legacy cfn.rs has no `comment` skip in build_block, so it
emits inert `other` nodes for trailing/inline comments. When the IR becomes the engine these vanish;
verify operation_order/control_context output is unchanged (it keys on op/cs leaves only, so it is).

## L2 CUTOVER — Layer-5: context fields + the architectural boundary (commits 9bce9e4, 752f647)
- **loop_stack + loops table** 100% (HARD GATE) — monotonic loop counter assigns each loop its
  sequence number at the loop node (DFS-discovery order = legacy `{routine}/loop{N}`), pushed so the
  loop's own condition/bounds + body see it; snapshot per op + per call site. loops-table size also
  gated.
- **under_asserterror** 100% (HARD GATE) — asserterror-nesting counter; Some(true) inside an
  asserterror body else None (legacy never emits Some(false)).

### THE BOUNDARY (established empirically): the test-harness trace re-port has done its job. It
validated, at 100% vs the REAL engine over 591 routines, the entire **structural + contextual spine**
of L2: op/cs visit order, record-op classification, the full CFN statement_tree skeleton, field
accesses, var assignments, condition refs, loop_stack/loops, under_asserterror (+ has_branching,
nesting_depth; identifier_references 99%). These needed only the IR + light scope (rvars/frvars/
implicit-bool).

The REMAINING rich PFeatures fields — **callee classification (PCallee), callee_text, argument_texts/
argument_infos/argument_bindings, temp_state, result_consumed/object_run_return_used** — are tightly
coupled to the full `Ctx` scope inputs (`source`, `variable_types_by_name`, `object_procedure_names`,
`enclosing_parameters`, `enclosing_record_variables`, and the implicit-receiver frame TEXT, not just
its is-record bool). Re-porting them into `tests/ir_dual_run.rs` would mean duplicating the engine's
entire scope-context assembly — wasteful and NOT the cutover's end state.

### NEXT (the cut itself): build engine-side `project_routine_features_ir(object, routine_ir, …Ctx
inputs) -> PFeatures` that walks the OWNED IR with the SAME scope `Ctx` the legacy `body_walk`
receives (the L2 driver already assembles these for every routine). Reuses, largely unchanged:
  - `classify.rs` (callee/expression-info) — adapt to read IR exprs + `Origin.byte` for raw text
    (the IR preserves exact byte spans, so callee_text/receiver/argument raw text are lossless).
  - `cfn.rs` → emit a real `PCFNNode` from the IR (the NCfn port proves the shape; swap NCfn→PCFNNode).
  - `operation_order.rs` + `control_context.rs` — run UNCHANGED on the IR-built `PCFNNode` (they key
    on op/cs leaves; statement_tree structural parity guarantees identical order/control_context).
  - `record_op.rs`, scope-frame builder — unchanged.
Then gate full serde-`PFeatures` equality vs `legacy_l2_features`, delete `body_walk`, and the IR is
the L2 engine. Order/control_context/scope_frames come "for free" via the reused post-passes once the
IR emits PCFNNode + the op/cs/record lists with their anchors — all of which are now order-validated.
